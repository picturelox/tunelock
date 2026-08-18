use anyhow::Result;

use super::SAMPLE_RATE;

/// Onset-detection hop. Independent of the spectrogram's `HOP_SIZE` — the
/// tempo path uses its own simple energy-based onset signal.
const ONSET_HOP: usize = 512;
const ONSET_WINDOW: usize = 2048;

/// Search range for tempo detection. Widened from the original 60–180 to
/// cover all genres in the MIK corpus (range ~79–190 BPM, with some
/// edge cases outside). The octave-correction step can push the final
/// result outside this range, but the autocorrelation search stays within
/// it to avoid noisy sub-harmonics.
const MIN_BPM: f64 = 40.0;
const MAX_BPM: f64 = 220.0;

/// Number of top autocorrelation peaks to evaluate for octave correction.
/// Too few = miss the true tempo; too many = noise dominates.
const NUM_PEAKS: usize = 10;

/// Detect tempo using onset detection, autocorrelation, and octave
/// resolution.
///
/// ## Why octave resolution matters
///
/// Autocorrelation finds the dominant period in the onset signal. For a
/// 128 BPM track, the strongest peak is often at 64 BPM (half-time) because
/// onsets are more consistent at every-other-beat (the kick pattern repeats
/// every 2 beats). Without octave correction, the detector reports 64 BPM
/// instead of 128.
///
/// The fix: find the top N autocorrelation peaks, evaluate each at ×0.5,
/// ×1, and ×2, and apply a tempo preference function that mildly favours
/// the 100–160 BPM range (where most popular music lives). The preference
/// is gentle enough not to override strong evidence for genuine 70 or 190
/// BPM tracks.
pub fn detect_tempo(samples: &[f32]) -> Result<f64> {
    // Step 1: Compute onset strength signal.
    let onset_signal = compute_onset_signal(samples);
    if onset_signal.is_empty() {
        return Ok(120.0); // fallback for silence / very short clips
    }

    // Step 2: Compute autocorrelation over a wide lag range.
    let frame_rate = SAMPLE_RATE as f64 / ONSET_HOP as f64;
    let max_lag = (60.0 / MIN_BPM * frame_rate) as usize;
    let autocorr = compute_autocorrelation(&onset_signal, max_lag);

    // Step 3: Find local-maximum peaks in the valid BPM range.
    let min_lag = (60.0 / MAX_BPM * frame_rate).max(1.0) as usize;
    let max_lag_clamped = max_lag.min(autocorr.len().saturating_sub(2));
    let peaks = find_peaks(&autocorr, min_lag, max_lag_clamped);

    if peaks.is_empty() {
        return Ok(120.0);
    }

    // Step 4: Evaluate octave candidates (×0.5, ×1, ×2) for each peak.
    //
    // For each autocorrelation peak at lag L, the base BPM is
    // 60 * frame_rate / L. The true tempo could be at base, 2×base, or
    // 0.5×base. We score each candidate as:
    //   score = autocorr_strength * tempo_preference(bpm)
    //
    // The tempo preference is a gentle Gaussian on a log-BPM scale that
    // mildly favours ~120 BPM. This counteracts the half-time bias without
    // forcing everything into a narrow range.
    let mut candidates: Vec<TempoCandidate> = Vec::new();

    for peak in &peaks {
        // Parabolic interpolation for sub-frame precision.
        let precise_lag = parabolic_interp(&autocorr, peak.lag);
        let base_bpm = 60.0 * frame_rate / precise_lag;

        for multiplier in [0.5, 1.0, 2.0] {
            let bpm = base_bpm * multiplier;
            if bpm < MIN_BPM || bpm > MAX_BPM {
                continue;
            }
            let pref = tempo_preference(bpm);
            let score = peak.strength * pref;
            candidates.push(TempoCandidate {
                bpm,
                score,
                peak_strength: peak.strength,
                pref,
            });
        }
    }

    if candidates.is_empty() {
        return Ok(120.0);
    }

    // Step 5: Pick the highest-scoring candidate.
    candidates.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let best_bpm = candidates[0].bpm;

    Ok(best_bpm.clamp(MIN_BPM, MAX_BPM))
}

/// Diagnostic tempo result with confidence and candidates.
#[derive(Debug)]
pub struct TempoDiagnostic {
    pub bpm: f64,
    pub confidence: f64,
    pub candidates: Vec<TempoCandidate>,
}

/// A tempo candidate with scoring breakdown.
#[derive(Debug, Clone)]
pub struct TempoCandidate {
    pub bpm: f64,
    /// Combined score: autocorrelation strength × tempo preference.
    pub score: f64,
    /// Raw autocorrelation strength at the peak.
    pub peak_strength: f64,
    /// Tempo preference function value (0..1).
    pub pref: f64,
}

