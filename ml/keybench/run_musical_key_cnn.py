#!/usr/bin/env python3
"""Export posteriors from the pinned MIT MusicalKeyCNN control model."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import sys
import time
from typing import Any

import torch


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Export MusicalKeyCNN 24-key posteriors")
    parser.add_argument("--dataset-dir", required=True, type=Path)
    parser.add_argument("--model-root", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--device", default="cuda" if torch.cuda.is_available() else "cpu")
    parser.add_argument("--limit", type=int)
    return parser.parse_args()


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def git_revision(root: Path) -> str | None:
    head = root / ".git" / "HEAD"
    try:
        value = head.read_text(encoding="utf-8").strip()
        if value.startswith("ref: "):
            return (root / ".git" / value[5:]).read_text(encoding="utf-8").strip()
        return value
    except OSError:
        return None


def load_external(root: Path, device: torch.device) -> dict[str, Any]:
    root = root.resolve()
    checkpoint = root / "checkpoints" / "keynet.pt"
    if not checkpoint.is_file():
        raise FileNotFoundError(f"MusicalKeyCNN checkpoint not found at {checkpoint}")
    sys.path.insert(0, str(root))
    from dataset import CAMELOT_MAPPING  # pylint: disable=import-error,import-outside-toplevel
    from eval import load_model  # pylint: disable=import-error,import-outside-toplevel
    from predict_keys import preprocess_audio  # pylint: disable=import-error,import-outside-toplevel

    labels: list[str | None] = [None] * 24
    for label, index in CAMELOT_MAPPING.items():
        if labels[int(index)] is None:
            labels[int(index)] = str(label)
    if any(label is None for label in labels):
        raise ValueError("External model vocabulary does not cover 24 classes")
    return {
        "model": load_model(checkpoint, device),
        "checkpoint": checkpoint,
        "labels": labels,
        "preprocess": preprocess_audio,
    }


def append_json(path: Path, value: dict[str, Any]) -> None:
    with path.open("a", encoding="utf-8", newline="\n") as handle:
        handle.write(json.dumps(value, separators=(",", ":")) + "\n")
        handle.flush()


def read_existing(path: Path) -> tuple[dict[str, Any] | None, set[str]]:
    metadata = None
    complete: set[str] = set()
    if not path.exists():
        return metadata, complete
    with path.open(encoding="utf-8") as handle:
        for line_number, line in enumerate(handle, start=1):
            if not line.strip():
                continue
            try:
                item = json.loads(line)
            except json.JSONDecodeError as error:
                raise ValueError(f"Invalid JSON at {path}:{line_number}: {error}") from error
            if item.get("type") == "metadata":
                metadata = item
            elif item.get("type") == "prediction" and item.get("status") == "ok":
                complete.add(str(item["track_id"]))
    return metadata, complete


def main() -> int:
    args = parse_args()
    device = torch.device(args.device)
    adapter = load_external(args.model_root, device)
    metadata = {
        "type": "metadata",
        "schema_version": 1,
        "model": "a1ex90/MusicalKeyCNN",
        "model_revision": git_revision(args.model_root),
        "checkpoint_sha256": sha256(adapter["checkpoint"]),
        "posterior_labels": adapter["labels"],
        "protocol": "external reference preprocessing: 44.1kHz, 105-bin CQT, hop 8820, full-track global pooling",
        "license": "MIT",
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    prior_metadata, complete = read_existing(args.output)
    if prior_metadata is None:
        append_json(args.output, metadata)
    elif prior_metadata != metadata:
        raise ValueError("Existing output metadata differs; choose a new output path")

    files = sorted((args.dataset_dir / "audio").glob("*.mp3"))
    if args.limit is not None:
        files = files[: args.limit]
    pending = [path for path in files if path.stem not in complete]
    print(
        f"MusicalKeyCNN export: selected={len(files)} cached={len(files) - len(pending)} "
        f"pending={len(pending)} device={device}",
        flush=True,
    )
    started = time.perf_counter()
    failures = 0
    for index, path in enumerate(pending, start=1):
        track_started = time.perf_counter()
        try:
            inputs = adapter["preprocess"](path).unsqueeze(0).to(device)
            with torch.inference_mode():
                posterior = adapter["model"](inputs).softmax(dim=1).squeeze(0).cpu()
            append_json(
                args.output,
                {
                    "type": "prediction",
                    "track_id": path.stem,
                    "status": "ok",
                    "posterior": [float(value) for value in posterior.tolist()],
                    "elapsed_ms": round((time.perf_counter() - track_started) * 1000),
                },
            )
        except Exception as error:  # keep corpus export resumable
            failures += 1
            append_json(
                args.output,
                {
                    "type": "prediction",
                    "track_id": path.stem,
                    "status": "error",
                    "error": f"{type(error).__name__}: {error}",
                },
            )
        if index == 1 or index % 10 == 0 or index == len(pending):
            elapsed = max(time.perf_counter() - started, 1e-9)
            print(
                f"processed={index}/{len(pending)} failed={failures} "
                f"rate={index / elapsed:.2f} tracks/s",
                flush=True,
            )
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
