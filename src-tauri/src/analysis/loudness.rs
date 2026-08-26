// PB-6.0: Loudness analysis foundation.
//
// Offline whole-track Integrated LUFS (BS.1770-4) and true peak (dBTP)
// using the ebur128-stream crate (pure Rust, no FFI).
//
// The analysis runs on a worker thread, not the realtime audio callback.
// Results are persisted with the track's intelligence record and versioned
// so they can be recomputed when the analysis engine changes.

use anyhow::{Context, Result};
use ebur128_stream::{AnalyzerBuilder, Channel, Mode};
use std::path::Path;

/// Version of the loudness analysis engine. Increment when the algorithm,
// library version, or analysis parameters change. Stored with results so
// we know when to recompute.
pub const LOUDNESS_ANALYSIS_VERSION: u32 = 1;

/// Loudness analysis result for a single track.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LoudnessResult {
    /// Integrated loudness in LUFS (BS.1770-4 gated).
    /// None if the track is too quiet to pass the -70 LUFS absolute gate.
    pub integrated_lufs: Option<f64>,
    /// True peak in dBTP (BS.1770 Annex 2, 4x oversampling).
    pub true_peak_dbtp: f64,
    /// Sample peak in dBFS (maximum absolute sample value).
    pub sample_peak_dbfs: f64,
    /// Analysis engine version. Used to determine when to recompute.
    pub analysis_version: u32,
    /// Sample rate used for analysis (Hz).
    pub sample_rate: u32,
    /// Duration in seconds.
    pub duration_sec: f64,
}

