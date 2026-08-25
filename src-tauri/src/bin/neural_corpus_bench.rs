//! Neural corpus benchmark: run a neural key artifact with TTA over a
//! key-manifest corpus and emit per-track 24-key posteriors as JSONL.
//!
//! Usage:
//!   cargo run --release --bin neural-corpus-bench -- ^
//!       --artifact <artifact-dir> ^
//!       --runtime <onnx-runtime-dylib> ^
//!       --manifest <json> ^
//!       --role development ^
//!       --output <jsonl> ^
//!       [--tta] ^
//!       [--limit N]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use tunelock_lib::neural_key::NeuralKeySession;

#[derive(Deserialize)]
struct KeyCorpusManifest {
    canonical_labels: Vec<String>,
    records: Vec<KeyManifestRecord>,
}

#[derive(Deserialize)]
struct KeyManifestRecord {
    role: String,
    id: String,
    audio_path: String,
    truth_index: usize,
    truth_label: String,
    artist: String,
    genre: String,
}

#[derive(Serialize)]
struct TrackResult {
    id: String,
    artist: String,
    genre: String,
    truth_index: usize,
    truth_label: String,
    pred_index: usize,
    pred_label: String,
    posterior: Vec<f32>,
    mirex: f64,
    error_type: String,
    total_ms: u64,
    failure: Option<String>,
}

fn mirex_score(truth: usize, predicted: usize) -> f64 {
    if truth == predicted {
        return 1.0;
    }
    let truth_tonic = truth % 12;
    let pred_tonic = predicted % 12;
    let truth_minor = truth >= 12;
    let pred_minor = predicted >= 12;
    if truth_tonic == pred_tonic {
        // parallel
        return 0.6;
    }
    if truth_minor == pred_minor && (truth_tonic - pred_tonic + 12) % 12 == 7 {
        // fifth
        return 0.5;
    }
    if truth_minor == pred_minor && (truth_tonic - pred_tonic + 12) % 12 == 5 {
        // fifth (other direction)
        return 0.5;
    }
    if truth_minor != pred_minor && (truth_tonic - pred_tonic + 12) % 12 == (if truth_minor { 9 } else { 3 }) {
        // relative
        return 0.4;
    }
    if truth_minor == pred_minor && (truth_tonic - pred_tonic + 12) % 12 == 1 {
        // semitone
        return 0.3;
    }
    if truth_minor == pred_minor && (truth_tonic - pred_tonic + 12) % 12 == 11 {
        // semitone (other direction)
        return 0.3;
    }
    0.0
}

fn error_type(truth: usize, predicted: usize) -> String {
    if truth == predicted {
        return "correct".to_string();
    }
    let truth_tonic = truth % 12;
    let pred_tonic = predicted % 12;
    let truth_minor = truth >= 12;
    let pred_minor = predicted >= 12;
    if truth_tonic == pred_tonic {
        return "parallel".to_string();
    }
    if truth_minor == pred_minor && (truth_tonic - pred_tonic + 12) % 12 == 7 {
        return "fifth".to_string();
    }
    if truth_minor == pred_minor && (truth_tonic - pred_tonic + 12) % 12 == 5 {
        return "fifth".to_string();
    }
    if truth_minor != pred_minor && (truth_tonic - pred_tonic + 12) % 12 == (if truth_minor { 9 } else { 3 }) {
        return "relative".to_string();
    }
    if truth_minor == pred_minor && ((truth_tonic - pred_tonic + 12) % 12 == 1 || (truth_tonic - pred_tonic + 12) % 12 == 11) {
        return "semitone".to_string();
    }
    "other".to_string()
}

