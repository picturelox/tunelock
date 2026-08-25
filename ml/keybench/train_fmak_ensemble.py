#!/usr/bin/env python3
"""Train diverse FMAK Myna heads with OOF predictions for ensemble selection.

Trains multiple Myna MLP heads with different architectures/hyperparameters
on FMAK development folds, producing OOF predictions for each. These OOF
predictions can then be used to train a selector that approaches the oracle
ceiling.

Also includes the frozen classical and S-KEY posteriors for diversity.
"""

from __future__ import annotations

import argparse
import json
import os
from collections import Counter
from pathlib import Path
from typing import Any

for variable in ("OMP_NUM_THREADS", "OPENBLAS_NUM_THREADS", "MKL_NUM_THREADS"):
    os.environ.setdefault(variable, "1")

import numpy as np
import torch
from torch import nn

KEY_COUNT = 24
EMBEDDING_DIM = 384


def load_embedding(cache_root: Path, corpus: str, track_id: str) -> np.ndarray | None:
    path = cache_root / corpus / f"{track_id}.npy"
    if not path.exists():
        return None
    emb = np.load(path)
    if emb.ndim == 2:
        emb = emb.mean(axis=0)
    return emb.astype(np.float32)


class MynaHead(nn.Module):
    def __init__(self, input_dim: int, hidden_dims: list[int], output_dim: int, dropout: float):
        super().__init__()
        layers = []
        prev = input_dim
        for h in hidden_dims:
            layers.extend([nn.Linear(prev, h), nn.ReLU(), nn.Dropout(dropout)])
            prev = h
        layers.append(nn.Linear(prev, output_dim))
        self.net = nn.Sequential(*layers)

    def forward(self, x: torch.Tensor) -> torch.Tensor:
        return self.net(x)


def mirex_score(truth: int, predicted: int) -> float:
    if truth == predicted:
        return 1.0
    t_t = truth % 12; p_t = predicted % 12
    t_m = truth >= 12; p_m = predicted >= 12
    if t_t == p_t: return 0.6
    if t_m == p_m and (t_t - p_t + 12) % 12 in (5, 7): return 0.5
    if t_m != p_m and (t_t - p_t + 12) % 12 == (9 if t_m else 3): return 0.4
    if t_m == p_m and (t_t - p_t + 12) % 12 in (1, 11): return 0.3
    return 0.0


def error_type(truth: int, predicted: int) -> str:
    if truth == predicted: return "correct"
    t_t = truth % 12; p_t = predicted % 12
    t_m = truth >= 12; p_m = predicted >= 12
    if t_t == p_t: return "parallel"
    if t_m == p_m and (t_t - p_t + 12) % 12 in (5, 7): return "fifth"
    if t_m != p_m and (t_t - p_t + 12) % 12 == (9 if t_m else 3): return "relative"
    if t_m == p_m and (t_t - p_t + 12) % 12 in (1, 11): return "semitone"
    return "other"


# Diverse model configurations
MODEL_CONFIGS = [
    {"name": "fmak_wide", "hidden_dims": [2048], "dropout": 0.75, "lr": 3e-4, "weight_decay": 1e-5},
    {"name": "fmak_deep", "hidden_dims": [1024, 1024], "dropout": 0.5, "lr": 3e-4, "weight_decay": 1e-5},
    {"name": "fmak_wider", "hidden_dims": [2048, 1024], "dropout": 0.85, "lr": 1e-4, "weight_decay": 1e-4},
    {"name": "fmak_lean", "hidden_dims": [512], "dropout": 0.3, "lr": 5e-4, "weight_decay": 1e-5},
]


