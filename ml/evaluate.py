#!/usr/bin/env python3
"""Evaluate a trained ONNX model on a test set.

Usage:
    python evaluate.py --model models/cqt_model_int8.onnx --test_set data/test.h5
"""

import argparse

import h5py
import numpy as np
import onnxruntime as ort
from sklearn.metrics import classification_report, confusion_matrix


def main():
    parser = argparse.ArgumentParser(description='Evaluate ONNX key detection model')
    parser.add_argument('--model', required=True, help='ONNX model file')
    parser.add_argument('--test_set', required=True, help='HDF5 test set')
    parser.add_argument('--feature', choices=['cqt', 'mel', 'hpcp'], required=True)
    args = parser.parse_args()

    # Load test data
    with h5py.File(args.test_set, 'r') as f:
        data = f[args.feature][:]
        labels = f['labels'][:]

    # Add channel dimension
    data = data[:, np.newaxis, :, :].astype(np.float32)

    # Load ONNX model
    session = ort.InferenceSession(args.model)
    input_name = session.get_inputs()[0].name

    # Run inference
    predictions = []
    for i in range(0, len(data), 32):
        batch = data[i:i+32]
        outputs = session.run(None, {input_name: batch})
        preds = np.argmax(outputs[0], axis=1)
        predictions.extend(preds)

    predictions = np.array(predictions)

    # Compute metrics
    accuracy = (predictions == labels).mean()
    print(f'Overall accuracy: {accuracy*100:.1f}%')

    # MIREX weighted score
    mirex = compute_mirex(predictions, labels)
    print(f'MIREX weighted score: {mirex:.3f}')

    # Camelot-compatible rate
    compatible = compute_camelot_compatible(predictions, labels)
    print(f'Camelot compatible: {compatible*100:.1f}%')

    # Confusion matrix
    key_names = [
        'C maj', 'C# maj', 'D maj', 'D# maj', 'E maj', 'F maj',
        'F# maj', 'G maj', 'G# maj', 'A maj', 'A# maj', 'B maj',
        'C min', 'C# min', 'D min', 'D# min', 'E min', 'F min',
        'F# min', 'G min', 'G# min', 'A min', 'A# min', 'B min',
    ]
    print('\nClassification report:')
    print(classification_report(labels, predictions, target_names=key_names))


def compute_mirex(preds, labels):
    """Compute MIREX weighted score.
    1.0 = correct, 0.5 = fifth, 0.3 = relative, 0.2 = parallel, 0.0 = other.
    """
    total = 0.0
    for p, l in zip(preds, labels):
        if p == l:
            total += 1.0
        elif is_fifth(p, l):
            total += 0.5
        elif is_relative(p, l):
            total += 0.3
        elif is_parallel(p, l):
            total += 0.2
    return total / len(preds)


def is_fifth(pred, label):
    """Check if pred is a perfect fifth from label (same mode)."""
    pred_pc, pred_mode = pred % 12, pred // 12
    label_pc, label_mode = label % 12, label // 12
    return pred_mode == label_mode and (pred_pc - label_pc) % 12 == 7


def is_relative(pred, label):
    """Check if pred is the relative major/minor of label."""
    pred_pc, pred_mode = pred % 12, pred // 12
    label_pc, label_mode = label % 12, label // 12
    if pred_mode == label_mode:
        return False
    # Relative: major is 3 semitones above minor
    if label_mode == 0:  # label is major, pred is minor
        return (pred_pc - label_pc + 12) % 12 == 9
    else:  # label is minor, pred is major
        return (pred_pc - label_pc + 12) % 12 == 3


def is_parallel(pred, label):
    """Check if pred is the parallel major/minor (same tonic)."""
    pred_pc, pred_mode = pred % 12, pred // 12
    label_pc, label_mode = label % 12, label // 12
    return pred_pc == label_pc and pred_mode != label_mode


def compute_camelot_compatible(preds, labels):
    """Compute Camelot-compatible rate (same key, ±1, or relative)."""
    total = 0
    for p, l in zip(preds, labels):
        if p == l or is_fifth(p, l) or is_relative(p, l) or is_parallel(p, l):
            total += 1
    return total / len(preds)


if __name__ == '__main__':
    main()
