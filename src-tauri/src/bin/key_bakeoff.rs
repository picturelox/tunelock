//! Posterior-level bakeoff for TuneLock and external key models.
//!
//! External adapters emit labeled probabilities. This binary deliberately owns
//! all key parsing and scoring so Rust `harmony/` + `proof/metrics.rs` remain the
//! source of truth; Python model runners do not grow another Camelot vocabulary.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use tunelock_lib::analysis::pitch_class_to_name;
use tunelock_lib::proof::corpus::{load_giantsteps, parse_standard_key, RowStatus};
use tunelock_lib::proof::metrics::{classify_error, is_camelot_compatible, mirex_score};

const KEY_COUNT: usize = 24;
const FOLDS: usize = 5;

#[derive(Debug)]
struct Args {
    giantsteps: String,
    tunelock: String,
    model: String,
    additional_models: Vec<String>,
    cases_out: Option<String>,
    out: Option<String>,
}

fn parse_args() -> Result<Args> {
    let mut giantsteps = None;
    let mut tunelock = None;
    let mut model = None;
    let mut additional_models = Vec::new();
    let mut cases_out = None;
    let mut out = None;
    let mut args = std::env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--giantsteps" => giantsteps = args.next(),
            "--tunelock" => tunelock = args.next(),
            "--model" => model = args.next(),
            "--additional-model" => additional_models.push(
                args.next()
                    .context("--additional-model requires a posterior JSONL path")?,
            ),
            "--cases-out" => cases_out = args.next(),
            "--out" => out = args.next(),
            "--help" | "-h" => {
                println!(
                    "Usage: tunelock-key-bakeoff --giantsteps <root> \\\n                     --tunelock <report.json> --model <posterior.jsonl> [--out <report.json>]"
                );
                std::process::exit(0);
            }
            other => bail!("unknown argument: {other}"),
        }
    }

    Ok(Args {
        giantsteps: giantsteps.context("--giantsteps is required")?,
        tunelock: tunelock.context("--tunelock is required")?,
        model: model.context("--model is required")?,
        additional_models,
        cases_out,
        out,
    })
}

#[derive(Debug, Deserialize)]
struct TuneLockReport {
    records: Vec<TuneLockRecord>,
}

