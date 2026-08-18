//! CLI test harness for TuneLock key detection.
//!
//! Usage:
//!   cargo run --bin tunelock-bench -- <folder_path>
//!
//! Processes all audio files in the given folder and prints a structured
//! report with key, confidence, runner-up candidates, per-stage timings,
//! and chroma vector.

use std::path::Path;
use std::time::Instant;

use tunelock_lib::analysis::ensemble::ProfileWeights;
use tunelock_lib::analysis::key_detector::{detect_key_diagnostic, KeyDiagnostic};
use tunelock_lib::analysis::{pitch_class_to_name, key_to_camelot};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: tunelock-bench <folder_path>");
        std::process::exit(1);
    }
    let folder = &args[1];

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

    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║           TuneLock Key Detection — Test Report               ║");
    println!("╠══════════════════════════════════════════════════════════════╣");
    println!("║ Pipeline: STFT(16384) → HPSS → 12-bin + 72-band chroma      ║");
    println!("║ Ensemble: Krumhansl + Temperley (12) + Sha'ath (72)          ║");
    println!("║ Temporal: 8-segment ranked voting                            ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!();
    println!("Folder: {}", folder);
    println!("Files:  {}", files.len());
    println!();

    let weights = ProfileWeights::default();
    let mut results: Vec<(String, KeyDiagnostic, f64)> = Vec::new();

    for (i, path) in files.iter().enumerate() {
        let filename = Path::new(path)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown");

        println!("── [{}/{}] {} ──", i + 1, files.len(), filename);

        let decode_start = Instant::now();
        let samples = match tunelock_lib::analysis::decoder::decode_audio(path) {
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

        // Winner
        let winner = diagnostic.candidates.first();
        if let Some(w) = winner {
            let mode = if w.is_major { "major" } else { "minor" };
            println!("  Key:       {} {}  |  {}  |  conf={:.3}",
                pitch_class_to_name(w.tonic), mode,
                key_to_camelot(w.tonic, w.is_major),
                w.confidence);
            println!("  Agreement: {:.1}%  ({}/8 segments)  |  avg score: {:.3}",
                w.agreement * 100.0, w.segment_count, w.avg_score);
        }

        // Runners-up
        if diagnostic.candidates.len() > 1 {
            println!("  Runners-up:");
            for (j, c) in diagnostic.candidates.iter().skip(1).take(4).enumerate() {
                let mode = if c.is_major { "major" } else { "minor" };
                println!("    {}. {} {}  ({})  conf={:.3}  agree={:.1}%  segs={}/8",
                    j + 2,
                    pitch_class_to_name(c.tonic), mode,
                    key_to_camelot(c.tonic, c.is_major),
                    c.confidence, c.agreement * 100.0, c.segment_count);
            }
        }

        // Chroma
        print!("  Chroma:    [");
        for (j, v) in diagnostic.chroma_mean.iter().enumerate() {
            let bar = (v * 10.0).round() as usize;
            print!("{}", "#".repeat(bar.min(10)));
            if j < 11 { print!(" "); }
        }
        println!("]");

        // Timings
        println!("  Timings:   decode={}ms  spec={}ms  hpss={}ms  chroma={}ms  ens={}ms  |  total={}ms",
            decode_ms,
            diagnostic.timings.spectrogram,
            diagnostic.timings.hpss,
            diagnostic.timings.chromagram,
            diagnostic.timings.ensemble,
            total_ms + decode_ms);

        // Sample info
        println!("  Samples:   {}  ({:.1}s @ {}Hz)",
            samples.len(),
            samples.len() as f64 / tunelock_lib::analysis::SAMPLE_RATE as f64,
            tunelock_lib::analysis::SAMPLE_RATE);

        println!();
        results.push((filename.to_string(), diagnostic, total_ms as f64 + decode_ms as f64));
    }

    // Summary
    if results.is_empty() {
        println!("No results to summarize.");
        return;
    }

    println!("═══════════════════════════════════════════════════════════════");
    println!("                         SUMMARY                               ");
    println!("═══════════════════════════════════════════════════════════════");
    println!();

    let mut total_time = 0.0f64;
    let mut total_confidence = 0.0f64;
    let mut total_agreement = 0.0f64;

    for (filename, diag, elapsed) in &results {
        let w = diag.candidates.first();
        let conf = w.map(|c| c.confidence).unwrap_or(0.0);
        let agree = w.map(|c| c.agreement).unwrap_or(0.0);
        let mode = w.map(|c| if c.is_major { "major" } else { "minor" }).unwrap_or("?");
        let tonic = w.map(|c| pitch_class_to_name(c.tonic)).unwrap_or("?");
        let camelot = w.map(|c| key_to_camelot(c.tonic, c.is_major)).unwrap_or_default();

        println!("  {:>30}  →  {} {}  ({})  conf={:.3}  agree={:.1}%  {:.0}ms",
            filename, tonic, mode, camelot, conf, agree * 100.0, elapsed);

        total_time += elapsed;
        total_confidence += conf;
        total_agreement += agree;
    }

    let n = results.len() as f64;
    println!();
    println!("  ─────────────────────────────────────────────");
    println!("  Avg confidence:  {:.3}", total_confidence / n);
    println!("  Avg agreement:   {:.1}%", total_agreement / n * 100.0);
    println!("  Avg time:        {:.0}ms", total_time / n);
    println!("  Total time:      {:.0}ms", total_time);
    println!();

    // Per-stage averages
    let mut avg_spec = 0u64;
    let mut avg_hpss = 0u64;
    let mut avg_chroma = 0u64;
    let mut avg_ens = 0u64;
    for (_, diag, _) in &results {
        avg_spec += diag.timings.spectrogram;
        avg_hpss += diag.timings.hpss;
        avg_chroma += diag.timings.chromagram;
        avg_ens += diag.timings.ensemble;
    }
    let n_u64 = results.len() as u64;
    println!("  Stage breakdown (avg):");
    println!("    Spectrogram:  {}ms", avg_spec / n_u64);
    println!("    HPSS:         {}ms", avg_hpss / n_u64);
    println!("    Chromagram:   {}ms", avg_chroma / n_u64);
    println!("    Ensemble:     {}ms", avg_ens / n_u64);
    println!();
    println!("═══════════════════════════════════════════════════════════════");
}
