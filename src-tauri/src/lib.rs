use std::sync::{
    atomic::{AtomicBool, AtomicU64},
    Arc,
};
use tauri::Manager;
use tokio::sync::Mutex;

pub mod analysis;
pub mod assist;
pub mod audio;
pub mod commands;
pub mod consensus;
pub mod db;
pub mod export;
pub mod harmony;
pub mod media;
pub mod models;
pub mod neural_key;
pub mod neural_key_audio;
pub mod neural_key_preprocess;
pub mod proof;

use db::Database;

pub struct AppState {
    pub db: Arc<Mutex<Database>>,
    pub analysis_queue: Arc<Mutex<AnalysisQueue>>,
    pub ollama: Arc<assist::OllamaClient>,
    pub assist_enabled: Arc<Mutex<bool>>,
    pub assist_model: Arc<Mutex<Option<String>>>,
    pub audio_engine: Arc<Mutex<Option<audio::AudioEngine>>>,
    /// Serializes engine creation and device replacement.
    pub audio_engine_lifecycle: Arc<AudioEngineLifecycle>,
    pub audio_engine_drain_started: Arc<AtomicBool>,
    pub audio_loads: Arc<AudioLoadCoordinator>,
}

/// Assigns and validates asynchronous load identities independently for every
/// player. Decode work may finish out of order; only the newest identity may
/// install into the engine.
pub struct AudioLoadCoordinator {
    next_generation: AtomicU64,
    current: [AtomicU64; audio::MAX_PLAYERS],
}

impl AudioLoadCoordinator {
    pub fn new() -> Self {
        Self {
            next_generation: AtomicU64::new(1),
            current: std::array::from_fn(|_| AtomicU64::new(0)),
        }
    }

    pub fn begin(&self, player: audio::PlayerId) -> Result<audio::LoadGeneration, String> {
        let index = player.as_index();
        if index >= audio::MAX_PLAYERS {
            return Err(format!("Player {} is out of range", player.0));
        }
        let generation = self
            .next_generation
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        self.current[index].store(generation, std::sync::atomic::Ordering::Release);
        Ok(audio::LoadGeneration(generation))
    }

    pub fn is_current(
        &self,
        player: audio::PlayerId,
        generation: audio::LoadGeneration,
    ) -> bool {
        player.as_index() < audio::MAX_PLAYERS
            && self.current[player.as_index()].load(std::sync::atomic::Ordering::Acquire)
                == generation.0
    }
}

/// Coordinates application-level engine replacement without putting any
/// synchronization on the realtime thread.
pub struct AudioEngineLifecycle {
    pub change: Mutex<()>,
    generation: AtomicU64,
}

impl AudioEngineLifecycle {
    pub fn new() -> Self {
        Self {
            change: Mutex::new(()),
            generation: AtomicU64::new(0),
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation.load(std::sync::atomic::Ordering::Acquire)
    }

    pub fn advance_generation(&self) -> u64 {
        self.generation
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel)
            + 1
    }
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

#[cfg(test)]
mod app_state_tests {
    use super::{AudioEngineLifecycle, AudioLoadCoordinator};
    use std::sync::Arc;

    #[tokio::test]
    async fn engine_lifecycle_serializes_concurrent_changes() {
        let lifecycle = Arc::new(AudioEngineLifecycle::new());
        let first_change = lifecycle.change.lock().await;

        let waiting_lifecycle = lifecycle.clone();
        let waiting = tokio::spawn(async move {
            let _change = waiting_lifecycle.change.lock().await;
            waiting_lifecycle.advance_generation()
        });

        tokio::task::yield_now().await;
        assert!(!waiting.is_finished());
        assert_eq!(lifecycle.generation(), 0);

        drop(first_change);
        assert_eq!(waiting.await.unwrap(), 1);
        assert_eq!(lifecycle.generation(), 1);
    }

    #[test]
    fn newest_player_load_generation_invalidates_older_work() {
        let loads = AudioLoadCoordinator::new();
        let player = crate::audio::PlayerId(1);
        let first = loads.begin(player).unwrap();
        let second = loads.begin(player).unwrap();
        assert!(!loads.is_current(player, first));
        assert!(loads.is_current(player, second));
        assert!(loads.begin(crate::audio::PlayerId(8)).is_err());
    }
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
                audio_engine: Arc::new(Mutex::new(None)),
                audio_engine_lifecycle: Arc::new(AudioEngineLifecycle::new()),
                audio_engine_drain_started: Arc::new(AtomicBool::new(false)),
                audio_loads: Arc::new(AudioLoadCoordinator::new()),
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
            // Audio engine (Transition Workbench — real-time playback)
            // Generalized player/bus vocabulary: PlayerId(u8), BusId, MAX_PLAYERS=8
            commands::audio_engine_init,
            commands::audio_engine_play,
            commands::audio_engine_pause,
            commands::audio_engine_stop,
            commands::audio_engine_seek,
            commands::audio_engine_set_crossfade,
            commands::audio_engine_set_tempo,
            commands::audio_engine_set_pitch,
            commands::audio_engine_set_player_gain,
            commands::audio_engine_set_pan,
            commands::audio_engine_set_mute,
            commands::audio_engine_set_solo,
            commands::audio_engine_set_bus,
            commands::audio_engine_set_eq,
            commands::audio_engine_set_eq_kill,
            commands::audio_engine_set_loop,
            commands::audio_engine_set_hot_cue,
            commands::audio_engine_jump_hot_cue,
            commands::audio_engine_nudge,
            commands::audio_engine_load_player,
            commands::audio_engine_set_master_gain,
            commands::audio_engine_set_bus_gain,
            commands::audio_engine_set_cue_enabled,
            commands::audio_engine_set_cue_gain,
            commands::audio_engine_set_cue_master_blend,
            commands::audio_engine_set_output_routing,
            commands::audio_engine_set_filter_mode,
            commands::audio_engine_set_filter_cutoff,
            commands::audio_engine_set_filter_resonance,
            commands::audio_engine_set_filter_drive,
            commands::audio_engine_get_meters,
            // PB-3: Professional audio I/O
            commands::audio_enumerate_devices,
            commands::audio_engine_set_device,
            // Beat-grid DSP
            commands::detect_beat_grid,
            // PB-2 Listening Lab
            commands::listening_lab_get_processor_info,
            commands::listening_lab_save_result,
            commands::listening_lab_get_results,
            commands::audio_engine_set_processor_type,
            commands::audio_engine_sync_launch,
            commands::audio_engine_set_listening_condition,
            commands::audio_engine_load_player_paused,
            commands::audio_engine_seek_source_seconds,
            commands::audio_engine_beat_sync,
            commands::audio_engine_bar_sync,
            commands::get_git_revision,
            commands::get_track_loudness,
            commands::audio_engine_set_loudness_match_gain,
            commands::audio_engine_compute_loudness_match,
            commands::get_loudness_comparison,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
