//! Corpus loading: parse an exported Mixed In Key CSV into normalised labels.
//!
//! MIK exports one row per track with Camelot key ("8A"), tempo, energy and an
//! absolute file Location. We normalise Camelot into `(tonic 0-11, is_major)`,
//! resolve whether the file exists on disk, and classify each row so the bench
//! can report cleanly rather than fail mysteriously.

use std::path::Path;

/// Formats the current symphonia build can decode natively.
/// `.m4a`/`.aif` arrive in Phase 2 via extra codec features + ffmpeg.
const DECODABLE_EXTS: &[&str] = &["mp3", "wav", "flac", "ogg", "opus"];

/// Files larger than this are DJ mixes, not tracks: decoding one into memory
/// at 44.1 kHz mono costs ~1.7 GB per hour of audio. Excluded from the corpus
/// until Phase 2 lands streaming decode.
const MAX_TRACK_BYTES: u64 = 150 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowStatus {
    /// File exists, has a key label, format decodable.
    Ready,
    /// Path in the CSV does not exist on disk (moved drive, deleted file).
    MissingFile,
    /// Extension we cannot decode yet (m4a, aif, video containers…).
    UnsupportedFormat,
    /// MIK left the Key field empty.
    NoKeyLabel,
    /// MIK's own "no stable key" verdict. Scored separately as abstention tests.
    Atonal,
    /// Over the mix-size threshold.
    LargeMix,
}

#[derive(Debug, Clone)]
pub struct CorpusRow {
    pub title: String,
    pub artist: String,
    pub key_camelot: Option<String>,
    pub truth_tonic: Option<usize>,
    pub truth_is_major: Option<bool>,
    pub truth_bpm: Option<f64>,
    pub truth_energy: Option<i32>,
    pub genre: String,
    pub location: String,
    pub extension: String,
    pub status: RowStatus,
}

#[derive(serde::Deserialize)]
struct MikRow {
    #[serde(rename = "Title", default)]
    title: String,
    #[serde(rename = "Artist", default)]
    artist: String,
    #[serde(rename = "Key", default)]
    key: String,
    #[serde(rename = "Tempo", default)]
    tempo: String,
    #[serde(rename = "Genre", default)]
    genre: String,
    #[serde(rename = "Location", default)]
    location: String,
    #[serde(rename = "Energy", default)]
    energy: String,
}

/// Camelot wheel number → major-key tonic. Inverse of
/// `analysis::key_to_camelot`'s major table.
const CAMELOT_NUMBER_TO_MAJOR_TONIC: [usize; 12] = [
    11, // 1  = B
    6,  // 2  = F#
    1,  // 3  = C#
    8,  // 4  = G#
    3,  // 5  = D#
    10, // 6  = A#
    5,  // 7  = F
    0,  // 8  = C
    7,  // 9  = G
    2,  // 10 = D
    9,  // 11 = A
    4,  // 12 = E
];

/// Parse a Camelot code ("8A", "12B") into `(tonic 0-11, is_major)`.
///
/// `nB` is the major key whose tonic is `CAMELOT_NUMBER_TO_MAJOR_TONIC[n-1]`.
/// `nA` is the relative *minor*, 3 semitones below that major tonic.
pub fn camelot_to_key(code: &str) -> Option<(usize, bool)> {
    let code = code.trim();
    if code.len() < 2 {
        return None;
    }
    let (num_str, letter) = code.split_at(code.len() - 1);
    let number: usize = num_str.parse().ok()?;
    if !(1..=12).contains(&number) {
        return None;
    }
    let major_tonic = CAMELOT_NUMBER_TO_MAJOR_TONIC[number - 1];
    match letter {
        "B" => Some((major_tonic, true)),
        "A" => Some(((major_tonic + 12 - 3) % 12, false)),
        _ => None,
    }
}

