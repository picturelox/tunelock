#!/usr/bin/env python3
"""Evaluate a Myna key head on base embeddings (no TTA) and emit 24-key posteriors.

This is a simplified version of evaluate_myna_tta.py that only uses the base
(unshifted) embedding view. It's intended for quick zero-shot evaluation on
new corpora where pitch-shifted embeddings haven't been extracted yet.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
from typing import Any, Callable

import numpy as np
import torch

from train_myna_head import (
    KeyHead,
    aggregate_track_logits,
    batched_logits,
    embedding_path,
    sha256,
    write_jsonl,
)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Evaluate Myna head on base embeddings (no TTA)")
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--embedding-cache", required=True, type=Path)
    parser.add_argument("--checkpoint", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--role", choices=("development", "validation"), default="development")
    parser.add_argument("--aggregation", choices=("probabilities", "logits"), default="probabilities")
    parser.add_argument("--batch-size", type=int, default=512)
    parser.add_argument("--device", default="cuda" if torch.cuda.is_available() else "cpu")
    return parser.parse_args()


def load_metadata(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def load_cached_embeddings(
    records: list[dict[str, Any]],
    path_for: Callable[[dict[str, Any]], Path],
    source_embedding_dim: int,
    embedding_slice: tuple[int, int],
) -> tuple[torch.Tensor, torch.Tensor, list[int]]:
    """Load embeddings, returning (embeddings, record_indices, valid_record_indices).

    Records without embedding files are silently skipped.
    """
    start, end = embedding_slice
    chunks: list[np.ndarray] = []
    record_indices: list[int] = []
    valid_indices: list[int] = []
    for record_index, record in enumerate(records):
        path = path_for(record)
        if not path.exists():
            continue
        value = np.load(path, allow_pickle=False)
        if value.ndim != 2 or value.shape[0] < 1 or value.shape[1] != source_embedding_dim:
            continue
        chunks.append(np.asarray(value[:, start:end], dtype=np.float32))
        record_indices.extend([record_index] * len(value))
        valid_indices.append(record_index)
    return (
        torch.from_numpy(np.concatenate(chunks)),
        torch.tensor(record_indices, dtype=torch.long),
        valid_indices,
    )


def main() -> int:
    args = parse_args()
    if args.output.exists():
        raise FileExistsError(f"Refusing to overwrite: {args.output}")

    manifest = load_metadata(args.manifest)
    manifest_hash = sha256(args.manifest)
    checkpoint = torch.load(args.checkpoint, map_location="cpu", weights_only=False)

    # Don't enforce manifest hash match — FMAK is a different corpus from GiantSteps
    # where the checkpoint was trained. This is zero-shot evaluation.
    print(f"Checkpoint: {args.checkpoint.name}", flush=True)
    print(f"Checkpoint manifest_sha256: {checkpoint.get('manifest_sha256', 'N/A')}", flush=True)
    print(f"Current manifest_sha256:    {manifest_hash}", flush=True)
    print("Note: Zero-shot evaluation (checkpoint trained on different corpus)", flush=True)

    embedding_dim = int(checkpoint.get("embedding_dim", 384))
    source_embedding_dim = int(checkpoint.get("source_embedding_dim", embedding_dim))
    raw_slice = checkpoint.get("embedding_slice", [0, embedding_dim])
    embedding_slice = (int(raw_slice[0]), int(raw_slice[1]))

    base_metadata = load_metadata(args.embedding_cache / "metadata.json")
    print(f"Base cache model: {base_metadata.get('model')}", flush=True)
    print(f"Base cache embedding_dim: {base_metadata.get('embedding_dim')}", flush=True)

    if args.role == "development":
        records = [r for r in manifest["records"] if r["role"] == "development"]
        states = checkpoint.get("state_dicts", [])
    else:
        records = [r for r in manifest["records"] if r["role"] == "training"]
        states = checkpoint.get("validation_state_dicts", [])

    if not records:
        raise ValueError(f"No {args.role} records in manifest")
    if not states:
        raise ValueError(f"No state dicts for role={args.role}")

    print(f"Records: {len(records)}, Models: {len(states)}", flush=True)

    # Load models
    models = []
    for state in states:
        model = KeyHead(
            checkpoint["hidden_dims"], float(checkpoint["dropout"]), embedding_dim
        )
        model.load_state_dict(state)
        models.append(model.to(args.device).eval())

    # Load base embeddings
    embeddings, record_indices, valid_indices = load_cached_embeddings(
        records,
        lambda record: embedding_path(args.embedding_cache, record),
        source_embedding_dim,
        embedding_slice,
    )
    valid_records = [records[i] for i in valid_indices]
    # Remap record indices from full-list positions to valid-list positions
    index_map = {old: new for new, old in enumerate(valid_indices)}
    record_indices = torch.tensor([index_map[int(i)] for i in record_indices], dtype=torch.long)
    print(f"Loaded {embeddings.shape[0]} chunks for {len(valid_records)}/{len(records)} records", flush=True)

    # Run inference
    all_posteriors: list[dict[str, Any]] = []
    for model_index, model in enumerate(models):
        chunk_logits = batched_logits(model, embeddings, args.batch_size, args.device)
        values = aggregate_track_logits(chunk_logits, record_indices, len(valid_records))
        if args.aggregation == "probabilities":
            values = values.softmax(dim=1)

        if model_index == 0:
            for i, record in enumerate(valid_records):
                posterior = values[i].cpu().numpy()
                pred_index = int(np.argmax(posterior))
                all_posteriors.append({
                    "id": record["id"],
                    "artist": record.get("artist", ""),
                    "genre": record.get("genre", ""),
                    "truth_index": record["truth_index"],
                    "truth_label": record["truth_label"],
                    "pred_index": pred_index,
                    "pred_label": manifest["canonical_labels"][pred_index],
                    "posterior": posterior.tolist(),
                    "model_index": model_index,
                })
        else:
            for i in range(len(valid_records)):
                posterior = values[i].cpu().numpy()
                all_posteriors[i]["posterior"] = (
                    np.array(all_posteriors[i]["posterior"]) + posterior
                ).tolist()

    if len(models) > 1:
        for entry in all_posteriors:
            avg = np.array(entry["posterior"]) / len(models)
            entry["posterior"] = avg.tolist()
            entry["pred_index"] = int(np.argmax(avg))
            entry["pred_label"] = manifest["canonical_labels"][entry["pred_index"]]

    # Compute metrics
    exact = sum(1 for e in all_posteriors if e["pred_index"] == e["truth_index"])
    total = len(all_posteriors)
    print(f"\n=== {args.checkpoint.name} on FMAK ===", flush=True)
    print(f"Scored: {total}", flush=True)
    print(f"Exact: {exact}/{total} ({100*exact/total:.1f}%)", flush=True)

    # Write JSONL
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("w", encoding="utf-8") as f:
        for entry in all_posteriors:
            f.write(json.dumps(entry, separators=(",", ":")) + "\n")
    print(f"Wrote: {args.output}", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
