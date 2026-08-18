use anyhow::Result;
use tauri::{command, Emitter, Manager, State, Window};
use walkdir::WalkDir;

use crate::analysis::key_detector::detect_key;
use crate::analysis::tempo_detector::detect_tempo;
use crate::models::*;
use crate::{AppState, AnalysisQueue};

#[command]
pub async fn scan_folder(
    state: State<'_, AppState>,
    path: String,
) -> Result<ScanResult, String> {
    let audio_extensions = ["mp3", "wav", "flac", "ogg", "aiff", "m4a", "aac", "wma"];
    
    let mut total_files = 0;
    let mut new_files = 0;
    let mut skipped = 0;
    
    let db = state.db.lock().await;
    
    for entry in WalkDir::new(&path).into_iter().filter_map(|e| e.ok()) {
        if entry.file_type().is_file() {
            if let Some(ext) = entry.path().extension() {
                let ext = ext.to_string_lossy().to_lowercase();
                if audio_extensions.contains(&ext.as_str()) {
                    total_files += 1;
                    
                    let path_str = entry.path().to_string_lossy().to_string();
                    let filename = entry.file_name().to_string_lossy().to_string();
                    let size = entry.metadata().map(|m| m.len() as i64).unwrap_or(0);
                    
                    match db.insert_track(&path_str, &filename, size) {
                        Ok(_) => new_files += 1,
                        Err(_) => skipped += 1,
                    }
                }
            }
        }
    }
    
    Ok(ScanResult {
        total_files,
        new_files,
        skipped,
    })
}

#[command]
pub async fn get_library_page(
    state: State<'_, AppState>,
    page: usize,
    page_size: usize,
    sort_by: String,
    sort_dir: String,
    filter: Option<LibraryFilter>,
) -> Result<LibraryPage, String> {
    let db = state.db.lock().await;
    db.get_library_page(page, page_size, &sort_by, &sort_dir, filter.as_ref())
        .map_err(|e| e.to_string())
}

#[command]
pub async fn start_analysis(
    state: State<'_, AppState>,
    window: Window,
) -> Result<(), String> {
    let mut queue = state.analysis_queue.lock().await;
    
    if queue.in_progress {
        return Ok(());
    }
    
    // Get pending tracks from database
    let db = state.db.lock().await;
    let pending = db.get_tracks_pending_analysis(1000)
        .map_err(|e| e.to_string())?;
    drop(db);
    
    queue.pending = pending;
    queue.in_progress = true;
    queue.paused = false;
    
    // Spawn analysis task
    let db_clone = state.db.clone();
    let queue_clone = state.analysis_queue.clone();
    
    tokio::spawn(async move {
        analyze_batch(db_clone, queue_clone, window).await;
    });
    
    Ok(())
}

#[command]
pub async fn pause_analysis(state: State<'_, AppState>) -> Result<(), String> {
    let mut queue = state.analysis_queue.lock().await;
    queue.paused = true;
    Ok(())
}

#[command]
pub async fn resume_analysis(state: State<'_, AppState>) -> Result<(), String> {
    let mut queue = state.analysis_queue.lock().await;
    queue.paused = false;
    Ok(())
}

#[command]
pub async fn cancel_analysis(state: State<'_, AppState>) -> Result<(), String> {
    let mut queue = state.analysis_queue.lock().await;
    queue.in_progress = false;
    queue.pending.clear();
    Ok(())
}

#[command]
pub async fn get_analysis_status(state: State<'_, AppState>) -> Result<AnalysisProgress, String> {
    let db = state.db.lock().await;
    let (total, completed) = db.get_analysis_stats().map_err(|e| e.to_string())?;
    drop(db);
    
    let queue = state.analysis_queue.lock().await;
    let in_progress = if queue.in_progress {
        total.saturating_sub(completed).min(queue.pending.len())
    } else {
        0
    };
    
    Ok(AnalysisProgress {
        total,
        completed,
        in_progress,
        speed_per_sec: 0.0, // TODO: Calculate from timing
        eta_seconds: 0.0,
    })
}

