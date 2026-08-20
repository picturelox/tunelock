//! Local key timeline with modulation boundaries.
//!
//! Instead of collapsing the 8-segment temporal vote to a single winner,
//! this module reports the key *per segment* with timestamps, so the UI
//! can show a timeline strip: "opens 5A, modulates to 12A at 2:14, returns
//! 5A at 4:40".
//!
//! Also includes honest abstention: if no segment has a clear winner
//! (confidence below threshold), the track is labeled "no stable key"
//! rather than guessing.

use serde::{Deserialize, Serialize};

use super::chromagram::{chromagram72_from_spec, chromagram_from_spec, compute_spectrogram};
use super::ensemble::{format_key, temporal_vote_ranked_dual, ProfileWeights, RankedCandidate};
use super::hpss::hpss;
use super::{HPSS_KERNEL, MAX_ANALYSIS_SECONDS, SAMPLE_RATE};

/// Confidence below which we abstain (report "no stable key").
/// Tuned to match MIK's "All" behavior — roughly 5-8% of tracks.
const ABSTENTION_THRESHOLD: f64 = 0.35;

/// A single segment in the key timeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeySegment {
    /// Start time in seconds.
    pub start_sec: f64,
    /// End time in seconds.
    pub end_sec: f64,
    /// Key in standard notation (e.g., "A minor").
    pub key_standard: String,
    /// Key in Camelot notation (e.g., "5A").
    pub key_camelot: String,
    /// Confidence of this segment's key (0.0–1.0).
    pub confidence: f64,
}

/// The complete key timeline for a track.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyTimeline {
    /// Per-segment key results, in temporal order.
    pub segments: Vec<KeySegment>,
    /// The global key (majority vote across segments).
    pub global_key_standard: String,
    pub global_key_camelot: String,
    pub global_confidence: f64,
    /// True if the track has no stable key (all segments below threshold).
    pub abstained: bool,
    /// True if the track modulates (segments disagree on the key).
    pub modulates: bool,
    /// Human-readable modulation summary (e.g., "Opens A minor, shifts to
    /// E minor at 2:14"). Empty if no modulation.
    pub modulation_summary: String,
}