fn main() -> anyhow::Result<()> {
    let mut artifact_dir = None;
    let mut runtime_dylib = None;
    let mut manifest_path = None;
    let mut role = "development".to_string();
    let mut output = None;
    let mut tta = false;
    let mut limit: Option<usize> = None;

    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--artifact" => artifact_dir = it.next(),
            "--runtime" => runtime_dylib = it.next(),
            "--manifest" => manifest_path = it.next(),
            "--role" => role = it.next().unwrap_or("development".into()),
            "--output" => output = it.next(),
            "--tta" => tta = true,
            "--limit" => limit = it.next().and_then(|s| s.parse().ok()),
            "--help" | "-h" => {
                eprintln!("Usage: neural-corpus-bench --artifact <dir> --runtime <dylib> --manifest <json> --output <jsonl> [--role development] [--tta] [--limit N]");
                std::process::exit(0);
            }
            other => eprintln!("Unknown argument: {other}"),
        }
    }

    let artifact_dir = artifact_dir.expect("--artifact is required");
    let runtime_dylib = runtime_dylib.expect("--runtime is required");
    let manifest_path = manifest_path.expect("--manifest is required");
    let output = output.expect("--output is required");

    let manifest_bytes = std::fs::read(&manifest_path)?;
    let manifest: KeyCorpusManifest = serde_json::from_slice(&manifest_bytes)?;
    assert_eq!(manifest.canonical_labels.len(), 24, "manifest must have 24 labels");

    let mut records: Vec<&KeyManifestRecord> = manifest.records.iter()
        .filter(|r| r.role == role)
        .collect();
    if let Some(n) = limit {
        records.truncate(n);
    }
    eprintln!("Loaded {} records with role={}", records.len(), role);

    eprintln!("Loading neural-key artifact and ONNX runtime...");
    let mut session = NeuralKeySession::load(
        std::path::Path::new(&artifact_dir),
        std::path::Path::new(&runtime_dylib),
    )?;
    let labels = manifest.canonical_labels.clone();
    eprintln!("Artifact loaded. Running {} (tta={})...", if tta { "predict_file_tta" } else { "predict_file" }, tta);

    let total = records.len();
    let completed = AtomicUsize::new(0);
    let failed = AtomicUsize::new(0);

    let start = Instant::now();
    let mut results: Vec<TrackResult> = Vec::with_capacity(total);
    for record in &records {
        let track_start = Instant::now();
        let posterior_result = if tta {
            session.predict_file_tta(std::path::Path::new(&record.audio_path))
        } else {
            session.predict_file(std::path::Path::new(&record.audio_path))
        };
        let total_ms = track_start.elapsed().as_millis() as u64;

        let done = completed.fetch_add(1, Ordering::Relaxed) + 1;
        if done % 25 == 0 || done == total {
            let elapsed = start.elapsed().as_secs_f64();
            let rate = done as f64 / elapsed;
            let eta = (total - done) as f64 / rate;
            eprintln!(
                "  [{}/{}] {:.1} tracks/s eta {:.0}s failed={}",
                done, total, rate, eta, failed.load(Ordering::Relaxed)
            );
        }

        match posterior_result {
            Ok(posterior) => {
                let (pred_index, _) = posterior
                    .iter()
                    .copied()
                    .enumerate()
                    .max_by(|a, b| a.1.total_cmp(&b.1))
                    .unwrap_or((0, 0.0));
                results.push(TrackResult {
                    id: record.id.clone(),
                    artist: record.artist.clone(),
                    genre: record.genre.clone(),
                    truth_index: record.truth_index,
                    truth_label: record.truth_label.clone(),
                    pred_index,
                    pred_label: labels[pred_index].clone(),
                    posterior: posterior.to_vec(),
                    mirex: mirex_score(record.truth_index, pred_index),
                    error_type: error_type(record.truth_index, pred_index),
                    total_ms,
                    failure: None,
                });
            }
            Err(e) => {
                failed.fetch_add(1, Ordering::Relaxed);
                results.push(TrackResult {
                    id: record.id.clone(),
                    artist: record.artist.clone(),
                    genre: record.genre.clone(),
                    truth_index: record.truth_index,
                    truth_label: record.truth_label.clone(),
                    pred_index: 0,
                    pred_label: String::new(),
                    posterior: vec![0.0; 24],
                    mirex: 0.0,
                    error_type: "failure".to_string(),
                    total_ms,
                    failure: Some(format!("{e}")),
                });
            }
        }
    }

    let elapsed = start.elapsed().as_secs_f64();
    let scored = results.iter().filter(|r| r.failure.is_none()).count();
    let exact = results.iter().filter(|r| r.mirex >= 1.0).count();
    let mirex_sum: f64 = results.iter().map(|r| r.mirex).sum();
    let failed_count = results.iter().filter(|r| r.failure.is_some()).count();

    // Error taxonomy
    let mut confusion: BTreeMap<String, usize> = BTreeMap::new();
    for r in &results {
        *confusion.entry(r.error_type.clone()).or_insert(0) += 1;
    }

    // By genre
    let mut by_genre: BTreeMap<String, (usize, usize, f64)> = BTreeMap::new();
    for r in &results {
        if r.failure.is_some() {
            continue;
        }
        let entry = by_genre.entry(r.genre.clone()).or_insert((0, 0, 0.0));
        entry.0 += 1;
        if r.mirex >= 1.0 {
            entry.1 += 1;
        }
        entry.2 += r.mirex;
    }

    eprintln!();
    eprintln!("══════════════════════════════════════════════════════");
    eprintln!("                  NEURAL BENCHMARK SUMMARY");
    eprintln!("══════════════════════════════════════════════════════");
    eprintln!("  Scored tracks:        {}  ({} failed)", scored, failed_count);
    eprintln!("  Key exact match:      {:.1}%", 100.0 * exact as f64 / scored.max(1) as f64);
    eprintln!("  MIREX weighted:       {:.3}", mirex_sum / scored.max(1) as f64);
    eprintln!("  Avg time/track:       {:.0} ms", results.iter().map(|r| r.total_ms).sum::<u64>() as f64 / scored.max(1) as f64);
    eprintln!("  Total elapsed:        {:.1}s", elapsed);
    eprintln!();
    eprintln!("  Error taxonomy:");
    for (error, count) in &confusion {
        eprintln!("    {:<12} {}", error, count);
    }
    eprintln!();
    eprintln!("  By genre:");
    for (genre, (n, exact, mirex)) in by_genre.iter().rev() {
        eprintln!("    {:<20} n={:<5} exact={:.1}%  mirex={:.3}", genre, n, 100.0 * *exact as f64 / *n as f64, mirex / *n as f64);
    }
    eprintln!("══════════════════════════════════════════════════════");

    // Write JSONL output
    let output_path = PathBuf::from(&output);
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::File::create(&output_path)?;
    use std::io::Write;
    for result in &results {
        writeln!(file, "{}", serde_json::to_string(result)?)?;
    }
    eprintln!("Posteriors written to: {}", output);

    Ok(())
}
