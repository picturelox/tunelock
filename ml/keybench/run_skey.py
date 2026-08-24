#!/usr/bin/env python3
"""Export S-KEY posteriors without duplicating TuneLock's harmony logic.

This is a research adapter around an external, pinned S-KEY checkout. It writes
one metadata line followed by one JSON object per track. The labels travel with
the posterior so TuneLock's Rust proof code remains responsible for parsing and
scoring keys.

The output is append-only and resumable. It belongs under ``ml/data/`` (which
is gitignored); model weights and benchmark audio are never bundled.
"""

from __future__ import annotations

import argparse
import json
import sys
import time
from pathlib import Path
from typing import Any

import torch


SCHEMA_VERSION = 1


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Export S-KEY 24-key posteriors")
    parser.add_argument("--dataset-dir", required=True, type=Path)
    parser.add_argument("--skey-root", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--device", default="cuda" if torch.cuda.is_available() else "cpu")
    parser.add_argument("--limit", type=int)
    parser.add_argument(
        "--duration-seconds",
        type=float,
        default=0.0,
        help="Centered crop length; 0 preserves the model's full-track protocol",
    )
    return parser.parse_args()


def load_adapter(skey_root: Path, device: torch.device) -> dict[str, Any]:
    root = skey_root.resolve()
    if not (root / "skey" / "key_detection.py").is_file():
        raise FileNotFoundError(f"S-KEY checkout not found at {root}")
    sys.path.insert(0, str(root))

    from skey.key_detection import (  # pylint: disable=import-error,import-outside-toplevel
        DEFAULT_CHECKPOINT_PATH,
        key_map,
        load_audio,
        load_checkpoint,
        load_model_components,
    )

    checkpoint = load_checkpoint(DEFAULT_CHECKPOINT_PATH)
    hcqt, chromanet, crop_fn = load_model_components(checkpoint, device)
    labels = [key_map[index] for index in range(24)]
    return {
        "checkpoint": checkpoint,
        "checkpoint_path": Path(DEFAULT_CHECKPOINT_PATH),
        "hcqt": hcqt,
        "chromanet": chromanet,
        "crop_fn": crop_fn,
        "labels": labels,
        "load_audio": load_audio,
    }


def metadata(adapter: dict[str, Any], args: argparse.Namespace) -> dict[str, Any]:
    checkpoint = adapter["checkpoint"]
    return {
        "type": "metadata",
        "schema_version": SCHEMA_VERSION,
        "model": "deezer/skey",
        "model_revision": git_revision(args.skey_root),
        "checkpoint": adapter["checkpoint_path"].name,
        "sample_rate": int(checkpoint["audio"]["sr"]),
        "posterior_labels": adapter["labels"],
        "aggregation": "mean model output across model batches",
        "duration_seconds": args.duration_seconds,
        "protocol": "full-track" if args.duration_seconds <= 0 else "center-crop",
    }


def git_revision(root: Path) -> str | None:
    head = root / ".git" / "HEAD"
    try:
        ref = head.read_text(encoding="utf-8").strip()
        if ref.startswith("ref: "):
            return (root / ".git" / ref[5:]).read_text(encoding="utf-8").strip()
        return ref
    except OSError:
        return None


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


def centered_crop(audio: torch.Tensor, sample_rate: int, seconds: float) -> torch.Tensor:
    if seconds <= 0:
        return audio
    wanted = int(round(sample_rate * seconds))
    if audio.shape[-1] <= wanted:
        return audio
    start = (audio.shape[-1] - wanted) // 2
    return audio[..., start : start + wanted]


def infer_posterior(adapter: dict[str, Any], audio: torch.Tensor, device: torch.device) -> torch.Tensor:
    batch = audio.unsqueeze(0).to(device)
    with torch.inference_mode():
        cropped = adapter["crop_fn"](adapter["hcqt"](batch), torch.zeros(1, device=device))
        outputs = adapter["chromanet"](cropped)
        posterior = outputs.mean(dim=0)
        posterior = posterior / posterior.sum().clamp_min(1e-12)
    return posterior.detach().cpu()


def audio_files(dataset_dir: Path) -> list[Path]:
    audio_dir = dataset_dir / "audio"
    files: list[Path] = []
    for extension in ("*.mp3", "*.flac", "*.wav"):
        files.extend(audio_dir.glob(extension))
    return sorted(files, key=lambda path: path.name)


def append_json(output: Path, item: dict[str, Any]) -> None:
    with output.open("a", encoding="utf-8", newline="\n") as handle:
        json.dump(item, handle, ensure_ascii=True, separators=(",", ":"))
        handle.write("\n")
        handle.flush()


def main() -> int:
    args = parse_args()
    if args.device.startswith("cuda") and not torch.cuda.is_available():
        raise RuntimeError("CUDA requested but PyTorch cannot see a CUDA device")
    device = torch.device(args.device)

    files = audio_files(args.dataset_dir)
    if args.limit is not None:
        files = files[: args.limit]
    if not files:
        raise FileNotFoundError(f"No benchmark audio found under {args.dataset_dir / 'audio'}")

    adapter = load_adapter(args.skey_root, device)
    expected_metadata = metadata(adapter, args)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    existing_metadata, completed = read_existing(args.output)
    if existing_metadata is None:
        append_json(args.output, expected_metadata)
    elif existing_metadata != expected_metadata:
        raise ValueError(
            "Output metadata does not match this run. Choose a new output path "
            "instead of mixing model revisions or crop protocols."
        )

    checkpoint = adapter["checkpoint"]
    sample_rate = int(checkpoint["audio"]["sr"])
    remaining = [path for path in files if path.stem not in completed]
    print(
        f"S-KEY posterior export: {len(files)} selected, {len(completed)} complete, "
        f"{len(remaining)} remaining, device={device}, protocol={expected_metadata['protocol']}",
        flush=True,
    )

    started = time.perf_counter()
    ok = 0
    failed = 0
    for index, path in enumerate(remaining, start=1):
        track_started = time.perf_counter()
        try:
            audio = adapter["load_audio"](str(path), sample_rate)
            audio = centered_crop(audio, sample_rate, args.duration_seconds)
            posterior = infer_posterior(adapter, audio, device)
            predicted_index = int(posterior.argmax().item())
            elapsed_ms = round((time.perf_counter() - track_started) * 1000)
            append_json(
                args.output,
                {
                    "type": "prediction",
                    "track_id": path.stem,
                    "file_name": path.name,
                    "status": "ok",
                    "posterior": [round(float(value), 10) for value in posterior.tolist()],
                    "predicted_index": predicted_index,
                    "predicted_label": adapter["labels"][predicted_index],
                    "elapsed_ms": elapsed_ms,
                },
            )
            ok += 1
        except Exception as exc:  # continue so a long corpus run remains resumable
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

        done = index
        if done == 1 or done % 10 == 0 or done == len(remaining):
            elapsed = time.perf_counter() - started
            rate = done / max(elapsed, 1e-9)
            eta_minutes = (len(remaining) - done) / max(rate, 1e-9) / 60.0
            print(
                f"[{done}/{len(remaining)}] ok={ok} failed={failed} "
                f"rate={rate * 60:.1f}/min eta={eta_minutes:.1f}m",
                flush=True,
            )

    return 0 if failed == 0 else 2


if __name__ == "__main__":
    raise SystemExit(main())
