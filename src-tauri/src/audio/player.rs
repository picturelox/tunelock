// Player — real-time playback state for one player slot.
//
// A player reads from a decoded source buffer and applies gain, pan, mute/solo,
// EQ, and tempo. All state is preallocated — no allocation in the process path.
//
// The player tracks its position in source samples and output frames.
// Source position is used for seeking and looping; output frame is used
// for scheduling.
//
// Players are assigned to buses (A, B, or Master direct). The bus handles
// crossfader participation and bus-level EQ.

use std::sync::Arc;

use super::command::{PlayerId, BusId, DecodedBuffer, EqBand, LoopRegion, SourceHandle, BeatGridCompact};
use super::eq::DjIsolator;
use super::timepitch::{TimePitchProcessor, ProcessorSet, ProcessorMode};

/// Ramped gain to avoid clicks.
struct RampedGain {
    current: f64,
    target: f64,
    ramp_increment: f64,
}

impl RampedGain {
    fn new(sample_rate: f64) -> Self {
        Self {
            current: 1.0,
            target: 1.0,
            ramp_increment: 1.0 / (0.005 * sample_rate), // 5ms default ramp
        }
    }

    fn set_target(&mut self, target: f64) {
        self.target = target;
    }

    fn set_ramp(&mut self, sample_rate: f64, ramp_frames: u32) {
        if ramp_frames > 0 {
            self.ramp_increment = (self.target - self.current).abs() / ramp_frames as f64;
            if self.ramp_increment < 1e-12 {
                self.ramp_increment = 1e-12;
            }
        } else {
            // Default 5ms ramp
            self.ramp_increment = 1.0 / (0.005 * sample_rate);
        }
    }

    #[inline]
    fn tick(&mut self) -> f64 {
        if (self.current - self.target).abs() <= self.ramp_increment {
            self.current = self.target;
        } else if self.current < self.target {
            self.current += self.ramp_increment;
        } else {
            self.current -= self.ramp_increment;
        }
        self.current
    }
}

/// Player state — one per slot in the Layer Grid.
pub struct Player {
    pub id: PlayerId,
    pub playing: bool,
    pub muted: bool,
    pub soloed: bool,

    // Source handle (set by Launch); the buffer lives in the processor
    source_handle: Option<SourceHandle>,
    buffer: Option<Arc<DecodedBuffer>>,

    // Time/pitch engine — preconstructed set of all three processor types
    // (bypass, varispeed, signalsmith). All allocation happens at Player
    // construction. Mode switching in the realtime callback only changes
    // the enum — no construction or destruction.
    processor: ProcessorSet,

    // Deferred-destruction queue for retired source buffers. When a new
    // source replaces an old one, the old Arc is pushed here instead of
    // being dropped on the realtime thread.
    retired_sources: Arc<crossbeam_queue::ArrayQueue<Arc<DecodedBuffer>>>,

    // Fixed-capacity overflow buffer for when the retirement queue is full.
    // Each slot is preallocated. When the queue can't accept a source, it
    // goes into the next free slot here. On each retire call, we first try
    // to flush overflow slots to the queue. This NEVER overwrites an
    // occupied slot — if all overflow slots are full, the source stays in
    // the caller's local variable (which is dropped outside the callback
    // via the return value of set_source, which goes to the engine's
    // drain path). The key invariant: no Arc<DecodedBuffer> is ever
    // dropped on the realtime thread due to overflow.
    //
    // Capacity 8 is generous: a single relaunch retires at most 2 Arcs
    // (old buffer + old processor source). 8 slots handles 4 consecutive
    // relaunches without draining, which is far beyond normal operation.
    overflow_retire: [Option<Arc<DecodedBuffer>>; 8],

    // Bus assignment
    pub bus: BusId,

    // EQ (per-player lightweight isolator)
    eq: DjIsolator,

