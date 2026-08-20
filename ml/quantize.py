#!/usr/bin/env python3
"""Quantize an ONNX model to INT8 for smaller size and faster inference.

Usage:
    python quantize.py --model models/cqt_model.onnx --output models/cqt_model_int8.onnx
"""

import argparse
import os

import numpy as np
import onnx
from onnxruntime.quantization import quantize_dynamic, QuantType


def main():
    parser = argparse.ArgumentParser(description='Quantize ONNX model to INT8')
    parser.add_argument('--model', required=True, help='Input ONNX model')
    parser.add_argument('--output', required=True, help='Output quantized ONNX model')
    args = parser.parse_args()

    if not os.path.exists(args.model):
        raise FileNotFoundError(f'Model not found: {args.model}')

    # Get input size
    model = onnx.load(args.model)
    input_size = os.path.getsize(args.model)
    print(f'Original model size: {input_size / 1024 / 1024:.2f} MB')

    # Dynamic quantization (weights only, activations stay float)
    quantize_dynamic(
        args.model,
        args.output,
        weight_type=QuantType.QInt8,
    )

    output_size = os.path.getsize(args.output)
    print(f'Quantized model size: {output_size / 1024 / 1024:.2f} MB')
    print(f'Compression ratio: {input_size / output_size:.1f}x')
    print(f'Saved to {args.output}')


if __name__ == '__main__':
    main()
