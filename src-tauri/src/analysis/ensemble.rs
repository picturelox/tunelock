//! Ensemble key detection.
//!
//! Combines three classical key-profile methods (Krumhansl, Temperley, Sha'ath)
//! with temporal segment voting. Produces a single winning key + confidence.
//!
//! This is Phase 6 of the blueprint. CNN-based detectors (CQT/Mel/HPCP via ONNX)
//! plug in here later behind the `onnx` feature flag — see `crate::analysis::cnn`
//! (not yet implemented; models not yet available offline).

use super::{key_to_camelot, pitch_class_to_name, shaath_major_72, shaath_minor_72, KeyProfiles};
use ndarray::Array2;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy)]
pub struct KeyVote {
    pub tonic: usize,   // 0..12
    pub is_major: bool,
    pub score: f64,     // profile match score (higher = better)
}

#[derive(Debug, Clone, Copy)]
pub struct ProfileWeights {
    pub krumhansl: f64,
    pub temperley: f64,
    pub shaath: f64,
}

impl Default for ProfileWeights {
    fn default() -> Self {
        // Blueprint starting values. Will be updated by MIK calibration later.
        Self { krumhansl: 0.4, temperley: 0.5, shaath: 0.5 }
    }
}

/// Run the three classical profiles on a 12-dim chroma vector (legacy path).
/// Uses the 12-element Sha'ath profile for backward compatibility.
pub fn classical_ensemble(chroma: &[f64; 12], w: ProfileWeights) -> KeyVote {
    let votes = [
        (vote_with_profile(chroma, &KeyProfiles::KRUMHANSL_MAJOR, &KeyProfiles::KRUMHANSL_MINOR), w.krumhansl),
        (vote_with_profile(chroma, &KeyProfiles::TEMPERLEY_MAJOR, &KeyProfiles::TEMPERLEY_MINOR), w.temperley),
        (vote_with_profile(chroma, &KeyProfiles::SHAATH_MAJOR, &KeyProfiles::SHAATH_MINOR), w.shaath),
    ];
    pick_winner(&votes)
}

/// Run the ensemble with Krumhansl + Temperley on 12-dim chroma and
/// Sha'ath on the 72-band Direct Spectral Kernel chroma.
///
/// This is the primary path — it uses the octave-weighted Sha'ath profiles
/// from libKeyFinder for improved accuracy.
pub fn classical_ensemble_dual(chroma12: &[f64; 12], chroma72: &[f64; 72], w: ProfileWeights) -> KeyVote {
    let votes = [
        (vote_with_profile(chroma12, &KeyProfiles::KRUMHANSL_MAJOR, &KeyProfiles::KRUMHANSL_MINOR), w.krumhansl),
        (vote_with_profile(chroma12, &KeyProfiles::TEMPERLEY_MAJOR, &KeyProfiles::TEMPERLEY_MINOR), w.temperley),
        (shaath72_vote(chroma72), w.shaath),
    ];
    pick_winner(&votes)
}

/// Weighted average across vote tables, pick the max.
fn pick_winner(votes: &[(ProfileScores, f64)]) -> KeyVote {
    let mut combined = [[0.0f64; 12]; 2];
    let mut weight_sum = 0.0;
    for (vote_table, weight) in votes {
        weight_sum += weight;
        for (tonic, s) in vote_table.major_scores.iter().enumerate() {
            combined[0][tonic] += s * weight;
        }
        for (tonic, s) in vote_table.minor_scores.iter().enumerate() {
            combined[1][tonic] += s * weight;
        }
    }
    if weight_sum > 0.0 {
        for mode in 0..2 {
            for tonic in 0..12 {
                combined[mode][tonic] /= weight_sum;
            }
        }
    }

    let mut best = KeyVote { tonic: 0, is_major: true, score: f64::NEG_INFINITY };
    for mode in 0..2 {
        for tonic in 0..12 {
            if combined[mode][tonic] > best.score {
                best = KeyVote { tonic, is_major: mode == 0, score: combined[mode][tonic] };
            }
        }
    }
    best
}

struct ProfileScores {
    major_scores: [f64; 12],
    minor_scores: [f64; 12],
}

fn vote_with_profile(chroma: &[f64; 12], maj: &[f64; 12], min: &[f64; 12]) -> ProfileScores {
    let mut major_scores = [0.0; 12];
    let mut minor_scores = [0.0; 12];
    for tonic in 0..12 {
        major_scores[tonic] = cosine_similarity_12(chroma, &rotate(maj, tonic));
        minor_scores[tonic] = cosine_similarity_12(chroma, &rotate(min, tonic));
    }
    ProfileScores { major_scores, minor_scores }
}

