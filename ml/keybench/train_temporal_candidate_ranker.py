#!/usr/bin/env python3
"""Train/apply a tiny temporal candidate ranker on out-of-fold Myna logits.

The expensive acoustic head stays frozen. For each candidate key this script
summarizes the ordered chunk evidence (support, rank, margin, persistence,
edges, thirds, and volatility). One shared linear scorer is trained listwise,
so the rule is transposition invariant and only a few dozen weights are added.

Training features come exclusively from five held-out MTG head states. Fold 0
selects regularization; the final ranker is then fit on all OOF tracks. Apply
mode requires that frozen JSON artifact and uses the full-MTG head on the
GiantSteps development embeddings.
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
from torch import nn

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
SEED = 20260824
WEIGHT_DECAYS = (0.0, 1e-4, 1e-3, 1e-2)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Train/apply a temporal Myna candidate ranker")
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--embedding-cache", required=True, type=Path)
    parser.add_argument("--pitch-cache", required=True, type=Path)
    parser.add_argument("--artifact", required=True, type=Path)
    parser.add_argument("--report", required=True, type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--feature-cache", type=Path)
    parser.add_argument("--mode", required=True, choices=("train", "apply"))
    parser.add_argument(
        "--fold-checkpoints",
        type=Path,
        nargs=5,
        metavar=("FOLD0", "FOLD1", "FOLD2", "FOLD3", "FOLD4"),
    )
    parser.add_argument("--checkpoint", type=Path)
    parser.add_argument("--batch-size", type=int, default=512)
    parser.add_argument("--epochs", type=int, default=80)
    parser.add_argument("--learning-rate", type=float, default=0.03)
    parser.add_argument(
        "--allow-training-pitch-method-mismatch", action="store_true"
    )
    parser.add_argument("--device", default="cuda" if torch.cuda.is_available() else "cpu")
    return parser.parse_args()


def atomic_json(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f"{path.name}.part.{os.getpid()}")
    temporary.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")
    os.replace(temporary, path)


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def feature_names() -> list[str]:
    base = [
        "logit_mean", "logit_median", "logit_std", "logit_min", "logit_max",
        "logit_q25", "logit_q75", "logit_trimmed_30",
        "prob_mean", "prob_median", "prob_std", "prob_min", "prob_max",
        "prob_q25", "prob_q75", "prob_volatility",
        "rank_mean", "rank_median", "rank_min", "rank_max",
        "top1_fraction", "top3_fraction", "top5_fraction", "top1_longest_run",
        "margin_mean", "margin_median", "margin_max", "margin_positive_fraction",
        "edge_prob_mean", "central_prob_mean", "central_minus_edge",
        "first_third_prob", "middle_third_prob", "last_third_prob",
        "middle_minus_outer", "prob_slope",
    ]
    return [*base, *(f"relative_{name}" for name in base)]


def longest_true_run(values: torch.Tensor) -> float:
    best = current = 0
    for value in values.tolist():
        current = current + 1 if value else 0
        best = max(best, current)
    return best / max(1, len(values))


def record_candidate_features(logits: torch.Tensor) -> torch.Tensor:
    if logits.ndim != 2 or logits.shape[0] < 3 or logits.shape[1] != KEY_COUNT:
        raise ValueError(f"Expected at least three ordered chunks by 24 keys, got {logits.shape}")
    probabilities = logits.softmax(dim=1)
    length = len(logits)
    trim = max(1, int(math.floor(length * 0.30)))
    ordered_logits = logits.sort(dim=0).values
    ranks = torch.argsort(torch.argsort(-probabilities, dim=1), dim=1).to(torch.float32)
    winners = probabilities.argmax(dim=1)
    best = probabilities.max(dim=1).values[:, None]
    candidate_is_best = torch.nn.functional.one_hot(winners, KEY_COUNT).bool()
    masked = probabilities.masked_fill(candidate_is_best, -1.0)
    best_other = masked.max(dim=1).values[:, None]
    margins = torch.where(candidate_is_best, probabilities - best_other, probabilities - best)
    edge_width = min(2, (length - 1) // 2)
    edge = torch.cat(
        (probabilities[:edge_width], probabilities[-edge_width:]), dim=0
    ).mean(dim=0)
    central = probabilities[edge_width:-edge_width].mean(dim=0)
    thirds = torch.tensor_split(probabilities, 3, dim=0)
    first, middle, last = (part.mean(dim=0) for part in thirds)
    time = torch.linspace(-1.0, 1.0, length, dtype=probabilities.dtype)
    slope = torch.sum(time[:, None] * probabilities, dim=0) / torch.sum(time * time)

    rows = []
    for candidate in range(KEY_COUNT):
        logit = logits[:, candidate]
        probability = probabilities[:, candidate]
        rank = ranks[:, candidate]
        margin = margins[:, candidate]
        top1 = winners == candidate
        rows.append(
            torch.stack(
                (
                    logit.mean(), logit.median(), logit.std(unbiased=False), logit.min(), logit.max(),
                    torch.quantile(logit, 0.25), torch.quantile(logit, 0.75),
                    ordered_logits[trim:-trim, candidate].mean(),
                    probability.mean(), probability.median(), probability.std(unbiased=False),
                    probability.min(), probability.max(), torch.quantile(probability, 0.25),
                    torch.quantile(probability, 0.75),
                    torch.abs(probability[1:] - probability[:-1]).mean(),
                    rank.mean() / 23.0, rank.median() / 23.0, rank.min() / 23.0,
                    rank.max() / 23.0, top1.float().mean(), (rank < 3).float().mean(),
                    (rank < 5).float().mean(),
                    torch.tensor(longest_true_run(top1), dtype=logits.dtype),
                    margin.mean(), margin.median(), margin.max(), (margin > 0).float().mean(),
                    edge[candidate], central[candidate], central[candidate] - edge[candidate],
                    first[candidate], middle[candidate], last[candidate],
                    middle[candidate] - 0.5 * (first[candidate] + last[candidate]),
                    slope[candidate],
                )
            )
        )
    raw = torch.stack(rows)
    relative = (raw - raw.mean(dim=0, keepdim=True)) / raw.std(
        dim=0, keepdim=True, unbiased=False
    ).clamp_min(1e-6)
    result = torch.cat((raw, relative), dim=1)
    if result.shape[1] != len(feature_names()) or not torch.isfinite(result).all():
        raise ValueError("Temporal candidate feature contract failed")
    return result


def feature_tracks(
    chunk_logits: torch.Tensor, record_indices: torch.Tensor, record_count: int
) -> tuple[torch.Tensor, torch.Tensor]:
    counts = torch.bincount(record_indices, minlength=record_count).tolist()
    chunks = torch.split(chunk_logits, counts)
    features = torch.stack([record_candidate_features(value) for value in chunks])
    baseline = torch.stack([value.mean(dim=0).softmax(dim=0) for value in chunks])
    return features, baseline


def checkpoint_contract(
    checkpoint: dict[str, Any], manifest_hash: str, base_metadata: dict[str, Any]
) -> tuple[int, int, tuple[int, int]]:
    if checkpoint.get("manifest_sha256") != manifest_hash:
        raise ValueError("Checkpoint was trained from a different manifest")
    dimension = int(checkpoint.get("embedding_dim", 384))
    source = int(checkpoint.get("source_embedding_dim", dimension))
    raw_slice = checkpoint.get("embedding_slice", [0, dimension])
    selected = (int(raw_slice[0]), int(raw_slice[1]))
    checkpoint_base = checkpoint.get("base_model", {})
    if (
        selected[1] - selected[0] != dimension
        or base_metadata.get("manifest_sha256") != manifest_hash
        or int(base_metadata.get("embedding_dim", 0)) != source
        or base_metadata.get("model") != checkpoint_base.get("model")
        or base_metadata.get("model_revision") != checkpoint_base.get("model_revision")
    ):
        raise ValueError("Checkpoint and embedding cache contracts differ")
    return dimension, source, selected


def extract_features(
    *,
    records: list[dict[str, Any]],
    record_folds: np.ndarray,
    checkpoints: list[Path],
    role: str,
    manifest: dict[str, Any],
    manifest_hash: str,
    embedding_cache: Path,
    pitch_cache: Path,
    batch_size: int,
    device: str,
    allow_pitch_mismatch: bool,
    feature_cache: Path | None,
) -> tuple[torch.Tensor, torch.Tensor, dict[str, Any]]:
    base_metadata = load_metadata(embedding_cache / "metadata.json")
    pitch_metadata = load_metadata(pitch_cache / "metadata.json")
    target_tables = {
        int(item["semitones"]): torch.tensor(item["target_by_source_index"], dtype=torch.long)
        for item in manifest["pitch_shift_targets"]
    }
    all_features = torch.empty((len(records), 13, KEY_COUNT, len(feature_names())))
    all_baseline = torch.empty((len(records), 13, KEY_COUNT))
    covered = torch.zeros(len(records), dtype=torch.bool)
    provenance = []

    for checkpoint_path in checkpoints:
        checkpoint = torch.load(checkpoint_path, map_location="cpu", weights_only=False)
        dimension, source, selected_slice = checkpoint_contract(
            checkpoint, manifest_hash, base_metadata
        )
        if role == "training":
            fold = int(checkpoint.get("validation_fold", -1))
            if fold not in range(5):
                raise ValueError("OOF checkpoint has no valid held-out fold")
            indices = np.flatnonzero(record_folds == fold)
            states = checkpoint.get("validation_state_dicts", [])
        else:
            fold = None
            indices = np.arange(len(records))
            states = checkpoint.get("state_dicts", [])
        if not len(indices) or not states:
            raise ValueError(f"Checkpoint has no applicable records/states: {checkpoint_path}")
        subset = [records[int(index)] for index in indices]

        cache_path = None
        cache_contract = None
        if feature_cache is not None:
            feature_cache.mkdir(parents=True, exist_ok=True)
            cache_path = feature_cache / f"{role}-fold-{fold if fold is not None else 'final'}.npz"
            cache_contract = hashlib.sha256(
                json.dumps(
                    {
                        "checkpoint_sha256": file_sha256(checkpoint_path),
                        "manifest_sha256": manifest_hash,
                        "pitch_metadata_sha256": file_sha256(pitch_cache / "metadata.json"),
                        "feature_names": feature_names(),
                        "record_ids": [record["id"] for record in subset],
                    },
                    sort_keys=True,
                ).encode("utf-8")
            ).hexdigest()
            if cache_path.exists():
                cached = np.load(cache_path, allow_pickle=False)
                stored_contract = str(cached["contract"].item())
                cached_features = cached["features"]
                cached_baseline = cached["baseline"]
                if (
                    stored_contract == cache_contract
                    and cached_features.shape
                    == (len(subset), 13, KEY_COUNT, len(feature_names()))
                    and cached_baseline.shape == (len(subset), 13, KEY_COUNT)
                ):
                    all_features[indices] = torch.from_numpy(cached_features)
                    all_baseline[indices] = torch.from_numpy(cached_baseline)
                    covered[indices] = True
                    provenance.append(
                        {
                            "path": str(checkpoint_path),
                            "sha256": file_sha256(checkpoint_path),
                            "fold": fold,
                            "head_contract_revision": checkpoint.get("head_contract_revision"),
                            "training_pitch_method": (
                                checkpoint.get("pitch_augmentation") or {}
                            ).get("pitch_method"),
                            "feature_cache": str(cache_path),
                            "feature_cache_contract": cache_contract,
                        }
                    )
                    print(f"feature-role={role} fold={fold} cache=hit records={len(subset)}")
                    continue

        available = validate_pitch_cache_metadata(
            pitch_metadata,
            manifest_hash=manifest_hash,
            base_metadata=checkpoint.get("base_model", {}),
            required_role=role,
        )
        shifts = sorted(available)
        if shifts != [-6, -5, -4, -3, -2, -1, 1, 2, 3, 4, 5, 6]:
            raise ValueError("Temporal ranker requires the complete twelve-view cache")
        training_method = (checkpoint.get("pitch_augmentation") or {}).get("pitch_method")
        evaluation_method = pitch_metadata.get("pitch_method")
        if training_method != evaluation_method and not allow_pitch_mismatch:
            raise ValueError(
                f"Pitch-method mismatch: training={training_method}, evaluation={evaluation_method}"
            )

        models = []
        for state in states:
            model = KeyHead(checkpoint["hidden_dims"], float(checkpoint["dropout"]), dimension)
            model.load_state_dict(state)
            models.append(model.to(device).eval())
        transforms: list[tuple[int, Callable[[dict[str, Any]], Path]]] = [
            (0, lambda record: embedding_path(embedding_cache, record))
        ]
        transforms.extend(
            (
                shift,
                lambda record, chosen=shift: pitch_embedding_path(pitch_cache, chosen, record),
            )
            for shift in shifts
        )
        for view_index, (shift, path_for) in enumerate(transforms):
            embeddings, local_indices = load_cached_embeddings(
                subset, path_for, source, selected_slice
            )
            seed_features = []
            seed_baselines = []
            for model in models:
                chunk_logits = batched_logits(model, embeddings, batch_size, device)
                values, baseline = feature_tracks(chunk_logits, local_indices, len(subset))
                if shift:
                    values = values[:, target_tables[shift], :]
                    baseline = baseline[:, target_tables[shift]]
                seed_features.append(values)
                seed_baselines.append(baseline)
            all_features[indices, view_index] = torch.stack(seed_features).mean(dim=0)
            all_baseline[indices, view_index] = torch.stack(seed_baselines).mean(dim=0)
            print(
                f"feature-role={role} fold={fold} view={view_index + 1}/13 shift={shift:+d} "
                f"records={len(subset)}",
                flush=True,
            )
        if cache_path is not None and cache_contract is not None:
            temporary = cache_path.with_name(f"{cache_path.stem}.part.{os.getpid()}.npz")
            np.savez_compressed(
                temporary,
                contract=np.asarray(cache_contract),
                features=all_features[indices].numpy(),
                baseline=all_baseline[indices].numpy(),
            )
            os.replace(temporary, cache_path)
        covered[indices] = True
        provenance.append(
            {
                "path": str(checkpoint_path),
                "sha256": file_sha256(checkpoint_path),
                "fold": fold,
                "head_contract_revision": checkpoint.get("head_contract_revision"),
                "training_pitch_method": training_method,
            }
        )

    if not covered.all() or not torch.isfinite(all_features).all():
        raise ValueError("Temporal feature extraction did not cover every record")
    return all_features, all_baseline, {
        "checkpoints": provenance,
        "evaluation_pitch_method": pitch_metadata.get("pitch_method"),
        "pitch_method_mismatch_allowed": allow_pitch_mismatch,
    }


class SharedLinearRanker(nn.Module):
    def __init__(self, dimensions: int) -> None:
        super().__init__()
        self.score = nn.Linear(dimensions, 1)

    def forward(self, inputs: torch.Tensor) -> torch.Tensor:
        return self.score(inputs).squeeze(-1)


def standardizer(features: torch.Tensor, track_indices: np.ndarray) -> tuple[torch.Tensor, torch.Tensor]:
    values = features[track_indices].reshape(-1, features.shape[-1])
    return values.mean(dim=0), values.std(dim=0, unbiased=False).clamp_min(1e-6)


def train_ranker(
    features: torch.Tensor,
    truth: torch.Tensor,
    track_indices: np.ndarray,
    *,
    mean: torch.Tensor,
    scale: torch.Tensor,
    weight_decay: float,
    epochs: int,
    learning_rate: float,
    device: str,
) -> SharedLinearRanker:
    torch.manual_seed(SEED)
    model = SharedLinearRanker(features.shape[-1]).to(device)
    nn.init.zeros_(model.score.weight)
    nn.init.zeros_(model.score.bias)
    optimizer = torch.optim.AdamW(
        model.parameters(), lr=learning_rate, weight_decay=weight_decay
    )
    track_tensor = torch.tensor(track_indices, dtype=torch.long)
    view_features = ((features[track_tensor] - mean) / scale).reshape(
        -1, KEY_COUNT, features.shape[-1]
    )
    targets = truth[track_tensor].repeat_interleave(features.shape[1])
    generator = torch.Generator().manual_seed(SEED)
    batch_size = 512
    model.train()
    for _ in range(epochs):
        order = torch.randperm(len(view_features), generator=generator)
        for start in range(0, len(order), batch_size):
            selected = order[start : start + batch_size]
            optimizer.zero_grad(set_to_none=True)
            scores = model(view_features[selected].to(device))
            loss = nn.functional.cross_entropy(scores, targets[selected].to(device))
            loss.backward()
            optimizer.step()
    return model.cpu().eval()


def predict(
    model: SharedLinearRanker,
    features: torch.Tensor,
    mean: torch.Tensor,
    scale: torch.Tensor,
) -> torch.Tensor:
    with torch.inference_mode():
        scores = model((features - mean) / scale)
        return scores.softmax(dim=2).mean(dim=1)


def metrics(posteriors: torch.Tensor, truth: torch.Tensor) -> dict[str, float | int]:
    exact = int((posteriors.argmax(dim=1) == truth).sum())
    nll = float(-torch.log(posteriors[torch.arange(len(truth)), truth].clamp_min(1e-12)).sum())
    return {"exact": exact, "total": len(truth), "accuracy": exact / len(truth), "nll": nll}


def model_from_artifact(artifact: dict[str, Any]) -> tuple[SharedLinearRanker, torch.Tensor, torch.Tensor]:
    names = feature_names()
    if artifact.get("feature_names") != names:
        raise ValueError("Temporal artifact feature vocabulary differs")
    model = SharedLinearRanker(len(names))
    with torch.no_grad():
        model.score.weight.copy_(torch.tensor([artifact["model"]["weights"]]))
        model.score.bias.copy_(torch.tensor([artifact["model"]["bias"]]))
    return model.eval(), torch.tensor(artifact["standardizer"]["mean"]), torch.tensor(
        artifact["standardizer"]["scale"]
    )


def main() -> int:
    args = parse_args()
    if args.artifact.exists() and args.mode == "train":
        raise FileExistsError(f"Refusing to overwrite artifact: {args.artifact}")
    if args.report.exists() or (args.output is not None and args.output.exists()):
        raise FileExistsError("Refusing to overwrite a temporal ranker result")
    manifest = load_metadata(args.manifest)
    manifest_hash = sha256(args.manifest)
    training_raw = [record for record in manifest["records"] if record["role"] == "training"]
    training, _ = deduplicate_recordings(training_raw)
    labels = np.asarray([int(record["truth_index"]) for record in training], dtype=np.int64)
    groups, _ = leakage_groups(training)
    folds = fixed_folds(labels, groups)

    if args.mode == "train":
        if args.fold_checkpoints is None or args.checkpoint is not None:
            raise ValueError("Train mode requires exactly five --fold-checkpoints")
        records = training
        features, baseline_views, provenance = extract_features(
            records=records,
            record_folds=folds,
            checkpoints=list(args.fold_checkpoints),
            role="training",
            manifest=manifest,
            manifest_hash=manifest_hash,
            embedding_cache=args.embedding_cache,
            pitch_cache=args.pitch_cache,
            batch_size=args.batch_size,
            device=args.device,
            allow_pitch_mismatch=args.allow_training_pitch_method_mismatch,
            feature_cache=args.feature_cache,
        )
        truth = torch.tensor(labels, dtype=torch.long)
        train_indices = np.flatnonzero(folds != 0)
        valid_indices = np.flatnonzero(folds == 0)
        mean, scale = standardizer(features, train_indices)
        sweep = []
        best = None
        for weight_decay in WEIGHT_DECAYS:
            model = train_ranker(
                features, truth, train_indices, mean=mean, scale=scale,
                weight_decay=weight_decay, epochs=args.epochs,
                learning_rate=args.learning_rate, device=args.device,
            )
            posterior = predict(model, features[valid_indices], mean, scale)
            result = metrics(posterior, truth[valid_indices])
            entry = {"weight_decay": weight_decay, **result}
            sweep.append(entry)
            candidate = (int(result["exact"]), -float(result["nll"]), -weight_decay, model)
            if best is None or candidate[:3] > best[:3]:
                best = candidate
            print(f"weight_decay={weight_decay:g} validation={result['exact']}/{len(valid_indices)}")
        assert best is not None
        selected_weight_decay = float(-best[2])
        full_indices = np.arange(len(records))
        full_mean, full_scale = standardizer(features, full_indices)
        final_model = train_ranker(
            features, truth, full_indices, mean=full_mean, scale=full_scale,
            weight_decay=selected_weight_decay, epochs=args.epochs,
            learning_rate=args.learning_rate, device=args.device,
        )
        full_posterior = predict(final_model, features, full_mean, full_scale)
        baseline = baseline_views.mean(dim=1)
        artifact = {
            "schema_version": 1,
            "experiment": "myna-oof-temporal-candidate-ranker",
            "manifest_sha256": manifest_hash,
            "script_sha256": sha256(Path(__file__)),
            "feature_names": feature_names(),
            "selection_fold": 0,
            "hyperparameters": {
                "epochs": args.epochs,
                "learning_rate": args.learning_rate,
                "weight_decay": selected_weight_decay,
                "seed": SEED,
            },
            "standardizer": {"mean": full_mean.tolist(), "scale": full_scale.tolist()},
            "model": {
                "kind": "shared-linear-listwise-v1",
                "weights": final_model.score.weight.detach().squeeze(0).tolist(),
                "bias": float(final_model.score.bias.detach().item()),
            },
            "provenance": provenance,
            "selection_sweep": sweep,
            "fold0_baseline": metrics(baseline[valid_indices], truth[valid_indices]),
            "training_fit": metrics(full_posterior, truth),
            "warning": "Hyperparameters selected on MTG fold 0; GiantSteps labels were not read.",
        }
        atomic_json(args.artifact, artifact)
        atomic_json(args.report, artifact)
        print(
            f"selected_weight_decay={selected_weight_decay:g} "
            f"fold0={best[0]}/{len(valid_indices)} "
            f"baseline={artifact['fold0_baseline']['exact']}/{len(valid_indices)}"
        )
        return 0

    if args.checkpoint is None or args.fold_checkpoints is not None or not args.artifact.exists():
        raise ValueError("Apply mode requires --checkpoint and an existing --artifact")
    artifact = load_metadata(args.artifact)
    if (
        artifact.get("schema_version") != 1
        or artifact.get("experiment") != "myna-oof-temporal-candidate-ranker"
        or artifact.get("manifest_sha256") != manifest_hash
    ):
        raise ValueError("Temporal ranker artifact is incompatible")
    records = [record for record in manifest["records"] if record["role"] == "development"]
    features, baseline_views, provenance = extract_features(
        records=records,
        record_folds=np.zeros(len(records), dtype=np.int64),
        checkpoints=[args.checkpoint],
        role="development",
        manifest=manifest,
        manifest_hash=manifest_hash,
        embedding_cache=args.embedding_cache,
        pitch_cache=args.pitch_cache,
        batch_size=args.batch_size,
        device=args.device,
        allow_pitch_mismatch=args.allow_training_pitch_method_mismatch,
        feature_cache=args.feature_cache,
    )
    model, mean, scale = model_from_artifact(artifact)
    posterior = predict(model, features, mean, scale)
    truth = torch.tensor([int(record["truth_index"]) for record in records], dtype=torch.long)
    baseline = baseline_views.mean(dim=1)
    report = {
        "schema_version": 1,
        "experiment": "myna-oof-temporal-candidate-ranker-application",
        "manifest_sha256": manifest_hash,
        "artifact_sha256": file_sha256(args.artifact),
        "checkpoint_provenance": provenance,
        "temporal_ranker": metrics(posterior, truth),
        "mean_logit_baseline": metrics(baseline, truth),
        "warning": "GiantSteps-key is a development benchmark, not a sealed final holdout.",
    }
    atomic_json(args.report, report)
    if args.output is not None:
        write_jsonl(
            args.output, records, manifest["canonical_labels"], posterior,
            f"temporal-ranker:{report['artifact_sha256'][:16]}",
            "OOF-MTG shared linear temporal candidate ranker; 13 aligned probability views",
            model_name="tunelock/myna-temporal-candidate-ranker",
            corpus_role="development benchmark; not an untouched final test",
            metadata_extra={"temporal_ranker_artifact_sha256": report["artifact_sha256"]},
        )
    print(
        f"temporal={report['temporal_ranker']['exact']}/{len(records)} "
        f"baseline={report['mean_logit_baseline']['exact']}/{len(records)}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
