#!/usr/bin/env python3
"""Cache the pinned S-KEY ChromaNet harmonic map for TuneLock corpora.

The hook is the 3 x 12 tensor immediately before S-KEY's final 1 x 1 mode
classifier. It is compact, pitch-ordered, and independent from the Myna
embedding family. Generated features and weights remain under ignored ml/data.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time
from typing import Any

import numpy as np
import torch


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Cache pinned S-KEY harmonic features")
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--skey-root", required=True, type=Path)
    parser.add_argument("--cache-dir", required=True, type=Path)
    parser.add_argument("--role", required=True, choices=("training", "development"))
    parser.add_argument("--limit", type=int)
    parser.add_argument("--device", default="cuda" if torch.cuda.is_available() else "cpu")
    return parser.parse_args()


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def git_revision(root: Path) -> str | None:
    try:
        return subprocess.check_output(
            ["git", "-C", str(root), "rev-parse", "HEAD"], text=True
        ).strip()
    except (OSError, subprocess.CalledProcessError):
        return None


def atomic_json(path: Path, value: dict[str, Any]) -> None:
    temporary = path.with_name(f"{path.name}.part.{os.getpid()}")
    temporary.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")
    os.replace(temporary, path)


def feature_path(root: Path, record: dict[str, Any]) -> Path:
    return root / str(record["corpus"]) / f"{record['id']}.npz"


def valid_feature(path: Path) -> bool:
    try:
        value = np.load(path, allow_pickle=False)
        feature = value["feature"]
        posterior = value["posterior"]
        return (
            feature.shape == (3, 12)
            and posterior.shape == (24,)
            and np.isfinite(feature).all()
            and np.isfinite(posterior).all()
            and abs(float(posterior.sum()) - 1.0) < 1e-4
        )
    except (OSError, ValueError, KeyError):
        return False


def atomic_feature(path: Path, feature: np.ndarray, posterior: np.ndarray) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f"{path.stem}.part.{os.getpid()}.npz")
    np.savez_compressed(temporary, feature=feature, posterior=posterior)
    os.replace(temporary, path)


def main() -> int:
    args = parse_args()
    if args.device.startswith("cuda") and not torch.cuda.is_available():
        raise RuntimeError("CUDA requested but unavailable")
    manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
    if manifest.get("schema_version") != 1 or len(manifest.get("canonical_labels", [])) != 24:
        raise ValueError("Expected a schema-1 Rust key manifest")
    records = [record for record in manifest["records"] if record["role"] == args.role]
    if args.limit is not None:
        records = records[: args.limit]
    if not records:
        raise ValueError(f"Manifest contains no role={args.role} records")

    root = args.skey_root.resolve()
    sys.path.insert(0, str(root))
    from skey.key_detection import (  # pylint: disable=import-error,import-outside-toplevel
        DEFAULT_CHECKPOINT_PATH,
        key_map,
        load_audio,
        load_checkpoint,
        load_model_components,
    )

    device = torch.device(args.device)
    checkpoint = load_checkpoint(DEFAULT_CHECKPOINT_PATH)
    hcqt, chromanet, crop = load_model_components(checkpoint, device)
    captured: list[torch.Tensor] = []

    def capture(_module: torch.nn.Module, _inputs: tuple[torch.Tensor], output: torch.Tensor) -> None:
        captured.append(output.detach().cpu())

    hook = chromanet.global_average_pool.register_forward_hook(capture)
    manifest_hash = sha256(args.manifest)
    metadata = {
        "schema_version": 1,
        "adapter": "tunelock/skey-harmonic-feature-cache",
        "manifest_sha256": manifest_hash,
        "role": args.role,
        "model": "deezer/skey",
        "model_revision": git_revision(root),
        "checkpoint_sha256": sha256(Path(DEFAULT_CHECKPOINT_PATH)),
        "sample_rate": int(checkpoint["audio"]["sr"]),
        "feature_contract": "ChromaNet global_average_pool output; squeeze to channels=3,pitch=12",
        "feature_shape": [3, 12],
        "posterior_labels": [key_map[index] for index in range(24)],
        "records": len(records),
    }
    args.cache_dir.mkdir(parents=True, exist_ok=True)
    metadata_path = args.cache_dir / f"metadata-{args.role}.json"
    if metadata_path.exists():
        existing = json.loads(metadata_path.read_text(encoding="utf-8"))
        if existing != metadata:
            raise ValueError("S-KEY feature cache metadata differs; use a new cache directory")
    else:
        atomic_json(metadata_path, metadata)

    pending = [record for record in records if not valid_feature(feature_path(args.cache_dir, record))]
    print(
        f"S-KEY harmonic features: role={args.role} records={len(records)} "
        f"complete={len(records) - len(pending)} pending={len(pending)} device={device}",
        flush=True,
    )
    started = time.perf_counter()
    failures = []
    try:
        for index, record in enumerate(pending, start=1):
            captured.clear()
            try:
                audio = load_audio(
                    str(record["audio_path"]), int(checkpoint["audio"]["sr"])
                ).to(device)
                with torch.inference_mode():
                    spectrogram = hcqt(audio.unsqueeze(0))
                    cropped = crop(spectrogram, torch.zeros(1, device=device))
                    posterior = chromanet(cropped).mean(dim=0)
                    posterior = posterior / posterior.sum().clamp_min(1e-12)
                if len(captured) != 1 or captured[0].shape != (1, 3, 12, 1):
                    raise ValueError(f"Unexpected hook output: {[tuple(x.shape) for x in captured]}")
                feature = captured[0][0, :, :, 0].numpy().astype(np.float32, copy=False)
                atomic_feature(
                    feature_path(args.cache_dir, record),
                    feature,
                    posterior.detach().cpu().numpy().astype(np.float32, copy=False),
                )
            except Exception as exc:  # preserve a long resumable run
                failures.append({"id": record["id"], "error": f"{type(exc).__name__}: {exc}"})
            if index == 1 or index % 25 == 0 or index == len(pending):
                elapsed = time.perf_counter() - started
                rate = index / max(elapsed, 1e-9)
                eta = (len(pending) - index) / max(rate, 1e-9) / 60.0
                print(
                    f"[{index}/{len(pending)}] failures={len(failures)} "
                    f"rate={rate * 60:.1f}/min eta={eta:.1f}m",
                    flush=True,
                )
    finally:
        hook.remove()

    complete = sum(valid_feature(feature_path(args.cache_dir, record)) for record in records)
    report = {**metadata, "complete": complete, "failures": failures}
    atomic_json(args.cache_dir / f"report-{args.role}.json", report)
    print(f"complete={complete}/{len(records)} failures={len(failures)}")
    return 0 if complete == len(records) and not failures else 2


if __name__ == "__main__":
    raise SystemExit(main())
