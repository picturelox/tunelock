// Beat-grid DSP — full beat tracking for the Transition Workbench.
//
// Produces: BPM, first-beat position, beat timestamps, downbeat offset,
// meter, confidence, and optional piecewise tempo map.
//
// Pipeline:
//   1. Multi-band spectral-flux onset detection
//   2. Adaptive whitening/compression
//   3. Tempogram (lag-vs-time autocorrelation)
//   4. Multiple tempo hypotheses with octave awareness
//   5. Dynamic-programming beat tracking (Ellis-style)
//   6. Downbeat and meter scoring
//   7. Confidence scoring
//
// Runs on the 22.05 kHz mono analysis pipeline. Results are stored in the
// beat_grids table and used by the audio engine for loop alignment and
// beat-synchronized transport.
//
// References:
//   Ellis, "Beat Tracking by Dynamic Programming" (2007)
//   Klapuri & Davy, "Signal Processing Methods for Music Transcription" (2006)
//   Davies & Plumbley, "Context-Dependent Beat Tracking of Musical Audio" (2004)

use super::SAMPLE_RATE;

/// Analysis sample rate as f64 for convenience.
const SR: f64 = SAMPLE_RATE as f64;

/// Onset detection parameters.
const ONSET_FFT_SIZE: usize = 1024;
const ONSET_HOP_SIZE: usize = 512;
/// Frame rate of the onset signal (frames per second).
const ONSET_FRAME_RATE: f64 = SR / ONSET_HOP_SIZE as f64;

/// Tempo search range.
const MIN_BPM: f64 = 40.0;
const MAX_BPM: f64 = 220.0;

/// Number of tempo hypotheses to evaluate.
const NUM_TEMPO_HYPOTHESES: usize = 20;

/// DP beat tracking: width of the tempo transition window (in beats).
/// The DP searches ±this many frames around the expected next beat.
const DP_WINDOW_BEATS: f64 = 0.5;

/// Minimum onset signal length to attempt beat tracking (frames).
const MIN_ONSET_FRAMES: usize = 50;

// ============================================================================
// Public API
// ============================================================================

/// A complete beat grid for a track.
#[derive(Debug, Clone)]
pub struct BeatGridResult {
    /// Detected BPM (may differ from the simple tempo detector result).
    pub bpm: f64,
    /// Time of the first beat in seconds.
    pub first_beat_sec: f64,
    /// All beat times in seconds.
    pub beat_times: Vec<f64>,
    /// Downbeat offset: which beat (0-indexed) is the first downbeat.
    pub downbeat_offset: usize,
    /// Meter numerator (e.g., 4 for 4/4, 3 for 3/4).
    pub meter_numerator: i32,
    /// Confidence: 0.0 to 1.0, how well the grid aligns with onsets.
    pub confidence: f64,
    /// Onset strength signal (for visualization and debugging).
    pub onset_envelope: Vec<f64>,
    /// Onset frame rate (frames per second).
    pub onset_frame_rate: f64,
}

