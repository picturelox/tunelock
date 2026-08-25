#!/usr/bin/env python3
"""Evaluate S-KEY harmonic head on FMAK cached features (zero-shot)."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any

import numpy as np
import torch

from extract_skey_harmonic_features import feature_path
from train_skey_harmonic_head import HarmonicHead, predict


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Evaluate S-KEY harmonic head on FMAK")
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--feature-cache", required=True, type=Path)
    parser.add_argument("--checkpoint", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--role", default="development")
    parser.add_argument("--device", default="cuda" if torch.cuda.is_available() else "cpu")
    parser.add_argument("--batch-size", type=int, default=512)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
    records = [r for r in manifest["records"] if r["role"] == args.role]
    print(f"Records: {len(records)}", flush=True)

    # Load checkpoint
    checkpoint = torch.load(args.checkpoint, map_location="cpu", weights_only=False)
    config = checkpoint["selection"]["selected"]["architecture"]
    states = checkpoint["states"]
    print(f"Checkpoint: {args.checkpoint.name}", flush=True)
    print(f"Config: {config}", flush=True)
    print(f"States: {len(states)}", flush=True)

    # Load features
    features_list = []
    valid_records = []
    for record in records:
        path = feature_path(args.feature_cache, record)
        if not path.exists():
            continue
        data = np.load(path, allow_pickle=False)
        features_list.append(data["feature"])
        valid_records.append(record)

    features = torch.from_numpy(np.stack(features_list)).float()
    print(f"Loaded {len(valid_records)}/{len(records)} feature files", flush=True)
    print(f"Features shape: {features.shape}", flush=True)

    # Load models and predict (average across seeds)
    all_posteriors = []
    for state in states:
        model = HarmonicHead(config).to(args.device)
        model.load_state_dict(state)
        posteriors = predict(model, features, args.batch_size, args.device)
        all_posteriors.append(posteriors)
    posteriors = torch.stack(all_posteriors).mean(dim=0)
    print(f"Posteriors shape: {posteriors.shape}", flush=True)

    # Compute metrics
    exact = 0
    all_results = []
    for i, record in enumerate(valid_records):
        posterior = posteriors[i].numpy()
        pred_index = int(np.argmax(posterior))
        truth_index = record["truth_index"]
        is_correct = pred_index == truth_index
        if is_correct:
            exact += 1
        all_results.append({
            "id": record["id"],
            "artist": record.get("artist", ""),
            "genre": record.get("genre", ""),
            "truth_index": truth_index,
            "truth_label": record["truth_label"],
            "pred_index": pred_index,
            "pred_label": manifest["canonical_labels"][pred_index],
            "posterior": posterior.tolist(),
        })

    total = len(valid_records)
    print(f"\n=== {args.checkpoint.name} on FMAK ===", flush=True)
    print(f"Scored: {total}", flush=True)
    print(f"Exact: {exact}/{total} ({100*exact/total:.1f}%)", flush=True)

    # Write JSONL
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("w", encoding="utf-8") as f:
        for entry in all_results:
            f.write(json.dumps(entry, separators=(",", ":")) + "\n")
    print(f"Wrote: {args.output}", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
