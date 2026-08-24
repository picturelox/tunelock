#!/usr/bin/env python3
"""Export a deterministic real-file decode/resample fixture for Rust parity."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import struct
import wave

import numpy as np
import torch
import torchaudio


SOURCE_RATE = 44_100
TARGET_RATE = 16_000
FRAMES = 17_640
SEED_LEFT = 0xA11CE001
SEED_RIGHT = 0xB0B12002


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Export Myna real-file audio parity fixtures")
    parser.add_argument("--output-dir", required=True, type=Path)
    return parser.parse_args()


def pcm_channel(seed: int) -> np.ndarray:
    state = seed
    values = np.empty(FRAMES, dtype=np.int16)
    for index in range(FRAMES):
        state = (1664525 * state + 1013904223) & 0xFFFFFFFF
        values[index] = np.int16((state >> 16) - (1 << 15))
    return values


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main() -> int:
    args = parse_args()
    args.output_dir.mkdir(parents=True, exist_ok=True)
    left = pcm_channel(SEED_LEFT)
    right = pcm_channel(SEED_RIGHT)
    wav_path = args.output_dir / "myna-stereo-44100-pcm16.wav"
    frames = bytearray()
    for left_sample, right_sample in zip(left, right):
        frames.extend(struct.pack("<hh", int(left_sample), int(right_sample)))
    with wave.open(str(wav_path), "wb") as output:
        output.setnchannels(2)
        output.setsampwidth(2)
        output.setframerate(SOURCE_RATE)
        output.writeframes(frames)

    waveform, sample_rate = torchaudio.load(str(wav_path))
    if waveform.shape != (2, FRAMES) or sample_rate != SOURCE_RATE:
        raise ValueError(f"unexpected decoded fixture: {waveform.shape} at {sample_rate}")
    mono = waveform.mean(dim=0, keepdim=True)
    expected = torchaudio.transforms.Resample(SOURCE_RATE, TARGET_RATE)(mono)
    expected_array = expected.squeeze(0).cpu().numpy().astype("<f4", copy=False)
    if expected_array.shape != (6_400,) or not np.isfinite(expected_array).all():
        raise ValueError(f"unexpected resampled fixture: {expected_array.shape}")
    expected_path = args.output_dir / "myna-stereo-44100-to-16000-f32le.bin"
    expected_path.write_bytes(expected_array.tobytes(order="C"))

    manifest = {
        "schema_version": 1,
        "generator": "tunelock/myna-real-file-parity-fixture",
        "source": {
            "file": wav_path.name,
            "sha256": sha256(wav_path),
            "format": "WAV PCM signed 16-bit little-endian",
            "sample_rate_hz": SOURCE_RATE,
            "channels": 2,
            "frames": FRAMES,
            "channel_generators": {
                "algorithm": "LCG state = 1664525 * state + 1013904223 mod 2^32; sample = int16(state >> 16)",
                "left_seed": SEED_LEFT,
                "right_seed": SEED_RIGHT,
            },
        },
        "reference": {
            "decode": "torchaudio.load using soundfile backend",
            "downmix": "arithmetic mean across channels in float32",
            "resample": {
                "implementation": "torchaudio.transforms.Resample",
                "target_sample_rate_hz": TARGET_RATE,
                "lowpass_filter_width": 6,
                "rolloff": 0.99,
                "method": "sinc_interp_hann",
            },
            "file": expected_path.name,
            "dtype": "float32 little-endian",
            "samples": int(expected_array.size),
            "bytes": expected_path.stat().st_size,
            "sha256": sha256(expected_path),
        },
        "versions": {
            "numpy": np.__version__,
            "torch": torch.__version__,
            "torchaudio": torchaudio.__version__,
        },
    }
    manifest_path = args.output_dir / "myna-stereo-44100-to-16000.json"
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    print(
        f"wav={wav_path} expected={expected_path} samples={expected_array.size} "
        f"sha256={sha256(expected_path)}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