#[derive(Debug, Deserialize)]
struct TuneLockRecord {
    title: String,
    pred_standard: Option<String>,
    #[serde(default)]
    candidates: Vec<TuneLockCandidate>,
    failure: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TuneLockCandidate {
    standard: String,
    confidence: f64,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum ExternalLine {
    #[serde(rename = "metadata")]
    Metadata {
        schema_version: u32,
        model: String,
        model_revision: Option<String>,
        posterior_labels: Vec<String>,
        protocol: String,
    },
    #[serde(rename = "prediction")]
    Prediction {
        track_id: String,
        status: String,
        #[serde(default)]
        posterior: Vec<f64>,
    },
}

#[derive(Debug, Clone, Copy)]
struct ModelOutput {
    pred: usize,
    posterior: Option<[f64; KEY_COUNT]>,
}

#[derive(Debug)]
struct Example {
    id: String,
    genre: String,
    truth: usize,
    tunelock: ModelOutput,
    external: ModelOutput,
}

#[derive(Debug, Clone, Serialize)]
struct Score {
    n: usize,
    exact: usize,
    exact_pct: f64,
    tonic_correct_pct: f64,
    mirex: f64,
    compatible_pct: f64,
    top3_pct: Option<f64>,
    top5_pct: Option<f64>,
    confusion: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Copy, Serialize)]
struct FusionConfig {
    external_weight: f64,
    tunelock_temperature: f64,
    external_temperature: f64,
}

#[derive(Debug, Serialize)]
struct FoldResult {
    fold: usize,
    train_n: usize,
    test_n: usize,
    config: FusionConfig,
    test_exact: usize,
    test_mirex: f64,
}

#[derive(Debug, Serialize)]
struct FusionResult {
    method: &'static str,
    coverage: usize,
    equal_weight: Score,
    out_of_fold: Score,
    folds: Vec<FoldResult>,
}

#[derive(Debug, Serialize)]
struct Complementarity {
    n: usize,
    both_correct: usize,
    tunelock_only_correct: usize,
    external_only_correct: usize,
    neither_correct: usize,
    exact_oracle: usize,
    exact_oracle_pct: f64,
    weighted_oracle_mirex: f64,
    top3_union_pct: Option<f64>,
    additions_needed_for_75_pct: usize,
    external_unique_corrections_needed_for_75_pct: usize,
}

#[derive(Debug)]
struct LoadedAdditionalModel {
    model: String,
    revision: Option<String>,
    protocol: String,
    outputs: HashMap<String, ModelOutput>,
}

#[derive(Debug, Serialize)]
struct AdditionalModelScore {
    model: String,
    revision: Option<String>,
    protocol: String,
    score: Score,
}

#[derive(Debug, Serialize)]
struct MultiModelOracle {
    models: Vec<String>,
    n: usize,
    exact: usize,
    exact_pct: f64,
    added_beyond_primary_pair: usize,
    weighted_mirex: f64,
    top3_union_pct: Option<f64>,
}

#[derive(Debug)]
struct MultiExample {
    id: String,
    truth: usize,
    outputs: Vec<ModelOutput>,
}

#[derive(Debug, Clone, Serialize)]
struct MultiFusionConfig {
    weights: Vec<f64>,
}

#[derive(Debug, Serialize)]
struct MultiFusionFold {
    fold: usize,
    train_n: usize,
    test_n: usize,
    config: MultiFusionConfig,
    test_exact: usize,
    test_mirex: f64,
}

#[derive(Debug, Serialize)]
struct MultiFusionResult {
    method: &'static str,
    models: Vec<String>,
    coverage: usize,
    equal_weight: Score,
    out_of_fold: Score,
    folds: Vec<MultiFusionFold>,
}

#[derive(Debug, Serialize)]
struct StackingModelOutput {
    model: String,
    pred_index: usize,
    posterior: [f64; KEY_COUNT],
}

#[derive(Debug, Serialize)]
struct StackingCase {
    id: String,
    genre: String,
    fold: usize,
    truth_index: usize,
    models: Vec<StackingModelOutput>,
}

#[derive(Debug, Serialize)]
struct StackingDataset {
    schema_version: u32,
    corpus_role: &'static str,
    canonical_labels: Vec<String>,
    records: Vec<StackingCase>,
}

#[derive(Debug, Serialize)]
struct BakeoffReport {
    schema_version: u32,
    corpus: String,
    corpus_role: &'static str,
    external_model: String,
    external_revision: Option<String>,
    external_protocol: String,
    tunelock: Score,
    external: Score,
    complementarity: Complementarity,
    fusion: Option<FusionResult>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    additional_models: Vec<AdditionalModelScore>,
    #[serde(skip_serializing_if = "Option::is_none")]
    multi_model_oracle: Option<MultiModelOracle>,
    #[serde(skip_serializing_if = "Option::is_none")]
    multi_model_fusion: Option<MultiFusionResult>,
    exclusions: BTreeMap<String, usize>,
}

fn key_index(tonic: usize, is_major: bool) -> usize {
    if is_major {
        tonic
    } else {
        12 + tonic
    }
}

fn key_parts(index: usize) -> (usize, bool) {
    (index % 12, index < 12)
}

fn parse_key_label(label: &str) -> Result<usize> {
    let mut parts = label.split_whitespace();
    let tonic = parts.next().context("missing tonic")?;
    let mode = parts.next().context("missing mode")?.to_ascii_lowercase();
    if parts.next().is_some() {
        bail!("unexpected key label: {label}");
    }
    let normalized = format!("{tonic} {mode}");
    let (pitch, major) = parse_standard_key(&normalized)
        .with_context(|| format!("unrecognized key label: {label}"))?;
    Ok(key_index(pitch, major))
}

fn normalize(values: &mut [f64; KEY_COUNT]) -> Result<()> {
    for value in values.iter_mut() {
        if !value.is_finite() {
            bail!("posterior contains a non-finite value");
        }
        *value = value.max(0.0);
    }
    let sum: f64 = values.iter().sum();
    if sum <= 0.0 {
        bail!("posterior has no positive mass");
    }
    for value in values.iter_mut() {
        *value /= sum;
    }
    Ok(())
}

fn argmax(values: &[f64; KEY_COUNT]) -> usize {
    values
        .iter()
        .enumerate()
        .max_by(|(left_index, left), (right_index, right)| {
            left.partial_cmp(right)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| right_index.cmp(left_index))
        })
        .map(|(index, _)| index)
        .unwrap_or(0)
}

fn canonical_posterior(labels: &[String], posterior: &[f64]) -> Result<[f64; KEY_COUNT]> {
    if labels.len() != KEY_COUNT || posterior.len() != KEY_COUNT {
        bail!(
            "expected {KEY_COUNT} labels and probabilities, got {} and {}",
            labels.len(),
            posterior.len()
        );
    }

    let mut result = [0.0; KEY_COUNT];
    let mut seen = HashSet::new();
    for (label, probability) in labels.iter().zip(posterior) {
        let index = parse_key_label(label)?;
        if !seen.insert(index) {
            bail!("duplicate canonical key in posterior labels: {label}");
        }
        result[index] = *probability;
    }
    normalize(&mut result)?;
    Ok(result)
}

