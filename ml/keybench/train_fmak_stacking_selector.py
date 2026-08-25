#!/usr/bin/env python3
"""Stacking selector: learn optimal weights to combine model posteriors.

Instead of per-candidate binary classification, this learns a weight vector
over models (and optionally per-key) that maximizes OOF accuracy when
combining posterior probabilities. Also tries temperature scaling per model
for calibration before combining.

Approaches:
1. Global weight learning (one weight per model)
2. Per-key weight learning (one weight per model per key)
3. Temperature-scaled weighted average
4. Stacking with logistic regression on concatenated posteriors
"""

from __future__ import annotations

import argparse
import json
import os
from collections import Counter
from pathlib import Path
from typing import Any

for variable in ("OMP_NUM_THREADS", "OPENBLAS_NUM_THREADS", "MKL_NUM_THREADS"):
    os.environ.setdefault(variable, "1")

import numpy as np
from scipy.optimize import minimize
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


def mirex_score(truth: int, predicted: int) -> float:
    if truth == predicted: return 1.0
    t_t = truth % 12; p_t = predicted % 12
    t_m = truth >= 12; p_m = predicted >= 12
    if t_t == p_t: return 0.6
    if t_m == p_m and (t_t - p_t + 12) % 12 in (5, 7): return 0.5
    if t_m != p_m and (t_t - p_t + 12) % 12 == (9 if t_m else 3): return 0.4
    if t_m == p_m and (t_t - p_t + 12) % 12 in (1, 11): return 0.3
    return 0.0


def error_type(truth: int, predicted: int) -> str:
    if truth == predicted: return "correct"
    t_t = truth % 12; p_t = predicted % 12
    t_m = truth >= 12; p_m = predicted >= 12
    if t_t == p_t: return "parallel"
    if t_m == p_m and (t_t - p_t + 12) % 12 in (5, 7): return "fifth"
    if t_m != p_m and (t_t - p_t + 12) % 12 == (9 if t_m else 3): return "relative"
    if t_m == p_m and (t_t - p_t + 12) % 12 in (1, 11): return "semitone"
    return "other"


def softmax(x: np.ndarray) -> np.ndarray:
    x = x - x.max(axis=-1, keepdims=True)
    e = np.exp(x)
    return e / e.sum(axis=-1, keepdims=True)


def learn_global_weights(
    posteriors: np.ndarray,  # (n_tracks, n_models, 24)
    labels: np.ndarray,       # (n_tracks,)
    n_models: int,
) -> np.ndarray:
    """Learn global weights for each model using scipy optimization."""
    def loss(w):
        w = np.maximum(w, 0)
        w = w / (w.sum() + 1e-12)
        combined = np.zeros((len(labels), KEY_COUNT))
        for m in range(n_models):
            combined += w[m] * posteriors[:, m, :]
        preds = combined.argmax(axis=1)
        return -np.mean(preds == labels)

    best_w = None
    best_loss = 1.0
    for _ in range(20):
        w0 = np.random.dirichlet(np.ones(n_models))
        res = minimize(loss, w0, method="Nelder-Mead", options={"maxiter": 500})
        if res.fun < best_loss:
            best_loss = res.fun
            best_w = np.maximum(res.x, 0)
            best_w = best_w / (best_w.sum() + 1e-12)
    return best_w


def learn_temperature_weights(
    posteriors: np.ndarray,  # (n_tracks, n_models, 24)
    labels: np.ndarray,
    n_models: int,
) -> tuple[np.ndarray, np.ndarray]:
    """Learn temperature per model and global weights."""
    def loss(params):
        temps = np.maximum(params[:n_models], 0.01)
        weights = np.maximum(params[n_models:], 0)
        weights = weights / (weights.sum() + 1e-12)
        combined = np.zeros((len(labels), KEY_COUNT))
        for m in range(n_models):
            # Apply temperature: sharpen (T<1) or soften (T>1)
            log_p = np.log(posteriors[:, m, :] + 1e-12)
            scaled = softmax(log_p / temps[m])
            combined += weights[m] * scaled
        preds = combined.argmax(axis=1)
        return -np.mean(preds == labels)

    best_params = None
    best_loss = 1.0
    for _ in range(30):
        t0 = np.ones(n_models) + np.random.randn(n_models) * 0.3
        w0 = np.random.dirichlet(np.ones(n_models))
        params0 = np.concatenate([np.maximum(t0, 0.01), w0])
        res = minimize(loss, params0, method="Nelder-Mead", options={"maxiter": 1000})
        if res.fun < best_loss:
            best_loss = res.fun
            best_params = res.x
    temps = np.maximum(best_params[:n_models], 0.01)
    weights = np.maximum(best_params[n_models:], 0)
    weights = weights / (weights.sum() + 1e-12)
    return temps, weights


