#!/usr/bin/env python3
"""Cache pinned Myna embeddings for the Rust-canonical key corpus manifest.

This is an experiment adapter, not product runtime code. Original audio is only
read. Each track is committed as an independent NumPy file so a long extraction
can resume without rewriting successful work.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import sys
import time
from typing import Any

import numpy as np
import torch
import transformers
from transformers import AutoModel


DEFAULT_MODEL = "oriyonay/myna-vertical"
DEFAULT_REVISION = "6b9e1e5aae0832335d61d7a38764114e496824d4"
SAFE_ID = re.compile(r"^[A-Za-z0-9._-]+$")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Cache Myna key-benchmark embeddings")
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--cache-dir", required=True, type=Path)
    parser.add_argument("--hf-cache", required=True, type=Path)
    parser.add_argument("--model", default=DEFAULT_MODEL)
    parser.add_argument("--revision", default=DEFAULT_REVISION)
    parser.add_argument("--n-samples", type=int, default=100_000)
    parser.add_argument(
        "--embedding-dim",
        type=int,
        default=384,
        help="Expected backbone output width (384 for Myna-Vertical, 1536 for Myna-85M).",
    )
    parser.add_argument(
        "--role",
        choices=("training", "development", "all"),
        default="all",
    )
    parser.add_argument("--device", default="cuda" if torch.cuda.is_available() else "cpu")
    parser.add_argument("--limit", type=int)
    return parser.parse_args()


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def load_manifest(path: Path) -> dict[str, Any]:
    data = json.loads(path.read_text(encoding="utf-8"))
    if data.get("schema_version") != 1:
        raise ValueError(f"Unsupported manifest schema: {data.get('schema_version')}")
    labels = data.get("canonical_labels", [])
    if len(labels) != 24:
        raise ValueError("Manifest must contain exactly 24 canonical labels")
    return data


def cache_path(root: Path, record: dict[str, Any]) -> Path:
    track_id = str(record["id"])
    corpus = str(record["corpus"])
    if not SAFE_ID.fullmatch(track_id) or not SAFE_ID.fullmatch(corpus):
        raise ValueError(f"Unsafe manifest identity: {corpus}/{track_id}")
    return root / corpus / f"{track_id}.npy"


def valid_cached(path: Path, embedding_dim: int) -> bool:
    if not path.is_file():
        return False
    try:
        value = np.load(path, mmap_mode="r", allow_pickle=False)
        return value.ndim == 2 and value.shape[0] > 0 and value.shape[1] == embedding_dim
    except (OSError, ValueError):
        return False


def atomic_save(path: Path, embeddings: np.ndarray) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f"{path.name}.part.{os.getpid()}.npy")
    if temporary.exists():
        raise FileExistsError(f"Refusing to overwrite stale temporary file: {temporary}")
    np.save(temporary, embeddings, allow_pickle=False)
    os.replace(temporary, path)


def write_metadata(
    root: Path,
    args: argparse.Namespace,
    manifest_hash: str,
    complete: int,
    failed: list[dict[str, str]],
) -> None:
    metadata = {
        "schema_version": 1,
        "adapter": "tunelock/myna-embedding-cache",
        "model": args.model,
        "model_revision": args.revision,
        "model_license": "MIT",
        "manifest_sha256": manifest_hash,
        "n_samples": args.n_samples,
        "embedding_dim": args.embedding_dim,
        "torch": torch.__version__,
        "transformers": transformers.__version__,
        "device": args.device,
        "complete": complete,
        "failed": failed,
    }
    target = root / "metadata.json"
    temporary = target.with_name(f"{target.name}.part.{os.getpid()}")
    temporary.write_text(json.dumps(metadata, indent=2) + "\n", encoding="utf-8")
    os.replace(temporary, target)


def validate_existing_metadata(
    root: Path,
    args: argparse.Namespace,
    manifest_hash: str,
) -> None:
    path = root / "metadata.json"
    if not path.exists():
        return
    metadata = json.loads(path.read_text(encoding="utf-8"))
    matches = (
        metadata.get("adapter") == "tunelock/myna-embedding-cache"
        and metadata.get("model") == args.model
        and metadata.get("model_revision") == args.revision
        and metadata.get("manifest_sha256") == manifest_hash
        and int(metadata.get("n_samples", -1)) == args.n_samples
        and int(metadata.get("embedding_dim", 384)) == args.embedding_dim
    )
    if not matches:
        raise ValueError(
            f"Embedding cache metadata does not match this run: {path}; use a new cache directory"
        )


def main() -> int:
    args = parse_args()
    if args.embedding_dim < 1:
        raise ValueError("--embedding-dim must be positive")
    manifest = load_manifest(args.manifest)
    records = [
        record
        for record in manifest["records"]
        if args.role == "all" or record["role"] == args.role
    ]
    if args.limit is not None:
        records = records[: args.limit]
    if not records:
        raise ValueError("No manifest records selected")

    args.cache_dir.mkdir(parents=True, exist_ok=True)
    args.hf_cache.mkdir(parents=True, exist_ok=True)
    manifest_hash = sha256(args.manifest)
    validate_existing_metadata(args.cache_dir, args, manifest_hash)
    pending = [
        record
        for record in records
        if not valid_cached(cache_path(args.cache_dir, record), args.embedding_dim)
    ]
    print(
        f"selected={len(records)} cached={len(records) - len(pending)} pending={len(pending)} "
        f"device={args.device}",
        flush=True,
    )
    if not pending:
        write_metadata(args.cache_dir, args, manifest_hash, len(records), [])
        return 0

    model = AutoModel.from_pretrained(
        args.model,
        revision=args.revision,
        trust_remote_code=True,
        cache_dir=args.hf_cache,
    ).to(args.device)
    model.eval()
    model.config.n_samples = args.n_samples
    model.config.n_frames = model.config._get_n_frames(args.n_samples)

    failures: list[dict[str, str]] = []
    started = time.perf_counter()
    for index, record in enumerate(pending, start=1):
        destination = cache_path(args.cache_dir, record)
        try:
            with torch.inference_mode():
                embeddings = model.from_file(str(record["audio_path"]))
            value = embeddings.detach().to(device="cpu", dtype=torch.float32).numpy()
            if (
                value.ndim != 2
                or value.shape[0] < 1
                or value.shape[1] != args.embedding_dim
            ):
                raise ValueError(f"unexpected embedding shape {value.shape}")
            if not np.isfinite(value).all():
                raise ValueError("non-finite embedding")
            atomic_save(destination, value)
        except Exception as error:  # keep the multi-hour extraction resumable
            failures.append({"id": str(record["id"]), "error": repr(error)})

        if index == 1 or index % 25 == 0 or index == len(pending):
            elapsed = max(time.perf_counter() - started, 1e-9)
            rate = index / elapsed
            print(
                f"processed={index}/{len(pending)} failed={len(failures)} "
                f"rate={rate:.2f} tracks/s",
                flush=True,
            )
            complete = sum(
                valid_cached(cache_path(args.cache_dir, row), args.embedding_dim)
                for row in records
            )
            write_metadata(args.cache_dir, args, manifest_hash, complete, failures)

    complete = sum(
        valid_cached(cache_path(args.cache_dir, row), args.embedding_dim) for row in records
    )
    print(f"complete={complete}/{len(records)} failed={len(failures)}", flush=True)
    return 1 if failures else 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except KeyboardInterrupt:
        print("Interrupted; completed per-track caches are preserved.", file=sys.stderr)
        raise SystemExit(130)
