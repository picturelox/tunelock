#!/usr/bin/env python3
"""Train a small supervised key head on pinned S-KEY harmonic maps.

Architecture selection is MTG-fold-only. Development mode requires the frozen
selection artifact, retrains for the recorded epoch counts on all deduplicated
MTG records, and only then reads development features.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import random
from typing import Any

import numpy as np
import torch
from torch import nn

from extract_skey_harmonic_features import feature_path, sha256, valid_feature
from train_myna_head import deduplicate_recordings, fixed_folds, leakage_groups, write_jsonl


KEY_COUNT = 24
SEEDS = (41, 42, 43)
EXPECTED_SKEY_LABELS = [
    "A Major", "Bb Major", "B Major", "C Major", "C# Major", "D Major",
    "D# Major", "E Major", "F Major", "F# Major", "G Major", "G# Major",
    "B minor", "C minor", "C# minor", "D minor", "D# minor", "E minor",
    "F minor", "F# minor", "G minor", "G# minor", "A minor", "Bb minor",
]
SKEY_TO_CANONICAL = (9, 10, 11, 0, 1, 2, 3, 4, 5, 6, 7, 8, 23, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22)
FEATURE_CANONICAL_ORDER = (3, 4, 5, 6, 7, 8, 9, 10, 11, 0, 1, 2)
ARCHITECTURES = (
    {"id": "point", "hidden": 0, "kernel": 1, "dropout": 0.0},
    {"id": "context-3", "hidden": 16, "kernel": 3, "dropout": 0.25},
    {"id": "context-5", "hidden": 16, "kernel": 5, "dropout": 0.25},
)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Train/apply a compact S-KEY harmonic head")
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--feature-cache", required=True, type=Path)
    parser.add_argument("--selection", required=True, type=Path)
    parser.add_argument("--report", required=True, type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--checkpoint", type=Path)
    parser.add_argument("--mode", required=True, choices=("select", "oof", "development"))
    parser.add_argument("--validation-fold", type=int, choices=range(5))
    parser.add_argument("--epochs", type=int, default=200)
    parser.add_argument("--patience", type=int, default=25)
    parser.add_argument("--batch-size", type=int, default=128)
    parser.add_argument("--learning-rate", type=float, default=3e-3)
    parser.add_argument("--weight-decay", type=float, default=1e-3)
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


def seed_everything(seed: int) -> None:
    random.seed(seed)
    np.random.seed(seed)
    torch.manual_seed(seed)
    if torch.cuda.is_available():
        torch.cuda.manual_seed_all(seed)
    torch.backends.cudnn.deterministic = True
    torch.backends.cudnn.benchmark = False


class HarmonicHead(nn.Module):
    def __init__(self, config: dict[str, Any]) -> None:
        super().__init__()
        hidden = int(config["hidden"])
        if hidden == 0:
            self.layers = nn.Conv1d(3, 2, kernel_size=1)
        else:
            kernel = int(config["kernel"])
            self.layers = nn.Sequential(
                nn.Conv1d(
                    3, hidden, kernel_size=kernel, padding=kernel // 2,
                    padding_mode="circular",
                ),
                nn.ReLU(),
                nn.Dropout(float(config["dropout"])),
                nn.Conv1d(hidden, 2, kernel_size=1),
            )

    def forward(self, features: torch.Tensor) -> torch.Tensor:
        return self.layers(features).flatten(start_dim=1)


def load_records_and_features(
    manifest: dict[str, Any], cache: Path, role: str
) -> tuple[list[dict[str, Any]], torch.Tensor, torch.Tensor]:
    raw = [record for record in manifest["records"] if record["role"] == role]
    records = deduplicate_recordings(raw)[0] if role == "training" else raw
    features = []
    baseline = []
    for record in records:
        path = feature_path(cache, record)
        if not valid_feature(path):
            raise ValueError(f"Missing or invalid S-KEY harmonic feature: {path}")
        value = np.load(path, allow_pickle=False)
        features.append(value["feature"][:, FEATURE_CANONICAL_ORDER])
        source = value["posterior"]
        canonical = np.zeros(KEY_COUNT, dtype=np.float32)
        for source_index, canonical_index in enumerate(SKEY_TO_CANONICAL):
            canonical[canonical_index] = source[source_index]
        baseline.append(canonical)
    return (
        records,
        torch.from_numpy(np.stack(features).astype(np.float32, copy=False)),
        torch.from_numpy(np.stack(baseline).astype(np.float32, copy=False)),
    )


def rotate_batch(
    features: torch.Tensor, labels: torch.Tensor, generator: torch.Generator
) -> tuple[torch.Tensor, torch.Tensor]:
    shifts = torch.randint(0, 12, (len(features),), generator=generator)
    rotated = torch.stack(
        [torch.roll(feature, shifts=int(shift), dims=1) for feature, shift in zip(features, shifts)]
    )
    modes = labels // 12
    tonics = (labels % 12 + shifts) % 12
    return rotated, modes * 12 + tonics


def accuracy_and_nll(posteriors: torch.Tensor, truth: torch.Tensor) -> dict[str, float | int]:
    exact = int((posteriors.argmax(dim=1) == truth).sum())
    nll = float(-torch.log(posteriors[torch.arange(len(truth)), truth].clamp_min(1e-12)).sum())
    return {"exact": exact, "total": len(truth), "accuracy": exact / len(truth), "nll": nll}


def train_epoch(
    model: HarmonicHead,
    optimizer: torch.optim.Optimizer,
    features: torch.Tensor,
    labels: torch.Tensor,
    batch_size: int,
    device: str,
    generator: torch.Generator,
) -> float:
    model.train()
    order = torch.randperm(len(features), generator=generator)
    losses = []
    for start in range(0, len(order), batch_size):
        indices = order[start : start + batch_size]
        inputs, targets = rotate_batch(features[indices], labels[indices], generator)
        optimizer.zero_grad(set_to_none=True)
        loss = nn.functional.cross_entropy(model(inputs.to(device)), targets.to(device))
        loss.backward()
        optimizer.step()
        losses.append(float(loss.detach().cpu()))
    return float(np.mean(losses))


def predict(model: HarmonicHead, features: torch.Tensor, batch_size: int, device: str) -> torch.Tensor:
    model.eval()
    values = []
    with torch.inference_mode():
        for start in range(0, len(features), batch_size):
            values.append(model(features[start : start + batch_size].to(device)).softmax(dim=1).cpu())
    return torch.cat(values)


def select_seed(
    config: dict[str, Any],
    seed: int,
    train_features: torch.Tensor,
    train_labels: torch.Tensor,
    valid_features: torch.Tensor,
    valid_labels: torch.Tensor,
    args: argparse.Namespace,
) -> tuple[int, dict[str, torch.Tensor], torch.Tensor, list[dict[str, float]]]:
    seed_everything(seed)
    model = HarmonicHead(config).to(args.device)
    optimizer = torch.optim.AdamW(
        model.parameters(), lr=args.learning_rate, weight_decay=args.weight_decay
    )
    generator = torch.Generator().manual_seed(seed)
    best_epoch = 0
    best_exact = -1
    best_nll = float("inf")
    best_state = {}
    best_posterior = torch.empty(0)
    history = []
    stale = 0
    for epoch in range(1, args.epochs + 1):
        loss = train_epoch(
            model, optimizer, train_features, train_labels,
            args.batch_size, args.device, generator,
        )
        posterior = predict(model, valid_features, args.batch_size, args.device)
        score = accuracy_and_nll(posterior, valid_labels)
        history.append({"epoch": epoch, "loss": loss, **score})
        candidate = (int(score["exact"]), -float(score["nll"]))
        if candidate > (best_exact, -best_nll):
            best_epoch = epoch
            best_exact = int(score["exact"])
            best_nll = float(score["nll"])
            best_state = {
                name: value.detach().cpu().clone() for name, value in model.state_dict().items()
            }
            best_posterior = posterior.clone()
            stale = 0
        else:
            stale += 1
        if stale >= args.patience:
            break
    return best_epoch, best_state, best_posterior, history


def train_full(
    config: dict[str, Any], seed: int, epochs: int,
    features: torch.Tensor, labels: torch.Tensor, args: argparse.Namespace,
) -> HarmonicHead:
    seed_everything(seed)
    model = HarmonicHead(config).to(args.device)
    optimizer = torch.optim.AdamW(
        model.parameters(), lr=args.learning_rate, weight_decay=args.weight_decay
    )
    generator = torch.Generator().manual_seed(seed)
    for _ in range(epochs):
        train_epoch(model, optimizer, features, labels, args.batch_size, args.device, generator)
    return model


def main() -> int:
    args = parse_args()
    if args.report.exists() or (args.output is not None and args.output.exists()):
        raise FileExistsError("Refusing to overwrite S-KEY harmonic-head results")
    if args.mode == "select" and args.selection.exists():
        raise FileExistsError(f"Refusing to overwrite selection: {args.selection}")
    if args.mode in ("oof", "development") and not args.selection.exists():
        raise FileNotFoundError(f"Selection does not exist: {args.selection}")
    manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
    labels = manifest.get("canonical_labels", [])
    if manifest.get("schema_version") != 1 or len(labels) != KEY_COUNT:
        raise ValueError("Expected a schema-1 Rust key manifest")
    metadata = json.loads(
        (args.feature_cache / "metadata-training.json").read_text(encoding="utf-8")
    )
    if (
        metadata.get("manifest_sha256") != sha256(args.manifest)
        or metadata.get("posterior_labels") != EXPECTED_SKEY_LABELS
    ):
        raise ValueError("S-KEY feature metadata is incompatible")
    records, features, baseline = load_records_and_features(manifest, args.feature_cache, "training")
    truth = torch.tensor([int(record["truth_index"]) for record in records], dtype=torch.long)
    groups, _ = leakage_groups(records)
    folds = fixed_folds(truth.numpy(), groups)

    if args.mode == "select":
        train_indices = np.flatnonzero(folds != 0)
        valid_indices = np.flatnonzero(folds == 0)
        experiments = []
        winner = None
        for config in ARCHITECTURES:
            seed_runs = []
            seed_posteriors = []
            for seed in SEEDS:
                epoch, state, posterior, history = select_seed(
                    config, seed, features[train_indices], truth[train_indices],
                    features[valid_indices], truth[valid_indices], args,
                )
                seed_runs.append(
                    {"seed": seed, "best_epoch": epoch, "best_state": state, "history": history}
                )
                seed_posteriors.append(posterior)
            ensemble = torch.stack(seed_posteriors).mean(dim=0)
            score = accuracy_and_nll(ensemble, truth[valid_indices])
            public_runs = [
                {"seed": run["seed"], "best_epoch": run["best_epoch"], "history": run["history"]}
                for run in seed_runs
            ]
            experiment = {"architecture": config, "metrics": score, "seed_runs": public_runs}
            experiments.append(experiment)
            candidate = (int(score["exact"]), -float(score["nll"]), -len(experiments), experiment)
            if winner is None or candidate[:3] > winner[:3]:
                winner = candidate
            print(f"architecture={config['id']} exact={score['exact']}/{len(valid_indices)}")
        assert winner is not None
        artifact = {
            "schema_version": 1,
            "experiment": "skey-supervised-harmonic-head-selection",
            "manifest_sha256": sha256(args.manifest),
            "feature_metadata_sha256": file_sha256(args.feature_cache / "metadata-training.json"),
            "script_sha256": sha256(Path(__file__)),
            "selection_fold": 0,
            "seeds": list(SEEDS),
            "hyperparameters": {
                "learning_rate": args.learning_rate,
                "weight_decay": args.weight_decay,
                "batch_size": args.batch_size,
                "max_epochs": args.epochs,
                "patience": args.patience,
                "augmentation": "uniform random circular roll over all 12 tonics per batch",
            },
            "skey_baseline": accuracy_and_nll(baseline[valid_indices], truth[valid_indices]),
            "experiments": experiments,
            "selected": winner[3],
            "warning": "Architecture/epochs selected on MTG fold 0 only; GiantSteps was not read.",
        }
        atomic_json(args.selection, artifact)
        atomic_json(args.report, artifact)
        print(
            f"selected={artifact['selected']['architecture']['id']} "
            f"exact={artifact['selected']['metrics']['exact']}/{len(valid_indices)} "
            f"skey={artifact['skey_baseline']['exact']}/{len(valid_indices)}"
        )
        return 0

    selection = json.loads(args.selection.read_text(encoding="utf-8"))
    if (
        selection.get("schema_version") != 1
        or selection.get("experiment") != "skey-supervised-harmonic-head-selection"
        or selection.get("manifest_sha256") != sha256(args.manifest)
    ):
        raise ValueError("Frozen S-KEY selection is incompatible")
    config = selection["selected"]["architecture"]
    epochs = [int(run["best_epoch"]) for run in selection["selected"]["seed_runs"]]

    if args.mode == "oof":
        if args.validation_fold is None or args.output is None:
            raise ValueError("OOF mode requires --validation-fold and --output")
        train_indices = np.flatnonzero(folds != args.validation_fold)
        valid_indices = np.flatnonzero(folds == args.validation_fold)
        models = [
            train_full(
                config, seed, epoch, features[train_indices], truth[train_indices], args
            )
            for seed, epoch in zip(SEEDS, epochs)
        ]
        posterior = torch.stack(
            [predict(model, features[valid_indices], args.batch_size, args.device) for model in models]
        ).mean(dim=0)
        valid_records = [records[int(index)] for index in valid_indices]
        report = {
            "schema_version": 1,
            "experiment": "skey-supervised-harmonic-head-oof",
            "manifest_sha256": sha256(args.manifest),
            "selection_sha256": file_sha256(args.selection),
            "validation_fold": args.validation_fold,
            "train_records": len(train_indices),
            "validation_records": len(valid_indices),
            "harmonic_head": accuracy_and_nll(posterior, truth[valid_indices]),
            "skey_baseline": accuracy_and_nll(baseline[valid_indices], truth[valid_indices]),
        }
        atomic_json(args.report, report)
        write_jsonl(
            args.output, valid_records, labels, posterior,
            f"skey-harmonic-head-oof:{report['selection_sha256'][:16]}",
            f"MTG supervised circular head; artist/recording fold {args.validation_fold} held out",
            model_name="tunelock/skey-supervised-harmonic-head",
            fold=args.validation_fold,
            corpus_role="training material; out-of-fold selector-training shard",
            metadata_extra={
                "selection_sha256": report["selection_sha256"],
                "head_contract_revision": report["selection_sha256"],
            },
        )
        print(
            f"fold={args.validation_fold} harmonic={report['harmonic_head']['exact']}/"
            f"{len(valid_indices)} skey={report['skey_baseline']['exact']}/{len(valid_indices)}"
        )
        return 0

    if args.validation_fold is not None:
        raise ValueError("--validation-fold is only valid in OOF mode")
    development_records, development_features, development_baseline = load_records_and_features(
        manifest, args.feature_cache, "development"
    )
    development_truth = torch.tensor(
        [int(record["truth_index"]) for record in development_records], dtype=torch.long
    )
    models = [
        train_full(config, seed, epoch, features, truth, args)
        for seed, epoch in zip(SEEDS, epochs)
    ]
    posterior = torch.stack(
        [predict(model, development_features, args.batch_size, args.device) for model in models]
    ).mean(dim=0)
    report = {
        "schema_version": 1,
        "experiment": "skey-supervised-harmonic-head-development",
        "manifest_sha256": sha256(args.manifest),
        "selection_sha256": file_sha256(args.selection),
        "selected_architecture": config,
        "seeds": list(SEEDS),
        "epochs": epochs,
        "model_parameters_each": sum(parameter.numel() for parameter in models[0].parameters()),
        "harmonic_head": accuracy_and_nll(posterior, development_truth),
        "skey_baseline": accuracy_and_nll(development_baseline, development_truth),
        "warning": "GiantSteps-key is a development benchmark, not a sealed final holdout.",
    }
    atomic_json(args.report, report)
    if args.checkpoint is not None:
        args.checkpoint.parent.mkdir(parents=True, exist_ok=True)
        torch.save(
            {
                "schema_version": 1,
                "selection": selection,
                "states": [model.state_dict() for model in models],
            },
            args.checkpoint,
        )
    if args.output is not None:
        write_jsonl(
            args.output, development_records, labels, posterior,
            f"skey-harmonic-head:{report['selection_sha256'][:16]}",
            "pinned S-KEY harmonic map plus MTG-supervised circular-convolution head",
            model_name="tunelock/skey-supervised-harmonic-head",
            corpus_role="development benchmark; not an untouched final test",
            metadata_extra={
                "selection_sha256": report["selection_sha256"],
                "head_contract_revision": report["selection_sha256"],
            },
        )
    print(
        f"harmonic={report['harmonic_head']['exact']}/{len(development_records)} "
        f"skey={report['skey_baseline']['exact']}/{len(development_records)}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
