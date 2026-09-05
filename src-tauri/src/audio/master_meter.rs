// PB-6.2: Realtime master meter — sample peak, true peak (dBTP), RMS.
//
// This runs in the realtime audio callback. It is:
//   - Allocation-free (all buffers preallocated)
//   - Lock-free (no mutexes, no atomics needed internally)
//   - Free of file I/O, DB calls, events, or logging
//   - Preconstructed once and reused for the engine's lifetime
//
// True peak uses the BS.1770 Annex 2 4x, 12-tap/phase coefficients also
// used by ebur128-stream 0.2.0 (src/peak.rs), checked against that analyzer.
// Its public reset clears FIR history as well as the peak, so this live
// tracker retains fixed stereo delay lines across reporting windows.
// Only full engine reconfiguration resets the FIR; no LUFS runs here.
//
// Sample peak and RMS are tracked independently with simple arithmetic,
// giving the meter three distinct measurements as required by PB-6.2.

#[cfg(test)]
use ebur128_stream::{AnalyzerBuilder, Channel, Mode};

/// BS.1770 Annex 2 polyphase FIR coefficients, ordered newest sample first.
/// Four phases of twelve taps; filter history is independent of block size.
const TRUE_PEAK_COEFFICIENTS: [[f32; 12]; 4] = [
    [0.0017089844, 0.010986328, -0.01965332, 0.033203125, -0.059448242, 0.1373291,
     0.97216797, -0.10229492, 0.047607422, -0.026611328, 0.014892578, -0.0083007813],
    [-0.029174805, 0.029296875, -0.051757812, 0.089111328, -0.16650391, 0.46508789,
     0.77978516, -0.20031738, 0.1015625, -0.05822754, 0.033081055, -0.018920898],
    [-0.018920898, 0.033081055, -0.05822754, 0.1015625, -0.20031738, 0.77978516,
     0.46508789, -0.16650391, 0.089111328, -0.051757812, 0.029296875, -0.029174805],
    [-0.0083007813, 0.014892578, -0.026611328, 0.047607422, -0.10229492, 0.97216797,
     0.1373291, -0.059448242, 0.033203125, -0.01965332, 0.010986328, 0.0017089844],
];

/// Realtime master meter for the audio engine callback.
///
/// Tracks three independent measurements:
/// - **Sample peak**: maximum absolute sample value (dBFS)
/// - **True peak**: oversampled inter-sample peak (dBTP, BS.1770 Annex 2)
/// - **RMS**: root mean square level (dBFS)
///
/// Window accumulators reset on reporting; the FIR delay lines do not.
pub struct RealtimeMasterMeter {
    // Continuous stereo FIR state, newest sample first
    delay: [[f32; 12]; 2],
    // Maximum reconstructed magnitude in the current reporting window
    block_true_peak: f64,
    // Invalid samples make true peak unavailable for this window
    invalid_input: bool,
    // Number of stereo pairs processed (for RMS, without a buffer limit)
    pair_count: usize,
    // Per-block sample peak (linear, max absolute value)
    block_sample_peak: f64,
    // Per-block sum of squares for RMS
    block_sum_sq: f64,
}

impl RealtimeMasterMeter {
    /// Create a new realtime master meter for the given sample rate.
    /// The FIR uses 4x reconstruction at every supported engine rate.
    pub fn new(_sample_rate: u32) -> Self {
        Self {
            delay: [[0.0; 12]; 2],
            block_true_peak: 0.0,
            invalid_input: false,
            pair_count: 0,
            block_sample_peak: 0.0,
            block_sum_sq: 0.0,
        }
    }

    /// Process one stereo sample pair. Called per sample in the callback.
    /// Each sample is reconstructed immediately; `finalize_block()` reports
    /// the current window without interrupting the filter history.
    #[inline]
    pub fn process(&mut self, left: f64, right: f64) {
        let sanitize = |s: f64| if s.is_finite() && (s as f32).is_finite() { s } else { 0.0 };
        let valid_left = sanitize(left);
        let valid_right = sanitize(right);
        self.invalid_input |= valid_left != left || valid_right != right;
        let (left, right) = (valid_left, valid_right);
        // Track sample peak and RMS directly (no oversampling needed)
        let abs_l = left.abs();
        let abs_r = right.abs();
        let max_abs = abs_l.max(abs_r);
        if max_abs > self.block_sample_peak {
            self.block_sample_peak = max_abs;
        }
        self.block_sum_sq += left * left + right * right;
        self.pair_count += 1;

        // Reconstruct all four phases with fixed-size continuous history
        self.block_true_peak = self.block_true_peak.max(max_abs);
        for (delay, sample) in self.delay.iter_mut().zip([left, right]) {
            delay.copy_within(..11, 1);
            delay[0] = sample as f32;
            for coefficients in &TRUE_PEAK_COEFFICIENTS {
                let mut reconstructed = 0.0f32;
                for (&coefficient, &sample) in coefficients.iter().zip(delay.iter()) {
                    reconstructed += coefficient * sample;
                }
                self.block_true_peak = self.block_true_peak.max(reconstructed.abs() as f64);
            }
        }
    }

