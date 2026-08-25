#!/usr/bin/env python3
"""Comprehensive frozen-model FMAK benchmark analysis.

Loads 24-key posteriors from all available models and computes:
  - exact accuracy, MIREX accuracy, top-3/top-5
  - calibration (ECE, reliability)
  - error types
  - genre breakdown
  - per-key confusion
  - model overlap (agreement, correlation)
  - 2/3/4/5-model oracle ceilings

Inputs are JSONL files with per-track 24-key posteriors.
Outputs a summary JSON and a human-readable report.
"""

from __future__ import annotations

import argparse
import json
import math
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any

import numpy as np


def load_jsonl(path: Path) -> list[dict[str, Any]]:
    records = []
    with path.open("r", encoding="utf-8") as f:
        for line in f:
            if line.strip():
                records.append(json.loads(line))
    return records


def load_classical_json(path: Path) -> list[dict[str, Any]]:
    """Convert the Rust bench tool's JSON output to the standard JSONL format."""
    data = json.loads(path.read_text(encoding="utf-8"))
    # Build a label-to-index map from the canonical labels
    # The bench tool uses Camelot codes; we need to map them to 0-23
    # The candidates are already sorted by confidence (highest first)
    # We need to construct a 24-element posterior from the candidates
    # The bench tool outputs all 24 candidates with confidence values
    # Standard Camelot wheel matching Rust harmony/mod.rs:
    # 8B=C major, 9B=G major, 10B=D major, 11B=A major, 12B=E major,
    # 1B=B major, 2B=F# major, 3B=C# major, 4B=G# major, 5B=D# major,
    # 6B=A# major, 7B=F major
    # Minor keys share the number with their relative major.
    camelot_to_index = {
        "1A": 20, "2A": 15, "3A": 22, "4A": 17, "5A": 12, "6A": 19,
        "7A": 14, "8A": 21, "9A": 16, "10A": 23, "11A": 18, "12A": 13,
        "1B": 11, "2B": 6, "3B": 1, "4B": 8, "5B": 3, "6B": 10,
        "7B": 5, "8B": 0, "9B": 7, "10B": 2, "11B": 9, "12B": 4,
    }
    records = []
    for rec in data["records"]:
        if rec.get("failure"):
            continue
        if not rec.get("candidates"):
            continue
        posterior = [0.0] * 24
        for cand in rec["candidates"]:
            idx = camelot_to_index.get(cand["camelot"])
            if idx is not None:
                posterior[idx] = cand["confidence"]
        # Normalize
        total = sum(posterior)
        if total > 0:
            posterior = [p / total for p in posterior]
        # Find truth index from truth_camelot
        truth_idx = camelot_to_index.get(rec.get("truth_camelot", ""), -1)
        if truth_idx < 0:
            continue
        pred_idx = int(np.argmax(posterior))
        records.append({
            "id": rec.get("title", rec.get("path", "")),
            "artist": rec.get("artist", ""),
            "genre": rec.get("genre", ""),
            "truth_index": truth_idx,
            "truth_label": rec.get("truth_camelot", ""),
            "pred_index": pred_idx,
            "pred_label": rec["candidates"][0]["camelot"] if rec["candidates"] else "",
            "posterior": posterior,
        })
    return records


def mirex_score(truth: int, predicted: int) -> float:
    if truth == predicted:
        return 1.0
    truth_tonic = truth % 12
    pred_tonic = predicted % 12
    truth_minor = truth >= 12
    pred_minor = predicted >= 12
    if truth_tonic == pred_tonic:
        return 0.6  # parallel
    if truth_minor == pred_minor and (truth_tonic - pred_tonic + 12) % 12 in (5, 7):
        return 0.5  # fifth
    if truth_minor != pred_minor and (truth_tonic - pred_tonic + 12) % 12 == (9 if truth_minor else 3):
        return 0.4  # relative
    if truth_minor == pred_minor and (truth_tonic - pred_tonic + 12) % 12 in (1, 11):
        return 0.3  # semitone
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


