#!/usr/bin/env python3
"""Export a deterministic nnAudio mel fixture for the Rust parity test.

The waveform is generated from integer arithmetic in both languages, so the
fixture measures preprocessing rather than file decoding or resampling.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path

import numpy as np
import nnAudio
import torch
from nnAudio.features import MelSpectrogram


SEED = 0x5EED1234
SAMPLES = 100_000


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Export the Myna mel parity fixture")
    parser.add_argument("--output-dir", required=True, type=Path)
    return parser.parse_args()


def waveform() -> np.ndarray:
    state = SEED
    values = np.empty(SAMPLES, dtype=np.float32)
    for index in range(SAMPLES):
        state = (1664525 * state + 1013904223) & 0xFFFFFFFF
        signed = int(state >> 8) - (1 << 23)
        values[index] = np.float32(signed) / np.float32(1 << 23)
    return values


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main() -> int:
    args = parse_args()
    if nnAudio.__version__ != "0.3.3":
        raise ValueError(f"Expected pinned nnAudio 0.3.3, found {nnAudio.__version__}")
    args.output_dir.mkdir(parents=True, exist_ok=True)
    signal = torch.from_numpy(waveform()).unsqueeze(0)
    preprocessor = MelSpectrogram(
        sr=16_000,
        n_fft=2_048,
        n_mels=128,
        hop_length=512,
        window="hann",
        center=True,
        pad_mode="reflect",
        power=2.0,
        htk=False,
        fmin=0.0,
        fmax=None,
        norm=1,
        verbose=False,
    ).eval()
    with torch.inference_mode():
        mel = preprocessor(signal).cpu().numpy().astype("<f4", copy=False)
    if mel.shape != (1, 128, 196) or not np.isfinite(mel).all():
        raise ValueError(f"unexpected mel fixture shape/value: {mel.shape}")

    binary = args.output_dir / "myna-mel-100000-f32le.bin"
    binary.write_bytes(mel.tobytes(order="C"))
    manifest = {
        "schema_version": 1,
        "generator": "tunelock/myna-mel-parity-fixture",
        "waveform": {
            "generator": "LCG state = 1664525 * state + 1013904223 mod 2^32; "
            "sample = ((state >> 8) - 2^23) / 2^23",
            "seed": SEED,
            "samples": SAMPLES,
            "sample_rate_hz": 16_000,
        },
        "preprocessor": {
            "implementation": "nnAudio MelSpectrogram 0.3.3",
            "n_fft": 2_048,
            "hop_length": 512,
            "n_mels": 128,
            "window": "periodic Hann",
            "center": True,
            "pad_mode": "reflect",
            "power": 2.0,
            "mel_scale": "Slaney",
            "normalization": "area",
        },
        "output": {
            "shape": list(mel.shape),
            "dtype": "float32 little-endian",
            "file": binary.name,
            "bytes": binary.stat().st_size,
            "sha256": sha256(binary),
        },
        "versions": {
            "nnAudio": nnAudio.__version__,
            "numpy": np.__version__,
            "torch": torch.__version__,
        },
    }
    (args.output_dir / "myna-mel-100000.json").write_text(
        json.dumps(manifest, indent=2) + "\n", encoding="utf-8"
    )
    print(f"fixture={binary} bytes={binary.stat().st_size} sha256={sha256(binary)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
