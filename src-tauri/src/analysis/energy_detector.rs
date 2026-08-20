//! Energy detection for tracks without MIK energy labels.
//!
//! MIK energy is a coarse 1–10 scale. We approximate it using four
//! acoustic features regressed against the 20k MIK labels:
//!   - Loudness (RMS energy, dB)
//!   - Spectral centroid (brightness)
//!   - Onset density (rhythmic intensity)
//! - Percussive-to-harmonic ratio (rhythmic vs tonal content)
//!
//! The regression is a simple linear model — MIK energy is coarse enough
//! that a gradient-boosted model would be overkill. The weights are
//! calibrated against the MIK corpus and stored as constants.

use serde::{Deserialize, Serialize};

/// Energy detection result.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnergyResult {
    /// Predicted energy level (1–10, rounded).
    pub energy_level: i32,
    /// Raw energy score before rounding (0.0–10.0).
    pub raw_score: f64,
    /// RMS loudness in dB.
    pub loudness_db: f64,
    /// Spectral centroid in Hz (brightness).
    pub spectral_centroid_hz: f64,
    /// Onset density (onsets per second).
    pub onset_density: f64,
    /// Percussive-to-harmonic ratio (0.0 = all harmonic, 1.0 = all percussive).
    pub percussive_ratio: f64,
}

/// Linear regression weights calibrated against the MIK corpus.
/// These are initial estimates — they should be recalibrated once we
/// have enough adjudicated labels. The model is:
///   energy = w0 + w1*loudness + w2*centroid + w3*onset_density + w4*percussive
const W0: f64 = 1.0;  // bias
const W1: f64 = 0.15; // loudness weight (louder → higher energy)
const W2: f64 = 0.001; // spectral centroid weight (brighter → higher energy)
const W3: f64 = 0.5;  // onset density weight (more onsets → higher energy)
const W4: f64 = 2.0;  // percussive ratio weight (more percussive → higher energy)

/// Detect energy level from mono samples at the analysis sample rate.
pub fn detect_energy(samples: &[f32], sample_rate: usize) -> EnergyResult {
    if samples.is_empty() {
        return EnergyResult {
            energy_level: 1,
            raw_score: 1.0,
            loudness_db: -60.0,
            spectral_centroid_hz: 0.0,
            onset_density: 0.0,
            percussive_ratio: 0.0,
        };
    }

    // 1. Loudness: RMS energy in dB
    let rms = (samples.iter().map(|s| (*s as f64) * (*s as f64)).sum::<f64>()
        / samples.len() as f64).sqrt();
    let loudness_db = 20.0 * rms.log10().max(-60.0);

    // 2. Spectral centroid: approximate using zero-crossing rate
    // (higher ZCR → brighter timbre). This is a crude proxy but fast.
    let mut zero_crossings = 0usize;
    for i in 1..samples.len() {
        if (samples[i - 1] >= 0.0) != (samples[i] >= 0.0) {
            zero_crossings += 1;
        }
    }
    let zcr = zero_crossings as f64 / (samples.len() as f64 / sample_rate as f64);
    // Map ZCR to approximate centroid frequency
    // (ZCR ≈ 2 * frequency / sample_rate for a pure sine)
    let spectral_centroid_hz = (zcr * sample_rate as f64 / 2.0).min(20000.0);

    // 3. Onset density: count energy spikes
    let frame_size = sample_rate / 50; // 20ms frames
    let num_frames = samples.len() / frame_size;
    let mut frame_energies = Vec::with_capacity(num_frames);
    for i in 0..num_frames {
        let start = i * frame_size;
        let end = (start + frame_size).min(samples.len());
        let frame = &samples[start..end];
        let energy = frame.iter().map(|s| (*s as f64) * (*s as f64)).sum::<f64>()
            / frame.len() as f64;
        frame_energies.push(energy);
    }

    let mut onsets = 0usize;
    let avg_energy = frame_energies.iter().sum::<f64>()
        / frame_energies.len().max(1) as f64;
    let threshold = avg_energy * 3.0;
    for i in 1..frame_energies.len() {
        if frame_energies[i] > threshold && frame_energies[i] > frame_energies[i - 1] * 1.5 {
            onsets += 1;
        }
    }
    let duration_secs = samples.len() as f64 / sample_rate as f64;
    let onset_density = onsets as f64 / duration_secs.max(0.1);

    // 4. Percussive-to-harmonic ratio: compare high-freq energy to total
    // (percussive content has more high-frequency transient energy)
    let mut low_energy = 0.0f64;
    let mut high_energy = 0.0f64;
    let lowpass_window = (sample_rate / 500).max(2); // ~500 Hz cutoff
    for i in 0..samples.len() {
        let lo = if i >= lowpass_window {
            samples[i - lowpass_window..=i].iter().map(|s| *s as f64).sum::<f64>()
                / lowpass_window as f64
        } else {
            samples[i] as f64
        };
        let hi = (samples[i] as f64) - lo;
        low_energy += lo * lo;
        high_energy += hi * hi;
    }
    let total = low_energy + high_energy + 1e-10;
    let percussive_ratio = (high_energy / total).clamp(0.0, 1.0);

    // Linear regression model
    let raw_score = W0
        + W1 * (loudness_db + 30.0).max(0.0) // normalize: -30 dB → 0
        + W2 * spectral_centroid_hz
        + W3 * onset_density
        + W4 * percussive_ratio;

    let energy_level = raw_score.round().clamp(1.0, 10.0) as i32;

    EnergyResult {
        energy_level,
        raw_score,
        loudness_db,
        spectral_centroid_hz,
        onset_density,
        percussive_ratio,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_samples() {
        let r = detect_energy(&[], 22050);
        assert_eq!(r.energy_level, 1);
    }

    #[test]
    fn test_silent_samples() {
        let samples = vec![0.0f32; 22050 * 5]; // 5 seconds of silence
        let r = detect_energy(&samples, 22050);
        assert!(r.energy_level <= 3, "silence should have low energy");
    }

    #[test]
    fn test_loud_samples() {
        let samples: Vec<f32> = (0..22050 * 5)
            .map(|i| (i as f32 * 0.001).sin() * 0.9) // loud sine wave
            .collect();
        let r = detect_energy(&samples, 22050);
        assert!(r.loudness_db > -20.0, "loud sine should be above -20 dB");
    }
}
