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
        // Sha'ath 72-band dominates — it's the strongest single method (CQT
        // approximation, octave-weighted, cosine-windowed). Krumhansl and
        // Temperley get low weights but still contribute: they provide
        // valuable mode (major/minor) discrimination that Sha'ath alone
        // lacks (Sha'ath-only drops from 67.8% to 61.4% exact, with parallel
        // mode errors spiking from 13 to 30).
        //
        // Calibrated on the 500-track MIK stratified sample.
        Self { krumhansl: 0.15, temperley: 0.25, shaath: 1.0 }
    }
}

/// Run the three classical profiles on a 12-dim chroma vector (legacy path).
/// Uses the 12-element Sha'ath profile for backward compatibility.
pub fn classical_ensemble(chroma: &[f64; 12], w: ProfileWeights) -> KeyVote {
    let krumhansl_ps = vote_with_profile(chroma, &KeyProfiles::KRUMHANSL_MAJOR, &KeyProfiles::KRUMHANSL_MINOR);
    let temperley_ps = vote_with_profile(chroma, &KeyProfiles::TEMPERLEY_MAJOR, &KeyProfiles::TEMPERLEY_MINOR);
    let shaath_ps = vote_with_profile(chroma, &KeyProfiles::SHAATH_MAJOR, &KeyProfiles::SHAATH_MINOR);

    let mut combined = [[0.0f64; 12]; 2];
    let mut weight_sum = 0.0;
    for (ps, weight) in [(&krumhansl_ps, w.krumhansl), (&temperley_ps, w.temperley), (&shaath_ps, w.shaath)] {
        weight_sum += weight;
        for t in 0..12 {
            combined[0][t] += ps.major_scores[t] * weight;
            combined[1][t] += ps.minor_scores[t] * weight;
        }
    }
    if weight_sum > 0.0 {
        for mode in 0..2 {
            for t in 0..12 {
                combined[mode][t] /= weight_sum;
            }
        }
    }
    pick_winner_from_scores(&combined)
}

/// Run the ensemble with Krumhansl + Temperley on 12-dim chroma and
/// Sha'ath on the 72-band Direct Spectral Kernel chroma.
///
/// This is the primary path — it uses the octave-weighted Sha'ath profiles
/// from libKeyFinder for improved accuracy.
pub fn classical_ensemble_dual(chroma12: &[f64; 12], chroma72: &[f64; 72], w: ProfileWeights) -> KeyVote {
    let scores = combined_scores_dual(chroma12, chroma72, w);
    pick_winner_from_scores(&scores)
}

/// Compute all 24 combined scores (12 major + 12 minor) for the dual path.
/// Returns `[[f64; 12]; 2]` where index 0 = major, 1 = minor.
///
/// This is the core scoring function. It does NOT discard the 23 non-winning
/// scores — they are all returned so that soft temporal aggregation can use
/// them. A key that comes second in every segment still contributes its
/// score to the aggregate, instead of being invisible as in hard voting.
pub fn combined_scores_dual(chroma12: &[f64; 12], chroma72: &[f64; 72], w: ProfileWeights) -> [[f64; 12]; 2] {
    let krumhansl_ps = vote_with_profile(chroma12, &KeyProfiles::KRUMHANSL_MAJOR, &KeyProfiles::KRUMHANSL_MINOR);
    let temperley_ps = vote_with_profile(chroma12, &KeyProfiles::TEMPERLEY_MAJOR, &KeyProfiles::TEMPERLEY_MINOR);
    let shaath_ps = shaath72_vote(chroma72);

    let mut combined = [[0.0f64; 12]; 2];
    let mut weight_sum = 0.0;
    for (ps, weight) in [(&krumhansl_ps, w.krumhansl), (&temperley_ps, w.temperley), (&shaath_ps, w.shaath)] {
        weight_sum += weight;
        for t in 0..12 {
            combined[0][t] += ps.major_scores[t] * weight;
            combined[1][t] += ps.minor_scores[t] * weight;
        }
    }
    if weight_sum > 0.0 {
        for mode in 0..2 {
            for t in 0..12 {
                combined[mode][t] /= weight_sum;
            }
        }
    }
    combined
}

