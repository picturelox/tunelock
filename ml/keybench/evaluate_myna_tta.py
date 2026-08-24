#!/usr/bin/env python3
"""Evaluate a trained Myna head with Rust-defined transposition alignment."""

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
    deduplicate_recordings,
    embedding_path,
    fixed_folds,
    leakage_groups,
    pitch_embedding_path,
    sha256,
    validate_pitch_cache_metadata,
    write_jsonl,
)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Apply pitch-equivariant TTA to a Myna key head")
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--embedding-cache", required=True, type=Path)
    parser.add_argument("--pitch-cache", required=True, type=Path)
    parser.add_argument("--checkpoint", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument(
        "--role",
        choices=("development", "validation"),
        default="development",
        help="Use final full-data states on development, or held-out-fold states on validation.",
    )
    parser.add_argument(
        "--aggregation",
        choices=("probabilities", "logits"),
        default="probabilities",
    )
    parser.add_argument("--batch-size", type=int, default=512)
    parser.add_argument("--semitones", type=int, nargs="+")
    parser.add_argument(
        "--original-weight",
        type=float,
        default=1.0,
        help="Weight of the unshifted posterior relative to each shifted posterior.",
    )
    parser.add_argument(
        "--allow-training-pitch-method-mismatch",
        action="store_true",
        help=(
            "Explicitly allow TTA views whose pitch method differs from the head's "
            "training augmentation. Intended only for named ablations."
        ),
    )
    parser.add_argument("--device", default="cuda" if torch.cuda.is_available() else "cpu")
    return parser.parse_args()


def load_metadata(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def load_cached_embeddings(
    records: list[dict[str, Any]],
    path_for: Callable[[dict[str, Any]], Path],
    source_embedding_dim: int,
    embedding_slice: tuple[int, int],
) -> tuple[torch.Tensor, torch.Tensor]:
    start, end = embedding_slice
    chunks: list[np.ndarray] = []
    record_indices: list[int] = []
    for record_index, record in enumerate(records):
        path = path_for(record)
        value = np.load(path, allow_pickle=False)
        if (
            value.ndim != 2
            or value.shape[0] < 1
            or value.shape[1] != source_embedding_dim
        ):
            raise ValueError(f"Invalid embedding cache {path}: {value.shape}")
        if not np.isfinite(value).all():
            raise ValueError(f"Non-finite embedding cache: {path}")
        chunks.append(np.asarray(value[:, start:end], dtype=np.float32))
        record_indices.extend([record_index] * len(value))
    return (
        torch.from_numpy(np.concatenate(chunks)),
        torch.tensor(record_indices, dtype=torch.long),
    )


def checkpoint_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def main() -> int:
    args = parse_args()
    if args.output.exists():
        raise FileExistsError(f"Refusing to overwrite existing result: {args.output}")

    manifest = load_metadata(args.manifest)
    manifest_hash = sha256(args.manifest)
    checkpoint = torch.load(args.checkpoint, map_location="cpu", weights_only=False)
    if checkpoint.get("manifest_sha256") != manifest_hash:
        raise ValueError("Checkpoint was trained from a different manifest")
    if args.original_weight <= 0:
        raise ValueError("--original-weight must be positive")
    embedding_dim = int(
        checkpoint.get(
            "embedding_dim", checkpoint.get("base_model", {}).get("embedding_dim", 384)
        )
    )
    if embedding_dim < 1:
        raise ValueError("Checkpoint has an invalid embedding dimension")
    source_embedding_dim = int(checkpoint.get("source_embedding_dim", embedding_dim))
    raw_slice = checkpoint.get("embedding_slice", [0, embedding_dim])
    if not isinstance(raw_slice, (list, tuple)) or len(raw_slice) != 2:
        raise ValueError("Checkpoint has an invalid embedding slice")
    embedding_slice = (int(raw_slice[0]), int(raw_slice[1]))
    if (
        not 0 <= embedding_slice[0] < embedding_slice[1] <= source_embedding_dim
        or embedding_slice[1] - embedding_slice[0] != embedding_dim
    ):
        raise ValueError("Checkpoint embedding dimensions are inconsistent")

    base_metadata = load_metadata(args.embedding_cache / "metadata.json")
    checkpoint_base = checkpoint.get("base_model", {})
    if (
        base_metadata.get("manifest_sha256") != manifest_hash
        or int(base_metadata.get("embedding_dim", 0)) != source_embedding_dim
        or base_metadata.get("model") != checkpoint_base.get("model")
        or base_metadata.get("model_revision") != checkpoint_base.get("model_revision")
    ):
        raise ValueError("Base cache does not match the trained checkpoint")

    if args.role == "development":
        records = [record for record in manifest["records"] if record["role"] == "development"]
        states = checkpoint.get("state_dicts", [])
        cache_role = "development"
    else:
        training_raw = [record for record in manifest["records"] if record["role"] == "training"]
        training, _ = deduplicate_recordings(training_raw)
        labels = np.asarray([int(record["truth_index"]) for record in training], dtype=np.int64)
        groups, _ = leakage_groups(training)
        folds = fixed_folds(labels, groups)
        validation_fold = int(checkpoint.get("validation_fold", -1))
        if validation_fold not in range(5):
            raise ValueError("Checkpoint does not record a valid held-out fold")
        records = [record for index, record in enumerate(training) if folds[index] == validation_fold]
        states = checkpoint.get("validation_state_dicts", [])
        cache_role = "training"
    if not records:
        raise ValueError(f"Manifest has no {args.role} records")

    pitch_metadata = load_metadata(args.pitch_cache / "metadata.json")
    available_shifts = validate_pitch_cache_metadata(
        pitch_metadata,
        manifest_hash=manifest_hash,
        base_metadata=checkpoint_base,
        required_role=cache_role,
    )
    training_pitch_metadata = checkpoint.get("pitch_augmentation")
    if isinstance(training_pitch_metadata, dict):
        training_method = training_pitch_metadata.get("pitch_method")
        evaluation_method = pitch_metadata.get("pitch_method")
        if (
            training_method != evaluation_method
            and not args.allow_training_pitch_method_mismatch
        ):
            raise ValueError(
                "TTA pitch method does not match the head's training augmentation: "
                f"training={training_method}, evaluation={evaluation_method}. "
                "Use --allow-training-pitch-method-mismatch only for a named ablation."
            )
    shifts = sorted(args.semitones if args.semitones is not None else available_shifts)
    target_tables = {
        int(item["semitones"]): torch.tensor(item["target_by_source_index"], dtype=torch.long)
        for item in manifest["pitch_shift_targets"]
    }
    if (
        not shifts
        or len(set(shifts)) != len(shifts)
        or any(
            shift == 0 or shift not in target_tables or shift not in available_shifts
            for shift in shifts
        )
    ):
        raise ValueError("Pitch cache shifts do not match the Rust manifest")

    if not states:
        raise ValueError(f"Checkpoint has no {args.role} state dictionaries")
    models = []
    for state in states:
        model = KeyHead(
            checkpoint["hidden_dims"], float(checkpoint["dropout"]), embedding_dim
        )
        model.load_state_dict(state)
        models.append(model.to(args.device).eval())

    totals = [torch.zeros((len(records), 24), dtype=torch.float32) for _ in models]
    diagnostic_views: list[list[torch.Tensor]] = [[] for _ in models]
    transforms: list[tuple[int, Callable[[dict[str, Any]], Path]]] = [
        (0, lambda record: embedding_path(args.embedding_cache, record))
    ]
    transforms.extend(
        (
            shift,
            lambda record, selected_shift=shift: pitch_embedding_path(
                args.pitch_cache, selected_shift, record
            ),
        )
        for shift in shifts
    )

    for transform_index, (shift, path_for) in enumerate(transforms, start=1):
        embeddings, record_indices = load_cached_embeddings(
            records, path_for, source_embedding_dim, embedding_slice
        )
        for model_index, model in enumerate(models):
            chunk_logits = batched_logits(model, embeddings, args.batch_size, args.device)
            values = aggregate_track_logits(chunk_logits, record_indices, len(records))
            if args.aggregation == "probabilities":
                values = values.softmax(dim=1)
            if shift:
                # Rust table maps original source index -> shifted target index.
                # Indexing shifted outputs by that table aligns every column back
                # to the original recording's 24-key vocabulary.
                values = values[:, target_tables[shift]]
            diagnostic_views[model_index].append(
                values.detach().cpu()
                if args.aggregation == "probabilities"
                else values.softmax(dim=1).detach().cpu()
            )
            totals[model_index] += values * (args.original_weight if shift == 0 else 1.0)
        print(
            f"transform={transform_index}/{len(transforms)} shift={shift:+d} "
            f"records={len(records)}",
            flush=True,
        )

    seed_posteriors = []
    total_weight = args.original_weight + len(shifts)
    for total in totals:
        averaged = total / total_weight
        if args.aggregation == "logits":
            averaged = averaged.softmax(dim=1)
        seed_posteriors.append(averaged)
    posteriors = torch.stack(seed_posteriors).mean(dim=0)
    view_stack = torch.stack(
        [
            torch.stack([model_views[index] for model_views in diagnostic_views]).mean(dim=0)
            for index in range(len(transforms))
        ]
    )
    view_mean = view_stack.mean(dim=0)
    view_entropy = -torch.sum(
        view_stack * torch.log(torch.clamp(view_stack, min=1e-12)), dim=2
    )
    view_js = torch.sum(
        view_stack
        * (
            torch.log(torch.clamp(view_stack, min=1e-12))
            - torch.log(torch.clamp(view_mean, min=1e-12)).unsqueeze(0)
        ),
        dim=2,
    )
    view_winners = view_stack.argmax(dim=2)
    prediction_extras = []
    for record_index in range(len(records)):
        winner_rate = torch.bincount(
            view_winners[:, record_index], minlength=24
        ).to(dtype=torch.float32) / len(transforms)
        prediction_extras.append(
            {
                "diagnostics": {
                    "tta": {
                        "view_count": len(transforms),
                        "candidate_std": view_stack[:, record_index, :].std(
                            dim=0, unbiased=False
                        ).tolist(),
                        "candidate_min": view_stack[:, record_index, :].min(dim=0).values.tolist(),
                        "candidate_max": view_stack[:, record_index, :].max(dim=0).values.tolist(),
                        "candidate_top1_rate": winner_rate.tolist(),
                        "entropy_mean": float(view_entropy[:, record_index].mean()),
                        "entropy_std": float(
                            view_entropy[:, record_index].std(unbiased=False)
                        ),
                        "js_to_mean_mean": float(view_js[:, record_index].mean()),
                        "js_to_mean_max": float(view_js[:, record_index].max()),
                    }
                }
            }
        )
    truth = torch.tensor([int(record["truth_index"]) for record in records], dtype=torch.long)
    exact = int((posteriors.argmax(dim=1) == truth).sum().item())
    protocol = (
        f"pitch-equivariant test-time augmentation shifts={[0, *shifts]}, "
        f"aligned by Rust manifest, {args.aggregation} averaged, original weight={args.original_weight}, "
        f"evaluation role={args.role}, "
        f"seed models={len(models)}"
    )
    revision = (
        f"checkpoint:{checkpoint_sha256(args.checkpoint)[:16]};"
        f"tta:{sha256(Path(__file__))[:16]}"
    )
    write_jsonl(
        args.output,
        records,
        manifest["canonical_labels"],
        posteriors,
        revision,
        protocol,
        model_name=str(checkpoint.get("model", "tunelock/myna-mtg-head")),
        fold=(validation_fold if args.role == "validation" else None),
        corpus_role=(
            "training material; out-of-fold selector-training shard"
            if args.role == "validation"
            else "development benchmark; not an untouched final test"
        ),
        metadata_extra={
            "head_contract_revision": checkpoint.get("head_contract_revision")
        },
        prediction_extras=prediction_extras,
    )
    print(
        f"wrote={args.output} records={len(records)} exact={exact}/{len(records)} "
        f"({exact / len(records):.1%})",
        flush=True,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
