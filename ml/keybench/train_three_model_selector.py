#!/usr/bin/env python3
"""Train TuneLock's lean three-model key selector without benchmark leakage.

The selector consumes one static classical posterior plus two out-of-fold neural
posterior streams. It learns only from the artist/recording-disjoint MTG folds
in the Rust-authored manifest. Development predictions are an application
target; GiantSteps labels are never opened by this program.

Source arguments use ``NAME=PATH``. A neural training model is supplied as five
repeated shards with the same name. A static model may use TuneLock's JSON
benchmark report and is named with ``--static-model``.
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
from sklearn.pipeline import make_pipeline
from sklearn.preprocessing import StandardScaler

from train_myna_head import KEY_COUNT, deduplicate_recordings, fixed_folds, leakage_groups


C_VALUES = (0.003, 0.01, 0.03, 0.1, 0.3)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Leakage-safe three-model posterior selector")
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument(
        "--training-source",
        action="append",
        required=True,
        help="NAME=PATH; repeat NAME for out-of-fold shards",
    )
    parser.add_argument(
        "--development-source",
        action="append",
        required=True,
        help="NAME=PATH; exactly one complete source per model",
    )
    parser.add_argument(
        "--static-model",
        action="append",
        default=[],
        help="Model that is not fitted to MTG and therefore does not need OOF fold markers",
    )
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--artifact", required=True, type=Path)
    parser.add_argument("--report", required=True, type=Path)
    parser.add_argument("--seed", type=int, default=42)
    parser.add_argument(
        "--selector-kind",
        choices=("candidate-logistic", "model-gate"),
        default="candidate-logistic",
    )
    return parser.parse_args()


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def split_spec(value: str) -> tuple[str, Path]:
    name, separator, raw_path = value.partition("=")
    name = name.strip()
    if not separator or not name or not raw_path.strip():
        raise ValueError(f"Expected NAME=PATH source, got {value!r}")
    return name, Path(raw_path.strip())


def normalized(values: np.ndarray) -> np.ndarray:
    result = np.maximum(np.asarray(values, dtype=np.float64), 0.0)
    if result.shape != (KEY_COUNT,) or not np.isfinite(result).all():
        raise ValueError(f"Expected a finite {KEY_COUNT}-value posterior")
    total = float(result.sum())
    if total <= 0:
        raise ValueError("Posterior has no positive mass")
    return result / total


def load_jsonl(
    path: Path, labels: list[str]
) -> tuple[dict[str, Any], dict[str, tuple[np.ndarray, int | None, dict[str, Any]]]]:
    metadata: dict[str, Any] | None = None
    predictions: dict[str, tuple[np.ndarray, int | None, dict[str, Any]]] = {}
    with path.open("r", encoding="utf-8") as handle:
        for line_number, line in enumerate(handle, start=1):
            if not line.strip():
                continue
            item = json.loads(line)
            if item.get("type") == "metadata":
                if metadata is not None:
                    raise ValueError(f"Multiple metadata rows in {path}")
                if item.get("schema_version") != 1 or item.get("posterior_labels") != labels:
                    raise ValueError(f"Posterior contract mismatch in {path}")
                metadata = item
                continue
            if item.get("type") != "prediction" or item.get("status") != "ok":
                continue
            if metadata is None:
                raise ValueError(f"Metadata must precede predictions in {path}:{line_number}")
            track_id = str(item["track_id"])
            if track_id in predictions:
                raise ValueError(f"Duplicate prediction {track_id!r} in {path}")
            fold = item.get("fold")
            predictions[track_id] = (
                normalized(np.asarray(item["posterior"])),
                None if fold is None else int(fold),
                item.get("diagnostics", {}),
            )
    if metadata is None:
        raise ValueError(f"Missing metadata in {path}")
    return metadata, predictions


def load_tunelock_json(
    path: Path, labels: list[str]
) -> tuple[dict[str, Any], dict[str, tuple[np.ndarray, int | None, dict[str, Any]]]]:
    label_to_index = {label: index for index, label in enumerate(labels)}
    data = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(data.get("records"), list):
        raise ValueError(f"Not a TuneLock benchmark report: {path}")
    predictions: dict[str, tuple[np.ndarray, int | None, dict[str, Any]]] = {}
    for record in data["records"]:
        if record.get("failure") is not None:
            continue
        candidates = record.get("candidates", [])
        if len(candidates) != KEY_COUNT:
            continue
        posterior = np.zeros(KEY_COUNT, dtype=np.float64)
        agreements = np.zeros(KEY_COUNT, dtype=np.float64)
        segment_counts = np.zeros(KEY_COUNT, dtype=np.float64)
        seen: set[int] = set()
        for candidate in candidates:
            label = str(candidate.get("standard", ""))
            if label not in label_to_index:
                raise ValueError(f"Unknown TuneLock label {label!r} in {path}")
            index = label_to_index[label]
            if index in seen:
                raise ValueError(f"Duplicate TuneLock label for {record.get('title')}")
            seen.add(index)
            posterior[index] = float(candidate["confidence"])
            agreements[index] = float(candidate.get("agreement", 0.0))
            segment_counts[index] = float(candidate.get("segment_count", 0.0))
        track_id = str(record["title"])
        if track_id in predictions:
            raise ValueError(f"Duplicate TuneLock prediction {track_id!r} in {path}")
        segment_total = max(float(segment_counts.sum()), 1.0)
        predictions[track_id] = (
            normalized(posterior),
            None,
            {
                "classical": {
                    "candidate_agreement": agreements.tolist(),
                    "candidate_segment_fraction": (segment_counts / segment_total).tolist(),
                }
            },
        )
    metadata = {
        "model": "TuneLock classical",
        "model_revision": data.get("analysis_version", "benchmark-report"),
        "protocol": "static deterministic engine; no fitted MTG parameters",
    }
    return metadata, predictions


def load_source(
    path: Path, labels: list[str]
) -> tuple[dict[str, Any], dict[str, tuple[np.ndarray, int | None, dict[str, Any]]]]:
    if path.suffix.lower() == ".jsonl":
        return load_jsonl(path, labels)
    return load_tunelock_json(path, labels)


def merge_sources(
    specs: list[str], labels: list[str]
) -> tuple[
    list[str],
    dict[str, dict[str, tuple[np.ndarray, int | None, dict[str, Any]]]],
    dict[str, Any],
    dict[str, str],
]:
    order: list[str] = []
    merged: dict[str, dict[str, tuple[np.ndarray, int | None, dict[str, Any]]]] = {}
    provenance: dict[str, Any] = {}
    identities: dict[str, tuple[Any, Any]] = {}
    hashes: dict[str, str] = {}
    for raw_spec in specs:
        name, path = split_spec(raw_spec)
        metadata, predictions = load_source(path, labels)
        if metadata.get("model") != name:
            raise ValueError(
                f"Source name {name!r} does not match metadata model {metadata.get('model')!r} "
                f"in {path}"
            )
        if name not in merged:
            order.append(name)
            merged[name] = {}
            provenance[name] = []
            identities[name] = (
                metadata.get("model"),
                metadata.get("head_contract_revision", metadata.get("model_revision")),
            )
        elif identities[name] != (
            metadata.get("model"),
            metadata.get("head_contract_revision", metadata.get("model_revision")),
        ):
            raise ValueError(f"Model identity changed across {name} shards: {path}")
        overlap = sorted(set(merged[name]) & set(predictions))
        if overlap:
            raise ValueError(f"Overlapping shards for {name}: {overlap[0]!r}")
        merged[name].update(predictions)
        provenance[name].append(
            {
                "path": str(path),
                "sha256": sha256(path),
                "model": metadata.get("model"),
                "model_revision": metadata.get("model_revision"),
                "head_contract_revision": metadata.get("head_contract_revision"),
                "protocol": metadata.get("protocol"),
            }
        )
        hashes[str(path)] = sha256(path)
    return order, merged, provenance, hashes


def posterior_ranks(posterior: np.ndarray) -> np.ndarray:
    order = np.argsort(-posterior, kind="stable")
    ranks = np.empty_like(order)
    ranks[order] = np.arange(KEY_COUNT)
    return ranks.astype(np.float64)


def track_context(posteriors: np.ndarray) -> np.ndarray:
    context: list[float] = []
    winners = np.argmax(posteriors, axis=1)
    for posterior in posteriors:
        ordered = np.sort(posterior)[::-1]
        entropy = -float(np.sum(posterior * np.log(np.maximum(posterior, 1e-12))))
        context.extend((ordered[0], ordered[1], ordered[0] - ordered[1], entropy))
    for left in range(len(posteriors)):
        for right in range(left + 1, len(posteriors)):
            midpoint = 0.5 * (posteriors[left] + posteriors[right])
            js = 0.5 * np.sum(
                posteriors[left]
                * np.log(np.maximum(posteriors[left], 1e-12) / np.maximum(midpoint, 1e-12))
            ) + 0.5 * np.sum(
                posteriors[right]
                * np.log(np.maximum(posteriors[right], 1e-12) / np.maximum(midpoint, 1e-12))
            )
            context.extend((float(winners[left] == winners[right]), float(js)))
    context.extend(
        (
            float(len(set(winners.tolist())) == 1),
            float(max(np.bincount(winners, minlength=KEY_COUNT))),
        )
    )
    return np.asarray(context, dtype=np.float64)


def diagnostic_features(diagnostic: dict[str, Any], candidate: int) -> np.ndarray:
    classical = diagnostic.get("classical", {})
    tta = diagnostic.get("tta", {})

    def candidate_value(container: dict[str, Any], field: str) -> float:
        values = container.get(field)
        if not isinstance(values, list) or len(values) != KEY_COUNT:
            return 0.0
        value = float(values[candidate])
        return value if np.isfinite(value) else 0.0

    classical_present = float(bool(classical))
    tta_present = float(bool(tta))
    scalars = []
    for field in ("entropy_mean", "entropy_std", "js_to_mean_mean", "js_to_mean_max"):
        value = float(tta.get(field, 0.0))
        scalars.append(value if np.isfinite(value) else 0.0)
    return np.asarray(
        (
            candidate_value(classical, "candidate_agreement"),
            candidate_value(classical, "candidate_segment_fraction"),
            classical_present,
            candidate_value(tta, "candidate_std"),
            candidate_value(tta, "candidate_min"),
            candidate_value(tta, "candidate_max"),
            candidate_value(tta, "candidate_top1_rate"),
            *scalars,
            tta_present,
        ),
        dtype=np.float64,
    )


def candidate_features(
    posteriors: np.ndarray,
    candidate: int,
    diagnostics: list[dict[str, Any]] | None = None,
) -> np.ndarray:
    ranks = np.stack([posterior_ranks(posterior) for posterior in posteriors])
    values = posteriors[:, candidate]
    candidate_ranks = ranks[:, candidate]
    pieces = [
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
    ]
    for left in range(len(posteriors)):
        for right in range(left + 1, len(posteriors)):
            pieces.append(
                np.asarray(
                    (values[left] * values[right], abs(values[left] - values[right])),
                    dtype=np.float64,
                )
            )
    model_diagnostics = diagnostics or [{} for _ in range(len(posteriors))]
    if len(model_diagnostics) != len(posteriors):
        raise ValueError("Diagnostic model count does not match posterior model count")
    pieces.extend(
        diagnostic_features(diagnostic, candidate) for diagnostic in model_diagnostics
    )
    return np.concatenate(pieces)


def build_features(
    posteriors: np.ndarray,
    diagnostics: list[list[dict[str, Any]]] | None = None,
) -> np.ndarray:
    track_diagnostics = diagnostics or [
        [{} for _ in range(posteriors.shape[1])] for _ in range(len(posteriors))
    ]
    if len(track_diagnostics) != len(posteriors):
        raise ValueError("Diagnostic track count does not match posterior track count")
    return np.asarray(
        [
            candidate_features(track, candidate, track_diagnostics[track_index])
            for track_index, track in enumerate(posteriors)
            for candidate in range(KEY_COUNT)
        ],
        dtype=np.float64,
    )


def candidate_targets(truth: np.ndarray) -> np.ndarray:
    return np.asarray(
        [int(candidate == target) for target in truth for candidate in range(KEY_COUNT)],
        dtype=np.int64,
    )


def make_estimator(c_value: float, seed: int):
    return make_pipeline(
        StandardScaler(),
        LogisticRegression(
            C=c_value,
            class_weight="balanced",
            max_iter=2_000,
            random_state=seed,
        ),
    )


def predict_tracks(estimator: Any, features: np.ndarray, track_indices: np.ndarray) -> np.ndarray:
    rows = np.concatenate(
        [np.arange(index * KEY_COUNT, (index + 1) * KEY_COUNT) for index in track_indices]
    )
    positive = estimator.predict_proba(features[rows])[:, 1].reshape(-1, KEY_COUNT)
    sums = positive.sum(axis=1, keepdims=True)
    if np.any(sums <= 0) or not np.isfinite(positive).all():
        raise ValueError("Selector emitted an invalid posterior")
    return positive / sums


def fit_for_tracks(
    features: np.ndarray,
    targets: np.ndarray,
    track_indices: np.ndarray,
    c_value: float,
    seed: int,
) -> Any:
    rows = np.concatenate(
        [np.arange(index * KEY_COUNT, (index + 1) * KEY_COUNT) for index in track_indices]
    )
    estimator = make_estimator(c_value, seed)
    estimator.fit(features[rows], targets[rows])
    return estimator


def score(posteriors: np.ndarray, truth: np.ndarray) -> tuple[int, float]:
    exact = int(np.sum(np.argmax(posteriors, axis=1) == truth))
    nll = -float(np.log(np.maximum(posteriors[np.arange(len(truth)), truth], 1e-12)).sum())
    return exact, nll


def choose_c(
    features: np.ndarray,
    targets: np.ndarray,
    truth: np.ndarray,
    folds: np.ndarray,
    eligible_folds: list[int],
    seed: int,
) -> tuple[float, list[dict[str, Any]]]:
    results: list[dict[str, Any]] = []
    for c_value in C_VALUES:
        exact_total = 0
        nll_total = 0.0
        for inner_fold in eligible_folds:
            train_indices = np.flatnonzero(
                np.isin(folds, [fold for fold in eligible_folds if fold != inner_fold])
            )
            test_indices = np.flatnonzero(folds == inner_fold)
            estimator = fit_for_tracks(
                features, targets, train_indices, c_value, seed + inner_fold
            )
            posterior = predict_tracks(estimator, features, test_indices)
            exact, nll = score(posterior, truth[test_indices])
            exact_total += exact
            nll_total += nll
        results.append({"C": c_value, "exact": exact_total, "nll": nll_total})
    selected = max(results, key=lambda item: (item["exact"], -item["nll"], -item["C"]))
    return float(selected["C"]), results


def nested_oof(
    features: np.ndarray,
    targets: np.ndarray,
    truth: np.ndarray,
    folds: np.ndarray,
    seed: int,
) -> tuple[np.ndarray, list[dict[str, Any]]]:
    fold_values = sorted({int(value) for value in folds.tolist()})
    if len(fold_values) != 5:
        raise ValueError("The selector proof contract requires exactly five fixed folds")
    oof = np.zeros((len(truth), KEY_COUNT), dtype=np.float64)
    reports: list[dict[str, Any]] = []
    for outer_fold in fold_values:
        eligible = [fold for fold in fold_values if fold != outer_fold]
        c_value, inner_results = choose_c(
            features, targets, truth, folds, eligible, seed + 100 * outer_fold
        )
        train_indices = np.flatnonzero(folds != outer_fold)
        test_indices = np.flatnonzero(folds == outer_fold)
        estimator = fit_for_tracks(
            features, targets, train_indices, c_value, seed + outer_fold
        )
        posterior = predict_tracks(estimator, features, test_indices)
        oof[test_indices] = posterior
        exact, nll = score(posterior, truth[test_indices])
        reports.append(
            {
                "fold": outer_fold,
                "train_tracks": len(train_indices),
                "test_tracks": len(test_indices),
                "selected_C": c_value,
                "test_exact": exact,
                "test_nll": nll,
                "inner_results": inner_results,
            }
        )
    return oof, reports


def build_gate_features(
    posteriors: np.ndarray,
    diagnostics: list[list[dict[str, Any]]],
) -> np.ndarray:
    rows: list[np.ndarray] = []
    model_count = posteriors.shape[1]
    for track_index, track in enumerate(posteriors):
        for model_index in range(model_count):
            winner = int(np.argmax(track[model_index]))
            identity = np.zeros(model_count, dtype=np.float64)
            identity[model_index] = 1.0
            rows.append(
                np.concatenate(
                    (
                        candidate_features(track, winner, diagnostics[track_index]),
                        identity,
                    )
                )
            )
    return np.asarray(rows, dtype=np.float64)


def gate_targets(posteriors: np.ndarray, truth: np.ndarray) -> np.ndarray:
    winners = np.argmax(posteriors, axis=2)
    return (winners == truth[:, np.newaxis]).astype(np.int64).reshape(-1)


def fit_gate(
    features: np.ndarray,
    targets: np.ndarray,
    track_indices: np.ndarray,
    model_count: int,
    c_value: float,
    seed: int,
) -> Any:
    rows = np.concatenate(
        [
            np.arange(index * model_count, (index + 1) * model_count)
            for index in track_indices
        ]
    )
    estimator = make_estimator(c_value, seed)
    estimator.fit(features[rows], targets[rows])
    return estimator


def predict_gate(
    estimator: Any,
    features: np.ndarray,
    track_indices: np.ndarray,
    posteriors: np.ndarray,
    strategy: str,
) -> np.ndarray:
    model_count = posteriors.shape[1]
    rows = np.concatenate(
        [
            np.arange(index * model_count, (index + 1) * model_count)
            for index in track_indices
        ]
    )
    reliability = estimator.predict_proba(features[rows])[:, 1].reshape(-1, model_count)
    selected_posteriors = posteriors[track_indices]
    if strategy == "hard":
        chosen = np.argmax(reliability, axis=1)
        return selected_posteriors[np.arange(len(chosen)), chosen]
    if strategy == "soft":
        weights = reliability / np.maximum(reliability.sum(axis=1, keepdims=True), 1e-12)
        return np.sum(selected_posteriors * weights[:, :, np.newaxis], axis=1)
    raise ValueError(strategy)


def choose_gate_config(
    features: np.ndarray,
    targets: np.ndarray,
    posteriors: np.ndarray,
    truth: np.ndarray,
    folds: np.ndarray,
    eligible_folds: list[int],
    seed: int,
) -> tuple[tuple[float, str], list[dict[str, Any]]]:
    results: list[dict[str, Any]] = []
    model_count = posteriors.shape[1]
    for c_value in C_VALUES:
        for strategy in ("hard", "soft"):
            exact_total = 0
            nll_total = 0.0
            for inner_fold in eligible_folds:
                train_indices = np.flatnonzero(
                    np.isin(folds, [fold for fold in eligible_folds if fold != inner_fold])
                )
                test_indices = np.flatnonzero(folds == inner_fold)
                estimator = fit_gate(
                    features,
                    targets,
                    train_indices,
                    model_count,
                    c_value,
                    seed + inner_fold,
                )
                posterior = predict_gate(
                    estimator, features, test_indices, posteriors, strategy
                )
                exact, nll = score(posterior, truth[test_indices])
                exact_total += exact
                nll_total += nll
            results.append(
                {
                    "C": c_value,
                    "strategy": strategy,
                    "exact": exact_total,
                    "nll": nll_total,
                }
            )
    selected = max(
        results,
        key=lambda item: (
            item["exact"],
            -item["nll"],
            item["strategy"] == "hard",
            -item["C"],
        ),
    )
    return (float(selected["C"]), str(selected["strategy"])), results


def nested_gate_oof(
    features: np.ndarray,
    targets: np.ndarray,
    posteriors: np.ndarray,
    truth: np.ndarray,
    folds: np.ndarray,
    seed: int,
) -> tuple[np.ndarray, list[dict[str, Any]]]:
    fold_values = sorted({int(value) for value in folds.tolist()})
    if len(fold_values) != 5:
        raise ValueError("The selector proof contract requires exactly five fixed folds")
    oof = np.zeros((len(truth), KEY_COUNT), dtype=np.float64)
    reports: list[dict[str, Any]] = []
    model_count = posteriors.shape[1]
    for outer_fold in fold_values:
        eligible = [fold for fold in fold_values if fold != outer_fold]
        (c_value, strategy), inner_results = choose_gate_config(
            features, targets, posteriors, truth, folds, eligible, seed + 100 * outer_fold
        )
        train_indices = np.flatnonzero(folds != outer_fold)
        test_indices = np.flatnonzero(folds == outer_fold)
        estimator = fit_gate(
            features, targets, train_indices, model_count, c_value, seed + outer_fold
        )
        posterior = predict_gate(estimator, features, test_indices, posteriors, strategy)
        oof[test_indices] = posterior
        exact, nll = score(posterior, truth[test_indices])
        reports.append(
            {
                "fold": outer_fold,
                "train_tracks": len(train_indices),
                "test_tracks": len(test_indices),
                "selected_C": c_value,
                "selected_strategy": strategy,
                "test_exact": exact,
                "test_nll": nll,
                "inner_results": inner_results,
            }
        )
    return oof, reports


def export_estimator(estimator: Any) -> dict[str, Any]:
    scaler: StandardScaler = estimator.named_steps["standardscaler"]
    classifier: LogisticRegression = estimator.named_steps["logisticregression"]
    return {
        "kind": "candidate_logistic_v2",
        "feature_count": int(classifier.coef_.shape[1]),
        "scaler_mean": scaler.mean_.tolist(),
        "scaler_scale": scaler.scale_.tolist(),
        "coefficient": classifier.coef_[0].tolist(),
        "intercept": float(classifier.intercept_[0]),
    }


def write_predictions(
    path: Path,
    labels: list[str],
    model_names: list[str],
    posteriors: np.ndarray,
    track_ids: list[str],
    artifact_hash: str,
) -> None:
    with path.open("w", encoding="utf-8", newline="\n") as handle:
        handle.write(
            json.dumps(
                {
                    "type": "metadata",
                    "schema_version": 1,
                    "model": "tunelock/three-model-key-selector",
                    "model_revision": f"artifact-sha256:{artifact_hash}",
                    "posterior_labels": labels,
                    "protocol": (
                        "selector fit only on MTG out-of-fold/static posteriors; "
                        "GiantSteps labels excluded from fitting and selection"
                    ),
                    "base_models": model_names,
                },
                separators=(",", ":"),
            )
            + "\n"
        )
        for track_id, posterior in zip(track_ids, posteriors):
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


def main() -> int:
    args = parse_args()
    outputs = [args.output, args.artifact, args.report]
    existing = [str(path) for path in outputs if path.exists()]
    if existing:
        raise FileExistsError("Refusing to overwrite result(s): " + ", ".join(existing))
    for path in outputs:
        path.parent.mkdir(parents=True, exist_ok=True)

    manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
    labels = manifest.get("canonical_labels", [])
    raw_records = [
        record for record in manifest.get("records", []) if record.get("role") == "training"
    ]
    records, duplicates_removed = deduplicate_recordings(raw_records)
    if manifest.get("schema_version") != 1 or len(labels) != KEY_COUNT or not records:
        raise ValueError("Expected a schema-1 Rust manifest with 24 labels and training rows")
    truth = np.asarray([int(record["truth_index"]) for record in records], dtype=np.int64)
    groups, _ = leakage_groups(records)
    folds = fixed_folds(truth, groups)
    track_ids = [str(record["id"]) for record in records]

    train_order, training, training_provenance, training_hashes = merge_sources(
        args.training_source, labels
    )
    development_order, development, development_provenance, development_hashes = merge_sources(
        args.development_source, labels
    )
    if len(train_order) != 3 or train_order != development_order:
        raise ValueError(
            "Exactly three models are required and training/development order must match; "
            f"got training={train_order}, development={development_order}"
        )
    static_models = set(args.static_model)
    if not static_models <= set(train_order):
        raise ValueError(f"Unknown static model(s): {sorted(static_models - set(train_order))}")
    for model_name in set(train_order) - static_models:
        training_contracts = {
            item.get("head_contract_revision") for item in training_provenance[model_name]
        }
        development_contracts = {
            item.get("head_contract_revision") for item in development_provenance[model_name]
        }
        if None in training_contracts or training_contracts != development_contracts:
            raise ValueError(
                f"Training/development head contract mismatch for {model_name}: "
                f"training={training_contracts}, development={development_contracts}"
            )

    training_tensor = np.zeros((len(records), 3, KEY_COUNT), dtype=np.float64)
    training_diagnostics: list[list[dict[str, Any]]] = [
        [{} for _ in train_order] for _ in records
    ]
    for track_index, (track_id, expected_fold) in enumerate(zip(track_ids, folds.tolist())):
        for model_index, model_name in enumerate(train_order):
            if track_id not in training[model_name]:
                raise ValueError(f"Missing training prediction for {model_name}/{track_id}")
            posterior, actual_fold, diagnostics = training[model_name][track_id]
            if model_name not in static_models and actual_fold != int(expected_fold):
                raise ValueError(
                    f"OOF fold mismatch for {model_name}/{track_id}: "
                    f"expected {expected_fold}, got {actual_fold}"
                )
            training_tensor[track_index, model_index] = posterior
            training_diagnostics[track_index][model_index] = diagnostics

    development_ids = sorted(set.intersection(*(set(development[name]) for name in train_order)))
    if not development_ids:
        raise ValueError("Development sources have no common tracks")
    development_tensor = np.asarray(
        [
            [development[name][track_id][0] for name in train_order]
            for track_id in development_ids
        ],
        dtype=np.float64,
    )
    development_diagnostics = [
        [development[name][track_id][2] for name in train_order]
        for track_id in development_ids
    ]

    all_indices = np.arange(len(records), dtype=np.int64)
    final_strategy: str | None = None
    if args.selector_kind == "candidate-logistic":
        features = build_features(training_tensor, training_diagnostics)
        targets = candidate_targets(truth)
        oof, fold_reports = nested_oof(features, targets, truth, folds, args.seed)
        final_c, final_cv = choose_c(
            features, targets, truth, folds, sorted(set(folds.tolist())), args.seed + 10_000
        )
        estimator = fit_for_tracks(features, targets, all_indices, final_c, args.seed)
        development_posteriors = predict_tracks(
            estimator,
            build_features(development_tensor, development_diagnostics),
            np.arange(len(development_ids)),
        )
        feature_contract = (
            "transposition-invariant shared-candidate v2: per-model posterior/rank/top-k, "
            "aggregate agreement, entropy, pairwise Jensen-Shannon, candidate interactions, "
            "classical section evidence and neural TTA stability"
        )
    else:
        features = build_gate_features(training_tensor, training_diagnostics)
        targets = gate_targets(training_tensor, truth)
        oof, fold_reports = nested_gate_oof(
            features, targets, training_tensor, truth, folds, args.seed
        )
        (final_c, final_strategy), final_cv = choose_gate_config(
            features,
            targets,
            training_tensor,
            truth,
            folds,
            sorted(set(folds.tolist())),
            args.seed + 10_000,
        )
        estimator = fit_gate(
            features, targets, all_indices, training_tensor.shape[1], final_c, args.seed
        )
        development_posteriors = predict_gate(
            estimator,
            build_gate_features(development_tensor, development_diagnostics),
            np.arange(len(development_ids)),
            development_tensor,
            final_strategy,
        )
        feature_contract = (
            "transposition-invariant model-gate v1: each model winner's shared-candidate "
            "support, posterior agreement, classical section evidence, neural TTA stability, "
            "and fixed model identity"
        )
    oof_exact, oof_nll = score(oof, truth)

    base_exact = [
        int(np.sum(np.argmax(training_tensor[:, index], axis=1) == truth)) for index in range(3)
    ]
    oracle = int(
        np.sum(
            np.any(np.argmax(training_tensor, axis=2) == truth[:, np.newaxis], axis=1)
        )
    )
    artifact = {
        "schema_version": 1,
        "model": "tunelock/three-model-key-selector",
        "canonical_labels": labels,
        "base_models": train_order,
        "training_protocol": (
            "five fixed artist/recording-group MTG folds; non-static base predictions are "
            "strictly out-of-fold; nested selector CV; GiantSteps labels excluded"
        ),
        "training_tracks": len(records),
        "static_models": sorted(static_models),
        "selector_kind": args.selector_kind,
        "selected_C": final_c,
        "selected_strategy": final_strategy,
        "feature_contract": feature_contract,
        "selector": {
            **export_estimator(estimator),
            "kind": (
                "candidate_logistic_v2"
                if args.selector_kind == "candidate-logistic"
                else "track_model_gate_logistic_v1"
            ),
        },
        "input_sha256": {
            "manifest": sha256(args.manifest),
            **training_hashes,
            **development_hashes,
        },
    }
    args.artifact.write_text(
        json.dumps(artifact, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    artifact_hash = sha256(args.artifact)
    write_predictions(
        args.output,
        labels,
        train_order,
        development_posteriors,
        development_ids,
        artifact_hash,
    )
    report = {
        "schema_version": 1,
        "artifact_sha256": artifact_hash,
        "output_sha256": sha256(args.output),
        "giantsteps_used_for_training_or_selection": False,
        "training_tracks": len(records),
        "exact_duplicate_records_removed": duplicates_removed,
        "base_models": [
            {"name": name, "oof_exact": exact, "oof_exact_pct": 100.0 * exact / len(records)}
            for name, exact in zip(train_order, base_exact)
        ],
        "three_model_oof_oracle": oracle,
        "three_model_oof_oracle_pct": 100.0 * oracle / len(records),
        "selector_nested_oof_exact": oof_exact,
        "selector_nested_oof_exact_pct": 100.0 * oof_exact / len(records),
        "selector_nested_oof_nll": oof_nll,
        "outer_folds": fold_reports,
        "selector_kind": args.selector_kind,
        "final_C": final_c,
        "final_strategy": final_strategy,
        "final_C_cross_validation": final_cv,
        "development_predictions": len(development_ids),
        "training_provenance": training_provenance,
        "development_provenance": development_provenance,
        "scikit_learn": sklearn.__version__,
    }
    args.report.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(
        f"models={train_order} training={len(records)} oracle={oracle}/{len(records)} "
        f"kind={args.selector_kind} nested_oof={oof_exact}/{len(records)} "
        f"final_C={final_c} strategy={final_strategy} "
        f"development={len(development_ids)}",
        flush=True,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
