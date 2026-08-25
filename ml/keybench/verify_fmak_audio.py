#!/usr/bin/env python3
"""Verify FMAK audio extraction, backfill recording MD5s, and audit duplicates.

After all FMAK archives are downloaded and extracted, this script:
  1. Checks that every manifest record has a real MP3 on disk.
  2. Computes MD5 per file and backfills recording_md5 into the manifest.
  3. Detects exact audio duplicates (same MD5, different track IDs).
  4. Detects label conflicts (same MD5, different key labels).
  5. Cross-references recording MD5s against the GiantSteps manifest for
     recording-family overlap.
  6. Writes an updated manifest and an audit report.

All outputs are local. The original manifest is not overwritten; a new
versioned manifest is written alongside the report.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from collections import defaultdict
from pathlib import Path
from typing import Any


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Verify FMAK audio, backfill MD5s, audit duplicates"
    )
    parser.add_argument("--manifest", required=True, type=Path,
                        help="Input FMAK manifest JSON (from export_fmak_manifest.py)")
    parser.add_argument("--giantsteps-manifest", required=True, type=Path,
                        help="GiantSteps manifest JSON for recording-MD5 overlap check")
    parser.add_argument("--output", required=True, type=Path,
                        help="Output manifest JSON with backfilled recording_md5")
    parser.add_argument("--report", required=True, type=Path,
                        help="Output audit report JSON")
    return parser.parse_args()


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


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


def main() -> int:
    args = parse_args()
    for artifact in (args.output, args.report):
        if artifact.exists():
            raise FileExistsError(f"Refusing to overwrite existing artifact: {artifact}")

    manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
    records = manifest["records"]
    print(f"Loaded {len(records)} FMAK records", flush=True)

    # Load GiantSteps MD5s for overlap check
    gs_manifest = json.loads(args.giantsteps_manifest.read_text(encoding="utf-8"))
    gs_md5s: dict[str, list[str]] = defaultdict(list)
    for gs_record in gs_manifest.get("records", []):
        md5_val = gs_record.get("recording_md5", "")
        if md5_val:
            gs_md5s[md5_val].append(gs_record.get("id", ""))
    print(f"Loaded {len(gs_md5s)} GiantSteps recording MD5s", flush=True)

    # Phase 1: verify file existence and compute MD5s
    missing_files: list[dict[str, Any]] = []
    md5_to_records: dict[str, list[dict[str, Any]]] = defaultdict(list)
    gs_overlap: list[dict[str, Any]] = []
    backfilled = 0

    for i, record in enumerate(records, 1):
        audio_path = Path(record["audio_path"])
        if not audio_path.exists():
            missing_files.append({"id": record["id"], "audio_path": str(audio_path)})
            continue
        file_md5 = md5(audio_path)
        record["recording_md5"] = file_md5
        backfilled += 1
        md5_to_records[file_md5].append(record)
        if file_md5 in gs_md5s:
            gs_overlap.append({
                "fmak_id": record["id"],
                "fmak_artist": record.get("artist", ""),
                "fmak_label": record["truth_label"],
                "giantsteps_ids": gs_md5s[file_md5],
                "recording_md5": file_md5,
            })
        if i % 500 == 0:
            print(f"  processed {i}/{len(records)} files", flush=True)

    # Phase 2: detect duplicates and label conflicts
    duplicates: list[dict[str, Any]] = []
    label_conflicts: list[dict[str, Any]] = []
    for file_md5, group in md5_to_records.items():
        if len(group) < 2:
            continue
        ids = [r["id"] for r in group]
        labels = set(r["truth_label"] for r in group)
        entry = {
            "recording_md5": file_md5,
            "track_ids": ids,
            "artists": list(set(r.get("artist", "") for r in group)),
            "labels": list(labels),
        }
        duplicates.append(entry)
        if len(labels) > 1:
            label_conflicts.append(entry)

    # Phase 3: build report
    existing = len(records) - len(missing_files)
    report: dict[str, Any] = {
        "schema_version": 1,
        "experiment": "fmak-audio-verification-and-dedup",
        "manifest_sha256": sha256(args.manifest),
        "giantsteps_manifest_sha256": sha256(args.giantsteps_manifest),
        "script_sha256": sha256(Path(__file__)),
        "total_records": len(records),
        "files_on_disk": existing,
        "missing_files": missing_files,
        "missing_count": len(missing_files),
        "md5_backfilled": backfilled,
        "unique_recordings": len(md5_to_records),
        "duplicate_groups": len(duplicates),
        "duplicate_tracks": sum(len(g) - 1 for g in duplicates),
        "duplicates": duplicates,
        "label_conflicts": label_conflicts,
        "label_conflict_count": len(label_conflicts),
        "giantsteps_recording_overlap": gs_overlap,
        "giantsteps_recording_overlap_count": len(gs_overlap),
        "warning": "FMAK audio licenses are per-track; noncommercial records must be "
        "excluded from commercial training paths. Recording MD5 enables dedup and "
        "artist/recording-disjoint fold computation at training time.",
    }

    atomic_json(args.report, report)

    # Write updated manifest with backfilled MD5s
    manifest["records"] = records
    atomic_json(args.output, manifest)

    print(
        f"files_on_disk={existing}/{len(records)} missing={len(missing_files)} "
        f"unique={len(md5_to_records)} duplicates={len(duplicates)} "
        f"label_conflicts={len(label_conflicts)} gs_overlap={len(gs_overlap)}",
        flush=True,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