/// Pick the best key from a 24-score table.
fn pick_winner_from_scores(scores: &[[f64; 12]; 2]) -> KeyVote {
    let mut best = KeyVote { tonic: 0, is_major: true, score: f64::NEG_INFINITY };
    for mode in 0..2 {
        for tonic in 0..12 {
            if scores[mode][tonic] > best.score {
                best = KeyVote { tonic, is_major: mode == 0, score: scores[mode][tonic] };
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

    // Log compression of the chroma. This compresses the dynamic range so
    // that the tonic and fifth (the two strongest bins) don't dominate the
    // cosine similarity. Weaker bins — the 3rd, 4th, 6th, and 7th degrees —
    // become more influential. These are the notes that differ between a
    // key and its fifth-neighbour (e.g. C major has F natural, G major has
    // F#), so amplifying their contribution directly combats fifth-
    // substitution errors.
    //
    // The multiplier (LOG_GAIN) controls compression strength. Higher = more
    // compression = weaker bins contribute more. The 500-track benchmark
    // showed that LOG_GAIN=2.0 gives 68.2% exact match with 55 fifth errors.
    // Increasing to 5.0 amplifies the distinguishing pitch classes (F vs F#
    // in the C vs G example) to directly target the fifth-error category.
    const LOG_GAIN: f64 = 5.0;
    let compressed: [f64; 12] = {
        let mut w = [0.0f64; 12];
        for i in 0..12 { w[i] = (1.0 + LOG_GAIN * chroma[i].abs()).ln(); }
        w
    };

    for tonic in 0..12 {
        major_scores[tonic] = cosine_similarity_12(&compressed, &rotate(maj, tonic));
        minor_scores[tonic] = cosine_similarity_12(&compressed, &rotate(min, tonic));
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

/// Pearson correlation for 12-dim vectors. Unlike cosine similarity, this
/// subtracts the mean before computing the dot product, so it measures the
/// *shape* of the distribution rather than the absolute magnitudes.
///
/// Returns [-1, 1]. Currently unused — cosine similarity proved more robust
/// for mode (major/minor) discrimination in corpus testing. Kept for
/// experimentation.
#[allow(dead_code)]
fn pearson_correlation_12(a: &[f64; 12], b: &[f64; 12]) -> f64 {
    let mean_a: f64 = a.iter().sum::<f64>() / 12.0;
    let mean_b: f64 = b.iter().sum::<f64>() / 12.0;
    let mut dot = 0.0;
    let mut norm_a = 0.0;
    let mut norm_b = 0.0;
    for i in 0..12 {
        let da = a[i] - mean_a;
        let db = b[i] - mean_b;
        dot += da * db;
        norm_a += da * da;
        norm_b += db * db;
    }
    if norm_a == 0.0 || norm_b == 0.0 {
        0.0
    } else {
        dot / (norm_a.sqrt() * norm_b.sqrt())
    }
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
///
/// Uses cosine similarity (as libKeyFinder does) plus a tonic-prominence
/// boost computed from the summed octave energy per pitch class.
fn shaath72_vote(chroma72: &[f64; 72]) -> ProfileScores {
    let major_prof = shaath_major_72();
    let minor_prof = shaath_minor_72();
    let mut major_scores = [0.0; 12];
    let mut minor_scores = [0.0; 12];

    // Log compression — same rationale as in vote_with_profile.
    const LOG_GAIN_72: f64 = 3.0;
    let compressed: [f64; 72] = {
        let mut w = [0.0f64; 72];
        for i in 0..72 { w[i] = (1.0 + LOG_GAIN_72 * chroma72[i].abs()).ln(); }
        w
    };

    for tonic in 0..12 {
        major_scores[tonic] = cosine_similarity_72(&compressed, &rotate_72(&major_prof, tonic));
        minor_scores[tonic] = cosine_similarity_72(&compressed, &rotate_72(&minor_prof, tonic));
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

/// **Soft temporal voting** — aggregates all 24 key scores per segment
/// instead of only keeping each segment's winner.
///
/// This fixes the critical issue where a key that comes second in every
/// segment is completely invisible in hard voting. With soft voting, its
/// consistently high scores accumulate and it appears in the ranked list.
///
/// The aggregation works as follows:
/// 1. For each segment, compute all 24 combined scores (12 major + 12 minor).
/// 2. Normalise each segment's scores to [0, 1] so that segments with
///    inherently higher cosine similarities don't dominate.
/// 3. Sum the normalised scores across segments.
/// 4. Rank by the aggregate score.
///
/// Confidence is calibrated as the ratio of the winner's aggregate score
/// to the sum of all aggregate scores. This gives a proper probability-like
/// measure that can be below 0.5 when the winner is uncertain.
pub fn temporal_vote_ranked_dual_soft(
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

    // Accumulate all 24 scores across segments.
    let mut aggregate = [[0.0f64; 12]; 2];
    let mut winner_counts = [[0usize; 12]; 2];
    let mut valid_segments = 0usize;

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

        let scores = combined_scores_dual(&mean12, &mean72, w);
        record_segment_winner(&scores, &mut winner_counts);

        // Normalise this segment's scores to [0, 1] so that segments with
        // inherently higher cosine similarities don't dominate. We shift
        // by the min and divide by the range.
        let min_score = scores.iter().flat_map(|row| row.iter()).copied()
            .fold(f64::INFINITY, f64::min);
        let max_score = scores.iter().flat_map(|row| row.iter()).copied()
            .fold(f64::NEG_INFINITY, f64::max);
        let range = (max_score - min_score).max(1e-10);

        for mode in 0..2 {
            for t in 0..12 {
                aggregate[mode][t] += (scores[mode][t] - min_score) / range;
            }
        }
        valid_segments += 1;
    }

    if valid_segments == 0 {
        return vec![];
    }

    // Build ranked candidates from the aggregate scores.
    let total_score: f64 = aggregate.iter().flat_map(|row| row.iter()).sum();
    if total_score <= 0.0 {
        return vec![];
    }

    let mut ranked: Vec<RankedCandidate> = Vec::with_capacity(24);
    for mode in 0..2 {
        for tonic in 0..12 {
            let is_major = mode == 0;
            let agg_score = aggregate[mode][tonic];
            // Confidence = fraction of total aggregate score. This is
            // a proper probability-like measure: if all 24 keys are equally
            // likely, confidence = 1/24 ≈ 0.042. If one key dominates,
            // confidence approaches 1.0.
            let confidence = agg_score / total_score;
            let segment_count = winner_counts[mode][tonic];
            ranked.push(RankedCandidate {
                tonic,
                is_major,
                confidence,
                agreement: segment_count as f64 / valid_segments as f64,
                avg_score: (agg_score / valid_segments as f64).clamp(0.0, 1.0),
                segment_count,
            });
        }
    }

    ranked.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(b.avg_score.partial_cmp(&a.avg_score).unwrap_or(std::cmp::Ordering::Equal))
    });
    ranked
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
            // Scores are now normalised to [0,1] before the tonic-prominence
            // boost, so they range [0, ~1.35]. Clamp to [0,1].
            let normalised_score = avg_score.clamp(0.0, 1.0);
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

// ============================================================================
// Soft voting variants for ablation
// ============================================================================

/// Soft temporal voting on 12-bin chroma only (Krumhansl + Temperley + Sha'ath-12).
/// Used for ablation: measures the 12-bin path in isolation.
pub fn temporal_vote_ranked_soft_12(
    chroma12: &Array2<f64>,
    segments: usize,
    w: ProfileWeights,
) -> Vec<RankedCandidate> {
    let (_, total_frames) = chroma12.dim();
    if total_frames == 0 {
        return vec![];
    }
    let segments = segments.max(1);
    let seg_len = total_frames / segments;

    let mut aggregate = [[0.0f64; 12]; 2];
    let mut winner_counts = [[0usize; 12]; 2];
    let mut valid_segments = 0usize;

    for s in 0..segments {
        let start = s * seg_len;
        let end = if s == segments - 1 { total_frames } else { (s + 1) * seg_len };
        if start >= end { continue; }

        let mut mean = [0.0f64; 12];
        for pc in 0..12 {
            let slice = chroma12.slice(ndarray::s![pc, start..end]);
            mean[pc] = slice.mean().unwrap_or(0.0);
        }
        let norm: f64 = mean.iter().map(|x| x * x).sum::<f64>().sqrt();
        if norm > 0.0 {
            for v in &mut mean { *v /= norm; }
        }

        // Compute all 24 scores for the 12-bin path
        let krumhansl_ps = vote_with_profile(&mean, &KeyProfiles::KRUMHANSL_MAJOR, &KeyProfiles::KRUMHANSL_MINOR);
        let temperley_ps = vote_with_profile(&mean, &KeyProfiles::TEMPERLEY_MAJOR, &KeyProfiles::TEMPERLEY_MINOR);
        let shaath_ps = vote_with_profile(&mean, &KeyProfiles::SHAATH_MAJOR, &KeyProfiles::SHAATH_MINOR);

        let mut combined = [[0.0f64; 12]; 2];
        let mut weight_sum = 0.0;
        for (ps, weight) in [(&krumhansl_ps, w.krumhansl), (&temperley_ps, w.temperley), (&shaath_ps, w.shaath)] {
            weight_sum += weight;
            for t in 0..12 {
                combined[0][t] += ps.major_scores[t] * weight;
                combined[1][t] += ps.minor_scores[t] * weight;
            }
        }
        if weight_sum > 0.0 {
            for mode in 0..2 {
                for t in 0..12 {
                    combined[mode][t] /= weight_sum;
                }
            }
        }
        record_segment_winner(&combined, &mut winner_counts);

        // Normalise to [0, 1] within this segment
        let min_score = combined.iter().flat_map(|r| r.iter()).copied().fold(f64::INFINITY, f64::min);
        let max_score = combined.iter().flat_map(|r| r.iter()).copied().fold(f64::NEG_INFINITY, f64::max);
        let range = (max_score - min_score).max(1e-10);
        for mode in 0..2 {
            for t in 0..12 {
                aggregate[mode][t] += (combined[mode][t] - min_score) / range;
            }
        }
        valid_segments += 1;
    }

    build_ranked_from_aggregate(&aggregate, &winner_counts, valid_segments)
}

/// Soft temporal voting on 72-band chroma only (Sha'ath-72).
/// Used for ablation: measures the 72-band path in isolation.
pub fn temporal_vote_ranked_soft_72(
    chroma72: &Array2<f64>,
    segments: usize,
    w: ProfileWeights,
) -> Vec<RankedCandidate> {
    let (_, total_frames) = chroma72.dim();
    if total_frames == 0 {
        return vec![];
    }
    let segments = segments.max(1);
    let seg_len = total_frames / segments;

    let mut aggregate = [[0.0f64; 12]; 2];
    let mut winner_counts = [[0usize; 12]; 2];
    let mut valid_segments = 0usize;

    for s in 0..segments {
        let start = s * seg_len;
        let end = if s == segments - 1 { total_frames } else { (s + 1) * seg_len };
        if start >= end { continue; }

        let mut mean72 = [0.0f64; 72];
        for b in 0..72 {
            let slice = chroma72.slice(ndarray::s![b, start..end]);
            mean72[b] = slice.mean().unwrap_or(0.0);
        }
        let norm72: f64 = mean72.iter().map(|x| x * x).sum::<f64>().sqrt();
        if norm72 > 0.0 {
            for v in &mut mean72 { *v /= norm72; }
        }

        let ps = shaath72_vote(&mean72);
        let segment_scores = [ps.major_scores, ps.minor_scores];
        record_segment_winner(&segment_scores, &mut winner_counts);

        // Normalise to [0, 1] within this segment
        let min_score = segment_scores[0].iter().chain(segment_scores[1].iter()).copied()
            .fold(f64::INFINITY, f64::min);
        let max_score = segment_scores[0].iter().chain(segment_scores[1].iter()).copied()
            .fold(f64::NEG_INFINITY, f64::max);
        let range = (max_score - min_score).max(1e-10);
        for t in 0..12 {
            aggregate[0][t] += (segment_scores[0][t] - min_score) / range;
            aggregate[1][t] += (segment_scores[1][t] - min_score) / range;
        }
        valid_segments += 1;
    }

    // Weight is irrelevant for 72-only, but we pass it for consistency
    let _ = w;
    build_ranked_from_aggregate(&aggregate, &winner_counts, valid_segments)
}

/// Record the strongest key in one section. Soft scores still determine the
/// final ranking; these counts are supporting evidence, not a second vote.
fn record_segment_winner(scores: &[[f64; 12]; 2], winner_counts: &mut [[usize; 12]; 2]) {
    let mut best_mode = 0usize;
    let mut best_tonic = 0usize;
    let mut best_score = f64::NEG_INFINITY;

    for mode in 0..2 {
        for tonic in 0..12 {
            if scores[mode][tonic] > best_score {
                best_score = scores[mode][tonic];
                best_mode = mode;
                best_tonic = tonic;
            }
        }
    }

    winner_counts[best_mode][best_tonic] += 1;
}

/// Build the ranked candidate list from an aggregate score table.
fn build_ranked_from_aggregate(
    aggregate: &[[f64; 12]; 2],
    winner_counts: &[[usize; 12]; 2],
    valid_segments: usize,
) -> Vec<RankedCandidate> {
    if valid_segments == 0 {
        return vec![];
    }
    let total_score: f64 = aggregate.iter().flat_map(|row| row.iter()).sum();
    if total_score <= 0.0 {
        return vec![];
    }

    let mut ranked: Vec<RankedCandidate> = Vec::with_capacity(24);
    for mode in 0..2 {
        for tonic in 0..12 {
            let agg_score = aggregate[mode][tonic];
            let confidence = agg_score / total_score;
            let segment_count = winner_counts[mode][tonic];
            ranked.push(RankedCandidate {
                tonic,
                is_major: mode == 0,
                confidence,
                agreement: segment_count as f64 / valid_segments as f64,
                avg_score: (agg_score / valid_segments as f64).clamp(0.0, 1.0),
                segment_count,
            });
        }
    }

    ranked.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(b.avg_score.partial_cmp(&a.avg_score).unwrap_or(std::cmp::Ordering::Equal))
    });
    ranked
}

// ============================================================================
// HPCP + braw/bgate path (Step 5 — separate from the plain-chroma path)
// ============================================================================

/// Which EDM profile family to use with HPCP chroma.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdmProfile {
    /// Faraldo braw — median profiles from Beatport corpus.
    Braw,
    /// Faraldo bgate — braw with 4 least relevant elements zeroed.
    Bgate,
}

/// Vote with a braw/bgate profile set on HPCP chroma.
/// Returns 36 scores: 12 major + 12 minor + 12 "other" (amodal).
fn edm_vote(hpcp: &[f64; 12], profile: EdmProfile) -> [[f64; 12]; 3] {
    let (maj, min, other) = match profile {
        EdmProfile::Braw => (
            &KeyProfiles::BRAW_MAJOR,
            &KeyProfiles::BRAW_MINOR,
            &KeyProfiles::BRAW_OTHER,
        ),
        EdmProfile::Bgate => (
            &KeyProfiles::BGATE_MAJOR,
            &KeyProfiles::BGATE_MINOR,
            &KeyProfiles::BGATE_OTHER,
        ),
    };

    const LOG_GAIN_HPCP: f64 = 5.0;
    let compressed: [f64; 12] = {
        let mut w = [0.0f64; 12];
        for i in 0..12 {
            w[i] = (1.0 + LOG_GAIN_HPCP * hpcp[i].abs()).ln();
        }
        w
    };

    let mut scores = [[0.0f64; 12]; 3];
    for tonic in 0..12 {
        scores[0][tonic] = cosine_similarity_12(&compressed, &rotate(maj, tonic));
        scores[1][tonic] = cosine_similarity_12(&compressed, &rotate(min, tonic));
        scores[2][tonic] = cosine_similarity_12(&compressed, &rotate(other, tonic));
    }
    scores
}

/// Soft temporal voting on HPCP chroma with braw/bgate profiles.
/// Separate EDM path — does NOT touch the plain-chroma Krumhansl/Temperley/
/// Sha'ath path. The "other" profile is folded into minor per Faraldo.
pub fn temporal_vote_edm_soft(
    hpcp: &Array2<f64>,
    segments: usize,
    profile: EdmProfile,
) -> Vec<RankedCandidate> {
    let (_, total_frames) = hpcp.dim();
    if total_frames == 0 {
        return vec![];
    }
    let segments = segments.max(1);
    let seg_len = total_frames / segments;

    let mut aggregate = [[0.0f64; 12]; 2];
    let mut winner_counts = [[0usize; 12]; 2];
    let mut valid_segments = 0usize;

    for s in 0..segments {
        let start = s * seg_len;
        let end = if s == segments - 1 { total_frames } else { (s + 1) * seg_len };
        if start >= end {
            continue;
        }

        let mut mean = [0.0f64; 12];
        for pc in 0..12 {
            let slice = hpcp.slice(ndarray::s![pc, start..end]);
            mean[pc] = slice.mean().unwrap_or(0.0);
        }
        let norm: f64 = mean.iter().map(|x| x * x).sum::<f64>().sqrt();
        if norm > 0.0 {
            for v in &mut mean {
                *v /= norm;
            }
        }

        let scores = edm_vote(&mean, profile);

        let all_scores = scores.iter().flat_map(|row| row.iter()).copied();
        let min_score = all_scores.clone().fold(f64::INFINITY, f64::min);
        let max_score = all_scores.fold(f64::NEG_INFINITY, f64::max);
        let range = (max_score - min_score).max(1e-10);

        let mut segment_scores = [[0.0f64; 12]; 2];
        for t in 0..12 {
            segment_scores[0][t] = scores[0][t];
            segment_scores[1][t] = scores[1][t].max(scores[2][t]);
            aggregate[0][t] += (segment_scores[0][t] - min_score) / range;
            aggregate[1][t] += (segment_scores[1][t] - min_score) / range;
        }
        record_segment_winner(&segment_scores, &mut winner_counts);
        valid_segments += 1;
    }

    build_ranked_from_aggregate(&aggregate, &winner_counts, valid_segments)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn soft_vote_reports_actual_section_winners() {
        let mut chroma12 = Array2::<f64>::zeros((12, 80));
        let mut chroma72 = Array2::<f64>::zeros((72, 80));
        for frame in 0..80 {
            chroma12[(0, frame)] = 1.0;
            for band in (0..72).step_by(12) {
                chroma72[(band, frame)] = 1.0;
            }
        }

        let ranked = temporal_vote_ranked_dual_soft(
            &chroma12,
            &chroma72,
            8,
            ProfileWeights::default(),
        );

        assert_eq!(ranked.iter().map(|candidate| candidate.segment_count).sum::<usize>(), 8);
        assert!(ranked.iter().any(|candidate| candidate.segment_count == 0));
        for candidate in ranked {
            let expected = candidate.segment_count as f64 / 8.0;
            assert!((candidate.agreement - expected).abs() < f64::EPSILON);
        }
    }
}
