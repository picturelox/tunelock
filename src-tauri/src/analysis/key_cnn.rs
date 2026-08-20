//! CNN-based key detection via ONNX Runtime.
//!
//! This module loads a trained ONNX model (from the `ml/` Python project)
//! and runs inference on CQT/Mel/HPCP features extracted from audio.
//!
//! The CNN is **lazily loaded** — the model file is only opened on first
//! use, so app startup stays fast. The CNN's output is plugged into the
//! existing ensemble as a weighted vote alongside Krumhansl, Temperley,
//! and Sha'ath.
//!
//! If the ONNX model file is not present, or the `ort` crate is not
//! available, this module gracefully degrades — the classical ensemble
//! continues to work without it.

use serde::{Deserialize, Serialize};

/// CNN key detection result.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CnnKeyResult {
    /// Predicted key index (0–23: 12 major + 12 minor).
    pub key_index: usize,
    /// Key in standard notation (e.g., "A minor").
    pub key_standard: String,
    /// Key in Camelot notation (e.g., "5A").
    pub key_camelot: String,
    /// Confidence (softmax probability of the top prediction).
    pub confidence: f64,
    /// Full probability distribution over 24 keys.
    pub probabilities: Vec<f64>,
}

/// Index to key name mapping (24 keys: 12 major + 12 minor).
const KEY_NAMES: &[(&str, &str)] = &[
    ("C major", "8B"), ("C# major", "3B"), ("D major", "10B"), ("D# major", "5B"),
    ("E major", "12B"), ("F major", "7B"), ("F# major", "2B"), ("G major", "9B"),
    ("G# major", "4B"), ("A major", "11B"), ("A# major", "6B"), ("B major", "1B"),
    ("C minor", "5A"), ("C# minor", "12A"), ("D minor", "7A"), ("D# minor", "2A"),
    ("E minor", "9A"), ("F minor", "4A"), ("F# minor", "11A"), ("G minor", "6A"),
    ("G# minor", "1A"), ("A minor", "8A"), ("A# minor", "3A"), ("B minor", "10A"),
];

/// Check if a CNN model file exists at the given path.
pub fn model_exists(path: &str) -> bool {
    std::path::Path::new(path).exists()
}

/// Run CNN inference on pre-extracted features.
///
/// This is a stub that returns `None` when the ONNX runtime is not
/// available. The actual inference requires the `ort` crate, which
/// is added as an optional dependency when models are ready.
///
/// To enable: add `ort = { version = "2", optional = true }` to Cargo.toml,
/// enable the `cnn` feature, and implement the inference here.
pub fn predict_cnn(_features: &[f32], _model_path: &str) -> Option<CnnKeyResult> {
    // Stub: CNN inference is not yet wired.
    // When models are trained and the `ort` crate is added, this function
    // will load the ONNX model and run inference.
    //
    // The integration plan:
    // 1. Load ONNX model with ort::session::Session
    // 2. Extract CQT/Mel/HPCP features from the audio samples
    // 3. Run inference to get a 24-way probability distribution
    // 4. Return the top prediction with confidence
    None
}

/// Get the key name and Camelot notation for a given index.
pub fn index_to_key(idx: usize) -> Option<(&'static str, &'static str)> {
    KEY_NAMES.get(idx).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_index_mapping() {
        let (std, cam) = index_to_key(0).unwrap();
        assert_eq!(std, "C major");
        assert_eq!(cam, "8B");

        let (std, cam) = index_to_key(21).unwrap();
        assert_eq!(std, "A minor");
        assert_eq!(cam, "8A");
    }

    #[test]
    fn test_model_exists() {
        assert!(!model_exists("nonexistent_model.onnx"));
    }

    #[test]
    fn test_predict_cnn_stub() {
        assert!(predict_cnn(&[0.0; 1000], "model.onnx").is_none());
    }
}