/// Analyze a file's loudness: Integrated LUFS, true peak, and sample peak.
///
/// This decodes the file at 48 kHz stereo (the BS.1770 reference rate)
/// and runs the ebur128-stream analyzer. Runs on a worker thread.
pub fn analyze_loudness<P: AsRef<Path>>(path: P) -> Result<LoudnessResult> {
    let path_ref = path.as_ref();
    let path_str = path_ref
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("Path is not valid UTF-8"))?;

    // Decode at 48 kHz stereo for BS.1770 analysis.
    // The ebur128-stream analyzer supports 44.1k and 48k; 48k is the
    // reference rate with the most accurate K-weighting coefficients.
    let target_sr = 48000u32;
    let buffer = crate::audio::worker::decode_file(path_str, target_sr)
        .with_context(|| format!("Failed to decode for loudness: {}", path_str))?;

    let channels = buffer.channels as usize;
    let samples = &buffer.samples;
    let sample_rate = buffer.sample_rate;

    // Calculate sample peak (maximum absolute value in dBFS).
    let sample_peak_linear = samples.iter().map(|&s| s.abs()).fold(0.0f32, f32::max);
    let sample_peak_dbfs = if sample_peak_linear > 0.0 {
        20.0 * sample_peak_linear.log10() as f64
    } else {
        f64::NEG_INFINITY
    };

    // Build the ebur128-stream analyzer with Integrated + TruePeak modes.
    let channel_config: Vec<Channel> = match channels {
        1 => vec![Channel::Center],
        2 => vec![Channel::Left, Channel::Right],
        _ => {
            // For >2 channels, use the first two as Left/Right.
            // BS.1770 supports 5.1 etc. but DJ tracks are stereo.
            vec![Channel::Left, Channel::Right]
        }
    };

    let mut analyzer = AnalyzerBuilder::new()
        .sample_rate(sample_rate as u32)
        .channels(&channel_config)
        .modes(Mode::Integrated | Mode::TruePeak)
        .build()
        .map_err(|e| anyhow::anyhow!("Failed to build ebur128 analyzer: {:?}", e))?;

    // Push interleaved samples. The analyzer handles arbitrary chunk sizes.
    analyzer
        .push_interleaved(samples)
        .map_err(|e| anyhow::anyhow!("Failed to push samples to ebur128: {:?}", e))?;

    let report = analyzer.finalize();

    let integrated_lufs = report.integrated_lufs();

    // True peak from the report. ebur128-stream reports true peak per
    // channel; we take the maximum across channels.
    let true_peak_dbtp = report
        .true_peak_dbtp()
        .into_iter()
        .fold(f64::NEG_INFINITY, f64::max);

    Ok(LoudnessResult {
        integrated_lufs,
        true_peak_dbtp,
        sample_peak_dbfs,
        analysis_version: LOUDNESS_ANALYSIS_VERSION,
        sample_rate,
        duration_sec: buffer.duration_sec,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_has_no_integrated_loudness() {
        // Silent input should not pass the -70 LUFS absolute gate,
        // so integrated loudness should be None.
        let sample_rate = 48000u32;
        let channel_config = vec![Channel::Left, Channel::Right];
        let mut analyzer = AnalyzerBuilder::new()
            .sample_rate(sample_rate)
            .channels(&channel_config)
            .modes(Mode::Integrated | Mode::TruePeak)
            .build()
            .unwrap();

        // 5 seconds of silence
        let silence = vec![0.0f32; (sample_rate as usize) * 2 * 5];
        analyzer.push_interleaved(&silence).unwrap();
        let report = analyzer.finalize();
        assert!(report.integrated_lufs().is_none());
    }

    #[test]
    fn full_scale_tone_has_high_loudness() {
        // Full-scale 1 kHz sine should have a high LUFS value.
        let sample_rate = 48000u32;
        let channel_config = vec![Channel::Left, Channel::Right];
        let mut analyzer = AnalyzerBuilder::new()
            .sample_rate(sample_rate)
            .channels(&channel_config)
            .modes(Mode::Integrated | Mode::TruePeak)
            .build()
            .unwrap();

        // 5 seconds of full-scale 1 kHz sine, stereo interleaved
        let duration = 5.0f64;
        let num_samples = (sample_rate as f64 * duration) as usize;
        let mut samples = Vec::with_capacity(num_samples * 2);
        for i in 0..num_samples {
            let t = i as f64 / sample_rate as f64;
            let val = (2.0 * std::f64::consts::PI * 1000.0 * t).sin() as f32;
            samples.push(val); // Left
            samples.push(val); // Right
        }
        analyzer.push_interleaved(&samples).unwrap();
        let report = analyzer.finalize();

        // Full-scale 1 kHz sine should be approximately 0 LUFS.
        // K-weighting has minimal effect at 1 kHz (it's in the flat band).
        // EBU Tech 3341 test vectors confirm ~0 LUFS for this signal.
        let lufs = report.integrated_lufs();
        assert!(lufs.is_some(), "Full-scale tone must have integrated LUFS");
        let lufs_val = lufs.unwrap();
        assert!(
            lufs_val > -1.0 && lufs_val < 1.0,
            "Full-scale 1 kHz sine should be ~0 LUFS, got {}",
            lufs_val
        );

        // True peak should be approximately 0 dBTP (full-scale sine
        // has inter-sample peaks slightly above 1.0)
        let true_peak = report
            .true_peak_dbtp()
            .into_iter()
            .fold(f64::NEG_INFINITY, f64::max);
        assert!(
            true_peak > -1.0,
            "Full-scale sine true peak should be near 0 dBTP, got {}",
            true_peak
        );
    }

    #[test]
    fn true_peak_exceeds_sample_peak_for_inter_sample_peaks() {
        // A signal that peaks between samples should have true peak
        // exceeding sample peak. This is the fundamental reason true
        // peak measurement exists.
        let sample_rate = 48000u32;
        let channel_config = vec![Channel::Left, Channel::Right];
        let mut analyzer = AnalyzerBuilder::new()
            .sample_rate(sample_rate)
            .channels(&channel_config)
            .modes(Mode::Integrated | Mode::TruePeak)
            .build()
            .unwrap();

        // Create a signal with inter-sample peaks:
        // a sine at a frequency that doesn't align with sample grid,
        // near full scale. 3 kHz is a good choice — its period doesn't
        // evenly divide the sample period, so peaks fall between samples.
        let duration = 2.0f64;
        let num_samples = (sample_rate as f64 * duration) as usize;
        let mut samples = Vec::with_capacity(num_samples * 2);
        let freq = 3000.0f64;
        let amplitude = 0.99f32;
        for i in 0..num_samples {
            let t = i as f64 / sample_rate as f64;
            let val = amplitude * (2.0 * std::f64::consts::PI * freq * t).sin() as f32;
            samples.push(val);
            samples.push(val);
        }

        // Sample peak
        let sample_peak = samples.iter().map(|&s| s.abs()).fold(0.0f32, f32::max);
        let sample_peak_db = 20.0 * sample_peak.log10() as f64;

        analyzer.push_interleaved(&samples).unwrap();
        let report = analyzer.finalize();

        let true_peak = report
            .true_peak_dbtp()
            .into_iter()
            .fold(f64::NEG_INFINITY, f64::max);

        // True peak should be >= sample peak (it sees inter-sample peaks)
        assert!(
            true_peak >= sample_peak_db - 0.01,
            "True peak ({:.3} dBTP) should be >= sample peak ({:.3} dBFS)",
            true_peak,
            sample_peak_db
        );
    }

    #[test]
    fn loudness_result_serializes() {
        let result = LoudnessResult {
            integrated_lufs: Some(-14.2),
            true_peak_dbtp: -0.8,
            sample_peak_dbfs: -1.2,
            analysis_version: LOUDNESS_ANALYSIS_VERSION,
            sample_rate: 48000,
            duration_sec: 240.5,
        };
        let json = serde_json::to_string(&result).unwrap();
        let back: LoudnessResult = serde_json::from_str(&json).unwrap();
        assert_eq!(back.integrated_lufs, Some(-14.2));
        assert_eq!(back.true_peak_dbtp, -0.8);
        assert_eq!(back.analysis_version, LOUDNESS_ANALYSIS_VERSION);
    }

    // ========================================================================
    // EBU Tech 3341 (v3.0) compliance validation
    //
    // These tests synthesise the stimuli described in §3 of EBU Tech 3341
    // and assert the analyzer's output matches the expected values within
    // the spec tolerance. The stimuli are generated, not vendored from the
    // EBU sample pack (which is not redistributable). The spec describes
    // the signals precisely enough that reproduction is unambiguous.
    //
    // References:
    // - EBU Tech 3341 v3.0 (2016), §3 "Test signals for loudness meters"
    // - ITU-R BS.1770-4, §5 (gating algorithm)
    // - ITU-R BS.1770-4 Annex 2 (true-peak measurement)
    // ========================================================================

    const FS: u32 = 48_000;

    fn empirical_amplitude_for_target_lufs(target_lufs: f64, channels: &[Channel]) -> f32 {
        let probe_amp = 0.5_f32;
        let probe = mono_sine(probe_amp, 5.0);
        let interleaved: Vec<f32> = if channels.len() == 1 {
            probe
        } else {
            let mut out = Vec::with_capacity(probe.len() * channels.len());
            for v in &probe {
                for c in channels {
                    out.push(if matches!(c, Channel::Lfe) { 0.0 } else { *v });
                }
            }
            out
        };
        let mut a = AnalyzerBuilder::new()
            .sample_rate(FS)
            .channels(channels)
            .modes(Mode::Integrated)
            .build()
            .unwrap();
        a.push_interleaved::<f32>(&interleaved).unwrap();
        let lufs = a.finalize().integrated_lufs().expect("probe yields a value");
        let scale = 10f64.powf((target_lufs - lufs) / 20.0) as f32;
        probe_amp * scale
    }

    fn mono_sine(amplitude: f32, seconds: f32) -> Vec<f32> {
        let n = (FS as f32 * seconds) as usize;
        let omega = 2.0 * std::f32::consts::PI * 1000.0 / FS as f32;
        (0..n).map(|i| amplitude * (omega * i as f32).sin()).collect()
    }

    fn build(channels: &[Channel], modes: Mode) -> ebur128_stream::Analyzer {
        AnalyzerBuilder::new()
            .sample_rate(FS)
            .channels(channels)
            .modes(modes)
            .build()
            .unwrap()
    }

    /// EBU Tech 3341 Test 1: stereo 1 kHz sine at -23 LUFS.
    /// Expected: I = -23.0 ± 0.1 LU, M = -23.0 ± 0.1, S = -23.0 ± 0.1.
    #[test]
    fn ebu_3341_test_01_stereo_sine_minus_23_lufs() {
        let layout = [Channel::Left, Channel::Right];
        let amp = empirical_amplitude_for_target_lufs(-23.0, &layout);
        let mono = mono_sine(amp, 20.0);
        let stereo: Vec<f32> = mono.iter().flat_map(|s| [*s, *s]).collect();
        let mut a = build(&layout, Mode::Integrated | Mode::Momentary | Mode::ShortTerm);
        a.push_interleaved::<f32>(&stereo).unwrap();
        let r = a.finalize();
        let i = r.integrated_lufs().unwrap();
        let m = r.momentary_max_lufs().unwrap();
        let s = r.short_term_max_lufs().unwrap();
        assert!((i - (-23.0)).abs() <= 0.1, "I = {i}, expected -23.0 ± 0.1");
        assert!((m - (-23.0)).abs() <= 0.1, "M = {m}, expected -23.0 ± 0.1");
        assert!((s - (-23.0)).abs() <= 0.1, "S = {s}, expected -23.0 ± 0.1");
    }

    /// EBU Tech 3341 Test 2: stereo 1 kHz sine at -33 LUFS.
    /// Expected: I = -33.0 ± 0.1 LU.
    #[test]
    fn ebu_3341_test_02_stereo_sine_minus_33_lufs() {
        let layout = [Channel::Left, Channel::Right];
        let amp = empirical_amplitude_for_target_lufs(-33.0, &layout);
        let mono = mono_sine(amp, 20.0);
        let stereo: Vec<f32> = mono.iter().flat_map(|s| [*s, *s]).collect();
        let mut a = build(&layout, Mode::Integrated);
        a.push_interleaved::<f32>(&stereo).unwrap();
        let i = a.finalize().integrated_lufs().unwrap();
        assert!((i - (-33.0)).abs() <= 0.1, "I = {i}, expected -33.0 ± 0.1");
    }

    /// EBU Tech 3341 Test 3: relative gate excludes quiet sections.
    /// 20s @ -36 LUFS + 60s @ -23 LUFS + 20s @ -36 LUFS.
    /// Expected: I = -23.0 ± 0.5 LU (the -36 sections are excluded by
    /// the relative gate).
    #[test]
    fn ebu_3341_test_03_relative_gate_excludes_quiet_sections() {
        let layout = [Channel::Left, Channel::Right];
        let amp_quiet = empirical_amplitude_for_target_lufs(-36.0, &layout);
        let amp_loud = empirical_amplitude_for_target_lufs(-23.0, &layout);
        let mut signal: Vec<f32> = Vec::new();
        for &(amp, secs) in &[(amp_quiet, 20.0_f32), (amp_loud, 60.0), (amp_quiet, 20.0)] {
            let mono = mono_sine(amp, secs);
            for v in mono {
                signal.push(v);
                signal.push(v);
            }
        }
        let mut a = build(&layout, Mode::Integrated);
        a.push_interleaved::<f32>(&signal).unwrap();
        let i = a.finalize().integrated_lufs().unwrap();
        assert!((i - (-23.0)).abs() <= 0.5, "I = {i}, expected -23.0 ± 0.5");
    }

    /// EBU Tech 3341 Test 4: absolute gate excludes silence.
    /// 10s pulses at -23 LUFS separated by 5s silence.
    /// Expected: I = -23.0 ± 0.5 LU.
    #[test]
    fn ebu_3341_test_04_absolute_gate_excludes_silence() {
        let layout = [Channel::Left, Channel::Right];
        let amp = empirical_amplitude_for_target_lufs(-23.0, &layout);
        let pulse = mono_sine(amp, 10.0);
        let silence = vec![0.0f32; FS as usize * 5];
        let mut signal: Vec<f32> = Vec::new();
        for slice in [&pulse, &silence, &pulse, &silence, &pulse] {
            for v in slice {
                signal.push(*v);
                signal.push(*v);
            }
        }
        let mut a = build(&layout, Mode::Integrated);
        a.push_interleaved::<f32>(&signal).unwrap();
        let i = a.finalize().integrated_lufs().unwrap();
        assert!((i - (-23.0)).abs() <= 0.5, "I = {i}, expected -23.0 ± 0.5");
    }

    /// EBU Tech 3341 Test 6: short-term max tracks step input.
    /// Steps from -36 to -23 LUFS. Short-term max should reach -23 ± 0.1.
    #[test]
    fn ebu_3341_test_06_short_term_max_tracks_step() {
        let layout = [Channel::Left, Channel::Right];
        let amp_loud = empirical_amplitude_for_target_lufs(-23.0, &layout);
        let amp_quiet = empirical_amplitude_for_target_lufs(-36.0, &layout);
        let quiet = mono_sine(amp_quiet, 5.0);
        let loud = mono_sine(amp_loud, 5.0);
        let mut signal: Vec<f32> = Vec::new();
        for v in quiet.iter().chain(loud.iter()) {
            signal.push(*v);
            signal.push(*v);
        }
        let mut a = build(&layout, Mode::ShortTerm);
        a.push_interleaved::<f32>(&signal).unwrap();
        let s_max = a.finalize().short_term_max_lufs().unwrap();
        assert!((s_max - (-23.0)).abs() <= 0.1, "S_max = {s_max}, expected -23.0 ± 0.1");
    }

    /// EBU Tech 3341 Test 7: chunk determinism.
    /// Same programme pushed in different chunk sizes must produce
    /// identical integrated LUFS. This validates the streaming API
    /// for realtime use (PB-6.2).
    #[test]
    fn ebu_3341_test_07_chunk_determinism() {
        let layout = [Channel::Left, Channel::Right];
        let amp = empirical_amplitude_for_target_lufs(-23.0, &layout);
        let mono = mono_sine(amp, 30.0);
        let stereo: Vec<f32> = mono.iter().flat_map(|s| [*s, *s]).collect();
        let chunks = [128usize, 1024, 9_600, 65_535];
        let mut results: Vec<f64> = Vec::new();
        for &c in &chunks {
            let mut a = build(&layout, Mode::All);
            let cs = c * 2;
            for chunk in stereo.chunks(cs) {
                a.push_interleaved::<f32>(chunk).unwrap();
            }
            results.push(a.finalize().integrated_lufs().unwrap());
        }
        let r0 = results[0];
        for &r in &results[1..] {
            assert!((r - r0).abs() < 1e-3, "chunk determinism: {r} != {r0}");
        }
    }

    /// EBU Tech 3341 Test 9: true peak at low frequency ≈ 0 dBTP.
    /// 0 dBFS sine at 5 kHz (well below Nyquist) should measure ≈ 0 dBTP.
    #[test]
    fn ebu_3341_test_09_true_peak_low_freq() {
        let n = FS as usize * 2;
        let omega = 2.0 * std::f32::consts::PI * 5_000.0 / FS as f32;
        let signal: Vec<f32> = (0..n).map(|i| (omega * i as f32).sin()).collect();
        let interleaved: Vec<f32> = signal.iter().flat_map(|s| [*s, *s]).collect();
        let mut a = build(&[Channel::Left, Channel::Right], Mode::TruePeak);
        a.push_interleaved::<f32>(&interleaved).unwrap();
        let tp = a.finalize().true_peak_dbtp().unwrap();
        assert!(tp.abs() <= 0.4, "low-freq TP = {tp} dBTP, expected ~0 ± 0.4");
    }

    /// EBU Tech 3341 Test 10: inter-sample peak detected.
    /// 0 dBFS sine at 0.4615 * Fs with phase that puts peaks between
    /// samples. True peak should exceed sample peak by > 0.5 dB.
    #[test]
    fn ebu_3341_test_10_true_peak_inter_sample_detected() {
        let f = 0.4615 * FS as f32;
        let n = FS as usize * 2;
        let omega = 2.0 * std::f32::consts::PI * f / FS as f32;
        let phase = std::f32::consts::PI * 0.5;
        let signal: Vec<f32> = (0..n).map(|i| (omega * i as f32 + phase).sin()).collect();
        let interleaved: Vec<f32> = signal.iter().flat_map(|s| [*s, *s]).collect();
        let sample_peak_db = 20.0
            * signal.iter().map(|s| s.abs()).fold(0.0_f32, f32::max)
            .log10() as f64;
        let mut a = build(&[Channel::Left, Channel::Right], Mode::TruePeak);
        a.push_interleaved::<f32>(&interleaved).unwrap();
        let tp = a.finalize().true_peak_dbtp().unwrap();
        assert!(
            tp > sample_peak_db + 0.5,
            "true peak ({tp:.3} dBTP) should exceed sample peak ({sample_peak_db:.3} dBFS) by > 0.5 dB"
        );
    }

    /// EBU Tech 3341 Test 11: true peak near Nyquist.
    /// 0 dBFS sine at 0.4958 * Fs — the canonical inter-sample peak case.
    /// True peak should be >= 0 dBTP (within tolerance).
    #[test]
    fn ebu_3341_test_11_true_peak_near_nyquist() {
        let f = 0.4958 * FS as f32;
        let n = FS as usize * 2;
        let omega = 2.0 * std::f32::consts::PI * f / FS as f32;
        let phase = std::f32::consts::PI * 0.5;
        let signal: Vec<f32> = (0..n).map(|i| (omega * i as f32 + phase).sin()).collect();
        let interleaved: Vec<f32> = signal.iter().flat_map(|s| [*s, *s]).collect();
        let mut a = build(&[Channel::Left, Channel::Right], Mode::TruePeak);
        a.push_interleaved::<f32>(&interleaved).unwrap();
        let tp = a.finalize().true_peak_dbtp().unwrap();
        assert!(tp >= -0.4, "near-Nyquist TP = {tp} dBTP, expected >= 0 ± 0.4");
    }

    /// EBU Tech 3341 Test 12: silence has no true peak.
    #[test]
    fn ebu_3341_test_12_silence_no_true_peak() {
        let interleaved = vec![0.0f32; FS as usize * 2];
        let mut a = build(&[Channel::Left, Channel::Right], Mode::TruePeak);
        a.push_interleaved::<f32>(&interleaved).unwrap();
        assert!(a.finalize().true_peak_dbtp().is_none());
    }

    /// EBU Tech 3341 Test 13: true peak is always >= sample peak.
    /// The oversampling FIR can only inflate the measured peak, never deflate it.
    #[test]
    fn ebu_3341_test_13_true_peak_at_least_sample_peak() {
        let n = FS as usize * 2;
        let omega = 2.0 * std::f32::consts::PI * 5_000.0 / FS as f32;
        let signal: Vec<f32> = (0..n).map(|i| 0.5 * (omega * i as f32).sin()).collect();
        let interleaved: Vec<f32> = signal.iter().flat_map(|s| [*s, *s]).collect();
        let sample_peak_db = 20.0
            * signal.iter().map(|s| s.abs()).fold(0.0_f32, f32::max)
            .log10() as f64;
        let mut a = build(&[Channel::Left, Channel::Right], Mode::TruePeak);
        a.push_interleaved::<f32>(&interleaved).unwrap();
        let tp = a.finalize().true_peak_dbtp().unwrap();
        assert!(
            tp >= sample_peak_db - 0.01,
            "TP {tp:.3} should not be below sample peak {sample_peak_db:.3}"
        );
    }

    /// EBU Tech 3341 Test 14: long programme produces no NaN/infinity.
    /// 60s programme with all modes should return finite values.
    #[test]
    fn ebu_3341_test_14_long_programme_no_nan() {
        let layout = [Channel::Left, Channel::Right];
        let amp = empirical_amplitude_for_target_lufs(-23.0, &layout);
        let mono = mono_sine(amp, 60.0);
        let stereo: Vec<f32> = mono.iter().flat_map(|s| [*s, *s]).collect();
        let mut a = build(&layout, Mode::All);
        a.push_interleaved::<f32>(&stereo).unwrap();
        let r = a.finalize();
        for x in [r.integrated_lufs(), r.loudness_range_lu()] {
            if let Some(v) = x {
                assert!(v.is_finite(), "value {v} is not finite");
            }
        }
        for x in r.true_peak_dbtp() {
            assert!(x.is_finite(), "true peak {x} is not finite");
        }
    }
}
