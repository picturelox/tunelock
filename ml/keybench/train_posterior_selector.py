#!/usr/bin/env python3
"""Train a production-shaped posterior selector without benchmark leakage.

The selector learns only from the fixed, artist/recording-family-disjoint MTG
validation fold. GiantSteps-key predictions are an application target, never a
training input. Key labels are not parsed here: their ordered numeric contract
comes from the Rust-generated corpus manifest.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
from typing import Any

for variable in ("OMP_NUM_THREADS", "OPENBLAS_NUM_THREADS", "MKL_NUM_THREADS"):
    os.environ.setdefault(variable, "1")

import numpy as np
import sklearn
from sklearn.linear_model import LogisticRegression
from sklearn.model_selection import StratifiedGroupKFold
from sklearn.pipeline import make_pipeline
from sklearn.preprocessing import StandardScaler

from train_myna_head import (
    KEY_COUNT,
    deduplicate_recordings,
    fixed_folds,
    leakage_groups,
)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Train an MTG-only TuneLock/neural posterior selector"
    )
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--training-tunelock", required=True, type=Path)
    parser.add_argument("--training-model", required=True, type=Path)
    parser.add_argument("--development-tunelock", required=True, type=Path)
    parser.add_argument("--development-model", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--artifact", required=True, type=Path)
    parser.add_argument("--report", required=True, type=Path)
    parser.add_argument("--validation-fold", type=int, default=0, choices=range(5))
    parser.add_argument("--seed", type=int, default=42)
    return parser.parse_args()


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def refuse_overwrite(paths: list[Path]) -> None:
    existing = [str(path) for path in paths if path.exists()]
    if existing:
        raise FileExistsError("Refusing to overwrite result(s): " + ", ".join(existing))
    for path in paths:
        path.parent.mkdir(parents=True, exist_ok=True)


def normalized(values: np.ndarray) -> np.ndarray:
    result = np.maximum(np.asarray(values, dtype=np.float64), 0.0)
    if result.shape[-1] != KEY_COUNT or not np.isfinite(result).all():
        raise ValueError(f"Expected finite (..., {KEY_COUNT}) posterior")
    sums = result.sum(axis=-1, keepdims=True)
    if np.any(sums <= 0):
        raise ValueError("Posterior has no positive mass")
    return result / sums


def load_external(path: Path, labels: list[str]) -> tuple[dict[str, Any], dict[str, np.ndarray]]:
    metadata: dict[str, Any] | None = None
    predictions: dict[str, np.ndarray] = {}
    with path.open("r", encoding="utf-8") as handle:
        for line_number, line in enumerate(handle, start=1):
            if not line.strip():
                continue
            item = json.loads(line)
            if item.get("type") == "metadata":
                if metadata is not None:
                    raise ValueError(f"Multiple metadata rows in {path}")
                if item.get("schema_version") != 1:
                    raise ValueError(f"Unsupported external schema in {path}")
                if item.get("posterior_labels") != labels:
                    raise ValueError(
                        f"Posterior label order in {path} does not exactly match the Rust manifest"
                    )
                metadata = item
            elif item.get("type") == "prediction" and item.get("status") == "ok":
                if metadata is None:
                    raise ValueError(f"Metadata must precede predictions in {path}")
                track_id = str(item["track_id"])
                if track_id in predictions:
                    raise ValueError(f"Duplicate external prediction: {track_id}")
                predictions[track_id] = normalized(np.asarray(item["posterior"]))
            else:
                continue
    if metadata is None:
        raise ValueError(f"Missing external metadata in {path}")
    return metadata, predictions


def load_tunelock(path: Path, labels: list[str]) -> dict[str, np.ndarray]:
    label_to_index = {label: index for index, label in enumerate(labels)}
    if len(label_to_index) != KEY_COUNT:
        raise ValueError("Rust manifest contains duplicate canonical labels")
    data = json.loads(path.read_text(encoding="utf-8"))
    predictions: dict[str, np.ndarray] = {}
    for record in data.get("records", []):
        if record.get("failure") is not None:
            continue
        candidates = record.get("candidates", [])
        if len(candidates) != KEY_COUNT:
            continue
        posterior = np.zeros(KEY_COUNT, dtype=np.float64)
        seen: set[int] = set()
        for candidate in candidates:
            label = str(candidate.get("standard", ""))
            if label not in label_to_index:
                raise ValueError(f"TuneLock label is absent from Rust manifest: {label!r}")
            index = label_to_index[label]
            if index in seen:
                raise ValueError(f"Duplicate TuneLock candidate label for {record.get('title')}")
            seen.add(index)
            posterior[index] = float(candidate["confidence"])
        track_id = str(record["title"])
        if track_id in predictions:
            raise ValueError(f"Duplicate TuneLock prediction: {track_id}")
        predictions[track_id] = normalized(posterior)
    return predictions


def posterior_ranks(posterior: np.ndarray) -> np.ndarray:
    order = np.argsort(-posterior, kind="stable")
    ranks = np.empty_like(order)
    ranks[order] = np.arange(KEY_COUNT)
    return ranks.astype(np.float64)


def track_context(posteriors: np.ndarray) -> np.ndarray:
    result: list[float] = []
    winners = np.argmax(posteriors, axis=1)
    for posterior in posteriors:
        ordered = np.sort(posterior)[::-1]
        entropy = -float(np.sum(posterior * np.log(np.maximum(posterior, 1e-12))))
        result.extend((ordered[0], ordered[1], ordered[0] - ordered[1], entropy))
    midpoint = 0.5 * (posteriors[0] + posteriors[1])
    js_divergence = 0.5 * np.sum(
        posteriors[0]
        * np.log(np.maximum(posteriors[0], 1e-12) / np.maximum(midpoint, 1e-12))
    ) + 0.5 * np.sum(
        posteriors[1]
        * np.log(np.maximum(posteriors[1], 1e-12) / np.maximum(midpoint, 1e-12))
    )
    result.extend((float(winners[0] == winners[1]), float(js_divergence)))
    return np.asarray(result, dtype=np.float64)


def candidate_features(posteriors: np.ndarray, candidate: int) -> np.ndarray:
    ranks = np.stack([posterior_ranks(posterior) for posterior in posteriors])
    values = posteriors[:, candidate]
    candidate_ranks = ranks[:, candidate]
    return np.concatenate(
        (
            values,
            np.log(np.maximum(values, 1e-12)),
            candidate_ranks / (KEY_COUNT - 1),
            1.0 / (candidate_ranks + 1.0),
            (candidate_ranks == 0).astype(np.float64),
            (candidate_ranks < 3).astype(np.float64),
            (candidate_ranks < 5).astype(np.float64),
            np.asarray(
                (
                    values.mean(),
                    values.max(),
                    values.min(),
                    values.std(),
                    np.sum(candidate_ranks == 0),
                    np.sum(candidate_ranks < 3),
                    np.sum(candidate_ranks < 5),
                ),
                dtype=np.float64,
            ),
            track_context(posteriors),
            np.asarray((values[0] * values[1], abs(values[0] - values[1]))),
        )
    )


def build_features(pairs: np.ndarray) -> np.ndarray:
    return np.asarray(
        [candidate_features(pair, candidate) for pair in pairs for candidate in range(KEY_COUNT)],
        dtype=np.float64,
    )


def candidate_targets(truth: np.ndarray) -> np.ndarray:
    return np.asarray(
        [int(candidate == target) for target in truth for candidate in range(KEY_COUNT)],
        dtype=np.int64,
    )


def track_posteriors(positive_scores: np.ndarray) -> np.ndarray:
    scores = positive_scores.reshape(-1, KEY_COUNT)
    return normalized(scores)


def make_logistic(c_value: float, seed: int):
    return make_pipeline(
        StandardScaler(),
        LogisticRegression(
            C=c_value,
            class_weight="balanced",
            max_iter=2_000,
            random_state=seed,
        ),
    )


def temperature_scale(posterior: np.ndarray, temperature: float) -> np.ndarray:
    return normalized(np.power(np.maximum(posterior, 1e-12), 1.0 / temperature))


BLEND_CONFIGS = [
    (round(weight / 20.0, 2), tunelock_temperature, model_temperature)
    for weight in range(21)
    for tunelock_temperature in (0.5, 0.75, 1.0, 1.5, 2.0)
    for model_temperature in (0.5, 0.75, 1.0, 1.5, 2.0)
]


def apply_blend(pairs: np.ndarray, config: tuple[float, float, float]) -> np.ndarray:
    weight, tunelock_temperature, model_temperature = config
    left = temperature_scale(pairs[:, 0, :], tunelock_temperature)
    right = temperature_scale(pairs[:, 1, :], model_temperature)
    return normalized((1.0 - weight) * left + weight * right)


def exact_count(posteriors: np.ndarray, truth: np.ndarray) -> int:
    return int(np.sum(np.argmax(posteriors, axis=1) == truth))


def select_blend(pairs: np.ndarray, truth: np.ndarray) -> tuple[float, float, float]:
    best: tuple[int, float, tuple[float, float, float]] | None = None
    for config in BLEND_CONFIGS:
        posterior = apply_blend(pairs, config)
        exact = exact_count(posterior, truth)
        log_likelihood = float(np.log(np.maximum(posterior[np.arange(len(truth)), truth], 1e-12)).sum())
        candidate = (exact, log_likelihood, config)
        if best is None or candidate[:2] > best[:2]:
            best = candidate
    assert best is not None
    return best[2]


def cross_validate(
    pairs: np.ndarray,
    truth: np.ndarray,
    groups: np.ndarray,
    seed: int,
) -> tuple[dict[str, Any], str, float | tuple[float, float, float]]:
    splitter = StratifiedGroupKFold(n_splits=5, shuffle=True, random_state=seed)
    folds = list(splitter.split(np.zeros(len(truth)), truth, groups))
    results: dict[str, Any] = {}

    blend_oof = np.zeros((len(truth), KEY_COUNT), dtype=np.float64)
    blend_fold_configs = []
    for train_indices, test_indices in folds:
        config = select_blend(pairs[train_indices], truth[train_indices])
        blend_fold_configs.append(config)
        blend_oof[test_indices] = apply_blend(pairs[test_indices], config)
    blend_exact = exact_count(blend_oof, truth)
    results["blend"] = {
        "exact": blend_exact,
        "n": len(truth),
        "exact_pct": 100.0 * blend_exact / len(truth),
        "fold_configs": blend_fold_configs,
    }

    features = build_features(pairs)
    targets = candidate_targets(truth)
    logistic_candidates: list[tuple[int, float, np.ndarray]] = []
    for c_value in (0.01, 0.03, 0.1, 0.3, 1.0, 3.0):
        oof = np.zeros((len(truth), KEY_COUNT), dtype=np.float64)
        for fold_index, (train_indices, test_indices) in enumerate(folds):
            train_rows = np.concatenate(
                [np.arange(index * KEY_COUNT, (index + 1) * KEY_COUNT) for index in train_indices]
            )
            test_rows = np.concatenate(
                [np.arange(index * KEY_COUNT, (index + 1) * KEY_COUNT) for index in test_indices]
            )
            estimator = make_logistic(c_value, seed + fold_index)
            estimator.fit(features[train_rows], targets[train_rows])
            oof[test_indices] = track_posteriors(
                estimator.predict_proba(features[test_rows])[:, 1]
            )
        exact = exact_count(oof, truth)
        log_likelihood = float(
            np.log(np.maximum(oof[np.arange(len(truth)), truth], 1e-12)).sum()
        )
        logistic_candidates.append((exact, log_likelihood, oof))
        results[f"logistic_C={c_value:g}"] = {
            "exact": exact,
            "n": len(truth),
            "exact_pct": 100.0 * exact / len(truth),
            "log_likelihood": log_likelihood,
        }

    logistic_index = max(
        range(len(logistic_candidates)),
        key=lambda index: logistic_candidates[index][:2],
    )
    c_values = (0.01, 0.03, 0.1, 0.3, 1.0, 3.0)
    best_c = c_values[logistic_index]
    logistic_exact, logistic_likelihood, _ = logistic_candidates[logistic_index]
    blend_likelihood = float(
        np.log(np.maximum(blend_oof[np.arange(len(truth)), truth], 1e-12)).sum()
    )
    # Exact accuracy decides; likelihood breaks ties; the simple blend wins a
    # perfect tie because it has the smallest production surface.
    if (logistic_exact, logistic_likelihood) > (blend_exact, blend_likelihood):
        return results, "logistic", best_c
    return results, "blend", select_blend(pairs, truth)


def export_logistic(estimator: Any) -> dict[str, Any]:
    scaler: StandardScaler = estimator.named_steps["standardscaler"]
    classifier: LogisticRegression = estimator.named_steps["logisticregression"]
    return {
        "kind": "candidate_logistic_v1",
        "feature_count": int(classifier.coef_.shape[1]),
        "scaler_mean": scaler.mean_.tolist(),
        "scaler_scale": scaler.scale_.tolist(),
        "coefficient": classifier.coef_[0].tolist(),
        "intercept": float(classifier.intercept_[0]),
    }


def main() -> int:
    args = parse_args()
    refuse_overwrite([args.output, args.artifact, args.report])
    manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
    labels = manifest.get("canonical_labels", [])
    raw_records = [
        record for record in manifest.get("records", []) if record.get("role") == "training"
    ]
    records, duplicate_records_removed = deduplicate_recordings(raw_records)
    if manifest.get("schema_version") != 1 or len(labels) != KEY_COUNT or not records:
        raise ValueError("Expected a schema-1 Rust manifest with 24 labels and training rows")

    all_truth = np.asarray([int(record["truth_index"]) for record in records], dtype=np.int64)
    all_groups, _ = leakage_groups(records)
    outer_folds = fixed_folds(all_truth, all_groups)
    selected_indices = np.flatnonzero(outer_folds == args.validation_fold)
    selected_records = [records[index] for index in selected_indices]
    selected_groups = all_groups[selected_indices]

    training_model_meta, training_model = load_external(args.training_model, labels)
    training_tunelock = load_tunelock(args.training_tunelock, labels)
    train_ids = [str(record["id"]) for record in selected_records]
    missing = [
        track_id
        for track_id in train_ids
        if track_id not in training_tunelock or track_id not in training_model
    ]
    if missing:
        raise ValueError(f"Missing training posterior(s) for {len(missing)} held-out tracks")
    train_pairs = np.asarray(
        [[training_tunelock[track_id], training_model[track_id]] for track_id in train_ids]
    )
    train_truth = np.asarray(
        [int(record["truth_index"]) for record in selected_records], dtype=np.int64
    )

    cv_results, selected_method, selected_parameter = cross_validate(
        train_pairs, train_truth, selected_groups, args.seed
    )
    if selected_method == "blend":
        selected_config = selected_parameter
        assert isinstance(selected_config, tuple)
        artifact_model = {
            "kind": "tempered_convex_blend_v1",
            "external_weight": selected_config[0],
            "tunelock_temperature": selected_config[1],
            "external_temperature": selected_config[2],
        }
        predictor = lambda pairs: apply_blend(pairs, selected_config)
    else:
        selected_c = float(selected_parameter)
        features = build_features(train_pairs)
        targets = candidate_targets(train_truth)
        estimator = make_logistic(selected_c, args.seed)
        estimator.fit(features, targets)
        artifact_model = export_logistic(estimator)
        artifact_model["C"] = selected_c
        predictor = lambda pairs: track_posteriors(
            estimator.predict_proba(build_features(pairs))[:, 1]
        )

    development_model_meta, development_model = load_external(args.development_model, labels)
    development_tunelock = load_tunelock(args.development_tunelock, labels)
    development_ids = sorted(set(development_tunelock) & set(development_model))
    if not development_ids:
        raise ValueError("No development predictions overlap")
    development_pairs = np.asarray(
        [[development_tunelock[track_id], development_model[track_id]] for track_id in development_ids]
    )
    development_posteriors = predictor(development_pairs)

    input_hashes = {
        "manifest": sha256(args.manifest),
        "training_tunelock": sha256(args.training_tunelock),
        "training_model": sha256(args.training_model),
        "development_tunelock": sha256(args.development_tunelock),
        "development_model": sha256(args.development_model),
    }
    artifact = {
        "schema_version": 1,
        "model": "tunelock/mtg-posterior-selector",
        "canonical_labels": labels,
        "base_models": ["TuneLock", training_model_meta["model"]],
        "training_protocol": (
            "fixed outer MTG validation fold; nested five-fold stratified recording/artist-group "
            "CV; GiantSteps-key excluded from fitting and method selection"
        ),
        "training_track_count": len(train_ids),
        "outer_validation_fold": args.validation_fold,
        "seed": args.seed,
        "input_sha256": input_hashes,
        "selector": artifact_model,
    }
    args.artifact.write_text(
        json.dumps(artifact, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )

    revision = "artifact-sha256:" + sha256(args.artifact)
    with args.output.open("w", encoding="utf-8", newline="\n") as handle:
        metadata = {
            "type": "metadata",
            "schema_version": 1,
            "model": "tunelock/mtg-posterior-selector",
            "model_revision": revision,
            "posterior_labels": labels,
            "protocol": artifact["training_protocol"],
            "base_model_revision": development_model_meta.get("model_revision"),
        }
        handle.write(json.dumps(metadata, separators=(",", ":")) + "\n")
        for track_id, posterior in zip(development_ids, development_posteriors):
            handle.write(
                json.dumps(
                    {
                        "type": "prediction",
                        "track_id": track_id,
                        "status": "ok",
                        "posterior": [round(float(value), 10) for value in posterior],
                    },
                    separators=(",", ":"),
                )
                + "\n"
            )

    report = {
        "schema_version": 1,
        "selected_method": selected_method,
        "selected_parameter": selected_parameter,
        "training_tracks": len(train_ids),
        "exact_duplicate_records_removed": duplicate_records_removed,
        "development_predictions": len(development_ids),
        "outer_validation_fold": args.validation_fold,
        "component_overlap_with_base_head_training": 0,
        "method_selection": "nested MTG-only cross-validation",
        "giantsteps_used_for_training_or_selection": False,
        "cv_results": cv_results,
        "artifact_sha256": sha256(args.artifact),
        "output_sha256": sha256(args.output),
        "scikit_learn": sklearn.__version__,
    }
    args.report.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    selected_cv = max(item["exact_pct"] for item in cv_results.values())
    print(
        f"training_tracks={len(train_ids)} selected={selected_method} "
        f"best_nested_cv={selected_cv:.1f}% development_predictions={len(development_ids)}"
    )
    print(f"artifact={args.artifact}")
    print(f"posterior={args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