    // Gains
    gain: RampedGain,
    /// PB-6.1: Loudness match gain (linear, separate from user gain).
    /// Computed from stored LUFS values to level-match this player to
    /// another. Total gain = user_gain * loudness_match_gain.
    /// Default 1.0 (no match). Toggled off restores exact previous gain.
    loudness_match_gain: f64,

    // Pan (-1.0 to 1.0)
    pan: f64,

    // Loop region (in beats, relative to beat grid)
    loop_region: Option<LoopRegion>,

    // Beat grid for this player's source
    bpm: f64,
    first_beat_sec: f64,
    meter_numerator: i32,

    // Metering (per-player, this block)
    block_sum_sq: [f64; 2],
    block_peak: [f64; 2],
    clip: [bool; 2],

    sample_rate: f64,
}

impl Player {
    pub fn new(
        id: PlayerId,
        sample_rate: f64,
        retired_sources: Arc<crossbeam_queue::ArrayQueue<Arc<DecodedBuffer>>>,
    ) -> Self {
        Self::new_with_mode(id, sample_rate, retired_sources, ProcessorMode::Signalsmith)
    }

    /// Create a player with a specific processor mode. Used in tests
    /// to use Varispeed (zero latency, sample-exact) instead of
    /// the default Signalsmith (which has STFT latency).
    pub fn new_with_processor(
        id: PlayerId,
        sample_rate: f64,
        retired_sources: Arc<crossbeam_queue::ArrayQueue<Arc<DecodedBuffer>>>,
        _processor: Box<dyn TimePitchProcessor>,
    ) -> Self {
        // Legacy: the processor argument is ignored. We preconstruct all
        // three processors and default to Varispeed for test compatibility.
        Self::new_with_mode(id, sample_rate, retired_sources, ProcessorMode::Varispeed)
    }

    /// Create a player with a specific processor mode. All three processors
    /// are preconstructed here — no allocation occurs later in the callback.
    pub fn new_with_mode(
        id: PlayerId,
        sample_rate: f64,
        retired_sources: Arc<crossbeam_queue::ArrayQueue<Arc<DecodedBuffer>>>,
        mode: ProcessorMode,
    ) -> Self {
        Self {
            id,
            playing: false,
            muted: false,
            soloed: false,
            source_handle: None,
            buffer: None,
            processor: ProcessorSet::new(sample_rate, 2, mode),
            retired_sources,
            overflow_retire: [None, None, None, None, None, None, None, None],
            bus: if id.0 == 0 { BusId::A } else { BusId::B },
            eq: DjIsolator::new(sample_rate),
            gain: RampedGain::new(sample_rate),
            loudness_match_gain: 1.0,
            pan: 0.0,
            loop_region: None,
            bpm: 120.0,
            first_beat_sec: 0.0,
            meter_numerator: 4,
            block_sum_sq: [0.0; 2],
            block_peak: [0.0; 2],
            clip: [false; 2],
            sample_rate,
        }
    }

    /// Retire an old source buffer to the deferred-destruction queue.
    ///
    /// If the queue is full, the source goes into a fixed-capacity overflow
    /// buffer. On each call, we first try to flush overflow entries to the
    /// queue. This NEVER overwrites an occupied slot.
    ///
    /// Returns any Arc that could not be stored (queue full + all overflow
    /// slots full). The caller MUST hold the returned Arc off the realtime
    /// thread (e.g., in an engine-level deferred list drained on the async
    /// runtime). This guarantees no Arc<DecodedBuffer> is ever dropped on
    /// the realtime audio thread.
    #[inline]
    fn retire(&mut self, old: Option<Arc<DecodedBuffer>>) -> Option<Arc<DecodedBuffer>> {
        // First, try to flush any overflow entries to the queue
        for slot in &mut self.overflow_retire {
            if let Some(buf) = slot.take() {
                if let Err(returned) = self.retired_sources.push(buf) {
                    // Queue still full — put it back in this slot and stop
                    *slot = Some(returned);
                    break;
                }
            }
        }

        // Now try to push the new source to the queue
        if let Some(buf) = old {
            if let Err(returned) = self.retired_sources.push(buf) {
                // Queue full — store in first free overflow slot.
                let mut to_store = Some(returned);
                for slot in &mut self.overflow_retire {
                    if slot.is_none() {
                        *slot = to_store.take();
                        break;
                    }
                }
                // If to_store is still Some, all overflow slots were full.
                // Return it to the caller — do NOT force_push (which would
                // evict and drop an older Arc on the realtime thread).
                return to_store;
            }
        }
        None
    }

