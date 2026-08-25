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
use super::timepitch::{TimePitchProcessor, default_processor};

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

    // Time/pitch engine — the replaceable abstraction. Owns the source
    // position; the player never reads the buffer directly.
    processor: Box<dyn TimePitchProcessor>,

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
        Self::new_with_processor(id, sample_rate, retired_sources, default_processor(sample_rate, 2))
    }

    /// Create a player with a specific time/pitch processor. Used in tests
    /// to use VarispeedProcessor (zero latency, sample-exact) instead of
    /// the default SignalsmithProcessor (which has STFT latency).
    pub fn new_with_processor(
        id: PlayerId,
        sample_rate: f64,
        retired_sources: Arc<crossbeam_queue::ArrayQueue<Arc<DecodedBuffer>>>,
        processor: Box<dyn TimePitchProcessor>,
    ) -> Self {
        Self {
            id,
            playing: false,
            muted: false,
            soloed: false,
            source_handle: None,
            buffer: None,
            processor,
            retired_sources,
            overflow_retire: [None, None, None, None, None, None, None, None],
            bus: if id.0 == 0 { BusId::A } else { BusId::B },
            eq: DjIsolator::new(sample_rate),
            gain: RampedGain::new(sample_rate),
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

    /// Swap the time/pitch processor. The old processor's source Arc is
    /// retired through the deferred-destruction queue. The new processor
    /// gets the current source re-attached at the current audible position.
    /// Used by the Listening Lab to switch between bypass, varispeed, and
    /// signalsmith.
    pub fn swap_processor(&mut self, new_processor: Box<dyn TimePitchProcessor>) {
        let old_processor = std::mem::replace(&mut self.processor, new_processor);
        // Re-attach the current source to the new processor at the current
        // audible position. set_source returns the new processor's old
        // source (None for a fresh processor), which we retire.
        if let Some(buf) = &self.buffer {
            let audible_pos = old_processor.position_frames();
            let old_src = self.processor.set_source(buf.clone(), audible_pos);
            let _ = self.retire(old_src);
        }
        // The old processor Box itself drops here. It's small (no large PCM
        // buffers — those are in the source Arcs, which are handled above).
        drop(old_processor);
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

        // Apply gain (ramped)
        let g = self.gain.tick();

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