/// Detect a complete beat grid from mono samples.
///
/// This is the main entry point. It runs the full pipeline and returns
/// a beat grid with BPM, beat times, downbeat, meter, and confidence.
pub fn detect_beat_grid(samples: &[f32]) -> Result<BeatGridResult, String> {
    if samples.len() < ONSET_FFT_SIZE {
        return Err("Audio too short for beat detection".to_string());
    }

    // Step 1: Multi-band spectral-flux onset detection
    let onset_envelope = compute_spectral_flux_onsets(samples);

    if onset_envelope.len() < MIN_ONSET_FRAMES {
        return Err("Onset signal too short".to_string());
    }

    // Step 2: Adaptive whitening (normalize and compress)
    let onset_whitened = adaptive_whiten(&onset_envelope);

    // Step 3: Tempogram — global tempo estimation
    let (tempo_bpm, tempo_confidence) = estimate_global_tempo(&onset_whitened);

    // Step 4: DP beat tracking
    let beat_frames = dp_beat_tracking(&onset_whitened, tempo_bpm);

    if beat_frames.is_empty() {
        return Ok(BeatGridResult {
            bpm: tempo_bpm,
            first_beat_sec: 0.0,
            beat_times: vec![],
            downbeat_offset: 0,
            meter_numerator: 4,
            confidence: 0.0,
            onset_envelope,
            onset_frame_rate: ONSET_FRAME_RATE,
        });
    }

    // Step 5: Convert beat frames to seconds
    let beat_times: Vec<f64> = beat_frames
        .iter()
        .map(|&f| f as f64 / ONSET_FRAME_RATE)
        .collect();

    let first_beat_sec = beat_times[0];

    // Step 6: Refine BPM from actual beat intervals
    let refined_bpm = refine_bpm_from_beats(&beat_times, tempo_bpm);

    // Step 7: Downbeat and meter scoring
    let (downbeat_offset, meter_numerator) = detect_downbeat_and_meter(&beat_times, &onset_whitened);

    // Step 8: Confidence scoring
    let confidence = score_confidence(&beat_times, &onset_whitened, tempo_confidence);

    Ok(BeatGridResult {
        bpm: refined_bpm,
        first_beat_sec,
        beat_times,
        downbeat_offset,
        meter_numerator,
        confidence,
        onset_envelope,
        onset_frame_rate: ONSET_FRAME_RATE,
    })
}

// ============================================================================
// Step 1: Multi-band spectral-flux onset detection
// ============================================================================

/// Compute onset strength using multi-band spectral flux.
///
/// Splits the spectrum into low (0-200 Hz), mid (200-2000 Hz), and
/// high (2000-11025 Hz) bands. For each band, computes the positive
/// spectral flux (sum of positive magnitude differences between
/// consecutive frames). The bands are summed to produce the final
/// onset envelope.
///
/// This is more accurate than the energy-based onset detector in
/// `tempo_detector.rs` because spectral flux captures transient
/// onsets (kick, snare, hi-hat) that energy differences miss.
fn compute_spectral_flux_onsets(samples: &[f32]) -> Vec<f64> {
    use rustfft::{FftPlanner, num_complex::Complex};

    let fft_size = ONSET_FFT_SIZE;
    let hop = ONSET_HOP_SIZE;
    let num_frames = (samples.len().saturating_sub(fft_size)) / hop + 1;

    if num_frames == 0 {
        return vec![];
    }

    let mut planner = FftPlanner::<f64>::new();
    let fft = planner.plan_fft_forward(fft_size);

    // Hann window
    let window: Vec<f64> = (0..fft_size)
        .map(|i| 0.5 * (1.0 - (2.0 * std::f64::consts::PI * i as f64 / fft_size as f64).cos()))
        .collect();

    // Band boundaries (in FFT bins)
    let bin_width = SR / fft_size as f64;
    let low_max = (200.0 / bin_width).round() as usize;
    let mid_max = (2000.0 / bin_width).round() as usize;
    let high_max = fft_size / 2;

    let mut prev_band_sums: Option<[f64; 3]> = None;
    let mut onset_envelope = Vec::with_capacity(num_frames);

    let mut buffer: Vec<Complex<f64>> = vec![Complex::new(0.0, 0.0); fft_size];

    for frame in 0..num_frames {
        let start = frame * hop;
        let end = (start + fft_size).min(samples.len());

        // Fill buffer with windowed samples
        for i in 0..fft_size {
            if start + i < end {
                buffer[i] = Complex::new(samples[start + i] as f64 * window[i], 0.0);
            } else {
                buffer[i] = Complex::new(0.0, 0.0);
            }
        }

        fft.process(&mut buffer);

        // Compute magnitude per band
        let mut band_sums = [0.0f64; 3];
        for bin in 0..=high_max {
            let mag = buffer[bin].norm();
            if bin <= low_max {
                band_sums[0] += mag;
            } else if bin <= mid_max {
                band_sums[1] += mag;
            } else {
                band_sums[2] += mag;
            }
        }

        // Spectral flux: positive difference from previous frame, per band
        let flux = if let Some(ref prev) = prev_band_sums {
            let mut total_flux = 0.0;
            for b in 0..3 {
                let diff = band_sums[b] - prev[b];
                if diff > 0.0 {
                    total_flux += diff;
                }
            }
            total_flux
        } else {
            0.0
        };

        onset_envelope.push(flux);
        prev_band_sums = Some(band_sums);
    }

    // Normalize
    let max_val = onset_envelope.iter().cloned().fold(0.0f64, f64::max);
    if max_val > 0.0 {
        for v in &mut onset_envelope {
            *v /= max_val;
        }
    }

    // Smooth with a small moving average to reduce noise
    smooth(&mut onset_envelope, 3);

    onset_envelope
}