fn load_external(
    path: &Path,
) -> Result<(String, Option<String>, String, HashMap<String, ModelOutput>)> {
    let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut metadata: Option<(String, Option<String>, String, Vec<String>)> = None;
    let mut predictions = HashMap::new();

    for (line_index, line) in BufReader::new(file).lines().enumerate() {
        let line = line.with_context(|| format!("reading line {}", line_index + 1))?;
        if line.trim().is_empty() {
            continue;
        }
        let item: ExternalLine = serde_json::from_str(&line)
            .with_context(|| format!("parsing JSON line {}", line_index + 1))?;
        match item {
            ExternalLine::Metadata {
                schema_version,
                model,
                model_revision,
                posterior_labels,
                protocol,
            } => {
                if schema_version != 1 {
                    bail!("unsupported external posterior schema {schema_version}");
                }
                if metadata.is_some() {
                    bail!("external posterior file contains multiple metadata lines");
                }
                metadata = Some((model, model_revision, protocol, posterior_labels));
            }
            ExternalLine::Prediction {
                track_id,
                status,
                posterior,
            } => {
                if status != "ok" {
                    continue;
                }
                let labels = &metadata
                    .as_ref()
                    .context("metadata must precede predictions")?
                    .3;
                let canonical = canonical_posterior(labels, &posterior)
                    .with_context(|| format!("canonicalizing posterior for {track_id}"))?;
                let output = ModelOutput {
                    pred: argmax(&canonical),
                    posterior: Some(canonical),
                };
                if predictions.insert(track_id.clone(), output).is_some() {
                    bail!("duplicate successful external prediction for {track_id}");
                }
            }
        }
    }

    let (model, revision, protocol, _) =
        metadata.context("external posterior metadata is missing")?;
    Ok((model, revision, protocol, predictions))
}

fn load_tunelock(path: &Path) -> Result<HashMap<String, ModelOutput>> {
    let report: TuneLockReport = serde_json::from_reader(
        File::open(path).with_context(|| format!("opening {}", path.display()))?,
    )
    .with_context(|| format!("parsing {}", path.display()))?;

    let mut outputs = HashMap::new();
    for record in report.records {
        if record.failure.is_some() {
            continue;
        }
        let Some(predicted) = record.pred_standard.as_deref() else {
            continue;
        };
        let pred = parse_key_label(predicted)
            .with_context(|| format!("parsing TuneLock prediction for {}", record.title))?;

        let posterior = if record.candidates.len() == KEY_COUNT {
            let labels: Vec<String> = record
                .candidates
                .iter()
                .map(|item| item.standard.clone())
                .collect();
            let values: Vec<f64> = record
                .candidates
                .iter()
                .map(|item| item.confidence)
                .collect();
            Some(canonical_posterior(&labels, &values).with_context(|| {
                format!("canonicalizing TuneLock posterior for {}", record.title)
            })?)
        } else {
            None
        };

        if let Some(distribution) = posterior {
            if argmax(&distribution) != pred {
                bail!(
                    "TuneLock winner disagrees with posterior argmax for {}",
                    record.title
                );
            }
        }
        outputs.insert(record.title, ModelOutput { pred, posterior });
    }
    Ok(outputs)
}

fn rank_contains(posterior: &[f64; KEY_COUNT], truth: usize, k: usize) -> bool {
    let mut order: Vec<usize> = (0..KEY_COUNT).collect();
    order.sort_by(|left, right| {
        posterior[*right]
            .partial_cmp(&posterior[*left])
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.cmp(right))
    });
    order.into_iter().take(k).any(|index| index == truth)
}

fn evaluate(outputs: &[(usize, ModelOutput)]) -> Score {
    let mut exact = 0usize;
    let mut tonic_correct = 0usize;
    let mut mirex_sum = 0.0;
    let mut compatible = 0usize;
    let mut top3 = 0usize;
    let mut top5 = 0usize;
    let mut posterior_n = 0usize;
    let mut confusion = BTreeMap::new();

    for (truth, output) in outputs {
        let (pred_tonic, pred_major) = key_parts(output.pred);
        let (truth_tonic, truth_major) = key_parts(*truth);
        let weighted = mirex_score(pred_tonic, pred_major, truth_tonic, truth_major);
        if output.pred == *truth {
            exact += 1;
        }
        if pred_tonic == truth_tonic {
            tonic_correct += 1;
        }
        if is_camelot_compatible(weighted) {
            compatible += 1;
        }
        mirex_sum += weighted;
        *confusion
            .entry(
                classify_error(pred_tonic, pred_major, truth_tonic, truth_major)
                    .as_str()
                    .to_string(),
            )
            .or_insert(0) += 1;

        if let Some(posterior) = &output.posterior {
            posterior_n += 1;
            if rank_contains(posterior, *truth, 3) {
                top3 += 1;
            }
            if rank_contains(posterior, *truth, 5) {
                top5 += 1;
            }
        }
    }

    let n = outputs.len();
    let denominator = n.max(1) as f64;
    Score {
        n,
        exact,
        exact_pct: 100.0 * exact as f64 / denominator,
        tonic_correct_pct: 100.0 * tonic_correct as f64 / denominator,
        mirex: mirex_sum / denominator,
        compatible_pct: 100.0 * compatible as f64 / denominator,
        top3_pct: (posterior_n == n && n > 0).then_some(100.0 * top3 as f64 / denominator),
        top5_pct: (posterior_n == n && n > 0).then_some(100.0 * top5 as f64 / denominator),
        confusion,
    }
}