def compute_metrics(records: list[dict[str, Any]]) -> dict[str, Any]:
    n = len(records)
    if n == 0:
        return {"n": 0}

    exact = sum(1 for r in records if r["pred_index"] == r["truth_index"])
    mirex_scores = [mirex_score(r["truth_index"], r["pred_index"]) for r in records]
    mirex_mean = sum(mirex_scores) / n

    # Top-3 / Top-5
    top3 = 0
    top5 = 0
    for r in records:
        posterior = r["posterior"]
        ranked = sorted(range(24), key=lambda i: posterior[i], reverse=True)
        if r["truth_index"] in ranked[:3]:
            top3 += 1
        if r["truth_index"] in ranked[:5]:
            top5 += 1

    # Error types
    errors = Counter(error_type(r["truth_index"], r["pred_index"]) for r in records)

    # Calibration (ECE)
    ece = compute_ece(records)

    # Genre breakdown
    by_genre: dict[str, dict[str, Any]] = {}
    for r in records:
        genre = r.get("genre", "unknown")
        if genre not in by_genre:
            by_genre[genre] = {"n": 0, "exact": 0, "mirex_sum": 0.0}
        by_genre[genre]["n"] += 1
        if r["pred_index"] == r["truth_index"]:
            by_genre[genre]["exact"] += 1
        by_genre[genre]["mirex_sum"] += mirex_score(r["truth_index"], r["pred_index"])

    for genre, stats in by_genre.items():
        stats["exact_pct"] = 100.0 * stats["exact"] / stats["n"]
        stats["mirex_mean"] = stats["mirex_sum"] / stats["n"]

    # Per-key confusion
    per_key: dict[int, dict[str, Any]] = {}
    for r in records:
        truth = r["truth_index"]
        if truth not in per_key:
            per_key[truth] = {"n": 0, "exact": 0, "errors": Counter()}
        per_key[truth]["n"] += 1
        if r["pred_index"] == r["truth_index"]:
            per_key[truth]["exact"] += 1
        else:
            per_key[truth]["errors"][r["pred_index"]] += 1

    per_key_summary = {}
    for key, stats in sorted(per_key.items()):
        per_key_summary[key] = {
            "n": stats["n"],
            "exact_pct": 100.0 * stats["exact"] / stats["n"],
            "top_errors": [{"pred_index": k, "count": v} for k, v in stats["errors"].most_common(3)],
        }

    return {
        "n": n,
        "exact": exact,
        "exact_pct": 100.0 * exact / n,
        "mirex_mean": mirex_mean,
        "top3_pct": 100.0 * top3 / n,
        "top5_pct": 100.0 * top5 / n,
        "errors": dict(errors),
        "ece": ece,
        "by_genre": by_genre,
        "per_key": per_key_summary,
    }


def compute_ece(records: list[dict[str, Any]], n_bins: int = 10) -> float:
    """Expected Calibration Error."""
    confidences = []
    correct = []
    for r in records:
        posterior = r["posterior"]
        pred_idx = int(np.argmax(posterior))
        confidences.append(posterior[pred_idx])
        correct.append(1.0 if pred_idx == r["truth_index"] else 0.0)

    confidences = np.array(confidences)
    correct = np.array(correct)

    bin_boundaries = np.linspace(0, 1, n_bins + 1)
    ece = 0.0
    n = len(confidences)
    for i in range(n_bins):
        lo, hi = bin_boundaries[i], bin_boundaries[i + 1]
        mask = (confidences > lo) & (confidences <= hi)
        if mask.sum() == 0:
            continue
        bin_conf = confidences[mask].mean()
        bin_acc = correct[mask].mean()
        bin_size = mask.sum() / n
        ece += bin_size * abs(bin_conf - bin_acc)
    return float(ece)


def compute_oracle_ceiling(model_posteriors: dict[str, list[dict[str, Any]]], model_names: list[str]) -> dict[str, Any]:
    """Compute oracle ceilings for 2/3/4/5-model combinations."""
    from itertools import combinations

    # Align records by ID
    id_sets = {name: {r["id"] for r in model_posteriors[name]} for name in model_names}
    common_ids = set.intersection(*id_sets.values()) if id_sets else set()

    results = {}
    for k in range(2, min(6, len(model_names) + 1)):
        best_exact = 0
        best_combo = None
        for combo in combinations(model_names, k):
            # For each track, the oracle picks the model with highest confidence on the correct key
            exact = 0
            for tid in common_ids:
                truth_idx = None
                best_correct_conf = -1
                for name in combo:
                    rec = next(r for r in model_posteriors[name] if r["id"] == tid)
                    if truth_idx is None:
                        truth_idx = rec["truth_index"]
                    pred_conf = rec["posterior"][truth_idx]
                    if pred_conf > best_correct_conf:
                        best_correct_conf = pred_conf
                # Oracle is correct if ANY model in the combo predicts correctly
                any_correct = False
                for name in combo:
                    rec = next(r for r in model_posteriors[name] if r["id"] == tid)
                    if rec["pred_index"] == truth_idx:
                        any_correct = True
                        break
                if any_correct:
                    exact += 1
            if exact > best_exact:
                best_exact = exact
                best_combo = combo
        results[f"{k}_model"] = {
            "oracle_exact": best_exact,
            "oracle_pct": 100.0 * best_exact / len(common_ids) if common_ids else 0,
            "best_combo": list(best_combo) if best_combo else [],
            "n_common": len(common_ids),
        }
    return results


