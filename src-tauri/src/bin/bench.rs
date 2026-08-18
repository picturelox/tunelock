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
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use rayon::prelude::*;
use tunelock_lib::analysis::decoder::decode_audio;
use tunelock_lib::analysis::ensemble::ProfileWeights;
use tunelock_lib::analysis::key_detector::detect_key_diagnostic;
use tunelock_lib::analysis::tempo_detector::detect_tempo;
use tunelock_lib::analysis::{key_to_camelot, pitch_class_to_name};
use tunelock_lib::proof::corpus::{load_giantsteps, load_mik_corpus, stratified_sample, CorpusRow, RowStatus};
use tunelock_lib::proof::metrics::{bpm_ratio, classify_error, is_camelot_compatible, mirex_score};

// ============================================================================
// CLI
// ============================================================================

struct Args {
    corpus: Option<String>,
    giantsteps: Option<String>,
    limit: Option<usize>,
    out: Option<String>,
    folder: Option<String>,
}

fn parse_args() -> Args {
    let mut a = Args { corpus: None, giantsteps: None, limit: None, out: None, folder: None };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--corpus" => a.corpus = it.next(),
            "--giantsteps" => a.giantsteps = it.next(),
            "--limit" => a.limit = it.next().and_then(|s| s.parse().ok()),
            "--out" => a.out = it.next(),
            "--help" | "-h" => {
                eprintln!("Usage:");
                eprintln!("  tunelock-bench --corpus <csv> [--limit N] [--out report.json]");
                eprintln!("  tunelock-bench --giantsteps <dataset_root> [--limit N] [--out report.json]");
                eprintln!("  tunelock-bench <folder>            (legacy diagnostic mode)");
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
    if let Some(corpus) = args.corpus {
        run_scored_corpus(&corpus, args.limit, args.out.as_deref());
    } else if let Some(gs) = args.giantsteps {
        run_giantsteps(&gs, args.limit, args.out.as_deref());
    } else if let Some(folder) = args.folder {
        legacy_folder_mode(&folder);
    } else {
        eprintln!("Usage:");
        eprintln!("  tunelock-bench --corpus <csv> [--limit N] [--out report.json]");
        eprintln!("  tunelock-bench --giantsteps <dataset_root> [--limit N] [--out report.json]");
        eprintln!("  tunelock-bench <folder>            (legacy diagnostic mode)");
        std::process::exit(1);
    }
}

fn run_giantsteps(root: &str, limit: Option<usize>, out: Option<&str>) {
    let rows = load_giantsteps(Path::new(root));
    if rows.is_empty() {
        eprintln!("No GiantSteps annotations found under {}", root);
        std::process::exit(1);
    }
    score_rows(rows, root, limit, out);
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

fn analyse_row(row: &CorpusRow, weights: ProfileWeights) -> TrackRecord {
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
    let samples = match decode_audio(&row.location) {
        Ok(s) => s,
        Err(e) => {
            rec.failure = Some(format!("decode: {}", e));
            rec.total_ms = start.elapsed().as_millis() as u64;
            return rec;
        }
    };
    rec.decode_ms = decode_start.elapsed().as_millis() as u64;

    let diag = match detect_key_diagnostic(&samples, weights, |_, _| {}) {
        Ok(d) => d,
        Err(e) => {
            rec.failure = Some(format!("key: {}", e));
            rec.total_ms = start.elapsed().as_millis() as u64;
            return rec;
        }
    };

    if let Ok(bpm) = detect_tempo(&samples) {
        rec.pred_bpm = Some(bpm);
    }

    if let Some(w) = diag.candidates.first() {
        rec.pred_camelot = Some(key_to_camelot(w.tonic, w.is_major));
        rec.pred_standard = Some(format!(
            "{} {}",
            pitch_class_to_name(w.tonic),
            if w.is_major { "major" } else { "minor" }
        ));
        rec.pred_confidence = Some(w.confidence);
        rec.pred_agreement = Some(w.agreement);

        rec.candidates = diag
            .candidates
            .iter()
            .take(5)
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

fn run_scored_corpus(corpus_path: &str, limit: Option<usize>, out: Option<&str>) {
    println!("Loading corpus: {}", corpus_path);
    let rows = match load_mik_corpus(corpus_path) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to load corpus: {}", e);
            std::process::exit(1);
        }
    };
    score_rows(rows, corpus_path, limit, out);
}

fn score_rows(rows: Vec<CorpusRow>, corpus_label: &str, limit: Option<usize>, out: Option<&str>) {
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
        ready = stratified_sample(&ready, limit);
    }
    let total = ready.len();
    println!("Scoring {} tracks{}…\n", total, limit.map(|l| format!(" (stratified limit {})", l)).unwrap_or_default());

    let weights = ProfileWeights::default();
    let done = AtomicUsize::new(0);
    let bench_start = Instant::now();

    let records: Vec<TrackRecord> = ready
        .par_iter()
        .map(|row| {
            let rec = analyse_row(row, weights);
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
        let genre = if r.genre.is_empty() { "(unknown)".to_string() } else { r.genre.to_lowercase() };
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
    let audio_exts = ["mp3", "wav", "flac", "ogg", "aiff", "m4a", "aac", "wma"];
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
        let samples = match decode_audio(path) {
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
