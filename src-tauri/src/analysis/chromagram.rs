use anyhow::Result;
use ndarray::Array2;
use rustfft::{num_complex::Complex, FftPlanner};

use super::chroma_transform::ChromaTransform;
use super::{BANDS_72, FFT_SIZE, HOP_SIZE, SAMPLE_RATE};

/// Compute magnitude spectrogram `[bins, frames]` where `bins = FFT_SIZE/2`.
pub fn compute_spectrogram(samples: &[f32]) -> Result<Array2<f64>> {
    if samples.len() < FFT_SIZE {
        return Ok(Array2::zeros((FFT_SIZE / 2, 0)));
    }
    let num_frames = (samples.len() - FFT_SIZE) / HOP_SIZE + 1;
    let bins = FFT_SIZE / 2;
    let mut spec = Array2::zeros((bins, num_frames));
    
    let mut planner = FftPlanner::new();
    let fft = planner.plan_fft_forward(FFT_SIZE);
    let mut buffer = vec![Complex::new(0.0f32, 0.0); FFT_SIZE];
    
    for frame_idx in 0..num_frames {
        let start = frame_idx * HOP_SIZE;
        for (i, buf_sample) in buffer.iter_mut().enumerate() {
            if start + i < samples.len() {
                let window = 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / FFT_SIZE as f32).cos());
                *buf_sample = Complex::new(samples[start + i] * window, 0.0);
            } else {
                *buf_sample = Complex::new(0.0, 0.0);
            }
        }
        fft.process(&mut buffer);
        for (bin, c) in buffer[..bins].iter().enumerate() {
            spec[[bin, frame_idx]] = ((c.re * c.re + c.im * c.im) as f64).sqrt();
        }
    }
    Ok(spec)
}

/// Convert a magnitude spectrogram into a 12-bin chromagram.
pub fn chromagram_from_spec(spec: &Array2<f64>) -> Array2<f64> {
    let (bins, frames) = spec.dim();
    let mut chroma = Array2::zeros((12, frames));
    for f in 0..frames {
        for bin in 1..bins {
            let freq = bin as f64 * SAMPLE_RATE as f64 / FFT_SIZE as f64;
            let midi_note = 12.0 * (freq / 440.0).log2() + 69.0;
            let pitch_class = (midi_note.round() as i32).rem_euclid(12) as usize;
            chroma[[pitch_class, f]] += spec[[bin, f]];
        }
    }
    chroma
}

/// One-shot: samples → chromagram (no HPSS).
pub fn compute_chromagram(samples: &[f32]) -> Result<Array2<f64>> {
    let spec = compute_spectrogram(samples)?;
    Ok(chromagram_from_spec(&spec))
}

/// Convert a magnitude spectrogram into a 72-band chromagram using the
/// Direct Spectral Kernel (CQT approximation from libKeyFinder).
///
/// Returns `[72, frames]` where the 72 bands are 6 octaves × 12 semitones
/// (C1 through B6).
pub fn chromagram72_from_spec(spec: &Array2<f64>) -> Array2<f64> {
    let (_bins, frames) = spec.dim();
    let ct = ChromaTransform::new(SAMPLE_RATE);
    let mut chroma = Array2::zeros((BANDS_72, frames));
    for f in 0..frames {
        let col = spec.column(f);
        let mag_slice: Vec<f64> = col.iter().copied().collect();
        let cv = ct.chroma_vector_from_magnitudes(&mag_slice);
        for b in 0..BANDS_72 {
            chroma[[b, f]] = cv[b];
        }
    }
    chroma
}
