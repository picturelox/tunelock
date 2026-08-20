/// TuneLock Harmony — the single source of truth for key/Camelot/relationship
/// logic in Rust. Mirrors `src/lib/harmony.ts` in TypeScript.
///
/// One vocabulary for mixing relationships, one for key/Camelot mapping.
/// No other module should define harmony types or functions.

// ============================================================================
// Pitch class names
// ============================================================================

pub const PITCH_NAMES_SHARP: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

pub const PITCH_NAMES_FLAT: [&str; 12] = [
    "C", "Db", "D", "Eb", "E", "F", "Gb", "G", "Ab", "A", "Bb", "B",
];

pub fn pitch_class_to_name(pitch_class: usize) -> &'static str {
    PITCH_NAMES_SHARP.get(pitch_class % 12).copied().unwrap_or("?")
}

// ============================================================================
// Key <-> Camelot conversion
// ============================================================================

/// Convert a key `(tonic_pitch_class, is_major)` to its Camelot wheel code.
///
/// On the Camelot wheel, a minor key shares its number with its **relative
/// major** (3 semitones higher), e.g. A minor and C major both sit at
/// position 8 (A minor = 8A, C major = 8B).
pub fn key_to_camelot(tonic: usize, is_major: bool) -> String {
    let major_number = |t: usize| -> u8 {
        match t % 12 {
            0 => 8, 1 => 3, 2 => 10, 3 => 5, 4 => 12, 5 => 7,
            6 => 2, 7 => 9, 8 => 4, 9 => 11, 10 => 6, 11 => 1,
            _ => 1,
        }
    };

    let (number, letter) = if is_major {
        (major_number(tonic), 'B')
    } else {
        (major_number((tonic + 3) % 12), 'A')
    };
    format!("{}{}", number, letter)
}

/// Parse a Camelot code string (e.g. "8A") into (number, is_major).
pub fn parse_camelot(camelot: &str) -> Option<(u8, bool)> {
    let s = camelot.trim();
    if s.len() < 2 {
        return None;
    }
    let letter = s.chars().last()?;
    let number_str = &s[..s.len() - 1];
    let number: u8 = number_str.parse().ok()?;
    if number < 1 || number > 12 {
        return None;
    }
    match letter {
        'A' | 'a' => Some((number, false)),
        'B' | 'b' => Some((number, true)),
        _ => None,
    }
}

/// Convert a Camelot code to standard key notation (e.g. "8A" -> "A minor").
pub fn camelot_to_standard(camelot: &str) -> Option<String> {
    let (number, is_major) = parse_camelot(camelot)?;
    // Reverse lookup: find the tonic pitch class for this wheel number.
    let major_tonic = |n: u8| -> Option<usize> {
        match n {
            8 => Some(0), 3 => Some(1), 10 => Some(2), 5 => Some(3),
            12 => Some(4), 7 => Some(5), 2 => Some(6), 9 => Some(7),
            4 => Some(8), 11 => Some(9), 6 => Some(10), 1 => Some(11),
            _ => None,
        }
    };

    if is_major {
        let tonic = major_tonic(number)?;
        Some(format!("{} major", pitch_class_to_name(tonic)))
    } else {
        // Minor tonic is 3 semitones below the relative major.
        let relative_major = major_tonic(number)?;
        let tonic = (relative_major + 12 - 3) % 12;
        Some(format!("{} minor", pitch_class_to_name(tonic)))
    }
}

/// Convert a standard key name (e.g. "A minor") to Camelot code.
pub fn standard_to_camelot(standard: &str) -> Option<String> {
    let s = standard.trim();
    let is_major = if s.ends_with("major") {
        true
    } else if s.ends_with("minor") {
        false
    } else {
        return None;
    };

    // Strip the mode suffix to get the tonic name.
    let tonic_name = if is_major {
        s.strip_suffix("major")?.trim()
    } else {
        s.strip_suffix("minor")?.trim()
    };

    let tonic = PITCH_NAMES_SHARP.iter().position(|&n| n == tonic_name)
        .or_else(|| PITCH_NAMES_FLAT.iter().position(|&n| n == tonic_name))?;

    Some(key_to_camelot(tonic, is_major))
}

// ============================================================================
// Mixing relationships
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelationshipType {
    SameKey,
    Neighbor,
    MoodShift,
    EnergyBoost,
    EnergyDrop,
    Tension,
    BridgeNeeded,
    Unknown,
}

