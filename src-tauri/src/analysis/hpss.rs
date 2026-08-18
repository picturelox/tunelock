//! Harmonic/Percussive Source Separation via median filtering.
//!
//! Reference: Fitzgerald, "Harmonic/Percussive Separation Using Median Filtering" (DAFx-10).
//! Horizontal median (along time) suppresses transients → harmonic estimate.
//! Vertical median (along frequency) suppresses pitches → percussive estimate.
//!
//! Performance notes (BUG-003 fix):
//! * The two median passes run in parallel across rows / columns using
//!   `ndarray::Zip::par_for_each` (rayon thread-pool).
//! * Each parallel task reuses a single stack-allocated window buffer —
//!   **no per-cell heap allocation** like the original naïve version.
//! * The soft-mask pass is a fused parallel element-wise Zip over all four
//!   spectrograms, avoiding an intermediate allocation.

use ndarray::{Array2, ArrayView1, ArrayViewMut1, Axis, Zip};

/// HPSS on a magnitude spectrogram `spec[bins, frames]`.
/// Returns `(harmonic, percussive)` with the same shape.
///
/// `kernel_size` is the median filter length along each axis (will be coerced to odd, min 3).
pub fn hpss(spec: &Array2<f64>, kernel_size: usize) -> (Array2<f64>, Array2<f64>) {
    let k = kernel_size.max(3) | 1; // force odd
    let (bins, frames) = spec.dim();
    
    // Horizontal median (along frames). Iterate rows (one row = one frequency bin).
    let mut harmonic = Array2::<f64>::zeros((bins, frames));
    Zip::from(harmonic.axis_iter_mut(Axis(0)))
        .and(spec.axis_iter(Axis(0)))
        .par_for_each(|out_row, in_row| {
            median_filter_1d(&in_row, k, out_row);
        });
    
    // Vertical median (along bins). Iterate columns (one column = one frame).
    let mut percussive = Array2::<f64>::zeros((bins, frames));
    Zip::from(percussive.axis_iter_mut(Axis(1)))
        .and(spec.axis_iter(Axis(1)))
        .par_for_each(|out_col, in_col| {
            median_filter_1d(&in_col, k, out_col);
        });
    
    // Soft mask: H_out = spec * H²/(H²+P²+eps),  P_out = spec * P²/(H²+P²+eps).
    // One fused parallel pass, no intermediate allocation.
    let mut h_out = Array2::<f64>::zeros((bins, frames));
    let mut p_out = Array2::<f64>::zeros((bins, frames));
    let eps = 1e-10;
    Zip::from(&mut h_out)
        .and(&mut p_out)
        .and(spec)
        .and(&harmonic)
        .and(&percussive)
        .par_for_each(|ho, po, &s, &h, &p| {
            let h2 = h * h;
            let p2 = p * p;
            let denom = h2 + p2 + eps;
            *ho = s * (h2 / denom);
            *po = s * (p2 / denom);
        });
    
    (h_out, p_out)
}

/// In-place 1D median filter. Reuses a single stack-allocated window buffer.
/// Handles boundaries by shrinking the window (same as scipy's `reflect='nearest'` mode at the edges).
fn median_filter_1d(input: &ArrayView1<f64>, kernel: usize, mut output: ArrayViewMut1<f64>) {
    let n = input.len();
    let half = kernel / 2;
    // Small stack-sized buffer. `kernel` is odd and typically ≤ 31 in our pipeline.
    let mut window: [f64; 64] = [0.0; 64];
    debug_assert!(kernel <= window.len(), "kernel {} exceeds window buffer 64", kernel);
    
    for i in 0..n {
        let lo = i.saturating_sub(half);
        let hi = (i + half + 1).min(n);
        let len = hi - lo;
        for (j, idx) in (lo..hi).enumerate() {
            window[j] = input[idx];
        }
        let slice = &mut window[..len];
        slice.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        output[i] = slice[len / 2];
    }
}
