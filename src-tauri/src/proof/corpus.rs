//! Corpus loading: parse an exported Mixed In Key CSV into normalised labels.
//!
//! MIK exports one row per track with Camelot key ("8A"), tempo, energy and an
//! absolute file Location. We normalise Camelot into `(tonic 0-11, is_major)`,
//! resolve whether the file exists on disk, and classify each row so the bench
//! can report cleanly rather than fail mysteriously.

use std::path::Path;

/// Formats the current symphonia build + ffmpeg sidecar can decode.
/// Phase 2 added: aac, alac, isomp4 (m4a/mp4), aiff, plus video containers
/// via the ffmpeg sidecar fallback.
const DECODABLE_EXTS: &[&str] = &[
    "mp3", "wav", "flac", "ogg", "oga", "opus", "aiff", "aif", "m4a", "aac",
    "wma", "alac", "mkv",
    // Video containers — audio extracted via ffmpeg sidecar.
    "mp4", "mov", "webm", "m4v", "avi", "flv", "mpg", "mpeg", "ts", "3gp",
];

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

/// Normalize a raw genre string into a small, meaningful taxonomy.
///
/// The MIK corpus contains 439 distinct raw genre strings including typos,
/// website names, combined tags, emojis, and arbitrary metadata. This
/// function maps them to ~15 meaningful categories so that stratified
/// sampling produces a representative sample rather than round-robin over
/// noise.
pub fn normalize_genre(raw: &str) -> &'static str {
    let g = raw.trim().to_lowercase();

    // Empty or placeholder
    if g.is_empty() || g == "unknown" || g == "unknown genre" || g == "unclassifiable" || g == "various" {
        return "unknown";
    }

    // Electronic — broad
    if g.contains("electr") || g.contains("edm") || g.contains("dance") {
        return "electronic";
    }

    // House family
    if g.contains("house") {
        return "house";
    }

    // Techno
    if g.contains("techno") {
        return "techno";
    }

    // Trance
    if g.contains("trance") || g.contains("psy") {
        return "trance";
    }

    // Dubstep / bass music
    if g.contains("dubstep") || g.contains("drum") || g.contains("bass") || g.contains("riddim") {
        return "bass";
    }

    // Hip-hop / rap
    if g.contains("hip") || g.contains("hop") || g.contains("rap") || g.contains("trap") {
        return "hip-hop";
    }

    // R&B / soul / funk
    if g.contains("r&b") || g.contains("r'n'b") || g.contains("r-n-b") || g.contains("soul") || g.contains("funk") {
        return "r&b";
    }

    // Rock
    if g.contains("rock") || g.contains("punk") || g.contains("metal") || g.contains("guitar") {
        return "rock";
    }

    // Pop
    if g.contains("pop") || g.contains("top 40") || g.contains("top40") {
        return "pop";
    }

    // Reggae / dancehall / latin
    if g.contains("reggae") || g.contains("dancehall") || g.contains("shatta") || g.contains("salsa") || g.contains("latin") || g.contains("ragga") {
        return "reggae-latin";
    }

    // Classical / soundtrack
    if g.contains("classical") || g.contains("orchestr") || g.contains("soundtrack") || g.contains("score") || g.contains("theme") || g.contains("anime") {
        return "classical";
    }

    // Jazz / blues
    if g.contains("jazz") || g.contains("blues") || g.contains("swing") {
        return "jazz";
    }

    // Ambient / chill
    if g.contains("chill") || g.contains("ambient") || g.contains("lounge") {
        return "ambient";
    }

    // World / folk
    if g.contains("world") || g.contains("folk") || g.contains("acoustic") || g.contains("country") {
        return "world";
    }

    // Fallback: anything else goes to "other"
    "other"
}

/// A simple, deterministic PRNG (xorshift64) so that sampling is
/// reproducible across platforms without depending on the standard
/// library's platform-specific RNG.
struct SeededRng {
    state: u64,
}

