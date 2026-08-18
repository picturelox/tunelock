use anyhow::Result;

use super::SAMPLE_RATE;

/// Onset-detection hop. Independent of the spectrogram’s `HOP_SIZE`— the
/// tempo path uses its own simple energy-based onset signal.
const ONSET_HOP: usize = 512;
const ONSET_WINDOW: usize = 2048;

/// Detect tempo using onset detection and autocorrelation.
///
/// Operates on the same downsampled audio as the key path (`SAMPLE_RATE`).
/// Energy onsets aren’t pitch-dependent so the lower rate has no impact on
/// tempo accuracy — we just need to keep all the rate constants consistent.
pub fn detect_tempo(samples: &[f32]) -> Result<f64> {
    // Step 1: Compute onset strength signal
    let onset_signal = compute_onset_signal(samples);

    // Step 2: Compute autocorrelation. Allow up to ~1 second of lag at the
    // onset-frame rate.
    let frame_rate = SAMPLE_RATE as f64 / ONSET_HOP as f64;
    let autocorr = compute_autocorrelation(&onset_signal, frame_rate as usize);

    // Step 3: Find peaks in the valid 60–180 BPM range.
    let min_lag = (60.0 / 180.0 * frame_rate) as usize; // 180 BPM
    let max_lag = (60.0 / 60.0 * frame_rate) as usize;  // 60 BPM
    
    let mut best_lag = min_lag;
    let mut best_strength = 0.0f64;
    
    for lag in min_lag..=max_lag.min(autocorr.len() - 1) {
        if autocorr[lag] > best_strength {
            best_strength = autocorr[lag];
            best_lag = lag;
        }
    }
    
    // Convert lag to BPM
    let bpm = 60.0 * frame_rate / best_lag as f64;

    Ok(bpm.clamp(60.0, 180.0))
}

/// Compute onset strength using energy differences
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
        
        // Compute energy in this frame
        let energy: f64 = samples[start..end]
            .iter()
            .map(|&s| (s * s) as f64)
            .sum();
        
        // Onset is positive change in energy
        let onset = (energy - prev_energy).max(0.0);
        onset_signal.push(onset);
        
        prev_energy = energy * 0.9; // Decay factor
    }
    
    onset_signal
}

/// Compute autocorrelation of a signal
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
    
    // Normalize
    let norm = autocorr[0];
    if norm > 0.0 {
        for val in &mut autocorr {
            *val /= norm;
        }
    }
    
    autocorr
}