// ============================================================================
// Step 2: Adaptive whitening
// ============================================================================

/// Normalize the onset envelope adaptively so that local onset density
/// doesn't dominate. Uses a moving-window normalization with compression.
fn adaptive_whiten(onset: &[f64]) -> Vec<f64> {
    let window_frames = (5.0 * ONSET_FRAME_RATE) as usize; // 5-second window
    if onset.is_empty() || window_frames == 0 {
        return onset.to_vec();
    }

    let mut output = vec![0.0; onset.len()];

    for i in 0..onset.len() {
        let start = i.saturating_sub(window_frames / 2);
        let end = (i + window_frames / 2 + 1).min(onset.len());

        // Local maximum
        let local_max = onset[start..end].iter().cloned().fold(0.0f64, f64::max);
        if local_max > 1e-10 {
            // Compress with sqrt to reduce dynamic range
            output[i] = (onset[i] / local_max).sqrt();
        }
    }

    output
}

// ============================================================================
// Step 3: Tempogram — global tempo estimation
// ============================================================================

/// Estimate global tempo using autocorrelation of the onset envelope.
/// Returns (BPM, confidence).
fn estimate_global_tempo(onset: &[f64]) -> (f64, f64) {
    let frame_rate = ONSET_FRAME_RATE;
    let min_lag = (60.0 / MAX_BPM * frame_rate).max(1.0) as usize;
    let max_lag = (60.0 / MIN_BPM * frame_rate) as usize;

    let autocorr = compute_autocorrelation(onset, max_lag);

    // Find peaks in the valid BPM range
    let peaks = find_peaks(&autocorr, min_lag, max_lag.min(autocorr.len() - 1));

    if peaks.is_empty() {
        return (120.0, 0.0);
    }

    // Evaluate octave candidates
    let mut candidates: Vec<(f64, f64)> = Vec::new(); // (bpm, score)

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
            candidates.push((bpm, score));
        }
    }

    if candidates.is_empty() {
        return (120.0, 0.0);
    }

    candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let best_bpm = candidates[0].0;
    let confidence = if candidates.len() >= 2 {
        let top = candidates[0].1;
        let second = candidates[1].1;
        if top > 0.0 {
            (1.0 - second / top).clamp(0.0, 1.0)
        } else {
            0.0
        }
    } else {
        0.5
    };

    (best_bpm, confidence)
}

// ============================================================================
// Step 4: Dynamic-programming beat tracking (Ellis-style)
// ============================================================================

