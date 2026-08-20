pub mod art;
pub mod beat_grid;
pub mod chroma_transform;
pub mod chromagram;
pub mod decoder;
pub mod energy_detector;
pub mod ensemble;
pub mod genre_profiles;
pub mod hpss;
pub mod key_cnn;
pub mod key_detector;
pub mod key_timeline;
pub mod tempo_detector;
pub mod waveform;

// Re-export harmony functions for backward compatibility.
// The canonical home is now `crate::harmony`.
pub use crate::harmony::{key_to_camelot, pitch_class_to_name};

// =============================================================================
// Audio-processing constants
//
// All MIR stages (spectrogram, HPSS, chromagram, tempo) read these. Changing
// them ripples through the whole pipeline.
//
// We run the analysis path at **22.050 kHz** instead of the source-typical
// 44.1 kHz. Rationale:
//
//   * Key detection cares about chroma, which folds the spectrum to 12 pitch
//     classes. Energy above ~11 kHz contributes mostly cymbal hash and
//     barely moves the chroma bins.
//   * 2x fewer samples -> 2x fewer STFT frames -> ~4x less HPSS work
//     (O(frames x bins)), plus a 180-second analysis cap on long tracks.
//   * Tempo detection works fine at 22 kHz -- it operates on energy onsets,
//     not pitch.
//
// Confidence is preserved because the 8-segment temporal ensemble still runs
// across (the analysis window of) the audio with the same Krumhansl /
// Temperley / Sha’ath profiles. We’re cutting compute, not signal.
//
// `FFT_SIZE` and `HOP_SIZE` are matched to the rate so frequency
// resolution is ~1.35 Hz/bin (excellent for chroma — the CQT bins are
// narrow enough to resolve individual semitones even at low frequencies)
// and time resolution is ~186 ms/hop (fine for slowly-varying key).
// =============================================================================
pub const SAMPLE_RATE: usize = 22050;
pub const FFT_SIZE: usize = 16384;
pub const HOP_SIZE: usize = 4096;

pub const BANDS_72: usize = 72;

/// Maximum seconds of audio fed into the spectrogram / HPSS / chroma stages.
/// Longer tracks get a centered window of this length.
///
/// At 22050 Hz / hop=4096, 180 s = ~969 frames. HPSS on that takes well
/// under 1 s in release. Tracks shorter than this are analysed in full.
pub const MAX_ANALYSIS_SECONDS: usize = 180;

/// HPSS median-filter kernel size, in frames.
///
/// At 22050 Hz / hop=4096, one frame is ~186 ms. Kernel=17 covers ~3.15 s,
/// which is longer than the default libKeyFinder kernel. The ablation sweep
/// on GiantSteps (604 tracks) showed:
///   kernel=5:  64.1% (387/604)
///   kernel=9:  63.4% (383/604)  [prior default]
///   kernel=17: 64.4% (389/604)  [best — chosen]
///   kernel=25: 63.4% (383/604)
/// A larger kernel gives cleaner harmonic separation for key detection,
/// which matters more than preserving short key changes on this corpus.
pub const HPSS_KERNEL: usize = 17;

/// Musical key profiles for classical key detection
pub struct KeyProfiles;

impl KeyProfiles {
    /// Krumhansl-Schmuckler key profiles (from psychological studies)
    pub const KRUMHANSL_MAJOR: [f64; 12] = [
        6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88
    ];
    
    pub const KRUMHANSL_MINOR: [f64; 12] = [
        6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17
    ];
    
    /// Temperley key profiles (from music theory)
    pub const TEMPERLEY_MAJOR: [f64; 12] = [
        0.26, 0.04, 0.11, 0.06, 0.16, 0.14, 0.06, 0.18, 0.05, 0.12, 0.05, 0.08
    ];
    
    pub const TEMPERLEY_MINOR: [f64; 12] = [
        0.26, 0.07, 0.12, 0.18, 0.07, 0.12, 0.07, 0.16, 0.14, 0.08, 0.10, 0.09
    ];
    
    /// Sha'ath key profiles (from corpus analysis).
    /// These are 12-element summaries kept for backward compatibility.
    pub const SHAATH_MAJOR: [f64; 12] = [
        0.184, 0.034, 0.061, 0.043, 0.098, 0.088, 0.038, 0.116, 0.037, 0.066, 0.036, 0.053
    ];

    pub const SHAATH_MINOR: [f64; 12] = [
        0.183, 0.047, 0.063, 0.113, 0.047, 0.064, 0.046, 0.097, 0.088, 0.049, 0.060, 0.054
    ];