fn temperature_scale(posterior: &[f64; KEY_COUNT], temperature: f64) -> [f64; KEY_COUNT] {
    let exponent = 1.0 / temperature;
    let mut scaled = [0.0; KEY_COUNT];
    for index in 0..KEY_COUNT {
        scaled[index] = posterior[index].max(1e-12).powf(exponent);
    }
    let sum: f64 = scaled.iter().sum();
    for value in &mut scaled {
        *value /= sum;
    }
    scaled
}

fn fuse(example: &Example, config: FusionConfig) -> ModelOutput {
    let tunelock = temperature_scale(
        example
            .tunelock
            .posterior
            .as_ref()
            .expect("fusion coverage checked"),
        config.tunelock_temperature,
    );
    let external = temperature_scale(
        example
            .external
            .posterior
            .as_ref()
            .expect("fusion coverage checked"),
        config.external_temperature,
    );
    let mut posterior = [0.0; KEY_COUNT];
    for index in 0..KEY_COUNT {
        posterior[index] = (1.0 - config.external_weight) * tunelock[index]
            + config.external_weight * external[index];
    }
    ModelOutput {
        pred: argmax(&posterior),
        posterior: Some(posterior),
    }
}

fn score_config(examples: &[&Example], config: FusionConfig) -> (usize, f64) {
    let mut exact = 0usize;
    let mut mirex = 0.0;
    for example in examples {
        let output = fuse(example, config);
        if output.pred == example.truth {
            exact += 1;
        }
        let (pred_tonic, pred_major) = key_parts(output.pred);
        let (truth_tonic, truth_major) = key_parts(example.truth);
        mirex += mirex_score(pred_tonic, pred_major, truth_tonic, truth_major);
    }
    (exact, mirex)
}

fn select_config(training: &[&Example]) -> FusionConfig {
    const TEMPERATURES: [f64; 6] = [0.5, 0.75, 1.0, 1.25, 1.5, 2.0];
    let mut best = FusionConfig {
        external_weight: 0.5,
        tunelock_temperature: 1.0,
        external_temperature: 1.0,
    };
    let mut best_score = (0usize, f64::NEG_INFINITY);

    for tunelock_temperature in TEMPERATURES {
        for external_temperature in TEMPERATURES {
            for step in 0..=20 {
                let candidate = FusionConfig {
                    external_weight: step as f64 / 20.0,
                    tunelock_temperature,
                    external_temperature,
                };
                let score = score_config(training, candidate);
                if score.0 > best_score.0
                    || (score.0 == best_score.0 && score.1 > best_score.1 + 1e-12)
                {
                    best = candidate;
                    best_score = score;
                }
            }
        }
    }
    best
}

fn stable_fold(id: &str) -> usize {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in id.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash as usize % FOLDS
}

fn evaluate_fusion(examples: &[Example]) -> Option<FusionResult> {
    let covered: Vec<&Example> = examples
        .iter()
        .filter(|example| {
            example.tunelock.posterior.is_some() && example.external.posterior.is_some()
        })
        .collect();
    if covered.is_empty() {
        return None;
    }

    let equal_config = FusionConfig {
        external_weight: 0.5,
        tunelock_temperature: 1.0,
        external_temperature: 1.0,
    };
    let equal_outputs: Vec<(usize, ModelOutput)> = covered
        .iter()
        .map(|example| (example.truth, fuse(example, equal_config)))
        .collect();

    let mut out_of_fold_outputs = Vec::with_capacity(covered.len());
    let mut folds = Vec::new();
    for fold in 0..FOLDS {
        let training: Vec<&Example> = covered
            .iter()
            .copied()
            .filter(|example| stable_fold(&example.id) != fold)
            .collect();
        let testing: Vec<&Example> = covered
            .iter()
            .copied()
            .filter(|example| stable_fold(&example.id) == fold)
            .collect();
        let config = select_config(&training);
        let (test_exact, test_mirex_sum) = score_config(&testing, config);
        out_of_fold_outputs.extend(
            testing
                .iter()
                .map(|example| (example.truth, fuse(example, config))),
        );
        folds.push(FoldResult {
            fold,
            train_n: training.len(),
            test_n: testing.len(),
            config,
            test_exact,
            test_mirex: test_mirex_sum / testing.len().max(1) as f64,
        });
    }

    Some(FusionResult {
        method: "5-fold out-of-fold temperature + convex posterior fusion",
        coverage: covered.len(),
        equal_weight: evaluate(&equal_outputs),
        out_of_fold: evaluate(&out_of_fold_outputs),
        folds,
    })
}