/// CPU-bound work for a single track. No DB, no IO beyond decoding.
/// Returns `(track_id, file_path, result)` where the result contains the
/// analysis numbers ready to be persisted.
struct TrackAnalysisRaw {
    track_id: i64,
    file_path: String,
    outcome: Result<(String, String, f64, f64)>, // (key_standard, key_camelot, confidence, bpm)
}

fn analyze_cpu(track_id: i64, file_path: String) -> TrackAnalysisRaw {
    let outcome: Result<(String, String, f64, f64)> = (|| {
        let samples = crate::analysis::decoder::decode_audio(&file_path)?;
        let key = detect_key(&samples)?;
        let bpm = detect_tempo(&samples)?;
        Ok((key.key_standard, key.key_camelot, key.confidence, bpm))
    })();
    TrackAnalysisRaw { track_id, file_path, outcome }
}

async fn analyze_batch(
    db: std::sync::Arc<tokio::sync::Mutex<crate::db::Database>>,
    queue: std::sync::Arc<tokio::sync::Mutex<AnalysisQueue>>,
    window: Window,
) {
    // Use N-1 cores so the UI stays responsive (min 1).
    let batch_size = std::thread::available_parallelism()
        .map(|n| n.get().saturating_sub(1).max(1))
        .unwrap_or(3);
    
    loop {
        // Respect pause / cancel
        {
            let q = queue.lock().await;
            if q.paused {
                drop(q);
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                continue;
            }
            if !q.in_progress {
                break;
            }
        }
        
        // Pop a batch from the queue.
        let batch: Vec<(i64, String)> = {
            let mut q = queue.lock().await;
            (0..batch_size).filter_map(|_| q.pending.pop()).collect()
        };
        
        if batch.is_empty() {
            let mut q = queue.lock().await;
            q.in_progress = false;
            break;
        }
        
        // Run the CPU-heavy work in parallel via rayon, off the tokio reactor.
        let results: Vec<TrackAnalysisRaw> = tokio::task::spawn_blocking(move || {
            use rayon::prelude::*;
            batch
                .into_par_iter()
                .map(|(id, path)| analyze_cpu(id, path))
                .collect()
        })
        .await
        .unwrap_or_default();
        
        // Serialise DB writes + event emits.
        let db_guard = db.lock().await;
        for r in results {
            match r.outcome {
                Ok((key_std, key_cam, conf, bpm)) => {
                    if let Err(e) = db_guard.update_track_analysis(r.track_id, &key_std, &key_cam, conf, bpm) {
                        eprintln!("[analyze] DB write failed for {}: {}", r.track_id, e);
                        continue;
                    }
                    match db_guard.get_track_by_id(r.track_id) {
                        Ok(Some(track)) => {
                            let _ = window.emit("track-analyzed", &track);
                        }
                        Ok(None) => {
                            eprintln!("[analyze] track {} missing after write", r.track_id);
                        }
                        Err(e) => {
                            eprintln!("[analyze] track fetch failed for {}: {}", r.track_id, e);
                        }
                    }
                }
                Err(e) => {
                    eprintln!("[analyze] failed for {}: {}", r.file_path, e);
                }
            }
        }
        drop(db_guard);
    }
}

/// One ranked key candidate in the Tuner response.
#[derive(serde::Serialize, Debug, Clone)]
pub struct KeyCandidate {
    pub key_standard: String,
    pub key_camelot: String,
    /// Final blended confidence (segment agreement + profile match), 0..1.
    pub confidence: f64,
    /// Fraction of temporal segments that picked this candidate, 0..1.
    pub agreement: f64,
    /// Average normalised profile-match score for the segments that voted
    /// for this candidate, 0..1.
    pub avg_score: f64,
    /// How many of the N temporal segments selected this candidate.
    pub segment_count: usize,
}