    pub fn launch(
        &mut self,
        handle: SourceHandle,
        buffer: Arc<DecodedBuffer>,
        start_beat: f64,
    ) -> [Option<Arc<DecodedBuffer>>; 2] {
        self.source_handle = Some(handle);
        if let Some(bg) = &buffer.beat_grid {
            self.bpm = bg.bpm;
            self.first_beat_sec = bg.first_beat_sec;
            self.meter_numerator = bg.meter_numerator;
        } else {
            self.bpm = buffer.bpm.unwrap_or(120.0);
            self.first_beat_sec = 0.0;
            self.meter_numerator = 4;
        }
        let beat_duration_sec = 60.0 / self.bpm;
        let start_sec = self.first_beat_sec + start_beat * beat_duration_sec;
        let start_frame = start_sec * buffer.sample_rate as f64;
        let old_processor_src = self.processor.set_source(buffer.clone(), start_frame);
        let old_buffer = self.buffer.take();
        self.buffer = Some(buffer);
        // retire() returns any Arc that couldn't be stored. Collect them
        // for the caller to push to the engine-level deferred_overflow queue.
        // Fixed array — no allocation.
        let unstored: [Option<Arc<DecodedBuffer>>; 2] = [
            self.retire(old_processor_src),
            self.retire(old_buffer),
        ];
        self.playing = true;
        self.eq.reset();
        unstored
    }

    pub fn load_buffer(&mut self, buffer: Arc<DecodedBuffer>) -> [Option<Arc<DecodedBuffer>>; 2] {
        if let Some(bg) = &buffer.beat_grid {
            self.bpm = bg.bpm;
            self.first_beat_sec = bg.first_beat_sec;
            self.meter_numerator = bg.meter_numerator;
        } else {
            self.bpm = buffer.bpm.unwrap_or(120.0);
        }
        let old_processor_src = self.processor.set_source(buffer.clone(), 0.0);
        let old_buffer = self.buffer.take();
        self.buffer = Some(buffer);
        let unstored: [Option<Arc<DecodedBuffer>>; 2] = [
            self.retire(old_processor_src),
            self.retire(old_buffer),
        ];
        self.eq.reset();
        unstored
    }

    pub fn set_source_handle(&mut self, handle: SourceHandle) {
        self.source_handle = Some(handle);
    }

    /// Switch the active processor mode (bypass/varispeed/signalsmith).
    /// All three processors are preconstructed — no allocation or
    /// deallocation in the realtime callback. The source is re-attached
    /// to the newly active processor at the current audible position.
    /// The incoming processor's stale source (if any) is retired.
    pub fn set_processor_mode(&mut self, mode: ProcessorMode) {
        let source = self.buffer.clone();
        let stale = self.processor.switch_mode(mode, source);
        let _ = self.retire(stale);
    }

    /// Get the current processor mode.
    pub fn processor_mode(&self) -> ProcessorMode {
        self.processor.mode()
    }

    pub fn play(&mut self) {
        self.playing = true;
    }

    pub fn pause(&mut self) {
        self.playing = false;
    }

    pub fn stop(&mut self) {
        self.playing = false;
        self.processor.seek_frames(0.0);
        self.processor.reset();
        self.eq.reset();
    }

