use std::sync::Arc;
use tauri::Manager;
use tokio::sync::Mutex;

pub mod analysis;
pub mod assist;
pub mod commands;
pub mod consensus;
pub mod db;
pub mod export;
pub mod harmony;
pub mod media;
pub mod models;
pub mod proof;

use db::Database;

pub struct AppState {
    pub db: Arc<Mutex<Database>>,
    pub analysis_queue: Arc<Mutex<AnalysisQueue>>,
    pub ollama: Arc<assist::OllamaClient>,
    pub assist_enabled: Arc<Mutex<bool>>,
    pub assist_model: Arc<Mutex<Option<String>>>,
}

pub struct AnalysisQueue {
    pub pending: Vec<(i64, String)>, // (track_id, file_path)
    pub in_progress: bool,
    pub paused: bool,
    /// Timestamp (millis since epoch) when the current batch started.
    pub batch_start_ms: Option<u128>,
    /// Total tracks completed since the queue started.
    pub completed_count: usize,
    /// Total time spent analyzing (ms) since the queue started.
    pub elapsed_ms: u128,
}

impl Default for AnalysisQueue {
    fn default() -> Self {
        Self {
            pending: Vec::new(),
            in_progress: false,
            paused: false,
            batch_start_ms: None,
            completed_count: 0,
            elapsed_ms: 0,
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .setup(|app| {
            // Initialize database
            let app_dir = app.path().app_data_dir().expect("Failed to get app data dir");
            std::fs::create_dir_all(&app_dir)?;
            let db_path = app_dir.join("library.db");
            
            let db = Arc::new(Mutex::new(
                Database::new(db_path).expect("Failed to initialize database")
            ));
            
            let state = AppState {
                db,
                analysis_queue: Arc::new(Mutex::new(AnalysisQueue::default())),
                ollama: Arc::new(assist::OllamaClient::new()),
                assist_enabled: Arc::new(Mutex::new(false)),
                assist_model: Arc::new(Mutex::new(None)),
            };
            
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::scan_folder,
            commands::get_library_page,
            commands::start_analysis,
            commands::pause_analysis,
            commands::resume_analysis,
            commands::cancel_analysis,
            commands::get_analysis_status,
            commands::analyze_file,
            commands::read_file_metadata,
            commands::generate_playlist,
            commands::get_compatible_tracks,
            commands::export_tracks,
            commands::save_playlist,
            commands::get_playlists,
            commands::delete_playlist,
            commands::save_mix,
            commands::load_mix,
            commands::get_playlist_tracks,
            commands::import_mik_csv,
            commands::get_consensus,
            commands::get_consensus_batch,
            commands::get_contested_tracks,
            commands::set_track_opinion,
            commands::import_traktor_nml,
            commands::get_waveform_data,
            commands::get_key_timeline,
            // Step 6: Gold set annotation
            commands::save_gold_annotation,
            commands::get_gold_annotations,
            commands::get_gold_annotation_summary,
            commands::save_training_session,
            commands::get_training_stats,
            // Phase 11: Assist layer
            commands::assist_status,
            commands::assist_set_enabled,
            commands::assist_set_model,
            commands::assist_analyze_setlist,
            commands::assist_repair_metadata,
            commands::assist_apply_metadata_repair,
            commands::assist_infer_genres,
            commands::assist_explain_transition,
            commands::assist_plan_set,
            // Transition Workbench (Phase 7 / Slice A)
            commands::get_beat_grid,
            commands::save_beat_grid_override,
            commands::reset_beat_grid_override,
            commands::get_transition_plan,
            commands::save_transition_plan,
            commands::get_stem_manifest,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
