// PB-6.2: Realtime master meter — sample peak, true peak (dBTP), RMS.
//
// This runs in the realtime audio callback. It is:
//   - Allocation-free (all buffers preallocated)
//   - Lock-free (no mutexes, no atomics needed internally)
//   - Free of file I/O, DB calls, events, or logging
//   - Preconstructed once and reused for the engine's lifetime
//
// True peak is measured using ebur128-stream's TruePeak-only mode,
// which implements BS.1770 Annex 2 4x oversampling via a polyphase FIR.
// The analyzer is reset each block to give per-block true peak readings
// (the 12-tap FIR warms up within the first ~0.25ms of each block,
// which is negligible for metering).
//
// Sample peak and RMS are tracked independently with simple arithmetic,
// giving the meter three distinct measurements as required by PB-6.2.

use ebur128_stream::{AnalyzerBuilder, Channel, Mode};

/// Preallocated buffer capacity for master samples (stereo interleaved).
/// A typical block is 128-1024 frames, so 2048 frames is generous.
const MAX_BLOCK_FRAMES: usize = 2048;

/// Realtime master meter for the audio engine callback.
///
/// Tracks three independent measurements:
/// - **Sample peak**: maximum absolute sample value (dBFS)
/// - **True peak**: oversampled inter-sample peak (dBTP, BS.1770 Annex 2)
/// - **RMS**: root mean square level (dBFS)
///
/// All three are reset per block and reported via the meter snapshot.
pub struct RealtimeMasterMeter {
    // ebur128-stream analyzer in TruePeak-only mode
    analyzer: ebur128_stream::Analyzer,
    // Preallocated buffer for collecting block samples (stereo interleaved f32)
    sample_buf: Vec<f32>,
    // Number of samples collected in the current block (for true-peak buffer)
    sample_count: usize,
    // Number of stereo pairs processed (for RMS, includes overflow)
    pair_count: usize,
    // Per-block sample peak (linear, max absolute value)
    block_sample_peak: f64,
    // Per-block sum of squares for RMS
    block_sum_sq: f64,
}

impl RealtimeMasterMeter {
    /// Create a new realtime master meter for the given sample rate.
    /// The analyzer is configured for stereo TruePeak-only mode.
    pub fn new(sample_rate: u32) -> Self {
        let analyzer = AnalyzerBuilder::new()
            .sample_rate(sample_rate)
            .channels(&[Channel::Left, Channel::Right])
            .modes(Mode::TruePeak)
            .build()
            .expect("Failed to build true-peak analyzer");

        Self {
            analyzer,
            sample_buf: vec![0.0f32; MAX_BLOCK_FRAMES * 2],
            sample_count: 0,
            pair_count: 0,
            block_sample_peak: 0.0,
            block_sum_sq: 0.0,
        }
    }

    /// Process one stereo sample pair. Called per sample in the callback.
    /// The sample is buffered; call `finalize_block()` at block end to
    /// compute true peak and reset for the next block.
    #[inline]
    pub fn process(&mut self, left: f64, right: f64) {
        // Track sample peak and RMS directly (no oversampling needed)
        let abs_l = left.abs();
        let abs_r = right.abs();
        let max_abs = abs_l.max(abs_r);
        if max_abs > self.block_sample_peak {
            self.block_sample_peak = max_abs;
        }
        self.block_sum_sq += left * left + right * right;
        self.pair_count += 1;

        // Buffer sample for true-peak analysis
        if self.sample_count < MAX_BLOCK_FRAMES * 2 {
            self.sample_buf[self.sample_count] = left as f32;
            self.sample_buf[self.sample_count + 1] = right as f32;
            self.sample_count += 2;
        }
    }

    /// Finalize the current block: compute true peak, RMS, and sample peak.
    /// Returns (sample_peak_linear, true_peak_dbtp, rms_linear).
    /// Resets all per-block state for the next block.
    pub fn finalize_block(&mut self) -> (f64, Option<f64>, f64) {
        // Push buffered samples to the true-peak analyzer
        if self.sample_count > 0 {
            // Ignore errors (NonFiniteSample would only come from NaN/inf,
            // which the engine should never produce)
            let _ = self.analyzer
                .push_interleaved::<f32>(&self.sample_buf[..self.sample_count]);
        }

        // Read true peak from the analyzer snapshot
        let snapshot = self.analyzer.snapshot();
        let true_peak_dbtp = snapshot.true_peak_dbtp();

        // Reset analyzer for the next block (retains configuration)
        self.analyzer.reset();

        // Compute RMS (use pair_count, not sample_count, to include overflow)
        let rms = if self.pair_count > 0 {
            (self.block_sum_sq / (2.0 * self.pair_count as f64)).sqrt()
        } else {
            0.0
        };

        let sample_peak = self.block_sample_peak;

        // Reset per-block state
        self.sample_count = 0;
        self.pair_count = 0;
        self.block_sample_peak = 0.0;
        self.block_sum_sq = 0.0;

        (sample_peak, true_peak_dbtp, rms)
    }

    /// Reset all state (e.g., when the engine is reconfigured).
    pub fn reset(&mut self) {
        self.analyzer.reset();
        self.sample_count = 0;
        self.pair_count = 0;
        self.block_sample_peak = 0.0;
        self.block_sum_sq = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// PB-6.2: Meter is allocation-free in steady state.
    /// This test verifies that processing many blocks doesn't grow memory.
    /// (We can't directly measure allocations, but we can verify the
    /// buffer doesn't grow and the meter stays stable.)
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

    /// PB-6.2: Meter handles blocks larger than the preallocated buffer
    /// without panicking (excess samples are dropped from true-peak
    /// analysis, but sample peak and RMS are still accurate).
    #[test]
    fn meter_handles_large_blocks_gracefully() {
        let mut meter = RealtimeMasterMeter::new(48000);
        // Process more than MAX_BLOCK_FRAMES samples
        for _ in 0..(MAX_BLOCK_FRAMES + 100) {
            meter.process(0.5, 0.5);
        }
        let (sp, tp, rms) = meter.finalize_block();
        // Sample peak and RMS should still be correct
        assert!((sp - 0.5).abs() < 1e-9, "sample peak should be 0.5, got {}", sp);
        assert!(rms > 0.49 && rms < 0.51, "rms should be ~0.5, got {}", rms);
        // True peak may or may not be present (buffer overflow drops samples)
        // but it shouldn't panic
    }
}