impl RelationshipType {
    pub fn label(&self) -> &'static str {
        match self {
            Self::SameKey => "Same key",
            Self::Neighbor => "Neighbor move",
            Self::MoodShift => "Mood shift",
            Self::EnergyBoost => "Energy boost",
            Self::EnergyDrop => "Energy drop",
            Self::Tension => "Tension jump",
            Self::BridgeNeeded => "Bridge needed",
            Self::Unknown => "Unknown",
        }
    }

    pub fn color(&self) -> &'static str {
        match self {
            Self::SameKey => "#22c55e",
            Self::Neighbor => "#84cc16",
            Self::MoodShift => "#a855f7",
            Self::EnergyBoost => "#f59e0b",
            Self::EnergyDrop => "#3b82f6",
            Self::Tension => "#ef4444",
            Self::BridgeNeeded => "#6b7280",
            Self::Unknown => "#444444",
        }
    }
}

/// Compute the harmonic relationship between two Camelot keys.
pub fn get_camelot_relationship(from_key: &str, to_key: &str) -> RelationshipType {
    let from = match parse_camelot(from_key) {
        Some(p) => p,
        None => return RelationshipType::Unknown,
    };
    let to = match parse_camelot(to_key) {
        Some(p) => p,
        None => return RelationshipType::Unknown,
    };

    if from.0 == to.0 && from.1 == to.1 {
        return RelationshipType::SameKey;
    }

    if from.0 == to.0 && from.1 != to.1 {
        return RelationshipType::MoodShift;
    }

    if from.1 == to.1 {
        let cw = ((to.0 as i32 - from.0 as i32 + 12) % 12) as u8;
        let ccw = ((from.0 as i32 - to.0 as i32 + 12) % 12) as u8;

        if cw == 1 || ccw == 1 {
            return RelationshipType::Neighbor;
        }
        if cw == 2 {
            return RelationshipType::EnergyBoost;
        }
        if ccw == 2 {
            return RelationshipType::EnergyDrop;
        }
        return RelationshipType::Tension;
    }

    RelationshipType::BridgeNeeded
}

// ============================================================================
// Tests (shared test vectors with TS)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// Reference table from the standard Camelot wheel.
    /// (tonic_pitch_class, is_major) -> expected camelot code.
    const EXPECTED_CAMELOT: &[(usize, bool, &str)] = &[
        // Major keys
        (0, true, "8B"), (7, true, "9B"), (2, true, "10B"), (9, true, "11B"),
        (4, true, "12B"), (11, true, "1B"), (6, true, "2B"), (1, true, "3B"),
        (8, true, "4B"), (3, true, "5B"), (10, true, "6B"), (5, true, "7B"),
        // Minor keys
        (9, false, "8A"), (4, false, "9A"), (11, false, "10A"), (6, false, "11A"),
        (1, false, "12A"), (8, false, "1A"), (3, false, "2A"), (10, false, "3A"),
        (5, false, "4A"), (0, false, "5A"), (7, false, "6A"), (2, false, "7A"),
    ];

    #[test]
    fn camelot_mapping_all_24_keys() {
        for &(tonic, is_major, expected) in EXPECTED_CAMELOT {
            let got = key_to_camelot(tonic, is_major);
            assert_eq!(
                got, expected,
                "key_to_camelot({}, {}) = {}, expected {}",
                tonic, is_major, got, expected
            );
        }
    }

    #[test]
    fn camelot_roundtrip() {
        for &(tonic, is_major, camelot) in EXPECTED_CAMELOT {
            let back = camelot_to_standard(camelot).unwrap();
            let expected_name = if is_major {
                format!("{} major", pitch_class_to_name(tonic))
            } else {
                format!("{} minor", pitch_class_to_name(tonic))
            };
            assert_eq!(back, expected_name, "camelot_to_standard({}) failed", camelot);
        }
    }

    #[test]
    fn relationship_same_key() {
        assert_eq!(
            get_camelot_relationship("8A", "8A"),
            RelationshipType::SameKey
        );
    }

    #[test]
    fn relationship_neighbor() {
        assert_eq!(
            get_camelot_relationship("8A", "9A"),
            RelationshipType::Neighbor
        );
        assert_eq!(
            get_camelot_relationship("8A", "7A"),
            RelationshipType::Neighbor
        );
    }

    #[test]
    fn relationship_mood_shift() {
        assert_eq!(
            get_camelot_relationship("8A", "8B"),
            RelationshipType::MoodShift
        );
    }

    #[test]
    fn relationship_energy_boost() {
        assert_eq!(
            get_camelot_relationship("8A", "10A"),
            RelationshipType::EnergyBoost
        );
    }

    #[test]
    fn relationship_energy_drop() {
        assert_eq!(
            get_camelot_relationship("8A", "6A"),
            RelationshipType::EnergyDrop
        );
    }

    #[test]
    fn relationship_bridge_needed() {
        assert_eq!(
            get_camelot_relationship("8A", "5B"),
            RelationshipType::BridgeNeeded
        );
    }

    #[test]
    fn relationship_unknown() {
        assert_eq!(
            get_camelot_relationship("invalid", "8A"),
            RelationshipType::Unknown
        );
    }
}
