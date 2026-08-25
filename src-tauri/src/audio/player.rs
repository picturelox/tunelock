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
        Self {
            id,
            playing: false,
            muted: false,
            soloed: false,
            source_handle: None,
            buffer: None,
            processor: default_processor(),
            retired_sources,
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
    /// If the queue is full (rare), the Arc drops immediately as a fallback.
    #[inline]
    fn retire(&self, old: Option<Arc<DecodedBuffer>>) {
        if let Some(buf) = old {
            if self.retired_sources.push(buf).is_err() {
                // Queue full — fall back to immediate drop. This is rare
                // (only if the engine thread hasn't drained in a long time)
                // and the allocation/deallocation is on the realtime thread,
                // but it's better than leaking memory.
            }
        }
    }

    pub fn launch(&mut self, handle: SourceHandle, buffer: Arc<DecodedBuffer>, start_beat: f64) {
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
        // Convert start_beat to source position
        let beat_duration_sec = 60.0 / self.bpm;
        let start_sec = self.first_beat_sec + start_beat * beat_duration_sec;
        let start_frame = start_sec * buffer.sample_rate as f64;
        // set_source returns the processor's old source; retire it to the
        // deferred-destruction queue so its Vec<f32> doesn't deallocate here.
        let old_processor_src = self.processor.set_source(buffer.clone(), start_frame);
        let old_buffer = self.buffer.take();
        self.buffer = Some(buffer);
        self.retire(old_processor_src);
        self.retire(old_buffer);
        self.playing = true;
        self.eq.reset();
    }

    pub fn load_buffer(&mut self, buffer: Arc<DecodedBuffer>) {
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
        self.retire(old_processor_src);
        self.retire(old_buffer);
        self.eq.reset();
    }

    pub fn set_source_handle(&mut self, handle: SourceHandle) {
        self.source_handle = Some(handle);
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