fn rotate(profile: &[f64; 12], shift: usize) -> [f64; 12] {
    let mut out = [0.0; 12];
    for i in 0..12 {
        out[i] = profile[(i + 12 - shift) % 12];
    }
    out
}

fn cosine_similarity_12(a: &[f64; 12], b: &[f64; 12]) -> f64 {
    let dot: f64 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f64 = a.iter().map(|x| x * x).sum::<f64>().sqrt();
    let norm_b: f64 = b.iter().map(|x| x * x).sum::<f64>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 { 0.0 } else { dot / (norm_a * norm_b) }
}

/// Cosine similarity for 72-dim vectors.
fn cosine_similarity_72(a: &[f64; 72], b: &[f64; 72]) -> f64 {
    let dot: f64 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f64 = a.iter().map(|x| x * x).sum::<f64>().sqrt();
    let norm_b: f64 = b.iter().map(|x| x * x).sum::<f64>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 { 0.0 } else { dot / (norm_a * norm_b) }
}

/// Rotate a 72-dim profile by `shift` semitones.
/// The profile is organised as 6 octave blocks of 12 semitones each.
/// Rotation shifts within each octave block independently (matching libKeyFinder).
fn rotate_72(profile: &[f64; 72], shift: usize) -> [f64; 72] {
    let mut out = [0.0; 72];
    for o in 0..6 {
        for s in 0..12 {
            let src = o * 12 + ((s + 12 - shift) % 12);
            out[o * 12 + s] = profile[src];
        }
    }
    out
}

/// Classify using the 72-element octave-weighted Sha'ath profiles.
/// Returns 24 scores (12 major + 12 minor), same shape as the 12-dim path.
fn shaath72_vote(chroma72: &[f64; 72]) -> ProfileScores {
    let major_prof = shaath_major_72();
    let minor_prof = shaath_minor_72();
    let mut major_scores = [0.0; 12];
    let mut minor_scores = [0.0; 12];
    for tonic in 0..12 {
        major_scores[tonic] = cosine_similarity_72(chroma72, &rotate_72(&major_prof, tonic));
        minor_scores[tonic] = cosine_similarity_72(chroma72, &rotate_72(&minor_prof, tonic));
    }
    ProfileScores { major_scores, minor_scores }
}

/// A ranked key candidate with the diagnostic information needed to
/// understand WHY the engine picked it. Surfaced in the Tuner UI so that
/// disagreement with reputable tools (Mixed In Key, etc.) is debuggable
/// instead of mysterious.
#[derive(Debug, Clone, Copy)]
pub struct RankedCandidate {
    pub tonic: usize,
    pub is_major: bool,
    /// Final confidence blending segment agreement and profile match score.
    pub confidence: f64,
    /// Fraction of temporal segments that voted for this exact (tonic, mode).
    pub agreement: f64,
    /// Average normalised profile-match score across the segments that voted for it
    /// (in 0..1 after normalising the cosine similarity from [-1,1] to [0,1]).
    pub avg_score: f64,
    /// How many of the N temporal segments selected this candidate.
    pub segment_count: usize,
}

/// Temporal segment voting that returns **all candidates** ranked by confidence,
/// not just the winner. Replaces the old `temporal_vote` (kept as a thin wrapper
/// below for compatibility).
///
/// Why: a high-confidence wrong answer is indistinguishable from a high-confidence
/// right answer when only the winner is exposed. Returning the runners-up lets the
/// UI surface "we picked X but Y came in close", which is critical for debugging
/// dominant/parallel-mode confusions.
/// Temporal segment voting on a 12-dim chromagram (legacy path).
pub fn temporal_vote_ranked(chromagram: &Array2<f64>, segments: usize, w: ProfileWeights) -> Vec<RankedCandidate> {
    let (_, total_frames) = chromagram.dim();
    if total_frames == 0 {
        return vec![];
    }
    let segments = segments.max(1);
    let seg_len = total_frames / segments;

    let mut votes: Vec<KeyVote> = Vec::with_capacity(segments);
    for s in 0..segments {
        let start = s * seg_len;
        let end = if s == segments - 1 { total_frames } else { (s + 1) * seg_len };
        if start >= end { continue; }

        let mut mean = [0.0f64; 12];
        for pc in 0..12 {
            let slice = chromagram.slice(ndarray::s![pc, start..end]);
            mean[pc] = slice.mean().unwrap_or(0.0);
        }
        let norm: f64 = mean.iter().map(|x| x * x).sum::<f64>().sqrt();
        if norm > 0.0 {
            for v in &mut mean { *v /= norm; }
        }
        votes.push(classical_ensemble(&mean, w));
    }

    rank_candidates(&votes)
}

