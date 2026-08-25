#!/usr/bin/env python3
"""Train a multi-model key selector on FMAK development folds.

This selector takes per-track posteriors from all 5 frozen models and learns
to select the correct key using nested artist/recording-family-disjoint OOF.

The selector is a per-candidate logistic regression: for each of the 24 key
candidates, it computes a score from the 5 model posteriors plus derived
features (margins, agreement, entropy, calibration-aware confidence). The
candidate with the highest score is the selector's prediction.

Inputs: JSONL files with per-track 24-key posteriors from each model.
Outputs: Selector artifact JSON + OOF evaluation report.
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
from sklearn.linear_model import LogisticRegression
from sklearn.preprocessing import StandardScaler
from sklearn.pipeline import make_pipeline

KEY_COUNT = 24


def load_jsonl(path: Path) -> dict[str, dict[str, Any]]:
    """Load JSONL posteriors keyed by track ID."""
    records = {}
    with path.open("r", encoding="utf-8") as f:
        for line in f:
            if not line.strip():
                continue
            item = json.loads(line)
            tid = item["id"]
            posterior = np.asarray(item["posterior"], dtype=np.float64)
            if posterior.shape != (KEY_COUNT,):
                raise ValueError(f"Bad posterior shape for {tid}: {posterior.shape}")
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
    """Load classical benchmark JSON and convert to standard format."""
    camelot_to_index = {
        "1A": 20, "2A": 15, "3A": 22, "4A": 17, "5A": 12, "6A": 19,
        "7A": 14, "8A": 21, "9A": 16, "10A": 23, "11A": 18, "12A": 13,
        "1B": 11, "2B": 6, "3B": 1, "4B": 8, "5B": 3, "6B": 10,
        "7B": 5, "8B": 0, "9B": 7, "10B": 2, "11B": 9, "12B": 4,
    }
    data = json.loads(path.read_text(encoding="utf-8"))
    records = {}
    for rec in data["records"]:
        if rec.get("failure"):
            continue
        if not rec.get("candidates"):
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
        truth_camelot = rec.get("truth_camelot", "")
        truth_idx = camelot_to_index.get(truth_camelot, -1)
        if truth_idx < 0:
            continue
        records[tid] = {
            "posterior": posterior,
            "pred_index": int(np.argmax(posterior)),
            "truth_index": truth_idx,
            "truth_label": truth_camelot,
            "artist": rec.get("artist", ""),
            "genre": rec.get("genre", ""),
        }
    return records


def load_folds(manifest_path: Path) -> dict[str, str]:
    """Load fold assignments from the locked manifest."""
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    fold_map = {}
    for r in manifest["records"]:
        if r.get("is_quarantined"):
            continue
        fold = r.get("fold", "")
        if fold and fold != "holdout":
            fold_map[r["id"]] = fold
    return fold_map


def build_features(posteriors: list[np.ndarray]) -> np.ndarray:
    """Build per-candidate features from N model posteriors.

    For each of 24 candidates, features are:
    - Raw posterior from each model (N features)
    - Rank of candidate in each model (N features)
    - Is this the argmax for each model? (N binary features)
    - Margin (posterior - max other) for each model (N features)
    - Entropy of each model's posterior (N features, shared across candidates)

    Total: N*4*24 + N*24 = N*5*24 features per track
    (but we build per-candidate, so it's N*5 features per candidate)
    """
    n_models = len(posteriors)
    features = np.zeros((KEY_COUNT, n_models * 4 + n_models), dtype=np.float64)

    for m_idx, post in enumerate(posteriors):
        ranked = np.argsort(-post)
        ranks = np.zeros(KEY_COUNT)
        for rank, idx in enumerate(ranked):
            ranks[idx] = rank / (KEY_COUNT - 1)

        argmax = int(np.argmax(post))
        is_argmax = np.zeros(KEY_COUNT)
        is_argmax[argmax] = 1.0

        max_val = post[argmax]
        margins = post.copy()
        margins[argmax] = -1.0
        second_max = margins.max()
        margin = post - max_val
        margin[argmax] = max_val - second_max

        entropy = -np.sum(post * np.log(post + 1e-12))

        base = m_idx * 4
        features[:, base + 0] = post
        features[:, base + 1] = ranks
        features[:, base + 2] = is_argmax
        features[:, base + 3] = margin

        # Entropy is shared across all candidates
        features[:, n_models * 4 + m_idx] = entropy

    return features


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
    parser = argparse.ArgumentParser(description="FMAK multi-model selector training")
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--classical-json", type=Path)
    parser.add_argument("--myna-v6-jsonl", type=Path)
    parser.add_argument("--myna-v8-jsonl", type=Path)
    parser.add_argument("--skey-jsonl", type=Path)
    parser.add_argument("--temporal-jsonl", type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--report", required=True, type=Path)
    parser.add_argument("--seed", type=int, default=42)
    args = parser.parse_args()

    # Load fold assignments
    fold_map = load_folds(args.manifest)
    print(f"Fold assignments: {Counter(fold_map.values())}", flush=True)

    # Load model posteriors
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
        print("Need at least 2 models for selector training!")
        return 1

    # Find common track IDs across all models with fold assignments
    common_ids = set(fold_map.keys())
    for name in model_names:
        common_ids &= set(model_data[name].keys())
    common_ids = sorted(common_ids)
    print(f"\nCommon tracks with folds: {len(common_ids)}", flush=True)

    # Build feature matrix and labels
    n_models = len(model_names)
    n_tracks = len(common_ids)

    # Per-candidate features: n_models*4 + n_models per candidate
    n_features_per_candidate = n_models * 4 + n_models
    X = np.zeros((n_tracks, KEY_COUNT * n_features_per_candidate), dtype=np.float64)
    y = np.zeros(n_tracks, dtype=np.int64)
    folds = []
    genres = []

    for i, tid in enumerate(common_ids):
        posteriors = [model_data[name][tid]["posterior"] for name in model_names]
        features = build_features(posteriors)
        X[i] = features.flatten()
        y[i] = model_data[model_names[0]][tid]["truth_index"]
        folds.append(fold_map[tid])
        genres.append(model_data[model_names[0]][tid].get("genre", "unknown"))

    folds = np.array(folds)
    unique_folds = sorted(set(folds))
    print(f"Folds: {unique_folds}", flush=True)

    # Nested OOF: train on 4 folds, evaluate on 1
    fold_results = []
    all_oof_preds = {}
    all_oof_scores = {}

    for test_fold in unique_folds:
        train_mask = folds != test_fold
        test_mask = folds == test_fold

        X_train, y_train = X[train_mask], y[train_mask]
        X_test, y_test = X[test_mask], y[test_mask]
        test_ids = [common_ids[i] for i in range(n_tracks) if test_mask[i]]

        print(f"\n--- Fold {test_fold}: train={train_mask.sum()} test={test_mask.sum()} ---", flush=True)

        # Train per-candidate logistic regression
        # The model takes the full flattened feature vector and predicts
        # which of 24 candidates is correct (multinomial)
        # But we actually want per-candidate scores, so we use a one-vs-rest
        # approach: for each candidate k, train a binary classifier
        # "is candidate k the correct key?"

        # Actually, simpler: treat it as a 24-class classification problem
        # where the input is the flattened feature vector
        # But that's 24 * n_features_per_candidate = 24 * (5*4+5) = 24 * 25 = 600 features
        # With ~4000 training samples, that should be fine

        # Use multinomial logistic regression with regularization
        best_C = 0.1
        best_score = 0
        for C in [0.01, 0.03, 0.1, 0.3, 1.0]:
            clf = make_pipeline(
                StandardScaler(),
                LogisticRegression(C=C, max_iter=2000, solver="lbfgs",
                                 multi_class="multinomial", random_state=args.seed),
            )
            # Quick inner CV: use the training folds themselves
            inner_folds = [f for f in unique_folds if f != test_fold]
            inner_scores = []
            for inner_test in inner_folds:
                inner_train = folds[train_mask] != inner_test
                inner_test_mask = (folds[train_mask] == inner_test)
                if inner_train.sum() < 10 or inner_test_mask.sum() < 10:
                    continue
                clf.fit(X_train[inner_train], y_train[inner_train])
                preds = clf.predict(X_train[inner_test_mask])
                acc = (preds == y_train[inner_test_mask]).mean()
                inner_scores.append(acc)
            if inner_scores:
                avg = np.mean(inner_scores)
                if avg > best_score:
                    best_score = avg
                    best_C = C

        print(f"  Best C: {best_C} (inner CV: {best_score:.3f})", flush=True)

        # Train final model for this fold
        clf = make_pipeline(
            StandardScaler(),
            LogisticRegression(C=best_C, max_iter=2000, solver="lbfgs",
                             multi_class="multinomial", random_state=args.seed),
        )
        clf.fit(X_train, y_train)

        # Predict on test fold
        preds = clf.predict(X_test)
        pred_proba = clf.predict_proba(X_test)

        exact = (preds == y_test).sum()
        total = len(preds)
        mirex = np.mean([mirex_score(int(y_test[i]), int(preds[i])) for i in range(total)])

        print(f"  Exact: {exact}/{total} ({100*exact/total:.1f}%)", flush=True)
        print(f"  MIREX: {mirex:.3f}", flush=True)

        fold_results.append({
            "fold": test_fold,
            "n_train": int(train_mask.sum()),
            "n_test": total,
            "exact": int(exact),
            "exact_pct": 100.0 * exact / total,
            "mirex_mean": float(mirex),
            "best_C": best_C,
        })

        for i, tid in enumerate(test_ids):
            all_oof_preds[tid] = int(preds[i])
            all_oof_scores[tid] = pred_proba[i].tolist()

    # Overall OOF metrics
    all_preds = np.array([all_oof_preds[tid] for tid in common_ids])
    all_truth = np.array([model_data[model_names[0]][tid]["truth_index"] for tid in common_ids])
    overall_exact = (all_preds == all_truth).mean()
    overall_mirex = np.mean([mirex_score(int(t), int(p)) for t, p in zip(all_truth, all_preds)])
    errors = Counter(error_type(int(t), int(p)) for t, p in zip(all_truth, all_preds))

    print(f"\n{'='*60}", flush=True)
    print(f"SELECTOR OOF RESULTS ({n_models} models)", flush=True)
    print(f"{'='*60}", flush=True)
    print(f"  Tracks: {len(common_ids)}", flush=True)
    print(f"  Exact: {overall_exact*100:.1f}%", flush=True)
    print(f"  MIREX: {overall_mirex:.3f}", flush=True)
    print(f"  Errors: {dict(errors)}", flush=True)
    print(f"  Models: {model_names}", flush=True)

    # Compare to best single model
    for name in model_names:
        single_exact = np.mean([
            model_data[name][tid]["pred_index"] == model_data[name][tid]["truth_index"]
            for tid in common_ids
        ])
        print(f"  {name} alone: {single_exact*100:.1f}%", flush=True)

    # By genre
    by_genre = {}
    for i, tid in enumerate(common_ids):
        genre = genres[i]
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
        "experiment": "fmak-frozen-model-selector-v1",
        "models": model_names,
        "n_features_per_candidate": n_features_per_candidate,
        "best_C_per_fold": {r["fold"]: r["best_C"] for r in fold_results},
        "seed": args.seed,
        "manifest_sha256": hashlib.sha256(args.manifest.read_bytes()).hexdigest(),
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(artifact, indent=2), encoding="utf-8")
    print(f"\nArtifact: {args.output}", flush=True)

    # Write report
    report = {
        "overall": {
            "n": len(common_ids),
            "exact_pct": float(overall_exact * 100),
            "mirex_mean": float(overall_mirex),
            "errors": dict(errors),
        },
        "fold_results": fold_results,
        "by_genre": {
            g: {"n": s["n"], "exact_pct": 100.0 * s["exact"] / s["n"]}
            for g, s in by_genre.items()
        },
        "single_model_comparison": {
            name: float(np.mean([
                model_data[name][tid]["pred_index"] == model_data[name][tid]["truth_index"]
                for tid in common_ids
            ]) * 100)
            for name in model_names
        },
        "oof_predictions": {
            tid: {"pred_index": int(all_oof_preds[tid]), "truth_index": int(all_truth[i])}
            for i, tid in enumerate(common_ids)
        },
    }
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, indent=2), encoding="utf-8")
    print(f"Report: {args.report}", flush=True)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
