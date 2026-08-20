// DJ setlist analysis — parse a pasted tracklist, identify tracks,
// match against the local library, and show the harmonic flow.
//
// This is the most novel Assist feature: it lets the user study other
// DJs' sets as harmonic journeys. The user pastes a tracklist (e.g.,
// from a SoundCloud or YouTube description), the LLM parses it into
// structured data, and we match each track against the local library.
//
// For tracks not in the library, we create a "reference entry" — a
// catalog item with just the metadata (artist, title, key if known)
// that can be placed in a mix plan before the user owns the file.
// This connects to Phase 12 (Cloud/URL reference library).

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::ollama::{ChatMessage, OllamaClient};

/// A single parsed track from a DJ setlist
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedTrack {
    pub position: usize,       // 1-indexed position in the set
    pub artist: String,
    pub title: String,
    pub timestamp: Option<String>, // "00:23:15" if available
    pub key_hint: Option<String>,  // if the tracklist includes key info
}

/// The full parsed setlist
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedSetlist {
    pub set_name: Option<String>,
    pub dj_name: Option<String>,
    pub tracks: Vec<ParsedTrack>,
}

/// A track from the parsed setlist matched against the local library
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchedTrack {
    pub parsed: ParsedTrack,
    pub local_match: Option<LocalMatch>,
    pub harmonic_flow: Option<String>, // e.g., "8A → 8A (same key)"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalMatch {
    pub track_id: i64,
    pub filename: String,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub key_camelot: Option<String>,
    pub bpm: Option<f64>,
    pub energy_level: Option<i32>,
    pub match_score: f64, // 0.0-1.0, how confident the match is
}

