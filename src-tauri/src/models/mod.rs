use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub id: i64,
    pub file_path: String,
    pub filename: String,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub duration_ms: Option<i64>,
    pub key_standard: Option<String>,
    pub key_camelot: Option<String>,
    pub key_confidence: Option<f64>,
    pub bpm: Option<f64>,
    pub energy_level: Option<i32>,
    pub file_format: Option<String>,
    pub file_size: i64,
    pub sample_rate: Option<i64>,
    pub bit_depth: Option<i64>,
    pub analyzed_at: Option<String>,
    pub status: TrackStatus,
    /// Absolute filesystem path to the cached cover-art image (PNG/JPEG).
    /// `None` means we have not extracted art for this track yet, or the
    /// audio file has no embedded picture. The frontend uses Tauri's asset
    /// protocol (`convertFileSrc`) to display it.
    #[serde(rename = "artwork_path")]
    pub artwork_path: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrackStatus {
    Pending,
    MetadataReady,
    Analyzing,
    Analyzed,
    Error,
}

impl From<String> for TrackStatus {
    fn from(s: String) -> Self {
        match s.as_str() {
            "pending" => TrackStatus::Pending,
            "metadata_ready" => TrackStatus::MetadataReady,
            "analyzing" => TrackStatus::Analyzing,
            "analyzed" => TrackStatus::Analyzed,
            "error" => TrackStatus::Error,
            _ => TrackStatus::Pending,
        }
    }
}

impl TrackStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            TrackStatus::Pending => "pending",
            TrackStatus::MetadataReady => "metadata_ready",
            TrackStatus::Analyzing => "analyzing",
            TrackStatus::Analyzed => "analyzed",
            TrackStatus::Error => "error",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CuePoint {
    pub id: i64,
    pub track_id: i64,
    pub position_ms: i64,
    pub name: Option<String>,
    pub color: Option<String>,
    pub hotcue_index: Option<i64>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Playlist {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
    pub rules: Option<serde_json::Value>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistRules {
    pub same_key: bool,
    pub plus_one: bool,
    pub minus_one: bool,
    pub plus_two: bool,
    pub minus_two: bool,
    pub dominant_to_subdominant: bool,
    pub subdominant_to_dominant: bool,
    pub energy_curve: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisProgress {
    pub total: usize,
    pub completed: usize,
    pub in_progress: usize,
    pub speed_per_sec: f64,
    pub eta_seconds: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackAnalysis {
    pub track_id: i64,
    pub key_standard: String,
    pub key_camelot: String,
    pub key_confidence: f64,
    pub bpm: f64,
    pub duration_ms: i64,
    pub energy_level: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryFilter {
    pub search: Option<String>,
    pub artist: Option<String>,
    pub key_camelot: Option<String>,
    pub min_bpm: Option<f64>,
    pub max_bpm: Option<f64>,
    pub status: Option<String>,
    /// Smart filter preset: "unanalyzed", "low-confidence", "high-confidence".
    /// Applied server-side so it works across all 20k tracks, not just the
    /// loaded page.
    pub smart_filter: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryPage {
    pub tracks: Vec<Track>,
    pub total_count: usize,
    pub page: usize,
    pub page_size: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportOptions {
    pub write_tags: bool,
    pub number_prefix: bool,
    pub include_cues: bool,
    pub dj_software_format: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanResult {
    pub total_files: usize,
    pub new_files: usize,
    pub skipped: usize,
}