/// Per-stage durations in milliseconds. Surfaced in the UI so we can spot
/// regressions and find the slowest stage to optimise next.
#[derive(serde::Serialize, Debug, Clone, Copy, Default)]
pub struct TunerTimings {
    pub decode_ms: u64,
    pub spectrogram_ms: u64,
    pub hpss_ms: u64,
    pub chromagram_ms: u64,
    pub ensemble_ms: u64,
    pub tempo_ms: u64,
    pub metadata_ms: u64,
    pub total_ms: u64,
}

/// Progress event payload emitted as the analysis runs. Frontend listens to
/// the `tuner-progress` event and updates a real 0..100% bar.
#[derive(serde::Serialize, Debug, Clone)]
pub struct TunerProgress {
    pub stage: String,
    pub percent: f64, // 0.0..1.0
}

/// Tuner-mode analysis result. Snake-case wire format to match the TS interface.
#[derive(serde::Serialize)]
pub struct TunerAnalysis {
    pub track_id: i64,
    pub file_path: String,
    pub filename: String,
    pub key_standard: String,
    pub key_camelot: String,
    pub key_confidence: f64,
    pub bpm: f64,
    pub duration_ms: i64,
    pub energy_level: Option<i32>,
    /// Title / artist / album extracted from the file's tags. Used by the
    /// Tuner UI to label the track with something better than its filename
    /// when album metadata is present.
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    /// Absolute path to the cached cover-art image, if any. The frontend
    /// passes this through `convertFileSrc` to render the picture.
    pub artwork_path: Option<String>,
    /// Top-N candidates, highest confidence first. Index 0 is the winner.
    pub candidates: Vec<KeyCandidate>,
    /// Mean chroma vector across the track, normalised so max == 1.0.
    /// Order: C, C#, D, D#, E, F, F#, G, G#, A, A#, B.
    pub chroma: [f64; 12],
    pub timings: TunerTimings,
}

fn candidate_from_ranked(r: &crate::analysis::ensemble::RankedCandidate) -> KeyCandidate {
    let mode = if r.is_major { "major" } else { "minor" };
    KeyCandidate {
        key_standard: format!("{} {}", crate::analysis::pitch_class_to_name(r.tonic), mode),
        key_camelot: crate::analysis::key_to_camelot(r.tonic, r.is_major),
        confidence: r.confidence,
        agreement: r.agreement,
        avg_score: r.avg_score,
        segment_count: r.segment_count,
    }
}

