#!/usr/bin/env python3
"""Evaluate the temporal candidate ranker on FMAK using cached Myna embeddings.

The temporal ranker is a linear model that takes 72 temporal features
(statistics of per-chunk logits) per candidate key and produces a score.
This script:
1. Loads per-chunk Myna embeddings from the FMAK cache
2. Runs the Myna v6 head to get per-chunk logits
3. Computes temporal features using the same logic as train_temporal_candidate_ranker.py
4. Applies the temporal ranker to get posteriors
"""

from __future__ import annotations

import json
import math
from pathlib import Path
from typing import Any

import numpy as np
import torch

from train_myna_head import (
    KeyHead,
    batched_logits,
    embedding_path,
    sha256,
)
from train_temporal_candidate_ranker import (
    feature_names,
    record_candidate_features,
    feature_tracks,
)


def main() -> int:
    import argparse
    parser = argparse.ArgumentParser()
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--embedding-cache", required=True, type=Path)
    parser.add_argument("--myna-checkpoint", required=True, type=Path)
    parser.add_argument("--ranker-checkpoint", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--role", default="development")
    parser.add_argument("--batch-size", type=int, default=512)
    parser.add_argument("--device", default="cuda" if torch.cuda.is_available() else "cpu")
    args = parser.parse_args()

    manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
    records = [r for r in manifest["records"] if r["role"] == args.role]
    print(f"Records: {len(records)}", flush=True)

    # Load Myna v6 head
    myna_ckpt = torch.load(args.myna_checkpoint, map_location="cpu", weights_only=False)
    embedding_dim = int(myna_ckpt.get("embedding_dim", 384))
    source_embedding_dim = int(myna_ckpt.get("source_embedding_dim", embedding_dim))
    raw_slice = myna_ckpt.get("embedding_slice", [0, embedding_dim])
    embedding_slice = (int(raw_slice[0]), int(raw_slice[1]))

    states = myna_ckpt.get("state_dicts", [])
    if not states:
        raise ValueError("No state dicts in Myna checkpoint")
    model = KeyHead(myna_ckpt["hidden_dims"], float(myna_ckpt["dropout"]), embedding_dim)
    model.load_state_dict(states[0])
    model = model.to(args.device).eval()
    print(f"Myna head loaded: {args.myna_checkpoint.name}", flush=True)

    # Load temporal ranker
    ranker = json.loads(args.ranker_checkpoint.read_text(encoding="utf-8"))
    ranker_weights = torch.tensor(ranker["model"]["weights"], dtype=torch.float32)
    ranker_bias = torch.tensor(ranker["model"]["bias"], dtype=torch.float32)
    standardizer_mean = torch.tensor(ranker["standardizer"]["mean"], dtype=torch.float32)
    standardizer_std = torch.tensor(ranker["standardizer"]["scale"], dtype=torch.float32)
    print(f"Ranker loaded: {args.ranker_checkpoint.name}", flush=True)
    print(f"  weights shape: {ranker_weights.shape}", flush=True)
    print(f"  bias shape: {ranker_bias.shape}", flush=True)

    # Load per-chunk embeddings
    start, end = embedding_slice
    chunks_list = []
    record_indices = []
    valid_records = []
    for record_index, record in enumerate(records):
        path = embedding_path(args.embedding_cache, record)
        if not path.exists():
            continue
        value = np.load(path, allow_pickle=False)
        if value.ndim != 2 or value.shape[0] < 3 or value.shape[1] != source_embedding_dim:
            continue
        chunks_list.append(np.asarray(value[:, start:end], dtype=np.float32))
        record_indices.extend([len(valid_records)] * len(value))
        valid_records.append(record)

    embeddings = torch.from_numpy(np.concatenate(chunks_list))
    record_indices = torch.tensor(record_indices, dtype=torch.long)
    print(f"Loaded {embeddings.shape[0]} chunks for {len(valid_records)}/{len(records)} records", flush=True)

    # Get per-chunk logits
    print("Computing per-chunk logits...", flush=True)
    chunk_logits = batched_logits(model, embeddings, args.batch_size, args.device)
    print(f"Chunk logits shape: {chunk_logits.shape}", flush=True)

    # Compute temporal features
    print("Computing temporal features...", flush=True)
    features, baseline = feature_tracks(chunk_logits, record_indices, len(valid_records))
    print(f"Temporal features shape: {features.shape}", flush=True)  # (n_records, 24, 72)

    # Standardize features
    n_records = features.shape[0]
    features_flat = features.reshape(n_records * 24, -1)
    features_flat = (features_flat - standardizer_mean) / standardizer_std.clamp_min(1e-6)
    features_flat = features_flat.reshape(n_records, 24, -1)

    # Apply ranker: score = features @ weights + bias
    # weights shape might be (72,) or (72, 1) — apply per candidate
    scores = features_flat @ ranker_weights + ranker_bias  # (n_records, 24)
    posteriors = scores.softmax(dim=1)
    print(f"Posteriors shape: {posteriors.shape}", flush=True)

    # Compute metrics
    exact = 0
    all_results = []
    for i, record in enumerate(valid_records):
        posterior = posteriors[i].cpu().numpy()
        pred_index = int(np.argmax(posterior))
        truth_index = record["truth_index"]
        if pred_index == truth_index:
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

    total = len(all_results)
    print(f"\n=== Temporal ranker on FMAK ===", flush=True)
    print(f"Scored: {total}", flush=True)
    print(f"Exact: {exact}/{total} ({100*exact/total:.1f}%)", flush=True)

    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("w", encoding="utf-8") as f:
        for entry in all_results:
            f.write(json.dumps(entry, separators=(",", ":")) + "\n")
    print(f"Wrote: {args.output}", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
