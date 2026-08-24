//! TuneLock benchmark / accuracy harness.
//!
//! Two modes:
//!
//!   1. Scored corpus mode (the Proof layer):
//!      cargo run --release --bin tunelock-bench -- ^
//!          --corpus ..\ground-truth\MIKCompleteLibrary.csv ^
//!          [--limit 500] [--out report.json]
//!
//!      Scores key + BPM predictions against ground truth with MIREX-weighted
//!      accuracy, an error-type confusion matrix, and per-genre / per-format
//!      breakdowns.
//!
//!   2. Legacy folder mode (pretty-printed diagnostics for a folder of audio):
//!      cargo run --release --bin tunelock-bench -- <folder_path>

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use rayon::prelude::*;
use tunelock_lib::media::decode_media;
use tunelock_lib::analysis::ensemble::ProfileWeights;
use tunelock_lib::analysis::key_detector::detect_key_diagnostic;
use tunelock_lib::analysis::tempo_detector::detect_tempo;
use tunelock_lib::analysis::{key_to_camelot, pitch_class_to_name};
use tunelock_lib::proof::corpus::{
    load_giantsteps, load_mik_corpus, normalize_genre, parse_standard_key,
    stratified_sample_seeded, CorpusRow, RowStatus,
};
use tunelock_lib::proof::metrics::{bpm_ratio, classify_error, is_camelot_compatible, mirex_score};

// ============================================================================
// CLI
// ============================================================================

struct Args {
    corpus: Option<String>,
    giantsteps: Option<String>,
    key_manifest: Option<String>,
    role: Option<String>,
    audio_root: Option<String>,
    limit: Option<usize>,
    out: Option<String>,
    folder: Option<String>,
    manifest: Option<String>,
    seed: Option<u64>,
    // Ablation flags
    no_hpss: bool,
    chroma12_only: bool,
    chroma72_only: bool,
    hpss_kernel: Option<usize>,
    analysis_seconds: Option<usize>,
    edm_braw: bool,
    edms_bgate: bool,
    edm_braw_plain: bool,
    edm_bgate_plain: bool,
}

fn parse_args() -> Args {
    let mut a = Args {
        corpus: None, giantsteps: None, key_manifest: None, role: None,
        audio_root: None, limit: None, out: None,
        folder: None, manifest: None, seed: None,
        no_hpss: false, chroma12_only: false, chroma72_only: false,
        hpss_kernel: None, analysis_seconds: None,
        edm_braw: false, edms_bgate: false,
        edm_braw_plain: false, edm_bgate_plain: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--corpus" => a.corpus = it.next(),
            "--giantsteps" => a.giantsteps = it.next(),
            "--key-manifest" => a.key_manifest = it.next(),
            "--role" => a.role = it.next(),
            "--audio-root" => a.audio_root = it.next(),
            "--limit" => a.limit = it.next().and_then(|s| s.parse().ok()),
            "--out" => a.out = it.next(),
            "--manifest" => a.manifest = it.next(),
            "--seed" => a.seed = it.next().and_then(|s| s.parse().ok()),
            "--no-hpss" => a.no_hpss = true,
            "--chroma12-only" => a.chroma12_only = true,
            "--chroma72-only" => a.chroma72_only = true,
            "--hpss-kernel" => a.hpss_kernel = it.next().and_then(|s| s.parse().ok()),
            "--analysis-seconds" => a.analysis_seconds = it.next().and_then(|s| s.parse().ok()),
            "--edm-braw" => a.edm_braw = true,
            "--edm-bgate" => a.edms_bgate = true,
            "--edm-braw-plain" => a.edm_braw_plain = true,
            "--edm-bgate-plain" => a.edm_bgate_plain = true,
            "--help" | "-h" => {
                eprintln!("Usage:");
                eprintln!("  tunelock-bench --corpus <csv> [--limit N] [--seed S] [--manifest m.json] [--out report.json]");
                eprintln!("  tunelock-bench --giantsteps <dataset_root> [--limit N] [--out report.json]");
                eprintln!("  tunelock-bench --key-manifest <json> --role <training|development> [--audio-root <dir>] [--out report.json]");
                eprintln!("  tunelock-bench <folder>            (legacy diagnostic mode)");
                eprintln!("");
                eprintln!("  Ablation flags:");
                eprintln!("    --no-hpss               Skip HPSS, use raw spectrogram");
                eprintln!("    --chroma12-only          Use only 12-bin path (Krumhansl+Temperley+Sha'ath-12)");
                eprintln!("    --chroma72-only          Use only 72-band path (Sha'ath-72)");
                eprintln!("    --hpss-kernel N          HPSS kernel size (default 9)");
                eprintln!("    --analysis-seconds N     Analysis window in seconds (default 180)");
                eprintln!("    --edm-braw               Use HPCP + Faraldo braw profiles (EDM path)");
                eprintln!("    --edm-bgate              Use HPCP + Faraldo bgate profiles (EDM path)");
                std::process::exit(0);
            }
            other if !other.starts_with("--") => a.folder = Some(other.to_string()),
            other => eprintln!("Unknown argument: {}", other),
        }
    }
    a
}

