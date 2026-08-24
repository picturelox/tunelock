#!/usr/bin/env python3
"""Reference one real file through a schema-4 Myna ONNX artifact and TTA."""

from __future__ import annotations

import argparse
import json
import math
from pathlib import Path
from typing import Any

import numpy as np
import onnxruntime
import torch
import torchaudio
from transformers import AutoModel

from extract_myna_pitch_embeddings import cached_phase_vocoder_shift


SHIFTS = [-6, -5, -4, -3, -2, -1, 1, 2, 3, 4, 5, 6]


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Probe a schema-4 Myna ONNX artifact")
    parser.add_argument("--artifact-dir", required=True, type=Path)
    parser.add_argument("--hf-cache", required=True, type=Path)
    parser.add_argument("--audio", required=True, type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--device", default="cuda" if torch.cuda.is_available() else "cpu")
    return parser.parse_args()


def load_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def softmax(values: np.ndarray) -> np.ndarray:
    shifted = values - values.max()
    exponentials = np.exp(shifted)
    return exponentials / exponentials.sum()


def align(posterior: np.ndarray, semitones: int) -> np.ndarray:
    result = np.empty(24, dtype=np.float32)
    for source in range(24):
        mode = (source // 12) * 12
        result[source] = posterior[mode + ((source % 12 + semitones) % 12)]
    return result


def main() -> int:
    args = parse_args()
    artifact = load_json(args.artifact_dir / "artifact.json")
    if artifact.get("schema_version") != 4:
        raise ValueError("Expected a schema-4 Myna artifact")
    pitch = artifact["input"]["pitch_preprocessing"]
    if pitch.get("semitone_views") != SHIFTS or pitch.get("original_weight") != 1.0:
        raise ValueError("Artifact does not use the pinned 12-view equal-weight TTA")

    backbone = artifact["backbone"]
    model = AutoModel.from_pretrained(
        backbone["model"],
        revision=backbone["revision"],
        trust_remote_code=True,
        cache_dir=args.hf_cache,
        local_files_only=True,
    ).to(args.device).eval()
    model.config.n_samples = int(artifact["input"]["audio_samples_per_chunk"])
    model.config.n_frames = model.config._get_n_frames(model.config.n_samples)
    mel_spectrogram = model.preprocessor.mel_spec.to(args.device)
    session = onnxruntime.InferenceSession(
        str(args.artifact_dir / artifact["model_file"]),
        providers=["CPUExecutionProvider"],
    )

    waveform, sample_rate = torchaudio.load(str(args.audio))
    waveform = waveform.mean(dim=0, keepdim=True).to(args.device)
    if sample_rate != 16_000:
        waveform = torchaudio.transforms.Resample(sample_rate, 16_000).to(args.device)(
            waveform
        )

    def infer(samples: torch.Tensor) -> np.ndarray:
        spectrogram = mel_spectrogram(samples)
        chunks = model.preprocessor._batch_spectrogram(
            spectrogram, model.config.n_frames
        )
        logits = session.run(
            ["chunk_logits"],
            {"mel_spectrogram": chunks.detach().cpu().numpy()},
        )[0]
        return softmax(logits.mean(axis=0)).astype(np.float32)

    with torch.inference_mode():
        base = infer(waveform)
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
            device=spectrum.device,
        )[..., None]
        aligned = [base]
        per_view = [{"semitones": 0, "posterior": base.tolist()}]
        for semitones in SHIFTS:
            rate = 2.0 ** (-float(semitones) / 12.0)
            resampler = torchaudio.transforms.Resample(
                int(16_000 / rate), 16_000
            ).to(args.device)
            shifted = cached_phase_vocoder_shift(
                spectrum,
                waveform.shape[-1],
                semitones,
                window,
                phase_advance,
                resampler,
            )
            posterior = infer(shifted)
            aligned.append(align(posterior, semitones))
            per_view.append(
                {"semitones": semitones, "posterior": posterior.tolist()}
            )
            print(f"shift={semitones:+d} complete", flush=True)
        tta = np.stack(aligned).mean(axis=0)

    result = {
        "schema_version": 1,
        "status": "research implementation-parity reference; not an accuracy score",
        "artifact": str(args.artifact_dir),
        "audio": str(args.audio),
        "device": args.device,
        "base_posterior": base.tolist(),
        "tta_posterior": tta.tolist(),
        "tta_top_index": int(tta.argmax()),
        "views": per_view,
    }
    rendered = json.dumps(result, indent=2) + "\n"
    if args.output is not None:
        if args.output.exists():
            raise FileExistsError(f"Refusing to overwrite {args.output}")
        args.output.write_text(rendered, encoding="utf-8")
        print(f"wrote={args.output}", flush=True)
    else:
        print(rendered, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
