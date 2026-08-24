//! Native preprocessing for the pinned Myna neural-key candidate.
//!
//! This mirrors nnAudio 0.3.3's default `MelSpectrogram` at 16 kHz. The
//! caller must supply finite mono samples already resampled to 16 kHz; file
//! decoding and resampler parity are deliberately separate promotion gates.

use std::sync::Arc;

use anyhow::{bail, Result};
use rustfft::{num_complex::Complex32, Fft, FftPlanner};

pub const MYNA_SAMPLE_RATE_HZ: usize = 16_000;
pub const MYNA_N_FFT: usize = 2_048;
pub const MYNA_HOP_LENGTH: usize = 512;
pub const MYNA_MEL_BINS: usize = 128;
pub const MYNA_FRAMES_PER_CHUNK: usize = 196;

const ONE_SIDED_BINS: usize = MYNA_N_FFT / 2 + 1;
const CENTER_PADDING: usize = MYNA_N_FFT / 2;

#[derive(Debug)]
pub struct MynaMelChunks {
    /// C-order `[chunk, channel=1, mel, frame]` values for the ONNX graph.
    pub values: Vec<f32>,
    pub chunk_count: usize,
}

pub struct MynaMelPreprocessor {
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    mel_basis: Vec<f32>,
}

impl std::fmt::Debug for MynaMelPreprocessor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MynaMelPreprocessor")
            .field("n_fft", &MYNA_N_FFT)
            .field("hop_length", &MYNA_HOP_LENGTH)
            .field("mel_bins", &MYNA_MEL_BINS)
            .finish()
    }
}

impl Default for MynaMelPreprocessor {
    fn default() -> Self {
        Self::new()
    }
}

impl MynaMelPreprocessor {
    pub fn new() -> Self {
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(MYNA_N_FFT);
        let window = (0..MYNA_N_FFT)
            .map(|index| {
                // scipy.signal.get_window("hann", n_fft, fftbins=True)
                let phase = 2.0_f64 * std::f64::consts::PI * index as f64 / MYNA_N_FFT as f64;
                (0.5_f64 - 0.5_f64 * phase.cos()) as f32
            })
            .collect();
        Self {
            fft,
            window,
            mel_basis: slaney_mel_basis(),
        }
    }

    /// Convert 16 kHz mono audio into exact Myna chunk layout.
    pub fn prepare(&self, samples: &[f32], frames_per_chunk: usize) -> Result<MynaMelChunks> {
        if frames_per_chunk == 0 {
            bail!("Myna frames_per_chunk must be positive");
        }
        if samples.len() <= CENTER_PADDING {
            bail!("Myna audio must exceed the 1024-sample reflection pad");
        }
        if samples.iter().any(|sample| !sample.is_finite()) {
            bail!("Myna audio contains a non-finite sample");
        }

        // Reflection padding by n_fft/2 on each side gives floor(N / hop) + 1
        // STFT frames, exactly matching nnAudio's Conv1d implementation.
        let total_frames = samples.len() / MYNA_HOP_LENGTH + 1;
        let chunk_count = total_frames / frames_per_chunk;
        if chunk_count == 0 {
            bail!("Myna audio is too short to form one complete model chunk");
        }
        let used_frames = chunk_count * frames_per_chunk;
        let chunk_stride = MYNA_MEL_BINS * frames_per_chunk;
        let mut values = vec![0.0_f32; chunk_count * chunk_stride];
        let mut spectrum = vec![Complex32::new(0.0, 0.0); MYNA_N_FFT];
        let mut power = vec![0.0_f32; ONE_SIDED_BINS];

        for frame in 0..used_frames {
            let padded_start = frame * MYNA_HOP_LENGTH;
            for (offset, value) in spectrum.iter_mut().enumerate() {
                let padded_index = padded_start + offset;
                let sample = reflected_sample(samples, padded_index);
                *value = Complex32::new(sample * self.window[offset], 0.0);
            }
            self.fft.process(&mut spectrum);
            for (target, bin) in power.iter_mut().zip(&spectrum[..ONE_SIDED_BINS]) {
                // nnAudio computes magnitude then raises it to power=2.0.
                *target = bin.re * bin.re + bin.im * bin.im;
            }

            let chunk = frame / frames_per_chunk;
            let local_frame = frame % frames_per_chunk;
            for mel in 0..MYNA_MEL_BINS {
                let weights = &self.mel_basis[mel * ONE_SIDED_BINS..(mel + 1) * ONE_SIDED_BINS];
                let mut sum = 0.0_f32;
                for (weight, bin_power) in weights.iter().zip(&power) {
                    sum += weight * bin_power;
                }
                values[chunk * chunk_stride + mel * frames_per_chunk + local_frame] = sum;
            }
        }

        Ok(MynaMelChunks {
            values,
            chunk_count,
        })
    }
}

fn reflected_sample(samples: &[f32], padded_index: usize) -> f32 {
    if padded_index < CENTER_PADDING {
        return samples[CENTER_PADDING - padded_index];
    }
    let index = padded_index - CENTER_PADDING;
    if index < samples.len() {
        samples[index]
    } else {
        samples[2 * samples.len() - 2 - index]
    }
}

