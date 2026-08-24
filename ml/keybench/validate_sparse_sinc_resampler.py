#!/usr/bin/env python3
"""Validate sparse pitch resampling against the pinned torchaudio fixture."""

from __future__ import annotations

import argparse
import json
import math
from pathlib import Path
import time

import numpy as np
import torch
import torchaudio

from extract_myna_pitch_embeddings import (
    SparseSincResampler,
    cached_phase_vocoder_shift,
)


DEFAULT_FIXTURE_DIR = (
    Path(__file__).resolve().parents[2]
    / "src-tauri"
    / "tests"
    / "fixtures"
    / "neural-key"
)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Check sparse-v1 against all twelve pinned torchaudio pitch views"
    )
    parser.add_argument("--fixture-dir", type=Path, default=DEFAULT_FIXTURE_DIR)
    parser.add_argument("--device", default="cpu")
    parser.add_argument("--max-error", type=float, default=1e-6)
    parser.add_argument("--mean-error", type=float, default=1e-7)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    torch.manual_seed(17)
    synthetic = torch.randn(2, 3, 113, device=args.device)
    synthetic_maximum = 0.0
    for original_rate, target_rate in ((37, 41), (41, 37), (5, 5)):
        expected = torchaudio.transforms.Resample(
            original_rate, target_rate
        ).to(args.device)(synthetic)
        actual = SparseSincResampler(
            original_rate, target_rate, args.device, chunk_size=29
        )(synthetic)
        synthetic_maximum = max(
            synthetic_maximum, float((actual - expected).abs().max())
        )
    print(f"synthetic_shape_parity_max_abs={synthetic_maximum:.9g}")
    if synthetic_maximum > args.max_error:
        raise ValueError(
            f"Synthetic sparse parity failed: {synthetic_maximum} > {args.max_error}"
        )

    metadata = json.loads(
        (args.fixture_dir / "myna-pitch-views.json").read_text(encoding="utf-8")
    )
    source = np.fromfile(
        args.fixture_dir / metadata["source"]["file"], dtype="<f4"
    )
    shifts = [int(value) for value in metadata["reference"]["shifts"]]
    reference = np.fromfile(
        args.fixture_dir / metadata["reference"]["file"], dtype="<f4"
    ).reshape(len(shifts), -1)
    if reference.shape[1] != source.size:
        raise ValueError("Pitch fixture shape does not match its source")

    waveform = torch.from_numpy(source.astype(np.float32, copy=True)).unsqueeze(0)
    waveform = waveform.to(args.device)
    window = torch.hann_window(512, device=args.device)
    spectrum = torch.stft(
        waveform,
        n_fft=512,
        hop_length=128,
        win_length=512,
        window=window,
        center=True,
        pad_mode="reflect",
        normalized=False,
        onesided=True,
        return_complex=True,
    )
    phase_advance = torch.linspace(
        0,
        math.pi * 128,
        spectrum.shape[-2],
        device=args.device,
    )[..., None]

    maximum = 0.0
    absolute_sum = 0.0
    sample_count = 0
    started = time.perf_counter()
    with torch.inference_mode():
        for index, semitones in enumerate(shifts):
            rate = 2.0 ** (-float(semitones) / 12.0)
            resampler = SparseSincResampler(
                int(16_000 / rate), 16_000, args.device
            )
            shifted = cached_phase_vocoder_shift(
                spectrum,
                source.size,
                semitones,
                window,
                phase_advance,
                resampler,
            )
            actual = shifted.squeeze(0).to(device="cpu").numpy()
            difference = np.abs(actual - reference[index])
            shift_maximum = float(difference.max())
            shift_mean = float(difference.mean())
            maximum = max(maximum, shift_maximum)
            absolute_sum += float(difference.sum(dtype=np.float64))
            sample_count += difference.size
            print(
                f"shift={semitones:+d} max_abs={shift_maximum:.9g} "
                f"mean_abs={shift_mean:.9g}"
            )

    mean = absolute_sum / sample_count
    elapsed = time.perf_counter() - started
    print(
        f"views={len(shifts)} samples_per_view={source.size} "
        f"max_abs={maximum:.9g} mean_abs={mean:.9g} elapsed_seconds={elapsed:.3f}"
    )
    if maximum > args.max_error or mean > args.mean_error:
        raise ValueError(
            f"Sparse parity failed: max={maximum} mean={mean}; "
            f"limits={args.max_error}/{args.mean_error}"
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