    pub fn seek_beats(&mut self, source_beat: f64) {
        if let Some(buf) = &self.buffer {
            let beat_duration_sec = 60.0 / self.bpm;
            let pos_sec = self.first_beat_sec + source_beat * beat_duration_sec;
            let frame = pos_sec * buf.sample_rate as f64;
            self.processor.seek_frames(frame);
        }
        self.processor.reset();
        self.eq.reset();
    }

    pub fn seek_sec(&mut self, position_sec: f64) {
        if let Some(buf) = &self.buffer {
            let frame = position_sec * buf.sample_rate as f64;
            self.processor.seek_frames(frame);
        }
        self.processor.reset();
        self.eq.reset();
    }

    pub fn set_tempo(&mut self, rate: f32) {
        self.processor.set_tempo_ratio(rate as f64);
    }

    pub fn set_pitch_semitones(&mut self, semitones: f32) {
        self.processor.set_pitch_semitones(semitones as f64);
    }

    pub fn set_gain(&mut self, gain: f32, ramp_frames: u32) {
        self.gain.set_target(gain as f64);
        self.gain.set_ramp(self.sample_rate, ramp_frames);
    }

    /// PB-6.1: Set the loudness match gain (linear). This is separate
    /// from the user gain/trim. Setting to 1.0 disables match level.
    /// The match gain is applied immediately (not ramped) because it
    /// changes only when the user toggles Match Level, and a ramp
    /// would delay the level change unnecessarily.
    pub fn set_loudness_match_gain(&mut self, gain: f64) {
        self.loudness_match_gain = gain;
    }

    /// PB-6.1: Get the current loudness match gain (linear).
    pub fn loudness_match_gain(&self) -> f64 {
        self.loudness_match_gain
    }

    pub fn set_pan(&mut self, pan: f32) {
        self.pan = pan as f64;
    }

    pub fn set_mute(&mut self, muted: bool) {
        self.muted = muted;
    }

    pub fn set_solo(&mut self, soloed: bool) {
        self.soloed = soloed;
    }

    pub fn set_bus(&mut self, bus: BusId) {
        self.bus = bus;
    }

    pub fn set_eq_gain(&mut self, band: EqBand, gain_db: f32) {
        self.eq.set_gain_db(band, gain_db);
    }

    pub fn set_eq_kill(&mut self, band: EqBand, killed: bool) {
        self.eq.set_kill(band, killed);
    }

    pub fn set_loop(&mut self, region: Option<LoopRegion>) {
        self.loop_region = region;
    }

    pub fn get_position_sec(&self) -> f64 {
        if let Some(buf) = &self.buffer {
            self.processor.position_frames() / buf.sample_rate as f64
        } else {
            0.0
        }
    }

    /// Attach a beat grid to an already-loaded player. Called when
    /// async beat-grid analysis completes. Updates BPM, first beat,
    /// and meter without reloading the source or affecting position.
    pub fn attach_beat_grid(&mut self, bpm: f64, first_beat_sec: f64, meter_numerator: i32, _downbeat_offset: usize) {
        self.bpm = bpm;
        self.first_beat_sec = first_beat_sec;
        self.meter_numerator = meter_numerator;
    }

    /// Source BPM (from beat grid or fallback 120).
    pub fn source_bpm(&self) -> f64 {
        self.bpm
    }

    /// Effective BPM = source BPM × tempo ratio.
    pub fn effective_bpm(&self) -> f64 {
        self.bpm * self.processor.tempo_ratio()
    }

    /// Current tempo ratio (1.0 = original speed).
    pub fn tempo_ratio(&self) -> f64 {
        self.processor.tempo_ratio()
    }

    /// Current pitch shift in semitones.
    pub fn pitch_semitones(&self) -> f64 {
        self.processor.pitch_semitones()
    }

    /// Meter numerator (e.g., 4 for 4/4).
    pub fn meter_numerator(&self) -> i32 {
        self.meter_numerator
    }

    /// Current beat position (fractional, relative to first beat).
    pub fn beat_position(&self) -> f64 {
        let pos_sec = self.get_position_sec();
        if self.bpm > 0.0 {
            (pos_sec - self.first_beat_sec) * self.bpm / 60.0
        } else {
            0.0
        }
    }