fn main() {
    let args = parse_args();

    // Build ablation config from flags
    let edm_profile = if args.edm_braw {
        Some(tunelock_lib::analysis::ensemble::EdmProfile::Braw)
    } else if args.edms_bgate {
        Some(tunelock_lib::analysis::ensemble::EdmProfile::Bgate)
    } else {
        None
    };
    let ablation = tunelock_lib::analysis::key_detector::AblationConfig {
        no_hpss: args.no_hpss,
        hpss_kernel: args.hpss_kernel.unwrap_or(tunelock_lib::analysis::HPSS_KERNEL),
        chroma12_only: args.chroma12_only,
        chroma72_only: args.chroma72_only,
        analysis_seconds: args.analysis_seconds,
        edm_profile,
    };

    // Build plain-chroma EDM option (braw/bgate profiles with plain chroma
    // instead of HPCP, to isolate profile vs representation effects)
    let edm_plain = if args.edm_braw_plain {
        Some(tunelock_lib::analysis::ensemble::EdmProfile::Braw)
    } else if args.edm_bgate_plain {
        Some(tunelock_lib::analysis::ensemble::EdmProfile::Bgate)
    } else {
        None
    };

    // Print ablation mode if any non-default settings
    let is_ablation = args.no_hpss || args.chroma12_only || args.chroma72_only
        || args.hpss_kernel.is_some() || args.analysis_seconds.is_some()
        || args.edm_braw || args.edms_bgate || args.edm_braw_plain || args.edm_bgate_plain;
    if is_ablation {
        eprintln!("Ablation mode:");
        if args.no_hpss { eprintln!("  no HPSS (raw spectrogram)"); }
        if args.chroma12_only { eprintln!("  12-bin only (Krumhansl+Temperley+Sha'ath-12)"); }
        if args.chroma72_only { eprintln!("  72-band only (Sha'ath-72)"); }
        if let Some(k) = args.hpss_kernel { eprintln!("  HPSS kernel: {}", k); }
        if let Some(s) = args.analysis_seconds { eprintln!("  Analysis window: {}s", s); }
        if args.edm_braw { eprintln!("  HPCP + braw (EDM path)"); }
        if args.edms_bgate { eprintln!("  HPCP + bgate (EDM path)"); }
        if args.edm_braw_plain { eprintln!("  plain chroma + braw (EDM profiles, no HPCP)"); }
        if args.edm_bgate_plain { eprintln!("  plain chroma + bgate (EDM profiles, no HPCP)"); }
    }

    if let Some(key_manifest) = args.key_manifest {
        let role = args.role.as_deref().unwrap_or("training");
        let audio_root = args.audio_root.as_deref().unwrap_or(".");
        run_key_manifest(
            &key_manifest,
            role,
            audio_root,
            args.limit,
            args.out.as_deref(),
            &ablation,
            edm_plain,
        );
    } else if let Some(corpus) = args.corpus {
        run_scored_corpus(&corpus, args.limit, args.out.as_deref(), args.manifest.as_deref(), args.seed, &ablation, edm_plain);
    } else if let Some(gs) = args.giantsteps {
        run_giantsteps(&gs, args.limit, args.out.as_deref(), &ablation, edm_plain);
    } else if let Some(folder) = args.folder {
        legacy_folder_mode(&folder);
    } else {
        eprintln!("Usage:");
        eprintln!("  tunelock-bench --corpus <csv> [--limit N] [--seed S] [--manifest m.json] [--out report.json]");
        eprintln!("  tunelock-bench --giantsteps <dataset_root> [--limit N] [--out report.json]");
        eprintln!("  tunelock-bench --key-manifest <json> --role <training|development> [--audio-root <dir>] [--out report.json]");
        eprintln!("  tunelock-bench <folder>            (legacy diagnostic mode)");
        std::process::exit(1);
    }
}

