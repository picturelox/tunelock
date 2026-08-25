#!/usr/bin/env python3
"""Train a per-candidate gradient boosting selector on FMAK development folds.

Instead of multinomial logistic regression on flattened features, this trains
a binary classifier per fold: "Is candidate k the correct key?" The final
prediction is the candidate with the highest score.

This approach:
- Uses leaner features (5 model posteriors + derived per-candidate stats)
- Uses gradient boosting (non-linear, handles interactions naturally)
- Trains per-candidate binary classifiers (simpler learning problem)
- Uses nested OOF with FMAK development folds
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from collections import Counter
from pathlib import Path
from typing import Any

for variable in ("OMP_NUM_THREADS", "OPENBLAS_NUM_THREADS", "MKL_NUM_THREADS"):
    os.environ.setdefault(variable, "1")

import numpy as np
from sklearn.ensemble import GradientBoostingClassifier
from sklearn.linear_model import LogisticRegression
from sklearn.preprocessing import StandardScaler
from sklearn.pipeline import make_pipeline

KEY_COUNT = 24


def load_jsonl(path: Path) -> dict[str, dict[str, Any]]:
    records = {}
    with path.open("r", encoding="utf-8") as f:
        for line in f:
            if not line.strip():
                continue
            item = json.loads(line)
            tid = item["id"]
            posterior = np.asarray(item["posterior"], dtype=np.float64)
            if posterior.shape != (KEY_COUNT,):
                raise ValueError(f"Bad posterior shape for {tid}")
            total = posterior.sum()
            if total > 0:
                posterior = posterior / total
            records[tid] = {
                "posterior": posterior,
                "pred_index": int(np.argmax(posterior)),
                "truth_index": int(item["truth_index"]),
                "truth_label": item["truth_label"],
                "artist": item.get("artist", ""),
                "genre": item.get("genre", ""),
            }
    return records


def load_classical_json(path: Path) -> dict[str, dict[str, Any]]:
    camelot_to_index = {
        "1A": 20, "2A": 15, "3A": 22, "4A": 17, "5A": 12, "6A": 19,
        "7A": 14, "8A": 21, "9A": 16, "10A": 23, "11A": 18, "12A": 13,
        "1B": 11, "2B": 6, "3B": 1, "4B": 8, "5B": 3, "6B": 10,
        "7B": 5, "8B": 0, "9B": 7, "10B": 2, "11B": 9, "12B": 4,
    }
    data = json.loads(path.read_text(encoding="utf-8"))
    records = {}
    for rec in data["records"]:
        if rec.get("failure") or not rec.get("candidates"):
            continue
        posterior = np.zeros(KEY_COUNT, dtype=np.float64)
        for cand in rec["candidates"]:
            idx = camelot_to_index.get(cand["camelot"])
            if idx is not None:
                posterior[idx] = cand["confidence"]
        total = posterior.sum()
        if total > 0:
            posterior = posterior / total
        tid = rec.get("title", "")
        truth_idx = camelot_to_index.get(rec.get("truth_camelot", ""), -1)
        if truth_idx < 0:
            continue
        records[tid] = {
            "posterior": posterior,
            "pred_index": int(np.argmax(posterior)),
            "truth_index": truth_idx,
            "truth_label": rec.get("truth_camelot", ""),
            "artist": rec.get("artist", ""),
            "genre": rec.get("genre", ""),
        }
    return records


def load_folds(manifest_path: Path) -> dict[str, str]:
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    fold_map = {}
    for r in manifest["records"]:
        if r.get("is_quarantined"):
            continue
        fold = r.get("fold", "")
        if fold and fold != "holdout":
            fold_map[r["id"]] = fold
    return fold_map


def build_candidate_features(posteriors: list[np.ndarray], candidate: int) -> np.ndarray:
    """Build features for a single candidate key across all models.

    Features per model (5 models × 6 features = 30):
    - posterior value for this candidate
    - rank of this candidate (0=best, 23=worst)
    - is this the argmax? (binary)
    - margin (posterior - second best if argmax, else posterior - best)
    - log posterior (calibration-aware)
    - posterior entropy (model uncertainty)

    Plus cross-model features (5):
    - mean posterior across models
    - std of posterior across models
    - fraction of models that pick this as argmax
    - max posterior across models
    - min posterior across models
    """
    n_models = len(posteriors)
    features = []

    for post in posteriors:
        ranked = np.argsort(-post)
        rank = np.where(ranked == candidate)[0][0] / (KEY_COUNT - 1)
        is_argmax = 1.0 if int(np.argmax(post)) == candidate else 0.0
        max_val = post.max()
        second = np.sort(post)[-2]
        margin = post[candidate] - max_val if is_argmax else post[candidate] - max_val
        if is_argmax:
            margin = max_val - second
        log_post = np.log(post[candidate] + 1e-12)
        entropy = -np.sum(post * np.log(post + 1e-12))
        features.extend([post[candidate], rank, is_argmax, margin, log_post, entropy])

    # Cross-model features
    cand_posteriors = [post[candidate] for post in posteriors]
    cand_argmax = [1.0 if int(np.argmax(post)) == candidate else 0.0 for post in posteriors]
    features.extend([
        np.mean(cand_posteriors),
        np.std(cand_posteriors),
        np.mean(cand_argmax),
        np.max(cand_posteriors),
        np.min(cand_posteriors),
    ])

    return np.array(features, dtype=np.float64)


def mirex_score(truth: int, predicted: int) -> float:
    if truth == predicted:
        return 1.0
    truth_tonic = truth % 12
    pred_tonic = predicted % 12
    truth_minor = truth >= 12
    pred_minor = predicted >= 12
    if truth_tonic == pred_tonic:
        return 0.6
    if truth_minor == pred_minor and (truth_tonic - pred_tonic + 12) % 12 in (5, 7):
        return 0.5
    if truth_minor != pred_minor and (truth_tonic - pred_tonic + 12) % 12 == (9 if truth_minor else 3):
        return 0.4
    if truth_minor == pred_minor and (truth_tonic - pred_tonic + 12) % 12 in (1, 11):
        return 0.3
    return 0.0


def error_type(truth: int, predicted: int) -> str:
    if truth == predicted:
        return "correct"
    truth_tonic = truth % 12
    pred_tonic = predicted % 12
    truth_minor = truth >= 12
    pred_minor = predicted >= 12
    if truth_tonic == pred_tonic:
        return "parallel"
    if truth_minor == pred_minor and (truth_tonic - pred_tonic + 12) % 12 in (5, 7):
        return "fifth"
    if truth_minor != pred_minor and (truth_tonic - pred_tonic + 12) % 12 == (9 if truth_minor else 3):
        return "relative"
    if truth_minor == pred_minor and (truth_tonic - pred_tonic + 12) % 12 in (1, 11):
        return "semitone"
    return "other"


def main() -> int:
    parser = argparse.ArgumentParser(description="FMAK per-candidate GBM selector")
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--classical-json", type=Path)
    parser.add_argument("--myna-v6-jsonl", type=Path)
    parser.add_argument("--myna-v8-jsonl", type=Path)
    parser.add_argument("--skey-jsonl", type=Path)
    parser.add_argument("--temporal-jsonl", type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--report", required=True, type=Path)
    parser.add_argument("--seed", type=int, default=42)
    parser.add_argument("--model-kind", choices=("gbm", "logistic"), default="gbm")
    args = parser.parse_args()

    fold_map = load_folds(args.manifest)
    print(f"Fold assignments: {Counter(fold_map.values())}", flush=True)

    model_data = {}
    model_names = []

    if args.classical_json and args.classical_json.exists():
        model_data["classical"] = load_classical_json(args.classical_json)
        model_names.append("classical")
        print(f"  classical: {len(model_data['classical'])} records", flush=True)

    for name, path in [
        ("myna_v6", args.myna_v6_jsonl),
        ("myna_v8", args.myna_v8_jsonl),
        ("skey", args.skey_jsonl),
        ("temporal", args.temporal_jsonl),
    ]:
        if path and path.exists():
            model_data[name] = load_jsonl(path)
            model_names.append(name)
            print(f"  {name}: {len(model_data[name])} records", flush=True)

    if len(model_names) < 2:
        print("Need at least 2 models!")
        return 1

    common_ids = set(fold_map.keys())
    for name in model_names:
        common_ids &= set(model_data[name].keys())
    common_ids = sorted(common_ids)
    print(f"\nCommon tracks with folds: {len(common_ids)}", flush=True)

    n_models = len(model_names)
    n_tracks = len(common_ids)

    # Build per-candidate feature matrix
    # Shape: (n_tracks * 24, n_features_per_candidate)
    # Label: 1 if candidate is the correct key, 0 otherwise
    n_features = n_models * 6 + 5
    X_all = np.zeros((n_tracks * KEY_COUNT, n_features), dtype=np.float64)
    y_all = np.zeros(n_tracks * KEY_COUNT, dtype=np.int64)
    track_idx = np.zeros(n_tracks * KEY_COUNT, dtype=np.int64)
    folds_all = []
    genres_all = []

    for i, tid in enumerate(common_ids):
        posteriors = [model_data[name][tid]["posterior"] for name in model_names]
        truth = model_data[model_names[0]][tid]["truth_index"]
        fold = fold_map[tid]
        genre = model_data[model_names[0]][tid].get("genre", "unknown")

        for k in range(KEY_COUNT):
            X_all[i * KEY_COUNT + k] = build_candidate_features(posteriors, k)
            y_all[i * KEY_COUNT + k] = 1 if k == truth else 0
            track_idx[i * KEY_COUNT + k] = i
            folds_all.append(fold)
            genres_all.append(genre)

    folds_all = np.array(folds_all)
    unique_folds = sorted(set(folds_all))
    print(f"Folds: {unique_folds}", flush=True)
    print(f"Feature dim: {n_features}", flush=True)
    print(f"Total candidate rows: {len(X_all)}", flush=True)

    # Nested OOF
    fold_results = []
    all_oof_preds = {}

    for test_fold in unique_folds:
        train_mask = folds_all != test_fold
        test_mask = folds_all == test_fold
        # Get unique test tracks
        test_track_mask = np.zeros(n_tracks, dtype=bool)
        for i in range(n_tracks):
            if fold_map.get(common_ids[i]) == test_fold:
                test_track_mask[i] = True

        X_train, y_train = X_all[train_mask], y_all[train_mask]
        X_test = X_all[test_mask]
        test_track_indices = np.where(test_track_mask)[0]

        n_train_tracks = train_mask.sum() // KEY_COUNT
        n_test_tracks = len(test_track_indices)
        print(f"\n--- Fold {test_fold}: train={n_train_tracks} tracks test={n_test_tracks} tracks ---", flush=True)

        # Train binary classifier: "is this candidate the correct key?"
        if args.model_kind == "gbm":
            clf = GradientBoostingClassifier(
                n_estimators=200,
                max_depth=4,
                learning_rate=0.05,
                subsample=0.8,
                random_state=args.seed,
            )
        else:
            clf = make_pipeline(
                StandardScaler(),
                LogisticRegression(C=0.1, max_iter=2000, solver="lbfgs",
                                 random_state=args.seed),
            )

        clf.fit(X_train, y_train)

        # Score each candidate for each test row
        if hasattr(clf, "predict_proba"):
            scores = clf.predict_proba(X_test)[:, 1]
        else:
            scores = clf.predict_proba(X_test)[:, 1]

        # scores has shape (n_test_candidates,) = (n_test_tracks * 24,)
        # Reshape to (n_test_tracks, 24) and pick argmax per track
        n_test_tracks = len(test_track_indices)
        scores_reshaped = scores[:n_test_tracks * KEY_COUNT].reshape(n_test_tracks, KEY_COUNT)
        preds = scores_reshaped.argmax(axis=1).tolist()

        truth = [model_data[model_names[0]][common_ids[ti]]["truth_index"] for ti in test_track_indices]
        exact = sum(p == t for p, t in zip(preds, truth))
        total = len(preds)
        mirex = np.mean([mirex_score(t, p) for t, p in zip(truth, preds)])

        print(f"  Exact: {exact}/{total} ({100*exact/total:.1f}%)", flush=True)
        print(f"  MIREX: {mirex:.3f}", flush=True)

        fold_results.append({
            "fold": test_fold,
            "n_train_tracks": n_train_tracks,
            "n_test_tracks": n_test_tracks,
            "exact": exact,
            "exact_pct": 100.0 * exact / total,
            "mirex_mean": float(mirex),
        })

        for i, ti in enumerate(test_track_indices):
            all_oof_preds[common_ids[ti]] = preds[i]

    # Overall
    all_preds = np.array([all_oof_preds[tid] for tid in common_ids])
    all_truth = np.array([model_data[model_names[0]][tid]["truth_index"] for tid in common_ids])
    overall_exact = (all_preds == all_truth).mean()
    overall_mirex = np.mean([mirex_score(int(t), int(p)) for t, p in zip(all_truth, all_preds)])
    errors = Counter(error_type(int(t), int(p)) for t, p in zip(all_truth, all_preds))

    print(f"\n{'='*60}", flush=True)
    print(f"SELECTOR V2 OOF RESULTS ({n_models} models, {args.model_kind})", flush=True)
    print(f"{'='*60}", flush=True)
    print(f"  Tracks: {len(common_ids)}", flush=True)
    print(f"  Exact: {overall_exact*100:.1f}%", flush=True)
    print(f"  MIREX: {overall_mirex:.3f}", flush=True)
    print(f"  Errors: {dict(errors)}", flush=True)

    for name in model_names:
        single_exact = np.mean([
            model_data[name][tid]["pred_index"] == model_data[name][tid]["truth_index"]
            for tid in common_ids
        ])
        print(f"  {name} alone: {single_exact*100:.1f}%", flush=True)

    # By genre
    by_genre = {}
    for i, tid in enumerate(common_ids):
        genre = model_data[model_names[0]][tid].get("genre", "unknown")
        if genre not in by_genre:
            by_genre[genre] = {"n": 0, "exact": 0}
        by_genre[genre]["n"] += 1
        if all_preds[i] == all_truth[i]:
            by_genre[genre]["exact"] += 1

    print(f"\n  By genre:", flush=True)
    for genre, stats in sorted(by_genre.items(), key=lambda x: -x[1]["n"]):
        print(f"    {genre:<20} n={stats['n']:<5} exact={100*stats['exact']/stats['n']:.1f}%", flush=True)

    # Write artifact
    artifact = {
        "schema_version": 1,
        "experiment": "fmak-frozen-model-selector-v2",
        "model_kind": args.model_kind,
        "models": model_names,
        "n_features": n_features,
        "seed": args.seed,
        "manifest_sha256": hashlib.sha256(args.manifest.read_bytes()).hexdigest(),
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(artifact, indent=2), encoding="utf-8")
    print(f"\nArtifact: {args.output}", flush=True)

    report = {
        "overall": {
            "n": int(len(common_ids)),
            "exact_pct": float(overall_exact * 100),
            "mirex_mean": float(overall_mirex),
            "errors": {k: int(v) for k, v in errors.items()},
        },
        "fold_results": [
            {**r, "exact": int(r["exact"]), "n_test_tracks": int(r["n_test_tracks"]),
             "n_train_tracks": int(r["n_train_tracks"]),
             "exact_pct": float(r["exact_pct"]), "mirex_mean": float(r["mirex_mean"])}
            for r in fold_results
        ],
        "by_genre": {
            g: {"n": int(s["n"]), "exact_pct": float(100.0 * s["exact"] / s["n"])}
            for g, s in by_genre.items()
        },
        "single_model_comparison": {
            name: float(np.mean([
                model_data[name][tid]["pred_index"] == model_data[name][tid]["truth_index"]
                for tid in common_ids
            ]) * 100)
            for name in model_names
        },
    }
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, indent=2), encoding="utf-8")
    print(f"Report: {args.report}", flush=True)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
