//! Accuracy metrics: MIREX weighted scoring and error taxonomy.
//!
//! The MIREX key-detection score gives partial credit for musically-adjacent
//! errors, which is what harmonic mixing actually cares about:
//!
//! | Prediction vs truth              | Credit |
//! |----------------------------------|--------|
//! | Correct (tonic + mode)           | 1.0    |
//! | Fifth (tonic ±7, same mode)      | 0.5    |
//! | Relative major/minor             | 0.3    |
//! | Parallel major/minor             | 0.2    |
//! | Anything else                    | 0.0    |

/// The kind of mistake made, for the confusion matrix. Ordered roughly by
/// severity of musical consequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum ErrorType {
    Correct,
    /// Tonic right, mode wrong (e.g. predicted C major, truth C minor).
    Parallel,
    /// Relative major/minor swap (C major ↔ A minor).
    Relative,
    /// Off by a perfect fifth, same mode.
    Fifth,
    /// Tonic off by a semitone.
    Semitone,
    /// None of the above.
    Other,
}

impl ErrorType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ErrorType::Correct => "correct",
            ErrorType::Parallel => "parallel",
            ErrorType::Relative => "relative",
            ErrorType::Fifth => "fifth",
            ErrorType::Semitone => "semitone",
            ErrorType::Other => "other",
        }
    }
}

/// MIREX weighted score in [0, 1].
pub fn mirex_score(
    pred_tonic: usize,
    pred_major: bool,
    truth_tonic: usize,
    truth_major: bool,
) -> f64 {
    if pred_tonic == truth_tonic && pred_major == truth_major {
        return 1.0;
    }
    let diff = (pred_tonic + 12 - truth_tonic) % 12;
    if pred_major == truth_major && (diff == 7 || diff == 5) {
        return 0.5; // perfect fifth up or down
    }
    if truth_major && !pred_major && diff == 9 {
        return 0.3; // relative minor of a major truth
    }
    if !truth_major && pred_major && diff == 3 {
        return 0.3; // relative major of a minor truth
    }
    if pred_tonic == truth_tonic && pred_major != truth_major {
        return 0.2; // parallel mode
    }
    0.0
}

/// Classify the error for the confusion matrix.
pub fn classify_error(
    pred_tonic: usize,
    pred_major: bool,
    truth_tonic: usize,
    truth_major: bool,
) -> ErrorType {
    if pred_tonic == truth_tonic && pred_major == truth_major {
        return ErrorType::Correct;
    }
    if pred_tonic == truth_tonic {
        return ErrorType::Parallel;
    }
    let diff = (pred_tonic + 12 - truth_tonic) % 12;
    if (truth_major && !pred_major && diff == 9) || (!truth_major && pred_major && diff == 3) {
        return ErrorType::Relative;
    }
    if pred_major == truth_major && (diff == 7 || diff == 5) {
        return ErrorType::Fifth;
    }
    if diff == 1 || diff == 11 {
        return ErrorType::Semitone;
    }
    ErrorType::Other
}

/// A Camelot-compatible prediction is any non-zero MIREX score — exact, fifth,
/// relative or parallel all mix acceptably in practice.
pub fn is_camelot_compatible(score: f64) -> bool {
    score > 0.0
}

/// BPM agreement within ±1. When `octave_corrected`, the prediction may be
/// halved or doubled before comparing — measuring how often the detector found
/// the right pulse but the wrong multiple.
pub fn bpm_within_one(pred: f64, truth: f64, octave_corrected: bool) -> bool {
    if !octave_corrected {
        return (pred - truth).abs() <= 1.0;
    }
    [0.5, 1.0, 2.0]
        .iter()
        .any(|m| (pred * m - truth).abs() <= 1.0)
}

/// Signed ratio pred/truth, aggregated to detect a systematic half/double bias.
pub fn bpm_ratio(pred: f64, truth: f64) -> f64 {
    if truth > 0.0 {
        pred / truth
    } else {
        1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mirex_exact() {
        assert_eq!(mirex_score(9, false, 9, false), 1.0); // A minor vs A minor
        assert_eq!(mirex_score(0, true, 0, true), 1.0);
    }

    #[test]
    fn mirex_fifth_both_directions() {
        // A minor (9) truth; E minor (4) is a fifth below A → diff 7.
        assert_eq!(mirex_score(4, false, 9, false), 0.5);
        // C major (0) truth; G major (7) is a fifth up → diff 7.
        assert_eq!(mirex_score(7, true, 0, true), 0.5);
        // Fifth must be same mode: G major vs A minor is NOT a fifth credit.
        assert_eq!(mirex_score(7, true, 9, false), 0.0);
    }

    #[test]
    fn mirex_relative() {
        // Truth C major (0, major); prediction A minor (9, minor) → 0.3.
        assert_eq!(mirex_score(9, false, 0, true), 0.3);
        // Truth A minor (9, minor); prediction C major (0, major) → 0.3.
        assert_eq!(mirex_score(0, true, 9, false), 0.3);
        // Wrong direction is not relative: C major predicted vs C major truth handled above;
        // A major predicted for A minor truth is parallel, not relative.
        assert_eq!(mirex_score(9, true, 9, false), 0.2);
    }

    #[test]
    fn mirex_other_is_zero() {
        assert_eq!(mirex_score(2, true, 9, false), 0.0); // D major vs A minor
    }

    #[test]
    fn error_taxonomy() {
        assert_eq!(classify_error(9, false, 9, false), ErrorType::Correct);
        assert_eq!(classify_error(9, true, 9, false), ErrorType::Parallel);
        assert_eq!(classify_error(0, true, 9, false), ErrorType::Relative);
        assert_eq!(classify_error(4, false, 9, false), ErrorType::Fifth);
        assert_eq!(classify_error(10, false, 9, false), ErrorType::Semitone);
        assert_eq!(classify_error(6, true, 9, false), ErrorType::Other);
    }

    #[test]
    fn bpm_octave_correction() {
        assert!(bpm_within_one(126.0, 126.2, false));
        assert!(!bpm_within_one(63.0, 126.0, false));
        assert!(bpm_within_one(63.0, 126.0, true)); // half-time detected
        assert!(bpm_within_one(252.0, 126.0, true)); // double-time detected
        assert!(!bpm_within_one(63.0, 100.0, true));
    }
}