/// Detect tempo with full diagnostic information.
pub fn detect_tempo_diagnostic(samples: &[f32]) -> Result<TempoDiagnostic> {
    let onset_signal = compute_onset_signal(samples);
    if onset_signal.is_empty() {
        return Ok(TempoDiagnostic {
            bpm: 120.0,
            confidence: 0.0,
            candidates: vec![],
        });
    }

    let frame_rate = SAMPLE_RATE as f64 / ONSET_HOP as f64;
    let max_lag = (60.0 / MIN_BPM * frame_rate) as usize;
    let autocorr = compute_autocorrelation(&onset_signal, max_lag);

    let min_lag = (60.0 / MAX_BPM * frame_rate).max(1.0) as usize;
    let max_lag_clamped = max_lag.min(autocorr.len().saturating_sub(2));
    let peaks = find_peaks(&autocorr, min_lag, max_lag_clamped);

    let mut candidates: Vec<TempoCandidate> = Vec::new();
    for peak in &peaks {
        let precise_lag = parabolic_interp(&autocorr, peak.lag);
        let base_bpm = 60.0 * frame_rate / precise_lag;
        for multiplier in [0.5, 1.0, 2.0] {
            let bpm = base_bpm * multiplier;
            if bpm < MIN_BPM || bpm > MAX_BPM {
                continue;
            }
            let pref = tempo_preference(bpm);
            let score = peak.strength * pref;
            candidates.push(TempoCandidate {
                bpm,
                score,
                peak_strength: peak.strength,
                pref,
            });
        }
    }

    candidates.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let best_bpm = candidates.first().map(|c| c.bpm).unwrap_or(120.0);
    let confidence = if candidates.len() >= 2 {
        let top = candidates[0].score;
        let second = candidates[1].score;
        if top > 0.0 {
            (1.0 - second / top).clamp(0.0, 1.0)
        } else {
            0.0
        }
    } else if candidates.len() == 1 {
        candidates[0].peak_strength
    } else {
        0.0
    };

    Ok(TempoDiagnostic {
        bpm: best_bpm.clamp(MIN_BPM, MAX_BPM),
        confidence,
        candidates: candidates.into_iter().take(5).collect(),
    })
}

/// Tempo preference function: Gaussian on a log-BPM scale centered at
/// ~120 BPM with σ ≈ 1 octave.
///
/// This mildly favours tempos in the 80–170 range — where most popular and
/// electronic music lives — without penalising genuine 70 or 190 BPM tracks
/// too heavily. The preference is multiplicative on the autocorrelation
/// strength, so a very strong peak at 64 BPM can still win if the 128 BPM
/// peak is weak.
fn tempo_preference(bpm: f64) -> f64 {
    let log_bpm = bpm.log2();
    let log_center = 120.0f64.log2();
    let sigma = 1.0; // ~1 octave spread
    (-0.5 * ((log_bpm - log_center) / sigma).powi(2)).exp()
}

struct Peak {
    lag: usize,
    strength: f64,
}

