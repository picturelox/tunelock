// TuneLock performance filter — TPT state variable filter.
//
// One filter per bus (Filter 1 on Bus A, Filter 2 on Bus B). This is
// shared-filter architecture: two musical filters on the mix buses, not
// eight redundant per-player filters.
//
// DSP: Zavalishin topology-preserving transform (TPT) SVF. Provides LP, BP,
// and HP outputs simultaneously from a single integrator pair, with
// unconditionally stable cutoff and resonance. Mode changes crossfade
// between outputs to stay click-free.
//
// The optional Drive stage is a separate nonlinear processor (tanh
// waveshaper) applied BEFORE the filter. Linear filter DSP and nonlinear
// drive DSP are deliberately separated — Clean mode never touches the
// drive path.
//
// Cutoff sweeps are logarithmic (musical) and all parameters are ramped
// per-sample to avoid zipper noise.

/// Filter mode. Bypass passes the input unaltered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterMode {
    Bypass,
    Lowpass,
    Bandpass,
    Highpass,
}

/// Default parameter ramp time (seconds).
const RAMP_SEC: f64 = 0.010; // 10 ms — slightly longer than EQ ramp for smooth sweeps
/// Mode crossfade time (seconds).
const MODE_XFADE_SEC: f64 = 0.010;

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

/// TuneLock performance filter (per bus).
pub struct TuneLockFilter {
    mode: FilterMode,
    // TPT SVF state (per channel: [ic1eq, ic2eq])
    ic1: [f64; 2],
    ic2: [f64; 2],

    // Ramped parameters
    cutoff_current: f64,      // Hz (log-domain sweeps are handled by the setter)
    cutoff_target: f64,
    resonance_current: f64,   // 0.0 - 1.0 mapped to Q
    resonance_target: f64,
    drive_current: f64,       // 0.0 = off, >0 = pre-filter drive gain
    drive_target: f64,
    ramp_inc: f64,

    // Mode crossfade: we compute LP/BP/HP outputs every sample and blend
    // between the previous mode's output mix and the new mode's.
    mode_blend: f64,          // 0.0 = old mode, 1.0 = new mode
    mode_blend_inc: f64,
    prev_mode: FilterMode,

    sample_rate: f64,
}

impl TuneLockFilter {
    pub fn new(sample_rate: f64) -> Self {
        Self {
            mode: FilterMode::Bypass,
            ic1: [0.0; 2],
            ic2: [0.0; 2],
            cutoff_current: 20000.0,
            cutoff_target: 20000.0,
            resonance_current: 0.0,
            resonance_target: 0.0,
            drive_current: 0.0,
            drive_target: 0.0,
            ramp_inc: 1.0 / (RAMP_SEC * sample_rate),
            mode_blend: 1.0,
            mode_blend_inc: 1.0 / (MODE_XFADE_SEC * sample_rate),
            prev_mode: FilterMode::Bypass,
            sample_rate,
        }
    }

    pub fn set_mode(&mut self, mode: FilterMode) {
        if mode != self.mode {
            self.prev_mode = self.mode;
            self.mode = mode;
            self.mode_blend = 0.0; // start crossfade to new mode
        }
    }

    /// Set cutoff in Hz. The caller may sweep logarithmically; this filter
    /// ramps linearly toward the target per-sample for zipper-free motion.
    pub fn set_cutoff_hz(&mut self, hz: f64) {
        self.cutoff_target = hz.clamp(20.0, self.sample_rate * 0.45);
    }

    /// Set resonance 0.0-1.0 (mapped to Q internally: 0.5 - ~20).
    pub fn set_resonance(&mut self, res: f64) {
        self.resonance_target = res.clamp(0.0, 1.0);
    }

    /// Set pre-filter drive (0.0 = off). Drive is a separate nonlinear stage.
    pub fn set_drive(&mut self, drive: f64) {
        self.drive_target = drive.clamp(0.0, 4.0);
    }

    pub fn mode(&self) -> FilterMode {
        self.mode
    }

    pub fn reset(&mut self) {
        self.ic1 = [0.0; 2];
        self.ic2 = [0.0; 2];
    }