#[derive(serde::Deserialize)]
struct KeyCorpusManifest {
    canonical_labels: Vec<String>,
    records: Vec<KeyManifestRecord>,
}

#[derive(serde::Deserialize)]
struct KeyManifestRecord {
    role: String,
    id: String,
    audio_path: String,
    truth_index: usize,
    truth_label: String,
    artist: String,
    genre: String,
}

fn run_key_manifest(
    manifest_path: &str,
    role: &str,
    audio_root: &str,
    limit: Option<usize>,
    out: Option<&str>,
    ablation: &tunelock_lib::analysis::key_detector::AblationConfig,
    edm_plain: Option<tunelock_lib::analysis::ensemble::EdmProfile>,
) {
    let rows = match load_key_manifest_rows(
        Path::new(manifest_path),
        role,
        Path::new(audio_root),
    ) {
        Ok(rows) => rows,
        Err(error) => {
            eprintln!("Failed to load key manifest: {error:#}");
            std::process::exit(1);
        }
    };
    if rows.is_empty() {
        eprintln!("No records with role {role:?} found in {manifest_path}");
        std::process::exit(1);
    }
    score_rows(
        rows,
        manifest_path,
        limit,
        out,
        None,
        None,
        ablation,
        edm_plain,
    );
}

fn load_key_manifest_rows(
    manifest_path: &Path,
    role: &str,
    audio_root: &Path,
) -> anyhow::Result<Vec<CorpusRow>> {
    if !matches!(role, "training" | "development") {
        anyhow::bail!("role must be training or development");
    }
    let bytes = std::fs::read(manifest_path)?;
    let manifest: KeyCorpusManifest = serde_json::from_slice(&bytes)?;
    if manifest.canonical_labels.len() != 24 {
        anyhow::bail!("key manifest must contain exactly 24 canonical labels");
    }

    let mut rows = Vec::new();
    for record in manifest.records.into_iter().filter(|record| record.role == role) {
        let canonical = manifest
            .canonical_labels
            .get(record.truth_index)
            .ok_or_else(|| anyhow::anyhow!("truth index out of range for {}", record.id))?;
        if canonical != &record.truth_label {
            anyhow::bail!(
                "truth label/index mismatch for {}: {:?} vs {:?}",
                record.id,
                record.truth_label,
                canonical
            );
        }
        let (tonic, is_major) = parse_standard_key(canonical)
            .ok_or_else(|| anyhow::anyhow!("invalid canonical key label {canonical:?}"))?;
        let source_path = PathBuf::from(&record.audio_path);
        let resolved_path = if source_path.is_absolute() {
            source_path
        } else {
            audio_root.join(source_path)
        };
        let extension = resolved_path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let status = if resolved_path.is_file() {
            RowStatus::Ready
        } else {
            RowStatus::MissingFile
        };
        rows.push(CorpusRow {
            title: record.id,
            artist: record.artist,
            key_camelot: Some(key_to_camelot(tonic, is_major)),
            truth_tonic: Some(tonic),
            truth_is_major: Some(is_major),
            truth_bpm: None,
            truth_energy: None,
            genre: record.genre,
            location: resolved_path.to_string_lossy().into_owned(),
            extension,
            status,
        });
    }
    Ok(rows)
}

