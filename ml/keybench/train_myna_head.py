#!/usr/bin/env python3
"""Train and evaluate an independent Myna key head on leakage-controlled data.

Key strings are never parsed here. Numeric targets and the ordered output labels
come from the Rust-generated corpus manifest. Model selection uses only a fixed,
artist/recording-group-disjoint MTG validation fold; GiantSteps-key is read only
after the epoch count has been selected and the final heads have been trained.
"""

from __future__ import annotations

import argparse
from collections import Counter, defaultdict
import copy
import hashlib
import json
import os
from pathlib import Path
import random
import re
import time
from typing import Any, Iterable
import unicodedata

import numpy as np
from sklearn.model_selection import StratifiedGroupKFold
import sklearn
import torch
from torch import nn


KEY_COUNT = 24
EMBEDDING_DIM = 384
ARTIST_SEPARATOR = re.compile(
    r"\s*(?:,|&|;|/|\bfeat\.?\b|\bfeaturing\b|\bvs\.?\b|\bx\b)\s*",
    flags=re.IGNORECASE,
)
SAFE_ID = re.compile(r"^[A-Za-z0-9._-]+$")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Train an MTG-supervised Myna key head")
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--embedding-cache", required=True, type=Path)
    parser.add_argument("--pitch-augmentation-cache", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--report", type=Path)
    parser.add_argument("--checkpoint", type=Path)
    parser.add_argument("--audit-only", action="store_true")
    parser.add_argument("--validation-fold", type=int, default=0, choices=range(5))
    parser.add_argument("--seeds", type=int, nargs="+", default=[41, 42, 43])
    parser.add_argument("--epochs", type=int, default=100)
    parser.add_argument("--patience", type=int, default=15)
    parser.add_argument("--batch-size", type=int, default=64)
    parser.add_argument("--learning-rate", type=float, default=3e-4)
    parser.add_argument("--weight-decay", type=float, default=1e-5)
    parser.add_argument("--hidden-dims", type=int, nargs="+", default=[2048])
    parser.add_argument("--dropout", type=float, default=0.75)
    parser.add_argument(
        "--amp",
        action="store_true",
        help="Use CUDA float16 autocast/gradient scaling for the MLP experiment.",
    )
    parser.add_argument("--device", default="cuda" if torch.cuda.is_available() else "cpu")
    return parser.parse_args()


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def load_manifest(path: Path) -> dict[str, Any]:
    data = json.loads(path.read_text(encoding="utf-8"))
    if data.get("schema_version") != 1 or len(data.get("canonical_labels", [])) != KEY_COUNT:
        raise ValueError("Expected schema-1 Rust key corpus manifest with 24 labels")
    return data


def normalized_artist_tokens(value: str) -> tuple[str, ...]:
    value = unicodedata.normalize("NFKD", value).encode("ascii", "ignore").decode("ascii")
    tokens = []
    for part in ARTIST_SEPARATOR.split(value.casefold()):
        normalized = "".join(character for character in part if character.isalnum())
        if normalized:
            tokens.append(normalized)
    return tuple(sorted(set(tokens)))


class DisjointSet:
    def __init__(self, size: int) -> None:
        self.parent = list(range(size))
        self.rank = [0] * size

    def find(self, value: int) -> int:
        while self.parent[value] != value:
            self.parent[value] = self.parent[self.parent[value]]
            value = self.parent[value]
        return value

    def union(self, left: int, right: int) -> None:
        left = self.find(left)
        right = self.find(right)
        if left == right:
            return
        if self.rank[left] < self.rank[right]:
            left, right = right, left
        self.parent[right] = left
        if self.rank[left] == self.rank[right]:
            self.rank[left] += 1


def leakage_groups(records: list[dict[str, Any]]) -> tuple[np.ndarray, list[tuple[str, ...]]]:
    sets = DisjointSet(len(records))
    token_owner: dict[str, int] = {}
    hash_owner: dict[str, int] = {}
    all_tokens: list[tuple[str, ...]] = []
    for index, record in enumerate(records):
        tokens = normalized_artist_tokens(str(record.get("artist", "")))
        all_tokens.append(tokens)
        for token in tokens:
            if token in token_owner:
                sets.union(index, token_owner[token])
            else:
                token_owner[token] = index
        recording_hash = str(record.get("recording_md5") or "").casefold()
        if recording_hash:
            if recording_hash in hash_owner:
                sets.union(index, hash_owner[recording_hash])
            else:
                hash_owner[recording_hash] = index

    # Empty artist metadata remains a distinct recording group.
    roots = [sets.find(index) for index in range(len(records))]
    root_to_group = {root: group for group, root in enumerate(sorted(set(roots)))}
    return np.asarray([root_to_group[root] for root in roots], dtype=np.int64), all_tokens


def fixed_folds(labels: np.ndarray, groups: np.ndarray) -> np.ndarray:
    folds = np.full(len(labels), -1, dtype=np.int64)
    splitter = StratifiedGroupKFold(n_splits=5, shuffle=True, random_state=20250823)
    for fold, (_, test_indices) in enumerate(splitter.split(np.zeros(len(labels)), labels, groups)):
        folds[test_indices] = fold
    if np.any(folds < 0):
        raise RuntimeError("Fold assignment did not cover every training record")
    return folds


def deduplicate_recordings(records: list[dict[str, Any]]) -> tuple[list[dict[str, Any]], int]:
    seen: dict[str, int] = {}
    kept: list[dict[str, Any]] = []
    removed = 0
    for record in records:
        recording_hash = str(record.get("recording_md5") or "")
        if not recording_hash:
            kept.append(record)
            continue
        if recording_hash in seen:
            prior = kept[seen[recording_hash]]
            if int(prior["truth_index"]) != int(record["truth_index"]):
                raise ValueError(
                    f"Conflicting labels for duplicate audio {prior['id']} and {record['id']}"
                )
            removed += 1
            continue
        seen[recording_hash] = len(kept)
        kept.append(record)
    return kept, removed


def split_audit(
    records: list[dict[str, Any]],
    groups: np.ndarray,
    tokens: list[tuple[str, ...]],
    folds: np.ndarray,
    validation_fold: int,
) -> dict[str, Any]:
    train_indices = np.flatnonzero(folds != validation_fold)
    valid_indices = np.flatnonzero(folds == validation_fold)
    train_tokens = {token for index in train_indices for token in tokens[index]}
    valid_tokens = {token for index in valid_indices for token in tokens[index]}
    train_hashes = {
        str(records[index].get("recording_md5"))
        for index in train_indices
        if records[index].get("recording_md5")
    }
    valid_hashes = {
        str(records[index].get("recording_md5"))
        for index in valid_indices
        if records[index].get("recording_md5")
    }
    train_groups = set(groups[train_indices].tolist())
    valid_groups = set(groups[valid_indices].tolist())
    if train_tokens & valid_tokens or train_hashes & valid_hashes or train_groups & valid_groups:
        raise RuntimeError("Artist/recording leakage found in fixed validation split")
    return {
        "records": len(records),
        "components": len(set(groups.tolist())),
        "largest_component": max(Counter(groups.tolist()).values()),
        "train_records": len(train_indices),
        "validation_records": len(valid_indices),
        "train_artist_tokens": len(train_tokens),
        "validation_artist_tokens": len(valid_tokens),
        "artist_token_overlap": 0,
        "recording_hash_overlap": 0,
        "component_overlap": 0,
        "fold_counts": dict(sorted(Counter(folds.tolist()).items())),
        "validation_label_counts": dict(
            sorted(Counter(int(records[index]["truth_index"]) for index in valid_indices).items())
        ),
    }


def embedding_path(root: Path, record: dict[str, Any]) -> Path:
    track_id = str(record["id"])
    corpus = str(record["corpus"])
    if not SAFE_ID.fullmatch(track_id) or not SAFE_ID.fullmatch(corpus):
        raise ValueError(f"Unsafe manifest identity: {corpus}/{track_id}")
    return root / corpus / f"{track_id}.npy"


def pitch_embedding_path(root: Path, semitones: int, record: dict[str, Any]) -> Path:
    return root / f"shift_{semitones:+d}" / record["corpus"] / f"{record['id']}.npy"


def load_embeddings(
    records: list[dict[str, Any]],
    cache_root: Path,
    pitch_cache_root: Path | None = None,
    pitch_targets: dict[int, list[int]] | None = None,
) -> tuple[torch.Tensor, torch.Tensor, torch.Tensor]:
    chunks: list[np.ndarray] = []
    labels: list[int] = []
    record_indices: list[int] = []
    for record_index, record in enumerate(records):
        sources = [(embedding_path(cache_root, record), int(record["truth_index"]))]
        if pitch_cache_root is not None:
            if not pitch_targets:
                raise ValueError("Pitch cache was supplied without Rust target tables")
            source_index = int(record["truth_index"])
            sources.extend(
                (
                    pitch_embedding_path(pitch_cache_root, semitones, record),
                    targets[source_index],
                )
                for semitones, targets in sorted(pitch_targets.items())
                if semitones != 0
            )

        for path, target in sources:
            value = np.load(path, allow_pickle=False)
            if value.ndim != 2 or value.shape[0] < 1 or value.shape[1] != EMBEDDING_DIM:
                raise ValueError(f"Invalid embedding cache {path}: {value.shape}")
            if not np.isfinite(value).all():
                raise ValueError(f"Non-finite embedding cache: {path}")
            chunks.append(np.asarray(value, dtype=np.float32))
            labels.extend([target] * len(value))
            record_indices.extend([record_index] * len(value))
    return (
        torch.from_numpy(np.concatenate(chunks)),
        torch.tensor(labels, dtype=torch.long),
        torch.tensor(record_indices, dtype=torch.long),
    )


class KeyHead(nn.Module):
    def __init__(self, hidden_dims: list[int], dropout: float) -> None:
        super().__init__()
        if not hidden_dims or any(dimension < 1 for dimension in hidden_dims):
            raise ValueError("At least one positive hidden dimension is required")
        layers: list[nn.Module] = []
        input_dim = EMBEDDING_DIM
        for index, hidden_dim in enumerate(hidden_dims):
            layers.extend((nn.Linear(input_dim, hidden_dim), nn.ReLU()))
            # This matches the published GiantSteps head: dropout separates the
            # two hidden layers rather than following every hidden layer.
            if index == 0:
                layers.append(nn.Dropout(dropout))
            input_dim = hidden_dim
        layers.append(nn.Linear(input_dim, KEY_COUNT))
        self.layers = nn.Sequential(*layers)

    def forward(self, inputs: torch.Tensor) -> torch.Tensor:
        return self.layers(inputs)


def seed_everything(seed: int) -> None:
    random.seed(seed)
    np.random.seed(seed)
    torch.manual_seed(seed)
    if torch.cuda.is_available():
        torch.cuda.manual_seed_all(seed)
    torch.backends.cudnn.deterministic = True
    torch.backends.cudnn.benchmark = False


def batched_logits(
    model: nn.Module,
    embeddings: torch.Tensor,
    batch_size: int,
    device: str,
) -> torch.Tensor:
    model.eval()
    values = []
    with torch.inference_mode():
        for start in range(0, len(embeddings), batch_size):
            values.append(model(embeddings[start : start + batch_size].to(device)).cpu())
    return torch.cat(values)


def aggregate_track_logits(
    chunk_logits: torch.Tensor, record_indices: torch.Tensor, record_count: int
) -> torch.Tensor:
    totals = torch.zeros((record_count, KEY_COUNT), dtype=chunk_logits.dtype)
    counts = torch.zeros((record_count, 1), dtype=chunk_logits.dtype)
    totals.index_add_(0, record_indices, chunk_logits)
    counts.index_add_(0, record_indices, torch.ones((len(record_indices), 1)))
    return totals / counts.clamp_min(1)


def validation_accuracy(
    model: nn.Module,
    embeddings: torch.Tensor,
    record_indices: torch.Tensor,
    record_labels: torch.Tensor,
    batch_size: int,
    device: str,
) -> tuple[float, torch.Tensor]:
    chunk_logits = batched_logits(model, embeddings, batch_size, device)
    logits = aggregate_track_logits(chunk_logits, record_indices, len(record_labels))
    accuracy = float((logits.argmax(dim=1) == record_labels).float().mean().item())
    return accuracy, logits.softmax(dim=1)


def train_epoch(
    model: nn.Module,
    optimizer: torch.optim.Optimizer,
    embeddings: torch.Tensor,
    labels: torch.Tensor,
    batch_size: int,
    device: str,
    generator: torch.Generator,
    amp: bool,
    scaler: torch.amp.GradScaler,
) -> float:
    model.train()
    order = torch.randperm(len(embeddings), generator=generator)
    losses = []
    for start in range(0, len(order), batch_size):
        indices = order[start : start + batch_size]
        inputs = embeddings[indices].to(device)
        targets = labels[indices].to(device)
        optimizer.zero_grad(set_to_none=True)
        with torch.autocast(
            device_type="cuda",
            dtype=torch.float16,
            enabled=amp,
        ):
            logits = model(inputs)
            loss = nn.functional.cross_entropy(logits, targets)
        scaler.scale(loss).backward()
        scaler.step(optimizer)
        scaler.update()
        losses.append(float(loss.detach().cpu()))
    return float(np.mean(losses))


def train_with_validation(
    seed: int,
    train_data: tuple[torch.Tensor, torch.Tensor, torch.Tensor],
    valid_data: tuple[torch.Tensor, torch.Tensor, torch.Tensor],
    valid_record_labels: torch.Tensor,
    args: argparse.Namespace,
) -> tuple[int, float, dict[str, torch.Tensor], torch.Tensor, list[dict[str, float]]]:
    seed_everything(seed)
    model = KeyHead(args.hidden_dims, args.dropout).to(args.device)
    optimizer = torch.optim.Adam(
        model.parameters(), lr=args.learning_rate, weight_decay=args.weight_decay
    )
    generator = torch.Generator().manual_seed(seed)
    scaler = torch.amp.GradScaler("cuda", enabled=args.amp)
    train_embeddings, train_labels, _ = train_data
    valid_embeddings, _, valid_record_indices = valid_data
    best_accuracy = -1.0
    best_epoch = 0
    best_state: dict[str, torch.Tensor] = {}
    best_probabilities = torch.empty(0)
    history = []
    stale = 0

    for epoch in range(1, args.epochs + 1):
        loss = train_epoch(
            model,
            optimizer,
            train_embeddings,
            train_labels,
            args.batch_size,
            args.device,
            generator,
            args.amp,
            scaler,
        )
        accuracy, probabilities = validation_accuracy(
            model,
            valid_embeddings,
            valid_record_indices,
            valid_record_labels,
            args.batch_size,
            args.device,
        )
        history.append({"epoch": epoch, "loss": loss, "validation_exact": accuracy})
        if accuracy > best_accuracy:
            best_accuracy = accuracy
            best_epoch = epoch
            best_state = {name: value.detach().cpu().clone() for name, value in model.state_dict().items()}
            best_probabilities = probabilities.clone()
            stale = 0
        else:
            stale += 1
        if epoch == 1 or epoch % 10 == 0 or stale >= args.patience:
            print(
                f"seed={seed} epoch={epoch} loss={loss:.4f} validation={accuracy:.3%} "
                f"best={best_accuracy:.3%}@{best_epoch}",
                flush=True,
            )
        if stale >= args.patience:
            break

    return best_epoch, best_accuracy, best_state, best_probabilities, history


def train_full(
    seed: int,
    epochs: int,
    data: tuple[torch.Tensor, torch.Tensor, torch.Tensor],
    args: argparse.Namespace,
) -> KeyHead:
    seed_everything(seed)
    model = KeyHead(args.hidden_dims, args.dropout).to(args.device)
    optimizer = torch.optim.Adam(
        model.parameters(), lr=args.learning_rate, weight_decay=args.weight_decay
    )
    generator = torch.Generator().manual_seed(seed)
    scaler = torch.amp.GradScaler("cuda", enabled=args.amp)
    embeddings, labels, _ = data
    for _ in range(epochs):
        train_epoch(
            model,
            optimizer,
            embeddings,
            labels,
            args.batch_size,
            args.device,
            generator,
            args.amp,
            scaler,
        )
    return model


def record_subset(
    records: list[dict[str, Any]], indices: Iterable[int]
) -> list[dict[str, Any]]:
    return [records[int(index)] for index in indices]


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
        metadata = {
            "type": "metadata",
            "schema_version": 1,
            "model": "tunelock/myna-vertical-mtg-head",
            "model_revision": revision,
            "posterior_labels": labels,
            "protocol": protocol,
        }
        handle.write(json.dumps(metadata, separators=(",", ":")) + "\n")
        for record, posterior in zip(records, posteriors.tolist()):
            prediction = {
                "type": "prediction",
                "track_id": record["id"],
                "status": "ok",
                "posterior": posterior,
            }
            handle.write(json.dumps(prediction, separators=(",", ":")) + "\n")
    os.replace(temporary, output)


def main() -> int:
    args = parse_args()
    manifest = load_manifest(args.manifest)
    manifest_hash = sha256(args.manifest)
    training_raw = [record for record in manifest["records"] if record["role"] == "training"]
    development = [record for record in manifest["records"] if record["role"] == "development"]
    training, duplicate_records_removed = deduplicate_recordings(training_raw)
    labels = np.asarray([int(record["truth_index"]) for record in training], dtype=np.int64)
    groups, artist_tokens = leakage_groups(training)
    folds = fixed_folds(labels, groups)
    audit = split_audit(training, groups, artist_tokens, folds, args.validation_fold)
    audit["exact_duplicate_records_removed"] = duplicate_records_removed
    audit["development_records"] = len(development)
    print(json.dumps(audit, sort_keys=True), flush=True)
    if args.audit_only:
        return 0
    if args.output is None or args.report is None or args.checkpoint is None:
        raise ValueError("--output, --report, and --checkpoint are required unless --audit-only")
    for path in (args.output, args.report, args.checkpoint):
        if path.exists():
            raise FileExistsError(f"Refusing to overwrite experiment artifact: {path}")

    train_indices = np.flatnonzero(folds != args.validation_fold)
    valid_indices = np.flatnonzero(folds == args.validation_fold)
    train_records = record_subset(training, train_indices)
    valid_records = record_subset(training, valid_indices)
    base_metadata_path = args.embedding_cache / "metadata.json"
    base_metadata = json.loads(base_metadata_path.read_text(encoding="utf-8"))
    if base_metadata.get("manifest_sha256") != manifest_hash:
        raise ValueError("Base embedding cache was produced from a different manifest")

    pitch_targets = None
    pitch_metadata = None
    if args.pitch_augmentation_cache is not None:
        pitch_metadata_path = args.pitch_augmentation_cache / "metadata.json"
        pitch_metadata = json.loads(pitch_metadata_path.read_text(encoding="utf-8"))
        if pitch_metadata.get("manifest_sha256") != manifest_hash:
            raise ValueError("Pitch embedding cache was produced from a different manifest")
        shifts = {int(value) for value in pitch_metadata.get("semitones", [])}
        pitch_targets = {
            int(item["semitones"]): [int(value) for value in item["target_by_source_index"]]
            for item in manifest.get("pitch_shift_targets", [])
            if int(item["semitones"]) in shifts
        }
        if set(pitch_targets) != shifts or 0 in shifts:
            raise ValueError("Pitch cache shifts do not match Rust target tables")

    train_data = load_embeddings(
        train_records,
        args.embedding_cache,
        args.pitch_augmentation_cache,
        pitch_targets,
    )
    valid_data = load_embeddings(valid_records, args.embedding_cache)
    full_data = load_embeddings(
        training,
        args.embedding_cache,
        args.pitch_augmentation_cache,
        pitch_targets,
    )
    development_data = load_embeddings(development, args.embedding_cache)
    valid_record_labels = torch.tensor(
        [int(record["truth_index"]) for record in valid_records], dtype=torch.long
    )

    started = time.perf_counter()
    seed_reports = []
    validation_probabilities = []
    validation_states = []
    final_states = []
    development_probabilities = []
    for seed in args.seeds:
        best_epoch, best_accuracy, best_state, valid_probabilities, history = train_with_validation(
            seed, train_data, valid_data, valid_record_labels, args
        )
        validation_probabilities.append(valid_probabilities)
        validation_states.append(best_state)
        seed_reports.append(
            {
                "seed": seed,
                "best_epoch": best_epoch,
                "best_validation_exact": best_accuracy,
                "history": history,
            }
        )

        final_model = train_full(seed, best_epoch, full_data, args)
        final_states.append({name: value.detach().cpu() for name, value in final_model.state_dict().items()})
        dev_chunk_logits = batched_logits(
            final_model, development_data[0], args.batch_size, args.device
        )
        dev_logits = aggregate_track_logits(
            dev_chunk_logits, development_data[2], len(development)
        )
        development_probabilities.append(dev_logits.softmax(dim=1))

    validation_ensemble = torch.stack(validation_probabilities).mean(dim=0)
    validation_exact = float(
        (validation_ensemble.argmax(dim=1) == valid_record_labels).float().mean().item()
    )
    development_ensemble = torch.stack(development_probabilities).mean(dim=0)
    script_hash = sha256(Path(__file__))
    base_revision = str(base_metadata["model_revision"])
    revision = f"myna:{base_revision};head:{script_hash[:16]}"
    augmentation_protocol = (
        f", pitch shifts={sorted(pitch_targets)}" if pitch_targets is not None else ", no pitch augmentation"
    )
    protocol = (
        f"MTG confidence=2 single-key, exact-audio deduplicated, fixed artist/recording-group "
        f"fold {args.validation_fold} for epoch selection, full-MTG retrain, seeds={args.seeds}"
        f"{augmentation_protocol}"
    )

    write_jsonl(
        args.output,
        development,
        manifest["canonical_labels"],
        development_ensemble,
        revision,
        protocol,
    )
    args.checkpoint.parent.mkdir(parents=True, exist_ok=True)
    checkpoint_temp = args.checkpoint.with_name(f"{args.checkpoint.name}.part.{os.getpid()}")
    torch.save(
        {
            "schema_version": 1,
            "model": "tunelock/myna-vertical-mtg-head",
            "base_model": base_metadata,
            "pitch_augmentation": pitch_metadata,
            "manifest_sha256": manifest_hash,
            "script_sha256": script_hash,
            "hidden_dims": args.hidden_dims,
            "dropout": args.dropout,
            "seeds": args.seeds,
            "amp": args.amp,
            "validation_fold": args.validation_fold,
            "epochs": [report["best_epoch"] for report in seed_reports],
            "validation_state_dicts": validation_states,
            "state_dicts": final_states,
        },
        checkpoint_temp,
    )
    os.replace(checkpoint_temp, args.checkpoint)

    report = {
        "schema_version": 1,
        "experiment": "myna-vertical-mtg-head",
        "manifest_sha256": manifest_hash,
        "script_sha256": script_hash,
        "base_model": base_metadata,
        "pitch_augmentation": pitch_metadata,
        "dependencies": {
            "numpy": np.__version__,
            "scikit_learn": sklearn.__version__,
            "torch": torch.__version__,
        },
        "hyperparameters": {
            "hidden_dims": args.hidden_dims,
            "dropout": args.dropout,
            "learning_rate": args.learning_rate,
            "weight_decay": args.weight_decay,
            "batch_size": args.batch_size,
            "amp": args.amp,
            "max_epochs": args.epochs,
            "patience": args.patience,
            "seeds": args.seeds,
        },
        "split_audit": audit,
        "validation_ensemble_exact": validation_exact,
        "seed_runs": seed_reports,
        "development_predictions": len(development),
        "elapsed_seconds": time.perf_counter() - started,
        "warning": "GiantSteps-key is a repeatedly observed development benchmark, not a sealed final holdout.",
    }
    args.report.parent.mkdir(parents=True, exist_ok=True)
    report_temp = args.report.with_name(f"{args.report.name}.part.{os.getpid()}")
    report_temp.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    os.replace(report_temp, args.report)
    print(
        f"validation ensemble exact={validation_exact:.1%}; development posteriors={len(development)}; "
        f"elapsed={report['elapsed_seconds']:.1f}s",
        flush=True,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
