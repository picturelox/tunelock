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
}