fn run_giantsteps(root: &str, limit: Option<usize>, out: Option<&str>, ablation: &tunelock_lib::analysis::key_detector::AblationConfig, edm_plain: Option<tunelock_lib::analysis::ensemble::EdmProfile>) {
    let rows = load_giantsteps(Path::new(root));
    if rows.is_empty() {
        eprintln!("No GiantSteps annotations found under {}", root);
        std::process::exit(1);
    }
    score_rows(rows, root, limit, out, None, None, ablation, edm_plain);
}

// ============================================================================
// Scored corpus mode
// ============================================================================

#[derive(serde::Serialize)]
struct CandidateOut {
    camelot: String,
    standard: String,
    confidence: f64,
    agreement: f64,
    segment_count: usize,
}

#[derive(serde::Serialize)]
struct TrackRecord {
    path: String,
    title: String,
    artist: String,
    genre: String,
    extension: String,
    truth_camelot: Option<String>,
    truth_bpm: Option<f64>,
    truth_energy: Option<i32>,
    pred_camelot: Option<String>,
    pred_standard: Option<String>,
    pred_confidence: Option<f64>,
    pred_agreement: Option<f64>,
    pred_bpm: Option<f64>,
    candidates: Vec<CandidateOut>,
    mirex: Option<f64>,
    error_type: Option<String>,
    tonic_correct: Option<bool>,
    bpm_error: Option<f64>,
    bpm_error_octave: Option<f64>,
    total_ms: u64,
    decode_ms: u64,
    failure: Option<String>,
}

#[derive(serde::Serialize, Default, Clone)]
struct Partition {
    n: usize,
    exact: usize,
    tonic_correct: usize,
    mirex_sum: f64,
    compatible: usize,
    bpm_scored: usize,
    bpm_exact: usize,
    bpm_octave: usize,
    confusion: BTreeMap<String, usize>,
}

impl Partition {
    fn add(&mut self, r: &TrackRecord) {
        self.n += 1;
        if let Some(m) = r.mirex {
            self.mirex_sum += m;
            if m >= 1.0 {
                self.exact += 1;
            }
            if is_camelot_compatible(m) {
                self.compatible += 1;
            }
            *self.confusion.entry(r.error_type.clone().unwrap_or_default()).or_insert(0) += 1;
        }
        if r.tonic_correct == Some(true) {
            self.tonic_correct += 1;
        }
        if r.bpm_error.is_some() {
            self.bpm_scored += 1;
            if r.bpm_error.map(|e| e.abs() <= 1.0).unwrap_or(false) {
                self.bpm_exact += 1;
            }
            if r.bpm_error_octave.map(|e| e.abs() <= 1.0).unwrap_or(false) {
                self.bpm_octave += 1;
            }
        }
    }
}

#[derive(serde::Serialize)]
struct PartitionOut {
    n: usize,
    exact_pct: f64,
    tonic_only_pct: f64,
    mirex_mean: f64,
    compatible_pct: f64,
    bpm_exact_pct: f64,
    bpm_octave_pct: f64,
    bpm_scored: usize,
    confusion: BTreeMap<String, usize>,
}

fn partition_out(p: &Partition) -> PartitionOut {
    let n = p.n.max(1) as f64;
    let bpm_n = p.bpm_scored.max(1) as f64;
    PartitionOut {
        n: p.n,
        exact_pct: 100.0 * p.exact as f64 / n,
        tonic_only_pct: 100.0 * p.tonic_correct as f64 / n,
        mirex_mean: p.mirex_sum / n,
        compatible_pct: 100.0 * p.compatible as f64 / n,
        bpm_exact_pct: 100.0 * p.bpm_exact as f64 / bpm_n,
        bpm_octave_pct: 100.0 * p.bpm_octave as f64 / bpm_n,
        bpm_scored: p.bpm_scored,
        confusion: p.confusion.clone(),
    }
}

