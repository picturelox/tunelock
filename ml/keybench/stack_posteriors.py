#!/usr/bin/env python3
"""Build leakage-safe out-of-fold posteriors from Rust-exported stacker cases.

The input already contains canonical numeric indices produced by TuneLock's
Rust proof layer. This script never parses key names or implements harmonic
relationships. It learns one transposition-invariant candidate ranker shared by
all 24 output positions, and writes OOF predictions back to the labeled JSONL
contract for final scoring in Rust.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
from typing import Any

# Keep the experiment deterministic and runnable in restricted benchmark
# environments where creating worker-process synchronization handles is denied.
for variable in ("OMP_NUM_THREADS", "OPENBLAS_NUM_THREADS", "MKL_NUM_THREADS"):
    os.environ.setdefault(variable, "1")

import numpy as np
import sklearn
from sklearn.ensemble import ExtraTreesClassifier, HistGradientBoostingClassifier
from sklearn.linear_model import LogisticRegression
from sklearn.pipeline import make_pipeline
from sklearn.preprocessing import StandardScaler


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="OOF posterior stacker")
    parser.add_argument("--cases", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument(
        "--method",
        required=True,
        choices=("logistic", "extra-trees", "hist-gradient"),
    )
    parser.add_argument("--seed", type=int, default=42)
    return parser.parse_args()


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def validate_cases(data: dict[str, Any]) -> tuple[list[dict[str, Any]], list[str], list[str]]:
    if data.get("schema_version") != 1:
        raise ValueError(f"Unsupported stacker schema: {data.get('schema_version')}")
    records = data.get("records", [])
    labels = data.get("canonical_labels", [])
    if len(labels) != 24 or not records:
        raise ValueError("Expected 24 labels and at least one stacker case")

    model_names = [item["model"] for item in records[0]["models"]]
    if len(model_names) < 2:
        raise ValueError("Stacking requires at least two posterior sources")
    for record in records:
        if [item["model"] for item in record["models"]] != model_names:
            raise ValueError(f"Model order changed at {record['id']}")
        if not 0 <= int(record["truth_index"]) < 24:
            raise ValueError(f"Truth index out of range at {record['id']}")
        for model in record["models"]:
            posterior = np.asarray(model["posterior"], dtype=np.float64)
            if posterior.shape != (24,) or not np.isfinite(posterior).all():
                raise ValueError(f"Invalid posterior for {record['id']} / {model['model']}")
    return records, labels, model_names


def posterior_ranks(posterior: np.ndarray) -> np.ndarray:
    order = np.argsort(-posterior, kind="stable")
    ranks = np.empty_like(order)
    ranks[order] = np.arange(len(order))
    return ranks.astype(np.float64)


def track_context(posteriors: np.ndarray) -> np.ndarray:
    context: list[float] = []
    winners = np.argmax(posteriors, axis=1)
    for posterior in posteriors:
        ordered = np.sort(posterior)[::-1]
        entropy = -float(np.sum(posterior * np.log(np.maximum(posterior, 1e-12))))
        context.extend(
            [
                float(ordered[0]),
                float(ordered[1]),
                float(ordered[0] - ordered[1]),
                entropy,
            ]
        )
    for left in range(len(winners)):
        for right in range(left + 1, len(winners)):
            context.append(float(winners[left] == winners[right]))
            midpoint = 0.5 * (posteriors[left] + posteriors[right])
            js = 0.5 * np.sum(
                posteriors[left]
                * np.log(np.maximum(posteriors[left], 1e-12) / np.maximum(midpoint, 1e-12))
            ) + 0.5 * np.sum(
                posteriors[right]
                * np.log(np.maximum(posteriors[right], 1e-12) / np.maximum(midpoint, 1e-12))
            )
            context.append(float(js))
    return np.asarray(context, dtype=np.float64)


def candidate_features(
    posteriors: np.ndarray,
    candidate: int,
    genre_index: int,
    genre_count: int,
) -> np.ndarray:
    ranks = np.stack([posterior_ranks(posterior) for posterior in posteriors])
    values = posteriors[:, candidate]
    candidate_ranks = ranks[:, candidate]
    pieces = [
        values,
        np.log(np.maximum(values, 1e-12)),
        candidate_ranks / 23.0,
        1.0 / (candidate_ranks + 1.0),
        (candidate_ranks == 0).astype(np.float64),
        (candidate_ranks < 3).astype(np.float64),
        (candidate_ranks < 5).astype(np.float64),
        np.asarray(
            [
                values.mean(),
                values.max(),
                values.min(),
                values.std(),
                np.sum(candidate_ranks == 0),
                np.sum(candidate_ranks < 3),
                np.sum(candidate_ranks < 5),
            ],
            dtype=np.float64,
        ),
        track_context(posteriors),
    ]
    for left in range(posteriors.shape[0]):
        for right in range(left + 1, posteriors.shape[0]):
            pieces.append(
                np.asarray(
                    [
                        values[left] * values[right],
                        abs(values[left] - values[right]),
                    ],
                    dtype=np.float64,
                )
            )
    genre = np.zeros(genre_count, dtype=np.float64)
    genre[genre_index] = 1.0
    pieces.append(genre)
    return np.concatenate(pieces)


def build_matrix(
    records: list[dict[str, Any]],
) -> tuple[np.ndarray, np.ndarray, np.ndarray, list[str]]:
    genres = sorted({str(record.get("genre", "")).strip().lower() for record in records})
    genre_to_index = {genre: index for index, genre in enumerate(genres)}
    rows: list[np.ndarray] = []
    targets: list[int] = []
    track_indices: list[int] = []
    for track_index, record in enumerate(records):
        posteriors = np.asarray(
            [model["posterior"] for model in record["models"]],
            dtype=np.float64,
        )
        posteriors = np.maximum(posteriors, 0.0)
        posteriors /= np.maximum(posteriors.sum(axis=1, keepdims=True), 1e-12)
        genre_index = genre_to_index[str(record.get("genre", "")).strip().lower()]
        truth = int(record["truth_index"])
        for candidate in range(24):
            rows.append(candidate_features(posteriors, candidate, genre_index, len(genres)))
            targets.append(int(candidate == truth))
            track_indices.append(track_index)
    return (
        np.asarray(rows, dtype=np.float64),
        np.asarray(targets, dtype=np.int64),
        np.asarray(track_indices, dtype=np.int64),
        genres,
    )


def make_estimator(method: str, seed: int):
    if method == "logistic":
        return make_pipeline(
            StandardScaler(),
            LogisticRegression(
                C=0.25,
                class_weight="balanced",
                max_iter=2_000,
                random_state=seed,
            ),
        )
    if method == "extra-trees":
        return ExtraTreesClassifier(
            n_estimators=500,
            max_features=0.7,
            min_samples_leaf=4,
            class_weight="balanced",
            n_jobs=1,
            random_state=seed,
        )
    if method == "hist-gradient":
        return HistGradientBoostingClassifier(
            learning_rate=0.05,
            max_iter=250,
            max_leaf_nodes=15,
            min_samples_leaf=20,
            l2_regularization=1.0,
            class_weight="balanced",
            random_state=seed,
        )
    raise ValueError(method)


def fit_oof(
    records: list[dict[str, Any]],
    features: np.ndarray,
    targets: np.ndarray,
    track_indices: np.ndarray,
    method: str,
    seed: int,
) -> np.ndarray:
    oof = np.zeros((len(records), 24), dtype=np.float64)
    folds = sorted({int(record["fold"]) for record in records})
    if len(folds) < 2:
        raise ValueError("At least two non-empty folds are required")

    for fold in folds:
        test_tracks = np.asarray(
            [index for index, record in enumerate(records) if int(record["fold"]) == fold],
            dtype=np.int64,
        )
        test_mask = np.isin(track_indices, test_tracks)
        train_mask = ~test_mask
        estimator = make_estimator(method, seed + fold)
        estimator.fit(features[train_mask], targets[train_mask])
        positive = estimator.predict_proba(features[test_mask])[:, 1]

        test_row_indices = np.flatnonzero(test_mask)
        for row_index, probability in zip(test_row_indices, positive):
            track_index = int(track_indices[row_index])
            candidate = row_index % 24
            oof[track_index, candidate] = float(probability)
        print(
            f"fold={fold} train_tracks={len(records) - len(test_tracks)} "
            f"test_tracks={len(test_tracks)}",
            flush=True,
        )

    sums = oof.sum(axis=1, keepdims=True)
    if np.any(sums <= 0) or not np.isfinite(oof).all():
        raise ValueError("OOF ranker emitted an invalid score row")
    return oof / sums


def write_jsonl(
    output: Path,
    records: list[dict[str, Any]],
    labels: list[str],
    model_names: list[str],
    posteriors: np.ndarray,
    method: str,
    cases_path: Path,
    genres: list[str],
    seed: int,
) -> None:
    if output.exists():
        raise FileExistsError(f"Refusing to overwrite existing result: {output}")
    output.parent.mkdir(parents=True, exist_ok=True)
    metadata = {
        "type": "metadata",
        "schema_version": 1,
        "model": f"tunelock/oof-candidate-ranker/{method}",
        "model_revision": f"cases-sha256:{sha256(cases_path)}",
        "posterior_labels": labels,
        "protocol": "fixed five-fold out-of-fold development estimate",
        "base_models": model_names,
        "feature_contract": "shared candidate features; no candidate index or key-name parsing",
        "genres": genres,
        "seed": seed,
        "scikit_learn": sklearn.__version__,
    }
    with output.open("w", encoding="utf-8", newline="\n") as handle:
        handle.write(json.dumps(metadata, ensure_ascii=True, separators=(",", ":")) + "\n")
        for record, posterior in zip(records, posteriors):
            predicted = int(np.argmax(posterior))
            item = {
                "type": "prediction",
                "track_id": record["id"],
                "status": "ok",
                "posterior": [round(float(value), 10) for value in posterior.tolist()],
                "predicted_index": predicted,
                "predicted_label": labels[predicted],
                "fold": int(record["fold"]),
            }
            handle.write(json.dumps(item, ensure_ascii=True, separators=(",", ":")) + "\n")


def main() -> int:
    args = parse_args()
    with args.cases.open("r", encoding="utf-8") as handle:
        data = json.load(handle)
    records, labels, model_names = validate_cases(data)
    features, targets, track_indices, genres = build_matrix(records)
    print(
        f"cases={len(records)} candidates={len(targets)} features={features.shape[1]} "
        f"models={model_names} method={args.method}",
        flush=True,
    )
    posteriors = fit_oof(
        records,
        features,
        targets,
        track_indices,
        args.method,
        args.seed,
    )
    exact = sum(
        int(np.argmax(posterior)) == int(record["truth_index"])
        for record, posterior in zip(records, posteriors)
    )
    print(f"numeric exact preview={exact}/{len(records)} ({100 * exact / len(records):.1f}%)")
    write_jsonl(
        args.output,
        records,
        labels,
        model_names,
        posteriors,
        args.method,
        args.cases,
        genres,
        args.seed,
    )
    print(f"OOF posterior written: {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
