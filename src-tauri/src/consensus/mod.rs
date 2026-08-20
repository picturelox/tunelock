//! Source-agnostic opinion model for key/BPM/energy consensus.
//!
//! Each track can carry multiple opinions from different sources:
//!   - TuneLock (our own analysis engine)
//!   - MIK (Mixed In Key CSV import)
//!   - Traktor (collection.nml import)
//!   - AcoustID/AcousticBrainz (fingerprint-based lookup)
//!
//! Opinions are reconciled into a consensus score that surfaces:
//!   - Agreement (all sources agree → high trust)
//!   - Contested (sources disagree → needs adjudication)
//!   - Unknown (only one source → moderate trust)
//!
//! The consensus indicator is the four-dot display in the library row.

use serde::{Deserialize, Serialize};

/// The source of a key/BPM/energy opinion.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum OpinionSource {
    /// TuneLock's own analysis engine.
    Tunelock,
    /// Mixed In Key CSV import.
    Mik,
    /// Traktor collection.nml import.
    Traktor,
    /// AcoustID/AcousticBrainz fingerprint lookup.
    Acoustid,
}

impl OpinionSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            OpinionSource::Tunelock => "tunelock",
            OpinionSource::Mik => "mik",
            OpinionSource::Traktor => "traktor",
            OpinionSource::Acoustid => "acoustid",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "tunelock" => Some(OpinionSource::Tunelock),
            "mik" => Some(OpinionSource::Mik),
            "traktor" => Some(OpinionSource::Traktor),
            "acoustid" => Some(OpinionSource::Acoustid),
            _ => None,
        }
    }
}

/// A single source's opinion about a track's key, BPM, and energy.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackOpinion {
    pub id: i64,
    pub track_id: i64,
    pub source: OpinionSource,
    pub key_camelot: Option<String>,
    pub key_standard: Option<String>,
    pub bpm: Option<f64>,
    pub energy: Option<i32>,
    /// How confident this source is in its own opinion (0.0–1.0).
    /// For TuneLock, this is the engine's confidence. For MIK, 1.0
    /// (MIK doesn't report uncertainty). For AcoustID, the lookup score.
    pub confidence: f64,
    /// Where this opinion came from (e.g., "MIK CSV 2024-01-13",
    /// "Traktor Pro 3.5", "AcoustID lookup 2024-06-01").
    pub provenance: String,
    pub created_at: String,
}

/// The consensus result for a track, computed from all available opinions.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsensusResult {
    /// Number of sources that have an opinion on this track.
    pub source_count: usize,
    /// Number of sources that agree on the key (Camelot).
    pub key_agreement: usize,
    /// Number of sources that agree on BPM (±1).
    pub bpm_agreement: usize,
    /// The consensus key (majority vote, or the highest-confidence opinion).
    pub consensus_key: Option<String>,
    /// The consensus BPM (median of agreeing sources).
    pub consensus_bpm: Option<f64>,
    /// "agreed" (all sources match), "contested" (disagreement),
    /// "single" (only one source), "unknown" (no opinions).
    pub status: ConsensusStatus,
    /// The per-source opinions, for the four-dot display.
    pub opinions: Vec<TrackOpinion>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ConsensusStatus {
    /// All sources agree on the key.
    Agreed,
    /// Sources disagree on the key.
    Contested,
    /// Only one source has an opinion.
    Single,
    /// No opinions available.
    Unknown,
}

/// Compute consensus from a list of opinions.
///
/// Key agreement is based on Camelot notation — two opinions agree if
/// their Camelot keys match exactly. BPM agreement is ±1 BPM.
pub fn compute_consensus(opinions: &[TrackOpinion]) -> ConsensusResult {
    if opinions.is_empty() {
        return ConsensusResult {
            source_count: 0,
            key_agreement: 0,
            bpm_agreement: 0,
            consensus_key: None,
            consensus_bpm: None,
            status: ConsensusStatus::Unknown,
            opinions: vec![],
        };
    }

    let source_count = opinions.len();

    // Key consensus: count how many agree with the most common Camelot key.
    let mut key_counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for o in opinions {
        if let Some(k) = &o.key_camelot {
            *key_counts.entry(k.as_str()).or_insert(0) += 1;
        }
    }
    let (consensus_key, key_agreement) = key_counts
        .iter()
        .max_by_key(|(_, &c)| c)
        .map(|(k, &c)| (Some(k.to_string()), c))
        .unwrap_or((None, 0));

    // BPM consensus: median of all BPM values, agreement = how many are ±1.
    let bpms: Vec<f64> = opinions.iter().filter_map(|o| o.bpm).collect();
    let consensus_bpm = if bpms.is_empty() {
        None
    } else {
        let mut sorted = bpms.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        Some(sorted[sorted.len() / 2])
    };
    let bpm_agreement = consensus_bpm
        .map(|cb| bpms.iter().filter(|b| (**b - cb).abs() <= 1.0).count())
        .unwrap_or(0);

    // Status: agreed if all sources with a key opinion agree, contested if
    // there's disagreement, single if only one source.
    let sources_with_key = opinions.iter().filter(|o| o.key_camelot.is_some()).count();
    let status = if sources_with_key <= 1 {
        ConsensusStatus::Single
    } else if key_agreement == sources_with_key {
        ConsensusStatus::Agreed
    } else {
        ConsensusStatus::Contested
    };

    ConsensusResult {
        source_count,
        key_agreement,
        bpm_agreement,
        consensus_key,
        consensus_bpm,
        status,
        opinions: opinions.to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_opinion(source: OpinionSource, key: &str, bpm: f64) -> TrackOpinion {
        TrackOpinion {
            id: 0,
            track_id: 1,
            source,
            key_camelot: Some(key.to_string()),
            key_standard: None,
            bpm: Some(bpm),
            energy: None,
            confidence: 0.8,
            provenance: "test".to_string(),
            created_at: "2024-01-01".to_string(),
        }
    }

    #[test]
    fn test_all_agree() {
        let ops = vec![
            make_opinion(OpinionSource::Tunelock, "5A", 128.0),
            make_opinion(OpinionSource::Mik, "5A", 128.0),
            make_opinion(OpinionSource::Traktor, "5A", 128.5),
        ];
        let c = compute_consensus(&ops);
        assert_eq!(c.status, ConsensusStatus::Agreed);
        assert_eq!(c.key_agreement, 3);
        assert_eq!(c.consensus_key, Some("5A".to_string()));
    }

    #[test]
    fn test_contested() {
        let ops = vec![
            make_opinion(OpinionSource::Tunelock, "5A", 128.0),
            make_opinion(OpinionSource::Mik, "12A", 128.0),
        ];
        let c = compute_consensus(&ops);
        assert_eq!(c.status, ConsensusStatus::Contested);
        assert_eq!(c.key_agreement, 1);
    }

    #[test]
    fn test_single_source() {
        let ops = vec![make_opinion(OpinionSource::Tunelock, "5A", 128.0)];
        let c = compute_consensus(&ops);
        assert_eq!(c.status, ConsensusStatus::Single);
    }

    #[test]
    fn test_no_opinions() {
        let c = compute_consensus(&[]);
        assert_eq!(c.status, ConsensusStatus::Unknown);
        assert_eq!(c.source_count, 0);
    }
}
