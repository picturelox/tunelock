// Deck — a single audio playback unit with gain, EQ, and tempo control.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

pub struct AudioBuffer {
    pub samples: Vec<f32>,   // Interleaved
    pub sample_rate: u32,
    pub channels: usize,
    pub duration: f64,
}

pub struct Deck {
    pub buffer: AudioBuffer,
    pub playing: AtomicBool,
    pub position_samples: AtomicU64, // Position in the source buffer (not output)
    pub gain: Mutex<f32>,
    pub playback_rate: Mutex<f32>,
    // 3-band EQ (simple biquad filters)
    pub low_gain: Mutex<f32>,  // dB
    pub mid_gain: Mutex<f32>,
    pub high_gain: Mutex<f32>,
    // Crossfade gain (set by mixer)
    pub crossfade_gain: Mutex<f32>,
    // Resampling state
    frac_pos: Mutex<f64>,
    // EQ filter state
    eq_state: Mutex<EqState>,
}

#[derive(Default, Clone)]
struct EqState {
    // Low shelf
    low_x1: [f64; 2],
    low_y1: [f64; 2],
    low_x2: [f64; 2],
    low_y2: [f64; 2],
    // Mid peaking
    mid_x1: [f64; 2],
    mid_y1: [f64; 2],
    mid_x2: [f64; 2],
    mid_y2: [f64; 2],
    // High shelf
    high_x1: [f64; 2],
    high_y1: [f64; 2],
    high_x2: [f64; 2],
    high_y2: [f64; 2],
}

impl Deck {
    pub fn new(buffer: AudioBuffer) -> Self {
        Self {
            buffer,
            playing: AtomicBool::new(false),
            position_samples: AtomicU64::new(0),
            gain: Mutex::new(1.0),
            playback_rate: Mutex::new(1.0),
            low_gain: Mutex::new(0.0),
            mid_gain: Mutex::new(0.0),
            high_gain: Mutex::new(0.0),
            crossfade_gain: Mutex::new(0.707), // cos(45°) for center crossfade
            frac_pos: Mutex::new(0.0),
            eq_state: Mutex::new(EqState::default()),
        }
    }

    pub fn play(&self) {
        self.playing.store(true, Ordering::Relaxed);
    }

    pub fn pause(&self) {
        self.playing.store(false, Ordering::Relaxed);
    }

    pub fn stop(&self) {
        self.playing.store(false, Ordering::Relaxed);
        self.position_samples.store(0, Ordering::Relaxed);
        *self.frac_pos.lock().unwrap() = 0.0;
    }

    pub fn seek(&self, seconds: f64) {
        let sample = (seconds * self.buffer.sample_rate as f64) as u64;
        let max = (self.buffer.samples.len() / self.buffer.channels) as u64;
        self.position_samples.store(sample.min(max), Ordering::Relaxed);
        *self.frac_pos.lock().unwrap() = 0.0;
    }

    pub fn get_position(&self) -> f64 {
        self.position_samples.load(Ordering::Relaxed) as f64 / self.buffer.sample_rate as f64
    }

    pub fn set_gain(&self, gain: f32) {
        *self.gain.lock().unwrap() = gain;
    }

    pub fn set_playback_rate(&self, rate: f32) {
        *self.playback_rate.lock().unwrap() = rate;
    }

    pub fn get_playback_rate(&self) -> f32 {
        *self.playback_rate.lock().unwrap()
    }

    pub fn set_crossfade_gain(&self, gain: f32) {
        *self.crossfade_gain.lock().unwrap() = gain;
    }

    /// Process audio: read from buffer at current position, apply EQ and gain,
    /// write into the output buffer.
    pub fn process(&self, output: &mut [f32], output_channels: usize) {
        if !self.playing.load(Ordering::Relaxed) {
            return;
        }

        let gain = *self.gain.lock().unwrap();
        let crossfade = *self.crossfade_gain.lock().unwrap();
        let rate = *self.playback_rate.lock().unwrap();
        let total_gain = gain * crossfade;

        let src_channels = self.buffer.channels;
        let src_samples = &self.buffer.samples;
        let total_src_samples = src_samples.len() / src_channels;

        let mut pos = self.position_samples.load(Ordering::Relaxed) as f64;
        let mut frac = *self.frac_pos.lock().unwrap();
        let mut eq_state = self.eq_state.lock().unwrap();

        let low_gain_db = *self.low_gain.lock().unwrap() as f64;
        let mid_gain_db = *self.mid_gain.lock().unwrap() as f64;
        let high_gain_db = *self.high_gain.lock().unwrap() as f64;

        for frame in output.chunks_mut(output_channels) {
            if pos >= total_src_samples as f64 {
                // End of buffer — stop
                self.playing.store(false, Ordering::Relaxed);
                break;
            }

            // Nearest-neighbor resampling (prototype quality)
            let src_idx = pos as usize * src_channels;

            for ch in 0..output_channels.min(src_channels) {
                let sample = src_samples[src_idx + ch] as f64;
                // Apply EQ
                let eqed = apply_eq(
                    sample,
                    ch,
                    &mut eq_state,
                    low_gain_db,
                    mid_gain_db,
                    high_gain_db,
                    self.buffer.sample_rate as f64,
                );
                // Apply gain
                frame[ch] += (eqed * total_gain as f64) as f32;
            }

            // Advance position
            frac += rate as f64;
            while frac >= 1.0 {
                frac -= 1.0;
                pos += 1.0;
            }
        }

        self.position_samples.store(pos as u64, Ordering::Relaxed);
        *self.frac_pos.lock().unwrap() = frac;
    }
}

/// Simple 3-band EQ: low shelf at 320Hz, mid peaking at 1kHz, high shelf at 3.2kHz.
fn apply_eq(
    sample: f64,
    ch: usize,
    state: &mut EqState,
    low_db: f64,
    mid_db: f64,
    high_db: f64,
    _sample_rate: f64,
) -> f64 {
    // For the prototype, use simple one-pole shelving filters
    // Low shelf
    let low_gain = 10.0_f64.powf(low_db / 20.0);
    let low_alpha = 0.1; // Simple smoothing
    state.low_x1[ch] = state.low_x1[ch] * (1.0 - low_alpha) + sample * low_alpha;
    let low_out = state.low_x1[ch] * low_gain + (sample - state.low_x1[ch]);

    // Mid peaking (simplified)
    let mid_gain = 10.0_f64.powf(mid_db / 20.0);
    state.mid_x1[ch] = state.mid_x1[ch] * 0.99 + low_out * 0.01;
    let mid_out = state.mid_x1[ch] * mid_gain + (low_out - state.mid_x1[ch]);

    // High shelf
    let high_gain = 10.0_f64.powf(high_db / 20.0);
    let high_alpha = 0.05;
    state.high_x1[ch] = state.high_x1[ch] * (1.0 - high_alpha) + mid_out * high_alpha;
    let high_out = (mid_out - state.high_x1[ch]) * high_gain + state.high_x1[ch];

    high_out
}
