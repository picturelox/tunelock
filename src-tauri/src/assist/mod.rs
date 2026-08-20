// Assist layer — optional LLM-powered features via Ollama.
//
// This module is NEVER on the critical analysis path. Key/BPM/energy
// detection runs entirely locally without any LLM call. All Assist
// features are user-initiated and degrade gracefully when Ollama is
// not installed.
//
// Features:
// - DJ setlist analysis (parse tracklists, match local library, show harmonic flow)
// - Metadata repair (parse filenames, fill missing artist/title/genre)
// - Genre inference (feeds adaptive profiles)
// - Transition explanations (LLM + deterministic template fallback)
// - NL set planning ("90 min, start mellow..." → sequenced mix)

pub mod ollama;
pub mod setlist;

pub use ollama::{AssistStatus, OllamaClient, OllamaModel, ChatMessage};
pub use setlist::{
    ParsedSetlist, ParsedTrack, MatchedTrack, LocalMatch,
    SetlistAnalysis, SetlistSummary, analyze_setlist,
};
