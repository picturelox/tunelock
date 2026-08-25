// DSP characterization tests — verify frequency response, multi-sample-rate
// stability, and rapid control behavior for the isolator EQ and TuneLock filters.
//
// These tests use offline rendering with known test signals (sine sweeps,
// impulse responses) to verify the DSP behaves correctly across:
//   - Multiple sample rates (44.1/48/96 kHz)
//   - Filter modes (LP/BP/HP) and cutoff ranges
//   - Resonance and drive extremes
//   - Rapid parameter changes (no clicks/blown values)
//   - Bypass transparency

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::super::command::DecodedBuffer;
    use super::super::command::EqBand;
    use super::super::filter::{TuneLockFilter, FilterMode};
    use super::super::eq::DjIsolator;

    // ── Test signal generators ───────────────────────────────────────

    fn sine_at_freq(freq: f64, sr: f64, frames: usize) -> Vec<f32> {
        (0..frames)
            .map(|i| {
                let t = i as f64 / sr;
                (2.0 * std::f64::consts::PI * freq * t).sin() as f32
            })
            .collect()
    }

    fn make_buffer(samples: Vec<f32>, sr: f64) -> Arc<DecodedBuffer> {
        let frames = samples.len() / 2;
        Arc::new(DecodedBuffer {
            samples,
            sample_rate: sr as u32,
            channels: 2,
            duration_sec: frames as f64 / sr,
            bpm: None,
            beat_grid: None,
        })
    }

    fn stereo_sine(freq: f64, sr: f64, frames: usize) -> Vec<f32> {
        let mut s = Vec::with_capacity(frames * 2);
        for v in sine_at_freq(freq, sr, frames) {
            s.push(v);
            s.push(v);
        }
        s
    }

    fn rms(samples: &[f32]) -> f64 {
        if samples.is_empty() {
            return 0.0;
        }
        let sum_sq: f64 = samples.iter().map(|&s| (s as f64) * (s as f64)).sum();
        (sum_sq / samples.len() as f64).sqrt()
    }

    fn rms_db(samples: &[f32]) -> f64 {
        let r = rms(samples).max(1e-10);
        20.0 * r.log10()
    }

    // ── Filter frequency response ────────────────────────────────────

    #[test]
    fn filter_lp_attenuates_above_cutoff() {
        let sr = 44100.0;
        let cutoff = 1000.0;
        let frames = 8192;

        // 5 kHz sine — well above 1 kHz cutoff
        let input = stereo_sine(5000.0, sr, frames);
        let mut filter = TuneLockFilter::new(sr);
        filter.set_mode(FilterMode::Lowpass);
        filter.set_cutoff_hz(cutoff);
        filter.set_resonance(0.0);
        filter.set_drive(0.0);

        let mut output = vec![0.0f32; frames * 2];
        for i in 0..frames {
            let (l, r) = filter.process(input[i * 2] as f64, input[i * 2 + 1] as f64);
            output[i * 2] = l as f32;
            output[i * 2 + 1] = r as f32;
        }

        // Skip warm-up (first 256 frames)
        let steady = &output[512..];
        let input_steady = &input[512..];

        let attenuation_db = rms_db(input_steady) - rms_db(steady);
        assert!(
            attenuation_db > 10.0,
            "LP at 1kHz should attenuate 5kHz by >10dB (got {attenuation_db:.1}dB)"
        );
    }

    #[test]
    fn filter_lp_passes_below_cutoff() {
        let sr = 44100.0;
        let cutoff = 2000.0;
        let frames = 8192;

        // 100 Hz sine — well below 2 kHz cutoff
        let input = stereo_sine(100.0, sr, frames);
        let mut filter = TuneLockFilter::new(sr);
        filter.set_mode(FilterMode::Lowpass);
        filter.set_cutoff_hz(cutoff);
        filter.set_resonance(0.0);

        let mut output = vec![0.0f32; frames * 2];
        for i in 0..frames {
            let (l, r) = filter.process(input[i * 2] as f64, input[i * 2 + 1] as f64);
            output[i * 2] = l as f32;
            output[i * 2 + 1] = r as f32;
        }

        let steady = &output[512..];
        let input_steady = &input[512..];

        let attenuation_db = rms_db(input_steady) - rms_db(steady);
        assert!(
            attenuation_db < 3.0,
            "LP at 2kHz should pass 100Hz with <3dB loss (got {attenuation_db:.1}dB)"
        );
    }

    #[test]
    fn filter_hp_attenuates_below_cutoff() {
        let sr = 44100.0;
        let cutoff = 2000.0;
        let frames = 8192;

        // 100 Hz sine — well below 2 kHz cutoff
        let input = stereo_sine(100.0, sr, frames);
        let mut filter = TuneLockFilter::new(sr);
        filter.set_mode(FilterMode::Highpass);
        filter.set_cutoff_hz(cutoff);
        filter.set_resonance(0.0);

        let mut output = vec![0.0f32; frames * 2];
        for i in 0..frames {
            let (l, r) = filter.process(input[i * 2] as f64, input[i * 2 + 1] as f64);
            output[i * 2] = l as f32;
            output[i * 2 + 1] = r as f32;
        }

        let steady = &output[512..];
        let input_steady = &input[512..];

        let attenuation_db = rms_db(input_steady) - rms_db(steady);
        assert!(
            attenuation_db > 10.0,
            "HP at 2kHz should attenuate 100Hz by >10dB (got {attenuation_db:.1}dB)"
        );
    }

    #[test]
    fn filter_bp_passes_midband() {
        let sr = 44100.0;
        let cutoff = 1000.0;
        let frames = 8192;

        // 1 kHz sine — at the bandpass center
        let input = stereo_sine(1000.0, sr, frames);
        let mut filter = TuneLockFilter::new(sr);
        filter.set_mode(FilterMode::Bandpass);
        filter.set_cutoff_hz(cutoff);
        filter.set_resonance(0.3);  // moderate Q for bandpass

        let mut output = vec![0.0f32; frames * 2];
        for i in 0..frames {
            let (l, r) = filter.process(input[i * 2] as f64, input[i * 2 + 1] as f64);
            output[i * 2] = l as f32;
            output[i * 2 + 1] = r as f32;
        }

        let steady = &output[512..];
        // BP should pass the center frequency (some loss expected, but not >20dB)
        let level_db = rms_db(steady);
        assert!(
            level_db > -30.0,
            "BP at 1kHz center should pass 1kHz signal (level {level_db:.1}dB)"
        );
    }

    // ── Bypass transparency ──────────────────────────────────────────

    #[test]
    fn filter_bypass_is_transparent() {
        let sr = 44100.0;
        let frames = 4096;
        let input = stereo_sine(440.0, sr, frames);

        let mut filter = TuneLockFilter::new(sr);
        filter.set_mode(FilterMode::Bypass);

        let mut max_diff = 0.0f64;
        for i in 0..frames {
            let (l, r) = filter.process(input[i * 2] as f64, input[i * 2 + 1] as f64);
            max_diff = max_diff.max((l - input[i * 2] as f64).abs());
            max_diff = max_diff.max((r - input[i * 2 + 1] as f64).abs());
        }
        assert!(
            max_diff < 1e-9,
            "Bypass must be transparent (max diff {max_diff})"
        );
    }

    // ── Resonance stability ──────────────────────────────────────────

    #[test]
    fn filter_max_resonance_does_not_explode() {
        let sr = 44100.0;
        let frames = 8192;
        let input = stereo_sine(440.0, sr, frames);

        let mut filter = TuneLockFilter::new(sr);
        filter.set_mode(FilterMode::Lowpass);
        filter.set_cutoff_hz(1000.0);
        filter.set_resonance(1.0);  // maximum resonance

        let mut max_abs = 0.0f64;
        for i in 0..frames {
            let (l, r) = filter.process(input[i * 2] as f64, input[i * 2 + 1] as f64);
            max_abs = max_abs.max(l.abs()).max(r.abs());
        }
        // High resonance can boost the signal, but it must not explode
        assert!(
            max_abs < 100.0,
            "Max resonance must not cause explosion (max {max_abs})"
        );
        assert!(
            max_abs.is_finite(),
            "Max resonance must not produce NaN/Inf"
        );
    }

    // ── Drive stability ──────────────────────────────────────────────

    #[test]
    fn filter_max_drive_does_not_explode() {
        let sr = 44100.0;
        let frames = 4096;
        let input = stereo_sine(440.0, sr, frames);

        let mut filter = TuneLockFilter::new(sr);
        filter.set_mode(FilterMode::Lowpass);
        filter.set_cutoff_hz(5000.0);
        filter.set_drive(1.0);  // maximum drive

        let mut max_abs = 0.0f64;
        for i in 0..frames {
            let (l, r) = filter.process(input[i * 2] as f64, input[i * 2 + 1] as f64);
            max_abs = max_abs.max(l.abs()).max(r.abs());
        }
        // tanh drive saturates, so output should be bounded
        assert!(
            max_abs < 2.0,
            "Max drive (tanh) should saturate, not explode (max {max_abs})"
        );
    }

    // ── Multi-sample-rate stability ──────────────────────────────────

    #[test]
    fn filter_stable_at_44100_hz() {
        let sr = 44100.0;
        let frames = 4096;
        let input = stereo_sine(440.0, sr, frames);

        let mut filter = TuneLockFilter::new(sr);
        filter.set_mode(FilterMode::Lowpass);
        filter.set_cutoff_hz(2000.0);

        let mut max_abs = 0.0f64;
        let mut has_nan = false;
        for i in 0..frames {
            let (l, r) = filter.process(input[i * 2] as f64, input[i * 2 + 1] as f64);
            if !l.is_finite() || !r.is_finite() {
                has_nan = true;
                break;
            }
            max_abs = max_abs.max(l.abs()).max(r.abs());
        }
        assert!(!has_nan, "No NaN/Inf at 44.1kHz");
        assert!(max_abs < 10.0, "Stable at 44.1kHz (max {max_abs})");
    }

    #[test]
    fn filter_stable_at_48000_hz() {
        let sr = 48000.0;
        let frames = 4096;
        let input = stereo_sine(440.0, sr, frames);

        let mut filter = TuneLockFilter::new(sr);
        filter.set_mode(FilterMode::Lowpass);
        filter.set_cutoff_hz(2000.0);

        let mut max_abs = 0.0f64;
        let mut has_nan = false;
        for i in 0..frames {
            let (l, r) = filter.process(input[i * 2] as f64, input[i * 2 + 1] as f64);
            if !l.is_finite() || !r.is_finite() {
                has_nan = true;
                break;
            }
            max_abs = max_abs.max(l.abs()).max(r.abs());
        }
        assert!(!has_nan, "No NaN/Inf at 48kHz");
        assert!(max_abs < 10.0, "Stable at 48kHz (max {max_abs})");
    }

    #[test]
    fn filter_stable_at_96000_hz() {
        let sr = 96000.0;
        let frames = 4096;
        let input = stereo_sine(440.0, sr, frames);

        let mut filter = TuneLockFilter::new(sr);
        filter.set_mode(FilterMode::Lowpass);
        filter.set_cutoff_hz(2000.0);

        let mut max_abs = 0.0f64;
        let mut has_nan = false;
        for i in 0..frames {
            let (l, r) = filter.process(input[i * 2] as f64, input[i * 2 + 1] as f64);
            if !l.is_finite() || !r.is_finite() {
                has_nan = true;
                break;
            }
            max_abs = max_abs.max(l.abs()).max(r.abs());
        }
        assert!(!has_nan, "No NaN/Inf at 96kHz");
        assert!(max_abs < 10.0, "Stable at 96kHz (max {max_abs})");
    }

    // ── Rapid parameter changes (no clicks) ──────────────────────────

    #[test]
    fn filter_rapid_cutoff_change_no_explosion() {
        let sr = 44100.0;
        let frames = 8192;
        let input = stereo_sine(440.0, sr, frames);

        let mut filter = TuneLockFilter::new(sr);
        filter.set_mode(FilterMode::Lowpass);
        filter.set_resonance(0.5);

        let mut max_abs = 0.0f64;
        for i in 0..frames {
            // Sweep cutoff rapidly: alternate between 200 Hz and 10 kHz
            // every 64 frames
            let cutoff = if (i / 64) % 2 == 0 { 200.0 } else { 10000.0 };
            filter.set_cutoff_hz(cutoff);
            let (l, r) = filter.process(input[i * 2] as f64, input[i * 2 + 1] as f64);
            max_abs = max_abs.max(l.abs()).max(r.abs());
        }
        assert!(
            max_abs < 100.0 && max_abs.is_finite(),
            "Rapid cutoff changes must not cause explosion (max {max_abs})"
        );
    }

    #[test]
    fn filter_rapid_mode_change_no_explosion() {
        let sr = 44100.0;
        let frames = 4096;
        let input = stereo_sine(440.0, sr, frames);

        let mut filter = TuneLockFilter::new(sr);
        filter.set_cutoff_hz(1000.0);
        filter.set_resonance(0.3);

        let mut max_abs = 0.0f64;
        for i in 0..frames {
            // Switch mode every 128 frames
            let mode = match (i / 128) % 4 {
                0 => FilterMode::Lowpass,
                1 => FilterMode::Bandpass,
                2 => FilterMode::Highpass,
                _ => FilterMode::Bypass,
            };
            filter.set_mode(mode);
            let (l, r) = filter.process(input[i * 2] as f64, input[i * 2 + 1] as f64);
            max_abs = max_abs.max(l.abs()).max(r.abs());
        }
        assert!(
            max_abs < 100.0 && max_abs.is_finite(),
            "Rapid mode changes must not cause explosion (max {max_abs})"
        );
    }

    // ── Isolator EQ characterization ────────────────────────────────

    #[test]
    fn isolator_bypass_preserves_amplitude() {
        // LR4 crossovers have phase shifts, so sample-by-sample comparison
        // is not meaningful. Instead, verify that the RMS level is preserved
        // (the amplitude response should be flat at unity gains).
        let sr = 44100.0;
        let frames = 8192;
        let mut isolator = DjIsolator::new(sr);
        // Default: no kills, all gains at 0 dB

        let input = stereo_sine(440.0, sr, frames);
        let mut output = vec![0.0f32; frames * 2];
        for i in 0..frames {
            let (l, r) = isolator.process(input[i * 2] as f64, input[i * 2 + 1] as f64);
            output[i * 2] = l as f32;
            output[i * 2 + 1] = r as f32;
        }

        // Skip warm-up
        let input_steady = &input[1024..];
        let output_steady = &output[1024..];

        let input_rms = rms(input_steady);
        let output_rms = rms(output_steady);
        let ratio_db = 20.0 * (output_rms / input_rms.max(1e-10)).log10();

        // At 440 Hz (between the 200 Hz and 2 kHz crossovers), the amplitude
        // should be preserved within ±3 dB (LR4 has some ripple)
        assert!(
            ratio_db.abs() < 3.0,
            "Isolator at unity should preserve amplitude within ±3dB (got {ratio_db:.1}dB)"
        );
    }

    #[test]
    fn isolator_kill_high_attenuates_treble() {
        let sr = 44100.0;
        let frames = 8192;
        let mut isolator = DjIsolator::new(sr);
        isolator.set_kill(EqBand::High, true);

        // 5 kHz sine — in the high band
        let input = stereo_sine(5000.0, sr, frames);
        let mut output = vec![0.0f32; frames * 2];
        for i in 0..frames {
            let (l, r) = isolator.process(input[i * 2] as f64, input[i * 2 + 1] as f64);
            output[i * 2] = l as f32;
            output[i * 2 + 1] = r as f32;
        }

        let steady = &output[512..];
        let input_steady = &input[512..];
        let attenuation_db = rms_db(input_steady) - rms_db(steady);
        assert!(
            attenuation_db > 20.0,
            "Kill high should attenuate 5kHz by >20dB (got {attenuation_db:.1}dB)"
        );
    }

    #[test]
    fn isolator_kill_low_attenuates_bass() {
        let sr = 44100.0;
        let frames = 8192;
        let mut isolator = DjIsolator::new(sr);
        isolator.set_kill(EqBand::Low, true);

        // 50 Hz sine — in the low band
        let input = stereo_sine(50.0, sr, frames);
        let mut output = vec![0.0f32; frames * 2];
        for i in 0..frames {
            let (l, r) = isolator.process(input[i * 2] as f64, input[i * 2 + 1] as f64);
            output[i * 2] = l as f32;
            output[i * 2 + 1] = r as f32;
        }

        let steady = &output[512..];
        let input_steady = &input[512..];
        let attenuation_db = rms_db(input_steady) - rms_db(steady);
        assert!(
            attenuation_db > 20.0,
            "Kill low should attenuate 50Hz by >20dB (got {attenuation_db:.1}dB)"
        );
    }

    #[test]
    fn isolator_stable_at_all_sample_rates() {
        for &sr in &[44100.0, 48000.0, 96000.0] {
            let frames = 2048;
            let mut isolator = DjIsolator::new(sr);
            let input = stereo_sine(440.0, sr, frames);

            let mut max_abs = 0.0f64;
            let mut has_nan = false;
            for i in 0..frames {
                let (l, r) = isolator.process(input[i * 2] as f64, input[i * 2 + 1] as f64);
                if !l.is_finite() || !r.is_finite() {
                    has_nan = true;
                    break;
                }
                max_abs = max_abs.max(l.abs()).max(r.abs());
            }
            assert!(!has_nan, "Isolator stable at {sr}Hz: no NaN/Inf");
            assert!(max_abs < 10.0, "Isolator stable at {sr}Hz (max {max_abs})");
        }
    }
}