/// Parse a standard key string like "C minor", "Eb minor", "F# major" into
/// `(tonic 0-11, is_major)`. Handles both sharps and flats.
pub fn parse_standard_key(s: &str) -> Option<(usize, bool)> {
    let s = s.trim();
    let (name, mode) = s.split_once(' ')?;
    let tonic = match name {
        "C" => 0,
        "C#" | "Db" => 1,
        "D" => 2,
        "D#" | "Eb" => 3,
        "E" => 4,
        "F" => 5,
        "F#" | "Gb" => 6,
        "G" => 7,
        "G#" | "Ab" => 8,
        "A" => 9,
        "A#" | "Bb" => 10,
        "B" => 11,
        _ => return None,
    };
    match mode {
        "major" => Some((tonic, true)),
        "minor" => Some((tonic, false)),
        _ => None,
    }
}

/// Load the GiantSteps key dataset: `annotations/key/*.key` +
/// `annotations/genre/*.genre` + `audio/*.mp3`. Rows without downloaded audio
/// are marked MissingFile so the bench can report partial coverage.
pub fn load_giantsteps(root: &Path) -> Vec<CorpusRow> {
    let key_dir = root.join("annotations").join("key");
    let genre_dir = root.join("annotations").join("genre");
    let audio_dir = root.join("audio");

    let mut rows = Vec::new();
    let entries = match std::fs::read_dir(&key_dir) {
        Ok(e) => e,
        Err(_) => return rows,
    };

    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        let stem = match path.file_stem().and_then(|s| s.to_str()) {
            // "<id>.LOFI.key" → stem strips only ".key"
            Some(s) => s.to_string(),
            None => continue,
        };
        let key_text = std::fs::read_to_string(&path).unwrap_or_default();
        let (truth_tonic, truth_is_major) = match parse_standard_key(key_text.trim()) {
            Some((t, m)) => (Some(t), Some(m)),
            None => (None, None),
        };

        let genre = std::fs::read_to_string(genre_dir.join(format!("{}.genre", stem)))
            .map(|g| g.trim().to_string())
            .unwrap_or_default();

        let audio = audio_dir.join(format!("{}.mp3", stem));
        let exists = audio.exists();
        let location = audio.to_string_lossy().to_string();

        let status = if !exists {
            RowStatus::MissingFile
        } else if truth_tonic.is_none() {
            RowStatus::NoKeyLabel
        } else {
            RowStatus::Ready
        };

        let key_camelot = match (truth_tonic, truth_is_major) {
            (Some(t), Some(m)) => Some(crate::analysis::key_to_camelot(t, m)),
            _ => None,
        };

        rows.push(CorpusRow {
            title: stem.clone(),
            artist: String::new(),
            key_camelot,
            truth_tonic,
            truth_is_major,
            truth_bpm: None,
            truth_energy: None,
            genre,
            location,
            extension: "mp3".to_string(),
            status,
        });
    }
    rows
}

/// Load and classify a MIK-export CSV. Never fails hard on a bad row — the row
/// is classified and the bench decides what to do with it.
pub fn load_mik_corpus<P: AsRef<Path>>(path: P) -> anyhow::Result<Vec<CorpusRow>> {
    let mut reader = csv::Reader::from_path(path)?;
    let mut rows = Vec::new();

    for result in reader.deserialize() {
        let raw: MikRow = match result {
            Ok(r) => r,
            Err(_) => continue, // tolerate malformed rows; corpus is user-exported
        };
        if raw.location.trim().is_empty() {
            continue;
        }

        let key_camelot = {
            let k = raw.key.trim().to_string();
            if k.is_empty() { None } else { Some(k) }
        };
        let (truth_tonic, truth_is_major) = match key_camelot.as_deref() {
            Some(k) => match camelot_to_key(k) {
                Some((t, m)) => (Some(t), Some(m)),
                None => (None, None),
            },
            None => (None, None),
        };

        let truth_bpm = raw.tempo.trim().parse::<f64>().ok().filter(|b| *b > 0.0);
        let truth_energy = raw.energy.trim().parse::<i32>().ok();

        let loc = raw.location.trim().to_string();
        let extension = Path::new(&loc)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        let status = classify(&loc, &extension, key_camelot.as_deref());

        rows.push(CorpusRow {
            title: raw.title,
            artist: raw.artist,
            key_camelot,
            truth_tonic,
            truth_is_major,
            truth_bpm,
            truth_energy,
            genre: raw.genre.trim().to_string(),
            location: loc,
            extension,
            status,
        });
    }

    Ok(rows)
}

