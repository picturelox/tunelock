#!/usr/bin/env python3
"""Export pinned torchaudio phase-vocoder outputs for native Rust parity."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
from pathlib import Path

import numpy as np
import torch
import torchaudio
import torchaudio.functional as audio_functional


SHIFTS = [-6, -5, -4, -3, -2, -1, 1, 2, 3, 4, 5, 6]
SAMPLE_RATE = 16_000
N_FFT = 512
HOP_LENGTH = 128


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Export Myna pitch-view parity fixture")
    parser.add_argument("--source-f32", required=True, type=Path)
    parser.add_argument("--output-dir", required=True, type=Path)
    return parser.parse_args()


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def shift_view(
    spectrum: torch.Tensor,
    original_length: int,
    semitones: int,
    window: torch.Tensor,
    phase_advance: torch.Tensor,
) -> torch.Tensor:
    rate = 2.0 ** (-float(semitones) / 12.0)
    stretched_spectrum = audio_functional.phase_vocoder(
        spectrum, rate, phase_advance
    )
    stretched_length = int(round(original_length / rate))
    stretched = torch.istft(
        stretched_spectrum,
        n_fft=N_FFT,
        hop_length=HOP_LENGTH,
        win_length=N_FFT,
        window=window,
        length=stretched_length,
    )
    shifted = torchaudio.transforms.Resample(
        int(SAMPLE_RATE / rate), SAMPLE_RATE
    )(stretched)
    if shifted.shape[-1] < original_length:
        shifted = torch.nn.functional.pad(
            shifted, (0, original_length - shifted.shape[-1])
        )
    return shifted[..., :original_length]


def main() -> int:
    args = parse_args()
    if torchaudio.__version__.split("+", maxsplit=1)[0] != "2.7.1":
        raise ValueError(f"Expected torchaudio 2.7.1, found {torchaudio.__version__}")
    source = np.fromfile(args.source_f32, dtype="<f4")
    if source.size < N_FFT or not np.isfinite(source).all():
        raise ValueError("Source fixture must contain enough finite float32 samples")
    waveform = torch.from_numpy(source.astype(np.float32, copy=True)).unsqueeze(0)
    window = torch.hann_window(N_FFT)
    spectrum = torch.stft(
        waveform,
        n_fft=N_FFT,
        hop_length=HOP_LENGTH,
        win_length=N_FFT,
        window=window,
        center=True,
        pad_mode="reflect",
        normalized=False,
        onesided=True,
        return_complex=True,
    )
    phase_advance = torch.linspace(
        0, math.pi * HOP_LENGTH, spectrum.shape[-2]
    )[..., None]
    output = []
    with torch.inference_mode():
        for semitones in SHIFTS:
            shifted = shift_view(
                spectrum,
                waveform.shape[-1],
                semitones,
                window,
                phase_advance,
            )
            values = shifted.squeeze(0).numpy().astype("<f4", copy=False)
            if values.shape != source.shape or not np.isfinite(values).all():
                raise ValueError(f"Unexpected shift {semitones:+d} output: {values.shape}")
            output.append(values)

    args.output_dir.mkdir(parents=True, exist_ok=True)
    output_path = args.output_dir / "myna-pitch-views-f32le.bin"
    np.stack(output).astype("<f4", copy=False).tofile(output_path)
    manifest = {
        "schema_version": 1,
        "generator": "tunelock/myna-pitch-view-parity-fixture",
        "source": {
            "file": args.source_f32.name,
            "sha256": sha256(args.source_f32),
            "dtype": "float32 little-endian",
            "sample_rate_hz": SAMPLE_RATE,
            "samples": int(source.size),
        },
        "reference": {
            "file": output_path.name,
            "sha256": sha256(output_path),
            "dtype": "float32 little-endian",
            "layout": "shift-major contiguous samples",
            "shifts": SHIFTS,
            "samples_per_shift": int(source.size),
            "stft": {
                "n_fft": N_FFT,
                "hop_length": HOP_LENGTH,
                "window": "periodic Hann",
                "center": True,
                "pad_mode": "reflect",
            },
            "phase_vocoder": "torchaudio.functional.phase_vocoder",
            "stretched_length": "round(original_length / rate)",
            "resampler": {
                "original_rate": "int(16000 / rate)",
                "target_rate": SAMPLE_RATE,
                "lowpass_filter_width": 6,
                "rolloff": 0.99,
                "method": "sinc_interp_hann",
            },
            "output_length": "right zero-pad then truncate to source length",
        },
        "versions": {
            "numpy": np.__version__,
            "torch": torch.__version__,
            "torchaudio": torchaudio.__version__,
        },
    }
    manifest_path = args.output_dir / "myna-pitch-views.json"
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    print(
        f"output={output_path} shifts={len(SHIFTS)} samples={source.size} "
        f"sha256={sha256(output_path)}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
