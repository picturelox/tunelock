// Genre inference — use the LLM to infer genre from artist/title/filename
// when genre metadata is missing. This feeds the Phase 9 adaptive profile
// weights, which is the one place an LLM indirectly improves key accuracy.
//
// Also includes transition explanation generation (LLM + deterministic
// template fallback) and natural-language set planning.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::ollama::OllamaClient;
use crate::harmony;

// ============================================================================
// Genre inference
// ============================================================================

/// A genre inference result
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenreInference {
    pub track_id: i64,
    pub inferred_genre: String,
    pub confidence: f64,
    pub reasoning: Option<String>,
}

const GENRE_SYSTEM: &str = r#"You are a music genre classification assistant. You infer the genre of a track from its artist, title, and filename.

Use these standard genres: electronic, house, techno, trance, dubstep, drum_and_bass, hip_hop, r&b, rock, metal, pop, jazz, classical, ambient, reggae, funk, soul, country, folk, blues, world.

Rules:
- Choose the single best-fitting genre from the list above
- Set confidence high (0.9+) for well-known artists, lower (0.5-0.7) for ambiguous cases
- Return ONLY valid JSON

Output format:
{"inferences": [{"trackId": 123, "genre": "electronic", "confidence": 0.9}]}
"#;

/// Infer genre for tracks with missing genre metadata.
pub async fn infer_genres(
    client: &OllamaClient,
    model: &str,
    tracks: &[(i64, String, Option<String>, Option<String>)],
    // (track_id, filename, title, artist)
) -> Result<Vec<GenreInference>> {
    let needs_genre: Vec<_> = tracks.iter()
        .filter(|(_, _, _, artist)| artist.is_some())
        .collect();

    if needs_genre.is_empty() {
        return Ok(vec![]);
    }

    let track_list: Vec<String> = needs_genre.iter().map(|(id, filename, title, artist)| {
        format!(
            r#"{{"trackId": {}, "filename": "{}", "title": "{}", "artist": "{}"}}"#,
            id,
            filename.replace('"', "\\\""),
            title.as_deref().unwrap_or("").replace('"', "\\\""),
            artist.as_deref().unwrap_or("").replace('"', "\\\""),
        )
    }).collect();

    let mut all_inferences = Vec::new();
    for chunk in track_list.chunks(50) {
        let user = format!("Infer genres for these tracks:\n\n[{}]", chunk.join(",\n"));
        match client.prompt_json(model, GENRE_SYSTEM, &user).await {
            Ok(value) => {
                if let Some(inferences) = value.get("inferences").and_then(|i| i.as_array()) {
                    for inf in inferences {
                        if let Some(track_id) = inf.get("trackId").and_then(|t| t.as_i64()) {
                            all_inferences.push(GenreInference {
                                track_id,
                                inferred_genre: inf.get("genre").and_then(|g| g.as_str()).unwrap_or("electronic").to_string(),
                                confidence: inf.get("confidence").and_then(|c| c.as_f64()).unwrap_or(0.5),
                                reasoning: inf.get("reasoning").and_then(|r| r.as_str()).map(|s| s.to_string()),
                            });
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!("Genre inference failed: {}", e);
            }
        }
    }

    Ok(all_inferences)
}

// ============================================================================
// Transition explanations
// ============================================================================

/// Generate a plain-English explanation of a harmonic transition.
/// Uses the LLM if available, falls back to a deterministic template.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransitionExplanation {
    pub from_key: String,
    pub to_key: String,
    pub from_bpm: Option<f64>,
    pub to_bpm: Option<f64>,
    pub explanation: String,
    pub source: String, // "llm" or "template"
}

const TRANSITION_SYSTEM: &str = r#"You are a DJ transition explainer. You explain why a transition between two tracks works (or doesn't) in plain English, using harmonic and rhythmic reasoning.

Rules:
- Explain the key relationship (same key, +1, -1, +2, relative major/minor, etc.)
- Explain the BPM change and whether it's manageable
- Mention any risks (large key jump, big tempo change)
- Keep it to 1-2 sentences, conversational tone
- Return ONLY the explanation text, no JSON"#;

/// Generate a transition explanation.
pub async fn explain_transition(
    client: &OllamaClient,
    model: &str,
    from_key: &str,
    to_key: &str,
    from_bpm: Option<f64>,
    to_bpm: Option<f64>,
) -> Result<TransitionExplanation> {
    // Try LLM first
    let user = format!(
        "Explain the transition from {} ({:.0} BPM) to {} ({:.0} BPM)",
        from_key,
        from_bpm.unwrap_or(0.0),
        to_key,
        to_bpm.unwrap_or(0.0),
    );

    match client.prompt(model, TRANSITION_SYSTEM, &user).await {
        Ok(explanation) => {
            Ok(TransitionExplanation {
                from_key: from_key.to_string(),
                to_key: to_key.to_string(),
                from_bpm,
                to_bpm,
                explanation,
                source: "llm".to_string(),
            })
        }
        Err(_) => {
            // Fall back to deterministic template
            Ok(TransitionExplanation {
                from_key: from_key.to_string(),
                to_key: to_key.to_string(),
                from_bpm,
                to_bpm,
                explanation: template_explanation(from_key, to_key, from_bpm, to_bpm),
                source: "template".to_string(),
            })
        }
    }
}

/// Deterministic template fallback for transition explanations.
/// This ensures the UI never depends on a network call.
pub fn template_explanation(
    from_key: &str,
    to_key: &str,
    from_bpm: Option<f64>,
    to_bpm: Option<f64>,
) -> String {
    let rel = harmony::get_camelot_relationship(from_key, to_key);
    let bpm_note = match (from_bpm, to_bpm) {
        (Some(f), Some(t)) => {
            let delta = ((t - f) / f * 100.0).abs();
            if delta < 2.0 {
                "BPM is nearly matched.".to_string()
            } else if delta < 8.0 {
                format!("BPM shifts {:.0}% — manageable.", delta)
            } else {
                format!("BPM jumps {:.0}% — risky without pitch adjustment.", delta)
            }
        }
        _ => "BPM unknown.".to_string(),
    };

    format!("{} {}", rel.label(), bpm_note)
}

// ============================================================================
// NL set planning
// ============================================================================

/// A natural-language set plan
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetPlan {
    pub description: String,
    pub track_ids: Vec<i64>,
    pub reasoning: String,
}

const SETPLAN_SYSTEM: &str = r#"You are a DJ set planner. You sequence tracks into a coherent DJ set based on the user's natural-language instructions.

You have access to a list of tracks with their key (Camelot notation), BPM, and energy level (1-10). Your job is to:
1. Select tracks that match the user's description (duration, energy arc, mood)
2. Order them using harmonic compatibility (same key, ±1, ±2, relative major/minor)
3. Manage the energy curve across the set
4. Keep BPM changes gradual (max 8% per transition)

Rules:
- Return ONLY valid JSON with track IDs in order
- Prefer harmonic compatibility over strict energy matching
- Include 8-20 tracks depending on the requested duration

Output format:
{"description": "Brief description of the set", "trackIds": [123, 456, 789, ...], "reasoning": "Why this sequence works"}
"#;

/// Plan a DJ set from a natural-language description.
pub async fn plan_set(
    client: &OllamaClient,
    model: &str,
    instruction: &str,
    available_tracks: &[(i64, String, Option<String>, Option<String>, Option<f64>, Option<i32>)],
    // (track_id, filename, title, key_camelot, bpm, energy_level)
) -> Result<SetPlan> {
    // Format the available tracks for the LLM
    let track_list: Vec<String> = available_tracks.iter().map(|(id, _, title, key, bpm, energy)| {
        format!(
            r#"{{"id": {}, "title": "{}", "key": "{}", "bpm": {}, "energy": {}}}"#,
            id,
            title.as_deref().unwrap_or("").replace('"', "\\\""),
            key.as_deref().unwrap_or(""),
            bpm.map(|b| format!("{:.0}", b)).unwrap_or("null".to_string()),
            energy.map(|e| e.to_string()).unwrap_or("null".to_string()),
        )
    }).collect();

    // Limit to 200 tracks to avoid context overflow
    let limited: String = track_list.iter().take(200).cloned().collect::<Vec<_>>().join(",\n");

    let user = format!(
        "Instruction: {}\n\nAvailable tracks:\n[{}]",
        instruction, limited
    );

    let value = client.prompt_json(model, SETPLAN_SYSTEM, &user).await?;
    let plan = SetPlan {
        description: value.get("description").and_then(|d| d.as_str()).unwrap_or("").to_string(),
        track_ids: value.get("trackIds")
            .and_then(|t| t.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_i64()).collect())
            .unwrap_or_default(),
        reasoning: value.get("reasoning").and_then(|r| r.as_str()).unwrap_or("").to_string(),
    };

    Ok(plan)
}