    /// Finalize the current block: compute true peak, RMS, and sample peak.
    /// Returns (sample_peak_linear, true_peak_dbtp, rms_linear).
    /// Resets all per-block state for the next block.
    pub fn finalize_block(&mut self) -> (f64, Option<f64>, f64) {
        // Report the reconstructed maximum for all frames in the window
        let true_peak_dbtp = if self.block_true_peak > 0.0 && !self.invalid_input {
            // Keep unavailable or invalid readings out of logarithmic output,
            // including empty windows and silence
            Some(20.0 * self.block_true_peak.log10())
        } else {
            None
        };

        // Clear the peak accumulator, preserving continuous FIR history
        self.block_true_peak = 0.0;
        self.invalid_input = false;

        // Keep the delay lines intact for reconstruction across windows
        let sample_peak = self.block_sample_peak;

        // Compute RMS from every stereo pair in this reporting window
        let rms = if self.pair_count > 0 {
            (self.block_sum_sq / (2.0 * self.pair_count as f64)).sqrt()
        } else {
            0.0
        };

        // Reset per-block state
        self.pair_count = 0;
        self.block_sample_peak = 0.0;
        self.block_sum_sq = 0.0;

        (sample_peak, true_peak_dbtp, rms)
    }

    /// Reset all state (e.g., when the engine is reconfigured).
    pub fn reset(&mut self) {
        self.delay = [[0.0; 12]; 2];
        self.block_true_peak = 0.0;
        self.invalid_input = false;
        self.pair_count = 0;
        self.block_sample_peak = 0.0;
        self.block_sum_sq = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pb62_true_peak_is_independent_of_report_boundaries() {
        for sr in [44_100, 48_000, 96_000] {
            let signal: Vec<f32> = (0..4096).map(|i| {
                (std::f64::consts::FRAC_PI_2 * i as f64 + std::f64::consts::FRAC_PI_4).sin() as f32 * 0.9
            }).chain(std::iter::repeat_n(0.0, 24)).collect();
            let mut reference = AnalyzerBuilder::new().sample_rate(sr)
                .channels(&[Channel::Left, Channel::Right]).modes(Mode::TruePeak).build().unwrap();
            for &s in &signal { reference.push_interleaved(&[s, -s]).unwrap(); }
            let expected = reference.snapshot().true_peak_dbtp().unwrap();
            for chunk in [1, 7, 64, 128, 511, 2048, 8192] {
                let mut meter = RealtimeMasterMeter::new(sr);
                let mut peak = f64::NEG_INFINITY;
                for part in signal.chunks(chunk) {
                    for &s in part { meter.process(s as f64, -s as f64); }
                    peak = peak.max(meter.finalize_block().1.unwrap_or(f64::NEG_INFINITY));
                }
                assert!((peak - expected).abs() < 1e-5, "sr={sr}, chunk={chunk}: {peak} vs {expected}");
            }
        }
    }

    #[test]
    fn pb62_true_peak_captures_late_large_buffer_transient() {
        let mut meter = RealtimeMasterMeter::new(96_000);
        for _ in 0..3000 { meter.process(0.0, 0.0); }
        for i in 0..128 {
            let s = (std::f64::consts::FRAC_PI_2 * i as f64 + std::f64::consts::FRAC_PI_4).sin();
            meter.process(0.0, s);
        }
        for _ in 0..24 { meter.process(0.0, 0.0); }
        let (sp, tp, _) = meter.finalize_block();
        assert!(tp.unwrap_or(f64::NEG_INFINITY) > 20.0 * sp.log10() + 1.0);
    }

    #[test]
    fn silence_produces_zero_meter() {
        let mut meter = RealtimeMasterMeter::new(48000);
        for _ in 0..256 {
            meter.process(0.0, 0.0);
        }
        let (sp, tp, rms) = meter.finalize_block();
        assert!(sp < 1e-9, "sample peak should be ~0, got {}", sp);
        assert!(tp.is_none(), "true peak should be None for silence");
        assert!(rms < 1e-9, "rms should be ~0, got {}", rms);
    }

    #[test]
    fn full_scale_tone_detected() {
        let mut meter = RealtimeMasterMeter::new(48000);
        let omega = 2.0 * std::f32::consts::PI * 1000.0 / 48000.0;
        for i in 0..1024 {
            let s = (omega * i as f32).sin() as f64;
            meter.process(s, s);
        }
        let (sp, tp, rms) = meter.finalize_block();
        // Sample peak should be ~1.0 (full scale sine)
        assert!(sp > 0.99, "sample peak should be ~1.0, got {}", sp);
        // True peak should be present and near 0 dBTP
        assert!(tp.is_some(), "true peak should be present");
        let tp_val = tp.unwrap();
        assert!(tp_val > -1.0 && tp_val < 1.0, "true peak should be near 0 dBTP, got {}", tp_val);
        // RMS should be ~0.707 (full scale sine RMS = 1/sqrt(2))
        assert!(rms > 0.69 && rms < 0.72, "rms should be ~0.707, got {}", rms);
    }

    /// PB-6.2: True peak must exceed sample peak for inter-sample peaks.
    /// A full-scale sine near Nyquist has peaks between samples that
    /// the 4x oversampling FIR detects.
    #[test]
    fn true_peak_exceeds_sample_peak_for_inter_sample_peaks() {
        let mut meter = RealtimeMasterMeter::new(48000);
        let f = 0.4615 * 48000.0f32; // near Nyquist
        let omega = 2.0 * std::f32::consts::PI * f / 48000.0;
        let phase = std::f32::consts::PI * 0.5; // peaks between samples
        for i in 0..2048 {
            let s = (omega * i as f32 + phase).sin() as f64;
            meter.process(s, s);
        }
        let (sp, tp, rms) = meter.finalize_block();
        let sp_db = 20.0 * sp.log10();
        let tp_db = tp.expect("true peak should be present");
        assert!(
            tp_db > sp_db + 0.3,
            "true peak ({:.3} dBTP) should exceed sample peak ({:.3} dBFS) by > 0.3 dB",
            tp_db, sp_db
        );
        // RMS should still be reasonable
        assert!(rms > 0.5, "rms should be substantial, got {}", rms);
    }

    /// PB-6.2: Readings remain stable across many reporting windows.
    /// This is a numerical stability test, not an allocation measurement.
    /// The separate callback allocation audit measures Rust heap activity;
    /// this test checks peak and RMS values on a repeating signal.
    #[test]
    fn meter_is_stable_across_many_blocks() {
        let mut meter = RealtimeMasterMeter::new(48000);
        let omega = 2.0 * std::f32::consts::PI * 1000.0 / 48000.0;
        // Process 1000 blocks of 256 samples each
        for _ in 0..1000 {
            for i in 0..256 {
                let s = (omega * i as f32).sin() * 0.5;
                meter.process(s as f64, s as f64);
            }
            let (sp, tp, rms) = meter.finalize_block();
            // Values should be consistent across blocks
            assert!(sp > 0.4 && sp <= 0.5, "sample peak should be ~0.5, got {}", sp);
            assert!(tp.is_some(), "true peak should be present");
            assert!(rms > 0.3 && rms < 0.4, "rms should be ~0.35, got {}", rms);
        }
    }

    /// PB-6.2: Meter handles blocks larger than the former buffer limit
    /// without dropping samples from true-peak analysis; sample peak
    /// and RMS include every stereo pair as well.
    #[test]
    fn meter_handles_large_blocks_gracefully() {
        let mut meter = RealtimeMasterMeter::new(48000);
        // Process more than the former 2048-frame buffer capacity
        for _ in 0..8192 {
            meter.process(0.5, 0.5);
        }
        let (sp, tp, rms) = meter.finalize_block();
        // Sample peak and RMS should still be correct
        assert!((sp - 0.5).abs() < 1e-9, "sample peak should be 0.5, got {}", sp);
        assert!(rms > 0.49 && rms < 0.51, "rms should be ~0.5, got {}", rms);
        // True peak must include the sample peak as a lower bound
        // and remain finite for valid, non-silent input
        assert!(tp.unwrap() >= 20.0 * sp.log10());
    }

    #[test]
    fn pb62_meter_recovers_after_invalid_input_and_reset() {
        let mut meter = RealtimeMasterMeter::new(48_000);
        for s in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, f64::MAX] {
            meter.process(s, 0.0);
        }
        let (sp, tp, rms) = meter.finalize_block();
        assert_eq!((sp, tp, rms), (0.0, None, 0.0));
        for _ in 0..64 { meter.process(0.25, -0.5); }
        let (sp, tp, rms) = meter.finalize_block();
        assert_eq!(sp, 0.5);
        assert!(tp.unwrap().is_finite());
        assert!((rms - (0.3125f64 / 2.0).sqrt()).abs() < 1e-9);
        meter.reset();
        assert_eq!(meter.finalize_block(), (0.0, None, 0.0));
        for _ in 0..64 { meter.process(0.0, 0.0); }
        assert_eq!(meter.finalize_block(), (0.0, None, 0.0));
    }

    #[test]
    fn pb62_true_peak_falls_after_signal_and_fir_tail() {
        let mut meter = RealtimeMasterMeter::new(48_000);
        for _ in 0..128 { meter.process(0.5, -0.5); }
        assert!(meter.finalize_block().1.is_some());
        for _ in 0..12 { meter.process(0.0, 0.0); }
        meter.finalize_block();
        for _ in 0..128 { meter.process(0.0, 0.0); }
        assert_eq!(meter.finalize_block(), (0.0, None, 0.0));
    }
}