/// Temporal segment voting using both 12-dim and 72-band chromagrams.
/// Krumhansl + Temperley operate on 12-dim; Sha'ath operates on 72-band.
pub fn temporal_vote_ranked_dual(
    chroma12: &Array2<f64>,
    chroma72: &Array2<f64>,
    segments: usize,
    w: ProfileWeights,
) -> Vec<RankedCandidate> {
    let (_, total_frames) = chroma12.dim();
    if total_frames == 0 {
        return vec![];
    }
    let segments = segments.max(1);
    let seg_len = total_frames / segments;

    let mut votes: Vec<KeyVote> = Vec::with_capacity(segments);
    for s in 0..segments {
        let start = s * seg_len;
        let end = if s == segments - 1 { total_frames } else { (s + 1) * seg_len };
        if start >= end { continue; }

        // 12-dim mean
        let mut mean12 = [0.0f64; 12];
        for pc in 0..12 {
            let slice = chroma12.slice(ndarray::s![pc, start..end]);
            mean12[pc] = slice.mean().unwrap_or(0.0);
        }
        let norm: f64 = mean12.iter().map(|x| x * x).sum::<f64>().sqrt();
        if norm > 0.0 {
            for v in &mut mean12 { *v /= norm; }
        }

        // 72-band mean
        let mut mean72 = [0.0f64; 72];
        for b in 0..72 {
            let slice = chroma72.slice(ndarray::s![b, start..end]);
            mean72[b] = slice.mean().unwrap_or(0.0);
        }
        let norm72: f64 = mean72.iter().map(|x| x * x).sum::<f64>().sqrt();
        if norm72 > 0.0 {
            for v in &mut mean72 { *v /= norm72; }
        }

        votes.push(classical_ensemble_dual(&mean12, &mean72, w));
    }

    rank_candidates(&votes)
}

fn rank_candidates(votes: &[KeyVote]) -> Vec<RankedCandidate> {
    if votes.is_empty() {
        return vec![];
    }

    let mut counts: HashMap<(usize, bool), (usize, f64)> = HashMap::new();
    for v in votes {
        let entry = counts.entry((v.tonic, v.is_major)).or_insert((0, 0.0));
        entry.0 += 1;
        entry.1 += v.score;
    }

    let total_segs = votes.len() as f64;
    let mut ranked: Vec<RankedCandidate> = counts
        .into_iter()
        .map(|((tonic, is_major), (count, sum_score))| {
            let agreement = count as f64 / total_segs;
            let avg_score = sum_score / count as f64;
            let normalised_score = ((avg_score + 1.0) / 2.0).clamp(0.0, 1.0);
            let confidence = 0.6 * agreement + 0.4 * normalised_score;
            RankedCandidate { tonic, is_major, confidence, agreement, avg_score: normalised_score, segment_count: count }
        })
        .collect();

    ranked.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(b.segment_count.cmp(&a.segment_count))
            .then(b.avg_score.partial_cmp(&a.avg_score).unwrap_or(std::cmp::Ordering::Equal))
    });
    ranked
}

/// Backwards-compatible single-winner API used by the batch analyser.
pub fn temporal_vote(chromagram: &Array2<f64>, segments: usize, w: ProfileWeights) -> KeyVote {
    match temporal_vote_ranked(chromagram, segments, w).first() {
        Some(c) => KeyVote { tonic: c.tonic, is_major: c.is_major, score: c.confidence },
        None => KeyVote { tonic: 0, is_major: true, score: 0.0 },
    }
}

/// Convenience: produce a human-readable `(standard, camelot, confidence)` triple.
pub fn format_key(vote: KeyVote) -> (String, String, f64) {
    let mode = if vote.is_major { "major" } else { "minor" };
    (
        format!("{} {}", pitch_class_to_name(vote.tonic), mode),
        key_to_camelot(vote.tonic, vote.is_major),
        vote.score,
    )
}
