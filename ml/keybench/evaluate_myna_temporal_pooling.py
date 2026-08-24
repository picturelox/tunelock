#!/usr/bin/env python3
"""Select and apply a leakage-safe temporal pooling rule for Myna key logits.

The existing head emits one 24-key logit vector per approximately 5.85-second
chunk and averages those vectors. This bakeoff compares that baseline with a
small, auditable family of robust section-pooling rules. Selection is allowed
only on one artist/recording-disjoint MTG validation fold. Development mode
requires the frozen selection artifact and cannot choose another rule.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
from pathlib import Path
from typing import Any, Callable

import numpy as np
import torch

from evaluate_myna_tta import load_cached_embeddings, load_metadata
from train_myna_head import (
    KeyHead,
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


KEY_COUNT = 24


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Select or apply robust temporal pooling for a trained Myna head"
    )
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--embedding-cache", required=True, type=Path)
    parser.add_argument("--pitch-cache", required=True, type=Path)
    parser.add_argument("--checkpoint", required=True, type=Path)
    parser.add_argument("--selection", required=True, type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--report", type=Path)
    parser.add_argument(
        "--role",
        required=True,
        choices=("validation", "development"),
        help="Validation selects a rule; development only applies a frozen rule.",
    )
    parser.add_argument("--batch-size", type=int, default=512)
    parser.add_argument("--semitones", type=int, nargs="+")
    parser.add_argument(
        "--allow-training-pitch-method-mismatch",
        action="store_true",
        help="Allow a named runtime-view ablation against a differently augmented head.",
    )
    parser.add_argument("--device", default="cuda" if torch.cuda.is_available() else "cpu")
    return parser.parse_args()


def canonical_candidates() -> list[dict[str, Any]]:
    """Fixed family ordered from simplest to most selective for stable tie breaks."""

    return [
        {"id": "mean-logits", "kind": "mean_logits"},
        {"id": "mean-probabilities", "kind": "mean_probabilities"},
        {"id": "median-logits", "kind": "median_logits"},
        {"id": "trimmed-logits-10", "kind": "trimmed_logits", "fraction": 0.10},
        {"id": "trimmed-logits-20", "kind": "trimmed_logits", "fraction": 0.20},
        {"id": "trimmed-logits-30", "kind": "trimmed_logits", "fraction": 0.30},
        {"id": "central-minus-1", "kind": "central_logits", "edge_chunks": 1},
        {"id": "central-minus-2", "kind": "central_logits", "edge_chunks": 2},
        {"id": "central-minus-3", "kind": "central_logits", "edge_chunks": 3},
        {"id": "confidence-logits-2", "kind": "confidence_logits", "alpha": 2.0},
        {"id": "confidence-logits-4", "kind": "confidence_logits", "alpha": 4.0},
        {"id": "confidence-logits-8", "kind": "confidence_logits", "alpha": 8.0},
        {"id": "consensus-logits-2", "kind": "consensus_logits", "alpha": 2.0},
        {"id": "consensus-logits-4", "kind": "consensus_logits", "alpha": 4.0},
        {"id": "consensus-logits-8", "kind": "consensus_logits", "alpha": 8.0},
        {"id": "top-confidence-50", "kind": "top_confidence_logits", "fraction": 0.50},
        {"id": "top-confidence-75", "kind": "top_confidence_logits", "fraction": 0.75},
    ]


def stable_json_hash(value: Any) -> str:
    encoded = json.dumps(value, sort_keys=True, separators=(",", ":")).encode("utf-8")
    return hashlib.sha256(encoded).hexdigest()


def normalized(values: torch.Tensor) -> torch.Tensor:
    return values / values.sum(dim=-1, keepdim=True).clamp_min(1e-12)


def pool_record_logits(logits: torch.Tensor, config: dict[str, Any]) -> torch.Tensor:
    """Return one normalized posterior from ordered chunk logits."""

    if logits.ndim != 2 or logits.shape[0] < 1 or logits.shape[1] != KEY_COUNT:
        raise ValueError(f"Expected non-empty chunk logits with shape (*, 24), got {logits.shape}")
    if not torch.isfinite(logits).all():
        raise ValueError("Temporal pool received non-finite logits")

    kind = str(config["kind"])
    if kind == "mean_logits":
        return logits.mean(dim=0).softmax(dim=0)
    if kind == "mean_probabilities":
        return normalized(logits.softmax(dim=1).mean(dim=0))
    if kind == "median_logits":
        return logits.median(dim=0).values.softmax(dim=0)
    if kind == "trimmed_logits":
        fraction = float(config["fraction"])
        trim = int(math.floor(len(logits) * fraction))
        if trim < 1 or 2 * trim >= len(logits):
            raise ValueError(f"Invalid trim={trim} for {len(logits)} chunks")
        ordered = logits.sort(dim=0).values
        return ordered[trim:-trim].mean(dim=0).softmax(dim=0)
    if kind == "central_logits":
        edge = int(config["edge_chunks"])
        if edge < 1 or 2 * edge >= len(logits):
            raise ValueError(f"Invalid edge crop={edge} for {len(logits)} chunks")
        return logits[edge:-edge].mean(dim=0).softmax(dim=0)

    probabilities = logits.softmax(dim=1)
    entropy = -(probabilities * probabilities.clamp_min(1e-12).log()).sum(dim=1)
    confidence = 1.0 - entropy / math.log(KEY_COUNT)
    if kind == "confidence_logits":
        weights = (float(config["alpha"]) * confidence).softmax(dim=0)
        return torch.sum(weights[:, None] * logits, dim=0).softmax(dim=0)
    if kind == "consensus_logits":
        consensus = normalized(probabilities.mean(dim=0))
        midpoint = 0.5 * (probabilities + consensus[None, :])
        js = 0.5 * torch.sum(
            probabilities
            * (probabilities.clamp_min(1e-12).log() - midpoint.clamp_min(1e-12).log()),
            dim=1,
        )
        js += 0.5 * torch.sum(
            consensus[None, :]
            * (consensus.clamp_min(1e-12).log() - midpoint.clamp_min(1e-12).log()),
            dim=1,
        )
        weights = (-float(config["alpha"]) * js).softmax(dim=0)
        return torch.sum(weights[:, None] * logits, dim=0).softmax(dim=0)
    if kind == "top_confidence_logits":
        keep = max(1, int(math.ceil(len(logits) * float(config["fraction"]))))
        indices = torch.topk(confidence, keep, sorted=False).indices
        return logits[indices].mean(dim=0).softmax(dim=0)
    raise ValueError(f"Unsupported temporal pooling kind: {kind}")


def pool_tracks(
    chunk_logits: torch.Tensor,
    record_indices: torch.Tensor,
    record_count: int,
    config: dict[str, Any],
) -> torch.Tensor:
    counts = torch.bincount(record_indices, minlength=record_count).tolist()
    if any(count < 1 for count in counts):
        raise ValueError("Every temporal record must contain at least one chunk")
    chunks = torch.split(chunk_logits, counts)
    return torch.stack([pool_record_logits(value, config) for value in chunks])


def atomic_json(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f"{path.name}.part.{os.getpid()}")
    temporary.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")
    os.replace(temporary, path)


def checkpoint_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def score(posteriors: torch.Tensor, truth: torch.Tensor) -> dict[str, float | int]:
    exact = int((posteriors.argmax(dim=1) == truth).sum().item())
    nll = float(
        -torch.log(posteriors[torch.arange(len(truth)), truth].clamp_min(1e-12)).sum().item()
    )
    return {"exact": exact, "total": len(truth), "accuracy": exact / len(truth), "nll": nll}


def main() -> int:
    args = parse_args()
    if args.role == "validation" and args.selection.exists():
        raise FileExistsError(f"Refusing to overwrite frozen selection: {args.selection}")
    if args.role == "development" and not args.selection.exists():
        raise FileNotFoundError(f"Frozen selection does not exist: {args.selection}")
    if args.output is not None and args.output.exists():
        raise FileExistsError(f"Refusing to overwrite output: {args.output}")

    candidates = canonical_candidates()
    candidate_hash = stable_json_hash(candidates)
    manifest = load_metadata(args.manifest)
    manifest_hash = sha256(args.manifest)
    checkpoint = torch.load(args.checkpoint, map_location="cpu", weights_only=False)
    if checkpoint.get("manifest_sha256") != manifest_hash:
        raise ValueError("Checkpoint was trained from a different manifest")

    embedding_dim = int(checkpoint.get("embedding_dim", 384))
    source_embedding_dim = int(checkpoint.get("source_embedding_dim", embedding_dim))
    raw_slice = checkpoint.get("embedding_slice", [0, embedding_dim])
    embedding_slice = (int(raw_slice[0]), int(raw_slice[1]))
    if embedding_slice[1] - embedding_slice[0] != embedding_dim:
        raise ValueError("Checkpoint embedding slice is inconsistent")

    base_metadata = load_metadata(args.embedding_cache / "metadata.json")
    checkpoint_base = checkpoint.get("base_model", {})
    if (
        base_metadata.get("manifest_sha256") != manifest_hash
        or int(base_metadata.get("embedding_dim", 0)) != source_embedding_dim
        or base_metadata.get("model") != checkpoint_base.get("model")
        or base_metadata.get("model_revision") != checkpoint_base.get("model_revision")
    ):
        raise ValueError("Base cache does not match checkpoint provenance")

    training_raw = [record for record in manifest["records"] if record["role"] == "training"]
    training, _ = deduplicate_recordings(training_raw)
    validation_fold: int | None = None
    if args.role == "validation":
        labels = np.asarray([int(record["truth_index"]) for record in training], dtype=np.int64)
        groups, _ = leakage_groups(training)
        folds = fixed_folds(labels, groups)
        validation_fold = int(checkpoint.get("validation_fold", -1))
        if validation_fold not in range(5):
            raise ValueError("Validation checkpoint does not identify a held-out fold")
        records = [record for index, record in enumerate(training) if folds[index] == validation_fold]
        states = checkpoint.get("validation_state_dicts", [])
        cache_role = "training"
        selected_ids = {candidate["id"] for candidate in candidates}
    else:
        selection = load_metadata(args.selection)
        if (
            selection.get("schema_version") != 1
            or selection.get("experiment") != "myna-temporal-pooling-selection"
            or selection.get("manifest_sha256") != manifest_hash
            or selection.get("candidate_contract_sha256") != candidate_hash
        ):
            raise ValueError("Frozen temporal selection has incompatible provenance")
        records = [record for record in manifest["records"] if record["role"] == "development"]
        states = checkpoint.get("state_dicts", [])
        cache_role = "development"
        selected_ids = {"mean-logits", str(selection["selected"]["id"])}

    if not records or not states:
        raise ValueError(f"No records or model states available for role={args.role}")

    pitch_metadata = load_metadata(args.pitch_cache / "metadata.json")
    available_shifts = validate_pitch_cache_metadata(
        pitch_metadata,
        manifest_hash=manifest_hash,
        base_metadata=checkpoint_base,
        required_role=cache_role,
    )
    training_pitch = checkpoint.get("pitch_augmentation") or {}
    training_method = training_pitch.get("pitch_method")
    evaluation_method = pitch_metadata.get("pitch_method")
    if training_method != evaluation_method and not args.allow_training_pitch_method_mismatch:
        raise ValueError(
            f"Pitch-method mismatch: training={training_method}, evaluation={evaluation_method}"
        )

    shifts = sorted(args.semitones if args.semitones is not None else available_shifts)
    target_tables = {
        int(item["semitones"]): torch.tensor(item["target_by_source_index"], dtype=torch.long)
        for item in manifest["pitch_shift_targets"]
    }
    if (
        not shifts
        or len(set(shifts)) != len(shifts)
        or any(shift == 0 or shift not in available_shifts or shift not in target_tables for shift in shifts)
    ):
        raise ValueError("Requested shifts are incomplete or incompatible")

    models = []
    for state in states:
        model = KeyHead(checkpoint["hidden_dims"], float(checkpoint["dropout"]), embedding_dim)
        model.load_state_dict(state)
        models.append(model.to(args.device).eval())

    active = [candidate for candidate in candidates if candidate["id"] in selected_ids]
    totals = {
        candidate["id"]: [torch.zeros((len(records), KEY_COUNT)) for _ in models]
        for candidate in active
    }
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
            for candidate in active:
                values = pool_tracks(chunk_logits, record_indices, len(records), candidate)
                if shift:
                    values = values[:, target_tables[shift]]
                totals[candidate["id"]][model_index] += values
        print(
            f"role={args.role} transform={transform_index}/{len(transforms)} "
            f"shift={shift:+d} records={len(records)} rules={len(active)}",
            flush=True,
        )

    posteriors = {}
    for candidate in active:
        seed_values = [normalized(total / len(transforms)) for total in totals[candidate["id"]]]
        posteriors[candidate["id"]] = normalized(torch.stack(seed_values).mean(dim=0))
    truth = torch.tensor([int(record["truth_index"]) for record in records], dtype=torch.long)
    metrics = {candidate["id"]: score(posteriors[candidate["id"]], truth) for candidate in active}

    common = {
        "schema_version": 1,
        "manifest_sha256": manifest_hash,
        "candidate_contract_sha256": candidate_hash,
        "script_sha256": sha256(Path(__file__)),
        "checkpoint_sha256": checkpoint_sha256(args.checkpoint),
        "head_contract_revision": checkpoint.get("head_contract_revision"),
        "base_model_revision": checkpoint_base.get("model_revision"),
        "training_pitch_method": training_method,
        "evaluation_pitch_method": evaluation_method,
        "pitch_method_mismatch_allowed": bool(args.allow_training_pitch_method_mismatch),
        "shifts": [0, *shifts],
        "seed_models": len(models),
    }

    if args.role == "validation":
        # Exact is primary; NLL breaks ties; canonical order favors simpler rules.
        ranking = {candidate["id"]: index for index, candidate in enumerate(candidates)}
        selected = max(
            active,
            key=lambda candidate: (
                int(metrics[candidate["id"]]["exact"]),
                -float(metrics[candidate["id"]]["nll"]),
                -ranking[candidate["id"]],
            ),
        )
        artifact = {
            **common,
            "experiment": "myna-temporal-pooling-selection",
            "role": "training material; artist/recording-disjoint held-out validation",
            "validation_fold": validation_fold,
            "candidates": candidates,
            "metrics": metrics,
            "selected": selected,
            "warning": "Selection used MTG validation only; GiantSteps labels were not read.",
        }
        atomic_json(args.selection, artifact)
        print(
            f"selected={selected['id']} exact={metrics[selected['id']]['exact']}/"
            f"{len(records)} baseline={metrics['mean-logits']['exact']}/{len(records)}",
            flush=True,
        )
        if args.report is not None:
            atomic_json(args.report, artifact)
        return 0

    selected = load_metadata(args.selection)["selected"]
    selected_posterior = posteriors[str(selected["id"])]
    result = {
        **common,
        "experiment": "myna-temporal-pooling-application",
        "selection_sha256": sha256(args.selection),
        "selected": selected,
        "metrics": metrics,
        "role": "development benchmark; not an untouched final test",
    }
    if args.report is not None:
        atomic_json(args.report, result)
    if args.output is not None:
        protocol = (
            f"frozen MTG-fold temporal pooling={selected['id']}; "
            f"pitch views={[0, *shifts]}; probability-space view average"
        )
        revision = (
            f"checkpoint:{common['checkpoint_sha256'][:16]};"
            f"selection:{result['selection_sha256'][:16]}"
        )
        write_jsonl(
            args.output,
            records,
            manifest["canonical_labels"],
            selected_posterior,
            revision,
            protocol,
            model_name=str(checkpoint.get("model", "tunelock/myna-mtg-head")),
            corpus_role="development benchmark; not an untouched final test",
            metadata_extra={
                "head_contract_revision": checkpoint.get("head_contract_revision"),
                "temporal_selection_sha256": result["selection_sha256"],
            },
        )
    print(
        f"applied={selected['id']} exact={metrics[str(selected['id'])]['exact']}/{len(records)} "
        f"baseline={metrics['mean-logits']['exact']}/{len(records)}",
        flush=True,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
