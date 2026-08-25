#!/usr/bin/env python3
"""Confidence calibration study on FMAK.

Tests whether model confidence (posterior mass) correlates with correctness.
Breaks down calibration by genre, error type, and confidence level.
Also tests whether selector confidence is better calibrated than individual models.
"""

from __future__ import annotations

import argparse
import json
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any

import numpy as np

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
            if posterior.sum() > 0:
                posterior = posterior / posterior.sum()
            records[tid] = {
                "posterior": posterior,
                "pred_index": int(np.argmax(posterior)),
                "truth_index": int(item["truth_index"]),
                "genre": item.get("genre", "unknown"),
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
        if posterior.sum() > 0:
            posterior = posterior / posterior.sum()
        tid = rec.get("title", "")
        truth_idx = camelot_to_index.get(rec.get("truth_camelot", ""), -1)
        if truth_idx < 0:
            continue
        records[tid] = {
            "posterior": posterior,
            "pred_index": int(np.argmax(posterior)),
            "truth_index": truth_idx,
            "genre": rec.get("genre", "unknown"),
        }
    return records


def error_type(truth: int, predicted: int) -> str:
    if truth == predicted: return "correct"
    t_t = truth % 12; p_t = predicted % 12
    t_m = truth >= 12; p_m = predicted >= 12
    if t_t == p_t: return "parallel"
    if t_m == p_m and (t_t - p_t + 12) % 12 in (5, 7): return "fifth"
    if t_m != p_m and (t_t - p_t + 12) % 12 == (9 if t_m else 3): return "relative"
    if t_m == p_m and (t_t - p_t + 12) % 12 in (1, 11): return "semitone"
    return "other"


def compute_ece(confidences: np.ndarray, correct: np.ndarray, n_bins: int = 10) -> float:
    ece = 0.0
    for i in range(n_bins):
        lo = i / n_bins; hi = (i + 1) / n_bins
        mask = (confidences >= lo) & (confidences < hi)
        if mask.sum() > 0:
            ece += abs(confidences[mask].mean() - correct[mask].mean()) * mask.sum() / len(confidences)
    return float(ece)


def main() -> int:
    parser = argparse.ArgumentParser(description="FMAK confidence calibration study")
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--classical-json", type=Path)
    parser.add_argument("--skey-jsonl", type=Path)
    parser.add_argument("--myna-v8-jsonl", type=Path)
    parser.add_argument("--fmak-models", type=Path, nargs="+", default=[])
    parser.add_argument("--fmak-names", type=str, nargs="+", default=[])
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()

    models = {}
    if args.classical_json and args.classical_json.exists():
        models["classical"] = load_classical_json(args.classical_json)
    if args.skey_jsonl and args.skey_jsonl.exists():
        models["skey"] = load_jsonl(args.skey_jsonl)
    if args.myna_v8_jsonl and args.myna_v8_jsonl.exists():
        models["myna_v8"] = load_jsonl(args.myna_v8_jsonl)
    fmak_names = args.fmak_names or [f"fmak_{i}" for i in range(len(args.fmak_models))]
    for name, path in zip(fmak_names, args.fmak_models):
        if path.exists():
            models[name] = load_jsonl(path)

    # Find common tracks
    common_ids = set(models[list(models.keys())[0]].keys())
    for name in models:
        common_ids &= set(models[name].keys())
    common_ids = sorted(common_ids)
    print(f"Common tracks: {len(common_ids)}", flush=True)

    # Load genres from manifest
    manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
    genre_map = {}
    for r in manifest["records"]:
        genre_map[r["id"]] = r.get("genre", "unknown")

    results = {}

    for name, data in models.items():
        print(f"\n{'='*60}", flush=True)
        print(f"Model: {name}", flush=True)
        print(f"{'='*60}", flush=True)

        confidences = np.array([data[tid]["posterior"].max() for tid in common_ids])
        preds = np.array([data[tid]["pred_index"] for tid in common_ids])
        truth = np.array([data[tid]["truth_index"] for tid in common_ids])
        correct = (preds == truth).astype(float)
        genres = np.array([genre_map.get(tid, "unknown") for tid in common_ids])

        # Overall ECE
        ece = compute_ece(confidences, correct)
        exact = correct.mean()
        print(f"  Exact: {exact*100:.1f}%", flush=True)
        print(f"  ECE: {ece:.3f}", flush=True)

        # Calibration by confidence bin
        bins = []
        n_bins = 10
        for i in range(n_bins):
            lo = i / n_bins; hi = (i + 1) / n_bins
            mask = (confidences >= lo) & (confidences < hi)
            if mask.sum() > 0:
                bins.append({
                    "range": f"{lo:.1f}-{hi:.1f}",
                    "n": int(mask.sum()),
                    "avg_confidence": float(confidences[mask].mean()),
                    "accuracy": float(correct[mask].mean()),
                    "gap": float(abs(confidences[mask].mean() - correct[mask].mean())),
                })
        print(f"  Calibration bins:", flush=True)
        for b in bins:
            print(f"    {b['range']}: n={b['n']:<5} conf={b['avg_confidence']:.3f} acc={b['accuracy']:.3f} gap={b['gap']:.3f}", flush=True)

        # Calibration by error type
        errors = [error_type(int(t), int(p)) for t, p in zip(truth, preds)]
        by_error = {}
        for et in ["correct", "fifth", "relative", "parallel", "semitone", "other"]:
            mask = np.array([e == et for e in errors])
            if mask.sum() > 0:
                by_error[et] = {
                    "n": int(mask.sum()),
                    "avg_confidence": float(confidences[mask].mean()),
                }
        print(f"  By error type:", flush=True)
        for et, stats in by_error.items():
            print(f"    {et:<12} n={stats['n']:<5} avg_conf={stats['avg_confidence']:.3f}", flush=True)

        # Calibration by genre
        by_genre = {}
        for genre in sorted(set(genres)):
            mask = genres == genre
            if mask.sum() >= 10:
                by_genre[genre] = {
                    "n": int(mask.sum()),
                    "exact_pct": float(correct[mask].mean() * 100),
                    "ece": compute_ece(confidences[mask], correct[mask]),
                    "avg_confidence": float(confidences[mask].mean()),
                }
        print(f"  By genre (n>=10):", flush=True)
        for g, stats in sorted(by_genre.items(), key=lambda x: -x[1]["n"]):
            print(f"    {g:<20} n={stats['n']:<5} exact={stats['exact_pct']:.1f}% ece={stats['ece']:.3f} conf={stats['avg_confidence']:.3f}", flush=True)

        # Confidence threshold analysis: what accuracy at different confidence thresholds?
        thresholds = [0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9]
        threshold_analysis = []
        for thresh in thresholds:
            mask = confidences >= thresh
            if mask.sum() > 0:
                threshold_analysis.append({
                    "threshold": thresh,
                    "n": int(mask.sum()),
                    "coverage": float(mask.sum() / len(confidences)),
                    "accuracy": float(correct[mask].mean()),
                })
        print(f"  Confidence threshold analysis:", flush=True)
        for ta in threshold_analysis:
            print(f"    conf>={ta['threshold']:.1f}: n={ta['n']:<5} coverage={ta['coverage']*100:.1f}% accuracy={ta['accuracy']*100:.1f}%", flush=True)

        results[name] = {
            "exact_pct": float(exact * 100),
            "ece": ece,
            "calibration_bins": bins,
            "by_error_type": by_error,
            "by_genre": by_genre,
            "threshold_analysis": threshold_analysis,
        }

    # === Model agreement as confidence signal ===
    print(f"\n{'='*60}", flush=True)
    print("Model agreement as confidence signal", flush=True)
    print(f"{'='*60}", flush=True)

    # For each track, count how many models agree on the top prediction
    agreement_counts = np.zeros(len(common_ids))
    all_preds = []
    for name in models:
        preds = np.array([models[name][tid]["pred_index"] for tid in common_ids])
        all_preds.append(preds)

    all_preds = np.array(all_preds)  # (n_models, n_tracks)
    for i in range(len(common_ids)):
        votes = all_preds[:, i]
        counts = Counter(votes)
        agreement_counts[i] = counts.most_common(1)[0][1] / len(models)

    truth = np.array([models[list(models.keys())[0]][tid]["truth_index"] for tid in common_ids])
    majority_preds = []
    for i in range(len(common_ids)):
        votes = all_preds[:, i]
        counts = Counter(votes)
        majority_preds.append(counts.most_common(1)[0][0])
    majority_preds = np.array(majority_preds)
    majority_correct = (majority_preds == truth).astype(float)

    print(f"  Majority vote accuracy: {majority_correct.mean()*100:.1f}%", flush=True)

    # Accuracy by agreement level
    for thresh in [1.0, 0.8, 0.6, 0.4, 0.2]:
        if thresh == 1.0:
            mask = agreement_counts == 1.0
        else:
            mask = agreement_counts >= thresh
        if mask.sum() > 0:
            acc = majority_correct[mask].mean()
            print(f"    agreement>={thresh:.1f}: n={mask.sum():<5} coverage={mask.sum()/len(common_ids)*100:.1f}% accuracy={acc*100:.1f}%", flush=True)

    results["majority_vote"] = {
        "exact_pct": float(majority_correct.mean() * 100),
        "n_models": len(models),
    }
    results["agreement_analysis"] = {
        f"agreement_{thresh:.1f}": {
            "n": int(mask.sum()),
            "coverage": float(mask.sum() / len(common_ids)),
            "accuracy": float(majority_correct[mask].mean()),
        }
        for thresh in [1.0, 0.8, 0.6, 0.4, 0.2]
        for mask in [(agreement_counts == 1.0) if thresh == 1.0 else (agreement_counts >= thresh)]
        if mask.sum() > 0
    }

    # Write output
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(results, indent=2), encoding="utf-8")
    print(f"\nReport: {args.output}", flush=True)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
