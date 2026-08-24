#!/usr/bin/env python3
"""Download, checksum-verify, and extract the pinned FMAK audio archives.

Every zip is pinned by Zenodo name, byte size, and MD5 from record
10.5281/zenodo.10719860. Downloads resume with HTTP Range requests, archives
are verified before extraction, and a status JSON makes the whole run
resumable. Audio is extracted under <root>/audio/<sub>/<track_id>.mp3.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import time
from typing import Any
import urllib.request
import zipfile


ZENODO = "https://zenodo.org/api/records/10719860/files/{name}/content"
ARCHIVES = [
    ("000-019.zip", 4150442299, "b86f6414820c1422b2c6cdf87be1ef3a"),
    ("020-039.zip", 4556521374, "a2da8377fdbc1d3a1f54dd60aa7b8f9b"),
    ("040-049.zip", 3250881628, "d70babe5f66bdf3e821c42a8b8aafb9b"),
    ("050-059.zip", 3902617703, "f53fcba704fce27e5c7f3ec2532dcb44"),
    ("060-069.zip", 3615294610, "1520f067d7caaf0813780ff69bc4ba85"),
    ("070-079.zip", 3433068795, "186643746fcb1f4722a28d3eb9c6b99c"),
    ("080-089.zip", 5024802097, "8cf882609fc2f301621c2e9f9da03214"),
    ("090-099.zip", 4825903776, "84f0f036e3778ffd97c10b591f803d06"),
    ("100-109.zip", 4767776779, "4a307f019d3354064814f05d1dffa1e2"),
    ("110-124.zip", 1734477426, "88d7dbcca82189ed75b7baa5aa132fc1"),
]


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Download pinned FMAK audio")
    parser.add_argument("--root", required=True, type=Path,
                        help="Data root; zips go to <root>/zips, audio to <root>/audio")
    parser.add_argument("--status", required=True, type=Path)
    parser.add_argument("--only", type=str,
                        help="Process only these archives (comma-separated names)")
    parser.add_argument("--list-only", action="store_true",
                        help="Download and verify but only list zip contents")
    parser.add_argument("--keep-zips", action="store_true",
                        help="Keep verified archives after extraction")
    return parser.parse_args()


def md5(path: Path) -> str:
    digest = hashlib.md5()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(8 * 1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def atomic_json(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f"{path.name}.part.{os.getpid()}")
    temporary.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")
    os.replace(temporary, path)


def download(url: str, target: Path, expected_size: int) -> None:
    """Resume-capable download; restarts if the server ignores Range."""
    while True:
        have = target.stat().st_size if target.exists() else 0
        if have == expected_size:
            return
        if have > expected_size:
            target.unlink()
            have = 0
        request = urllib.request.Request(url)
        if have:
            request.add_header("Range", f"bytes={have}-")
        with urllib.request.urlopen(request) as response:
            append = response.status == 206 and have > 0
            mode = "ab" if append else "wb"
            started = time.perf_counter()
            done = have if append else 0
            with target.open(mode) as handle:
                while True:
                    block = response.read(8 * 1024 * 1024)
                    if not block:
                        break
                    handle.write(block)
                    done += len(block)
                    elapsed = max(time.perf_counter() - started, 1e-9)
                    rate = (done - have) / elapsed / (1024 * 1024)
                    print(
                        f"  {target.name}: {done / 1e9:.2f}/{expected_size / 1e9:.2f} GB "
                        f"({rate:.1f} MB/s)",
                        flush=True,
                    )
        # Connection may drop mid-transfer; loop resumes from disk size.


def extract(archive: Path, audio_root: Path) -> int:
    extracted = 0
    with zipfile.ZipFile(archive) as bundle:
        for name in bundle.namelist():
            leaf = Path(name).name
            stem = Path(leaf).stem
            if not leaf.endswith(".mp3") or not (stem.isdigit() and len(stem) == 6):
                continue
            target = audio_root / stem[:3] / leaf
            if target.exists() and target.stat().st_size == bundle.getinfo(name).file_size:
                extracted += 1
                continue
            target.parent.mkdir(parents=True, exist_ok=True)
            temporary = target.with_name(f"{target.name}.part.{os.getpid()}")
            with bundle.open(name) as source, temporary.open("wb") as handle:
                while True:
                    block = source.read(8 * 1024 * 1024)
                    if not block:
                        break
                    handle.write(block)
            os.replace(temporary, target)
            extracted += 1
    return extracted


def main() -> int:
    args = parse_args()
    zips = args.root / "zips"
    audio = args.root / "audio"
    zips.mkdir(parents=True, exist_ok=True)
    audio.mkdir(parents=True, exist_ok=True)
    status: dict[str, Any] = json.loads(args.status.read_text(encoding="utf-8")) if args.status.exists() else {}
    selected = set(args.only.split(",")) if args.only else None
    archives = [entry for entry in ARCHIVES if selected is None or entry[0] in selected]
    for name, size, checksum in archives:
        archive = zips / name
        if status.get(name, {}).get("extracted") and not args.list_only:
            print(f"{name}: already extracted, skipping", flush=True)
            continue
        print(f"{name}: downloading ({size / 1e9:.2f} GB)", flush=True)
        download(ZENODO.format(name=name), archive, size)
        print(f"{name}: verifying md5", flush=True)
        digest = md5(archive)
        if digest != checksum:
            raise ValueError(f"{name} md5 mismatch: {digest} != {checksum}")
        if args.list_only:
            with zipfile.ZipFile(archive) as bundle:
                entries = bundle.namelist()
            print(f"{name}: {len(entries)} entries, first: {entries[:5]}", flush=True)
            continue
        count = extract(archive, audio)
        status[name] = {"md5": checksum, "size": size, "extracted": True, "mp3_written": count}
        atomic_json(args.status, status)
        print(f"{name}: extracted {count} mp3 files", flush=True)
        if not args.keep_zips:
            archive.unlink()
    complete = sum(1 for entry in status.values() if entry.get("extracted"))
    print(f"archives complete: {complete}/{len(ARCHIVES)}", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
