#!/usr/bin/env python3
"""Export posteriors from the Apache-2.0 KeyMyna Billboard ONNX artifact.

The model accepts a 16 kHz mono waveform and returns 24 probabilities. This
adapter performs only audio I/O, resampling, inference, and resumable JSONL
writing. TuneLock's Rust proof code owns key parsing and all accuracy metrics.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import time
from pathlib import Path
from typing import Any

import numpy as np
import onnxruntime as ort
import torch
import torchaudio


SCHEMA_VERSION = 1
SAMPLE_RATE = 16_000
# Output order documented by the author's KeyMyna inference contract. These are
# transported as strings; the Rust bakeoff canonicalizes them.
POSTERIOR_LABELS = [
    "C major", "C minor", "Db major", "Db minor", "D major", "D minor",
    "Eb major", "Eb minor", "E major", "E minor", "F major", "F minor",
    "Gb major", "Gb minor", "G major", "G minor", "Ab major", "Ab minor",
    "A major", "A minor", "Bb major", "Bb minor", "B major", "B minor",
]


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Export KeyMyna ONNX posteriors")
    parser.add_argument("--dataset-dir", required=True, type=Path)
    parser.add_argument("--model", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--device", choices=("cuda", "cpu"), default="cuda")
    parser.add_argument("--limit", type=int)
    return parser.parse_args()


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def metadata(model_path: Path, providers: list[str]) -> dict[str, Any]:
    return {
        "type": "metadata",
        "schema_version": SCHEMA_VERSION,
        "model": "oriyonay/key-detection-v1/keymyna-bb.onnx",
        "model_revision": f"sha256:{sha256(model_path)}",
        "license": "apache-2.0",
        "sample_rate": SAMPLE_RATE,
        "posterior_labels": POSTERIOR_LABELS,
        "aggregation": "model-internal full-waveform chunk aggregation",
        "protocol": "full-track",
        "providers": providers,
    }


def read_existing(output: Path) -> tuple[dict[str, Any] | None, set[str]]:
    if not output.exists():
        return None, set()
    found_metadata: dict[str, Any] | None = None
    completed: set[str] = set()
    with output.open("r", encoding="utf-8") as handle:
        for line_number, line in enumerate(handle, start=1):
            if not line.strip():
                continue
            try:
                item = json.loads(line)
            except json.JSONDecodeError as exc:
                raise ValueError(f"Invalid JSON on {output}:{line_number}: {exc}") from exc
            if item.get("type") == "metadata":
                found_metadata = item
            elif item.get("type") == "prediction" and item.get("status") == "ok":
                completed.add(str(item["track_id"]))
    return found_metadata, completed


def append_json(output: Path, item: dict[str, Any]) -> None:
    with output.open("a", encoding="utf-8", newline="\n") as handle:
        json.dump(item, handle, ensure_ascii=True, separators=(",", ":"))
        handle.write("\n")
        handle.flush()


def audio_files(dataset_dir: Path) -> list[Path]:
    audio_dir = dataset_dir / "audio"
    files: list[Path] = []
    for extension in ("*.mp3", "*.flac", "*.wav"):
        files.extend(audio_dir.glob(extension))
    return sorted(files, key=lambda path: path.name)


def load_waveform(path: Path) -> np.ndarray:
    waveform, source_rate = torchaudio.load(str(path), backend="soundfile")
    waveform = waveform.mean(dim=0, keepdim=True)
    if source_rate != SAMPLE_RATE:
        waveform = torchaudio.functional.resample(waveform, source_rate, SAMPLE_RATE)
    peak = waveform.abs().max()
    if peak > 0:
        waveform = waveform / peak
    return waveform.numpy().astype(np.float32, copy=False)


def make_session(model_path: Path, device: str) -> ort.InferenceSession:
    available = ort.get_available_providers()
    if device == "cuda":
        if "CUDAExecutionProvider" not in available:
            raise RuntimeError(f"CUDAExecutionProvider unavailable; found {available}")
        requested = ["CUDAExecutionProvider", "CPUExecutionProvider"]
    else:
        requested = ["CPUExecutionProvider"]
    session = ort.InferenceSession(str(model_path), providers=requested)
    inputs = session.get_inputs()
    outputs = session.get_outputs()
    if len(inputs) != 1 or inputs[0].name != "waveform":
        raise ValueError(f"Unexpected ONNX inputs: {[(item.name, item.shape) for item in inputs]}")
    if len(outputs) != 1 or outputs[0].name != "probs":
        raise ValueError(f"Unexpected ONNX outputs: {[(item.name, item.shape) for item in outputs]}")
    return session


def normalize_posterior(raw: np.ndarray) -> np.ndarray:
    posterior = np.asarray(raw, dtype=np.float64).reshape(-1)
    if posterior.shape != (24,):
        raise ValueError(f"Expected 24 probabilities, got {posterior.shape}")
    if not np.isfinite(posterior).all():
        raise ValueError("Posterior contains non-finite values")
    posterior = np.maximum(posterior, 0.0)
    total = float(posterior.sum())
    if total <= 0:
        raise ValueError("Posterior contains no positive mass")
    return posterior / total


def main() -> int:
    args = parse_args()
    if not args.model.is_file():
        raise FileNotFoundError(args.model)
    files = audio_files(args.dataset_dir)
    if args.limit is not None:
        files = files[: args.limit]
    if not files:
        raise FileNotFoundError(f"No benchmark audio under {args.dataset_dir / 'audio'}")

    session = make_session(args.model, args.device)
    expected_metadata = metadata(args.model, session.get_providers())
    args.output.parent.mkdir(parents=True, exist_ok=True)
    existing_metadata, completed = read_existing(args.output)
    if existing_metadata is None:
        append_json(args.output, expected_metadata)
    elif existing_metadata != expected_metadata:
        raise ValueError("Output metadata differs; use a new path for a new model/runtime contract")

    remaining = [path for path in files if path.stem not in completed]
    print(
        f"KeyMyna ONNX export: {len(files)} selected, {len(completed)} complete, "
        f"{len(remaining)} remaining, providers={session.get_providers()}",
        flush=True,
    )
    started = time.perf_counter()
    ok = 0
    failed = 0
    for index, path in enumerate(remaining, start=1):
        track_started = time.perf_counter()
        try:
            waveform = load_waveform(path)
            raw = session.run(["probs"], {"waveform": waveform})[0]
            posterior = normalize_posterior(raw)
            predicted_index = int(posterior.argmax())
            append_json(
                args.output,
                {
                    "type": "prediction",
                    "track_id": path.stem,
                    "file_name": path.name,
                    "status": "ok",
                    "posterior": [round(float(value), 10) for value in posterior.tolist()],
                    "predicted_index": predicted_index,
                    "predicted_label": POSTERIOR_LABELS[predicted_index],
                    "elapsed_ms": round((time.perf_counter() - track_started) * 1000),
                },
            )
            ok += 1
        except Exception as exc:  # preserve progress during a corpus run
            failed += 1
            append_json(
                args.output,
                {
                    "type": "prediction",
                    "track_id": path.stem,
                    "file_name": path.name,
                    "status": "error",
                    "error": f"{type(exc).__name__}: {exc}",
                },
            )

        if index == 1 or index % 10 == 0 or index == len(remaining):
            elapsed = time.perf_counter() - started
            rate = index / max(elapsed, 1e-9)
            eta_minutes = (len(remaining) - index) / max(rate, 1e-9) / 60.0
            print(
                f"[{index}/{len(remaining)}] ok={ok} failed={failed} "
                f"rate={rate * 60:.1f}/min eta={eta_minutes:.1f}m",
                flush=True,
            )

    return 0 if failed == 0 else 2


if __name__ == "__main__":
    raise SystemExit(main())