#[derive(serde::Serialize)]
struct CorpusReport {
    corpus_path: String,
    scored: usize,
    failed: usize,
    status_counts: BTreeMap<String, usize>,
    overall: PartitionOut,
    by_genre: BTreeMap<String, PartitionOut>,
    by_extension: BTreeMap<String, PartitionOut>,
    bpm_ratio_median: f64,
    avg_total_ms: f64,
    records: Vec<TrackRecord>,
}

fn analyse_row(row: &CorpusRow, weights: ProfileWeights, ablation: &tunelock_lib::analysis::key_detector::AblationConfig, edm_plain: Option<tunelock_lib::analysis::ensemble::EdmProfile>) -> TrackRecord {
    let start = Instant::now();
    let mut rec = TrackRecord {
        path: row.location.clone(),
        title: row.title.clone(),
        artist: row.artist.clone(),
        genre: row.genre.clone(),
        extension: row.extension.clone(),
        truth_camelot: row.key_camelot.clone(),
        truth_bpm: row.truth_bpm,
        truth_energy: row.truth_energy,
        pred_camelot: None,
        pred_standard: None,
        pred_confidence: None,
        pred_agreement: None,
        pred_bpm: None,
        candidates: Vec::new(),
        mirex: None,
        error_type: None,
        tonic_correct: None,
        bpm_error: None,
        bpm_error_octave: None,
        total_ms: 0,
        decode_ms: 0,
        failure: None,
    };

    let decode_start = Instant::now();
    let samples = match decode_media(&row.location) {
        Ok(s) => s,
        Err(e) => {
            rec.failure = Some(format!("decode: {}", e));
            rec.total_ms = start.elapsed().as_millis() as u64;
            return rec;
        }
    };
    rec.decode_ms = decode_start.elapsed().as_millis() as u64;

    let candidates = if let Some(edm) = edm_plain {
        match tunelock_lib::analysis::key_detector::detect_key_edm_plain_chroma(&samples, ablation, edm) {
            Ok(c) => c,
            Err(e) => {
                rec.failure = Some(format!("key: {}", e));
                rec.total_ms = start.elapsed().as_millis() as u64;
                return rec;
            }
        }
    } else {
        match tunelock_lib::analysis::key_detector::detect_key_ablation(&samples, weights, ablation) {
            Ok(c) => c,
            Err(e) => {
                rec.failure = Some(format!("key: {}", e));
                rec.total_ms = start.elapsed().as_millis() as u64;
                return rec;
            }
        }
    };

    if let Ok(bpm) = detect_tempo(&samples) {
        rec.pred_bpm = Some(bpm);
    }

    if let Some(w) = candidates.first() {
        rec.pred_camelot = Some(key_to_camelot(w.tonic, w.is_major));
        rec.pred_standard = Some(format!(
            "{} {}",
            pitch_class_to_name(w.tonic),
            if w.is_major { "major" } else { "minor" }
        ));
        rec.pred_confidence = Some(w.confidence);
        rec.pred_agreement = Some(w.agreement);

        rec.candidates = candidates
            .iter()
            .map(|c| CandidateOut {
                camelot: key_to_camelot(c.tonic, c.is_major),
                standard: format!(
                    "{} {}",
                    pitch_class_to_name(c.tonic),
                    if c.is_major { "major" } else { "minor" }
                ),
                confidence: c.confidence,
                agreement: c.agreement,
                segment_count: c.segment_count,
            })
            .collect();

        if let (Some(tt), Some(tm)) = (row.truth_tonic, row.truth_is_major) {
            let score = mirex_score(w.tonic, w.is_major, tt, tm);
            rec.mirex = Some(score);
            rec.error_type = Some(classify_error(w.tonic, w.is_major, tt, tm).as_str().to_string());
            rec.tonic_correct = Some(w.tonic == tt);
        }
    }

    if let (Some(pred), Some(truth)) = (rec.pred_bpm, row.truth_bpm) {
        rec.bpm_error = Some(pred - truth);
        // Octave-corrected error: distance after choosing the best of ×0.5/×1/×2.
        let corrected = [0.5, 1.0, 2.0]
            .iter()
            .map(|m| pred * m - truth)
            .min_by(|a, b| a.abs().partial_cmp(&b.abs()).unwrap())
            .unwrap();
        rec.bpm_error_octave = Some(corrected);
    }

    rec.total_ms = start.elapsed().as_millis() as u64;
    rec
}