fn complementarity(examples: &[Example]) -> Complementarity {
    let mut both = 0usize;
    let mut tunelock_only = 0usize;
    let mut external_only = 0usize;
    let mut neither = 0usize;
    let mut weighted_oracle = 0.0;
    let mut top3_union = 0usize;
    let mut top3_n = 0usize;

    for example in examples {
        match (
            example.tunelock.pred == example.truth,
            example.external.pred == example.truth,
        ) {
            (true, true) => both += 1,
            (true, false) => tunelock_only += 1,
            (false, true) => external_only += 1,
            (false, false) => neither += 1,
        }
        let (truth_tonic, truth_major) = key_parts(example.truth);
        let tune = key_parts(example.tunelock.pred);
        let ext = key_parts(example.external.pred);
        weighted_oracle += mirex_score(tune.0, tune.1, truth_tonic, truth_major).max(mirex_score(
            ext.0,
            ext.1,
            truth_tonic,
            truth_major,
        ));

        if let (Some(tune_posterior), Some(ext_posterior)) =
            (&example.tunelock.posterior, &example.external.posterior)
        {
            top3_n += 1;
            if rank_contains(tune_posterior, example.truth, 3)
                || rank_contains(ext_posterior, example.truth, 3)
            {
                top3_union += 1;
            }
        }
    }

    let n = examples.len();
    let target = ((0.75 * n as f64).ceil() as usize).min(n);
    let tunelock_exact = both + tunelock_only;
    let additions_needed = target.saturating_sub(tunelock_exact);
    Complementarity {
        n,
        both_correct: both,
        tunelock_only_correct: tunelock_only,
        external_only_correct: external_only,
        neither_correct: neither,
        exact_oracle: both + tunelock_only + external_only,
        exact_oracle_pct: 100.0 * (both + tunelock_only + external_only) as f64 / n.max(1) as f64,
        weighted_oracle_mirex: weighted_oracle / n.max(1) as f64,
        top3_union_pct: (top3_n == n && n > 0).then_some(100.0 * top3_union as f64 / n as f64),
        additions_needed_for_75_pct: additions_needed,
        external_unique_corrections_needed_for_75_pct: additions_needed.min(external_only),
    }
}

fn evaluate_additional_models(
    examples: &[Example],
    primary_model: &str,
    additional: &[LoadedAdditionalModel],
) -> (Vec<AdditionalModelScore>, Option<MultiModelOracle>) {
    if additional.is_empty() {
        return (Vec::new(), None);
    }

    let scores = additional
        .iter()
        .map(|candidate| {
            let pairs: Vec<(usize, ModelOutput)> = examples
                .iter()
                .filter_map(|example| {
                    candidate
                        .outputs
                        .get(&example.id)
                        .copied()
                        .map(|output| (example.truth, output))
                })
                .collect();
            AdditionalModelScore {
                model: candidate.model.clone(),
                revision: candidate.revision.clone(),
                protocol: candidate.protocol.clone(),
                score: evaluate(&pairs),
            }
        })
        .collect();

    let covered: Vec<(&Example, Vec<ModelOutput>)> = examples
        .iter()
        .filter_map(|example| {
            let outputs: Option<Vec<ModelOutput>> = additional
                .iter()
                .map(|candidate| candidate.outputs.get(&example.id).copied())
                .collect();
            outputs.map(|outputs| (example, outputs))
        })
        .collect();
    if covered.is_empty() {
        return (scores, None);
    }

    let mut pair_exact = 0usize;
    let mut multi_exact = 0usize;
    let mut weighted_mirex = 0.0;
    let mut top3_union = 0usize;
    let mut top3_n = 0usize;
    for (example, extra_outputs) in &covered {
        let pair_is_correct =
            example.tunelock.pred == example.truth || example.external.pred == example.truth;
        if pair_is_correct {
            pair_exact += 1;
        }
        if pair_is_correct
            || extra_outputs
                .iter()
                .any(|output| output.pred == example.truth)
        {
            multi_exact += 1;
        }

        let (truth_tonic, truth_major) = key_parts(example.truth);
        let mut outputs = vec![example.tunelock, example.external];
        outputs.extend(extra_outputs.iter().copied());
        weighted_mirex += outputs
            .iter()
            .map(|output| {
                let (tonic, major) = key_parts(output.pred);
                mirex_score(tonic, major, truth_tonic, truth_major)
            })
            .fold(0.0, f64::max);

        if outputs.iter().all(|output| output.posterior.is_some()) {
            top3_n += 1;
            if outputs
                .iter()
                .any(|output| rank_contains(output.posterior.as_ref().unwrap(), example.truth, 3))
            {
                top3_union += 1;
            }
        }
    }

    let n = covered.len();
    let mut models = vec!["TuneLock".to_string(), primary_model.to_string()];
    models.extend(additional.iter().map(|candidate| candidate.model.clone()));
    let oracle = MultiModelOracle {
        models,
        n,
        exact: multi_exact,
        exact_pct: 100.0 * multi_exact as f64 / n as f64,
        added_beyond_primary_pair: multi_exact.saturating_sub(pair_exact),
        weighted_mirex: weighted_mirex / n as f64,
        top3_union_pct: (top3_n == n).then_some(100.0 * top3_union as f64 / n as f64),
    };
    (scores, Some(oracle))
}

