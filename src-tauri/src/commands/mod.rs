use anyhow::Result;
use tauri::{command, Emitter, Manager, State, Window};
use walkdir::WalkDir;

use crate::analysis::energy_detector::detect_energy;
use crate::analysis::genre_profiles::weights_for_genre;
use crate::analysis::key_detector::detect_key;
use crate::analysis::key_timeline::{compute_key_timeline, KeyTimeline};
use crate::analysis::tempo_detector::detect_tempo;
use crate::analysis::waveform::{generate_waveform, WaveformData};
use crate::consensus::{compute_consensus, ConsensusResult, OpinionSource};
use crate::models::*;
use crate::{AppState, AnalysisQueue};

#[command]
pub async fn scan_folder(
    state: State<'_, AppState>,
    path: String,
) -> Result<ScanResult, String> {
    let audio_extensions = [
        "mp3", "wav", "flac", "ogg", "oga", "opus", "aiff", "aif", "m4a", "aac",
        "wma", "alac", "mkv",
        // Video containers — audio is extracted via ffmpeg sidecar.
        "mp4", "mov", "webm", "m4v", "avi", "flv", "mpg", "mpeg", "ts", "3gp",
    ];
    
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
    queue.completed_count = 0;
    queue.elapsed_ms = 0;
    
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

    // Calculate speed and ETA from elapsed time and completed count.
    let elapsed_secs = queue.elapsed_ms as f64 / 1000.0;
    let speed_per_sec = if elapsed_secs > 0.0 && queue.completed_count > 0 {
        queue.completed_count as f64 / elapsed_secs
    } else {
        0.0
    };

    let remaining = total.saturating_sub(completed);
    let eta_seconds = if speed_per_sec > 0.0 {
        remaining as f64 / speed_per_sec
    } else {
        0.0
    };

    Ok(AnalysisProgress {
        total,
        completed,
        in_progress,
        speed_per_sec,
        eta_seconds,
    })
}

/// CPU-bound work for a single track. No DB, no IO beyond decoding.
/// Returns `(track_id, file_path, result)` where the result contains the
/// analysis numbers ready to be persisted.
struct TrackAnalysisRaw {
    track_id: i64,
    file_path: String,
    outcome: Result<(String, String, f64, f64, Option<i32>)>, // (key_standard, key_camelot, confidence, bpm, energy)
}