fn classify(location: &str, extension: &str, key: Option<&str>) -> RowStatus {
    match std::fs::metadata(location) {
        Err(_) => return RowStatus::MissingFile,
        Ok(m) if m.len() > MAX_TRACK_BYTES => return RowStatus::LargeMix,
        _ => {}
    }
    if !DECODABLE_EXTS.contains(&extension) {
        return RowStatus::UnsupportedFormat;
    }
    match key {
        None => RowStatus::NoKeyLabel,
        Some(k) if k == "All" => RowStatus::Atonal,
        Some(_) => RowStatus::Ready,
    }
}

/// Deterministic stratified sample: round-robin across genres so a 500-track
/// sample covers the collection's diversity instead of its head.
pub fn stratified_sample(rows: &[CorpusRow], limit: usize) -> Vec<CorpusRow> {
    let mut by_genre: std::collections::BTreeMap<String, Vec<CorpusRow>> =
        std::collections::BTreeMap::new();
    for row in rows {
        let genre = if row.genre.is_empty() {
            "(unknown)".to_string()
        } else {
            row.genre.to_lowercase()
        };
        by_genre.entry(genre).or_default().push(row.clone());
    }
    // Stable order within each genre for reproducibility.
    for v in by_genre.values_mut() {
        v.sort_by(|a, b| a.location.cmp(&b.location));
    }

    let mut out = Vec::with_capacity(limit.min(rows.len()));
    let mut idx = 0usize;
    'outer: loop {
        let mut added_this_round = false;
        for v in by_genre.values() {
            if idx < v.len() {
                out.push(v[idx].clone());
                added_this_round = true;
                if out.len() >= limit {
                    break 'outer;
                }
            }
        }
        if !added_this_round {
            break;
        }
        idx += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Round-trip: every Camelot code the engine can emit must parse back to
    /// the exact key that produced it.
    #[test]
    fn camelot_parse_inverts_key_to_camelot() {
        for tonic in 0..12 {
            for is_major in [true, false] {
                let code = crate::analysis::key_to_camelot(tonic, is_major);
                let parsed = camelot_to_key(&code)
                    .unwrap_or_else(|| panic!("failed to parse {}", code));
                assert_eq!(
                    parsed,
                    (tonic, is_major),
                    "code {} should parse to tonic={} major={}",
                    code,
                    tonic,
                    is_major
                );
            }
        }
    }

    #[test]
    fn standard_key_parsing() {
        assert_eq!(parse_standard_key("C minor"), Some((0, false)));
        assert_eq!(parse_standard_key("F minor"), Some((5, false)));
        assert_eq!(parse_standard_key("Eb minor"), Some((3, false)));
        assert_eq!(parse_standard_key("Gb major"), Some((6, true)));
        assert_eq!(parse_standard_key("C# minor"), Some((1, false)));
        assert_eq!(parse_standard_key("Bb major"), Some((10, true)));
        assert_eq!(parse_standard_key("H minor"), None);
        assert_eq!(parse_standard_key(""), None);
    }

    #[test]
    fn camelot_parse_handles_known_codes() {
        assert_eq!(camelot_to_key("8A"), Some((9, false))); // A minor
        assert_eq!(camelot_to_key("8B"), Some((0, true))); // C major
        assert_eq!(camelot_to_key("1A"), Some((8, false))); // G# minor
        assert_eq!(camelot_to_key("All"), None);
        assert_eq!(camelot_to_key(""), None);
        assert_eq!(camelot_to_key("13A"), None);
        assert_eq!(camelot_to_key("8C"), None);
    }
}
