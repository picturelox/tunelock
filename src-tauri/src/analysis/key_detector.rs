//! Key detection — Phase 6 pipeline:
//! 1. Decode samples (done upstream).
//! 2. Compute magnitude spectrogram.
//! 3. HPSS → keep harmonic component.
//! 4. Build chromagram from harmonic spectrogram.
//! 5. Temporal segment voting across 3 classical profiles.
//! 6. Return standard + Camelot + confidence.
//!
//! The CNN stage (CQT/Mel/HPCP via ONNX) is stubbed — it will plug into
//! `ensemble::classical_ensemble`'s output via a separate CNN vote once
//! pretrained models are available.

use anyhow::Result;
use ndarray::Array2;

use super::chromagram::{chromagram72_from_spec, chromagram_from_spec, compute_spectrogram};
use super::ensemble::{format_key, temporal_vote_ranked_dual_soft, temporal_vote_ranked_soft_12, temporal_vote_ranked_soft_72, ProfileWeights, RankedCandidate};
use super::hpss::hpss;
use super::{HPSS_KERNEL, MAX_ANALYSIS_SECONDS, SAMPLE_RATE};

/// If `samples` is longer than `MAX_ANALYSIS_SECONDS`, return a centered slice
/// of that length. Otherwise return `samples` unchanged.
///
/// Why: at 11 kHz, 90 s = ~990k samples -> ~1900 STFT frames -> HPSS finishes
/// in well under a second. Long tracks (5-10 min) have plenty of redundant
/// tonal content; the middle slice is usually where the song has settled
/// into its key, away from intros / outros / breakdowns. The 8-segment
/// ensemble still spans that 90 s, so confidence stays strong.
fn window_for_analysis(samples: &[f32]) -> &[f32] {
    let max_samples = MAX_ANALYSIS_SECONDS * SAMPLE_RATE;
    if samples.len() <= max_samples {
        return samples;
    }
    let start = (samples.len() - max_samples) / 2;
    &samples[start..start + max_samples]
}

pub struct KeyResult {
    pub key_standard: String,
    pub key_camelot: String,
    pub confidence: f64,
}

/// Ablation configuration for controlled experiments.
/// Each field toggles or modifies one engine component so we can
/// measure its individual contribution to accuracy.
#[derive(Debug, Clone)]
pub struct AblationConfig {
    /// If true, skip HPSS and use the raw spectrogram for chroma.
    pub no_hpss: bool,
    /// HPSS kernel size (default 9). Only used if no_hpss is false.
    pub hpss_kernel: usize,
    /// If true, use only the 12-bin path (Krumhansl + Temperley + Sha'ath-12).
    pub chroma12_only: bool,
    /// If true, use only the 72-band path (Sha'ath-72).
    pub chroma72_only: bool,
    /// Override the analysis window in seconds (default 180).
    pub analysis_seconds: Option<usize>,
}

impl Default for AblationConfig {
    fn default() -> Self {
        Self {
            no_hpss: false,
            hpss_kernel: HPSS_KERNEL,
            chroma12_only: false,
            chroma72_only: false,
            analysis_seconds: None,
        }
    }
}

/// Ablation entry point: runs the key detection pipeline with configurable
/// component toggles. Used by the bench to measure each component's
/// contribution to accuracy.
pub fn detect_key_ablation(
    samples: &[f32],
    weights: ProfileWeights,
    cfg: &AblationConfig,
) -> Result<Vec<RankedCandidate>> {
    // 0. Window
    let max_samples = cfg.analysis_seconds.unwrap_or(MAX_ANALYSIS_SECONDS) * SAMPLE_RATE;
    let samples_win = if samples.len() <= max_samples {
        samples
    } else {
        let start = (samples.len() - max_samples) / 2;
        &samples[start..start + max_samples]
    };

    // 1. Spectrogram
    let spec = compute_spectrogram(samples_win)?;
    let (_, frames) = spec.dim();
    if frames == 0 {
        return Ok(vec![]);
    }

    // 2. HPSS (or skip)
    let harmonic_source: Array2<f64> = if cfg.no_hpss {
        spec.clone()
    } else {
        let (h, _p) = hpss(&spec, cfg.hpss_kernel);
        h
    };

    // 3. Chromagrams
    let chroma12 = chromagram_from_spec(&harmonic_source);
    let chroma72 = chromagram72_from_spec(&harmonic_source);

    // 4. Voting — use the appropriate path based on ablation flags
    if cfg.chroma12_only {
        // 12-bin only: use the legacy single-chroma path with soft voting
        Ok(temporal_vote_ranked_soft_12(&chroma12, 8, weights))
    } else if cfg.chroma72_only {
        // 72-band only: use Sha'ath 72 exclusively
        Ok(temporal_vote_ranked_soft_72(&chroma72, 8, weights))
    } else {
        // Default dual path
        Ok(temporal_vote_ranked_dual_soft(&chroma12, &chroma72, 8, weights))
    }
}

/// Main entry point used by the analysis worker.
pub fn detect_key(samples: &[f32]) -> Result<KeyResult> {
    detect_key_with_weights(samples, ProfileWeights::default())
}

