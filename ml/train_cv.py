#!/usr/bin/env python3
"""Train a small CNN with k-fold cross-validation to get an honest accuracy estimate.

The full KeyCNN (424K params) massively overfits 604 tracks. This script tests
a much smaller model (50K params) with aggressive regularization and reports
cross-validated accuracy.

Usage:
    python train_cv.py --features D:/tunelock-ml/data/features_gs.h5 --model cqt --epochs 60
"""

import argparse
import os

import h5py
import numpy as np
import torch
import torch.nn as nn
import torch.optim as optim
from torch.utils.data import DataLoader, TensorDataset
from sklearn.model_selection import StratifiedKFold


class SmallKeyCNN(nn.Module):
    """Compact CNN for key classification — designed for small datasets.
    
    ~50K parameters (vs 424K in the full model) with aggressive dropout.
    """

    def __init__(self, input_channels=1, input_height=80, input_width=252, num_classes=24):
        super().__init__()

        self.features = nn.Sequential(
            nn.Conv2d(input_channels, 16, kernel_size=3, padding=1),
            nn.BatchNorm2d(16),
            nn.ReLU(),
            nn.MaxPool2d((2, 4)),

            nn.Conv2d(16, 32, kernel_size=3, padding=1),
            nn.BatchNorm2d(32),
            nn.ReLU(),
            nn.MaxPool2d((2, 4)),

            nn.Conv2d(32, 64, kernel_size=3, padding=1),
            nn.BatchNorm2d(64),
            nn.ReLU(),
            nn.AdaptiveAvgPool2d((1, 1)),
        )

        self.classifier = nn.Sequential(
            nn.Flatten(),
            nn.Linear(64, 32),
            nn.ReLU(),
            nn.Dropout(0.6),
            nn.Linear(32, num_classes),
        )

    def forward(self, x):
        x = self.features(x)
        x = self.classifier(x)
        return x


def augment_data(data, labels, num_augments=4):
    """Time-shift + pitch-shift augmentation.

    Bug fixes (Step 7):
    1. Time shift was rolling axis=0 (the batch dimension) instead of
       the time axis. Now rolls axis=2 (height = time frames).
    2. Added pitch-shift augmentation: circularly shift the frequency
       bins by ±1-7 semitones and correspondingly shift the label.
       This is the augmentation used by Korzeniowski et al. (2017).
    """
    augmented_data = [data]
    augmented_labels = [labels]

    n_samples = len(data)

    for _ in range(num_augments):
        # Time-shift: roll along the time axis (axis=2 in [N, C, H, W])
        shifts = np.random.randint(-50, 50, size=n_samples)
        shifted = np.array([np.roll(d, s, axis=2) for d, s in zip(data, shifts)])
        augmented_data.append(shifted)
        augmented_labels.append(labels)

    # Pitch-shift: circularly shift frequency bins and labels
    # Each semitone shift maps label L to (L + shift) % 12 + (L // 12) * 12
    pitch_shifts = [-4, -3, -2, -1, 1, 2, 3, 4, 5, 6, 7]
    for ps in pitch_shifts[:4]:  # Use 4 pitch shifts to limit augmentation size
        shifted_data = np.roll(data, ps, axis=3)  # axis=3 = frequency bins
        shifted_labels = []
        for lbl in labels:
            mode = lbl // 12  # 0=major, 1=minor
            tonic = lbl % 12
            new_tonic = (tonic + ps) % 12
            shifted_labels.append(new_tonic + mode * 12)
        augmented_data.append(shifted_data)
        augmented_labels.append(np.array(shifted_labels))

    return np.concatenate(augmented_data), np.concatenate(augmented_labels)


