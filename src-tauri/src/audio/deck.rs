// Deck — real-time playback state for one deck.
//
// The deck reads from an rtrb ring buffer (filled by a worker thread) and
// applies gain, EQ, and crossfade. All state is preallocated — no allocation
// in the process path.
//
// The deck tracks its position in both source samples and output frames.
// Source position is used for seeking and looping; output frame is used
// for scheduling.

use super::command::{DeckId, DecodedBuffer, EqBand, LoopRegion};
use super::eq::DjIsolator;
use super::ring_buffer::RingBufferConsumer;

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
            ramp_increment: 1.0 / (0.005 * sample_rate), // 5ms ramp
        }
    }

    fn set_target(&mut self, target: f64) {
        self.target = target;
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

    fn reset(&mut self) {
        self.current = 1.0;
        self.target = 1.0;
    }
}

pub struct Deck {
    pub id: DeckId,
    pub playing: bool,

    // Source buffer (set by LoadDeck command)
    buffer: Option<DecodedBuffer>,

    // Position in the source buffer (in samples, per channel)
    source_position: f64,  // Fractional for resampling

    // Ring buffer consumer (for streaming from worker thread)
    ring_consumer: Option<RingBufferConsumer>,

    // EQ
    eq: DjIsolator,

    // Gains
    deck_gain: RampedGain,
    crossfade_gain: RampedGain,

    // Tempo (playback rate — 1.0 = original)
    tempo: f32,

    // Loop region (in beats, relative to beat grid)
    loop_region: Option<LoopRegion>,

    // Beat grid for this deck
    bpm: f64,
    first_beat_ms: f64,

    // Metering (per-deck, this block)
    block_sum_sq: [f64; 2],
    block_peak: [f64; 2],
    clip: [bool; 2],

    sample_rate: f64,
}

impl Deck {
    pub fn new(id: DeckId, sample_rate: f64) -> Self {
        Self {
            id,
            playing: false,
            buffer: None,
            source_position: 0.0,
            ring_consumer: None,
            eq: DjIsolator::new(sample_rate),
            deck_gain: RampedGain::new(sample_rate),
            crossfade_gain: RampedGain::new(0.707), // cos(45°) for center
            tempo: 1.0,
            loop_region: None,
            bpm: 120.0,
            first_beat_ms: 0.0,
            block_sum_sq: [0.0; 2],
            block_peak: [0.0; 2],
            clip: [false; 2],
            sample_rate,
        }
    }

    pub fn load_buffer(&mut self, buffer: DecodedBuffer) {
        self.bpm = buffer.bpm.unwrap_or(120.0);
        self.buffer = Some(buffer);
        self.source_position = 0.0;
        self.eq.reset();
    }

    pub fn set_ring_consumer(&mut self, consumer: RingBufferConsumer) {
        self.ring_consumer = Some(consumer);
    }

    pub fn play(&mut self) {
        self.playing = true;
    }

    pub fn pause(&mut self) {
        self.playing = false;
    }

    pub fn stop(&mut self) {
        self.playing = false;
        self.source_position = 0.0;
        self.eq.reset();
    }

    pub fn seek(&mut self, position_sec: f64) {
        if let Some(buf) = &self.buffer {
            let sample = position_sec * buf.sample_rate as f64;
            let max = (buf.samples.len() / buf.channels as usize) as f64;
            self.source_position = sample.min(max).max(0.0);
        }
        self.eq.reset();
    }

    pub fn set_tempo(&mut self, rate: f32) {
        self.tempo = rate;
    }

    pub fn set_gain(&mut self, gain: f32) {
        self.deck_gain.set_target(gain as f64);
    }

    pub fn set_crossfade_gain(&mut self, gain: f32) {
        self.crossfade_gain.set_target(gain as f64);
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
            self.source_position / buf.sample_rate as f64
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

    /// Reset block metering (called at start of each output block).
    pub fn reset_block_meters(&mut self) {
        self.block_sum_sq = [0.0; 2];
        self.block_peak = [0.0; 2];
        // Don't reset clip flags — they're cleared by the UI
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

    /// Process one stereo sample pair from this deck.
    /// Returns (left, right) output. If not playing or no buffer, returns (0, 0).
    #[inline]
    pub fn process_sample(&mut self) -> (f64, f64) {
        if !self.playing {
            return (0.0, 0.0);
        }

        let buf = match &self.buffer {
            Some(b) => b,
            None => return (0.0, 0.0),
        };

        let channels = buf.channels as usize;
        let total_frames = buf.samples.len() / channels;
        let pos = self.source_position as usize;

        // Check for end of buffer
        if pos >= total_frames {
            self.playing = false;
            return (0.0, 0.0);
        }

        // Read source sample (stereo — take first two channels)
        let src_l = buf.samples[pos * channels] as f64;
        let src_r = if channels >= 2 {
            buf.samples[pos * channels + 1] as f64
        } else {
            src_l
        };

        // Apply EQ
        let (eq_l, eq_r) = self.eq.process(src_l, src_r);

        // Apply gains (ramped)
        let deck_g = self.deck_gain.tick();
        let xf_g = self.crossfade_gain.tick();
        let total_gain = deck_g * xf_g;

        let out_l = eq_l * total_gain;
        let out_r = eq_r * total_gain;

        // Update meters
        self.block_sum_sq[0] += out_l * out_l;
        self.block_sum_sq[1] += out_r * out_r;
        let abs_l = out_l.abs();
        let abs_r = out_r.abs();
        if abs_l > self.block_peak[0] { self.block_peak[0] = abs_l; }
        if abs_r > self.block_peak[1] { self.block_peak[1] = abs_r; }
        if abs_l >= 1.0 { self.clip[0] = true; }
        if abs_r >= 1.0 { self.clip[1] = true; }

        // Advance source position by tempo
        self.source_position += self.tempo as f64;

        // Handle looping
        if let Some(loop_region) = &self.loop_region {
            let beat_duration_sec = 60.0 / self.bpm;
            let loop_start_sec = self.first_beat_ms / 1000.0
                + loop_region.start_beat * beat_duration_sec;
            let loop_end_sec = loop_start_sec + loop_region.length_beats * beat_duration_sec;
            let loop_end_sample = loop_end_sec * buf.sample_rate as f64;

            if self.source_position >= loop_end_sample {
                let loop_start_sample = loop_start_sec * buf.sample_rate as f64;
                self.source_position = loop_start_sample;
                self.eq.reset();
            }
        }

        (out_l, out_r)
    }
}
