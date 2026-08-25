// TrackIntelligenceSnapshot — the sole contract between Core Intelligence
// (non-real-time analysis) and the Performance Engine (real-time playback).
//
// Core Intelligence owns models, selectors, embeddings, and calibration.
// The Performance Engine owns audio buffers, transport, DSP, and routing.
// This snapshot is the ONLY thing that crosses the boundary.
//
// Rules:
//   - The snapshot is immutable. Analysis publishes a new snapshot; playback
//     consumes the latest completed one. No in-place mutation.
//   - The real-time callback never blocks on analysis. Playback must never
//     wait for key/BPM readout.
//   - Confidence is honest: the tier reflects measured model agreement on
//     held-out data, not an invented formula.
//
// The canonical 24-key vocabulary (indices 0-23) is shared with the Rust
// harmony/ module and the TypeScript lib/harmony.ts mirror.

use serde::{Deserialize, Serialize};

/// Current schema version. Bump when fields change; readers must handle
/// older versions gracefully.
pub const SNAPSHOT_SCHEMA_VERSION: u32 = 1;

/// Canonical 24-key labels, indices 0-23.
/// 0-11 = majors (C, C#, D, D#, E, F, F#, G, G#, A, A#, B)
/// 12-23 = minors (Cm, C#m, Dm, D#m, Em, Fm, F#m, Gm, G#m, Am, A#m, Bm)
pub const KEY_LABELS: [&str; 24] = [
    "C major", "C# major", "D major", "D# major", "E major", "F major",
    "F# major", "G major", "G# major", "A major", "A# major", "B major",
    "C minor", "C# minor", "D minor", "D# minor", "E minor", "F minor",
    "F# minor", "G minor", "G# minor", "A minor", "A# minor", "B minor",
];

/// Camelot wheel labels, same ordering as KEY_LABELS.
pub const CAMELOT_LABELS: [&str; 24] = [
    "8B", "3B", "10B", "5B", "12B", "7B", "2B", "9B", "4B", "11B", "6B", "1B",
    "5A", "12A", "7A", "2A", "9A", "4A", "11A", "6A", "1A", "8A", "3A", "10A",
];

/// Confidence tier derived from measured model agreement on held-out data.
///
/// Calibrated on FMAK (4,908 tracks, 7-model ensemble):
///   - High:   all models agree. 84.7% measured accuracy, ~48% coverage.
///   - Medium: >=60% of models agree. 75.8% measured accuracy, ~81% coverage.
///   - Low:    models disagree. Selector fallback, ~19% coverage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConfidenceTier {
    High,
    Medium,
    Low,
    /// Analysis has not completed or no models have reported.
    Unknown,
}

/// A single key alternative with its score.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyAlternative {
    /// Index into the canonical 24-key vocabulary (0-23).
    pub key_index: u8,
    /// Posterior mass for this key (0.0-1.0).
    pub score: f32,
}

/// Beat grid information for quantized launch and sync.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeatGridInfo {
    pub bpm: f64,
    pub first_beat_sec: f64,
    pub meter_numerator: i32,
    /// Index of the first downbeat within the beat sequence (0 = first beat
    /// is a downbeat).
    pub downbeat_offset: usize,
}

/// The immutable snapshot of everything Core Intelligence knows about a track.
///
/// Published atomically (Arc swap) by the analysis side. The playback side
/// reads the latest completed snapshot without blocking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackIntelligenceSnapshot {
    /// Schema version for forward/backward compatibility.
    pub schema_version: u32,
    /// Analysis revision — increments each time analysis produces a new
    /// result for this track. Playback can detect staleness.
    pub analysis_revision: u64,

    // ── Tempo & rhythm ─────────────────────────────────────────────
    /// Beat grid (BPM, first beat, meter, downbeat offset).
    /// None if analysis has not completed.
    pub beat_grid: Option<BeatGridInfo>,

    // ── Key ────────────────────────────────────────────────────────
    /// Primary key: index into the canonical 24-key vocabulary.
    pub primary_key_index: Option<u8>,
    /// Full 24-class posterior from the selector (normalized, sums to ~1.0).
    /// All zeros if analysis has not completed.
    pub key_posterior: [f32; 24],
    /// Ranked alternatives (top-N, descending score, excluding primary).
    pub alternatives: Vec<KeyAlternative>,
    /// Confidence tier from model agreement (calibrated on FMAK).
    pub confidence_tier: ConfidenceTier,
    /// Raw model agreement fraction (0.0-1.0). 1.0 = all models agree.
    pub model_agreement: f32,

    // ── Energy ─────────────────────────────────────────────────────
    /// Per-section energy (0.0-1.0 normalized), if computed.
    /// Placeholder for the Transition Workbench's energy-aware features.
    pub energy_curve: Option<Vec<f32>>,

    // ── Future fields (reserved) ───────────────────────────────────
    /// Local key movement map (key index per section). Future.
    pub local_key_map: Option<Vec<u8>>,
    /// Integrated loudness (LUFS). Future.
    pub loudness_lufs: Option<f64>,
}