def train_and_predict_oof(
    config: dict, X_all: np.ndarray, y_all: np.ndarray, fold_ids: np.ndarray,
    unique_folds: list[str], device: str, epochs: int, patience: int, seed: int,
) -> tuple[np.ndarray, dict]:
    """Train OOF for one config. Returns (oof_posteriors, metrics)."""
    oof_posteriors = np.zeros((len(X_all), KEY_COUNT), dtype=np.float32)
    oof_preds = np.zeros(len(X_all), dtype=np.int64)

    for test_fold in unique_folds:
        train_mask = fold_ids != test_fold
        test_mask = fold_ids == test_fold

        X_train = torch.from_numpy(X_all[train_mask]).to(device)
        y_train = torch.from_numpy(y_all[train_mask]).to(device)
        X_test = torch.from_numpy(X_all[test_mask]).to(device)

        torch.manual_seed(seed)
        np.random.seed(seed)

        model = MynaHead(EMBEDDING_DIM, config["hidden_dims"], KEY_COUNT, config["dropout"]).to(device)
        optimizer = torch.optim.AdamW(model.parameters(), lr=config["lr"], weight_decay=config["weight_decay"])
        criterion = nn.CrossEntropyLoss()

        # Use a small validation split from training for early stopping
        n_train = len(X_train)
        perm = torch.randperm(n_train)
        val_size = max(1, n_train // 10)
        val_idx = perm[:val_size]
        train_idx = perm[val_size:]

        best_val_acc = 0.0
        best_state = None
        no_improve = 0
        batch_size = 64

        for epoch in range(epochs):
            model.train()
            train_perm = torch.randperm(len(train_idx))
            for i in range(0, len(train_idx), batch_size):
                batch = train_idx[train_perm[i:i + batch_size]]
                optimizer.zero_grad()
                loss = criterion(model(X_train[batch]), y_train[batch])
                loss.backward()
                optimizer.step()

            model.eval()
            with torch.no_grad():
                val_preds = model(X_train[val_idx]).argmax(dim=1)
                val_acc = (val_preds == y_train[val_idx]).float().mean().item()
            if val_acc > best_val_acc:
                best_val_acc = val_acc
                best_state = {k: v.clone() for k, v in model.state_dict().items()}
                no_improve = 0
            else:
                no_improve += 1
                if no_improve >= patience:
                    break

        model.load_state_dict(best_state)
        model.eval()
        with torch.no_grad():
            logits = model(X_test)
            probs = torch.softmax(logits, dim=1).cpu().numpy()
        oof_posteriors[test_mask] = probs
        oof_preds[test_mask] = probs.argmax(axis=1)

    exact = (oof_preds == y_all).mean()
    mirex = np.mean([mirex_score(int(t), int(p)) for t, p in zip(y_all, oof_preds)])
    errors = Counter(error_type(int(t), int(p)) for t, p in zip(y_all, oof_preds))

    # ECE
    confidences = oof_posteriors.max(axis=1)
    binary_correct = (oof_preds == y_all).astype(float)
    n_bins = 10
    ece = 0.0
    for bin_idx in range(n_bins):
        lo = bin_idx / n_bins; hi = (bin_idx + 1) / n_bins
        mask = (confidences >= lo) & (confidences < hi)
        if mask.sum() > 0:
            ece += abs(confidences[mask].mean() - binary_correct[mask].mean()) * mask.sum() / len(y_all)

    # Top-3
    top3 = sum(y_all[i] in np.argsort(-oof_posteriors[i])[:3] for i in range(len(y_all)))

    metrics = {
        "exact_pct": float(exact * 100),
        "mirex_mean": float(mirex),
        "top3_pct": float(top3 / len(y_all) * 100),
        "ece": float(ece),
        "errors": {k: int(v) for k, v in errors.items()},
    }
    return oof_posteriors, metrics


def main() -> int:
    parser = argparse.ArgumentParser(description="Train diverse FMAK ensemble with OOF")
    parser.add_argument("--fmak-manifest", required=True, type=Path)
    parser.add_argument("--embedding-cache", required=True, type=Path)
    parser.add_argument("--output-dir", required=True, type=Path)
    parser.add_argument("--device", default="cuda" if torch.cuda.is_available() else "cpu")
    parser.add_argument("--epochs", type=int, default=80)
    parser.add_argument("--patience", type=int, default=12)
    parser.add_argument("--seed", type=int, default=42)
    args = parser.parse_args()

    fmak = json.loads(args.fmak_manifest.read_text(encoding="utf-8"))

    # Load all FMAK development records with embeddings
    embeddings, labels, track_ids, folds = [], [], [], []
    for rec in fmak["records"]:
        fold = rec.get("fold", "")
        if fold == "holdout" or not fold or rec.get("is_quarantined"):
            continue
        emb = load_embedding(args.embedding_cache, rec["corpus"], rec["id"])
        if emb is None:
            continue
        embeddings.append(emb)
        labels.append(int(rec["truth_index"]))
        track_ids.append(rec["id"])
        folds.append(fold)

    X_all = np.array(embeddings, dtype=np.float32)
    y_all = np.array(labels, dtype=np.int64)
    fold_ids = np.array(folds)
    unique_folds = sorted(set(folds))

    print(f"Records: {len(X_all)}", flush=True)
    print(f"Folds: {unique_folds}", flush=True)
    print(f"Fold counts: {Counter(folds)}", flush=True)

    args.output_dir.mkdir(parents=True, exist_ok=True)

    all_metrics = {}
    all_oof = {}

    for config in MODEL_CONFIGS:
        name = config["name"]
        print(f"\n{'='*60}", flush=True)
        print(f"Training: {name}", flush=True)
        print(f"  hidden={config['hidden_dims']} dropout={config['dropout']} lr={config['lr']}", flush=True)
        print(f"{'='*60}", flush=True)

        oof_post, metrics = train_and_predict_oof(
            config, X_all, y_all, fold_ids, unique_folds,
            args.device, args.epochs, args.patience, args.seed
        )

        print(f"  Exact: {metrics['exact_pct']:.1f}%", flush=True)
        print(f"  MIREX: {metrics['mirex_mean']:.3f}", flush=True)
        print(f"  Top-3: {metrics['top3_pct']:.1f}%", flush=True)
        print(f"  ECE: {metrics['ece']:.3f}", flush=True)
        print(f"  Errors: {metrics['errors']}", flush=True)

        # Save OOF posteriors
        oof_path = args.output_dir / f"{name}_oof.jsonl"
        with oof_path.open("w", encoding="utf-8") as f:
            for i, tid in enumerate(track_ids):
                f.write(json.dumps({
                    "id": tid,
                    "truth_index": int(y_all[i]),
                    "truth_label": fmak["canonical_labels"][int(y_all[i])],
                    "posterior": oof_post[i].tolist(),
                    "pred_index": int(oof_post[i].argmax()),
                    "artist": "",
                    "genre": "",
                }) + "\n")

        all_metrics[name] = metrics
        all_oof[name] = oof_path

    # Save summary
    summary = {
        "experiment": "fmak-diverse-ensemble-oof-v1",
        "n_records": len(X_all),
        "seed": args.seed,
        "epochs": args.epochs,
        "patience": args.patience,
        "model_metrics": all_metrics,
        "oof_files": {k: str(v) for k, v in all_oof.items()},
    }
    summary_path = args.output_dir / "ensemble_summary.json"
    summary_path.write_text(json.dumps(summary, indent=2), encoding="utf-8")
    print(f"\nSummary: {summary_path}", flush=True)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
