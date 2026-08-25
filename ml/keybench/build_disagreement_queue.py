#!/usr/bin/env python3
"""Build a blind-labeling queue from Rekordbox vs MIK disagreements.

Reads the Rekordbox opinion JSONL produced by import_rekordbox_opinions.py and
a Mixed In Key CSV, then emits a prioritized queue of tracks where the two
vendors disagree. The queue is designed for blind human adjudication: engine
and vendor answers are hidden from the labeler, only the audio path and a
neutral track identifier are exposed.

Priority order (highest disagreement first):
  1. "other"     — completely different key, no MIREX relation
  2. "semitone"  — off by a half step (common misclassification)
  3. "parallel"  — same tonic, wrong mode
  4. "relative"  — relative major/minor confusion
  5. "fifth"     — perfect fifth apart (compatible in DJ mixing)

Output: JSONL with one entry per disagreement track, sorted by priority then
by track title. Each entry contains:
  - queue_id: stable sequential ID for labeling workflow
  - location: local audio path for the labeler
  - title, artist: for identification (hidden from labeler in blind mode)
  - rekordbox_label, mik_label: vendor opinions (hidden from labeler)
  - mirex_category: the disagreement type (hidden from labeler)
  - priority: numeric priority (1=highest)

All output is local-only and gitignored.
"""

from __future__ import annotations

import argparse
import csv as csv_module
import hashlib
import json
import os
from pathlib import Path
from typing import Any

from import_rekordbox_opinions import (
    CANONICAL_LABELS,
    camelot_index,
    mirex_category,
    normalized_location,
)


PRIORITY_ORDER = {
    "other": 1,
    "semitone": 2,
    "parallel": 3,
    "relative": 4,
    "fifth": 5,
}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Build a blind-labeling queue from Rekordbox vs MIK disagreements"
    )
    parser.add_argument("--opinions", required=True, type=Path,
                        help="Rekordbox opinion JSONL from import_rekordbox_opinions.py")
    parser.add_argument("--mik-csv", required=True, type=Path,
                        help="Mixed In Key library CSV")
    parser.add_argument("--output", required=True, type=Path,
                        help="Output disagreement queue JSONL")
    parser.add_argument("--report", required=True, type=Path,
                        help="Output queue summary report JSON")
    return parser.parse_args()


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def load_mik_keys(mik_csv: Path) -> dict[str, dict[str, Any]]:
    """Load MIK CSV keyed by normalized location -> {key_index, key_label, bpm, title, artist}."""
    keys: dict[str, dict[str, Any]] = {}
    with mik_csv.open(newline="", encoding="utf-8-sig") as handle:
        for row in csv_module.DictReader(handle):
            index = camelot_index(row.get("Key", ""))
            location = row.get("Location", "").strip()
            if index is not None and location:
                keys[normalized_location(location)] = {
                    "key_index": index,
                    "key_label": CANONICAL_LABELS[index],
                    "bpm": float(row.get("Tempo", "0") or 0) or None,
                    "title": row.get("Title", ""),
                    "artist": row.get("Artist", ""),
                }
    return keys


def load_rekordbox_opinions(opinions_path: Path) -> list[dict[str, Any]]:
    records: list[dict[str, Any]] = []
    with opinions_path.open("r", encoding="utf-8") as handle:
        for line in handle:
            entry = json.loads(line)
            if entry.get("type") == "metadata":
                continue
            if entry.get("canonical_index") is not None:
                records.append(entry)
    return records


def build_queue(
    rekordbox_records: list[dict[str, Any]],
    mik_keys: dict[str, dict[str, Any]],
) -> list[dict[str, Any]]:
    queue: list[dict[str, Any]] = []
    for record in rekordbox_records:
        mik = mik_keys.get(normalized_location(record["location"]))
        if mik is None:
            continue
        category = mirex_category(mik["key_index"], record["canonical_index"])
        if category == "correct":
            continue
        queue.append({
            "location": record["location"],
            "title": record.get("title") or mik.get("title", ""),
            "artist": record.get("artist") or mik.get("artist", ""),
            "rekordbox_label": record["canonical_label"],
            "mik_label": mik["key_label"],
            "rekordbox_bpm": record.get("bpm"),
            "mik_bpm": mik.get("bpm"),
            "mirex_category": category,
            "priority": PRIORITY_ORDER.get(category, 99),
            "file_exists": Path(record["location"]).exists(),
        })
    queue.sort(key=lambda e: (e["priority"], e["title"].casefold(), e["artist"].casefold()))
    for i, entry in enumerate(queue, 1):
        entry["queue_id"] = i
    return queue


def main() -> int:
    args = parse_args()
    for artifact in (args.output, args.report):
        if artifact.exists():
            raise FileExistsError(f"Refusing to overwrite existing artifact: {artifact}")

    mik_keys = load_mik_keys(args.mik_csv)
    rekordbox_records = load_rekordbox_opinions(args.opinions)
    queue = build_queue(rekordbox_records, mik_keys)

    from collections import Counter
    category_counts = Counter(e["mirex_category"] for e in queue)
    file_exists_count = sum(1 for e in queue if e["file_exists"])

    report: dict[str, Any] = {
        "schema_version": 1,
        "experiment": "rekordbox-mik-disagreement-queue",
        "opinions_sha256": sha256(args.opinions),
        "mik_csv_sha256": sha256(args.mik_csv),
        "script_sha256": sha256(Path(__file__)),
        "rekordbox_records_with_key": len(rekordbox_records),
        "mik_rows_with_key": len(mik_keys),
        "matched_by_location": len(rekordbox_records) - sum(
            1 for r in rekordbox_records
            if normalized_location(r["location"]) not in mik_keys
        ),
        "disagreement_count": len(queue),
        "disagreement_with_file_on_disk": file_exists_count,
        "category_breakdown": dict(category_counts.most_common()),
        "priority_order": {v: k for k, v in PRIORITY_ORDER.items()},
        "warning": "Vendor opinions are hidden from labelers in blind mode. "
        "Only audio and a neutral ID should be shown.",
    }

    args.output.parent.mkdir(parents=True, exist_ok=True)
    tmp_output = args.output.with_name(f"{args.output.name}.part.{os.getpid()}")
    with tmp_output.open("w", encoding="utf-8") as handle:
        for entry in queue:
            handle.write(json.dumps(entry, separators=(",", ":")) + "\n")
    os.replace(tmp_output, args.output)

    tmp_report = args.report.with_name(f"{args.report.name}.part.{os.getpid()}")
    tmp_report.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    os.replace(tmp_report, args.report)

    print(
        f"disagreements={len(queue)} with_file={file_exists_count} "
        f"categories={dict(category_counts.most_common())}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