impl Default for TrackIntelligenceSnapshot {
    fn default() -> Self {
        Self {
            schema_version: SNAPSHOT_SCHEMA_VERSION,
            analysis_revision: 0,
            beat_grid: None,
            primary_key_index: None,
            key_posterior: [0.0; 24],
            alternatives: Vec::new(),
            confidence_tier: ConfidenceTier::Unknown,
            model_agreement: 0.0,
            energy_curve: None,
            local_key_map: None,
            loudness_lufs: None,
        }
    }
}

impl TrackIntelligenceSnapshot {
    /// Create a snapshot from a key analysis result.
    pub fn from_key_analysis(
        revision: u64,
        posterior: [f32; 24],
        agreement: f32,
    ) -> Self {
        let primary = posterior
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
            .filter(|(_, &v)| v > 0.0)
            .map(|(i, _)| i as u8);

        let mut alternatives: Vec<KeyAlternative> = posterior
            .iter()
            .enumerate()
            .filter(|(i, &v)| Some(*i as u8) != primary && v > 0.01)
            .map(|(i, &v)| KeyAlternative {
                key_index: i as u8,
                score: v,
            })
            .collect();
        alternatives.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        alternatives.truncate(3);

        let confidence_tier = if agreement >= 0.99 {
            ConfidenceTier::High
        } else if agreement >= 0.6 {
            ConfidenceTier::Medium
        } else {
            ConfidenceTier::Low
        };

        Self {
            schema_version: SNAPSHOT_SCHEMA_VERSION,
            analysis_revision: revision,
            beat_grid: None,
            primary_key_index: primary,
            key_posterior: posterior,
            alternatives,
            confidence_tier,
            model_agreement: agreement,
            energy_curve: None,
            local_key_map: None,
            loudness_lufs: None,
        }
    }

    /// The primary key's standard label (e.g., "A minor").
    pub fn primary_key_label(&self) -> Option<&'static str> {
        self.primary_key_index.map(|i| KEY_LABELS[i as usize])
    }

    /// The primary key's Camelot label (e.g., "8A").
    pub fn primary_camelot(&self) -> Option<&'static str> {
        self.primary_key_index.map(|i| CAMELOT_LABELS[i as usize])
    }

    /// True if the snapshot contains usable key information.
    pub fn has_key(&self) -> bool {
        self.primary_key_index.is_some()
    }

    /// True if the snapshot contains usable beat grid information.
    pub fn has_beat_grid(&self) -> bool {
        self.beat_grid.is_some()
    }

    /// True if this snapshot is newer than another (by analysis revision).
    pub fn is_newer_than(&self, other: &Self) -> bool {
        self.analysis_revision > other.analysis_revision
    }
}

/// Compute harmonic compatibility between two key indices using the
/// posterior-weighted musical relationship matrix.
///
/// Compatibility(A, B) = sum over all (i, j) of P(A=i) * P(B=j) * compat(i, j)
///
/// where compat(i, j) is:
///   1.0 — same key
///   0.8 — perfect fifth (Camelot ±1)
///   0.7 — relative major/minor
///   0.6 — parallel major/minor (same tonic, different mode)
///   0.4 — semitone apart (same mode)
///   0.0 — otherwise
pub fn compatibility(a_posterior: &[f32; 24], b_posterior: &[f32; 24]) -> f64 {
    let mut total = 0.0f64;
    for i in 0..24 {
        for j in 0..24 {
            total += (a_posterior[i] as f64) * (b_posterior[j] as f64) * key_compat(i, j);
        }
    }
    total
}