fn run_scored_corpus(corpus_path: &str, limit: Option<usize>, out: Option<&str>, manifest: Option<&str>, seed: Option<u64>, ablation: &tunelock_lib::analysis::key_detector::AblationConfig, edm_plain: Option<tunelock_lib::analysis::ensemble::EdmProfile>) {
    println!("Loading corpus: {}", corpus_path);
    let rows = match load_mik_corpus(corpus_path) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to load corpus: {}", e);
            std::process::exit(1);
        }
    };
    score_rows(rows, corpus_path, limit, out, manifest, seed, ablation, edm_plain);
}

fn score_rows(rows: Vec<CorpusRow>, corpus_label: &str, limit: Option<usize>, out: Option<&str>, manifest: Option<&str>, seed: Option<u64>, ablation: &tunelock_lib::analysis::key_detector::AblationConfig, edm_plain: Option<tunelock_lib::analysis::ensemble::EdmProfile>) {
    // Classification summary
    let mut status_counts: BTreeMap<String, usize> = BTreeMap::new();
    for r in &rows {
        let label = match r.status {
            RowStatus::Ready => "ready",
            RowStatus::MissingFile => "missing_file",
            RowStatus::UnsupportedFormat => "unsupported_format",
            RowStatus::NoKeyLabel => "no_key_label",
            RowStatus::Atonal => "atonal",
            RowStatus::LargeMix => "large_mix",
        };
        *status_counts.entry(label.to_string()).or_insert(0) += 1;
    }
    println!("Corpus classification ({} rows):", rows.len());
    for (k, v) in &status_counts {
        println!("  {:<20} {}", k, v);
    }

    let mut ready: Vec<CorpusRow> = rows
        .into_iter()
        .filter(|r| r.status == RowStatus::Ready)
        .collect();

    if let Some(limit) = limit {
        let s = seed.unwrap_or(0x542E_4E4C_6F63_6B00);
        ready = stratified_sample_seeded(&ready, limit, s);
    }

    // Write manifest (the selected sample) for reproducibility.
    if let Some(manifest_path) = manifest {
        let manifest_entries: Vec<serde_json::Value> = ready.iter().map(|r| {
            serde_json::json!({
                "location": r.location,
                "title": r.title,
                "artist": r.artist,
                "genre_raw": r.genre,
                "genre_normalized": tunelock_lib::proof::corpus::normalize_genre(&r.genre),
                "key_camelot": r.key_camelot,
                "extension": r.extension,
            })
        }).collect();
        let manifest_json = serde_json::json!({
            "seed": seed.unwrap_or(0x542E_4E4C_6F63_6B00),
            "limit": limit,
            "count": ready.len(),
            "tracks": manifest_entries,
        });
        match std::fs::write(manifest_path, serde_json::to_string_pretty(&manifest_json).unwrap()) {
            Ok(_) => println!("Manifest written to {} ({} tracks)", manifest_path, ready.len()),
            Err(e) => eprintln!("Failed to write manifest: {}", e),
        }
    }

    let total = ready.len();
    println!("Scoring {} tracks{}…\n", total, limit.map(|l| format!(" (stratified limit {})", l)).unwrap_or_default());

    let weights = ProfileWeights::default();
    let done = AtomicUsize::new(0);
    let bench_start = Instant::now();

    let records: Vec<TrackRecord> = ready
        .par_iter()
        .map(|row| {
            let rec = analyse_row(row, weights, ablation, edm_plain);
            let n = done.fetch_add(1, Ordering::Relaxed) + 1;
            if n % 50 == 0 || n == total {
                let elapsed = bench_start.elapsed().as_secs_f64();
                let rate = n as f64 / elapsed.max(0.001);
                eprintln!(
                    "  [{}/{}] {:.1} tracks/min  eta {:.0} min",
                    n,
                    total,
                    rate * 60.0,
                    (total - n) as f64 / (rate * 60.0).max(0.001)
                );
            }
            rec
        })
        .collect();

    // Aggregate
    let mut overall = Partition::default();
    let mut by_genre: BTreeMap<String, Partition> = BTreeMap::new();
    let mut by_ext: BTreeMap<String, Partition> = BTreeMap::new();
    let mut ratios: Vec<f64> = Vec::new();
    let mut failed = 0usize;

    for r in &records {
        if r.failure.is_some() {
            failed += 1;
            continue;
        }
        overall.add(r);
        let genre = normalize_genre(&r.genre).to_string();
        by_genre.entry(genre).or_default().add(r);
        by_ext.entry(r.extension.clone()).or_default().add(r);
        if let (Some(pred), Some(truth)) = (r.pred_bpm, r.truth_bpm) {
            ratios.push(bpm_ratio(pred, truth));
        }
    }
    ratios.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let bpm_ratio_median = if ratios.is_empty() { 1.0 } else { ratios[ratios.len() / 2] };

    let avg_total_ms = {
        let scored: Vec<&TrackRecord> = records.iter().filter(|r| r.failure.is_none()).collect();
        if scored.is_empty() {
            0.0
        } else {
            scored.iter().map(|r| r.total_ms as f64).sum::<f64>() / scored.len() as f64
        }
    };

    let report = CorpusReport {
        corpus_path: corpus_label.to_string(),
        scored: overall.n,
        failed,
        status_counts,
        overall: partition_out(&overall),
        by_genre: by_genre.iter().map(|(k, v)| (k.clone(), partition_out(v))).collect(),
        by_extension: by_ext.iter().map(|(k, v)| (k.clone(), partition_out(v))).collect(),
        bpm_ratio_median,
        avg_total_ms,
        records,
    };

    print_summary(&report);

    if let Some(out_path) = out {
        match serde_json::to_string_pretty(&report) {
            Ok(json) => {
                if let Err(e) = std::fs::write(out_path, json) {
                    eprintln!("Failed to write report: {}", e);
                } else {
                    println!("Report written: {}", out_path);
                }
            }
            Err(e) => eprintln!("Failed to serialise report: {}", e),
        }
    }
}