/// Analyze a single audio file at `path` for Tuner Mode.
///
/// Pipeline (with per-stage progress events):
///   1. decode        ->  10%
///   2. spectrogram   ->  40%
///   3. HPSS          ->  60%
///   4. chromagram    ->  75%
///   5. ensemble vote ->  85%
///   6. tempo         ->  95%
///   7. metadata + DB -> 100%
///
/// Also **auto-imports the file into the library** (upsert by file_path),
/// writes metadata, writes analysis. The returned `track_id` is the DB id.
#[command]
pub async fn analyze_file(
    state: State<'_, AppState>,
    window: Window,
    path: String,
) -> Result<TunerAnalysis, String> {
    use lofty::file::{AudioFile, TaggedFileExt};
    use lofty::probe::Probe;
    use lofty::tag::Accessor;
    use std::time::Instant;

    let total_start = Instant::now();
    let mut timings = TunerTimings::default();

    let filename = std::path::Path::new(&path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();

    // File size for the DB insert.
    let file_size = std::fs::metadata(&path)
        .map(|m| m.len() as i64)
        .unwrap_or(0);

    // Helper to emit progress events. Logs failures but never aborts.
    let emit_progress = |stage: &str, percent: f64| {
        let payload = TunerProgress { stage: stage.to_string(), percent };
        if let Err(e) = window.emit("tuner-progress", &payload) {
            eprintln!("[tuner] failed to emit progress: {}", e);
        }
    };

    emit_progress("decode", 0.02);

    // Decode + analysis on a blocking thread so we don't stall the tokio reactor.
    // The progress callback needs to cross the thread boundary; we use an mpsc channel.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(String, f64)>();
    let path_for_blocking = path.clone();
    let analyze_handle = tokio::task::spawn_blocking(move || -> Result<(
        crate::analysis::key_detector::KeyDiagnostic,
        f64,
        u64, // decode_ms
        u64, // tempo_ms
    ), String> {
        let _ = tx.send(("decode".to_string(), 0.05));
        let decode_start = Instant::now();
        let samples = crate::analysis::decoder::decode_audio(&path_for_blocking)
            .map_err(|e| format!("Decode failed: {}", e))?;
        let decode_ms = decode_start.elapsed().as_millis() as u64;
        let _ = tx.send(("decode".to_string(), 0.20));

        let tx_for_stages = tx.clone();
        let diagnostic = crate::analysis::key_detector::detect_key_diagnostic(
            &samples,
            crate::analysis::ensemble::ProfileWeights::default(),
            |stage, percent| {
                let _ = tx_for_stages.send((stage.to_string(), percent));
            },
        )
        .map_err(|e| format!("Key detection failed: {}", e))?;

        let _ = tx.send(("tempo".to_string(), 0.88));
        let tempo_start = Instant::now();
        let bpm = crate::analysis::tempo_detector::detect_tempo(&samples)
            .map_err(|e| format!("Tempo detection failed: {}", e))?;
        let tempo_ms = tempo_start.elapsed().as_millis() as u64;
        let _ = tx.send(("tempo".to_string(), 0.95));

        Ok((diagnostic, bpm, decode_ms, tempo_ms))
    });

    // Drain the progress channel concurrently while the blocking task runs.
    let window_for_drain = window.clone();
    tokio::spawn(async move {
        while let Some((stage, percent)) = rx.recv().await {
            let payload = TunerProgress { stage, percent };
            let _ = window_for_drain.emit("tuner-progress", &payload);
        }
    });

    let (diagnostic, bpm, decode_ms, tempo_ms) = analyze_handle
        .await
        .map_err(|e| format!("Join error: {}", e))??;

    // Repackage stage timings into the wire shape.
    timings.decode_ms = decode_ms;
    timings.spectrogram_ms = diagnostic.timings.spectrogram;
    timings.hpss_ms = diagnostic.timings.hpss;
    timings.chromagram_ms = diagnostic.timings.chromagram;
    timings.ensemble_ms = diagnostic.timings.ensemble;
    timings.tempo_ms = tempo_ms;

    // Read metadata (title/artist/album/duration/format) — best-effort.
    let metadata_start = Instant::now();
    let (title, artist, album, duration_ms, file_format, sample_rate, bit_depth) =
        match Probe::open(&path).and_then(|p| p.read()) {
            Ok(tagged) => {
                let props = tagged.properties();
                let dur = props.duration().as_millis() as i64;
                let sr = props.sample_rate().map(|r| r as i64);
                let bd = props.bit_depth().map(|d| d as i64);
                let tag = tagged.primary_tag();
                let title = tag.and_then(|t| t.title().map(|s| s.to_string()));
                let artist = tag.and_then(|t| t.artist().map(|s| s.to_string()));
                let album = tag.and_then(|t| t.album().map(|s| s.to_string()));
                let fmt = std::path::Path::new(&path)
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_lowercase();
                (title, artist, album, dur, fmt, sr, bd)
            }
            Err(e) => {
                eprintln!("[tuner] metadata read failed for {}: {}", path, e);
                (None, None, None, 0, String::new(), None, None)
            }
        };
    timings.metadata_ms = metadata_start.elapsed().as_millis() as u64;

    // Best-effort album-art extraction. We need the track id (assigned by
    // the DB upsert below) before we can write the cached image, so this
    // is split: probe + cache below, after the upsert.

    // Pick the winner.
    let winner = diagnostic
        .candidates
        .first()
        .cloned()
        .unwrap_or(crate::analysis::ensemble::RankedCandidate {
            tonic: 0,
            is_major: true,
            confidence: 0.0,
            agreement: 0.0,
            avg_score: 0.0,
            segment_count: 0,
        });
    let key_standard = format!(
        "{} {}",
        crate::analysis::pitch_class_to_name(winner.tonic),
        if winner.is_major { "major" } else { "minor" }
    );
    let key_camelot = crate::analysis::key_to_camelot(winner.tonic, winner.is_major);

    // Resolve the artwork cache directory once. This must come before the
    // DB lock so that the lock-section stays focused on DB work only.
    let art_cache_dir = window
        .app_handle()
        .path()
        .app_data_dir()
        .map(|d| d.join("art"))
        .ok();

    // Auto-import into the library (UPSERT + metadata + analysis).
    let track_id = {
        let db = state.db.lock().await;
        let track_id = db
            .insert_track(&path, &filename, file_size)
            .map_err(|e| format!("DB insert failed: {}", e))?;
        if let Err(e) = db.update_track_metadata(
            track_id,
            title.as_deref(),
            artist.as_deref(),
            album.as_deref(),
            Some(duration_ms),
            &file_format,
            sample_rate,
            bit_depth,
        ) {
            eprintln!("[tuner] metadata write failed: {}", e);
        }
        if let Err(e) = db.update_track_analysis(
            track_id,
            &key_standard,
            &key_camelot,
            winner.confidence,
            bpm,
        ) {
            eprintln!("[tuner] analysis write failed: {}", e);
        }
        track_id
    };

    // Extract + cache embedded artwork after the upsert (we need the row id).
    // Done off-lock and best-effort; failures only log.
    let artwork_path: Option<String> = if let Some(cache_dir) = art_cache_dir.as_ref() {
        let path_for_art = std::path::PathBuf::from(&path);
        let cache_dir = cache_dir.clone();
        let extracted =
            tokio::task::spawn_blocking(move || {
                crate::analysis::art::extract_and_cache_artwork(&path_for_art, &cache_dir, track_id)
            })
            .await
            .unwrap_or_else(|e| {
                eprintln!("[tuner] artwork extraction join error: {}", e);
                Ok(None)
            });
        match extracted {
            Ok(Some(p)) => p.to_str().map(|s| s.to_string()),
            Ok(None) => None,
            Err(e) => {
                eprintln!("[tuner] artwork extraction failed: {}", e);
                None
            }
        }
    } else {
        None
    };

    // Persist the artwork path and emit a `track-analyzed` event with the
    // fully populated row (including artwork_path) so the Library view picks
    // up the cover art live, not just the key/BPM.
    {
        let db = state.db.lock().await;
        if let Some(ref ap) = artwork_path {
            if let Err(e) = db.update_track_artwork(track_id, Some(ap.as_str())) {
                eprintln!("[tuner] artwork path write failed: {}", e);
            }
        }
        if let Ok(Some(track)) = db.get_track_by_id(track_id) {
            let _ = window.emit("track-analyzed", &track);
        }
    }

    timings.total_ms = total_start.elapsed().as_millis() as u64;

    // Final structured log. Tuned to be greppable in dev console.
    eprintln!(
        "[tuner] DONE  {}  -> {} ({})  bpm={:.1}  conf={:.2}  agree={:.2}  segs={}/8  total={}ms  (decode={} spec={} hpss={} chroma={} ens={} tempo={} meta={})",
        filename,
        key_standard, key_camelot, bpm, winner.confidence, winner.agreement, winner.segment_count,
        timings.total_ms,
        timings.decode_ms, timings.spectrogram_ms, timings.hpss_ms,
        timings.chromagram_ms, timings.ensemble_ms, timings.tempo_ms, timings.metadata_ms,
    );
    if diagnostic.candidates.len() > 1 {
        eprintln!("[tuner] candidates:");
        for (i, c) in diagnostic.candidates.iter().take(5).enumerate() {
            let name = format!(
                "{} {}",
                crate::analysis::pitch_class_to_name(c.tonic),
                if c.is_major { "major" } else { "minor" }
            );
            let cam = crate::analysis::key_to_camelot(c.tonic, c.is_major);
            eprintln!(
                "  {}.  {:>10}  ({:>3})  conf={:.3}  agree={:.2}  segs={}/8  score={:.3}",
                i + 1, name, cam, c.confidence, c.agreement, c.segment_count, c.avg_score
            );
        }
    }

    emit_progress("done", 1.0);

    let candidates: Vec<KeyCandidate> = diagnostic
        .candidates
        .iter()
        .take(5)
        .map(candidate_from_ranked)
        .collect();

    Ok(TunerAnalysis {
        track_id,
        file_path: path,
        filename,
        key_standard,
        key_camelot,
        key_confidence: winner.confidence,
        bpm,
        duration_ms,
        energy_level: None,
        title,
        artist,
        album,
        artwork_path,
        candidates,
        chroma: diagnostic.chroma_mean,
        timings,
    })
}

#[command]
pub async fn read_file_metadata(path: String) -> Result<FileMetadata, String> {
    use lofty::file::{AudioFile, TaggedFileExt};
    use lofty::probe::Probe;
    use lofty::tag::Accessor;
    
    let tagged_file = Probe::open(&path)
        .map_err(|e| e.to_string())?
        .read()
        .map_err(|e| e.to_string())?;
    
    let properties = tagged_file.properties();
    let duration_ms = properties.duration().as_millis() as i64;
    let sample_rate = properties.sample_rate().map(|r| r as i64);
    let bit_depth = properties.bit_depth().map(|d| d as i64);
    
    let tag = tagged_file.primary_tag();
    let title = tag.and_then(|t| t.title().map(|s| s.to_string()));
    let artist = tag.and_then(|t| t.artist().map(|s| s.to_string()));
    let album = tag.and_then(|t| t.album().map(|s| s.to_string()));
    
    Ok(FileMetadata {
        title,
        artist,
        album,
        duration_ms: Some(duration_ms),
        sample_rate,
        bit_depth,
    })
}

#[derive(serde::Serialize)]
pub struct FileMetadata {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub duration_ms: Option<i64>,
    pub sample_rate: Option<i64>,
    pub bit_depth: Option<i64>,
}

#[command]
pub async fn generate_playlist(
    _state: State<'_, AppState>,
    _start_track_id: i64,
    _rules: PlaylistRules,
    _max_length: usize,
) -> Result<Vec<Track>, String> {
    // TODO: Implement harmonic playlist generation
    Ok(vec![])
}

#[command]
pub async fn get_compatible_tracks(
    _state: State<'_, AppState>,
    _track_id: i64,
    _rules: PlaylistRules,
) -> Result<Vec<Track>, String> {
    // TODO: Implement compatible track finding
    Ok(vec![])
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    pub copied: usize,
    pub failed: usize,
    pub playlist_path: Option<String>,
}

#[command]
pub async fn export_tracks(
    state: State<'_, AppState>,
    track_ids: Vec<i64>,
    target_dir: String,
    options: ExportOptions,
) -> Result<ExportResult, String> {
    let db = state.db.lock().await;
    let mut tracks = Vec::with_capacity(track_ids.len());
    for id in track_ids {
        if let Some(t) = db.get_track_by_id(id).map_err(|e| e.to_string())? {
            tracks.push(t);
        }
    }
    drop(db);
    
    let report = crate::export::export_tracks(
        &tracks,
        std::path::Path::new(&target_dir),
        &options,
    )
    .map_err(|e| e.to_string())?;
    
    Ok(ExportResult {
        copied: report.copied,
        failed: report.failed,
        playlist_path: report.playlist_path.map(|p| p.to_string_lossy().to_string()),
    })
}
