#!/usr/bin/env python3
"""Train a CNN key detector following Korzeniowski & Widmer's architecture.

Usage:
    python train.py --features data/features.h5 --model cqt --epochs 50 --output models/cqt_model.onnx
"""

import argparse
import os
from pathlib import Path

import h5py
import numpy as np
import torch
import torch.nn as nn
import torch.optim as optim
from torch.utils.data import DataLoader, TensorDataset, random_split


class KeyCNN(nn.Module):
    """Korzeniowski-style CNN for key classification.

    Architecture:
        4 conv2d layers with batch norm + ReLU + max pool
        2 dense layers with dropout
        24-way softmax output (12 major + 12 minor)
    """

    def __init__(self, input_channels=1, input_height=80, input_width=252, num_classes=24):
        super().__init__()

        self.features = nn.Sequential(
            # Conv block 1
            nn.Conv2d(input_channels, 32, kernel_size=3, padding=1),
            nn.BatchNorm2d(32),
            nn.ReLU(),
            nn.MaxPool2d((2, 2)),

            # Conv block 2
            nn.Conv2d(32, 64, kernel_size=3, padding=1),
            nn.BatchNorm2d(64),
            nn.ReLU(),
            nn.MaxPool2d((2, 2)),

            # Conv block 3
            nn.Conv2d(64, 128, kernel_size=3, padding=1),
            nn.BatchNorm2d(128),
            nn.ReLU(),
            nn.MaxPool2d((2, 2)),

            # Conv block 4
            nn.Conv2d(128, 256, kernel_size=3, padding=1),
            nn.BatchNorm2d(256),
            nn.ReLU(),
            nn.AdaptiveAvgPool2d((1, 1)),
        )

        self.classifier = nn.Sequential(
            nn.Flatten(),
            nn.Linear(256, 128),
            nn.ReLU(),
            nn.Dropout(0.5),
            nn.Linear(128, num_classes),
        )

    def forward(self, x):
        x = self.features(x)
        x = self.classifier(x)
        return x


def train_model(model, train_loader, val_loader, epochs, device, lr=0.001):
    """Train the model and return training history."""
    criterion = nn.CrossEntropyLoss()
    optimizer = optim.Adam(model.parameters(), lr=lr, weight_decay=1e-4)
    scheduler = optim.lr_scheduler.ReduceLROnPlateau(optimizer, patience=5, factor=0.5)

    best_val_acc = 0.0
    best_state = None

    for epoch in range(epochs):
        # Training
        model.train()
        train_loss = 0.0
        train_correct = 0
        train_total = 0

        for inputs, labels in train_loader:
            inputs, labels = inputs.to(device), labels.to(device)
            optimizer.zero_grad()
            outputs = model(inputs)
            loss = criterion(outputs, labels)
            loss.backward()
            optimizer.step()

            train_loss += loss.item()
            _, predicted = outputs.max(1)
            train_total += labels.size(0)
            train_correct += predicted.eq(labels).sum().item()

        train_acc = 100.0 * train_correct / train_total

        # Validation
        model.eval()
        val_loss = 0.0
        val_correct = 0
        val_total = 0

        with torch.no_grad():
            for inputs, labels in val_loader:
                inputs, labels = inputs.to(device), labels.to(device)
                outputs = model(inputs)
                loss = criterion(outputs, labels)

                val_loss += loss.item()
                _, predicted = outputs.max(1)
                val_total += labels.size(0)
                val_correct += predicted.eq(labels).sum().item()

        val_acc = 100.0 * val_correct / val_total
        scheduler.step(val_loss)

        print(f'Epoch {epoch+1}/{epochs}: train_acc={train_acc:.1f}% val_acc={val_acc:.1f}%')

        if val_acc > best_val_acc:
            best_val_acc = val_acc
            best_state = model.state_dict().copy()

    # Restore best model
    if best_state is not None:
        model.load_state_dict(best_state)

    return best_val_acc


def export_onnx(model, filepath, input_shape=(1, 1, 80, 252)):
    """Export model to ONNX format."""
    model.eval()
    dummy = torch.randn(*input_shape)
    torch.onnx.export(
        model, dummy, filepath,
        export_params=True,
        opset_version=14,
        do_constant_folding=True,
        input_names=['input'],
        output_names=['output'],
        dynamic_axes={'input': {0: 'batch'}, 'output': {0: 'batch'}},
    )
    print(f'Exported ONNX model to {filepath}')


def main():
    parser = argparse.ArgumentParser(description='Train CNN key detector')
    parser.add_argument('--features', required=True, help='HDF5 features file')
    parser.add_argument('--model', choices=['cqt', 'mel', 'hpcp'], required=True)
    parser.add_argument('--epochs', type=int, default=50)
    parser.add_argument('--batch_size', type=int, default=32)
    parser.add_argument('--lr', type=float, default=0.001)
    parser.add_argument('--output', required=True, help='Output ONNX file')
    parser.add_argument('--device', default='cuda' if torch.cuda.is_available() else 'cpu')
    args = parser.parse_args()

    device = torch.device(args.device)
    print(f'Using device: {device}')

    # Load features
    with h5py.File(args.features, 'r') as f:
        data = f[args.model][:]
        labels = f['labels'][:]

    print(f'Loaded {len(data)} samples with {len(np.unique(labels))} classes')

    # Add channel dimension
    data = data[:, np.newaxis, :, :]  # (N, 1, H, W)

    # Split train/val (80/20)
    dataset = TensorDataset(
        torch.FloatTensor(data),
        torch.LongTensor(labels),
    )
    train_size = int(0.8 * len(dataset))
    val_size = len(dataset) - train_size
    train_ds, val_ds = random_split(dataset, [train_size, val_size])

    train_loader = DataLoader(train_ds, batch_size=args.batch_size, shuffle=True)
    val_loader = DataLoader(val_ds, batch_size=args.batch_size)

    # Create model
    input_height, input_width = data.shape[2], data.shape[3]
    model = KeyCNN(input_height=input_height, input_width=input_width).to(device)

    # Train
    best_acc = train_model(model, train_loader, val_loader, args.epochs, device, args.lr)
    print(f'Best validation accuracy: {best_acc:.1f}%')

    # Export to ONNX
    os.makedirs(os.path.dirname(args.output), exist_ok=True)
    export_onnx(model, args.output, input_shape=(1, 1, input_height, input_width))


if __name__ == '__main__':
    main()