    /// Faraldo braw profiles — median profiles from Beatport EDM corpus.
    /// Designed for HPCP chroma, not plain chroma.
    /// Source: Faraldo, Jordà, Herrera (2017) via Essentia key.cpp.
    /// There are 3 profiles: major, minor, and "other" (amodal/ambiguous).
    pub const BRAW_MAJOR: [f64; 12] = [
        1.0000, 0.1573, 0.4200, 0.1570, 0.5296, 0.3669, 0.1632, 0.7711, 0.1676, 0.3827, 0.2113, 0.2965
    ];
    pub const BRAW_MINOR: [f64; 12] = [
        1.0000, 0.2330, 0.3615, 0.3905, 0.2925, 0.3777, 0.1961, 0.7425, 0.2701, 0.2161, 0.4228, 0.2272
    ];
    pub const BRAW_OTHER: [f64; 12] = [
        1.0000, 0.2608, 0.3528, 0.2935, 0.4393, 0.3580, 0.2137, 0.7809, 0.2578, 0.2539, 0.3233, 0.2615
    ];

    /// Faraldo bgate profiles — same as braw but with the 4 least relevant
    /// elements zeroed. Essentia's default profile type.
    pub const BGATE_MAJOR: [f64; 12] = [
        1.00, 0.00, 0.42, 0.00, 0.53, 0.37, 0.00, 0.77, 0.00, 0.38, 0.21, 0.30
    ];
    pub const BGATE_MINOR: [f64; 12] = [
        1.00, 0.00, 0.36, 0.39, 0.00, 0.38, 0.00, 0.74, 0.27, 0.00, 0.42, 0.23
    ];
    pub const BGATE_OTHER: [f64; 12] = [
        1.00, 0.26, 0.35, 0.29, 0.44, 0.36, 0.21, 0.78, 0.26, 0.25, 0.32, 0.26
    ];
}

// =============================================================================
// 72-element Sha'ath tone profiles with octave weighting.
//
// These are the original profiles from libKeyFinder `constants.cpp`.
// Each profile is 72 elements (6 octaves × 12 semitones), with per-octave
// weights applied. Index 0 = C1, index 1 = C#1, ..., index 11 = B1,
// index 12 = C2, etc.
//
// Octave weights from libKeyFinder:
//   [0.400, 0.556, 0.525, 0.608, 0.599, 0.491]
// =============================================================================

/// Raw Sha'ath major profile (12 semitones, unweighted).
/// From libKeyFinder `constants.cpp` MAJOR_PROFILE.
const SHAATH_MAJOR_RAW: [f64; 12] = [
    7.239005026181452,  3.503511667251587,  3.584451775366494,
    2.845118164786763,  5.818988921185498,  4.558650574153210,
    2.447788505455065,  6.994731921468295,  3.391066136735049,
    4.556142566551435,  4.073926666635236,  4.459327573788869,
];

/// Raw Sha'ath minor profile (12 semitones, unweighted).
/// From libKeyFinder `constants.cpp` MINOR_PROFILE.
const SHAATH_MINOR_RAW: [f64; 12] = [
    7.002550450602844,  3.143602790159967,  4.359043197149625,
    5.404181207189341,  3.672344208793061,  4.089711849177979,
    3.907914359915540,  6.199602885623165,  3.634246256252774,
    2.872411910798756,  5.354679997945427,  3.832420385950484,
];

/// Per-octave weights from libKeyFinder `constants.cpp`.
const OCTAVE_WEIGHTS: [f64; 6] = [
    0.3999726755, 0.556344252483, 0.524966363451,
    0.608475483843, 0.5989811568, 0.49072435318,
];

/// Build the full 72-element Sha'ath major profile (octave-weighted).
pub fn shaath_major_72() -> [f64; 72] {
    let mut profile = [0.0f64; 72];
    for o in 0..6 {
        for s in 0..12 {
            profile[o * 12 + s] = OCTAVE_WEIGHTS[o] * SHAATH_MAJOR_RAW[s];
        }
    }
    profile
}

/// Build the full 72-element Sha'ath minor profile (octave-weighted).
pub fn shaath_minor_72() -> [f64; 72] {
    let mut profile = [0.0f64; 72];
    for o in 0..6 {
        for s in 0..12 {
            profile[o * 12 + s] = OCTAVE_WEIGHTS[o] * SHAATH_MINOR_RAW[s];
        }
    }
    profile
}

// `pitch_class_to_name` and `key_to_camelot` have moved to `crate::harmony`.
// The `pub use` re-export at the top of this file keeps existing callers
// working. The Camelot test vectors now live in `harmony/mod.rs`.
// Camelot tests have moved to `harmony/mod.rs` — the canonical home for
// all key/Camelot/relationship logic and test vectors.