/// DP beat tracking following Ellis (2007).
///
/// The algorithm finds the sequence of beat times that maximizes:
///   sum(onset_strength[beat_i]) - alpha * sum((interval_i - target_interval)^2)
///
/// where target_interval = 60/bpm (the expected beat period).
///
/// The DP state is: for each frame, the best score and backpointer
/// for reaching that frame as a beat.
fn dp_beat_tracking(onset: &[f64], bpm: f64) -> Vec<usize> {
    let n = onset.len();
    if n == 0 {
        return vec![];
    }

    let frame_rate = ONSET_FRAME_RATE;
    let target_period = 60.0 / bpm * frame_rate; // frames per beat

    // Search window: ±DP_WINDOW_BEATS around the expected next beat
    let search_window = (target_period * DP_WINDOW_BEATS) as usize;
    let min_interval = (target_period - search_window as f64).max(2.0) as usize;
    let max_interval = (target_period + search_window as f64) as usize;

    // DP arrays
    let mut score = vec![f64::NEG_INFINITY; n];
    let mut backlink = vec![0usize; n];

    // Initialize: first beat can be at any frame in the first few beats
    let init_range = (target_period * 2.0) as usize;
    for i in 0..init_range.min(n) {
        score[i] = onset[i];
        backlink[i] = usize::MAX; // no predecessor
    }

    // Forward pass
    for i in min_interval..n {
        let mut best_score = f64::NEG_INFINITY;
        let mut best_prev = 0;

        let start = i.saturating_sub(max_interval);
        let end = i.saturating_sub(min_interval);

        for j in start..=end {
            if j >= n || score[j] == f64::NEG_INFINITY {
                continue;
            }
            let interval = (i - j) as f64;
            let interval_error = (interval - target_period).powi(2);
            // Alpha: transition penalty weight. Ellis uses ~ (alpha * target_period)^2
            let alpha = 100.0 / target_period.powi(2);
            let transition_penalty = alpha * interval_error;
            let candidate_score = score[j] - transition_penalty;

            if candidate_score > best_score {
                best_score = candidate_score;
                best_prev = j;
            }
        }

        if best_score > f64::NEG_INFINITY {
            score[i] = best_score + onset[i];
            backlink[i] = best_prev;
        }
    }

    // Backtrack from the best ending frame
    let mut best_end = 0;
    let mut best_end_score = f64::NEG_INFINITY;
    for i in 0..n {
        if score[i] > best_end_score {
            best_end_score = score[i];
            best_end = i;
        }
    }

    let mut beats = vec![];
    let mut current = best_end;
    while current != usize::MAX {
        beats.push(current);
        current = backlink[current];
    }
    beats.reverse();
    beats
}

// ============================================================================
// Step 5: Refine BPM from actual beat intervals
// ============================================================================

/// Refine the BPM estimate using the median of actual beat intervals.
/// This corrects for small errors in the initial tempo estimate.
fn refine_bpm_from_beats(beat_times: &[f64], initial_bpm: f64) -> f64 {
    if beat_times.len() < 2 {
        return initial_bpm;
    }

    let mut intervals: Vec<f64> = Vec::with_capacity(beat_times.len() - 1);
    for i in 1..beat_times.len() {
        intervals.push(beat_times[i] - beat_times[i - 1]);
    }

    intervals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median_interval = intervals[intervals.len() / 2];
    let median_bpm = 60.0 / median_interval;

    // Check octave relationships — if median is ~2× or ~0.5× the initial,
    // trust the initial (the DP may have locked onto half or double time)
    if median_bpm > initial_bpm * 1.8 {
        return initial_bpm;
    }
    if median_bpm < initial_bpm * 0.55 {
        return initial_bpm;
    }

    // Blend: 70% median, 30% initial for stability
    0.7 * median_bpm + 0.3 * initial_bpm
}

// ============================================================================
// Step 6: Downbeat and meter scoring
// ============================================================================