/// Compute a key timeline from mono samples.
pub fn compute_key_timeline(
    samples: &[f32],
    weights: ProfileWeights,
) -> anyhow::Result<KeyTimeline> {
    // Trim to analysis window
    let max_samples = MAX_ANALYSIS_SECONDS * SAMPLE_RATE;
    let samples = if samples.len() <= max_samples {
        samples
    } else {
        let start = (samples.len() - max_samples) / 2;
        &samples[start..start + max_samples]
    };

    let spec = compute_spectrogram(samples)?;
    let (_, frames) = spec.dim();
    if frames == 0 {
        return Ok(KeyTimeline {
            segments: vec![],
            global_key_standard: "unknown".into(),
            global_key_camelot: "".into(),
            global_confidence: 0.0,
            abstained: true,
            modulates: false,
            modulation_summary: "No audio data".into(),
        });
    }

    let (harmonic, _) = hpss(&spec, HPSS_KERNEL);
    let chroma12 = chromagram_from_spec(&harmonic);
    let chroma72 = chromagram72_from_spec(&harmonic);

    // Get the ranked candidates (this already does 8-segment voting)
    let candidates = temporal_vote_ranked_dual(&chroma12, &chroma72, 8, weights);

    // The global key is the top candidate
    let global = candidates.first();
    let (global_key, global_camelot, global_confidence) = match global {
        Some(c) => {
            let vote = super::ensemble::KeyVote {
                tonic: c.tonic,
                is_major: c.is_major,
                score: c.confidence,
            };
            let (s, cam, conf) = format_key(vote);
            (s, cam, conf)
        }
        None => ("unknown".into(), "".into(), 0.0),
    };

    // Build per-segment timeline. We divide the track into 8 equal segments
    // and determine the key for each one independently.
    // This is a simplified approach — a full implementation would use
    // modulation boundary detection, but the 8-segment vote already gives
    // us the raw material.
    let num_segments = 8;
    let frames_per_segment = frames / num_segments;
    let duration_secs = samples.len() as f64 / SAMPLE_RATE as f64;
    let segment_duration = duration_secs / num_segments as f64;

    let mut segments = Vec::with_capacity(num_segments);
    let mut segment_keys: Vec<String> = Vec::new();

    for i in 0..num_segments {
        let start_frame = i * frames_per_segment;
        let end_frame = if i == num_segments - 1 {
            frames
        } else {
            (i + 1) * frames_per_segment
        };

        if end_frame <= start_frame {
            continue;
        }

        // Extract the chroma for this segment
        let seg_chroma12 = chroma12.slice(ndarray::s![.., start_frame..end_frame]).to_owned();
        let seg_chroma72 = chroma72.slice(ndarray::s![.., start_frame..end_frame]).to_owned();

        // Vote on this segment's key
        let seg_candidates = temporal_vote_ranked_dual(&seg_chroma12, &seg_chroma72, 1, weights);
        let seg_top = seg_candidates.first();

        if let Some(c) = seg_top {
            let vote = super::ensemble::KeyVote {
                tonic: c.tonic,
                is_major: c.is_major,
                score: c.confidence,
            };
            let (std, cam, conf) = format_key(vote);

            // Check abstention
            if conf < ABSTENTION_THRESHOLD {
                segment_keys.push("no stable key".into());
                segments.push(KeySegment {
                    start_sec: i as f64 * segment_duration,
                    end_sec: (i + 1) as f64 * segment_duration,
                    key_standard: "no stable key".into(),
                    key_camelot: "".into(),
                    confidence: conf,
                });
            } else {
                segment_keys.push(cam.clone());
                segments.push(KeySegment {
                    start_sec: i as f64 * segment_duration,
                    end_sec: (i + 1) as f64 * segment_duration,
                    key_standard: std,
                    key_camelot: cam,
                    confidence: conf,
                });
            }
        }
    }

    // Determine if the track modulates
    let distinct_keys: Vec<&String> = segment_keys.iter()
        .filter(|k| !k.is_empty() && k.as_str() != "no stable key")
        .collect();
    let distinct_count = distinct_keys.iter().collect::<std::collections::HashSet<_>>().len();
    let modulates = distinct_count > 1;

    // Build modulation summary
    let modulation_summary = if modulates {
        build_modulation_summary(&segments)
    } else {
        String::new()
    };

    // Abstention: all segments below threshold
    let abstained = segments.iter().all(|s| s.key_standard == "no stable key")
        || global_confidence < ABSTENTION_THRESHOLD;

    Ok(KeyTimeline {
        segments,
        global_key_standard: if abstained { "no stable key".into() } else { global_key },
        global_key_camelot: if abstained { "".into() } else { global_camelot },
        global_confidence,
        abstained,
        modulates,
        modulation_summary,
    })
}

fn build_modulation_summary(segments: &[KeySegment]) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut prev_key: Option<&str> = None;

    for seg in segments {
        if seg.key_standard == "no stable key" {
            continue;
        }
        if prev_key.is_some() && prev_key != Some(seg.key_camelot.as_str()) {
            let time = format!("{:.0}:{:02}", seg.start_sec / 60.0, seg.start_sec % 60.0);
            parts.push(format!("shifts to {} at {}", seg.key_standard, time));
        } else if prev_key.is_none() {
            parts.push(format!("Opens {}", seg.key_standard));
        }
        prev_key = Some(seg.key_camelot.as_str());
    }

    parts.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_modulation_summary() {
        let segments = vec![
            KeySegment {
                start_sec: 0.0,
                end_sec: 30.0,
                key_standard: "A minor".into(),
                key_camelot: "5A".into(),
                confidence: 0.8,
            },
            KeySegment {
                start_sec: 30.0,
                end_sec: 60.0,
                key_standard: "E minor".into(),
                key_camelot: "12A".into(),
                confidence: 0.7,
            },
        ];
        let summary = build_modulation_summary(&segments);
        assert!(summary.contains("Opens A minor"));
        assert!(summary.contains("shifts to E minor"));
    }
}