impl SeededRng {
    fn new(seed: u64) -> Self {
        // Avoid the degenerate all-zero state.
        Self { state: seed.max(1) }
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    /// Fisher-Yates shuffle in place.
    fn shuffle<T>(&mut self, slice: &mut [T]) {
        for i in (1..slice.len()).rev() {
            let j = (self.next_u64() % (i as u64 + 1)) as usize;
            slice.swap(i, j);
        }
    }
}

/// Deterministic stratified sample: shuffle within each normalized genre
/// bucket (seeded), then round-robin across genres so the sample covers
/// the collection's diversity.
///
/// **Important:** This replaces the prior broken stratification that used
/// raw genre strings (439 distinct values, 378 with one track). Genres
/// are now normalized to ~15 meaningful categories via `normalize_genre`.
pub fn stratified_sample(rows: &[CorpusRow], limit: usize) -> Vec<CorpusRow> {
    stratified_sample_seeded(rows, limit, 0x542E_4E4C_6F63_6B00)
}

/// Seeded variant for explicit reproducibility.
pub fn stratified_sample_seeded(rows: &[CorpusRow], limit: usize, seed: u64) -> Vec<CorpusRow> {
    // Only sample from Ready rows.
    let ready: Vec<&CorpusRow> = rows.iter().filter(|r| r.status == RowStatus::Ready).collect();
    if ready.is_empty() {
        return Vec::new();
    }

    // Group by normalized genre.
    let mut by_genre: std::collections::BTreeMap<&str, Vec<CorpusRow>> =
        std::collections::BTreeMap::new();
    for row in &ready {
        let genre = normalize_genre(&row.genre);
        by_genre.entry(genre).or_default().push((*row).clone());
    }

    // Seeded shuffle within each genre bucket for reproducibility.
    let mut rng = SeededRng::new(seed);
    for v in by_genre.values_mut() {
        rng.shuffle(v);
    }

    // Round-robin across genre buckets.
    let mut out = Vec::with_capacity(limit.min(ready.len()));
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

    #[test]
    fn genre_normalization_maps_common_strings() {
        assert_eq!(normalize_genre("Tech House"), "house");
        assert_eq!(normalize_genre("Deep House"), "house");
        assert_eq!(normalize_genre("Techno"), "techno");
        assert_eq!(normalize_genre("Trance"), "trance");
        assert_eq!(normalize_genre("Psy-trance"), "trance");
        assert_eq!(normalize_genre("Hip-Hop"), "hip-hop");
        assert_eq!(normalize_genre("rap & hip hop"), "hip-hop");
        assert_eq!(normalize_genre("Trap"), "hip-hop");
        assert_eq!(normalize_genre("R&B"), "r&b");
        assert_eq!(normalize_genre("r'n'b / hip-hop"), "hip-hop");
        assert_eq!(normalize_genre("Rock"), "rock");
        assert_eq!(normalize_genre("rock & roll"), "rock");
        assert_eq!(normalize_genre("Pop"), "pop");
        assert_eq!(normalize_genre("Top 40"), "pop");
        assert_eq!(normalize_genre("Reggae"), "reggae-latin");
        assert_eq!(normalize_genre("Salsa"), "reggae-latin");
        assert_eq!(normalize_genre("Classical"), "classical");
        assert_eq!(normalize_genre("Soundtrack"), "classical");
        assert_eq!(normalize_genre("Jazz"), "jazz");
        assert_eq!(normalize_genre("Blues"), "jazz");
        assert_eq!(normalize_genre("Chill-out"), "ambient");
        assert_eq!(normalize_genre("Ambient"), "ambient");
        assert_eq!(normalize_genre("World"), "world");
        assert_eq!(normalize_genre("Folk"), "world");
        assert_eq!(normalize_genre("Electronic"), "electronic");
        assert_eq!(normalize_genre("EDM"), "electronic");
        assert_eq!(normalize_genre("Dubstep"), "bass");
        assert_eq!(normalize_genre("Drum and Bass"), "bass");
        assert_eq!(normalize_genre(""), "unknown");
        assert_eq!(normalize_genre("unknown genre"), "unknown");
        assert_eq!(normalize_genre("🎉"), "other");
        assert_eq!(normalize_genre("www.electronicfresh.com"), "electronic");
    }

    #[test]
    fn genre_normalization_collapses_to_small_taxonomy() {
        // The MIK corpus has 439 distinct raw genre strings. After
        // normalization they should collapse to ~15 categories.
        let raw_genres = [
            "Tech House", "Deep House", "House", "Soul House", "Tropical House",
            "Techno", "Acid Techno", "Hardcore Hard-Techno",
            "Trance", "Psy-trance",
            "Dubstep", "Drum & Bass", "Riddim Bass",
            "Hip-Hop", "Rap", "Trap", "Underground Rap",
            "R&B", "Soul", "Funk",
            "Rock", "Rock & Roll", "Punk",
            "Pop", "Top 40", "Synthpop",
            "Reggae", "Dancehall", "Salsa", "Reggaeton",
            "Classical", "Soundtrack", "Score",
            "Jazz", "Blues",
            "Chill-out", "Ambient", "Lounge",
            "World", "Folk", "Country",
            "Electronic", "EDM", "Dance",
            "", "Unknown", "Unclassifiable",
            "🎉", "remix-nation.com",
        ];
        let normalized: std::collections::HashSet<&str> =
            raw_genres.iter().map(|g| normalize_genre(g)).collect();
        // Should be well under 20 categories.
        assert!(
            normalized.len() <= 20,
            "expected <= 20 normalized genres, got {}: {:?}",
            normalized.len(),
            normalized
        );
    }

    #[test]
    fn seeded_sample_is_deterministic() {
        fn make_row(genre: &str, location: &str) -> CorpusRow {
            CorpusRow {
                title: location.to_string(),
                artist: String::new(),
                key_camelot: Some("8A".to_string()),
                truth_tonic: Some(9),
                truth_is_major: Some(false),
                truth_bpm: None,
                truth_energy: None,
                genre: genre.to_string(),
                location: location.to_string(),
                extension: "mp3".to_string(),
                status: RowStatus::Ready,
            }
        }

        let rows: Vec<CorpusRow> = (0..100)
            .map(|i| {
                let genre = match i % 5 {
                    0 => "House",
                    1 => "Techno",
                    2 => "Hip-Hop",
                    3 => "Rock",
                    _ => "Pop",
                };
                make_row(genre, &format!("track_{:03}.mp3", i))
            })
            .collect();

        let sample1 = stratified_sample_seeded(&rows, 25, 42);
        let sample2 = stratified_sample_seeded(&rows, 25, 42);

        // Same seed → identical sample.
        assert_eq!(sample1.len(), sample2.len());
        for (a, b) in sample1.iter().zip(sample2.iter()) {
            assert_eq!(a.location, b.location);
        }

        // Different seed → different order (with high probability).
        let sample3 = stratified_sample_seeded(&rows, 25, 999);
        let same = sample1.iter().zip(sample3.iter())
            .filter(|(a, b)| a.location == b.location)
            .count();
        // At least one position should differ.
        assert!(same < 25, "different seeds produced identical samples");
    }

    #[test]
    fn seeded_sample_covers_all_genres() {
        fn make_row(genre: &str, location: &str) -> CorpusRow {
            CorpusRow {
                title: location.to_string(),
                artist: String::new(),
                key_camelot: Some("8A".to_string()),
                truth_tonic: Some(9),
                truth_is_major: Some(false),
                truth_bpm: None,
                truth_energy: None,
                genre: genre.to_string(),
                location: location.to_string(),
                extension: "mp3".to_string(),
                status: RowStatus::Ready,
            }
        }

        // 50 house + 5 jazz. With round-robin, a 20-track sample should
        // include both genres, not just the dominant one.
        let mut rows: Vec<CorpusRow> = (0..50)
            .map(|i| make_row("House", &format!("house_{:02}.mp3", i)))
            .collect();
        rows.extend((0..5).map(|i| make_row("Jazz", &format!("jazz_{:02}.mp3", i))));

        let sample = stratified_sample_seeded(&rows, 20, 42);
        let genres: std::collections::HashSet<&str> =
            sample.iter().map(|r| normalize_genre(&r.genre)).collect();
        assert!(
            genres.contains("house") && genres.contains("jazz"),
            "sample should cover both house and jazz, got: {:?}",
            genres
        );
    }
}