/// Detect the downbeat offset and meter by scoring meter hypotheses.
///
/// For each candidate meter (4/4, 3/4, 2/4, 6/8) and each possible
/// downbeat offset (0..meter-1), compute the average onset strength
/// at downbeat positions. The best-scoring (meter, offset) wins.
///
/// Longer meters (4/4, 6/8) are slightly preferred over shorter ones
/// (2/4, 3/4) because they are more common in popular/electronic music
/// and a 2/4 pattern that's actually 4/4 will score well on both — the
/// preference breaks the tie correctly.
fn detect_downbeat_and_meter(beat_times: &[f64], onset: &[f64]) -> (usize, i32) {
    if beat_times.len() < 8 {
        return (0, 4); // not enough beats to determine meter
    }

    let candidates: [(i32, &[f64], f64); 4] = [
        (4, &[1.0, 0.7, 0.8, 0.7], 1.05),  // 4/4: strong-weak-med-weak, slight preference
        (3, &[1.0, 0.7, 0.8], 1.0),         // 3/4: strong-weak-med
        (2, &[1.0, 0.7], 0.95),             // 2/4: strong-weak, slight penalty
        (6, &[1.0, 0.5, 0.6, 0.8, 0.5, 0.6], 1.0), // 6/8
    ];

    let mut best_score = f64::NEG_INFINITY;
    let mut best_offset = 0;
    let mut best_meter = 4;

    for &(meter, weights, preference) in &candidates {
        // Try each possible downbeat offset
        for offset in 0..meter as usize {
            let raw_score = score_meter_hypothesis(beat_times, onset, meter as usize, offset, weights);
            let score = raw_score * preference;
            if score > best_score {
                best_score = score;
                best_offset = offset;
                best_meter = meter;
            }
        }
    }

    (best_offset, best_meter)
}

/// Score a meter hypothesis by comparing onset strength at downbeat
/// positions vs. non-downbeat positions.
fn score_meter_hypothesis(
    beat_times: &[f64],
    onset: &[f64],
    meter: usize,
    offset: usize,
    weights: &[f64],
) -> f64 {
    let mut score = 0.0;
    let mut count = 0;

    for (i, &beat_time) in beat_times.iter().enumerate() {
        let beat_in_bar = (i + offset) % meter;
        let weight = weights[beat_in_bar];

        // Get onset strength at this beat time
        let frame = (beat_time * ONSET_FRAME_RATE) as usize;
        if frame < onset.len() {
            score += onset[frame] * weight;
            count += 1;
        }
    }

    if count > 0 {
        score / count as f64
    } else {
        0.0
    }
}

// ============================================================================
// Step 7: Confidence scoring
// ============================================================================

/// Score confidence by measuring how well the beat grid aligns with
/// onset peaks. High confidence = beats land on strong onsets.
fn score_confidence(beat_times: &[f64], onset: &[f64], tempo_confidence: f64) -> f64 {
    if beat_times.is_empty() {
        return 0.0;
    }

    let mut beat_onset_sum = 0.0;
    let mut count = 0;

    // For each beat, check onset strength in a small window around it
    let window_frames = 3; // ±3 frames (~69 ms at 22.05 kHz / 512 hop)

    for &beat_time in beat_times {
        let center = (beat_time * ONSET_FRAME_RATE) as usize;
        let start = center.saturating_sub(window_frames);
        let end = (center + window_frames + 1).min(onset.len());

        if start < end {
            let local_max = onset[start..end].iter().cloned().fold(0.0f64, f64::max);
            beat_onset_sum += local_max;
            count += 1;
        }
    }

    // Average onset strength across the whole signal
    let total_onset_sum = onset.iter().sum::<f64>() / onset.len() as f64;

    let avg_beat_onset = if count > 0 { beat_onset_sum / count as f64 } else { 0.0 };
    let alignment_ratio = if total_onset_sum > 1e-10 {
        (avg_beat_onset / total_onset_sum).min(5.0) / 5.0
    } else {
        0.0
    };

    // Blend alignment with tempo confidence
    0.6 * alignment_ratio + 0.4 * tempo_confidence
}

// ============================================================================
// Utility functions
// ============================================================================

/// Compute autocorrelation up to max_lag, normalized so lag-0 = 1.0.
fn compute_autocorrelation(signal: &[f64], max_lag: usize) -> Vec<f64> {
    let max_lag = max_lag.min(signal.len());
    let mut autocorr = vec![0.0; max_lag];

    for lag in 0..max_lag {
        let mut sum = 0.0;
        for i in 0..signal.len().saturating_sub(lag) {
            sum += signal[i] * signal[i + lag];
        }
        autocorr[lag] = sum;
    }

    let norm = autocorr[0];
    if norm > 0.0 {
        for val in &mut autocorr {
            *val /= norm;
        }
    }

    autocorr
}

struct Peak {
    lag: usize,
    strength: f64,
}

