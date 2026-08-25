#!/usr/bin/env python3
"""Controlled acoustic retraining experiment: MTG vs FMAK vs MTG+FMAK.

Trains the same Myna MLP head architecture on three different training sets
and evaluates each on the FMAK holdout set (548 records). This answers
whether key-model performance is data-limited or architecture-limited.

Training sets:
  1. MTG-only: 1,349 MTG training records (existing baseline)
  2. FMAK-only: 4,939 FMAK development records (OOF with locked folds)
  3. MTG+FMAK: 1,349 MTG + 4,939 FMAK = 6,288 records

Evaluation: FMAK holdout (548 records, never seen in any training set)

The architecture is identical across all three: Myna 384-dim embedding ->
2048 hidden -> 24 key output, with dropout=0.75, the same hyperparameters
used for the original myna-v6-accuracy-final.pt.
"""

from __future__ import annotations

import argparse
import hashlib
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


def load_manifest(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def load_embedding(cache_root: Path, corpus: str, track_id: str) -> np.ndarray | None:
    path = cache_root / corpus / f"{track_id}.npy"
    if not path.exists():
        return None
    emb = np.load(path)
    # Pool chunk-level embeddings (N, 384) -> (384,) by mean pooling
    if emb.ndim == 2:
        emb = emb.mean(axis=0)
    return emb.astype(np.float32)


class MynaHead(nn.Module):
    def __init__(self, input_dim: int = EMBEDDING_DIM, hidden_dims: list[int] = [2048],
                 output_dim: int = KEY_COUNT, dropout: float = 0.75):
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
    truth_tonic = truth % 12
    pred_tonic = predicted % 12
    truth_minor = truth >= 12
    pred_minor = predicted >= 12
    if truth_tonic == pred_tonic:
        return 0.6
    if truth_minor == pred_minor and (truth_tonic - pred_tonic + 12) % 12 in (5, 7):
        return 0.5
    if truth_minor != pred_minor and (truth_tonic - pred_tonic + 12) % 12 == (9 if truth_minor else 3):
        return 0.4
    if truth_minor == pred_minor and (truth_tonic - pred_tonic + 12) % 12 in (1, 11):
        return 0.3
    return 0.0


def error_type(truth: int, predicted: int) -> str:
    if truth == predicted:
        return "correct"
    truth_tonic = truth % 12
    pred_tonic = predicted % 12
    truth_minor = truth >= 12
    pred_minor = predicted >= 12
    if truth_tonic == pred_tonic:
        return "parallel"
    if truth_minor == pred_minor and (truth_tonic - pred_tonic + 12) % 12 in (5, 7):
        return "fifth"
    if truth_minor != pred_minor and (truth_tonic - pred_tonic + 12) % 12 == (9 if truth_minor else 3):
        return "relative"
    if truth_minor == pred_minor and (truth_tonic - pred_tonic + 12) % 12 in (1, 11):
        return "semitone"
    return "other"


def prepare_dataset(
    manifest: dict[str, Any],
    cache_root: Path,
    role_filter: str | None = None,
    fold_filter: str | None = None,
    exclude_fold: str | None = None,
) -> tuple[np.ndarray, np.ndarray, list[str]]:
    """Load embeddings and labels for matching records."""
    embeddings = []
    labels = []
    track_ids = []
    for rec in manifest["records"]:
        if role_filter and rec.get("role") != role_filter:
            continue
        fold = rec.get("fold", "")
        if fold_filter and fold != fold_filter:
            continue
        if exclude_fold and fold == exclude_fold:
            continue
        if rec.get("is_quarantined"):
            continue
        emb = load_embedding(cache_root, rec["corpus"], rec["id"])
        if emb is None:
            continue
        embeddings.append(emb)
        labels.append(int(rec["truth_index"]))
        track_ids.append(rec["id"])
    return np.array(embeddings, dtype=np.float32), np.array(labels, dtype=np.int64), track_ids


def train_head(
    X_train: np.ndarray,
    y_train: np.ndarray,
    X_val: np.ndarray | None,
    y_val: np.ndarray | None,
    device: str = "cuda",
    epochs: int = 100,
    patience: int = 15,
    batch_size: int = 64,
    lr: float = 3e-4,
    weight_decay: float = 1e-5,
    seed: int = 42,
) -> tuple[MynaHead, dict[str, list[float]]]:
    """Train a Myna head with early stopping."""
    torch.manual_seed(seed)
    np.random.seed(seed)

    model = MynaHead().to(device)
    optimizer = torch.optim.AdamW(model.parameters(), lr=lr, weight_decay=weight_decay)
    criterion = nn.CrossEntropyLoss()

    X_t = torch.from_numpy(X_train).to(device)
    y_t = torch.from_numpy(y_train).to(device)

    if X_val is not None and y_val is not None:
        X_v = torch.from_numpy(X_val).to(device)
        y_v = torch.from_numpy(y_val).to(device)
    else:
        X_v = y_v = None

    history = {"train_loss": [], "val_acc": []}
    best_val_acc = 0.0
    best_state = None
    no_improve = 0

    n = len(X_t)
    for epoch in range(epochs):
        model.train()
        perm = torch.randperm(n)
        epoch_loss = 0.0
        for i in range(0, n, batch_size):
            batch_idx = perm[i:i + batch_size]
            optimizer.zero_grad()
            logits = model(X_t[batch_idx])
            loss = criterion(logits, y_t[batch_idx])
            loss.backward()
            optimizer.step()
            epoch_loss += loss.item() * len(batch_idx)
        epoch_loss /= n
        history["train_loss"].append(epoch_loss)

        if X_v is not None:
            model.eval()
            with torch.no_grad():
                preds = model(X_v).argmax(dim=1)
                acc = (preds == y_v).float().mean().item()
            history["val_acc"].append(acc)
            if acc > best_val_acc:
                best_val_acc = acc
                best_state = {k: v.clone() for k, v in model.state_dict().items()}
                no_improve = 0
            else:
                no_improve += 1
                if no_improve >= patience:
                    print(f"    Early stop at epoch {epoch+1} (best val: {best_val_acc:.3f})", flush=True)
                    break
        else:
            # No validation: keep last state
            best_state = {k: v.clone() for k, v in model.state_dict().items()}

    if best_state is not None:
        model.load_state_dict(best_state)

    return model, history


def evaluate(model: MynaHead, X: np.ndarray, y: np.ndarray, device: str = "cuda") -> dict[str, Any]:
    model.eval()
    with torch.no_grad():
        logits = model(torch.from_numpy(X).to(device))
        probs = torch.softmax(logits, dim=1)
        preds = probs.argmax(dim=1).cpu().numpy()

    exact = (preds == y).mean()
    mirex = np.mean([mirex_score(int(t), int(p)) for t, p in zip(y, preds)])
    errors = Counter(error_type(int(t), int(p)) for t, p in zip(y, preds))

    # Top-3
    top3 = 0
    for i in range(len(y)):
        top3_indices = np.argsort(-probs[i].cpu().numpy())[:3]
        if y[i] in top3_indices:
            top3 += 1
    top3_acc = top3 / len(y)

    # ECE
    confidences = probs.max(dim=1).values.cpu().numpy()
    binary_correct = (preds == y).astype(float)
    n_bins = 10
    ece = 0.0
    for bin_idx in range(n_bins):
        lo = bin_idx / n_bins
        hi = (bin_idx + 1) / n_bins
        mask = (confidences >= lo) & (confidences < hi)
        if mask.sum() > 0:
            avg_conf = confidences[mask].mean()
            avg_acc = binary_correct[mask].mean()
            ece += abs(avg_conf - avg_acc) * mask.sum() / len(y)

    return {
        "n": int(len(y)),
        "exact_pct": float(exact * 100),
        "mirex_mean": float(mirex),
        "top3_pct": float(top3_acc * 100),
        "ece": float(ece),
        "errors": {k: int(v) for k, v in errors.items()},
    }


def main() -> int:
    parser = argparse.ArgumentParser(description="Controlled acoustic retraining experiment")
    parser.add_argument("--mtg-manifest", required=True, type=Path)
    parser.add_argument("--fmak-manifest", required=True, type=Path,
                       help="FMAK locked corpus manifest with fold assignments")
    parser.add_argument("--mtg-embedding-cache", required=True, type=Path)
    parser.add_argument("--fmak-embedding-cache", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--device", default="cuda" if torch.cuda.is_available() else "cpu")
    parser.add_argument("--epochs", type=int, default=100)
    parser.add_argument("--patience", type=int, default=15)
    parser.add_argument("--seeds", type=int, nargs="+", default=[42, 43, 44])
    args = parser.parse_args()

    # Load manifests
    mtg_manifest = load_manifest(args.mtg_manifest)
    fmak_locked = load_manifest(args.fmak_manifest)

    # FMAK holdout embeddings are not cached, so we evaluate on FMAK development OOF.
    # For MTG-only, we evaluate on all FMAK development records (cross-corpus).
    # For FMAK-only and MTG+FMAK, we use proper OOF with locked folds.

    # Get FMAK development folds
    fmak_folds = {}
    for rec in fmak_locked["records"]:
        fold = rec.get("fold", "")
        if fold and fold != "holdout" and not rec.get("is_quarantined"):
            fmak_folds[rec["id"]] = fold
    unique_folds = sorted(set(fmak_folds.values()))
    print(f"FMAK folds: {unique_folds}", flush=True)

    # Prepare full FMAK development set (for MTG-only cross-corpus evaluation)
    print("Preparing FMAK development set for MTG cross-corpus eval...", flush=True)
    X_fmak_all, y_fmak_all, fmak_all_ids = [], [], []
    for rec in fmak_locked["records"]:
        fold = rec.get("fold", "")
        if fold == "holdout" or not fold or rec.get("is_quarantined"):
            continue
        emb = load_embedding(args.fmak_embedding_cache, rec["corpus"], rec["id"])
        if emb is None:
            continue
        X_fmak_all.append(emb)
        y_fmak_all.append(int(rec["truth_index"]))
        fmak_all_ids.append(rec["id"])
    X_fmak_all = np.array(X_fmak_all, dtype=np.float32)
    y_fmak_all = np.array(y_fmak_all, dtype=np.int64)
    print(f"  FMAK development: {len(X_fmak_all)} records", flush=True)

    results = {}

    # === Experiment 1: MTG-only ===
    print(f"\n{'='*60}", flush=True)
    print("EXPERIMENT 1: MTG-only training", flush=True)
    print(f"{'='*60}", flush=True)

    X_mtg_train, y_mtg_train, _ = prepare_dataset(
        mtg_manifest, args.mtg_embedding_cache, role_filter="training"
    )
    X_mtg_dev, y_mtg_dev, _ = prepare_dataset(
        mtg_manifest, args.mtg_embedding_cache, role_filter="development"
    )
    print(f"  MTG training: {len(X_mtg_train)} records", flush=True)
    print(f"  MTG development (val): {len(X_mtg_dev)} records", flush=True)

    mtg_results = []
    for seed in args.seeds:
        print(f"\n  Seed {seed}:", flush=True)
        model, history = train_head(
            X_mtg_train, y_mtg_train, X_mtg_dev, y_mtg_dev,
            device=args.device, epochs=args.epochs, patience=args.patience, seed=seed
        )
        # Evaluate on FMAK development (cross-corpus generalization)
        metrics = evaluate(model, X_fmak_all, y_fmak_all, device=args.device)
        print(f"    FMAK dev: exact={metrics['exact_pct']:.1f}% mirex={metrics['mirex_mean']:.3f} ece={metrics['ece']:.3f}", flush=True)
        mtg_results.append(metrics)

    results["mtg_only"] = {
        "n_train": len(X_mtg_train),
        "n_val": len(X_mtg_dev),
        "n_fmak_eval": len(X_fmak_all),
        "seeds": mtg_results,
        "mean_exact": float(np.mean([r["exact_pct"] for r in mtg_results])),
        "mean_mirex": float(np.mean([r["mirex_mean"] for r in mtg_results])),
        "mean_ece": float(np.mean([r["ece"] for r in mtg_results])),
    }
    print(f"\n  MTG-only mean: exact={results['mtg_only']['mean_exact']:.1f}% mirex={results['mtg_only']['mean_mirex']:.3f} ece={results['mtg_only']['mean_ece']:.3f}", flush=True)

    # === Experiment 2: FMAK-only (OOF with locked folds) ===
    print(f"\n{'='*60}", flush=True)
    print("EXPERIMENT 2: FMAK-only training (OOF with locked folds)", flush=True)
    print(f"{'='*60}", flush=True)

    # For FMAK, use 4 folds for training, 1 for validation (OOF)
    # Average across all 5 fold combinations
    fmak_fold_results = []
    for test_fold in unique_folds:
        print(f"\n  Fold {test_fold} as validation:", flush=True)
        X_train_list, y_train_list = [], []
        X_val_list, y_val_list = [], []

        for rec in fmak_locked["records"]:
            fold = rec.get("fold", "")
            if fold == "holdout" or not fold or rec.get("is_quarantined"):
                continue
            emb = load_embedding(args.fmak_embedding_cache, rec["corpus"], rec["id"])
            if emb is None:
                continue
            if fold == test_fold:
                X_val_list.append(emb)
                y_val_list.append(int(rec["truth_index"]))
            else:
                X_train_list.append(emb)
                y_train_list.append(int(rec["truth_index"]))

        X_train = np.array(X_train_list, dtype=np.float32)
        y_train = np.array(y_train_list, dtype=np.int64)
        X_val = np.array(X_val_list, dtype=np.float32)
        y_val = np.array(y_val_list, dtype=np.int64)

        print(f"    train={len(X_train)} val={len(X_val)}", flush=True)

        seed_results = []
        for seed in args.seeds[:2]:  # Use fewer seeds for OOF to save time
            print(f"    Seed {seed}:", flush=True)
            model, history = train_head(
                X_train, y_train, X_val, y_val,
                device=args.device, epochs=args.epochs, patience=args.patience, seed=seed
            )
            # Evaluate on the OOF validation fold
            metrics = evaluate(model, X_val, y_val, device=args.device)
            print(f"      OOF: exact={metrics['exact_pct']:.1f}% mirex={metrics['mirex_mean']:.3f} ece={metrics['ece']:.3f}", flush=True)
            seed_results.append(metrics)
        fmak_fold_results.append({
            "val_fold": test_fold,
            "n_train": len(X_train),
            "n_val": len(X_val),
            "seeds": seed_results,
        })

    # Average FMAK OOF results
    all_fmak_oof = [r for fr in fmak_fold_results for r in fr["seeds"]]
    results["fmak_only"] = {
        "n_train_avg": int(np.mean([fr["n_train"] for fr in fmak_fold_results])),
        "n_val_avg": int(np.mean([fr["n_val"] for fr in fmak_fold_results])),
        "n_fmak_eval": len(X_fmak_all),
        "fold_results": fmak_fold_results,
        "mean_exact": float(np.mean([r["exact_pct"] for r in all_fmak_oof])),
        "mean_mirex": float(np.mean([r["mirex_mean"] for r in all_fmak_oof])),
        "mean_ece": float(np.mean([r["ece"] for r in all_fmak_oof])),
    }
    print(f"\n  FMAK-only mean: exact={results['fmak_only']['mean_exact']:.1f}% mirex={results['fmak_only']['mean_mirex']:.3f} ece={results['fmak_only']['mean_ece']:.3f}", flush=True)

    # === Experiment 3: MTG + FMAK ===
    print(f"\n{'='*60}", flush=True)
    print("EXPERIMENT 3: MTG + FMAK combined training", flush=True)
    print(f"{'='*60}", flush=True)

    # Combine MTG training + FMAK development (all folds except one for validation)
    combined_fold_results = []
    for test_fold in unique_folds:
        print(f"\n  Fold {test_fold} as FMAK validation:", flush=True)
        X_train_list, y_train_list = [], []
        X_val_list, y_val_list = [], []

        # Add all MTG training records
        for rec in mtg_manifest["records"]:
            if rec.get("role") != "training":
                continue
            emb = load_embedding(args.mtg_embedding_cache, rec["corpus"], rec["id"])
            if emb is None:
                continue
            X_train_list.append(emb)
            y_train_list.append(int(rec["truth_index"]))

        # Add FMAK development records (except validation fold)
        for rec in fmak_locked["records"]:
            fold = rec.get("fold", "")
            if fold == "holdout" or not fold or rec.get("is_quarantined"):
                continue
            emb = load_embedding(args.fmak_embedding_cache, rec["corpus"], rec["id"])
            if emb is None:
                continue
            if fold == test_fold:
                X_val_list.append(emb)
                y_val_list.append(int(rec["truth_index"]))
            else:
                X_train_list.append(emb)
                y_train_list.append(int(rec["truth_index"]))

        X_train = np.array(X_train_list, dtype=np.float32)
        y_train = np.array(y_train_list, dtype=np.int64)
        X_val = np.array(X_val_list, dtype=np.float32)
        y_val = np.array(y_val_list, dtype=np.int64)

        print(f"    train={len(X_train)} val={len(X_val)}", flush=True)

        seed_results = []
        for seed in args.seeds[:2]:
            print(f"    Seed {seed}:", flush=True)
            model, history = train_head(
                X_train, y_train, X_val, y_val,
                device=args.device, epochs=args.epochs, patience=args.patience, seed=seed
            )
            metrics = evaluate(model, X_val, y_val, device=args.device)
            print(f"      OOF: exact={metrics['exact_pct']:.1f}% mirex={metrics['mirex_mean']:.3f} ece={metrics['ece']:.3f}", flush=True)
            seed_results.append(metrics)
        combined_fold_results.append({
            "val_fold": test_fold,
            "n_train": len(X_train),
            "n_val": len(X_val),
            "seeds": seed_results,
        })

    all_combined_oof = [r for fr in combined_fold_results for r in fr["seeds"]]
    results["mtg_fmak"] = {
        "n_train_avg": int(np.mean([fr["n_train"] for fr in combined_fold_results])),
        "n_val_avg": int(np.mean([fr["n_val"] for fr in combined_fold_results])),
        "n_fmak_eval": len(X_fmak_all),
        "fold_results": combined_fold_results,
        "mean_exact": float(np.mean([r["exact_pct"] for r in all_combined_oof])),
        "mean_mirex": float(np.mean([r["mirex_mean"] for r in all_combined_oof])),
        "mean_ece": float(np.mean([r["ece"] for r in all_combined_oof])),
    }
    print(f"\n  MTG+FMAK mean: exact={results['mtg_fmak']['mean_exact']:.1f}% mirex={results['mtg_fmak']['mean_mirex']:.3f} ece={results['mtg_fmak']['mean_ece']:.3f}", flush=True)

    # === Summary ===
    print(f"\n{'='*60}", flush=True)
    print("RETRAINING EXPERIMENT SUMMARY", flush=True)
    print(f"{'='*60}", flush=True)
    print(f"  FMAK eval set: {len(X_fmak_all)} development records", flush=True)
    print(f"  {'Experiment':<20} {'n_train':<10} {'Exact':<10} {'MIREX':<10} {'ECE':<10}", flush=True)
    for name, key in [("MTG-only", "mtg_only"), ("FMAK-only", "fmak_only"), ("MTG+FMAK", "mtg_fmak")]:
        r = results[key]
        n = r.get("n_train", r.get("n_train_avg", 0))
        print(f"  {name:<20} {n:<10} {r['mean_exact']:<10.1f} {r['mean_mirex']:<10.3f} {r['mean_ece']:<10.3f}", flush=True)

    # Write results
    args.output.parent.mkdir(parents=True, exist_ok=True)
    output = {
        "experiment": "controlled-acoustic-retraining-v1",
        "fmak_eval_size": len(X_fmak_all),
        "seeds": args.seeds,
        "epochs": args.epochs,
        "patience": args.patience,
        "results": results,
        "mtg_manifest_sha256": hashlib.sha256(args.mtg_manifest.read_bytes()).hexdigest(),
        "fmak_manifest_sha256": hashlib.sha256(args.fmak_manifest.read_bytes()).hexdigest(),
    }
    args.output.write_text(json.dumps(output, indent=2), encoding="utf-8")
    print(f"\nResults: {args.output}", flush=True)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