fn hz_to_slaney_mel(frequency: f64) -> f64 {
    const LINEAR_HZ_PER_MEL: f64 = 200.0 / 3.0;
    const MIN_LOG_HZ: f64 = 1_000.0;
    const MIN_LOG_MEL: f64 = MIN_LOG_HZ / LINEAR_HZ_PER_MEL;
    const LOG_STEP: f64 = 0.068_751_777_420_949_12; // ln(6.4) / 27
    if frequency >= MIN_LOG_HZ {
        MIN_LOG_MEL + (frequency / MIN_LOG_HZ).ln() / LOG_STEP
    } else {
        frequency / LINEAR_HZ_PER_MEL
    }
}

fn slaney_mel_to_hz(mel: f64) -> f64 {
    const LINEAR_HZ_PER_MEL: f64 = 200.0 / 3.0;
    const MIN_LOG_HZ: f64 = 1_000.0;
    const MIN_LOG_MEL: f64 = MIN_LOG_HZ / LINEAR_HZ_PER_MEL;
    const LOG_STEP: f64 = 0.068_751_777_420_949_12; // ln(6.4) / 27
    if mel >= MIN_LOG_MEL {
        MIN_LOG_HZ * (LOG_STEP * (mel - MIN_LOG_MEL)).exp()
    } else {
        LINEAR_HZ_PER_MEL * mel
    }
}

fn slaney_mel_basis() -> Vec<f32> {
    let minimum_mel = hz_to_slaney_mel(0.0);
    let maximum_mel = hz_to_slaney_mel(MYNA_SAMPLE_RATE_HZ as f64 / 2.0);
    let mel_frequencies: Vec<f64> = (0..MYNA_MEL_BINS + 2)
        .map(|index| {
            let mel = minimum_mel
                + (maximum_mel - minimum_mel) * index as f64 / (MYNA_MEL_BINS + 1) as f64;
            slaney_mel_to_hz(mel)
        })
        .collect();
    let fft_frequencies: Vec<f64> = (0..ONE_SIDED_BINS)
        .map(|bin| bin as f64 * MYNA_SAMPLE_RATE_HZ as f64 / MYNA_N_FFT as f64)
        .collect();
    let mut basis = vec![0.0_f32; MYNA_MEL_BINS * ONE_SIDED_BINS];

    for mel in 0..MYNA_MEL_BINS {
        let lower_width = mel_frequencies[mel + 1] - mel_frequencies[mel];
        let upper_width = mel_frequencies[mel + 2] - mel_frequencies[mel + 1];
        let normalization = 2.0 / (mel_frequencies[mel + 2] - mel_frequencies[mel]);
        for (bin, frequency) in fft_frequencies.iter().copied().enumerate() {
            let lower = (frequency - mel_frequencies[mel]) / lower_width;
            let upper = (mel_frequencies[mel + 2] - frequency) / upper_width;
            // librosa/nnAudio first stores the triangle in float32, then
            // applies Slaney normalization in-place.
            let triangle = lower.min(upper).max(0.0) as f32;
            basis[mel * ONE_SIDED_BINS + bin] = (triangle as f64 * normalization) as f32;
        }
    }
    basis
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deterministic_waveform() -> Vec<f32> {
        let mut state = 0x5eed_1234_u32;
        (0..100_000)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let signed = (state >> 8) as i32 - (1 << 23);
                signed as f32 / (1_u32 << 23) as f32
            })
            .collect()
    }

    #[test]
    fn rejects_invalid_or_incomplete_audio() {
        let preprocessor = MynaMelPreprocessor::new();
        assert!(preprocessor.prepare(&[], MYNA_FRAMES_PER_CHUNK).is_err());
        assert!(preprocessor.prepare(&vec![0.0; 100_000], 0).is_err());
        let mut invalid = vec![0.0; 100_000];
        invalid[42] = f32::NAN;
        assert!(preprocessor
            .prepare(&invalid, MYNA_FRAMES_PER_CHUNK)
            .is_err());
    }

    #[test]
    fn matches_pinned_nnaudio_reference_fixture() {
        let expected_bytes =
            include_bytes!("../tests/fixtures/neural-key/myna-mel-100000-f32le.bin");
        let expected: Vec<f32> = expected_bytes
            .chunks_exact(4)
            .map(|bytes| f32::from_le_bytes(bytes.try_into().unwrap()))
            .collect();
        let actual = MynaMelPreprocessor::new()
            .prepare(&deterministic_waveform(), MYNA_FRAMES_PER_CHUNK)
            .unwrap();
        assert_eq!(actual.chunk_count, 1);
        assert_eq!(actual.values.len(), 128 * 196);
        assert_eq!(actual.values.len(), expected.len());

        let mut maximum_absolute_error = 0.0_f32;
        let mut maximum_relative_error = 0.0_f32;
        for (actual, expected) in actual.values.iter().zip(&expected) {
            let absolute = (actual - expected).abs();
            maximum_absolute_error = maximum_absolute_error.max(absolute);
            maximum_relative_error = maximum_relative_error.max(absolute / expected.abs().max(1.0));
        }
        eprintln!(
            "nnAudio mel parity max_abs={maximum_absolute_error} max_rel={maximum_relative_error}"
        );
        assert!(
            maximum_absolute_error <= 5.0e-4 && maximum_relative_error <= 1.0e-5,
            "nnAudio mel parity failed: max_abs={maximum_absolute_error} max_rel={maximum_relative_error}"
        );
    }
}