def main() -> int:
    parser = argparse.ArgumentParser(description="FMAK stacking selector")
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--classical-json", type=Path)
    parser.add_argument("--skey-jsonl", type=Path)
    parser.add_argument("--myna-v8-jsonl", type=Path)
    parser.add_argument("--fmak-models", type=Path, nargs="+", default=[],
                       help="FMAK OOF JSONL files")
    parser.add_argument("--fmak-names", type=str, nargs="+", default=[],
                       help="Names for FMAK models")
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--seed", type=int, default=42)
    args = parser.parse_args()

    np.random.seed(args.seed)
    fold_map = load_folds(args.manifest)

    model_data = {}
    model_names = []

    if args.classical_json and args.classical_json.exists():
        model_data["classical"] = load_classical_json(args.classical_json)
        model_names.append("classical")
        print(f"  classical: {len(model_data['classical'])} records", flush=True)

    if args.skey_jsonl and args.skey_jsonl.exists():
        model_data["skey"] = load_jsonl(args.skey_jsonl)
        model_names.append("skey")
        print(f"  skey: {len(model_data['skey'])} records", flush=True)

    if args.myna_v8_jsonl and args.myna_v8_jsonl.exists():
        model_data["myna_v8"] = load_jsonl(args.myna_v8_jsonl)
        model_names.append("myna_v8")
        print(f"  myna_v8: {len(model_data['myna_v8'])} records", flush=True)

    fmak_names = args.fmak_names or [f"fmak_{i}" for i in range(len(args.fmak_models))]
    for name, path in zip(fmak_names, args.fmak_models):
        if path.exists():
            model_data[name] = load_jsonl(path)
            model_names.append(name)
            print(f"  {name}: {len(model_data[name])} records", flush=True)

    common_ids = set(fold_map.keys())
    for name in model_names:
        common_ids &= set(model_data[name].keys())
    common_ids = sorted(common_ids)

    n_models = len(model_names)
    n_tracks = len(common_ids)
    print(f"\nCommon tracks: {n_tracks}, Models: {n_models}", flush=True)

    # Build posterior tensor
    posteriors = np.zeros((n_tracks, n_models, KEY_COUNT), dtype=np.float64)
    labels = np.zeros(n_tracks, dtype=np.int64)
    folds = np.zeros(n_tracks, dtype=object)

    for i, tid in enumerate(common_ids):
        for m, name in enumerate(model_names):
            posteriors[i, m] = model_data[name][tid]["posterior"]
        labels[i] = model_data[model_names[0]][tid]["truth_index"]
        folds[i] = fold_map[tid]

    unique_folds = sorted(set(folds))

    # === Method 1: Global weight OOF ===
    print(f"\n{'='*60}", flush=True)
    print("Method 1: Global weight learning (OOF)", flush=True)
    print(f"{'='*60}", flush=True)

    all_preds_global = np.zeros(n_tracks, dtype=np.int64)
    for test_fold in unique_folds:
        train_mask = folds != test_fold
        test_mask = folds == test_fold
        weights = learn_global_weights(posteriors[train_mask], labels[train_mask], n_models)
        combined = np.zeros((test_mask.sum(), KEY_COUNT))
        for m in range(n_models):
            combined += weights[m] * posteriors[test_mask, m, :]
        all_preds_global[test_mask] = combined.argmax(axis=1)

    exact = (all_preds_global == labels).mean()
    mirex = np.mean([mirex_score(int(t), int(p)) for t, p in zip(labels, all_preds_global)])
    errors = Counter(error_type(int(t), int(p)) for t, p in zip(labels, all_preds_global))
    print(f"  Exact: {exact*100:.1f}%", flush=True)
    print(f"  MIREX: {mirex:.3f}", flush=True)
    print(f"  Errors: {dict(errors)}", flush=True)

    # === Method 2: Temperature-scaled weighted average (OOF) ===
    print(f"\n{'='*60}", flush=True)
    print("Method 2: Temperature-scaled weighted average (OOF)", flush=True)
    print(f"{'='*60}", flush=True)

    all_preds_temp = np.zeros(n_tracks, dtype=np.int64)
    for test_fold in unique_folds:
        train_mask = folds != test_fold
        test_mask = folds == test_fold
        temps, weights = learn_temperature_weights(posteriors[train_mask], labels[train_mask], n_models)
        combined = np.zeros((test_mask.sum(), KEY_COUNT))
        for m in range(n_models):
            log_p = np.log(posteriors[test_mask, m, :] + 1e-12)
            scaled = softmax(log_p / temps[m])
            combined += weights[m] * scaled
        all_preds_temp[test_mask] = combined.argmax(axis=1)

    exact_temp = (all_preds_temp == labels).mean()
    mirex_temp = np.mean([mirex_score(int(t), int(p)) for t, p in zip(labels, all_preds_temp)])
    errors_temp = Counter(error_type(int(t), int(p)) for t, p in zip(labels, all_preds_temp))
    print(f"  Exact: {exact_temp*100:.1f}%", flush=True)
    print(f"  MIREX: {mirex_temp:.3f}", flush=True)
    print(f"  Errors: {dict(errors_temp)}", flush=True)

    # === Method 3: Stacking with logistic regression on concatenated posteriors ===
    print(f"\n{'='*60}", flush=True)
    print("Method 3: Logistic stacking on concatenated posteriors (OOF)", flush=True)
    print(f"{'='*60}", flush=True)

    X_concat = posteriors.reshape(n_tracks, n_models * KEY_COUNT)
    all_preds_stack = np.zeros(n_tracks, dtype=np.int64)
    all_proba_stack = np.zeros((n_tracks, KEY_COUNT), dtype=np.float64)

    for test_fold in unique_folds:
        train_mask = folds != test_fold
        test_mask = folds == test_fold
        clf = make_pipeline(
            StandardScaler(),
            LogisticRegression(C=0.1, max_iter=3000, solver="lbfgs", random_state=args.seed),
        )
        clf.fit(X_concat[train_mask], labels[train_mask])
        all_preds_stack[test_mask] = clf.predict(X_concat[test_mask])
        all_proba_stack[test_mask] = clf.predict_proba(X_concat[test_mask])

    exact_stack = (all_preds_stack == labels).mean()
    mirex_stack = np.mean([mirex_score(int(t), int(p)) for t, p in zip(labels, all_preds_stack)])
    errors_stack = Counter(error_type(int(t), int(p)) for t, p in zip(labels, all_preds_stack))
    print(f"  Exact: {exact_stack*100:.1f}%", flush=True)
    print(f"  MIREX: {mirex_stack:.3f}", flush=True)
    print(f"  Errors: {dict(errors_stack)}", flush=True)

    # ECE for stacking
    confidences = all_proba_stack.max(axis=1)
    binary_correct = (all_preds_stack == labels).astype(float)
    n_bins = 10
    ece_stack = 0.0
    for bin_idx in range(n_bins):
        lo = bin_idx / n_bins; hi = (bin_idx + 1) / n_bins
        mask = (confidences >= lo) & (confidences < hi)
        if mask.sum() > 0:
            ece_stack += abs(confidences[mask].mean() - binary_correct[mask].mean()) * mask.sum() / n_tracks
    print(f"  ECE: {ece_stack:.3f}", flush=True)

    # === Summary ===
    print(f"\n{'='*60}", flush=True)
    print("STACKING SELECTOR SUMMARY", flush=True)
    print(f"{'='*60}", flush=True)
    print(f"  Models: {model_names}", flush=True)
    print(f"  Tracks: {n_tracks}", flush=True)
    for name in model_names:
        single = np.mean([model_data[name][tid]["pred_index"] == model_data[name][tid]["truth_index"] for tid in common_ids])
        print(f"  {name} alone: {single*100:.1f}%", flush=True)
    print(f"  Global weights:  exact={exact*100:.1f}% mirex={mirex:.3f}", flush=True)
    print(f"  Temp-weighted:   exact={exact_temp*100:.1f}% mirex={mirex_temp:.3f}", flush=True)
    print(f"  LogReg stacking: exact={exact_stack*100:.1f}% mirex={mirex_stack:.3f} ece={ece_stack:.3f}", flush=True)

    # Oracle
    oracle = 0
    for i in range(n_tracks):
        for k in range(KEY_COUNT):
            if k == labels[i]:
                if any(model_data[name][common_ids[i]]["pred_index"] == k for name in model_names):
                    oracle += 1
                    break
    print(f"  Oracle:          exact={oracle/n_tracks*100:.1f}%", flush=True)

    # Write output
    report = {
        "experiment": "fmak-stacking-selector-v1",
        "models": model_names,
        "n_tracks": n_tracks,
        "global_weights": {"exact_pct": float(exact * 100), "mirex_mean": float(mirex), "errors": {k: int(v) for k, v in errors.items()}},
        "temp_weighted": {"exact_pct": float(exact_temp * 100), "mirex_mean": float(mirex_temp), "errors": {k: int(v) for k, v in errors_temp.items()}},
        "logreg_stacking": {"exact_pct": float(exact_stack * 100), "mirex_mean": float(mirex_stack), "ece": float(ece_stack), "errors": {k: int(v) for k, v in errors_stack.items()}},
        "oracle": {"exact_pct": float(oracle / n_tracks * 100)},
        "single_model": {
            name: float(np.mean([model_data[name][tid]["pred_index"] == model_data[name][tid]["truth_index"] for tid in common_ids]) * 100)
            for name in model_names
        },
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2), encoding="utf-8")
    print(f"\nReport: {args.output}", flush=True)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