def train_fold(model, train_loader, val_loader, epochs, device, lr=0.001):
    """Train one fold and return the validation accuracy.

    Bug fix (Step 7): Previously, the best epoch was selected based on
    the validation fold's accuracy, which is the same fold used for
    reporting. This introduces model-selection bias — the reported
    accuracy is inflated because we "peeked" at the test fold during
    training.

    Now we split the training data into train + internal validation
    (90/10) and select the best epoch on the internal validation only.
    The external validation fold is evaluated only once, after training.
    """
    criterion = nn.CrossEntropyLoss()
    optimizer = optim.Adam(model.parameters(), lr=lr, weight_decay=1e-3)
    scheduler = optim.lr_scheduler.CosineAnnealingLR(optimizer, T_max=epochs)

    # Split training data into train + internal validation
    train_size = int(0.9 * len(train_loader.dataset))
    internal_val_size = len(train_loader.dataset) - train_size
    train_subset, internal_val_subset = torch.utils.data.random_split(
        train_loader.dataset,
        [train_size, internal_val_size],
        generator=torch.Generator().manual_seed(42)
    )
    internal_train_loader = DataLoader(train_subset, batch_size=train_loader.batch_size, shuffle=True)
    internal_val_loader = DataLoader(internal_val_subset, batch_size=train_loader.batch_size)

    best_internal_acc = 0.0
    best_state = None

    for epoch in range(epochs):
        model.train()
        for inputs, labels in internal_train_loader:
            inputs, labels = inputs.to(device), labels.to(device)
            optimizer.zero_grad()
            outputs = model(inputs)
            loss = criterion(outputs, labels)
            loss.backward()
            optimizer.step()

        # Internal validation (for epoch selection only)
        model.eval()
        internal_correct = 0
        internal_total = 0
        with torch.no_grad():
            for inputs, labels in internal_val_loader:
                inputs, labels = inputs.to(device), labels.to(device)
                outputs = model(inputs)
                _, predicted = outputs.max(1)
                internal_total += labels.size(0)
                internal_correct += predicted.eq(labels).sum().item()

        internal_acc = 100.0 * internal_correct / max(internal_total, 1)
        scheduler.step()

        if internal_acc > best_internal_acc:
            best_internal_acc = internal_acc
            best_state = {k: v.clone() for k, v in model.state_dict().items()}

    # Load best epoch (selected on internal validation only)
    if best_state is not None:
        model.load_state_dict(best_state)

    # Evaluate on the external validation fold (only once)
    model.eval()
    val_correct = 0
    val_total = 0
    with torch.no_grad():
        for inputs, labels in val_loader:
            inputs, labels = inputs.to(device), labels.to(device)
            outputs = model(inputs)
            _, predicted = outputs.max(1)
            val_total += labels.size(0)
            val_correct += predicted.eq(labels).sum().item()

    val_acc = 100.0 * val_correct / max(val_total, 1)
    return val_acc


def main():
    parser = argparse.ArgumentParser(description='Train small CNN with k-fold CV')
    parser.add_argument('--features', required=True)
    parser.add_argument('--model', choices=['cqt', 'mel', 'hpcp'], required=True)
    parser.add_argument('--epochs', type=int, default=60)
    parser.add_argument('--batch_size', type=int, default=32)
    parser.add_argument('--augment', type=int, default=4)
    parser.add_argument('--output', default=None, help='Output ONNX file (best fold)')
    parser.add_argument('--k_folds', type=int, default=5)
    parser.add_argument('--device', default='cuda' if torch.cuda.is_available() else 'cpu')
    args = parser.parse_args()

    device = torch.device(args.device)
    print(f'Using device: {device}')

    with h5py.File(args.features, 'r') as f:
        data = f[args.model][:]
        labels = f['labels'][:]

    print(f'Loaded {len(data)} samples, {len(np.unique(labels))} classes')
    print(f'Model: SmallKeyCNN (~50K params)')
    print(f'K-fold: {args.k_folds}, Augment: {args.augment}x, Epochs: {args.epochs}')

    data = data[:, np.newaxis, :, :]
    input_height, input_width = data.shape[2], data.shape[3]

    skf = StratifiedKFold(n_splits=args.k_folds, shuffle=True, random_state=42)
    fold_accs = []

    for fold, (train_idx, val_idx) in enumerate(skf.split(data, labels)):
        print(f'\n--- Fold {fold+1}/{args.k_folds} ---')

        train_data = data[train_idx]
        train_labels = labels[train_idx]
        val_data = data[val_idx]
        val_labels = labels[val_idx]

        # Augment training set only
        if args.augment > 0:
            train_data, train_labels = augment_data(train_data, train_labels, args.augment)

        train_ds = TensorDataset(torch.FloatTensor(train_data), torch.LongTensor(train_labels))
        val_ds = TensorDataset(torch.FloatTensor(val_data), torch.LongTensor(val_labels))
        train_loader = DataLoader(train_ds, batch_size=args.batch_size, shuffle=True)
        val_loader = DataLoader(val_ds, batch_size=args.batch_size)

        model = SmallKeyCNN(input_height=input_height, input_width=input_width).to(device)
        num_params = sum(p.numel() for p in model.parameters())
        if fold == 0:
            print(f'Parameters: {num_params:,}')

        acc = train_fold(model, train_loader, val_loader, args.epochs, device)
        fold_accs.append(acc)
        print(f'Fold {fold+1} best val accuracy: {acc:.1f}%')

        # Export the best fold's model
        if args.output and acc == max(fold_accs):
            model_cpu = model.to('cpu')
            model_cpu.eval()
            dummy = torch.randn(1, 1, input_height, input_width)
            os.makedirs(os.path.dirname(args.output), exist_ok=True)
            torch.onnx.export(
                model_cpu, dummy, args.output,
                export_params=True, opset_version=14,
                do_constant_folding=True,
                input_names=['input'], output_names=['output'],
                dynamic_axes={'input': {0: 'batch'}, 'output': {0: 'batch'}},
            )

    print(f'\n=== Cross-validated Results ===')
    print(f'Per-fold: {["%.1f%%" % a for a in fold_accs]}')
    print(f'Mean: {np.mean(fold_accs):.1f}% ± {np.std(fold_accs):.1f}%')
    print(f'Best fold: {max(fold_accs):.1f}%')
    print(f'Worst fold: {min(fold_accs):.1f}%')


if __name__ == '__main__':
    main()