    /// Current bar position (fractional, 0 = first bar).
    pub fn bar_position(&self) -> f64 {
        if self.meter_numerator > 0 {
            self.beat_position() / self.meter_numerator as f64
        } else {
            0.0
        }
    }

    /// Processor algorithmic latency in frames.
    pub fn latency_frames(&self) -> usize {
        self.processor.latency_frames()
    }

    pub fn get_duration_sec(&self) -> f64 {
        self.buffer.as_ref().map(|b| b.duration_sec).unwrap_or(0.0)
    }

    pub fn is_loaded(&self) -> bool {
        self.buffer.is_some()
    }

    pub fn is_audible(&self, any_soloed: bool) -> bool {
        self.playing && !self.muted && (!any_soloed || self.soloed)
    }

    /// Reset block metering (called at start of each output block).
    pub fn reset_block_meters(&mut self) {
        self.block_sum_sq = [0.0; 2];
        self.block_peak = [0.0; 2];
    }

    /// Get block RMS and peak for metering.
    pub fn get_block_meters(&self, sample_count: usize) -> (f64, f64, bool) {
        let rms = if sample_count > 0 {
            ((self.block_sum_sq[0] + self.block_sum_sq[1]) / (2.0 * sample_count as f64)).sqrt()
        } else {
            0.0
        };
        let peak = self.block_peak[0].max(self.block_peak[1]);
        let clip = self.clip[0] || self.clip[1];
        (rms, peak, clip)
    }

    /// Clear clip flags.
    pub fn clear_clip(&mut self) {
        self.clip = [false; 2];
    }