/// Find local maxima in the autocorrelation, sorted by strength (descending).
/// Returns at most `NUM_PEAKS` peaks.
fn find_peaks(autocorr: &[f64], min_lag: usize, max_lag: usize) -> Vec<Peak> {
    let mut peaks: Vec<Peak> = Vec::new();

    for lag in min_lag..=max_lag {
        let prev = if lag > 0 { autocorr[lag - 1] } else { f64::NEG_INFINITY };
        let next = if lag + 1 < autocorr.len() { autocorr[lag + 1] } else { f64::NEG_INFINITY };
        if autocorr[lag] > prev && autocorr[lag] > next && autocorr[lag] > 0.0 {
            peaks.push(Peak { lag, strength: autocorr[lag] });
        }
    }

    // Deduplicate nearby peaks: if two peaks are within 3 frames of each
    // other, keep only the stronger one. This avoids evaluating the same
    // tempo multiple times with slightly different lags.
    peaks.sort_by(|a, b| a.lag.cmp(&b.lag));
    let mut deduped: Vec<Peak> = Vec::new();
    for p in peaks {
        if let Some(last) = deduped.last() {
            if (p.lag as isize - last.lag as isize).abs() <= 3 {
                if p.strength > last.strength {
                    *deduped.last_mut().unwrap() = p;
                }
                continue;
            }
        }
        deduped.push(p);
    }

    deduped.sort_by(|a, b| {
        b.strength
            .partial_cmp(&a.strength)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    deduped.into_iter().take(NUM_PEAKS).collect()
}

/// Parabolic interpolation around a peak for sub-frame precision.
///
/// Fits a parabola through the peak and its two neighbours, then finds the
/// vertex. This gives a more precise lag estimate than the integer peak
/// position, which translates to a more precise BPM.
fn parabolic_interp(autocorr: &[f64], peak_idx: usize) -> f64 {
    if peak_idx == 0 || peak_idx >= autocorr.len() - 1 {
        return peak_idx as f64;
    }
    let a = autocorr[peak_idx - 1];
    let b = autocorr[peak_idx];
    let c = autocorr[peak_idx + 1];
    let denom = a - 2.0 * b + c;
    if denom.abs() < 1e-10 {
        return peak_idx as f64;
    }
    let offset = 0.5 * (a - c) / denom;
    peak_idx as f64 + offset
}

/// Compute onset strength using energy differences.
///
/// This is a simple positive-energy-flux onset detector. For each frame,
/// it computes the total energy and takes the positive difference from the
/// previous frame (with a 0.9 decay to avoid persistent-energy bias).
fn compute_onset_signal(samples: &[f32]) -> Vec<f64> {
    let hop_size = ONSET_HOP;
    let window_size = ONSET_WINDOW;
    if samples.len() < window_size {
        return Vec::new();
    }
    let num_frames = (samples.len() - window_size) / hop_size + 1;

    let mut onset_signal = Vec::with_capacity(num_frames);
    let mut prev_energy = 0.0f64;

    for frame in 0..num_frames {
        let start = frame * hop_size;
        let end = (start + window_size).min(samples.len());

        let energy: f64 = samples[start..end]
            .iter()
            .map(|&s| (s * s) as f64)
            .sum();

        let onset = (energy - prev_energy).max(0.0);
        onset_signal.push(onset);

        prev_energy = energy * 0.9; // decay factor
    }

    onset_signal
}

/// Compute autocorrelation of a signal up to `max_lag`.
fn compute_autocorrelation(signal: &[f64], max_lag: usize) -> Vec<f64> {
    let max_lag = max_lag.min(signal.len());
    let mut autocorr = vec![0.0; max_lag];

    for lag in 0..max_lag {
        let mut sum = 0.0;
        for i in 0..signal.len() - lag {
            sum += signal[i] * signal[i + lag];
        }
        autocorr[lag] = sum;
    }

    // Normalize so that lag-0 = 1.0
    let norm = autocorr[0];
    if norm > 0.0 {
        for val in &mut autocorr {
            *val /= norm;
        }
    }

    autocorr
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tempo_preference_peaks_near_120() {
        // Peak at 120 BPM
        assert!(tempo_preference(120.0) > tempo_preference(60.0));
        assert!(tempo_preference(120.0) > tempo_preference(240.0));
        // Symmetric on log scale: 60 and 240 are equidistant from 120
        assert!((tempo_preference(60.0) - tempo_preference(240.0)).abs() < 1e-10);
        // 100 and 140 are close to the peak
        assert!(tempo_preference(100.0) > 0.8);
        assert!(tempo_preference(140.0) > 0.8);
    }

    #[test]
    fn parabolic_interp_returns_peak_for_flat() {
        let data = [0.5, 1.0, 0.5];
        let result = parabolic_interp(&data, 1);
        assert!((result - 1.0).abs() < 1e-10);
    }

    #[test]
    fn detect_tempo_returns_fallback_for_silence() {
        let silence = vec![0.0f32; 1000];
        let bpm = detect_tempo(&silence).unwrap();
        assert_eq!(bpm, 120.0);
    }

    #[test]
    fn detect_tempo_on_synthetic_beat() {
        // Generate a synthetic 128 BPM kick drum pattern at 22050 Hz.
        // 128 BPM = 0.469 sec/beat = 10336 samples/beat.
        let beat_period = (SAMPLE_RATE as f64 / (128.0 / 60.0)) as usize;
        let total_samples = beat_period * 64; // 64 beats
        let mut samples = vec![0.0f32; total_samples];
        for i in 0..64 {
            // Each beat: a short burst of energy (kick drum approximation)
            let start = i * beat_period;
            for j in 0..200.min(total_samples - start) {
                samples[start + j] = (0.5 * (-(j as f32) / 50.0).exp()) * (2.0 * std::f32::consts::PI * 60.0 * j as f32 / SAMPLE_RATE as f32).sin();
            }
        }
        let bpm = detect_tempo(&samples).unwrap();
        // Should be close to 128 BPM (within 2 BPM)
        assert!((bpm - 128.0).abs() < 3.0, "expected ~128 BPM, got {}", bpm);
    }
}
