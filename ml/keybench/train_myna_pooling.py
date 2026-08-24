#!/usr/bin/env python3
"""Train track-level Myna heads with leakage-safe statistical pooling.

The existing KeyMyna-style head assigns the global label to every six-second
chunk and averages chunk logits at inference. This ablation instead pools a
track's frozen embeddings first, so only one supervised decision is learned per
recording. Numeric targets and transposition tables come exclusively from the
Rust-generated manifest.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import time
from typing import Any, Iterable

import numpy as np
import sklearn
import torch
from torch import nn

from train_myna_head import (
    deduplicate_recordings,
    embedding_path,
    fixed_folds,
    leakage_groups,
    pitch_embedding_path,
    seed_everything,
    sha256,
)


KEY_COUNT = 24
POOLING_PARTS = {
    "mean": ("mean",),
    "mean-std": ("mean", "std"),
    "mean-max": ("mean", "max"),
    "mean-std-max": ("mean", "std", "max"),
}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Train a leakage-safe track-level Myna pooling head"
    )
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--embedding-cache", required=True, type=Path)
    parser.add_argument("--pitch-augmentation-cache", required=True, type=Path)
    parser.add_argument("--development-pitch-cache", type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--report", required=True, type=Path)
    parser.add_argument("--checkpoint", required=True, type=Path)
    parser.add_argument("--pooling", choices=tuple(POOLING_PARTS), default="mean-std")
    parser.add_argument("--validation-fold", type=int, default=0, choices=range(5))
    parser.add_argument("--seeds", type=int, nargs="+", default=[41, 42, 43])
    parser.add_argument("--epochs", type=int, default=100)
    parser.add_argument("--patience", type=int, default=15)
    parser.add_argument("--batch-size", type=int, default=256)
    parser.add_argument("--learning-rate", type=float, default=3e-4)
    parser.add_argument("--weight-decay", type=float, default=1e-4)
    parser.add_argument("--hidden-dims", type=int, nargs="+", default=[2048])
    parser.add_argument("--dropout", type=float, default=0.75)
    parser.add_argument("--layer-norm", action="store_true")
    parser.add_argument("--amp", action="store_true")
    parser.add_argument(
        "--tta-aggregation", choices=("logits", "probabilities"), default="logits"
    )
    parser.add_argument("--device", default="cuda" if torch.cuda.is_available() else "cpu")
    return parser.parse_args()


def load_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def validate_cache(
    metadata: dict[str, Any],
    manifest_hash: str,
    expected_model: str | None = None,
    expected_revision: str | None = None,
) -> tuple[str, str, int]:
    if metadata.get("manifest_sha256") != manifest_hash:
        raise ValueError("Embedding cache was generated from a different manifest")
    model = str(metadata.get("model", ""))
    revision = str(metadata.get("model_revision", ""))
    dimension = int(metadata.get("embedding_dim", 384))
    if expected_model is not None and model != expected_model:
        raise ValueError(f"Embedding model mismatch: {model!r} != {expected_model!r}")
    if expected_revision is not None and revision != expected_revision:
        raise ValueError("Embedding revision mismatch")
    if dimension < 1:
        raise ValueError("Embedding dimension must be positive")
    return model, revision, dimension


def pool_embedding(path: Path, parts: tuple[str, ...], dimension: int) -> np.ndarray:
    value = np.load(path, allow_pickle=False)
    if value.ndim != 2 or value.shape[0] < 1 or value.shape[1] != dimension:
        raise ValueError(f"Invalid embedding cache {path}: {value.shape}")
    value = np.asarray(value, dtype=np.float32)
    if not np.isfinite(value).all():
        raise ValueError(f"Non-finite embedding cache: {path}")
    vectors = []
    for part in parts:
        if part == "mean":
            vectors.append(value.mean(axis=0))
        elif part == "std":
            vectors.append(value.std(axis=0))
        elif part == "max":
            vectors.append(value.max(axis=0))
        else:
            raise ValueError(part)
    return np.concatenate(vectors).astype(np.float32, copy=False)


def pitch_targets(manifest: dict[str, Any]) -> dict[int, list[int]]:
    tables = {
        int(item["semitones"]): [int(value) for value in item["target_by_source_index"]]
        for item in manifest["pitch_shift_targets"]
    }
    if set(tables) != set(range(-6, 7)) or any(len(values) != KEY_COUNT for values in tables.values()):
        raise ValueError("Rust manifest must contain complete [-6, 6] transposition tables")
    return tables


def load_training_vectors(
    records: list[dict[str, Any]],
    embedding_cache: Path,
    pitch_cache: Path,
    target_tables: dict[int, list[int]],
    parts: tuple[str, ...],
    dimension: int,
) -> tuple[torch.Tensor, torch.Tensor]:
    vectors: list[np.ndarray] = []
    targets: list[int] = []
    for record in records:
        source_index = int(record["truth_index"])
        vectors.append(pool_embedding(embedding_path(embedding_cache, record), parts, dimension))
        targets.append(source_index)
        for semitones, table in sorted(target_tables.items()):
            if semitones == 0:
                continue
            vectors.append(
                pool_embedding(
                    pitch_embedding_path(pitch_cache, semitones, record),
                    parts,
                    dimension,
                )
            )
            targets.append(table[source_index])
    return torch.from_numpy(np.stack(vectors)), torch.tensor(targets, dtype=torch.long)


def load_clean_vectors(
    records: list[dict[str, Any]],
    embedding_cache: Path,
    parts: tuple[str, ...],
    dimension: int,
) -> torch.Tensor:
    return torch.from_numpy(
        np.stack(
            [
                pool_embedding(embedding_path(embedding_cache, record), parts, dimension)
                for record in records
            ]
        )
    )


class PoolingHead(nn.Module):
    def __init__(
        self,
        input_dim: int,
        hidden_dims: list[int],
        dropout: float,
        layer_norm: bool,
    ) -> None:
        super().__init__()
        if not hidden_dims or any(value < 1 for value in hidden_dims):
            raise ValueError("At least one positive hidden dimension is required")
        layers: list[nn.Module] = []
        if layer_norm:
            layers.append(nn.LayerNorm(input_dim))
        previous = input_dim
        for index, hidden in enumerate(hidden_dims):
            layers.extend((nn.Linear(previous, hidden), nn.ReLU()))
            if index == 0:
                layers.append(nn.Dropout(dropout))
            previous = hidden
        layers.append(nn.Linear(previous, KEY_COUNT))
        self.layers = nn.Sequential(*layers)

    def forward(self, inputs: torch.Tensor) -> torch.Tensor:
        return self.layers(inputs)


def batched_logits(
    model: nn.Module, vectors: torch.Tensor, batch_size: int, device: str
) -> torch.Tensor:
    model.eval()
    outputs = []
    with torch.inference_mode():
        for start in range(0, len(vectors), batch_size):
            outputs.append(model(vectors[start : start + batch_size].to(device)).cpu())
    return torch.cat(outputs)


def train_epoch(
    model: nn.Module,
    optimizer: torch.optim.Optimizer,
    vectors: torch.Tensor,
    labels: torch.Tensor,
    batch_size: int,
    device: str,
    generator: torch.Generator,
    amp: bool,
    scaler: torch.amp.GradScaler,
) -> float:
    model.train()
    order = torch.randperm(len(vectors), generator=generator)
    losses = []
    for start in range(0, len(order), batch_size):
        indices = order[start : start + batch_size]
        inputs = vectors[indices].to(device)
        targets = labels[indices].to(device)
        optimizer.zero_grad(set_to_none=True)
        with torch.autocast(device_type="cuda", dtype=torch.float16, enabled=amp):
            loss = nn.functional.cross_entropy(model(inputs), targets)
        scaler.scale(loss).backward()
        scaler.step(optimizer)
        scaler.update()
        losses.append(float(loss.detach().cpu()))
    return float(np.mean(losses))


def train_selected(
    seed: int,
    train_data: tuple[torch.Tensor, torch.Tensor],
    valid_vectors: torch.Tensor,
    valid_labels: torch.Tensor,
    input_dim: int,
    args: argparse.Namespace,
) -> tuple[int, float, dict[str, torch.Tensor], list[dict[str, float]]]:
    seed_everything(seed)
    model = PoolingHead(
        input_dim, args.hidden_dims, args.dropout, args.layer_norm
    ).to(args.device)
    optimizer = torch.optim.Adam(
        model.parameters(), lr=args.learning_rate, weight_decay=args.weight_decay
    )
    generator = torch.Generator().manual_seed(seed)
    scaler = torch.amp.GradScaler("cuda", enabled=args.amp)
    train_vectors, train_labels = train_data
    best_epoch = 0
    best_accuracy = -1.0
    best_state: dict[str, torch.Tensor] = {}
    history = []
    stale = 0

    for epoch in range(1, args.epochs + 1):
        loss = train_epoch(
            model,
            optimizer,
            train_vectors,
            train_labels,
            args.batch_size,
            args.device,
            generator,
            args.amp,
            scaler,
        )
        logits = batched_logits(model, valid_vectors, args.batch_size, args.device)
        accuracy = float((logits.argmax(dim=1) == valid_labels).float().mean().item())
        history.append({"epoch": epoch, "loss": loss, "validation_exact": accuracy})
        if accuracy > best_accuracy:
            best_epoch = epoch
            best_accuracy = accuracy
            best_state = {
                name: value.detach().cpu().clone()
                for name, value in model.state_dict().items()
            }
            stale = 0
        else:
            stale += 1
        if epoch == 1 or epoch % 10 == 0 or stale >= args.patience:
            print(
                f"seed={seed} epoch={epoch} loss={loss:.4f} "
                f"validation={accuracy:.3%} best={best_accuracy:.3%}@{best_epoch}",
                flush=True,
            )
        if stale >= args.patience:
            break
    return best_epoch, best_accuracy, best_state, history


def train_full(
    seed: int,
    epochs: int,
    data: tuple[torch.Tensor, torch.Tensor],
    input_dim: int,
    args: argparse.Namespace,
) -> PoolingHead:
    seed_everything(seed)
    model = PoolingHead(
        input_dim, args.hidden_dims, args.dropout, args.layer_norm
    ).to(args.device)
    optimizer = torch.optim.Adam(
        model.parameters(), lr=args.learning_rate, weight_decay=args.weight_decay
    )
    generator = torch.Generator().manual_seed(seed)
    scaler = torch.amp.GradScaler("cuda", enabled=args.amp)
    for _ in range(epochs):
        train_epoch(
            model,
            optimizer,
            data[0],
            data[1],
            args.batch_size,
            args.device,
            generator,
            args.amp,
            scaler,
        )
    return model


def aligned_tta_posteriors(
    models: list[nn.Module],
    records: list[dict[str, Any]],
    embedding_cache: Path,
    shifted_cache: Path | None,
    target_tables: dict[int, list[int]],
    parts: tuple[str, ...],
    dimension: int,
    args: argparse.Namespace,
) -> torch.Tensor:
    totals = [torch.zeros((len(records), KEY_COUNT), dtype=torch.float32) for _ in models]
    transforms: list[tuple[int, Path | None]] = [(0, None)]
    if shifted_cache is not None:
        transforms.extend((shift, shifted_cache) for shift in sorted(target_tables) if shift)

    for transform_index, (shift, cache) in enumerate(transforms, start=1):
        if shift == 0:
            vectors = load_clean_vectors(records, embedding_cache, parts, dimension)
        else:
            vectors = torch.from_numpy(
                np.stack(
                    [
                        pool_embedding(
                            pitch_embedding_path(cache, shift, record), parts, dimension
                        )
                        for record in records
                    ]
                )
            )
        for index, model in enumerate(models):
            values = batched_logits(model, vectors, args.batch_size, args.device)
            if args.tta_aggregation == "probabilities":
                values = values.softmax(dim=1)
            if shift:
                values = values[:, torch.tensor(target_tables[shift], dtype=torch.long)]
            totals[index] += values
        print(
            f"evaluation transform={transform_index}/{len(transforms)} shift={shift:+d}",
            flush=True,
        )

    seed_outputs = []
    for total in totals:
        averaged = total / len(transforms)
        if args.tta_aggregation == "logits":
            averaged = averaged.softmax(dim=1)
        seed_outputs.append(averaged)
    return torch.stack(seed_outputs).mean(dim=0)


def write_jsonl(
    output: Path,
    records: list[dict[str, Any]],
    labels: list[str],
    posteriors: torch.Tensor,
    revision: str,
    protocol: str,
) -> None:
    if output.exists():
        raise FileExistsError(f"Refusing to overwrite existing result: {output}")
    output.parent.mkdir(parents=True, exist_ok=True)
    temporary = output.with_name(f"{output.name}.part.{os.getpid()}")
    with temporary.open("w", encoding="utf-8", newline="\n") as handle:
        handle.write(
            json.dumps(
                {
                    "type": "metadata",
                    "schema_version": 1,
                    "model": "tunelock/myna-track-pooling-head",
                    "model_revision": revision,
                    "posterior_labels": labels,
                    "protocol": protocol,
                },
                separators=(",", ":"),
            )
            + "\n"
        )
        for record, posterior in zip(records, posteriors.tolist()):
            handle.write(
                json.dumps(
                    {
                        "type": "prediction",
                        "track_id": record["id"],
                        "status": "ok",
                        "posterior": posterior,
                    },
                    separators=(",", ":"),
                )
                + "\n"
            )
    os.replace(temporary, output)


def subset(records: list[dict[str, Any]], indices: Iterable[int]) -> list[dict[str, Any]]:
    return [records[int(index)] for index in indices]


def main() -> int:
    args = parse_args()
    started = time.perf_counter()
    manifest = load_json(args.manifest)
    manifest_hash = sha256(args.manifest)
    if manifest.get("schema_version") != 1 or len(manifest.get("canonical_labels", [])) != KEY_COUNT:
        raise ValueError("Expected schema-1 Rust key manifest with 24 canonical labels")

    base_metadata = load_json(args.embedding_cache / "metadata.json")
    model_name, model_revision, embedding_dim = validate_cache(
        base_metadata, manifest_hash
    )
    pitch_metadata = load_json(args.pitch_augmentation_cache / "metadata.json")
    validate_cache(pitch_metadata, manifest_hash, model_name, model_revision)
    if pitch_metadata.get("role", "training") != "training":
        raise ValueError("Pitch augmentation cache must contain training records")
    development_pitch_metadata = None
    if args.development_pitch_cache is not None:
        development_pitch_metadata = load_json(
            args.development_pitch_cache / "metadata.json"
        )
        validate_cache(
            development_pitch_metadata, manifest_hash, model_name, model_revision
        )
        if development_pitch_metadata.get("role") != "development":
            raise ValueError("Development pitch cache has the wrong role")

    parts = POOLING_PARTS[args.pooling]
    input_dim = embedding_dim * len(parts)
    tables = pitch_targets(manifest)
    training_raw = [
        record for record in manifest["records"] if record["role"] == "training"
    ]
    training, duplicate_count = deduplicate_recordings(training_raw)
    development = [
        record for record in manifest["records"] if record["role"] == "development"
    ]
    labels = np.asarray(
        [int(record["truth_index"]) for record in training], dtype=np.int64
    )
    groups, _ = leakage_groups(training)
    folds = fixed_folds(labels, groups)
    train_indices = np.flatnonzero(folds != args.validation_fold)
    valid_indices = np.flatnonzero(folds == args.validation_fold)
    train_records = subset(training, train_indices)
    valid_records = subset(training, valid_indices)

    print(
        f"pooling={args.pooling} input_dim={input_dim} train={len(train_records)} "
        f"validation={len(valid_records)} development={len(development)}",
        flush=True,
    )
    train_data = load_training_vectors(
        train_records,
        args.embedding_cache,
        args.pitch_augmentation_cache,
        tables,
        parts,
        embedding_dim,
    )
    valid_vectors = load_clean_vectors(
        valid_records, args.embedding_cache, parts, embedding_dim
    )
    valid_labels = torch.tensor(
        [int(record["truth_index"]) for record in valid_records], dtype=torch.long
    )

    validation_states = []
    run_reports = []
    selected_epochs = []
    for seed in args.seeds:
        best_epoch, best_accuracy, best_state, history = train_selected(
            seed,
            train_data,
            valid_vectors,
            valid_labels,
            input_dim,
            args,
        )
        validation_states.append(best_state)
        selected_epochs.append(best_epoch)
        run_reports.append(
            {
                "seed": seed,
                "best_epoch": best_epoch,
                "best_validation_exact": best_accuracy,
                "history": history,
            }
        )

    validation_models = []
    for state in validation_states:
        model = PoolingHead(
            input_dim, args.hidden_dims, args.dropout, args.layer_norm
        )
        model.load_state_dict(state)
        validation_models.append(model.to(args.device).eval())
    validation_posteriors = aligned_tta_posteriors(
        validation_models,
        valid_records,
        args.embedding_cache,
        args.pitch_augmentation_cache,
        tables,
        parts,
        embedding_dim,
        args,
    )
    validation_exact = int(
        (validation_posteriors.argmax(dim=1) == valid_labels).sum().item()
    )
    print(
        f"validation TTA exact={validation_exact}/{len(valid_records)} "
        f"({validation_exact / len(valid_records):.1%})",
        flush=True,
    )

    full_data = load_training_vectors(
        training,
        args.embedding_cache,
        args.pitch_augmentation_cache,
        tables,
        parts,
        embedding_dim,
    )
    final_models = []
    final_states = []
    for seed, epochs in zip(args.seeds, selected_epochs):
        model = train_full(seed, epochs, full_data, input_dim, args)
        final_states.append(
            {
                name: value.detach().cpu().clone()
                for name, value in model.state_dict().items()
            }
        )
        final_models.append(model)

    development_posteriors = aligned_tta_posteriors(
        final_models,
        development,
        args.embedding_cache,
        args.development_pitch_cache,
        tables,
        parts,
        embedding_dim,
        args,
    )
    development_labels = torch.tensor(
        [int(record["truth_index"]) for record in development], dtype=torch.long
    )
    development_exact = int(
        (development_posteriors.argmax(dim=1) == development_labels).sum().item()
    )
    print(
        f"development exact={development_exact}/{len(development)} "
        f"({development_exact / len(development):.1%})",
        flush=True,
    )

    checkpoint_payload = {
        "schema_version": 1,
        "model": "tunelock/myna-track-pooling-head",
        "manifest_sha256": manifest_hash,
        "base_model": base_metadata,
        "pitch_augmentation": pitch_metadata,
        "pooling": args.pooling,
        "pooling_parts": list(parts),
        "embedding_dim": embedding_dim,
        "input_dim": input_dim,
        "hidden_dims": args.hidden_dims,
        "dropout": args.dropout,
        "layer_norm": args.layer_norm,
        "validation_fold": args.validation_fold,
        "seeds": args.seeds,
        "epochs": selected_epochs,
        "state_dicts": final_states,
        "validation_state_dicts": validation_states,
    }
    args.checkpoint.parent.mkdir(parents=True, exist_ok=True)
    temporary = args.checkpoint.with_name(
        f"{args.checkpoint.name}.part.{os.getpid()}"
    )
    torch.save(checkpoint_payload, temporary)
    os.replace(temporary, args.checkpoint)
    checkpoint_hash = hashlib.sha256(args.checkpoint.read_bytes()).hexdigest()

    protocol = (
        f"track-level {args.pooling} pooling; fixed artist/recording-disjoint "
        f"validation fold {args.validation_fold}; Rust-aligned pitch shifts; "
        f"TTA={args.development_pitch_cache is not None}/{args.tta_aggregation}; "
        f"seeds={args.seeds}"
    )
    write_jsonl(
        args.output,
        development,
        manifest["canonical_labels"],
        development_posteriors,
        f"checkpoint:{checkpoint_hash[:16]}",
        protocol,
    )
    report = {
        "schema_version": 1,
        "experiment": "myna-track-pooling-head",
        "manifest_sha256": manifest_hash,
        "base_model": base_metadata,
        "dependencies": {
            "numpy": np.__version__,
            "scikit_learn": sklearn.__version__,
            "torch": torch.__version__,
        },
        "hyperparameters": {
            "pooling": args.pooling,
            "input_dim": input_dim,
            "hidden_dims": args.hidden_dims,
            "dropout": args.dropout,
            "layer_norm": args.layer_norm,
            "learning_rate": args.learning_rate,
            "weight_decay": args.weight_decay,
            "batch_size": args.batch_size,
            "amp": args.amp,
            "max_epochs": args.epochs,
            "patience": args.patience,
            "seeds": args.seeds,
            "tta_aggregation": args.tta_aggregation,
        },
        "split": {
            "training_records": len(train_records),
            "validation_records": len(valid_records),
            "full_training_records": len(training),
            "development_records": len(development),
            "exact_duplicate_records_removed": duplicate_count,
            "artist_recording_component_overlap": 0,
        },
        "validation_tta_exact": validation_exact,
        "validation_tta_n": len(valid_records),
        "development_exact": development_exact,
        "development_n": len(development),
        "seed_runs": run_reports,
        "elapsed_seconds": time.perf_counter() - started,
        "warning": (
            "GiantSteps-key is a repeatedly observed development benchmark, "
            "not a sealed final holdout."
        ),
    }
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
