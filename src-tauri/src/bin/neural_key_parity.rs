//! Audit real-file Rust neural-key inference against cached Python posteriors.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Component, Path, PathBuf};
use std::time::Instant;

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::json;
use tunelock_lib::neural_key::NeuralKeySession;

const KEY_COUNT: usize = 24;

#[derive(Deserialize)]
struct CorpusManifest {
    records: Vec<CorpusRecord>,
}

#[derive(Deserialize)]
struct CorpusRecord {
    id: String,
    role: String,
    audio_path: PathBuf,
}

#[derive(Deserialize)]
struct ReferenceLine {
    #[serde(rename = "type")]
    line_type: String,
    track_id: Option<String>,
    status: Option<String>,
    posterior: Option<Vec<f32>>,
}

fn safe_manifest_path(root: &Path, relative: &Path) -> Result<PathBuf> {
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_) | Component::CurDir))
    {
        bail!("manifest audio path must be a safe relative path");
    }
    Ok(root.join(relative))
}

fn reference_posteriors(path: &Path) -> Result<HashMap<String, [f32; KEY_COUNT]>> {
    let reader =
        BufReader::new(File::open(path).with_context(|| format!("opening {}", path.display()))?);
    let mut result = HashMap::new();
    for (index, line) in reader.lines().enumerate() {
        let line =
            line.with_context(|| format!("reading {} line {}", path.display(), index + 1))?;
        let item: ReferenceLine = serde_json::from_str(&line)
            .with_context(|| format!("parsing {} line {}", path.display(), index + 1))?;
        if item.line_type != "prediction" || item.status.as_deref() != Some("ok") {
            continue;
        }
        let track_id = item
            .track_id
            .context("reference prediction is missing track_id")?;
        let values = item
            .posterior
            .context("reference prediction is missing posterior")?;
        let posterior: [f32; KEY_COUNT] = values.try_into().map_err(|values: Vec<f32>| {
            anyhow::anyhow!("expected 24 values, found {}", values.len())
        })?;
        if posterior
            .iter()
            .any(|value| !value.is_finite() || *value < 0.0)
            || (posterior.iter().sum::<f32>() - 1.0).abs() > 1.0e-3
        {
            bail!("reference posterior for {track_id} is invalid");
        }
        if result.insert(track_id.clone(), posterior).is_some() {
            bail!("duplicate reference prediction for {track_id}");
        }
    }
    Ok(result)
}

fn top_index(posterior: &[f32; KEY_COUNT]) -> usize {
    posterior
        .iter()
        .enumerate()
        .max_by(|left, right| left.1.total_cmp(right.1))
        .map(|(index, _)| index)
        .unwrap()
}

fn run(arguments: &[std::ffi::OsString]) -> Result<()> {
    if !(5..=6).contains(&arguments.len()) {
        bail!(
            "usage: tunelock-neural-key-parity <artifact-directory> <onnx-runtime-dylib> \
             <manifest.json> <reference.jsonl> <audio-root> [limit]"
        );
    }
    let artifact_directory = PathBuf::from(&arguments[0]);
    let runtime_dylib = PathBuf::from(&arguments[1]);
    let manifest_path = PathBuf::from(&arguments[2]);
    let reference_path = PathBuf::from(&arguments[3]);
    let audio_root = PathBuf::from(&arguments[4]);
    let limit = arguments
        .get(5)
        .map(|value| {
            value
                .to_string_lossy()
                .parse::<usize>()
                .context("limit must be a positive integer")
        })
        .transpose()?
        .unwrap_or(20);
    if limit == 0 {
        bail!("limit must be positive");
    }

    let manifest: CorpusManifest = serde_json::from_reader(
        File::open(&manifest_path)
            .with_context(|| format!("opening {}", manifest_path.display()))?,
    )
    .with_context(|| format!("parsing {}", manifest_path.display()))?;
    let references = reference_posteriors(&reference_path)?;
    let mut session = NeuralKeySession::load(&artifact_directory, &runtime_dylib)?;
    let labels = session.artifact.contract.output.posterior_labels.clone();
    let mut records = Vec::new();
    let mut failures = Vec::new();
    let mut top_matches = 0_usize;
    let mut total_absolute_error = 0.0_f64;
    let mut compared_values = 0_usize;
    let mut global_maximum_error = 0.0_f32;
    let mut total_inference_ms = 0_u128;
    let mut maximum_inference_ms = 0_u128;
    let started = Instant::now();

    for record in manifest
        .records
        .iter()
        .filter(|record| record.role == "development")
        .filter(|record| references.contains_key(&record.id))
        .take(limit)
    {
        let reference = &references[&record.id];
        let path = safe_manifest_path(&audio_root, &record.audio_path)?;
        let track_started = Instant::now();
        match session.predict_file(&path) {
            Ok(actual) => {
                let latency_ms = track_started.elapsed().as_millis();
                total_inference_ms += latency_ms;
                maximum_inference_ms = maximum_inference_ms.max(latency_ms);
                let expected_top = top_index(reference);
                let actual_top = top_index(&actual);
                top_matches += usize::from(expected_top == actual_top);
                let mut track_maximum_error = 0.0_f32;
                let mut track_absolute_error = 0.0_f64;
                for (actual, expected) in actual.iter().zip(reference) {
                    let error = (actual - expected).abs();
                    track_maximum_error = track_maximum_error.max(error);
                    global_maximum_error = global_maximum_error.max(error);
                    track_absolute_error += error as f64;
                    total_absolute_error += error as f64;
                    compared_values += 1;
                }
                records.push(json!({
                    "track_id": record.id,
                    "top_match": expected_top == actual_top,
                    "reference_top_index": expected_top,
                    "reference_top_label": labels[expected_top],
                    "rust_top_index": actual_top,
                    "rust_top_label": labels[actual_top],
                    "maximum_absolute_error": track_maximum_error,
                    "mean_absolute_error": track_absolute_error / KEY_COUNT as f64,
                    "latency_ms": latency_ms,
                }));
            }
            Err(error) => {
                failures.push(json!({
                    "track_id": record.id,
                    "error": format!("{error:#}"),
                }));
            }
        }
        eprintln!(
            "attempted={}/{} compared={} failures={}",
            records.len() + failures.len(),
            limit,
            records.len(),
            failures.len()
        );
    }

    let compared = records.len();
    if compared == 0 {
        bail!("no development records were compared");
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema_version": 1,
            "status": "research implementation-parity audit; not an accuracy score",
            "artifact": artifact_directory,
            "reference": reference_path,
            "requested": limit,
            "compared": compared,
            "failures": failures,
            "top1_agreement": top_matches as f64 / compared as f64,
            "maximum_absolute_posterior_error": global_maximum_error,
            "mean_absolute_posterior_error": total_absolute_error / compared_values as f64,
            "mean_inference_ms": total_inference_ms as f64 / compared as f64,
            "maximum_inference_ms": maximum_inference_ms,
            "elapsed_ms": started.elapsed().as_millis(),
            "records": records,
        }))?
    );
    Ok(())
}

fn main() -> Result<()> {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    run(&arguments)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_paths_cannot_escape_the_explicit_audio_root() {
        let root = Path::new("audio-root");
        assert!(safe_manifest_path(root, Path::new("corpus/audio/file.mp3")).is_ok());
        assert!(safe_manifest_path(root, Path::new("../secret.mp3")).is_err());
        assert!(safe_manifest_path(root, Path::new("C:\\secret.mp3")).is_err());
        assert!(safe_manifest_path(root, Path::new("")).is_err());
    }
}