fn analyze_cpu(track_id: i64, file_path: String) -> TrackAnalysisRaw {
    let outcome: Result<(String, String, f64, f64, Option<i32>)> = (|| {
        let samples = crate::media::decode_media(&file_path)?;
        let key = detect_key(&samples)?;
        let bpm = detect_tempo(&samples)?;
        // Detect energy if the track doesn't already have one from MIK.
        // We detect it regardless — the caller can choose which to keep.
        let energy = detect_energy(&samples, crate::analysis::SAMPLE_RATE);
        Ok((key.key_standard, key.key_camelot, key.confidence, bpm, Some(energy.energy_level)))
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

        // Track batch timing for speed/ETA calculation.
        let batch_start = std::time::Instant::now();

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

        let batch_elapsed_ms = batch_start.elapsed().as_millis();

        // Serialise DB writes + event emits.
        let db_guard = db.lock().await;
        let mut completed_this_batch = 0;
        for r in results {
            match r.outcome {
                Ok((key_std, key_cam, conf, bpm, energy)) => {
                    if let Err(e) = db_guard.update_track_analysis(r.track_id, &key_std, &key_cam, conf, bpm) {
                        eprintln!("[analyze] DB write failed for {}: {}", r.track_id, e);
                        continue;
                    }
                    // Update energy level if we detected one
                    if let Some(energy_level) = energy {
                        let _ = db_guard.update_track_energy(r.track_id, energy_level);
                    }
                    // Also store as a TuneLock opinion for consensus
                    let _ = db_guard.upsert_opinion(
                        r.track_id,
                        "tunelock",
                        Some(&key_cam),
                        Some(&key_std),
                        Some(bpm),
                        energy,
                        conf,
                        "TuneLock engine",
                    );
                    completed_this_batch += 1;
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

        // Update queue timing stats.
        {
            let mut q = queue.lock().await;
            q.completed_count += completed_this_batch;
            q.elapsed_ms += batch_elapsed_ms;
        }
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
    /// Number of valid temporal sections used for the section-vote evidence.
    pub section_count: usize,
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
///   6. tempo + energy -> 95%
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
        i32, // energy_level
    ), String> {
        let _ = tx.send(("decode".to_string(), 0.05));
        let decode_start = Instant::now();
        let samples = crate::media::decode_media(&path_for_blocking)
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
        let energy_level = detect_energy(&samples, crate::analysis::SAMPLE_RATE).energy_level;
        let _ = tx.send(("tempo".to_string(), 0.95));

        Ok((diagnostic, bpm, decode_ms, tempo_ms, energy_level))
    });

    // Drain the progress channel concurrently while the blocking task runs.
    let window_for_drain = window.clone();
    tokio::spawn(async move {
        while let Some((stage, percent)) = rx.recv().await {
            let payload = TunerProgress { stage, percent };
            let _ = window_for_drain.emit("tuner-progress", &payload);
        }
    });

    let (diagnostic, bpm, decode_ms, tempo_ms, energy_level) = analyze_handle
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
        if let Err(e) = db.update_track_energy(track_id, energy_level) {
            eprintln!("[tuner] energy write failed: {}", e);
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
    let section_count: usize = diagnostic
        .candidates
        .iter()
        .map(|candidate| candidate.segment_count)
        .sum();

    // Final structured log. Tuned to be greppable in dev console.
    eprintln!(
        "[tuner] DONE  {}  -> {} ({})  bpm={:.1}  conf={:.2}  agree={:.2}  segs={}/{}  total={}ms  (decode={} spec={} hpss={} chroma={} ens={} tempo={} meta={})",
        filename, key_standard, key_camelot, bpm, winner.confidence, winner.agreement,
        winner.segment_count, section_count,
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
                "  {}.  {:>10}  ({:>3})  conf={:.3}  agree={:.2}  segs={}/{}  score={:.3}",
                i + 1, name, cam, c.confidence, c.agreement, c.segment_count,
                section_count, c.avg_score
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
        energy_level: Some(energy_level),
        title,
        artist,
        album,
        artwork_path,
        candidates,
        section_count,
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
    state: State<'_, AppState>,
    start_track_id: i64,
    rules: PlaylistRules,
    max_length: usize,
) -> Result<Vec<Track>, String> {
    let db = state.db.lock().await;

    // Get the seed track
    let seed = db.get_track_by_id(start_track_id)
        .map_err(|e| e.to_string())?
        .ok_or("Seed track not found")?;

    let seed_key = match &seed.key_camelot {
        Some(k) => k.clone(),
        None => return Ok(vec![seed]), // No key → can't find compatible tracks
    };

    // Get all analyzed tracks from the library
    let page = db.get_library_page(0, 5000, "bpm", "asc", None)
        .map_err(|e| e.to_string())?;

    // Filter to tracks with a key and that aren't the seed
    let candidates: Vec<Track> = page.tracks.into_iter()
        .filter(|t| t.id != seed.id && t.key_camelot.is_some())
        .collect();

    // Score each candidate by harmonic compatibility and BPM similarity
    let seed_bpm = seed.bpm.unwrap_or(128.0);
    let mut scored: Vec<(f64, Track)> = candidates.into_iter()
        .map(|t| {
            let compat = harmony_compatibility_score(&seed_key, t.key_camelot.as_ref().unwrap(), &rules);
            let bpm_diff = ((t.bpm.unwrap_or(seed_bpm) - seed_bpm).abs()).min(20.0);
            let bpm_score = 1.0 - (bpm_diff / 20.0);
            let score = compat * 0.7 + bpm_score * 0.3;
            (score, t)
        })
        .filter(|(s, _)| *s > 0.0)
        .collect();

    // Sort by score descending
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    // Build the playlist: seed + top candidates
    let mut playlist = vec![seed];
    for (_, track) in scored.into_iter().take(max_length.saturating_sub(1)) {
        playlist.push(track);
    }

    Ok(playlist)
}

#[command]
pub async fn get_compatible_tracks(
    state: State<'_, AppState>,
    track_id: i64,
    rules: PlaylistRules,
) -> Result<Vec<Track>, String> {
    let db = state.db.lock().await;

    // Get the focal track
    let focal = db.get_track_by_id(track_id)
        .map_err(|e| e.to_string())?
        .ok_or("Track not found")?;

    let focal_key = match &focal.key_camelot {
        Some(k) => k.clone(),
        None => return Ok(vec![]),
    };

    // Get analyzed tracks
    let page = db.get_library_page(0, 5000, "key_camelot", "asc", None)
        .map_err(|e| e.to_string())?;

    // Filter and score
    let candidates: Vec<Track> = page.tracks.into_iter()
        .filter(|t| t.id != focal.id && t.key_camelot.is_some())
        .filter(|t| {
            harmony_compatibility_score(&focal_key, t.key_camelot.as_ref().unwrap(), &rules) > 0.0
        })
        .collect();

    Ok(candidates)
}

/// Compute a harmonic compatibility score (0.0–1.0) between two Camelot keys
/// based on the selected playlist rules.
fn harmony_compatibility_score(seed: &str, candidate: &str, rules: &PlaylistRules) -> f64 {
    if seed == candidate {
        return if rules.same_key { 1.0 } else { 0.0 };
    }

    // Parse Camelot positions
    let (seed_num, seed_letter) = parse_camelot(seed);
    let (cand_num, cand_letter) = parse_camelot(candidate);

    if seed_num == 0 || cand_num == 0 {
        return 0.0;
    }

    let diff = ((cand_num - seed_num + 12) % 12) as i32;
    let same_mode = seed_letter == cand_letter;

    // +1 or -1 (same mode, adjacent on the wheel)
    if same_mode && (diff == 1 || diff == 11) {
        return if rules.plus_one || rules.minus_one { 0.9 } else { 0.0 };
    }

    // +2 or -2 (energy boost)
    if same_mode && (diff == 2 || diff == 10) {
        return if rules.plus_two || rules.minus_two { 0.7 } else { 0.0 };
    }

    // Major → Minor (dominant to subdominant: same number, A→B)
    if !same_mode && seed_num == cand_num {
        if seed_letter == 'A' && cand_letter == 'B' {
            return if rules.dominant_to_subdominant { 0.8 } else { 0.0 };
        }
        if seed_letter == 'B' && cand_letter == 'A' {
            return if rules.subdominant_to_dominant { 0.8 } else { 0.0 };
        }
    }

    // Relative major/minor (9A → 10B, 10B → 9A, etc.)
    if !same_mode {
        let relative_diff = if seed_letter == 'A' {
            ((cand_num - seed_num + 1 + 12) % 12) as i32
        } else {
            ((cand_num - seed_num - 1 + 12) % 12) as i32
        };
        if relative_diff == 0 {
            return 0.6; // Relative major/minor — always compatible
        }
    }

    0.0
}

/// Parse a Camelot key like "5A" into (number, letter).
fn parse_camelot(key: &str) -> (i32, char) {
    let key = key.trim();
    if key.len() < 2 {
        return (0, ' ');
    }
    let letter = key.chars().last().unwrap();
    let num: i32 = key[..key.len() - 1].parse().unwrap_or(0);
    (num, letter)
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

// ============================================================================
// Playlist commands
// ============================================================================

#[command]
pub async fn save_playlist(
    state: State<'_, AppState>,
    name: String,
    track_ids: Vec<i64>,
    description: Option<String>,
) -> Result<Playlist, String> {
    let db = state.db.lock().await;
    let playlist = db.create_playlist(&name, description.as_deref())
        .map_err(|e| e.to_string())?;
    for (i, track_id) in track_ids.iter().enumerate() {
        db.add_track_to_playlist(playlist.id, *track_id, i as i64)
            .map_err(|e| e.to_string())?;
    }
    Ok(playlist)
}

#[command]
pub async fn get_playlists(state: State<'_, AppState>) -> Result<Vec<Playlist>, String> {
    let db = state.db.lock().await;
    db.get_playlists().map_err(|e| e.to_string())
}

#[command]
pub async fn delete_playlist(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    let db = state.db.lock().await;
    db.delete_playlist(id).map_err(|e| e.to_string())
}

#[command]
pub async fn save_mix(
    state: State<'_, AppState>,
    id: Option<i64>,
    name: String,
    description: Option<String>,
    track_ids: Vec<i64>,
    clip_notes: Vec<(usize, String)>,
) -> Result<i64, String> {
    let db = state.db.lock().await;
    db.save_mix(id, &name, description.as_deref(), &track_ids, &clip_notes)
        .map_err(|e| e.to_string())
}

#[command]
pub async fn load_mix(
    state: State<'_, AppState>,
    playlist_id: i64,
) -> Result<serde_json::Value, String> {
    let db = state.db.lock().await;
    let (playlist, track_ids, clip_notes) = db.load_mix(playlist_id)
        .map_err(|e| e.to_string())?;
    Ok(serde_json::json!({
        "id": playlist.id,
        "name": playlist.name,
        "description": playlist.description,
        "trackIds": track_ids,
        "clipNotes": clip_notes,
        "createdAt": playlist.created_at,
    }))
}

#[command]
pub async fn get_playlist_tracks(
    state: State<'_, AppState>,
    playlist_id: i64,
) -> Result<Vec<Track>, String> {
    let db = state.db.lock().await;
    db.get_playlist_tracks(playlist_id).map_err(|e| e.to_string())
}

// ============================================================================
// MIK CSV import
// ============================================================================

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MikImportResult {
    pub total_rows: usize,
    pub matched: usize,
    pub unmatched: usize,
    pub errors: Vec<String>,
}

#[command]
pub async fn import_mik_csv(
    state: State<'_, AppState>,
    csv_path: String,
) -> Result<MikImportResult, String> {
    let file = std::fs::File::open(&csv_path)
        .map_err(|e| format!("Failed to open CSV: {}", e))?;
    let mut rdr = csv::Reader::from_reader(file);

    // MIK CSV columns: Title, Artist, Key, Tempo, Genre, Album, Grouping,
    // Date Added, Location, Comment, Year, Overall Volume, Energy, CuePoints, ClippedPeaks
    #[derive(serde::Deserialize, Debug)]
    struct MikRow {
        #[serde(rename = "Title")]
        _title: String,
        #[serde(rename = "Key")]
        key: String,
        #[serde(rename = "Genre")]
        genre: String,
        #[serde(rename = "Location")]
        location: String,
        #[serde(rename = "Energy")]
        energy: String,
    }

    let db = state.db.lock().await;
    let mut total_rows = 0;
    let mut matched = 0;
    let mut unmatched = 0;
    let mut errors: Vec<String> = Vec::new();

    for result in rdr.deserialize::<MikRow>() {
        total_rows += 1;
        match result {
            Ok(row) => {
                let energy: Option<i32> = row.energy.trim().parse().ok();
                let genre = if row.genre.trim().is_empty() {
                    None
                } else {
                    Some(row.genre.trim())
                };
                let mik_key = if row.key.trim().is_empty() {
                    None
                } else {
                    Some(row.key.trim())
                };
                match db.update_mik_reference(&row.location, mik_key, energy, genre) {
                    Ok(true) => {
                        matched += 1;
                        // Also store as an MIK opinion for consensus.
                        // We need the track_id — look it up by path.
                        let normalized = row.location.replace('/', "\\");
                        let track = db.get_track_by_path(&row.location)
                            .or_else(|_| db.get_track_by_path(&normalized))
                            .ok()
                            .flatten();
                        if let Some(t) = track {
                            let _ = db.upsert_opinion(
                                t.id,
                                "mik",
                                mik_key,
                                None,
                                None,
                                energy,
                                1.0,
                                "MIK CSV import",
                            );
                        }
                    }
                    Ok(false) => unmatched += 1,
                    Err(e) => {
                        errors.push(format!("Row {}: {}", total_rows, e));
                        unmatched += 1;
                    }
                }
            }
            Err(e) => {
                errors.push(format!("Row {}: parse error: {}", total_rows, e));
                unmatched += 1;
            }
        }
    }

    Ok(MikImportResult {
        total_rows,
        matched,
        unmatched,
        errors,
    })
}

// ============================================================================
// Consensus commands
// ============================================================================

/// Get the consensus result for a single track (all opinions + agreement).
#[command]
pub async fn get_consensus(
    state: State<'_, AppState>,
    track_id: i64,
) -> Result<ConsensusResult, String> {
    let db = state.db.lock().await;
    let opinions = db.get_opinions_for_track(track_id).map_err(|e| e.to_string())?;
    Ok(compute_consensus(&opinions))
}

/// Get consensus for a batch of tracks (for library display).
/// Returns a map of track_id → ConsensusResult.
#[command]
pub async fn get_consensus_batch(
    state: State<'_, AppState>,
    track_ids: Vec<i64>,
) -> Result<std::collections::HashMap<i64, ConsensusResult>, String> {
    let db = state.db.lock().await;
    let opinions_map = db.get_opinions_batch(&track_ids).map_err(|e| e.to_string())?;
    let mut result = std::collections::HashMap::new();
    for track_id in track_ids {
        let opinions = opinions_map.get(&track_id).cloned().unwrap_or_default();
        result.insert(track_id, compute_consensus(&opinions));
    }
    Ok(result)
}

/// Get the list of tracks with contested opinions (for adjudication queue).
#[command]
pub async fn get_contested_tracks(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> Result<Vec<i64>, String> {
    let db = state.db.lock().await;
    db.get_contested_tracks(limit.unwrap_or(100)).map_err(|e| e.to_string())
}

/// Manually set an opinion for a track (used by adjudication UI to write
/// the human verdict as a "gold" opinion).
#[command]
pub async fn set_track_opinion(
    state: State<'_, AppState>,
    track_id: i64,
    source: String,
    key_camelot: Option<String>,
    key_standard: Option<String>,
    bpm: Option<f64>,
    energy: Option<i32>,
    confidence: Option<f64>,
    provenance: String,
) -> Result<(), String> {
    let db = state.db.lock().await;
    db.upsert_opinion(
        track_id,
        &source,
        key_camelot.as_deref(),
        key_standard.as_deref(),
        bpm,
        energy,
        confidence.unwrap_or(1.0),
        &provenance,
    ).map_err(|e| e.to_string())
}

// ============================================================================
// Traktor NML import
// ============================================================================

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NmlImportResult {
    pub total_entries: usize,
    pub matched: usize,
    pub unmatched: usize,
    pub errors: Vec<String>,
}

/// Import a Traktor collection.nml file. Parses each entry's key and BPM
/// and stores them as opinions with source "traktor".
#[command]
pub async fn import_traktor_nml(
    state: State<'_, AppState>,
    nml_path: String,
) -> Result<NmlImportResult, String> {
    let file_content = std::fs::read_to_string(&nml_path)
        .map_err(|e| format!("Failed to read NML: {}", e))?;

    let db = state.db.lock().await;
    let mut total_entries = 0;
    let mut matched = 0;
    let mut unmatched = 0;
    let mut errors: Vec<String> = Vec::new();

    // Simple NML parsing: extract <ENTRY> blocks with KEY and BPM attributes.
    // NML format: <ENTRY TITLE="..." ARTIST="..."><LOCATION FILE="..." DIR="..."/><MUSICAL_KEY VALUE="..."/><TEMPO BPM="..."/></ENTRY>
    // We use a simple string-based parser to avoid adding a heavy XML dependency.
    let content = file_content.as_str();
    let mut pos = 0;
    while let Some(entry_start) = content[pos..].find("<ENTRY ") {
        let abs_start = pos + entry_start;
        let entry_end = content[abs_start..].find("</ENTRY>")
            .map(|e| abs_start + e + 8)
            .unwrap_or(content.len());
        let entry = &content[abs_start..entry_end];
        total_entries += 1;

        // Extract location (file + dir)
        let file_name = extract_attr(entry, "LOCATION", "FILE");
        let dir = extract_attr(entry, "LOCATION", "DIR");

        // Extract musical key (Traktor uses internal numeric key codes)
        let key_val = extract_attr(entry, "MUSICAL_KEY", "VALUE");

        // Extract BPM
        let bpm_str = extract_attr(entry, "TEMPO", "BPM");

        // Build the full path (Traktor dirs end with : and use forward slashes)
        let full_path = if let (Some(f), Some(d)) = (&file_name, &dir) {
            let clean_dir = d.replace("file://localhost/", "").replace("/", "\\");
            Some(format!("{}{}", clean_dir, f))
        } else {
            file_name.clone()
        };

        // Try to match by path or filename
        let matched_track = if let Some(ref path) = full_path {
            // Try exact match
            let normalized = path.replace('/', "\\");
            db.get_track_by_path(path)
                .or_else(|_| db.get_track_by_path(&normalized))
                .or_else(|_| {
                    // Try by filename only
                    if let Some(fname) = path.rsplit(['\\', '/']).next() {
                        db.get_track_by_filename(fname)
                    } else {
                        Ok(None)
                    }
                })
                .map_err(|e| e.to_string())?
        } else {
            None
        };

        if let Some(track) = matched_track {
            // Convert Traktor key code to Camelot.
            // Traktor uses a numeric mapping: 0=C maj(8B), 1=D maj(9B), etc.
            let camelot = key_val
                .as_deref()
                .and_then(|v| v.parse::<u32>().ok())
                .and_then(traktor_key_to_camelot);

            let bpm = bpm_str.as_deref().and_then(|s| s.parse::<f64>().ok());

            db.upsert_opinion(
                track.id,
                "traktor",
                camelot.as_deref(),
                None,
                bpm,
                None,
                1.0,
                "Traktor NML import",
            ).map_err(|e| e.to_string())?;
            matched += 1;
        } else {
            unmatched += 1;
        }

        pos = entry_end;
    }

    Ok(NmlImportResult {
        total_entries,
        matched,
        unmatched,
        errors,
    })
}

/// Extract an XML attribute value from a tag.
/// e.g. extract_attr(s, "TEMPO", "BPM") finds <TEMPO BPM="128.0" ...> and returns "128.0"
fn extract_attr(content: &str, tag: &str, attr: &str) -> Option<String> {
    let tag_start = content.find(&format!("<{}", tag))?;
    let tag_end = content[tag_start..].find('>')?;
    let tag_content = &content[tag_start..tag_start + tag_end];
    let attr_pattern = format!("{}=\"", attr);
    let attr_start = tag_content.find(&attr_pattern)?;
    let value_start = attr_start + attr_pattern.len();
    let value_end = tag_content[value_start..].find('"')?;
    Some(tag_content[value_start..value_start + value_end].to_string())
}

/// Convert Traktor's internal key code to Camelot notation.
/// Traktor uses a 0-indexed mapping where:
/// 0=C maj(8B), 1=D maj(9B), 2=E maj(10B), ..., 11=A maj(7B)
/// 12=C min(5A), 13=D min(6A), ..., 23=A min(4A)
fn traktor_key_to_camelot(code: u32) -> Option<String> {
    let major = [
        "8B", "9B", "10B", "11B", "12B", "1B", "2B", "3B", "4B", "5B", "6B", "7B",
    ];
    let minor = [
        "5A", "6A", "7A", "8A", "9A", "10A", "11A", "12A", "1A", "2A", "3A", "4A",
    ];
    if code < 12 {
        Some(major[code as usize].to_string())
    } else if code < 24 {
        Some(minor[(code - 12) as usize].to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_traktor_key_mapping() {
        assert_eq!(traktor_key_to_camelot(0), Some("8B".to_string())); // C major
        assert_eq!(traktor_key_to_camelot(1), Some("9B".to_string())); // D major
        assert_eq!(traktor_key_to_camelot(12), Some("5A".to_string())); // C minor
        assert_eq!(traktor_key_to_camelot(13), Some("6A".to_string())); // D minor
        assert_eq!(traktor_key_to_camelot(24), None); // Invalid
    }

    #[test]
    fn test_extract_attr() {
        let xml = r#"<TEMPO BPM="128.5" /><MUSICAL_KEY VALUE="0" />"#;
        assert_eq!(extract_attr(xml, "TEMPO", "BPM"), Some("128.5".to_string()));
        assert_eq!(extract_attr(xml, "MUSICAL_KEY", "VALUE"), Some("0".to_string()));
    }
}

// ============================================================================
// Waveform generation
// ============================================================================

/// Generate a three-band waveform for a track. The waveform is computed
/// from the decoded audio and returned immediately (not cached yet —
/// caching to disk will be added in a future pass).
#[command]
pub async fn get_waveform_data(
    state: State<'_, AppState>,
    track_id: i64,
) -> Result<WaveformData, String> {
    let db = state.db.lock().await;
    let track = db.get_track_by_id(track_id)
        .map_err(|e| e.to_string())?
        .ok_or("Track not found")?;
    drop(db);

    // Decode the audio file
    let samples = crate::media::decode_media(&track.file_path)
        .map_err(|e| format!("Decode failed: {}", e))?;

    // Generate the waveform (CPU-heavy, run in spawn_blocking)
    let waveform = tokio::task::spawn_blocking(move || {
        generate_waveform(&samples)
    })
    .await
    .map_err(|e| format!("Waveform generation failed: {}", e))?;

    Ok(waveform)
}

// ============================================================================
// Key timeline (modulation detection + abstention)
// ============================================================================

/// Compute a per-segment key timeline for a track. Shows where the key
/// changes throughout the track, and whether the track has a stable key
/// at all (abstention for atonal/noisy material).
#[command]
pub async fn get_key_timeline(
    state: State<'_, AppState>,
    track_id: i64,
) -> Result<KeyTimeline, String> {
    let db = state.db.lock().await;
    let track = db.get_track_by_id(track_id)
        .map_err(|e| e.to_string())?
        .ok_or("Track not found")?;

    // Get genre for adaptive profile weights
    let genre = db.get_track_genre(track_id).map_err(|e| e.to_string())?;
    drop(db);

    let samples = crate::media::decode_media(&track.file_path)
        .map_err(|e| format!("Decode failed: {}", e))?;

    let weights = weights_for_genre(genre.as_deref());

    let timeline = tokio::task::spawn_blocking(move || {
        compute_key_timeline(&samples, weights)
    })
    .await
    .map_err(|e| format!("Timeline computation failed: {}", e))?
    .map_err(|e| format!("Timeline computation failed: {}", e))?;

    Ok(timeline)
}

// ============================================================================
// Gold set annotation commands (Step 6)
// ============================================================================

#[command]
pub async fn save_gold_annotation(
    state: State<'_, AppState>,
    annotation: GoldAnnotation,
) -> Result<i64, String> {
    let db = state.db.lock().await;
    db.save_gold_annotation(&annotation).map_err(|e| e.to_string())
}

#[command]
pub async fn get_gold_annotations(
    state: State<'_, AppState>,
    track_id: i64,
) -> Result<Vec<GoldAnnotation>, String> {
    let db = state.db.lock().await;
    db.get_gold_annotations(track_id).map_err(|e| e.to_string())
}

#[command]
pub async fn get_gold_annotation_summary(
    state: State<'_, AppState>,
) -> Result<GoldAnnotationSummary, String> {
    let db = state.db.lock().await;
    db.get_gold_annotation_summary().map_err(|e| e.to_string())
}

#[command]
pub async fn save_training_session(
    state: State<'_, AppState>,
    session: TrainingSession,
) -> Result<i64, String> {
    let db = state.db.lock().await;
    db.save_training_session(&session).map_err(|e| e.to_string())
}

#[command]
pub async fn get_training_stats(
    state: State<'_, AppState>,
) -> Result<TrainingStats, String> {
    let db = state.db.lock().await;
    db.get_training_stats().map_err(|e| e.to_string())
}

// ============================================================================
// Assist layer commands (Phase 11)
// ============================================================================

#[command]
pub async fn assist_status(
    state: State<'_, AppState>,
) -> Result<crate::assist::AssistStatus, String> {
    let (available, models) = state.ollama.check_status().await;
    let enabled = *state.assist_enabled.lock().await;
    let selected_model = state.assist_model.lock().await.clone();
    Ok(crate::assist::AssistStatus {
        available,
        ollama_url: "http://localhost:11434".to_string(),
        models,
        selected_model,
        enabled,
    })
}

#[command]
pub async fn assist_set_enabled(
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<(), String> {
    *state.assist_enabled.lock().await = enabled;
    Ok(())
}

#[command]
pub async fn assist_set_model(
    state: State<'_, AppState>,
    model: String,
) -> Result<(), String> {
    *state.assist_model.lock().await = Some(model);
    Ok(())
}

#[command]
pub async fn assist_analyze_setlist(
    state: State<'_, AppState>,
    raw_text: String,
) -> Result<crate::assist::SetlistAnalysis, String> {
    let enabled = *state.assist_enabled.lock().await;
    if !enabled {
        return Err("Assist layer is not enabled".to_string());
    }
    let model = state.assist_model.lock().await.clone()
        .ok_or("No model selected")?;

    // Get library tracks for matching
    let db = state.db.lock().await;
    let page = db.get_library_page(0, 5000, "filename", "asc", None)
        .map_err(|e| e.to_string())?;
    drop(db);

    // Build the library tuple for matching
    let library: Vec<(i64, String, Option<String>, Option<String>, Option<String>, Option<f64>, Option<i32>)> = page.tracks.iter().map(|t| {
        (t.id, t.filename.clone(), t.title.clone(), t.artist.clone(),
         t.key_camelot.clone(), t.bpm, t.energy_level)
    }).collect();

    crate::assist::analyze_setlist(&state.ollama, &model, &raw_text, &library)
        .await
        .map_err(|e| e.to_string())
}

#[command]
pub async fn assist_repair_metadata(
    state: State<'_, AppState>,
) -> Result<crate::assist::MetadataRepairBatch, String> {
    let enabled = *state.assist_enabled.lock().await;
    if !enabled {
        return Err("Assist layer is not enabled".to_string());
    }
    let model = state.assist_model.lock().await.clone()
        .ok_or("No model selected")?;

    // Get library tracks with missing metadata
    let db = state.db.lock().await;
    let page = db.get_library_page(0, 5000, "filename", "asc", None)
        .map_err(|e| e.to_string())?;
    drop(db);

    let tracks: Vec<(i64, String, Option<String>, Option<String>, Option<String>, Option<String>)> = page.tracks.iter().map(|t| {
        (t.id, t.filename.clone(), t.title.clone(), t.artist.clone(),
         t.album.clone(), None) // genre not in Track struct yet
    }).collect();

    crate::assist::repair_metadata(&state.ollama, &model, &tracks)
        .await
        .map_err(|e| e.to_string())
}

#[command]
pub async fn assist_apply_metadata_repair(
    state: State<'_, AppState>,
    proposal: crate::assist::MetadataProposal,
) -> Result<(), String> {
    // Apply a single metadata proposal to the database
    let db = state.db.lock().await;
    db.update_track_metadata(
        proposal.track_id,
        proposal.proposed_title.as_deref(),
        proposal.proposed_artist.as_deref(),
        proposal.proposed_album.as_deref(),
        None, // duration_ms — not changed
        "",   // file_format — not changed
        None, None, // sample_rate, bit_depth — not changed
    ).map_err(|e| e.to_string())?;
    Ok(())
}

#[command]
pub async fn assist_infer_genres(
    state: State<'_, AppState>,
) -> Result<Vec<crate::assist::GenreInference>, String> {
    let enabled = *state.assist_enabled.lock().await;
    if !enabled {
        return Err("Assist layer is not enabled".to_string());
    }
    let model = state.assist_model.lock().await.clone()
        .ok_or("No model selected")?;

    let db = state.db.lock().await;
    let page = db.get_library_page(0, 5000, "filename", "asc", None)
        .map_err(|e| e.to_string())?;
    drop(db);

    let tracks: Vec<(i64, String, Option<String>, Option<String>)> = page.tracks.iter().map(|t| {
        (t.id, t.filename.clone(), t.title.clone(), t.artist.clone())
    }).collect();

    crate::assist::infer_genres(&state.ollama, &model, &tracks)
        .await
        .map_err(|e| e.to_string())
}

#[command]
pub async fn assist_explain_transition(
    state: State<'_, AppState>,
    from_key: String,
    to_key: String,
    from_bpm: Option<f64>,
    to_bpm: Option<f64>,
) -> Result<crate::assist::TransitionExplanation, String> {
    let enabled = *state.assist_enabled.lock().await;
    if !enabled {
        // Return template fallback if assist is not enabled
        let explanation = crate::assist::template_explanation(&from_key, &to_key, from_bpm, to_bpm);
        return Ok(crate::assist::TransitionExplanation {
            from_key,
            to_key,
            from_bpm,
            to_bpm,
            explanation,
            source: "template".to_string(),
        });
    }
    let model = state.assist_model.lock().await.clone()
        .ok_or("No model selected")?;

    crate::assist::explain_transition(&state.ollama, &model, &from_key, &to_key, from_bpm, to_bpm)
        .await
        .map_err(|e| e.to_string())
}

#[command]
pub async fn assist_plan_set(
    state: State<'_, AppState>,
    instruction: String,
) -> Result<crate::assist::SetPlan, String> {
    let enabled = *state.assist_enabled.lock().await;
    if !enabled {
        return Err("Assist layer is not enabled".to_string());
    }
    let model = state.assist_model.lock().await.clone()
        .ok_or("No model selected")?;

    let db = state.db.lock().await;
    let page = db.get_library_page(0, 5000, "filename", "asc", None)
        .map_err(|e| e.to_string())?;
    drop(db);

    let tracks: Vec<(i64, String, Option<String>, Option<String>, Option<f64>, Option<i32>)> = page.tracks.iter().map(|t| {
        (t.id, t.filename.clone(), t.title.clone(), t.key_camelot.clone(), t.bpm, t.energy_level)
    }).collect();

    crate::assist::plan_set(&state.ollama, &model, &instruction, &tracks)
        .await
        .map_err(|e| e.to_string())
}

// ============================================================================
// Transition Workbench commands (Phase 7 / Slice A)
// ============================================================================

#[command]
pub async fn get_beat_grid(
    state: State<'_, AppState>,
    track_id: i64,
) -> Result<Option<BeatGrid>, String> {
    let db = state.db.lock().await;
    db.get_beat_grid(track_id).map_err(|e| e.to_string())
}

#[command]
pub async fn save_beat_grid_override(
    state: State<'_, AppState>,
    track_id: i64,
    bpm: f64,
    first_beat_ms: i64,
    meter_numerator: i32,
    downbeat_offset_beats: i32,
) -> Result<(), String> {
    let db = state.db.lock().await;
    db.save_beat_grid_override(track_id, bpm, first_beat_ms, meter_numerator, downbeat_offset_beats)
        .map_err(|e| e.to_string())
}

#[command]
pub async fn reset_beat_grid_override(
    state: State<'_, AppState>,
    track_id: i64,
) -> Result<(), String> {
    let db = state.db.lock().await;
    db.reset_beat_grid_override(track_id).map_err(|e| e.to_string())
}

#[command]
pub async fn get_transition_plan(
    state: State<'_, AppState>,
    playlist_id: i64,
    transition_id: String,
) -> Result<Option<TransitionPlan>, String> {
    let db = state.db.lock().await;
    db.get_transition_plan(playlist_id, &transition_id).map_err(|e| e.to_string())
}

#[command]
pub async fn save_transition_plan(
    state: State<'_, AppState>,
    plan: TransitionPlan,
) -> Result<(), String> {
    let db = state.db.lock().await;
    db.save_transition_plan(&plan).map_err(|e| e.to_string())
}

#[command]
pub async fn get_stem_manifest(
    state: State<'_, AppState>,
    track_id: i64,
) -> Result<Option<StemManifest>, String> {
    let db = state.db.lock().await;
    db.get_stem_manifest(track_id).map_err(|e| e.to_string())
}

// ============================================================================
// Audio engine commands (Transition Workbench — real-time playback)
//
// The engine uses a generalized player/bus vocabulary:
//   - PlayerId(u8): eight player slots (MAX_PLAYERS=8)
//   - BusId::A, BusId::B, BusId::Master: two crossfader buses + direct-to-master
//   - SourceHandle: lightweight reference to decoded audio in the engine registry
//
// The two-track Transition Workbench uses Players 0 and 1 on Buses A and B.
// The Layer Lab exposes all eight slots.
// ============================================================================

#[command]
pub async fn audio_engine_init(state: State<'_, AppState>) -> Result<u32, String> {
    let mut engine_slot = state.audio_engine.lock().await;
    if engine_slot.is_some() {
        return Err("Audio engine already initialized".to_string());
    }
    let engine = crate::audio::AudioEngine::new().map_err(|e| e)?;
    let sr = engine.sample_rate();
    engine.start().map_err(|e| e)?;
    *engine_slot = Some(engine);
    drop(engine_slot);

    // Spawn an engine-owned non-realtime drain task that periodically
    // drains retired source buffers. This is independent of UI meter
    // polling — it runs even when the UI is hidden, minimized, or not
    // polling meters. Drains every 100ms (10Hz), which is far faster
    // than any realistic source retirement rate.
    let engine_arc = state.audio_engine.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(100));
        loop {
            interval.tick().await;
            let engine_slot = engine_arc.lock().await;
            if let Some(engine) = engine_slot.as_ref() {
                engine.drain_retired_sources();
            } else {
                // Engine was dropped — stop the drain task.
                break;
            }
        }
    });

    Ok(sr)
}

#[command]
pub async fn audio_engine_play(state: State<'_, AppState>, player: u8) -> Result<(), String> {
    let engine_slot = state.audio_engine.lock().await;
    if let Some(engine) = engine_slot.as_ref() {
        let frame = engine.current_frame();
        engine.send_command(crate::audio::EngineCommand::Resume {
            player: crate::audio::PlayerId(player),
            at_frame: frame,
        });
    }
    Ok(())
}

#[command]
pub async fn audio_engine_pause(state: State<'_, AppState>, player: u8) -> Result<(), String> {
    let engine_slot = state.audio_engine.lock().await;
    if let Some(engine) = engine_slot.as_ref() {
        let frame = engine.current_frame();
        engine.send_command(crate::audio::EngineCommand::Pause {
            player: crate::audio::PlayerId(player),
            at_frame: frame,
        });
    }
    Ok(())
}

#[command]
pub async fn audio_engine_stop(state: State<'_, AppState>, player: u8) -> Result<(), String> {
    let engine_slot = state.audio_engine.lock().await;
    if let Some(engine) = engine_slot.as_ref() {
        let frame = engine.current_frame();
        engine.send_command(crate::audio::EngineCommand::Stop {
            player: crate::audio::PlayerId(player),
            at_frame: frame,
        });
    }
    Ok(())
}

#[command]
pub async fn audio_engine_seek(state: State<'_, AppState>, player: u8, source_beat: f64) -> Result<(), String> {
    let engine_slot = state.audio_engine.lock().await;
    if let Some(engine) = engine_slot.as_ref() {
        let frame = engine.current_frame();
        engine.send_command(crate::audio::EngineCommand::Seek {
            player: crate::audio::PlayerId(player),
            at_frame: frame,
            source_beat,
        });
    }
    Ok(())
}

#[command]
pub async fn audio_engine_set_crossfade(state: State<'_, AppState>, position: f32) -> Result<(), String> {
    let engine_slot = state.audio_engine.lock().await;
    if let Some(engine) = engine_slot.as_ref() {
        let frame = engine.current_frame();
        engine.send_command(crate::audio::EngineCommand::SetCrossfade { at_frame: frame, position });
    }
    Ok(())
}

#[command]
pub async fn audio_engine_set_tempo(state: State<'_, AppState>, player: u8, rate: f32) -> Result<(), String> {
    let engine_slot = state.audio_engine.lock().await;
    if let Some(engine) = engine_slot.as_ref() {
        let frame = engine.current_frame();
        engine.send_command(crate::audio::EngineCommand::SetTempo {
            player: crate::audio::PlayerId(player),
            at_frame: frame,
            rate,
        });
    }
    Ok(())
}

#[command]
pub async fn audio_engine_set_pitch(state: State<'_, AppState>, player: u8, semitones: f32) -> Result<(), String> {
    let engine_slot = state.audio_engine.lock().await;
    if let Some(engine) = engine_slot.as_ref() {
        let frame = engine.current_frame();
        engine.send_command(crate::audio::EngineCommand::SetPitch {
            player: crate::audio::PlayerId(player),
            at_frame: frame,
            semitones,
        });
    }
    Ok(())
}

#[command]
pub async fn audio_engine_set_player_gain(state: State<'_, AppState>, player: u8, gain: f32) -> Result<(), String> {
    let engine_slot = state.audio_engine.lock().await;
    if let Some(engine) = engine_slot.as_ref() {
        let frame = engine.current_frame();
        engine.send_command(crate::audio::EngineCommand::SetGain {
            player: crate::audio::PlayerId(player),
            at_frame: frame,
            gain,
            ramp_frames: 220, // ~5ms at 44.1kHz
        });
    }
    Ok(())
}

#[command]
pub async fn audio_engine_set_pan(state: State<'_, AppState>, player: u8, pan: f32) -> Result<(), String> {
    let engine_slot = state.audio_engine.lock().await;
    if let Some(engine) = engine_slot.as_ref() {
        let frame = engine.current_frame();
        engine.send_command(crate::audio::EngineCommand::SetPan {
            player: crate::audio::PlayerId(player),
            at_frame: frame,
            pan,
        });
    }
    Ok(())
}

#[command]
pub async fn audio_engine_set_mute(state: State<'_, AppState>, player: u8, muted: bool) -> Result<(), String> {
    let engine_slot = state.audio_engine.lock().await;
    if let Some(engine) = engine_slot.as_ref() {
        let frame = engine.current_frame();
        engine.send_command(crate::audio::EngineCommand::SetMute {
            player: crate::audio::PlayerId(player),
            at_frame: frame,
            muted,
        });
    }
    Ok(())
}

#[command]
pub async fn audio_engine_set_solo(state: State<'_, AppState>, player: u8, soloed: bool) -> Result<(), String> {
    let engine_slot = state.audio_engine.lock().await;
    if let Some(engine) = engine_slot.as_ref() {
        let frame = engine.current_frame();
        engine.send_command(crate::audio::EngineCommand::SetSolo {
            player: crate::audio::PlayerId(player),
            at_frame: frame,
            soloed,
        });
    }
    Ok(())
}

#[command]
pub async fn audio_engine_set_bus(state: State<'_, AppState>, player: u8, bus: String) -> Result<(), String> {
    let engine_slot = state.audio_engine.lock().await;
    if let Some(engine) = engine_slot.as_ref() {
        let bus_id = match bus.as_str() {
            "a" | "A" => crate::audio::BusId::A,
            "b" | "B" => crate::audio::BusId::B,
            _ => crate::audio::BusId::Master,
        };
        let frame = engine.current_frame();
        engine.send_command(crate::audio::EngineCommand::SetBus {
            player: crate::audio::PlayerId(player),
            at_frame: frame,
            bus: bus_id,
        });
    }
    Ok(())
}

#[command]
pub async fn audio_engine_set_eq(state: State<'_, AppState>, player: u8, band: String, gain_db: f32) -> Result<(), String> {
    let engine_slot = state.audio_engine.lock().await;
    if let Some(engine) = engine_slot.as_ref() {
        let band_id = match band.as_str() {
            "low" => crate::audio::EqBand::Low,
            "mid" => crate::audio::EqBand::Mid,
            _ => crate::audio::EqBand::High,
        };
        let frame = engine.current_frame();
        engine.send_command(crate::audio::EngineCommand::SetEqGain {
            player: crate::audio::PlayerId(player),
            at_frame: frame,
            band: band_id,
            gain_db,
        });
    }
    Ok(())
}

#[command]
pub async fn audio_engine_set_eq_kill(state: State<'_, AppState>, player: u8, band: String, killed: bool) -> Result<(), String> {
    let engine_slot = state.audio_engine.lock().await;
    if let Some(engine) = engine_slot.as_ref() {
        let band_id = match band.as_str() {
            "low" => crate::audio::EqBand::Low,
            "mid" => crate::audio::EqBand::Mid,
            _ => crate::audio::EqBand::High,
        };
        let frame = engine.current_frame();
        engine.send_command(crate::audio::EngineCommand::SetEqKill {
            player: crate::audio::PlayerId(player),
            at_frame: frame,
            band: band_id,
            killed,
        });
    }
    Ok(())
}

#[command]
pub async fn audio_engine_set_loop(state: State<'_, AppState>, player: u8, start_beat: Option<f64>, length_beats: Option<f64>) -> Result<(), String> {
    let engine_slot = state.audio_engine.lock().await;
    if let Some(engine) = engine_slot.as_ref() {
        let loop_region = match (start_beat, length_beats) {
            (Some(start), Some(len)) => Some(crate::audio::LoopRegion { start_beat: start, length_beats: len }),
            _ => None,
        };
        let frame = engine.current_frame();
        engine.send_command(crate::audio::EngineCommand::SetLoop {
            player: crate::audio::PlayerId(player),
            at_frame: frame,
            loop_region,
        });
    }
    Ok(())
}

#[command]
pub async fn audio_engine_load_player(state: State<'_, AppState>, player: u8, file_path: String) -> Result<(), String> {
    let mut engine_slot = state.audio_engine.lock().await;
    let engine = engine_slot.as_mut().ok_or("Audio engine not initialized")?;
    let target_sr = engine.sample_rate();

    // Decode on a background thread (not the audio callback)
    let buffer = tokio::task::spawn_blocking(move || {
        crate::audio::worker::decode_file(&file_path, target_sr)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())??;

    // Register the source and launch the player. The Launch command carries
    // an Arc clone of the buffer; the callback loads it into the player.
    let source = engine.register_source(buffer);
    engine.launch_player(
        crate::audio::PlayerId(player),
        source,
        0.0,
        crate::audio::Quantize::Immediate,
    )?;
    Ok(())
}

#[command]
pub async fn audio_engine_set_master_gain(state: State<'_, AppState>, gain: f32) -> Result<(), String> {
    let engine_slot = state.audio_engine.lock().await;
    if let Some(engine) = engine_slot.as_ref() {
        let frame = engine.current_frame();
        engine.send_command(crate::audio::EngineCommand::SetMasterGain { at_frame: frame, gain });
    }
    Ok(())
}

#[command]
pub async fn audio_engine_sync_launch(
    state: State<'_, AppState>,
    player_a: u8,
    player_b: u8,
) -> Result<(), String> {
    let engine_slot = state.audio_engine.lock().await;
    if let Some(engine) = engine_slot.as_ref() {
        // Schedule both launches at the same future frame. Use a small
        // lookahead (e.g., 1024 frames ≈ 21ms at 48k) to ensure both
        // commands arrive before the target frame.
        let frame = engine.current_frame();
        let target_frame = frame + 1024;
        engine.send_command(crate::audio::EngineCommand::Resume {
            player: crate::audio::PlayerId(player_a),
            at_frame: target_frame,
        });
        engine.send_command(crate::audio::EngineCommand::Resume {
            player: crate::audio::PlayerId(player_b),
            at_frame: target_frame,
        });
    }
    Ok(())
}

/// Get the current git revision (short SHA) for the running build.
/// Used by the Listening Lab to record which DSP revision produced
/// each human rating. Falls back to the Cargo package version if git
/// is not available.
#[command]
pub async fn get_git_revision() -> Result<String, String> {
    // Try to get the git SHA from the build-time environment variable.
    // This is set by a build script if present, or falls back to the
    // Cargo package version.
    let sha = option_env!("TUNELOCK_GIT_SHA").unwrap_or(env!("CARGO_PKG_VERSION"));
    Ok(sha.to_string())
}

/// Seek a player to a position in seconds (not beats). Used by the
/// Listening Lab's ABX cue positioning, where the cue is denominated
/// in seconds rather than beats.
#[command]
pub async fn audio_engine_seek_source_seconds(
    state: State<'_, AppState>,
    player: u8,
    source_seconds: f64,
) -> Result<(), String> {
    let engine_slot = state.audio_engine.lock().await;
    if let Some(engine) = engine_slot.as_ref() {
        let frame = engine.current_frame();
        engine.send_command(crate::audio::EngineCommand::SeekSourceSeconds {
            player: crate::audio::PlayerId(player),
            at_frame: frame,
            source_seconds,
        });
    }
    Ok(())
}

/// Beat Sync: tempo-match player B to player A's effective BPM and
/// align their nearest beat-grid beats. Both players start playing.
#[command]
pub async fn audio_engine_beat_sync(
    state: State<'_, AppState>,
    player_a: u8,
    player_b: u8,
) -> Result<(), String> {
    let engine_slot = state.audio_engine.lock().await;
    if let Some(engine) = engine_slot.as_ref() {
        let frame = engine.current_frame();
        engine.send_command(crate::audio::EngineCommand::BeatSync {
            player_a: crate::audio::PlayerId(player_a),
            player_b: crate::audio::PlayerId(player_b),
            at_frame: frame,
        });
    }
    Ok(())
}

/// Bar Sync: tempo-match player B to player A and align downbeat/bar
/// boundaries. Both players start playing.
#[command]
pub async fn audio_engine_bar_sync(
    state: State<'_, AppState>,
    player_a: u8,
    player_b: u8,
) -> Result<(), String> {
    let engine_slot = state.audio_engine.lock().await;
    if let Some(engine) = engine_slot.as_ref() {
        let frame = engine.current_frame();
        engine.send_command(crate::audio::EngineCommand::BarSync {
            player_a: crate::audio::PlayerId(player_a),
            player_b: crate::audio::PlayerId(player_b),
            at_frame: frame,
        });
    }
    Ok(())
}

#[command]
pub async fn audio_engine_set_listening_condition(
    state: State<'_, AppState>,
    player: u8,
    processor_type: String,
    tempo_rate: f32,
    pitch_semitones: f32,
) -> Result<(), String> {
    let pt = match processor_type.as_str() {
        "bypass" => crate::audio::command::ProcessorType::Bypass,
        "varispeed" => crate::audio::command::ProcessorType::Varispeed,
        "signalsmith" => crate::audio::command::ProcessorType::Signalsmith,
        other => return Err(format!("Unknown processor type: {}", other)),
    };
    let engine_slot = state.audio_engine.lock().await;
    if let Some(engine) = engine_slot.as_ref() {
        let frame = engine.current_frame();
        engine.send_command(crate::audio::EngineCommand::SetListeningCondition {
            player: crate::audio::PlayerId(player),
            at_frame: frame,
            processor_type: pt,
            tempo_rate,
            pitch_semitones,
        });
    }
    Ok(())
}

/// Load a player with a source but do NOT start playback. The player
/// is loaded in a paused state, ready for explicit launch via play or
/// sync_launch. Used by the Listening Lab so load doesn't auto-play.
///
/// This is the Musical Time Bridge: when loading a file, the command
/// looks up the track in the TuneLock database by path, fetches any
/// existing beat grid analysis, and attaches it to the DecodedBuffer.
/// If no beat grid exists, it runs the existing beat-grid detector
/// on a background thread and attaches the result. This wires Core
/// Intelligence's rhythmic data into the Performance Engine without
/// reimplementing analysis.
#[command]
pub async fn audio_engine_load_player_paused(
    state: State<'_, AppState>,
    player: u8,
    file_path: String,
) -> Result<(), String> {
    // Phase 1: Get the target sample rate from the engine.
    let target_sr = {
        let engine_slot = state.audio_engine.lock().await;
        let engine = engine_slot.as_ref().ok_or("Audio engine not initialized")?;
        engine.sample_rate()
    };

    // Phase 2: Look up the track in the DB by path and fetch its beat grid.
    // Clone the DB Arc so we can use it in spawn_blocking.
    let db_arc = state.db.clone();
    let file_path_for_db = file_path.clone();
    let beat_grid_info: Option<(f64, f64, i32, usize, Option<f64>)> = {
        let db = db_arc.lock().await;
        // Try to find the track by path.
        if let Ok(Some(track)) = db.get_track_by_path(&file_path_for_db) {
            // Track found — try to get its beat grid.
            if let Ok(Some(bg)) = db.get_beat_grid(track.id) {
                Some((
                    bg.bpm,
                    bg.first_beat_ms as f64 / 1000.0,
                    bg.meter_numerator,
                    bg.downbeat_offset_beats as usize,
                    bg.confidence,
                ))
            } else if let Some(track_bpm) = track.bpm {
                // No beat grid, but the track has a BPM from analysis.
                // Create a minimal beat grid with default meter.
                Some((track_bpm, 0.0, 4, 0, None))
            } else {
                None
            }
        } else {
            None
        }
    };

    // Phase 3: Decode the audio file on a background thread.
    let file_path_for_decode = file_path.clone();
    let mut buffer = tokio::task::spawn_blocking(move || {
        crate::audio::worker::decode_file(&file_path_for_decode, target_sr)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Decode task failed: {}", e))??;

    // Phase 4: If we have beat grid info from the DB, attach it.
    // If not, try to run the beat-grid detector on the decoded audio.
    if let Some((bpm, first_beat_sec, meter_numerator, downbeat_offset, _confidence)) = beat_grid_info {
        buffer.bpm = Some(bpm);
        buffer.beat_grid = Some(crate::audio::command::BeatGridCompact {
            bpm,
            first_beat_sec,
            meter_numerator,
            downbeat_offset,
        });
    } else {
        // No existing analysis — run the beat-grid detector on the
        // decoded audio. This uses the EXISTING analysis pipeline,
        // not a new one. We decode to mono at the analysis sample rate.
        let file_path_for_analysis = file_path.clone();
        let detected = tokio::task::spawn_blocking(move || -> Result<Option<crate::analysis::beat_grid::BeatGridResult>, String> {
            let samples = crate::media::decode_media(&file_path_for_analysis).map_err(|e| e.to_string())?;
            crate::analysis::beat_grid::detect_beat_grid(&samples).map(Some).map_err(|e| e)
        })
        .await
        .map_err(|e| format!("Beat grid detection task failed: {}", e))?;

        if let Ok(Some(grid)) = detected {
            buffer.bpm = Some(grid.bpm);
            buffer.beat_grid = Some(crate::audio::command::BeatGridCompact {
                bpm: grid.bpm,
                first_beat_sec: grid.first_beat_sec,
                meter_numerator: grid.meter_numerator,
                downbeat_offset: grid.downbeat_offset,
            });
        }
        // If detection failed, buffer.bpm and beat_grid stay None.
        // The player will fall back to 120 BPM as before.
    }

    // Phase 5: Register the source and launch (paused).
    let mut engine_slot = state.audio_engine.lock().await;
    let engine = engine_slot.as_mut().ok_or("Audio engine not initialized")?;
    let source = engine.register_source(buffer);
    engine.launch_player(
        crate::audio::PlayerId(player),
        source,
        0.0,
        crate::audio::Quantize::Immediate,
    )?;
    // Immediately pause to stop playback that launch_player just started.
    let frame = engine.current_frame();
    engine.send_command(crate::audio::EngineCommand::Pause {
        player: crate::audio::PlayerId(player),
        at_frame: frame,
    });
    Ok(())
}

#[command]
pub async fn audio_engine_set_processor_type(
    state: State<'_, AppState>,
    player: u8,
    processor_type: String,
) -> Result<(), String> {
    let pt = match processor_type.as_str() {
        "bypass" => crate::audio::command::ProcessorType::Bypass,
        "varispeed" => crate::audio::command::ProcessorType::Varispeed,
        "signalsmith" => crate::audio::command::ProcessorType::Signalsmith,
        other => return Err(format!("Unknown processor type: {}", other)),
    };
    let engine_slot = state.audio_engine.lock().await;
    if let Some(engine) = engine_slot.as_ref() {
        let frame = engine.current_frame();
        engine.send_command(crate::audio::EngineCommand::SetProcessorType {
            player: crate::audio::PlayerId(player),
            at_frame: frame,
            processor_type: pt,
        });
    }
    Ok(())
}

#[command]
pub async fn audio_engine_set_bus_gain(state: State<'_, AppState>, bus: String, gain: f32) -> Result<(), String> {
    let engine_slot = state.audio_engine.lock().await;
    if let Some(engine) = engine_slot.as_ref() {
        let bus_id = match bus.as_str() {
            "a" | "A" => crate::audio::BusId::A,
            "b" | "B" => crate::audio::BusId::B,
            _ => crate::audio::BusId::Master,
        };
        let frame = engine.current_frame();
        engine.send_command(crate::audio::EngineCommand::SetBusGain {
            bus: bus_id,
            at_frame: frame,
            gain,
        });
    }
    Ok(())
}

fn parse_filter_bus(bus: &str) -> crate::audio::BusId {
    match bus {
        "a" | "A" => crate::audio::BusId::A,
        "b" | "B" => crate::audio::BusId::B,
        _ => crate::audio::BusId::Master,
    }
}

#[command]
pub async fn audio_engine_set_filter_mode(state: State<'_, AppState>, bus: String, mode: String) -> Result<(), String> {
    let engine_slot = state.audio_engine.lock().await;
    if let Some(engine) = engine_slot.as_ref() {
        let mode_param = match mode.as_str() {
            "lp" | "lowpass" | "low" => crate::audio::FilterModeParam::Lowpass,
            "bp" | "bandpass" | "band" => crate::audio::FilterModeParam::Bandpass,
            "hp" | "highpass" | "high" => crate::audio::FilterModeParam::Highpass,
            _ => crate::audio::FilterModeParam::Bypass,
        };
        let frame = engine.current_frame();
        engine.send_command(crate::audio::EngineCommand::SetFilterMode {
            bus: parse_filter_bus(&bus),
            at_frame: frame,
            mode: mode_param,
        });
    }
    Ok(())
}

#[command]
pub async fn audio_engine_set_filter_cutoff(state: State<'_, AppState>, bus: String, hz: f32) -> Result<(), String> {
    let engine_slot = state.audio_engine.lock().await;
    if let Some(engine) = engine_slot.as_ref() {
        let frame = engine.current_frame();
        engine.send_command(crate::audio::EngineCommand::SetFilterCutoff {
            bus: parse_filter_bus(&bus),
            at_frame: frame,
            hz,
        });
    }
    Ok(())
}

#[command]
pub async fn audio_engine_set_filter_resonance(state: State<'_, AppState>, bus: String, resonance: f32) -> Result<(), String> {
    let engine_slot = state.audio_engine.lock().await;
    if let Some(engine) = engine_slot.as_ref() {
        let frame = engine.current_frame();
        engine.send_command(crate::audio::EngineCommand::SetFilterResonance {
            bus: parse_filter_bus(&bus),
            at_frame: frame,
            resonance,
        });
    }
    Ok(())
}

#[command]
pub async fn audio_engine_set_filter_drive(state: State<'_, AppState>, bus: String, drive: f32) -> Result<(), String> {
    let engine_slot = state.audio_engine.lock().await;
    if let Some(engine) = engine_slot.as_ref() {
        let frame = engine.current_frame();
        engine.send_command(crate::audio::EngineCommand::SetFilterDrive {
            bus: parse_filter_bus(&bus),
            at_frame: frame,
            drive,
        });
    }
    Ok(())
}

/// Meter readout for the UI. Uses the generalized player/bus vocabulary.
/// Players 0 and 1 correspond to the Transition Workbench's A and B decks.
#[derive(serde::Serialize)]
pub struct AudioMeterReadout {
    pub playing: bool,
    pub current_frame: u64,
    pub players: [PlayerMeterEntry; 8],
    pub bus_a_rms: f64,
    pub bus_a_peak: f64,
    pub bus_b_rms: f64,
    pub bus_b_peak: f64,
    pub master_rms: f64,
    pub master_peak: f64,
    /// PROVISIONAL: sample-peak, not true-peak. Renamed to prevent UI from
    /// presenting this as a true-peak measurement. True-peak arrives with PB-6.
    pub master_sample_peak_provisional: f64,
    pub master_clip: bool,
    pub crossfade_position: f64,
    pub underruns: u64,
    pub commands_dropped: u64,
}

#[derive(serde::Serialize, Default)]
pub struct PlayerMeterEntry {
    pub playing: bool,
    pub position_sec: f64,
    pub rms: f64,
    pub peak: f64,
    pub clip: bool,
    // Musical telemetry (PB-2.3 Musical Time Bridge)
    pub source_bpm: f64,
    pub effective_bpm: f64,
    pub tempo_ratio: f64,
    pub pitch_semitones: f64,
    pub beat_position: f64,
    pub bar_position: f64,
    pub meter_numerator: i32,
    pub processor_mode: i32,
}

#[command]
pub async fn audio_engine_get_meters(state: State<'_, AppState>) -> Result<AudioMeterReadout, String> {
    let engine_slot = state.audio_engine.lock().await;
    let engine = engine_slot.as_ref().ok_or("Audio engine not initialized")?;

    // Drain retired sources on every meter poll (~30Hz). This guarantees
    // that old PCM buffers are destroyed on the Tauri async thread, not
    // on the realtime audio callback. The meter poll is the ideal place
    // because the UI calls it regularly during playback.
    engine.drain_retired_sources();

    let m = engine.get_meters();
    let players = m.players.iter().map(|p| PlayerMeterEntry {
        playing: p.playing,
        position_sec: p.position_sec,
        rms: p.rms,
        peak: p.peak,
        clip: p.clip,
        source_bpm: p.source_bpm,
        effective_bpm: p.effective_bpm,
        tempo_ratio: p.tempo_ratio,
        pitch_semitones: p.pitch_semitones,
        beat_position: p.beat_position,
        bar_position: p.bar_position,
        meter_numerator: p.meter_numerator,
        processor_mode: p.processor_mode,
    }).collect::<Vec<_>>().try_into().unwrap_or_default();
    Ok(AudioMeterReadout {
        playing: m.playing,
        current_frame: m.current_frame,
        players,
        bus_a_rms: m.bus_a_rms,
        bus_a_peak: m.bus_a_peak,
        bus_b_rms: m.bus_b_rms,
        bus_b_peak: m.bus_b_peak,
        master_rms: m.master_rms,
        master_peak: m.master_peak,
        master_sample_peak_provisional: m.master_sample_peak_provisional,
        master_clip: m.master_clip,
        crossfade_position: m.crossfade_position,
        underruns: m.underruns,
        commands_dropped: m.commands_dropped,
    })
}

// ============================================================================
// Beat-grid DSP commands
// ============================================================================

#[derive(serde::Serialize)]
pub struct BeatMarkerResult {
    pub source_frame: u64,
    pub beat_number: u32,
    pub is_downbeat: bool,
    pub confidence: f64,
}

#[derive(serde::Serialize)]
pub struct TempoSegmentResult {
    pub start_source_frame: u64,
    pub start_beat: f64,
    pub bpm: f64,
}

#[derive(serde::Serialize)]
pub struct BeatGridDetectionResult {
    pub bpm: f64,
    pub first_beat_ms: i64,
    pub beat_times_ms: Vec<i64>,
    pub beat_markers: Vec<BeatMarkerResult>,
    pub downbeat_offset: usize,
    pub downbeat_confidence: f64,
    pub meter_numerator: i32,
    pub confidence: f64,
    pub tempo_segments: Vec<TempoSegmentResult>,
}

#[command]
pub async fn detect_beat_grid(state: State<'_, AppState>, track_id: i64) -> Result<BeatGridDetectionResult, String> {
    // Get the file path from the database
    let file_path = {
        let db = state.db.lock().await;
        db.get_track_by_id(track_id)
            .map_err(|e| e.to_string())?
            .ok_or("Track not found")?
            .file_path
    };

    // Decode and detect on a background thread
    let result = tokio::task::spawn_blocking(move || -> Result<BeatGridDetectionResult, String> {
        let samples = crate::media::decode_media(&file_path).map_err(|e| e.to_string())?;
        let grid = crate::analysis::beat_grid::detect_beat_grid(&samples).map_err(|e| e)?;
        Ok(BeatGridDetectionResult {
            bpm: grid.bpm,
            first_beat_ms: (grid.first_beat_sec * 1000.0) as i64,
            beat_times_ms: grid.beat_times.iter().map(|t| (t * 1000.0) as i64).collect(),
            beat_markers: grid.beat_markers.iter().map(|m| BeatMarkerResult {
                source_frame: m.source_frame,
                beat_number: m.beat_number,
                is_downbeat: m.is_downbeat,
                confidence: m.confidence,
            }).collect(),
            downbeat_offset: grid.downbeat_offset,
            downbeat_confidence: grid.downbeat_confidence,
            meter_numerator: grid.meter_numerator,
            confidence: grid.confidence,
            tempo_segments: grid.tempo_segments.iter().map(|s| TempoSegmentResult {
                start_source_frame: s.start_source_frame,
                start_beat: s.start_beat,
                bpm: s.bpm,
            }).collect(),
        })
    })
    .await
    .map_err(|e| e.to_string())??;

    // Save to database
    let db = state.db.lock().await;
    db.save_beat_grid(&crate::models::BeatGrid {
        track_id,
        source: "engine".to_string(),
        bpm: result.bpm,
        first_beat_ms: result.first_beat_ms,
        meter_numerator: result.meter_numerator,
        downbeat_offset_beats: result.downbeat_offset as i32,
        confidence: Some(result.confidence),
        is_override: false,
    })
    .map_err(|e| e.to_string())?;

    Ok(result)
}

// ── PB-3: Professional audio I/O ─────────────────────────────────────

/// A description of one available audio output device, for the UI.
#[derive(serde::Serialize)]
pub struct AudioDeviceEntry {
    pub name: String,
    pub sample_rates: Vec<u32>,
    pub default_sample_rate: u32,
    pub default_channels: u16,
    pub is_default: bool,
}

/// The list of available audio output devices.
#[derive(serde::Serialize)]
pub struct AudioDeviceListResponse {
    pub devices: Vec<AudioDeviceEntry>,
    pub default_device_index: Option<usize>,
}

#[command]
pub async fn audio_enumerate_devices() -> Result<AudioDeviceListResponse, String> {
    let list = crate::audio::io::enumerate_output_devices()?;
    let devices = list
        .devices
        .into_iter()
        .map(|d| {
            let sample_rates = d.supported_sample_rates();
            AudioDeviceEntry {
                name: d.name,
                sample_rates,
                default_sample_rate: d.default_sample_rate,
                default_channels: d.default_channels,
                is_default: d.is_default,
            }
        })
        .collect();
    Ok(AudioDeviceListResponse {
        devices,
        default_device_index: list.default_device_index,
    })
}

#[command]
pub async fn audio_engine_set_device(
    state: State<'_, AppState>,
    device_name: Option<String>,
    sample_rate: Option<u32>,
    buffer_size: Option<u32>,
) -> Result<u32, String> {
    // Rebuild the engine with the new config. This requires stopping the
    // current stream and creating a new one. The source registry is lost;
    // the UI must re-launch sources after a device change.
    //
    // PB-3 MVP: output is always stereo. Channel count is NOT selectable.
    // Multi-output routing (Master 1/2, Cue 3/4) is a future phase.
    let config = crate::audio::io::AudioDeviceConfig {
        device_name,
        sample_rate,
        buffer_size: buffer_size.map(crate::audio::io::BufferSizePreference::Fixed),
    };

    let new_engine = crate::audio::engine::AudioEngine::new_with_config(&config)?;
    let new_sr = new_engine.sample_rate();

    let mut engine_slot = state.audio_engine.lock().await;
    // Drop the old engine (stops the old stream)
    *engine_slot = Some(new_engine);
    // Start the new stream
    if let Some(engine) = engine_slot.as_ref() {
        engine.start().map_err(|e| format!("Failed to start stream: {}", e))?;
    }

    Ok(new_sr)
}

// ── PB-2 Listening Lab ───────────────────────────────────────────────
//
// Developer-only tool for human validation of the time/pitch processor.
// Uses the production Performance Engine and SignalsmithProcessor.
// Results are persisted locally as JSON for comparison across DSP revisions.

/// Processor information for the Listening Lab display.
#[derive(serde::Serialize)]
pub struct ListeningLabProcessorInfo {
    /// "signalsmith" or "varispeed"
    pub processor_type: String,
    /// Algorithmic latency in frames
    pub latency_frames: usize,
    /// Engine sample rate
    pub sample_rate: u32,
}

/// A saved listening lab result.
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct ListeningLabResult {
    pub id: Option<i64>,
    pub timestamp: String,
    pub processor: String,
    pub tempo_percent: f64,
    pub pitch_semitones: f64,
    pub material: String,
    pub track_name: Option<String>,
    pub transients: u8,
    pub bass: u8,
    pub vocals: u8,
    pub stereo: u8,
    pub artifacts: u8,
    pub overall: u8,
    pub abx_correct: Option<u32>,
    pub abx_trials: Option<u32>,
    pub notes: Option<String>,
    /// Git revision at the time of the test (for DSP revision comparison).
    pub git_revision: Option<String>,
}

#[command]
pub async fn listening_lab_get_processor_info(
    state: State<'_, AppState>,
) -> Result<ListeningLabProcessorInfo, String> {
    let engine_slot = state.audio_engine.lock().await;
    let engine = engine_slot.as_ref().ok_or("Audio engine not initialized")?;
    let sr = engine.sample_rate();
    // Create a temporary Signalsmith instance to measure its latency.
    // This is the real algorithmic latency (input_latency + output_latency).
    let proc = crate::audio::timepitch::default_processor(sr as f64, 2);
    let latency = proc.latency_frames();
    Ok(ListeningLabProcessorInfo {
        processor_type: "signalsmith".to_string(),
        latency_frames: latency,
        sample_rate: sr,
    })
}

#[command]
pub async fn listening_lab_save_result(
    state: State<'_, AppState>,
    result: ListeningLabResult,
) -> Result<i64, String> {
    let db = state.db.lock().await;
    db.save_listening_lab_result(
        &result.timestamp,
        &result.processor,
        result.tempo_percent,
        result.pitch_semitones,
        &result.material,
        result.track_name.as_deref(),
        result.transients,
        result.bass,
        result.vocals,
        result.stereo,
        result.artifacts,
        result.overall,
        result.abx_correct,
        result.abx_trials,
        result.notes.as_deref(),
        result.git_revision.as_deref(),
    )
    .map_err(|e| format!("Failed to save listening lab result: {}", e))
}

#[command]
pub async fn listening_lab_get_results(
    state: State<'_, AppState>,
) -> Result<Vec<ListeningLabResult>, String> {
    let db = state.db.lock().await;
    db.get_listening_lab_results()
        .map_err(|e| format!("Failed to query listening lab results: {}", e))
}