fn weight_compositions(model_count: usize, units: usize) -> Vec<Vec<f64>> {
    fn visit(
        slots_left: usize,
        units_left: usize,
        total_units: usize,
        prefix: &mut Vec<f64>,
        output: &mut Vec<Vec<f64>>,
    ) {
        if slots_left == 1 {
            prefix.push(units_left as f64 / total_units as f64);
            output.push(prefix.clone());
            prefix.pop();
            return;
        }
        for units in 0..=units_left {
            prefix.push(units as f64 / total_units as f64);
            visit(
                slots_left - 1,
                units_left - units,
                total_units,
                prefix,
                output,
            );
            prefix.pop();
        }
    }

    let mut output = Vec::new();
    visit(
        model_count,
        units,
        units,
        &mut Vec::with_capacity(model_count),
        &mut output,
    );
    output
}

fn fuse_multi(example: &MultiExample, config: &MultiFusionConfig) -> ModelOutput {
    debug_assert_eq!(example.outputs.len(), config.weights.len());
    let mut posterior = [0.0; KEY_COUNT];
    for (output, weight) in example.outputs.iter().zip(&config.weights) {
        let distribution = output.posterior.as_ref().expect("fusion coverage checked");
        for index in 0..KEY_COUNT {
            posterior[index] += weight * distribution[index];
        }
    }
    ModelOutput {
        pred: argmax(&posterior),
        posterior: Some(posterior),
    }
}

fn score_multi_config(examples: &[&MultiExample], config: &MultiFusionConfig) -> (usize, f64, f64) {
    let mut exact = 0usize;
    let mut mirex = 0.0;
    let mut negative_log_likelihood = 0.0;
    for example in examples {
        let output = fuse_multi(example, config);
        if output.pred == example.truth {
            exact += 1;
        }
        let (pred_tonic, pred_major) = key_parts(output.pred);
        let (truth_tonic, truth_major) = key_parts(example.truth);
        mirex += mirex_score(pred_tonic, pred_major, truth_tonic, truth_major);
        negative_log_likelihood -= output.posterior.unwrap()[example.truth].max(1e-12).ln();
    }
    (exact, mirex, negative_log_likelihood)
}

fn select_multi_config(training: &[&MultiExample], candidates: &[Vec<f64>]) -> MultiFusionConfig {
    let mut best = MultiFusionConfig {
        weights: vec![1.0 / candidates[0].len() as f64; candidates[0].len()],
    };
    let mut best_score = (0usize, f64::NEG_INFINITY, f64::INFINITY);
    for weights in candidates {
        let config = MultiFusionConfig {
            weights: weights.clone(),
        };
        let score = score_multi_config(training, &config);
        if score.0 > best_score.0
            || (score.0 == best_score.0 && score.1 > best_score.1 + 1e-12)
            || (score.0 == best_score.0
                && (score.1 - best_score.1).abs() <= 1e-12
                && score.2 < best_score.2)
        {
            best = config;
            best_score = score;
        }
    }
    best
}

fn evaluate_multi_fusion(
    examples: &[Example],
    primary_model: &str,
    additional: &[LoadedAdditionalModel],
) -> Option<MultiFusionResult> {
    if additional.is_empty() {
        return None;
    }
    let covered: Vec<MultiExample> = examples
        .iter()
        .filter_map(|example| {
            let extras: Option<Vec<ModelOutput>> = additional
                .iter()
                .map(|candidate| candidate.outputs.get(&example.id).copied())
                .collect();
            extras.and_then(|extras| {
                let mut outputs = vec![example.tunelock, example.external];
                outputs.extend(extras);
                outputs
                    .iter()
                    .all(|output| output.posterior.is_some())
                    .then(|| MultiExample {
                        id: example.id.clone(),
                        truth: example.truth,
                        outputs,
                    })
            })
        })
        .collect();
    if covered.is_empty() {
        return None;
    }

    let model_count = covered[0].outputs.len();
    let candidates = weight_compositions(model_count, 20);
    let equal_config = MultiFusionConfig {
        weights: vec![1.0 / model_count as f64; model_count],
    };
    let equal_outputs: Vec<(usize, ModelOutput)> = covered
        .iter()
        .map(|example| (example.truth, fuse_multi(example, &equal_config)))
        .collect();

    let mut out_of_fold_outputs = Vec::with_capacity(covered.len());
    let mut folds = Vec::new();
    for fold in 0..FOLDS {
        let training: Vec<&MultiExample> = covered
            .iter()
            .filter(|example| stable_fold(&example.id) != fold)
            .collect();
        let testing: Vec<&MultiExample> = covered
            .iter()
            .filter(|example| stable_fold(&example.id) == fold)
            .collect();
        let config = select_multi_config(&training, &candidates);
        let (test_exact, test_mirex_sum, _) = score_multi_config(&testing, &config);
        out_of_fold_outputs.extend(
            testing
                .iter()
                .map(|example| (example.truth, fuse_multi(example, &config))),
        );
        folds.push(MultiFusionFold {
            fold,
            train_n: training.len(),
            test_n: testing.len(),
            config,
            test_exact,
            test_mirex: test_mirex_sum / testing.len().max(1) as f64,
        });
    }

    let mut models = vec!["TuneLock".to_string(), primary_model.to_string()];
    models.extend(additional.iter().map(|candidate| candidate.model.clone()));
    Some(MultiFusionResult {
        method: "5-fold out-of-fold convex posterior fusion; weights on a 0.05 simplex grid",
        models,
        coverage: covered.len(),
        equal_weight: evaluate(&equal_outputs),
        out_of_fold: evaluate(&out_of_fold_outputs),
        folds,
    })
}