/// Result of analyzing a full DJ setlist
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetlistAnalysis {
    pub parsed: ParsedSetlist,
    pub matched_tracks: Vec<MatchedTrack>,
    pub summary: SetlistSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetlistSummary {
    pub total_tracks: usize,
    pub matched_locally: usize,
    pub unmatched: usize,
    pub key_flow: Vec<String>,    // e.g., ["8A", "8A", "9A", "7A", ...]
    pub bpm_range: Option<(f64, f64)>,
    pub energy_arc: Vec<Option<i32>>, // energy per track if matched
    pub transitions: Vec<String>, // plain-English transition descriptions
}

/// Parse a raw tracklist text into structured data using the LLM.
pub async fn parse_setlist(
    client: &OllamaClient,
    model: &str,
    raw_text: &str,
) -> Result<ParsedSetlist> {
    let system = r#"You are a DJ setlist parser. You parse raw tracklist text from DJ sets, radio shows, or live recordings into structured JSON.

Rules:
- Extract artist and title for each track
- Preserve the order (position 1, 2, 3, ...)
- Extract timestamps if present (format: "HH:MM:SS" or "MM:SS")
- Extract key hints if present (e.g., "8A", "C minor", "Ab major")
- If the set has a name or DJ name in the text, extract it
- Return ONLY valid JSON, no markdown formatting

Output format:
{
  "setName": "string or null",
  "djName": "string or null", 
  "tracks": [
    {"position": 1, "artist": "Artist Name", "title": "Track Title", "timestamp": "00:05:30", "keyHint": "8A"}
  ]
}"#;

    let user = format!("Parse this tracklist:\n\n{}", raw_text);

    let value = client.prompt_json(model, system, &user).await?;

    // Deserialize from the JSON value
    let parsed: ParsedSetlist = serde_json::from_value(value)
        .context("Failed to deserialize parsed setlist")?;

    Ok(parsed)
}

/// Match parsed tracks against the local library.
/// Uses fuzzy matching on artist + title.
pub fn match_tracks(
    parsed: &ParsedSetlist,
    library: &[(i64, String, Option<String>, Option<String>, Option<String>, Option<f64>, Option<i32>)],
    // (track_id, filename, title, artist, key_camelot, bpm, energy_level)
) -> Vec<MatchedTrack> {
    parsed.tracks.iter().map(|pt| {
        let best_match = find_best_match(pt, library);
        MatchedTrack {
            parsed: pt.clone(),
            local_match: best_match,
            harmonic_flow: None, // filled in after all tracks are matched
        }
    }).collect()
}

/// Find the best local library match for a parsed track.
fn find_best_match(
    pt: &ParsedTrack,
    library: &[(i64, String, Option<String>, Option<String>, Option<String>, Option<f64>, Option<i32>)],
) -> Option<LocalMatch> {
    let mut best: Option<(f64, usize)> = None;
    let pt_artist = normalize_string(&pt.artist);
    let pt_title = normalize_string(&pt.title);

    for (i, &(id, ref filename, ref title, ref artist, ref key, bpm, energy)) in library.iter().enumerate() {
        let lib_artist = normalize_string(artist.as_deref().unwrap_or(""));
        let lib_title = normalize_string(title.as_deref().unwrap_or(""));
        let lib_filename = normalize_string(filename);

        // Score: combine artist and title similarity
        let artist_score = if pt_artist.is_empty() || lib_artist.is_empty() {
            0.0
        } else {
            string_similarity(&pt_artist, &lib_artist)
        };

        let title_score = if pt_title.is_empty() {
            0.0
        } else {
            string_similarity(&pt_title, &lib_title)
                .max(string_similarity(&pt_title, &lib_filename))
        };

        // Weighted: title is more important than artist
        let score = title_score * 0.6 + artist_score * 0.4;

        if score > 0.5 && (best.is_none() || score > best.unwrap().0) {
            best = Some((score, i));
        }
    }

    best.map(|(score, i)| {
        let (id, ref filename, ref title, ref artist, ref key, bpm, energy) = library[i];
        LocalMatch {
            track_id: id,
            filename: filename.clone(),
            title: title.clone(),
            artist: artist.clone(),
            key_camelot: key.clone(),
            bpm,
            energy_level: energy,
            match_score: score,
        }
    })
}

/// Normalize a string for comparison: lowercase, trim, remove special chars.
fn normalize_string(s: &str) -> String {
    s.to_lowercase()
        .trim()
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == ' ')
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Simple string similarity using token overlap (Jaccard-like).
fn string_similarity(a: &str, b: &str) -> f64 {
    let a_tokens: std::collections::HashSet<&str> = a.split_whitespace().collect();
    let b_tokens: std::collections::HashSet<&str> = b.split_whitespace().collect();

    if a_tokens.is_empty() || b_tokens.is_empty() {
        return 0.0;
    }

    let intersection = a_tokens.intersection(&b_tokens).count();
    let union = a_tokens.union(&b_tokens).count();

    intersection as f64 / union as f64
}

/// Analyze the harmonic flow of a matched setlist.
/// Produces key flow, BPM range, energy arc, and transition descriptions.
pub fn analyze_harmonic_flow(matched: &mut Vec<MatchedTrack>) -> SetlistSummary {
    let total_tracks = matched.len();
    let matched_locally = matched.iter().filter(|m| m.local_match.is_some()).count();
    let unmatched = total_tracks - matched_locally;

    // Build key flow from matched tracks
    let key_flow: Vec<String> = matched.iter()
        .filter_map(|m| m.local_match.as_ref())
        .filter_map(|lm| lm.key_camelot.as_ref())
        .map(|k| k.clone())
        .collect();

    // BPM range
    let bpms: Vec<f64> = matched.iter()
        .filter_map(|m| m.local_match.as_ref())
        .filter_map(|lm| lm.bpm)
        .collect();
    let bpm_range = if !bpms.is_empty() {
        Some((bpms.iter().cloned().fold(f64::INFINITY, f64::min), bpms.iter().cloned().fold(f64::NEG_INFINITY, f64::max)))
    } else {
        None
    };

    // Energy arc
    let energy_arc: Vec<Option<i32>> = matched.iter()
        .map(|m| m.local_match.as_ref().and_then(|lm| lm.energy_level))
        .collect();

    // Generate transition descriptions
    let transitions = generate_transition_descriptions(matched);

    // Fill in harmonic_flow for each track
    for i in 0..matched.len() {
        let flow = if i == 0 {
            matched[i].local_match.as_ref()
                .and_then(|lm| lm.key_camelot.as_ref())
                .map(|k| format!("Opening: {}", k))
                .unwrap_or_else(|| "Opening (unmatched)".to_string())
        } else {
            let prev_key = matched[i-1].local_match.as_ref().and_then(|lm| lm.key_camelot.as_ref());
            let curr_key = matched[i].local_match.as_ref().and_then(|lm| lm.key_camelot.as_ref());
            match (prev_key, curr_key) {
                (Some(prev), Some(curr)) => format!("{} → {}", prev, curr),
                (Some(prev), None) => format!("{} → ?", prev),
                (None, Some(curr)) => format!("? → {}", curr),
                (None, None) => "? → ?".to_string(),
            }
        };
        matched[i].harmonic_flow = Some(flow);
    }

    SetlistSummary {
        total_tracks,
        matched_locally,
        unmatched,
        key_flow,
        bpm_range,
        energy_arc,
        transitions,
    }
}

/// Generate plain-English transition descriptions for the setlist.
fn generate_transition_descriptions(matched: &[MatchedTrack]) -> Vec<String> {
    let mut descriptions = Vec::new();
    for i in 0..matched.len().saturating_sub(1) {
        let prev = &matched[i];
        let curr = &matched[i + 1];
        let desc = match (&prev.local_match, &curr.local_match) {
            (Some(p), Some(c)) => {
                let p_key = p.key_camelot.as_deref().unwrap_or("?");
                let c_key = c.key_camelot.as_deref().unwrap_or("?");
                let p_bpm = p.bpm.map(|b| format!("{:.0}", b)).unwrap_or("?".to_string());
                let c_bpm = c.bpm.map(|b| format!("{:.0}", b)).unwrap_or("?".to_string());
                if p_key == c_key {
                    format!("Track {} → {}: Same key ({}), BPM {} → {}", p_key, c_key, p_key, p_bpm, c_bpm)
                } else {
                    format!("Track {} → {}: Key shift {} → {}, BPM {} → {}", p_key, c_key, p_key, c_key, p_bpm, c_bpm)
                }
            }
            _ => format!("Transition (unmatched tracks)"),
        };
        descriptions.push(desc);
    }
    descriptions
}

/// Full pipeline: parse setlist, match against library, analyze flow.
pub async fn analyze_setlist(
    client: &OllamaClient,
    model: &str,
    raw_text: &str,
    library: &[(i64, String, Option<String>, Option<String>, Option<String>, Option<f64>, Option<i32>)],
) -> Result<SetlistAnalysis> {
    let parsed = parse_setlist(client, model, raw_text).await?;
    let mut matched = match_tracks(&parsed, library);
    let summary = analyze_harmonic_flow(&mut matched);

    Ok(SetlistAnalysis {
        parsed,
        matched_tracks: matched,
        summary,
    })
}
