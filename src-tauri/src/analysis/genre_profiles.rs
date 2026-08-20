//! Genre-adaptive key profile weights.
//!
//! The literature is unambiguous that genre-specific templates outperform
//! universal ones for template-based key detection. This module selects
//! the appropriate profile weight set based on genre metadata.
//!
//! Genre inference is simple string matching against known genre labels.
//! For tracks without genre metadata, the default (balanced) weights are used.

use super::ensemble::ProfileWeights;

/// Get the appropriate profile weights for a given genre.
///
/// Returns the default balanced weights if the genre is unknown or empty.
pub fn weights_for_genre(genre: Option<&str>) -> ProfileWeights {
    let genre = match genre {
        Some(g) if !g.is_empty() => g.to_lowercase(),
        _ => return ProfileWeights::default(),
    };

    // Electronic genres: Sha'ath (72-band) tends to perform best
    // because electronic music has clear harmonic content.
    if is_electronic(&genre) {
        return ProfileWeights {
            krumhansl: 0.3,
            temperley: 0.4,
            shaath: 0.6,
        };
    }

    // Classical: Krumhansl was designed for classical music and tends to
    // outperform the others there. But classical also benefits from
    // Temperley's corrections.
    if is_classical(&genre) {
        return ProfileWeights {
            krumhansl: 0.6,
            temperley: 0.5,
            shaath: 0.3,
        };
    }

    // Rock/pop: Temperley's profiles were derived from rock/pop corpora
    // and tend to outperform Krumhansl there.
    if is_rock_pop(&genre) {
        return ProfileWeights {
            krumhansl: 0.3,
            temperley: 0.6,
            shaath: 0.4,
        };
    }

    // Hip-hop/R&B: more percussive, Sha'ath's 72-band approach handles
    // the harmonic content better.
    if is_hip_hop(&genre) {
        return ProfileWeights {
            krumhansl: 0.2,
            temperley: 0.4,
            shaath: 0.5,
        };
    }

    // Jazz: Krumhansl was originally validated on jazz standards
    if is_jazz(&genre) {
        return ProfileWeights {
            krumhansl: 0.5,
            temperley: 0.5,
            shaath: 0.3,
        };
    }

    ProfileWeights::default()
}

fn is_electronic(genre: &str) -> bool {
    const ELECTRONIC_KEYWORDS: &[&str] = &[
        "electronic", "edm", "house", "techno", "trance", "dubstep",
        "drum & bass", "dnb", "garage", "bass", "disco", "synth",
        "ambient", "idm", "breakbeat", "electro", "minimal",
    ];
    ELECTRONIC_KEYWORDS.iter().any(|k| genre.contains(k))
}

fn is_classical(genre: &str) -> bool {
    const CLASSICAL_KEYWORDS: &[&str] = &[
        "classical", "orchestral", "symphony", "baroque", "chamber",
        "opera", "piano", "string quartet", "concerto",
    ];
    CLASSICAL_KEYWORDS.iter().any(|k| genre.contains(k))
}

fn is_rock_pop(genre: &str) -> bool {
    const ROCK_POP_KEYWORDS: &[&str] = &[
        "rock", "pop", "indie", "alternative", "punk", "metal",
        "folk", "country", "blues",
    ];
    ROCK_POP_KEYWORDS.iter().any(|k| genre.contains(k))
}

fn is_hip_hop(genre: &str) -> bool {
    const HIP_HOP_KEYWORDS: &[&str] = &[
        "hip", "hop", "rap", "r&b", "rnb", "soul", "trap", "funk",
    ];
    HIP_HOP_KEYWORDS.iter().any(|k| genre.contains(k))
}

fn is_jazz(genre: &str) -> bool {
    const JAZZ_KEYWORDS: &[&str] = &[
        "jazz", "swing", "bebop", "fusion", "smooth jazz",
    ];
    JAZZ_KEYWORDS.iter().any(|k| genre.contains(k))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_electronic_weights() {
        let w = weights_for_genre(Some("Electronic"));
        assert!(w.shaath > w.krumhansl, "Sha'ath should dominate for electronic");
    }

    #[test]
    fn test_classical_weights() {
        let w = weights_for_genre(Some("Classical"));
        assert!(w.krumhansl >= w.shaath, "Krumhansl should be strong for classical");
    }

    #[test]
    fn test_rock_weights() {
        let w = weights_for_genre(Some("Rock"));
        assert!(w.temperley >= w.krumhansl, "Temperley should dominate for rock");
    }

    #[test]
    fn test_unknown_genre_defaults() {
        let w = weights_for_genre(None);
        let default = ProfileWeights::default();
        assert_eq!(w.krumhansl, default.krumhansl);
    }

    #[test]
    fn test_empty_genre_defaults() {
        let w = weights_for_genre(Some(""));
        let default = ProfileWeights::default();
        assert_eq!(w.krumhansl, default.krumhansl);
    }
}