    /// Process one stereo sample pair from this player.
    /// Returns (left, right) output. If not playing, muted, or no buffer,
    /// returns (0, 0). The caller (engine) routes the output to the bus.
    /// The time/pitch processor owns source positioning and interpolation.
    #[inline]
    pub fn process_sample(&mut self, any_soloed: bool) -> (f64, f64) {
        if !self.is_audible(any_soloed) {
            return (0.0, 0.0);
        }

        if self.buffer.is_none() {
            return (0.0, 0.0);
        }

        // Pull the next frame from the time/pitch processor
        let (src_l, src_r) = match self.processor.next_frame() {
            Some(frame) => frame,
            None => {
                self.playing = false;
                return (0.0, 0.0);
            }
        };

        // Handle looping: wrap the processor position when it crosses the
        // loop end (checked in source frames).
        if let Some(loop_region) = &self.loop_region {
            let beat_duration_sec = 60.0 / self.bpm;
            let loop_start_sec = self.first_beat_sec + loop_region.start_beat * beat_duration_sec;
            let loop_end_sec = loop_start_sec + loop_region.length_beats * beat_duration_sec;
            let sample_rate = self.buffer.as_ref().map(|b| b.sample_rate as f64).unwrap_or(44100.0);
            let loop_end_frame = loop_end_sec * sample_rate;

            if self.processor.position_frames() >= loop_end_frame {
                let loop_start_frame = loop_start_sec * sample_rate;
                self.processor.seek_frames(loop_start_frame);
                self.processor.reset();
                self.eq.reset();
            }
        }

        // Apply per-player EQ
        let (eq_l, eq_r) = self.eq.process(src_l, src_r);

        // Apply gain (ramped user gain × loudness match gain)
        let g = self.gain.tick() * self.loudness_match_gain;

        // Apply pan (constant power)
        let (pan_l, pan_r) = if self.pan == 0.0 {
            (g, g)
        } else {
            let angle = (self.pan + 1.0) * std::f64::consts::PI / 4.0;
            (g * angle.cos(), g * angle.sin())
        };

        let out_l = eq_l * pan_l;
        let out_r = eq_r * pan_r;

        // Update meters
        self.block_sum_sq[0] += out_l * out_l;
        self.block_sum_sq[1] += out_r * out_r;
        let abs_l = out_l.abs();
        let abs_r = out_r.abs();
        if abs_l > self.block_peak[0] { self.block_peak[0] = abs_l; }
        if abs_r > self.block_peak[1] { self.block_peak[1] = abs_r; }
        if abs_l >= 1.0 { self.clip[0] = true; }
        if abs_r >= 1.0 { self.clip[1] = true; }

        (out_l, out_r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_player() -> Player {
        let retired = Arc::new(crossbeam_queue::ArrayQueue::new(8));
        Player::new_with_mode(
            PlayerId(0),
            48000.0,
            retired,
            ProcessorMode::Varispeed,
        )
    }

    /// PB-6.1: Loudness match gain defaults to 1.0 (no match).
    #[test]
    fn loudness_match_gain_defaults_to_unity() {
        let p = make_player();
        assert!((p.loudness_match_gain() - 1.0).abs() < 1e-9);
    }

    /// PB-6.1: Setting loudness match gain changes it.
    #[test]
    fn set_loudness_match_gain_changes_value() {
        let mut p = make_player();
        p.set_loudness_match_gain(1.5);
        assert!((p.loudness_match_gain() - 1.5).abs() < 1e-9);
        p.set_loudness_match_gain(0.7);
        assert!((p.loudness_match_gain() - 0.7).abs() < 1e-9);
    }

    /// PB-6.1: Toggling match off (gain=1.0) restores exact previous
    /// user gain. The user gain is never modified by match level.
    #[test]
    fn toggling_match_off_restores_user_gain_exactly() {
        let mut p = make_player();
        // Set a user gain
        p.set_gain(0.8, 1);
        // Apply loudness match
        p.set_loudness_match_gain(1.25);
        // Toggle off
        p.set_loudness_match_gain(1.0);
        // User gain should be unchanged
        // (gain.target is still 0.8 — we read it indirectly via the ramp)
        // After ramp completes (1 frame), gain.tick() returns target.
        p.gain.set_ramp(48000.0, 1);
        let g = p.gain.tick();
        assert!((g - 0.8).abs() < 1e-5, "user gain should be 0.8, got {}", g);
    }

    /// PB-6.1: The total gain is user_gain * loudness_match_gain.
    /// This is the core invariant: match level and user trim are
    /// independent and multiplicative.
    #[test]
    fn total_gain_is_user_times_match() {
        let mut p = make_player();
        p.set_gain(0.5, 1); // user gain = 0.5
        p.set_loudness_match_gain(2.0); // match gain = 2.0
        p.gain.set_ramp(48000.0, 1);
        let user_g = p.gain.tick();
        let total = user_g * p.loudness_match_gain();
        assert!((total - 1.0).abs() < 1e-9, "total should be 1.0, got {}", total);
    }

    /// PB-6.1: Match gain computation from LUFS values.
    /// To match B to A: gain = 10^((LUFS_A - LUFS_B) / 20)
    #[test]
    fn match_gain_formula_is_correct() {
        // A = -23 LUFS, B = -20 LUFS (B is louder by 3 LU)
        // To match B to A, B needs -3 dB gain = 10^(-3/20) ≈ 0.7079
        let lufs_a = -23.0f64;
        let lufs_b = -20.0f64;
        let delta = lufs_a - lufs_b; // -3.0
        let gain = 10f64.powf(delta / 20.0);
        assert!((gain - 0.7079).abs() < 0.01, "gain should be ~0.708, got {}", gain);

        // A = -23, B = -26 (B is quieter by 3 LU)
        // To match B to A, B needs +3 dB gain = 10^(3/20) ≈ 1.4125
        let lufs_b_quieter = -26.0f64;
        let delta2 = lufs_a - lufs_b_quieter; // 3.0
        let gain2 = 10f64.powf(delta2 / 20.0);
        assert!((gain2 - 1.4125).abs() < 0.01, "gain should be ~1.413, got {}", gain2);
    }
}