fn write_stacking_cases(
    path: &str,
    examples: &[Example],
    primary_model: &str,
    additional: &[LoadedAdditionalModel],
) -> Result<()> {
    let canonical_labels = (0..KEY_COUNT)
        .map(|index| {
            let (tonic, major) = key_parts(index);
            format!(
                "{} {}",
                pitch_class_to_name(tonic),
                if major { "major" } else { "minor" }
            )
        })
        .collect();

    let mut records = Vec::new();
    for example in examples {
        let mut named_outputs = vec![
            ("TuneLock".to_string(), example.tunelock),
            (primary_model.to_string(), example.external),
        ];
        let mut missing = false;
        for candidate in additional {
            if let Some(output) = candidate.outputs.get(&example.id).copied() {
                named_outputs.push((candidate.model.clone(), output));
            } else {
                missing = true;
                break;
            }
        }
        if missing
            || named_outputs
                .iter()
                .any(|(_, output)| output.posterior.is_none())
        {
            continue;
        }
        records.push(StackingCase {
            id: example.id.clone(),
            genre: example.genre.clone(),
            fold: stable_fold(&example.id),
            truth_index: example.truth,
            models: named_outputs
                .into_iter()
                .map(|(model, output)| StackingModelOutput {
                    model,
                    pred_index: output.pred,
                    posterior: output.posterior.unwrap(),
                })
                .collect(),
        });
    }

    let dataset = StackingDataset {
        schema_version: 1,
        corpus_role: "development benchmark; truth-bearing stacker cases",
        canonical_labels,
        records,
    };
    std::fs::write(path, serde_json::to_string_pretty(&dataset)?)
        .with_context(|| format!("writing stacker cases to {path}"))?;
    println!("Stacker cases written: {path}");
    Ok(())
}

fn print_report(report: &BakeoffReport) {
    println!(
        "Key posterior bakeoff ({} matched tracks)",
        report.complementarity.n
    );
    println!(
        "  TuneLock: {:>5.1}% exact ({}/{}), MIREX {:.3}, top-3 {}",
        report.tunelock.exact_pct,
        report.tunelock.exact,
        report.tunelock.n,
        report.tunelock.mirex,
        report
            .tunelock
            .top3_pct
            .map(|value| format!("{value:.1}%"))
            .unwrap_or_else(|| "unavailable (report lacks all 24 candidates)".to_string())
    );
    println!(
        "  {}: {:>5.1}% exact ({}/{}), MIREX {:.3}, top-3 {:.1}%",
        report.external_model,
        report.external.exact_pct,
        report.external.exact,
        report.external.n,
        report.external.mirex,
        report.external.top3_pct.unwrap_or(0.0)
    );
    println!(
        "  Exact oracle: {:>5.1}% ({}) = both {} + TuneLock-only {} + external-only {}",
        report.complementarity.exact_oracle_pct,
        report.complementarity.exact_oracle,
        report.complementarity.both_correct,
        report.complementarity.tunelock_only_correct,
        report.complementarity.external_only_correct
    );
    println!(
        "  75% target needs {} net additions; external top-1 offers {} unique corrections",
        report.complementarity.additions_needed_for_75_pct,
        report.complementarity.external_only_correct
    );
    if let Some(fusion) = &report.fusion {
        println!(
            "  Equal posterior blend: {:>5.1}% exact, MIREX {:.3}",
            fusion.equal_weight.exact_pct, fusion.equal_weight.mirex
        );
        println!(
            "  OOF calibrated blend:  {:>5.1}% exact, MIREX {:.3}",
            fusion.out_of_fold.exact_pct, fusion.out_of_fold.mirex
        );
    }
    if let Some(oracle) = &report.multi_model_oracle {
        println!(
            "  Multi-model exact oracle: {:>5.1}% ({}/{}), +{} beyond primary pair",
            oracle.exact_pct, oracle.exact, oracle.n, oracle.added_beyond_primary_pair
        );
    }
    if let Some(fusion) = &report.multi_model_fusion {
        println!(
            "  Multi-model OOF blend:   {:>5.1}% exact, MIREX {:.3}",
            fusion.out_of_fold.exact_pct, fusion.out_of_fold.mirex
        );
    }
}

