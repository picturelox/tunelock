// 3-band DJ isolator EQ with complementary crossover filters.
//
// The EQ uses Linkwitz-Riley 4th-order crossovers (24 dB/oct) which sum to
// a flat response when all bands are at 0 dB. This provides true isolation
// and clean kills.
//
// Parameter changes are ramped over several milliseconds to avoid clicks
// and zipper noise. The ramp is applied per-block in the callback.
//
// All state is preallocated. No allocation in the process path.

/// Crossover frequencies (Hz).
const LOW_CROSSOVER: f64 = 200.0;   // Low/Mid boundary
const MID_CROSSOVER: f64 = 2000.0;  // Mid/High boundary

/// Ramp time for parameter changes (seconds).
const RAMP_TIME_SEC: f64 = 0.005;   // 5 ms

/// Number of biquad stages per crossover (2 for Linkwitz-Riley 4th order).
const CROSSOVER_STAGES: usize = 2;

/// A single biquad filter (Direct Form I).
#[derive(Clone, Copy, Default)]
struct Biquad {
    // Coefficients
    b0: f64, b1: f64, b2: f64,
    a1: f64, a2: f64,
    // State (per channel)
    x1: [f64; 2], x2: [f64; 2],
    y1: [f64; 2], y2: [f64; 2],
}

impl Biquad {
    fn process(&mut self, sample: f64, ch: usize) -> f64 {
        let out = self.b0 * sample
            + self.b1 * self.x1[ch]
            + self.b2 * self.x2[ch]
            - self.a1 * self.y1[ch]
            - self.a2 * self.y2[ch];

        self.x2[ch] = self.x1[ch];
        self.x1[ch] = sample;
        self.y2[ch] = self.y1[ch];
        self.y1[ch] = out;

        out
    }

    fn clear(&mut self) {
        self.x1 = [0.0; 2];
        self.x2 = [0.0; 2];
        self.y1 = [0.0; 2];
        self.y2 = [0.0; 2];
    }
}

/// Linkwitz-Riley 2nd-order (12 dB/oct) biquad — used as a stage in the
/// 4th-order crossover. Two cascaded 2nd-order stages give 4th-order (24 dB/oct).
fn lr2_lowpass(sample_rate: f64, crossover: f64) -> Biquad {
    // Linkwitz-Riley lowpass = Butterworth 2nd order squared
    let wc = 2.0 * std::f64::consts::PI * crossover / sample_rate;
    let cos_wc = wc.cos();
    let sin_wc = wc.sin();

    // Butterworth 2nd order Q = 0.7071
    let alpha = sin_wc / (2.0 * 0.7071);

    let b0 = (1.0 - cos_wc) / 2.0;
    let b1 = 1.0 - cos_wc;
    let b2 = (1.0 - cos_wc) / 2.0;
    let a0 = 1.0 + alpha;
    let a1 = -2.0 * cos_wc;
    let a2 = 1.0 - alpha;

    Biquad {
        b0: b0 / a0, b1: b1 / a0, b2: b2 / a0,
        a1: a1 / a0, a2: a2 / a0,
        x1: [0.0; 2], x2: [0.0; 2], y1: [0.0; 2], y2: [0.0; 2],
    }
}

fn lr2_highpass(sample_rate: f64, crossover: f64) -> Biquad {
    let wc = 2.0 * std::f64::consts::PI * crossover / sample_rate;
    let cos_wc = wc.cos();
    let sin_wc = wc.sin();
    let alpha = sin_wc / (2.0 * 0.7071);

    let b0 = (1.0 + cos_wc) / 2.0;
    let b1 = -(1.0 + cos_wc);
    let b2 = (1.0 + cos_wc) / 2.0;
    let a0 = 1.0 + alpha;
    let a1 = -2.0 * cos_wc;
    let a2 = 1.0 - alpha;

    Biquad {
        b0: b0 / a0, b1: b1 / a0, b2: b2 / a0,
        a1: a1 / a0, a2: a2 / a0,
        x1: [0.0; 2], x2: [0.0; 2], y1: [0.0; 2], y2: [0.0; 2],
    }
}

/// 3-band DJ isolator for one deck.
/// Uses Linkwitz-Riley 4th-order crossovers to split into low/mid/high,
/// applies per-band gain, then sums back together.
pub struct DjIsolator {
    // Low band: LR4 lowpass at LOW_CROSSOVER
    low_lp: [Biquad; CROSSOVER_STAGES],
    // High band: LR4 highpass at MID_CROSSOVER
    high_hp: [Biquad; CROSSOVER_STAGES],
    // Mid band: LR4 highpass at LOW_CROSSOVER, then LR4 lowpass at MID_CROSSOVER
    mid_hp: [Biquad; CROSSOVER_STAGES],
    mid_lp: [Biquad; CROSSOVER_STAGES],

    // Band gains (linear), with ramp targets
    low_gain_current: [f64; 2],   // [left, right] — per-channel for stereo
    mid_gain_current: [f64; 2],
    high_gain_current: [f64; 2],
    low_gain_target: f64,
    mid_gain_target: f64,
    high_gain_target: f64,
    ramp_increment: f64,          // Per-sample ramp step