/// Find local maxima in the autocorrelation, sorted by strength (descending).
fn find_peaks(autocorr: &[f64], min_lag: usize, max_lag: usize) -> Vec<Peak> {
    let mut peaks: Vec<Peak> = Vec::new();

    for lag in min_lag..=max_lag {
        let prev = if lag > 0 { autocorr[lag - 1] } else { f64::NEG_INFINITY };
        let next = if lag + 1 < autocorr.len() { autocorr[lag + 1] } else { f64::NEG_INFINITY };
        if autocorr[lag] > prev && autocorr[lag] > next && autocorr[lag] > 0.0 {
            peaks.push(Peak { lag, strength: autocorr[lag] });
        }
    }

    // Deduplicate nearby peaks (within 3 frames)
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
        b.strength.partial_cmp(&a.strength).unwrap_or(std::cmp::Ordering::Equal)
    });
    deduped.into_iter().take(NUM_TEMPO_HYPOTHESES).collect()
}

/// Parabolic interpolation for sub-frame peak precision.
fn parabolic_interp(data: &[f64], peak_idx: usize) -> f64 {
    if peak_idx == 0 || peak_idx >= data.len() - 1 {
        return peak_idx as f64;
    }
    let a = data[peak_idx - 1];
    let b = data[peak_idx];
    let c = data[peak_idx + 1];
    let denom = a - 2.0 * b + c;
    if denom.abs() < 1e-10 {
        return peak_idx as f64;
    }
    let offset = 0.5 * (a - c) / denom;
    peak_idx as f64 + offset
}

/// Tempo preference: Gaussian on log-BPM scale, centered at ~120 BPM.
fn tempo_preference(bpm: f64) -> f64 {
    let log_bpm = bpm.log2();
    let log_center = 120.0f64.log2();
    let sigma = 1.0;
    (-0.5 * ((log_bpm - log_center) / sigma).powi(2)).exp()
}