fn main() -> Result<()> {
    let args = parse_args()?;
    let (external_model, external_revision, external_protocol, external_outputs) =
        load_external(Path::new(&args.model))?;
    let additional_models: Vec<LoadedAdditionalModel> = args
        .additional_models
        .iter()
        .map(|path| {
            let (model, revision, protocol, outputs) = load_external(Path::new(path))?;
            Ok(LoadedAdditionalModel {
                model,
                revision,
                protocol,
                outputs,
            })
        })
        .collect::<Result<_>>()?;
    let tunelock_outputs = load_tunelock(Path::new(&args.tunelock))?;
    let corpus = load_giantsteps(Path::new(&args.giantsteps));
    if corpus.is_empty() {
        bail!("no GiantSteps annotations found under {}", args.giantsteps);
    }

    let mut exclusions = BTreeMap::new();
    let mut examples = Vec::new();
    for row in corpus {
        if row.status != RowStatus::Ready {
            *exclusions
                .entry("corpus_not_ready".to_string())
                .or_insert(0) += 1;
            continue;
        }
        let (Some(tonic), Some(major)) = (row.truth_tonic, row.truth_is_major) else {
            *exclusions.entry("missing_truth".to_string()).or_insert(0) += 1;
            continue;
        };
        let Some(tunelock) = tunelock_outputs.get(&row.title).copied() else {
            *exclusions
                .entry("missing_tunelock".to_string())
                .or_insert(0) += 1;
            continue;
        };
        let Some(external) = external_outputs.get(&row.title).copied() else {
            *exclusions
                .entry("missing_external".to_string())
                .or_insert(0) += 1;
            continue;
        };
        examples.push(Example {
            id: row.title,
            genre: row.genre,
            truth: key_index(tonic, major),
            tunelock,
            external,
        });
    }
    examples.sort_by(|left, right| left.id.cmp(&right.id));
    if examples.is_empty() {
        bail!("no tracks matched across corpus, TuneLock report, and external posterior file");
    }

    let tunelock_pairs: Vec<(usize, ModelOutput)> = examples
        .iter()
        .map(|example| (example.truth, example.tunelock))
        .collect();
    let external_pairs: Vec<(usize, ModelOutput)> = examples
        .iter()
        .map(|example| (example.truth, example.external))
        .collect();
    let (additional_scores, multi_model_oracle) =
        evaluate_additional_models(&examples, &external_model, &additional_models);
    let multi_model_fusion = evaluate_multi_fusion(&examples, &external_model, &additional_models);
    if let Some(path) = args.cases_out.as_deref() {
        write_stacking_cases(path, &examples, &external_model, &additional_models)?;
    }
    let report = BakeoffReport {
        schema_version: 1,
        corpus: args.giantsteps.clone(),
        corpus_role: "development benchmark; not an untouched final test",
        external_model,
        external_revision,
        external_protocol,
        tunelock: evaluate(&tunelock_pairs),
        external: evaluate(&external_pairs),
        complementarity: complementarity(&examples),
        fusion: evaluate_fusion(&examples),
        additional_models: additional_scores,
        multi_model_oracle,
        multi_model_fusion,
        exclusions,
    };

    print_report(&report);
    if let Some(path) = args.out {
        std::fs::write(&path, serde_json::to_string_pretty(&report)?)
            .with_context(|| format!("writing {path}"))?;
        println!("Report written: {path}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_label_order_can_be_canonicalized() {
        let labels = vec![
            "A Major", "Bb Major", "B Major", "C Major", "C# Major", "D Major", "D# Major",
            "E Major", "F Major", "F# Major", "G Major", "G# Major", "B minor", "C minor",
            "C# minor", "D minor", "D# minor", "E minor", "F minor", "F# minor", "G minor",
            "G# minor", "A minor", "Bb minor",
        ]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
        let mut posterior = vec![0.0; KEY_COUNT];
        posterior[3] = 1.0; // S-KEY's C major slot.
        let canonical = canonical_posterior(&labels, &posterior).unwrap();
        assert_eq!(argmax(&canonical), 0); // TuneLock canonical C major.
    }

    #[test]
    fn temperature_scaling_preserves_argmax() {
        let mut posterior = [0.01; KEY_COUNT];
        posterior[17] = 0.77;
        normalize(&mut posterior).unwrap();
        for temperature in [0.5, 0.75, 1.0, 1.5, 2.0] {
            assert_eq!(argmax(&temperature_scale(&posterior, temperature)), 17);
        }
    }

    #[test]
    fn stable_fold_is_repeatable_and_bounded() {
        assert_eq!(stable_fold("1004923.LOFI"), stable_fold("1004923.LOFI"));
        assert!(stable_fold("1004923.LOFI") < FOLDS);
    }
}