/// Musical compatibility between two canonical key indices (0-23).
fn key_compat(a: usize, b: usize) -> f64 {
    if a == b {
        return 1.0;
    }
    let a_tonic = a % 12;
    let b_tonic = b % 12;
    let a_minor = a >= 12;
    let b_minor = b >= 12;
    let semitone_dist = ((a_tonic as i32 - b_tonic as i32).abs()).min(12 - (a_tonic as i32 - b_tonic as i32).abs());

    // Same mode
    if a_minor == b_minor {
        match semitone_dist {
            7 | 5 => 0.8,  // Perfect fifth (Camelot ±1)
            2 => 0.3,       // Whole tone (energy shift)
            1 | 11 => 0.4,  // Semitone (tension)
            _ => 0.0,
        }
    } else {
        // Different mode
        match semitone_dist {
            3 => 0.7,  // Relative major/minor (Am ↔ C)
            9 => 0.7,  // Relative minor/major (C ↔ Am)
            0 => 0.6,  // Parallel (C ↔ Cm)
            _ => 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn single_key_posterior(index: usize) -> [f32; 24] {
        let mut p = [0.0f32; 24];
        p[index] = 1.0;
        p
    }

    #[test]
    fn snapshot_defaults_are_empty() {
        let s = TrackIntelligenceSnapshot::default();
        assert_eq!(s.schema_version, SNAPSHOT_SCHEMA_VERSION);
        assert!(!s.has_key());
        assert!(!s.has_beat_grid());
        assert_eq!(s.confidence_tier, ConfidenceTier::Unknown);
    }

    #[test]
    fn from_key_analysis_picks_primary() {
        let mut p = [0.0f32; 24];
        p[21] = 0.6; // A minor (index 21)
        p[0] = 0.3;  // C major (index 0)
        p[7] = 0.1;  // G major (index 7)

        let s = TrackIntelligenceSnapshot::from_key_analysis(1, p, 0.85);
        assert_eq!(s.primary_key_index, Some(21));
        assert_eq!(s.primary_key_label(), Some("A minor"));
        assert_eq!(s.primary_camelot(), Some("8A"));
        assert_eq!(s.confidence_tier, ConfidenceTier::Medium);
        assert_eq!(s.alternatives.len(), 2);
        assert_eq!(s.alternatives[0].key_index, 0); // highest-scoring alternative first
    }

    #[test]
    fn confidence_tier_thresholds() {
        let p = single_key_posterior(0);
        assert_eq!(
            TrackIntelligenceSnapshot::from_key_analysis(1, p, 1.0).confidence_tier,
            ConfidenceTier::High
        );
        assert_eq!(
            TrackIntelligenceSnapshot::from_key_analysis(1, p, 0.7).confidence_tier,
            ConfidenceTier::Medium
        );
        assert_eq!(
            TrackIntelligenceSnapshot::from_key_analysis(1, p, 0.3).confidence_tier,
            ConfidenceTier::Low
        );
    }

    #[test]
    fn revision_ordering() {
        let a = TrackIntelligenceSnapshot::from_key_analysis(1, single_key_posterior(0), 1.0);
        let b = TrackIntelligenceSnapshot::from_key_analysis(2, single_key_posterior(0), 1.0);
        assert!(b.is_newer_than(&a));
        assert!(!a.is_newer_than(&b));
    }

    #[test]
    fn identical_keys_have_unit_compatibility() {
        let p = single_key_posterior(21); // A minor
        let c = compatibility(&p, &p);
        assert!((c - 1.0).abs() < 1e-9, "same key must be 1.0, got {c}");
    }

    #[test]
    fn relative_compatibility() {
        let a_minor = single_key_posterior(21); // A minor
        let c_major = single_key_posterior(0);  // C major
        let c = compatibility(&a_minor, &c_major);
        assert!((c - 0.7).abs() < 1e-9, "relative keys must be 0.7, got {c}");
    }

    #[test]
    fn fifth_compatibility() {
        let a_minor = single_key_posterior(21); // A minor
        let e_minor = single_key_posterior(16); // E minor (fifth below)
        let c = compatibility(&a_minor, &e_minor);
        assert!((c - 0.8).abs() < 1e-9, "fifth must be 0.8, got {c}");
    }

    #[test]
    fn uncertain_key_spreads_compatibility() {
        // A track with 50% A minor + 50% C major vs a certain A minor:
        // compatibility = 0.5*1.0 + 0.5*0.7 = 0.85
        let mut uncertain = [0.0f32; 24];
        uncertain[21] = 0.5; // A minor
        uncertain[0] = 0.5;  // C major
        let certain = single_key_posterior(21); // A minor
        let c = compatibility(&uncertain, &certain);
        assert!((c - 0.85).abs() < 1e-9, "uncertain key compat: expected 0.85, got {c}");
    }

    #[test]
    fn snapshot_serializes() {
        let s = TrackIntelligenceSnapshot::from_key_analysis(42, single_key_posterior(20), 0.95);
        let json = serde_json::to_string(&s).unwrap();
        let roundtrip: TrackIntelligenceSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(roundtrip.primary_key_index, Some(20));
        assert_eq!(roundtrip.analysis_revision, 42);
    }
}