fn print_summary(r: &CorpusReport) {
    let o = &r.overall;
    println!();
    println!("══════════════════════════════════════════════════════");
    println!("                  ACCURACY SUMMARY");
    println!("══════════════════════════════════════════════════════");
    println!("  Scored tracks:        {}  ({} failed)", o.n, r.failed);
    println!("  Key exact match:      {:.1}%", o.exact_pct);
    println!("  Tonic only (mode-agnostic): {:.1}%", o.tonic_only_pct);
    println!("  MIREX weighted:       {:.3}", o.mirex_mean);
    println!("  Camelot compatible:   {:.1}%", o.compatible_pct);
    println!("  BPM ±1:               {:.1}%   (n={})", o.bpm_exact_pct, o.bpm_scored);
    println!("  BPM ±1 octave-fixed:  {:.1}%", o.bpm_octave_pct);
    println!("  BPM ratio median:     {:.3}  (1.0 = unbiased, 0.5 = systematic half-time)", r.bpm_ratio_median);
    println!("  Avg time/track:       {:.0} ms", r.avg_total_ms);
    println!();
    println!("  Error taxonomy:");
    for (k, v) in &o.confusion {
        println!("    {:<12} {}", k, v);
    }
    println!();
    println!("  By genre:");
    for (g, p) in &r.by_genre {
        println!(
            "    {:<22} n={:<5} exact={:>5.1}%  mirex={:.3}  bpm±1={:>5.1}%",
            g, p.n, p.exact_pct, p.mirex_mean, p.bpm_exact_pct
        );
    }
    println!();
    println!("  By format:");
    for (e, p) in &r.by_extension {
        println!(
            "    {:<8} n={:<5} exact={:>5.1}%  mirex={:.3}",
            e, p.n, p.exact_pct, p.mirex_mean
        );
    }
    println!("══════════════════════════════════════════════════════");
}