pub fn detect_key_with_weights(samples: &[f32], weights: ProfileWeights) -> Result<KeyResult> {
    // 0. Trim to a centered window so very long tracks don’t blow up timing.
    let samples = window_for_analysis(samples);

    // 1. Magnitude spectrogram
    let spec = compute_spectrogram(samples)?;
    let (_, frames) = spec.dim();
    if frames == 0 {
        return Ok(KeyResult {
            key_standard: "unknown".into(),
            key_camelot: "".into(),
            confidence: 0.0,
        });
    }

    // 2. HPSS
    let (harmonic, _percussive) = hpss(&spec, HPSS_KERNEL);

    // 3. Chromagrams: 12-bin (Krumhansl + Temperley) and 72-band (Sha'ath)
    let chroma12 = chromagram_from_spec(&harmonic);
    let chroma72 = chromagram72_from_spec(&harmonic);

    // 4. Temporal segment voting — dual path (soft aggregation)
    let ranked = temporal_vote_ranked_dual_soft(&chroma12, &chroma72, 8, weights);
    let vote = match ranked.first() {
        Some(c) => super::ensemble::KeyVote { tonic: c.tonic, is_major: c.is_major, score: c.confidence },
        None => super::ensemble::KeyVote { tonic: 0, is_major: true, score: 0.0 },
    };

    // 5. Format
    let (standard, camelot, confidence) = format_key(vote);
    Ok(KeyResult {
        key_standard: standard,
        key_camelot: camelot,
        confidence,
    })
}

/// Per-stage timing for the diagnostic key-detection path. Milliseconds.
#[derive(Debug, Clone, Copy, Default)]
pub struct StageTimings {
    pub spectrogram: u64,
    pub hpss: u64,
    pub chromagram: u64,
    pub ensemble: u64,
}

/// Rich diagnostic result. Returned by the Tuner path so the UI can show
/// runners-up, the raw chroma vector, and per-stage timings.
pub struct KeyDiagnostic {
    pub candidates: Vec<RankedCandidate>,
    /// Mean chroma vector across the entire track (12 pitch classes,
    /// normalised so max == 1.0 for display).
    pub chroma_mean: [f64; 12],
    pub timings: StageTimings,
}

/// Same pipeline as `detect_key_with_weights` but instrumented and returning
/// the full ranked candidate list + chroma vector + timings.
///
/// Used by the Tuner command. The batch analysis path keeps using
/// `detect_key_with_weights` to avoid paying for diagnostic plumbing on
/// every library track.
///
/// The `on_stage` callback is invoked after each stage completes with
/// `(stage_name, percent_complete)` so the caller can stream progress to the UI.
pub fn detect_key_diagnostic(
    samples: &[f32],
    weights: ProfileWeights,
    mut on_stage: impl FnMut(&str, f64),
) -> Result<KeyDiagnostic> {
    use std::time::Instant;

    let mut timings = StageTimings::default();

    // 0. Trim to a centered window for very long tracks. We keep a separate
    //    binding (`samples_win`) instead of reusing the parameter so it’s
    //    obvious where the windowing happens; downstream stages read from it.
    let samples_win = window_for_analysis(samples);

    let t = Instant::now();
    let spec = compute_spectrogram(samples_win)?;
    timings.spectrogram = t.elapsed().as_millis() as u64;
    on_stage("spectrogram", 0.40);

    let (_, frames) = spec.dim();
    if frames == 0 {
        return Ok(KeyDiagnostic {
            candidates: vec![],
            chroma_mean: [0.0; 12],
            timings,
        });
    }

    let t = Instant::now();
    let (harmonic, _percussive) = hpss(&spec, HPSS_KERNEL);
    timings.hpss = t.elapsed().as_millis() as u64;
    on_stage("hpss", 0.60);

    let t = Instant::now();
    let chroma12 = chromagram_from_spec(&harmonic);
    let chroma72 = chromagram72_from_spec(&harmonic);
    timings.chromagram = t.elapsed().as_millis() as u64;
    on_stage("chromagram", 0.75);

    let t = Instant::now();
    let candidates = temporal_vote_ranked_dual_soft(&chroma12, &chroma72, 8, weights);
    timings.ensemble = t.elapsed().as_millis() as u64;
    on_stage("ensemble", 0.85);

    // Track-level corrections on the top candidate.
    // (Disabled — fifth-disambiguation and mode-flip both degraded accuracy
    //  on the 500-track benchmark. The engine is already well-tuned for
    //  this corpus, and naive post-hoc chroma heuristics create more errors
    //  than they fix. See ACCURACY.md for the full ablation results.)

    let chroma_mean = mean_chroma(&chroma12);

    Ok(KeyDiagnostic { candidates, chroma_mean, timings })
}

/// Mean chroma across the full track, normalised so the max bin == 1.0.
/// Purely for display; the per-segment votes already use the raw chromagram.
fn mean_chroma(chromagram: &Array2<f64>) -> [f64; 12] {
    let (_, frames) = chromagram.dim();
    if frames == 0 {
        return [0.0; 12];
    }
    let mut mean = [0.0_f64; 12];
    for pc in 0..12 {
        let row = chromagram.slice(ndarray::s![pc, ..]);
        mean[pc] = row.mean().unwrap_or(0.0);
    }
    let max = mean.iter().copied().fold(0.0_f64, f64::max);
    if max > 0.0 {
        for v in &mut mean {
            *v /= max;
        }
    }
    mean
}