    // Kill flags
    low_killed: bool,
    mid_killed: bool,
    high_killed: bool,

    sample_rate: f64,
}

impl DjIsolator {
    pub fn new(sample_rate: f64) -> Self {
        let lp_low = lr2_lowpass(sample_rate, LOW_CROSSOVER);
        let hp_mid = lr2_highpass(sample_rate, LOW_CROSSOVER);
        let lp_mid = lr2_lowpass(sample_rate, MID_CROSSOVER);
        let hp_high = lr2_highpass(sample_rate, MID_CROSSOVER);

        Self {
            low_lp: [lp_low; CROSSOVER_STAGES],
            high_hp: [hp_high; CROSSOVER_STAGES],
            mid_hp: [hp_mid; CROSSOVER_STAGES],
            mid_lp: [lp_mid; CROSSOVER_STAGES],
            low_gain_current: [1.0; 2],
            mid_gain_current: [1.0; 2],
            high_gain_current: [1.0; 2],
            low_gain_target: 1.0,
            mid_gain_target: 1.0,
            high_gain_target: 1.0,
            ramp_increment: 1.0 / (RAMP_TIME_SEC * sample_rate),
            low_killed: false,
            mid_killed: false,
            high_killed: false,
            sample_rate,
        }
    }

    pub fn set_gain_db(&mut self, band: EqBand, gain_db: f32) {
        let gain_linear = if gain_db <= -60.0 {
            0.0
        } else {
            10.0_f64.powf(gain_db as f64 / 20.0)
        };
        match band {
            EqBand::Low => self.low_gain_target = gain_linear,
            EqBand::Mid => self.mid_gain_target = gain_linear,
            EqBand::High => self.high_gain_target = gain_linear,
        }
    }

    pub fn set_kill(&mut self, band: EqBand, killed: bool) {
        match band {
            EqBand::Low => {
                self.low_killed = killed;
                self.low_gain_target = if killed { 0.0 } else { 1.0 };
            }
            EqBand::Mid => {
                self.mid_killed = killed;
                self.mid_gain_target = if killed { 0.0 } else { 1.0 };
            }
            EqBand::High => {
                self.high_killed = killed;
                self.high_gain_target = if killed { 0.0 } else { 1.0 };
            }
        }
    }

    pub fn reset(&mut self) {
        for stage in self.low_lp.iter_mut() { stage.clear(); }
        for stage in self.high_hp.iter_mut() { stage.clear(); }
        for stage in self.mid_hp.iter_mut() { stage.clear(); }
        for stage in self.mid_lp.iter_mut() { stage.clear(); }
        self.low_gain_current = [1.0; 2];
        self.mid_gain_current = [1.0; 2];
        self.high_gain_current = [1.0; 2];
        self.low_gain_target = 1.0;
        self.mid_gain_target = 1.0;
        self.high_gain_target = 1.0;
    }

    /// Process a stereo sample (interleaved L, R).
    /// Returns the processed stereo sample (L, R).
    #[inline]
    pub fn process(&mut self, left: f64, right: f64) -> (f64, f64) {
        // Ramp gains toward target
        for ch in 0..2 {
            self.low_gain_current[ch] = ramp_toward(
                self.low_gain_current[ch],
                self.low_gain_target,
                self.ramp_increment,
            );
            self.mid_gain_current[ch] = ramp_toward(
                self.mid_gain_current[ch],
                self.mid_gain_target,
                self.ramp_increment,
            );
            self.high_gain_current[ch] = ramp_toward(
                self.high_gain_current[ch],
                self.high_gain_target,
                self.ramp_increment,
            );
        }

        let samples = [left, right];
        let mut output = [0.0f64; 2];

        for ch in 0..2 {
            let s = samples[ch];

            // Low band: cascade through lowpass stages
            let mut low = s;
            for stage in self.low_lp.iter_mut() {
                low = stage.process(low, ch);
            }

            // Mid band: highpass at LOW_CROSSOVER, then lowpass at MID_CROSSOVER
            let mut mid = s;
            for stage in self.mid_hp.iter_mut() {
                mid = stage.process(mid, ch);
            }
            for stage in self.mid_lp.iter_mut() {
                mid = stage.process(mid, ch);
            }

            // High band: cascade through highpass stages
            let mut high = s;
            for stage in self.high_hp.iter_mut() {
                high = stage.process(high, ch);
            }

            // Sum with gains
            output[ch] = low * self.low_gain_current[ch]
                + mid * self.mid_gain_current[ch]
                + high * self.high_gain_current[ch];
        }

        (output[0], output[1])
    }
}

#[inline]
fn ramp_toward(current: f64, target: f64, increment: f64) -> f64 {
    if (current - target).abs() <= increment {
        target
    } else if current < target {
        current + increment
    } else {
        current - increment
    }
}

use super::command::EqBand;