// ============================================================================
// Legacy folder mode (unchanged behaviour, kept for smoke-testing a folder)
// ============================================================================

fn legacy_folder_mode(folder: &str) {
    let audio_exts = [
        "mp3", "wav", "flac", "ogg", "oga", "opus", "aiff", "aif", "m4a", "aac",
        "wma", "alac", "mkv",
        // Video containers — audio extracted via ffmpeg sidecar.
        "mp4", "mov", "webm", "m4v", "avi", "flv", "mpg", "mpeg", "ts", "3gp",
    ];
    let mut files: Vec<String> = Vec::new();

    for entry in walkdir::WalkDir::new(folder).into_iter().filter_map(|e| e.ok()) {
        if entry.file_type().is_file() {
            if let Some(ext) = entry.path().extension() {
                let ext = ext.to_string_lossy().to_lowercase();
                if audio_exts.contains(&ext.as_str()) {
                    files.push(entry.path().to_string_lossy().to_string());
                }
            }
        }
    }

    if files.is_empty() {
        eprintln!("No audio files found in {}", folder);
        std::process::exit(1);
    }

    println!("Folder: {}   Files: {}", folder, files.len());

    let weights = ProfileWeights::default();

    for (i, path) in files.iter().enumerate() {
        let filename = Path::new(path)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown");

        println!("── [{}/{}] {} ──", i + 1, files.len(), filename);

        let decode_start = Instant::now();
        let samples = match decode_media(path) {
            Ok(s) => s,
            Err(e) => {
                println!("  ERROR: Decode failed: {}", e);
                continue;
            }
        };
        let decode_ms = decode_start.elapsed().as_millis() as u64;

        let diag_start = Instant::now();
        let diagnostic = match detect_key_diagnostic(&samples, weights, |_, _| {}) {
            Ok(d) => d,
            Err(e) => {
                println!("  ERROR: Key detection failed: {}", e);
                continue;
            }
        };
        let total_ms = diag_start.elapsed().as_millis() as u64;

        if let Some(w) = diagnostic.candidates.first() {
            let mode = if w.is_major { "major" } else { "minor" };
            println!(
                "  Key:       {} {}  |  {}  |  conf={:.3}",
                pitch_class_to_name(w.tonic),
                mode,
                key_to_camelot(w.tonic, w.is_major),
                w.confidence
            );
            println!(
                "  Agreement: {:.1}%  ({}/8 segments)  |  avg score: {:.3}",
                w.agreement * 100.0,
                w.segment_count,
                w.avg_score
            );
        }

        if diagnostic.candidates.len() > 1 {
            println!("  Runners-up:");
            for (j, c) in diagnostic.candidates.iter().skip(1).take(4).enumerate() {
                let mode = if c.is_major { "major" } else { "minor" };
                println!(
                    "    {}. {} {}  ({})  conf={:.3}  agree={:.1}%  segs={}/8",
                    j + 2,
                    pitch_class_to_name(c.tonic),
                    mode,
                    key_to_camelot(c.tonic, c.is_major),
                    c.confidence,
                    c.agreement * 100.0,
                    c.segment_count
                );
            }
        }

        println!(
            "  Timings:   decode={}ms  spec={}ms  hpss={}ms  chroma={}ms  ens={}ms  |  total={}ms",
            decode_ms,
            diagnostic.timings.spectrogram,
            diagnostic.timings.hpss,
            diagnostic.timings.chromagram,
            diagnostic.timings.ensemble,
            total_ms + decode_ms
        );
        println!();
    }
}