    /// Process one stereo frame. Returns (left, right).
    #[inline]
    pub fn process(&mut self, left: f64, right: f64) -> (f64, f64) {
        // Ramp parameters
        self.cutoff_current = ramp_toward(self.cutoff_current, self.cutoff_target, self.cutoff_target.max(self.cutoff_current) * self.ramp_inc);
        self.resonance_current = ramp_toward(self.resonance_current, self.resonance_target, self.ramp_inc);
        self.drive_current = ramp_toward(self.drive_current, self.drive_target, self.ramp_inc * 4.0);
        self.mode_blend = ramp_toward(self.mode_blend, 1.0, self.mode_blend_inc);

        if self.mode == FilterMode::Bypass && self.prev_mode == FilterMode::Bypass {
            return (left, right);
        }

        let input = [left, right];
        let mut new_mode_out = [0.0f64; 2];
        let mut prev_mode_out = [0.0f64; 2];

        // TPT SVF coefficients (computed from ramped cutoff/resonance)
        let g = (std::f64::consts::PI * self.cutoff_current / self.sample_rate)
            .tan();
        let k = 2.0 - 1.9 * self.resonance_current; // resonance 0..1 → k 2..0.1
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;

        for ch in 0..2 {
            let mut x = input[ch];

            // Optional pre-filter drive (separate nonlinear stage)
            if self.drive_current > 1e-9 {
                let d = 1.0 + self.drive_current * 3.0;
                x = (x * d).tanh() / d.tanh();
            }

            // TPT SVF (Zavalishin)
            let v3 = x - self.ic2[ch];
            let v1 = a1 * self.ic1[ch] + a2 * v3;
            let v2 = self.ic2[ch] + a2 * self.ic1[ch] + a3 * v3;

            self.ic1[ch] = 2.0 * v1 - self.ic1[ch];
            self.ic2[ch] = 2.0 * v2 - self.ic2[ch];

            let lp = v2;
            let bp = v1;
            let hp = x - k * v1 - v2;

            new_mode_out[ch] = match self.mode {
                FilterMode::Bypass => input[ch],
                FilterMode::Lowpass => lp,
                FilterMode::Bandpass => bp,
                FilterMode::Highpass => hp,
            };
            prev_mode_out[ch] = match self.prev_mode {
                FilterMode::Bypass => input[ch],
                FilterMode::Lowpass => lp,
                FilterMode::Bandpass => bp,
                FilterMode::Highpass => hp,
            };
        }

        // Crossfade between previous and current mode
        let b = self.mode_blend;
        (
            prev_mode_out[0] * (1.0 - b) + new_mode_out[0] * b,
            prev_mode_out[1] * (1.0 - b) + new_mode_out[1] * b,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f64 = 44100.0;

    fn render(filter: &mut TuneLockFilter, freq: f64, frames: usize) -> Vec<f64> {
        let mut out = Vec::with_capacity(frames);
        for i in 0..frames {
            let t = i as f64 / SR;
            let s = (2.0 * std::f64::consts::PI * freq * t).sin();
            let (l, _) = filter.process(s, s);
            out.push(l);
        }
        out
    }

    fn rms(signal: &[f64]) -> f64 {
        (signal.iter().map(|s| s * s).sum::<f64>() / signal.len() as f64).sqrt()
    }

    #[test]
    fn bypass_is_transparent() {
        let mut f = TuneLockFilter::new(SR);
        let out = render(&mut f, 440.0, 4410);
        for i in 100..4410 {
            let t = i as f64 / SR;
            let expected = (2.0 * std::f64::consts::PI * 440.0 * t).sin();
            assert!(
                (out[i] - expected).abs() < 1e-12,
                "bypass must be bit-transparent at frame {i}"
            );
        }
    }

    #[test]
    fn lowpass_attenuates_highs() {
        let mut f = TuneLockFilter::new(SR);
        f.set_mode(FilterMode::Lowpass);
        f.set_cutoff_hz(500.0);
        f.set_resonance(0.0);

        // 10 kHz well above 500 Hz cutoff should be strongly attenuated.
        let out = render(&mut f, 10000.0, 4410);
        let steady = &out[1000..]; // skip ramp/settling
        let level = rms(steady);
        assert!(
            level < 0.05,
            "lowpass at 500 Hz must attenuate 10 kHz (got RMS {level})"
        );
    }

    #[test]
    fn lowpass_passes_lows() {
        let mut f = TuneLockFilter::new(SR);
        f.set_mode(FilterMode::Lowpass);
        f.set_cutoff_hz(2000.0);
        f.set_resonance(0.0);

        // 100 Hz well below 2 kHz cutoff should pass nearly intact.
        let out = render(&mut f, 100.0, 4410);
        let steady = &out[1000..];
        let level = rms(steady);
        assert!(
            level > 0.6,
            "lowpass at 2 kHz must pass 100 Hz (got RMS {level})"
        );
    }

    #[test]
    fn highpass_attenuates_lows() {
        let mut f = TuneLockFilter::new(SR);
        f.set_mode(FilterMode::Highpass);
        f.set_cutoff_hz(2000.0);
        f.set_resonance(0.0);

        let out = render(&mut f, 100.0, 4410);
        let steady = &out[1000..];
        let level = rms(steady);
        assert!(
            level < 0.05,
            "highpass at 2 kHz must attenuate 100 Hz (got RMS {level})"
        );
    }

    #[test]
    fn bandpass_isolates_band() {
        let mut f = TuneLockFilter::new(SR);
        f.set_mode(FilterMode::Bandpass);
        f.set_cutoff_hz(1000.0);
        f.set_resonance(0.7);

        // 1 kHz at center should pass; 100 Hz should be attenuated.
        let pass = render(&mut f, 1000.0, 4410);
        let pass_level = rms(&pass[1000..]);

        let mut f2 = TuneLockFilter::new(SR);
        f2.set_mode(FilterMode::Bandpass);
        f2.set_cutoff_hz(1000.0);
        f2.set_resonance(0.7);
        let stop = render(&mut f2, 100.0, 4410);
        let stop_level = rms(&stop[1000..]);

        assert!(pass_level > 0.3, "bandpass center should pass (got {pass_level})");
        assert!(
            stop_level < pass_level * 0.3,
            "bandpass must attenuate far-off frequencies (pass {pass_level}, stop {stop_level})"
        );
    }

    #[test]
    fn mode_switch_does_not_click() {
        let mut f = TuneLockFilter::new(SR);
        f.set_mode(FilterMode::Lowpass);
        f.set_cutoff_hz(2000.0);
        // Settle
        let _ = render(&mut f, 440.0, 4410);
        // Switch to highpass mid-stream
        f.set_mode(FilterMode::Highpass);
        f.set_cutoff_hz(2000.0);
        let out = render(&mut f, 440.0, 4410);
        // No sample should jump by more than a reasonable slew
        let mut max_jump = 0.0f64;
        for i in 1..out.len() {
            max_jump = max_jump.max((out[i] - out[i - 1]).abs());
        }
        assert!(
            max_jump < 0.2,
            "mode switch must not click (max sample-to-sample jump {max_jump})"
        );
    }

    #[test]
    fn drive_adds_harmonics() {
        // Drive on a pure sine should add harmonic content (higher peak-to-RMS
        // ratio changes crest factor). Without drive, crest factor of a sine
        // is sqrt(2). With drive, the waveform flattens (crest decreases).
        let mut clean = TuneLockFilter::new(SR);
        clean.set_mode(FilterMode::Lowpass);
        clean.set_cutoff_hz(20000.0);
        let clean_out = render(&mut clean, 440.0, 4410);
        let clean_steady = &clean_out[1000..];
        let clean_crest = clean_steady.iter().map(|s| s.abs()).fold(0.0f64, f64::max) / rms(clean_steady);

        let mut driven = TuneLockFilter::new(SR);
        driven.set_mode(FilterMode::Lowpass);
        driven.set_cutoff_hz(20000.0);
        driven.set_drive(2.0);
        let driven_out = render(&mut driven, 440.0, 4410);
        let driven_steady = &driven_out[1000..];
        let driven_crest = driven_steady.iter().map(|s| s.abs()).fold(0.0f64, f64::max) / rms(driven_steady);

        assert!(
            driven_crest < clean_crest - 0.01,
            "drive must flatten the waveform (clean crest {clean_crest}, driven {driven_crest})"
        );
    }

    #[test]
    fn resonance_boosts_cutoff_region() {
        // With resonance, a signal at the cutoff should be boosted vs. no resonance.
        let mut flat = TuneLockFilter::new(SR);
        flat.set_mode(FilterMode::Lowpass);
        flat.set_cutoff_hz(1000.0);
        flat.set_resonance(0.0);
        let flat_out = render(&mut flat, 1000.0, 4410);
        let flat_level = rms(&flat_out[1000..]);

        let mut reso = TuneLockFilter::new(SR);
        reso.set_mode(FilterMode::Lowpass);
        reso.set_cutoff_hz(1000.0);
        reso.set_resonance(0.9);
        let reso_out = render(&mut reso, 1000.0, 4410);
        let reso_level = rms(&reso_out[1000..]);

        assert!(
            reso_level > flat_level * 1.5,
            "resonance must boost cutoff region (flat {flat_level}, reso {reso_level})"
        );
    }
}