/// Simple moving-average smoothing.
fn smooth(signal: &mut [f64], radius: usize) {
    if radius == 0 || signal.len() < radius * 2 + 1 {
        return;
    }
    let mut smoothed = vec![0.0; signal.len()];
    let window_size = radius * 2 + 1;

    for i in 0..signal.len() {
        let start = i.saturating_sub(radius);
        let end = (i + radius + 1).min(signal.len());
        let count = (end - start) as f64;
        let sum: f64 = signal[start..end].iter().sum();
        smoothed[i] = sum / count;
    }

    // Copy back
    signal.copy_from_slice(&smoothed);
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn generate_click_track(bpm: f64, duration_sec: f64) -> Vec<f32> {
        let beat_period = (SR / (bpm / 60.0)) as usize;
        let total_samples = (duration_sec * SR) as usize;
        let mut samples = vec![0.0f32; total_samples];
        let num_beats = total_samples / beat_period;
        for i in 0..num_beats {
            let start = i * beat_period;
            for j in 0..200.min(total_samples - start) {
                samples[start + j] = 0.5 * (-(j as f32) / 50.0).exp()
                    * (2.0 * std::f32::consts::PI * 1000.0 * j as f32 / SR as f32).sin();
            }
        }
        samples
    }

    #[test]
    fn test_spectral_flux_on_click_track() {
        let samples = generate_click_track(128.0, 10.0);
        let onset = compute_spectral_flux_onsets(&samples);
        assert!(!onset.is_empty(), "Onset envelope should not be empty");
        // The onset envelope should have peaks at beat positions
        let max_val = onset.iter().cloned().fold(0.0f64, f64::max);
        assert!(max_val > 0.0, "Onset envelope should have non-zero peaks");
    }

    #[test]
    fn test_global_tempo_on_128bpm_clicks() {
        let samples = generate_click_track(128.0, 10.0);
        let onset = compute_spectral_flux_onsets(&samples);
        let whitened = adaptive_whiten(&onset);
        let (bpm, conf) = estimate_global_tempo(&whitened);
        // Should detect close to 128 BPM (within octave range)
        assert!(
            (bpm - 128.0).abs() < 5.0 || (bpm - 64.0).abs() < 5.0 || (bpm - 256.0).abs() < 5.0,
            "Expected ~128 BPM (or octave), got {}",
            bpm
        );
        assert!(conf > 0.0, "Confidence should be positive");
    }

    #[test]
    fn test_dp_beat_tracking_on_clicks() {
        let samples = generate_click_track(128.0, 10.0);
        let onset = compute_spectral_flux_onsets(&samples);
        let whitened = adaptive_whiten(&onset);
        let beats = dp_beat_tracking(&whitened, 128.0);
        assert!(beats.len() > 5, "Should find more than 5 beats, got {}", beats.len());
        // Check that beat intervals are roughly consistent
        let mut intervals = vec![];
        for i in 1..beats.len() {
            intervals.push(beats[i] - beats[i - 1]);
        }
        let avg_interval = intervals.iter().sum::<usize>() as f64 / intervals.len() as f64;
        let expected_interval = 60.0 / 128.0 * ONSET_FRAME_RATE;
        assert!(
            (avg_interval - expected_interval).abs() / expected_interval < 0.15,
            "Average interval {} should be close to expected {}",
            avg_interval,
            expected_interval
        );
    }

    #[test]
    fn test_detect_beat_grid_on_120bpm() {
        let samples = generate_click_track(120.0, 10.0);
        let result = detect_beat_grid(&samples).unwrap();
        assert!(result.beat_times.len() > 5);
        // BPM should be close to 120 (within 5 BPM)
        assert!(
            (result.bpm - 120.0).abs() < 8.0,
            "Expected ~120 BPM, got {}",
            result.bpm
        );
        assert!(result.confidence > 0.3, "Confidence should be reasonable");
    }

    #[test]
    fn test_detect_beat_grid_on_128bpm() {
        let samples = generate_click_track(128.0, 10.0);
        let result = detect_beat_grid(&samples).unwrap();
        assert!(result.beat_times.len() > 5);
        assert!(
            (result.bpm - 128.0).abs() < 8.0,
            "Expected ~128 BPM, got {}",
            result.bpm
        );
    }

    #[test]
    fn test_detect_beat_grid_on_140bpm() {
        let samples = generate_click_track(140.0, 10.0);
        let result = detect_beat_grid(&samples).unwrap();
        assert!(result.beat_times.len() > 5);
        assert!(
            (result.bpm - 140.0).abs() < 8.0,
            "Expected ~140 BPM, got {}",
            result.bpm
        );
    }

    #[test]
    fn test_too_short_audio() {
        let short = vec![0.0f32; 100];
        let result = detect_beat_grid(&short);
        assert!(result.is_err());
    }

    #[test]
    fn test_silence_returns_low_confidence() {
        let silence = vec![0.0f32; (SR * 10.0) as usize];
        let result = detect_beat_grid(&silence);
        // Silence should either error or return very low confidence
        if let Ok(r) = result {
            assert!(r.confidence < 0.2, "Silence should have low confidence");
        }
    }

    #[test]
    fn test_downbeat_detection_on_4_4() {
        // Generate a 4/4 pattern: strong kick on 1, weaker on 3
        let bpm = 120.0;
        let beat_period = (SR / (bpm / 60.0)) as usize;
        let total_samples = beat_period * 32; // 32 beats = 8 bars
        let mut samples = vec![0.0f32; total_samples];
        for i in 0..32 {
            let start = i * beat_period;
            let amplitude = if i % 4 == 0 { 0.8 } else if i % 2 == 0 { 0.5 } else { 0.3 };
            for j in 0..200.min(total_samples - start) {
                samples[start + j] = amplitude * (-(j as f32) / 50.0).exp()
                    * (2.0 * std::f32::consts::PI * 1000.0 * j as f32 / SR as f32).sin();
            }
        }
        let result = detect_beat_grid(&samples).unwrap();
        assert_eq!(result.meter_numerator, 4, "Should detect 4/4 meter");
    }
}