def compute_model_overlap(model_posteriors: dict[str, list[dict[str, Any]]], model_names: list[str]) -> dict[str, Any]:
    """Compute pairwise model agreement and correlation."""
    from itertools import combinations

    # Align by ID
    id_to_idx = {}
    for name in model_names:
        for i, r in enumerate(model_posteriors[name]):
            if r["id"] not in id_to_idx:
                id_to_idx[r["id"]] = {}
            id_to_idx[r["id"]][name] = i

    common_ids = [tid for tid in id_to_idx if all(name in id_to_idx[tid] for name in model_names)]

    overlap = {}
    for a, b in combinations(model_names, 2):
        agreements = 0
        both_correct = 0
        a_correct_b_wrong = 0
        a_wrong_b_correct = 0
        both_wrong = 0
        for tid in common_ids:
            ra = model_posteriors[a][id_to_idx[tid][a]]
            rb = model_posteriors[b][id_to_idx[tid][b]]
            pa = ra["pred_index"] == ra["truth_index"]
            pb = rb["pred_index"] == rb["truth_index"]
            if ra["pred_index"] == rb["pred_index"]:
                agreements += 1
            if pa and pb:
                both_correct += 1
            elif pa and not pb:
                a_correct_b_wrong += 1
            elif not pa and pb:
                a_wrong_b_correct += 1
            else:
                both_wrong += 1
        n = len(common_ids)
        overlap[f"{a}_vs_{b}"] = {
            "agreement_pct": 100.0 * agreements / n,
            "both_correct": both_correct,
            "a_correct_b_wrong": a_correct_b_wrong,
            "a_wrong_b_correct": a_wrong_b_correct,
            "both_wrong": both_wrong,
        }
    return overlap


def main() -> int:
    parser = argparse.ArgumentParser(description="Comprehensive FMAK benchmark analysis")
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--classical-json", type=Path)
    parser.add_argument("--myna-v6-base-jsonl", type=Path)
    parser.add_argument("--myna-v6-tta-jsonl", type=Path)
    parser.add_argument("--myna-v8-jsonl", type=Path)
    parser.add_argument("--skey-jsonl", type=Path)
    parser.add_argument("--temporal-jsonl", type=Path)
    args = parser.parse_args()

    models: dict[str, list[dict[str, Any]]] = {}
    model_sources = {
        "classical": (args.classical_json, "json"),
        "myna_v6_base": (args.myna_v6_base_jsonl, "jsonl"),
        "myna_v6_tta": (args.myna_v6_tta_jsonl, "jsonl"),
        "myna_v8_compact": (args.myna_v8_jsonl, "jsonl"),
        "skey_harmonic": (args.skey_jsonl, "jsonl"),
        "temporal_ranker": (args.temporal_jsonl, "jsonl"),
    }

    for name, (path, fmt) in model_sources.items():
        if path is None or not path.exists():
            print(f"  {name}: not available ({path})")
            continue
        if fmt == "json":
            records = load_classical_json(path)
        else:
            records = load_jsonl(path)
        models[name] = records
        print(f"  {name}: {len(records)} records from {path.name}")

    if not models:
        print("No model outputs found!")
        return 1

    # Per-model metrics
    all_metrics = {}
    for name, records in models.items():
        print(f"\n=== {name} ===")
        m = compute_metrics(records)
        all_metrics[name] = m
        print(f"  n={m['n']}  exact={m['exact_pct']:.1f}%  mirex={m['mirex_mean']:.3f}  top3={m['top3_pct']:.1f}%  top5={m['top5_pct']:.1f}%  ece={m['ece']:.3f}")
        print(f"  errors: {m['errors']}")

    # Oracle ceilings
    model_names = list(models.keys())
    if len(model_names) >= 2:
        print(f"\n=== Oracle ceilings ({len(model_names)} models) ===")
        oracle = compute_oracle_ceiling(models, model_names)
        for k, v in oracle.items():
            print(f"  {k}: {v['oracle_pct']:.1f}% (n={v['n_common']}, combo={v['best_combo']})")
    else:
        oracle = {}

    # Model overlap
    if len(model_names) >= 2:
        print(f"\n=== Model overlap ===")
        overlap = compute_model_overlap(models, model_names)
        for pair, stats in overlap.items():
            print(f"  {pair}: agreement={stats['agreement_pct']:.1f}%  both_correct={stats['both_correct']}  a_correct_b_wrong={stats['a_correct_b_wrong']}  a_wrong_b_correct={stats['a_wrong_b_correct']}  both_wrong={stats['both_wrong']}")
    else:
        overlap = {}

    # Write summary
    summary = {
        "models_evaluated": model_names,
        "per_model": all_metrics,
        "oracle_ceilings": oracle,
        "model_overlap": overlap,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(summary, indent=2), encoding="utf-8")
    print(f"\nSummary written to: {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
