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
    """Time-shift augmentation."""
    augmented_data = [data]
    augmented_labels = [labels]
    for _ in range(num_augments):
        shifts = np.random.randint(-50, 50, size=len(data))
        shifted = np.array([np.roll(d, s, axis=0) for d, s in zip(data, shifts)])
        augmented_data.append(shifted)
        augmented_labels.append(labels)
    return np.concatenate(augmented_data), np.concatenate(augmented_labels)


def train_fold(model, train_loader, val_loader, epochs, device, lr=0.001):
    criterion = nn.CrossEntropyLoss()
    optimizer = optim.Adam(model.parameters(), lr=lr, weight_decay=1e-3)
    scheduler = optim.lr_scheduler.CosineAnnealingLR(optimizer, T_max=epochs)

    best_val_acc = 0.0
    best_state = None

    for epoch in range(epochs):
        model.train()
        for inputs, labels in train_loader:
            inputs, labels = inputs.to(device), labels.to(device)
            optimizer.zero_grad()
            outputs = model(inputs)
            loss = criterion(outputs, labels)
            loss.backward()
            optimizer.step()

        # Validation
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

        val_acc = 100.0 * val_correct / val_total
        scheduler.step()

        if val_acc > best_val_acc:
            best_val_acc = val_acc
            best_state = {k: v.clone() for k, v in model.state_dict().items()}

    if best_state is not None:
        model.load_state_dict(best_state)
    return best_val_acc


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
