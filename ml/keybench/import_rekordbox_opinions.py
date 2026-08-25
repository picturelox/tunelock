#!/usr/bin/env python3
"""Import a Rekordbox XML collection export as vendor-opinion data.

Rekordbox keys, BPMs, and beat grids are algorithmic opinions: they feed the
assisted leaderboard and disagreement mining, never the acoustic ground truth.
The importer emits one JSONL opinion per collection track plus an audit report
with coverage, vocabulary, file-existence, and MIK cross-vendor agreement.
"""

from __future__ import annotations

import argparse
from collections import Counter
import hashlib
import json
import os
from pathlib import Path
from typing import Any
from urllib.parse import unquote, urlparse
import xml.etree.ElementTree as ET


CANONICAL_LABELS = [
    "C major", "C# major", "D major", "D# major", "E major", "F major",
    "F# major", "G major", "G# major", "A major", "A# major", "B major",
    "C minor", "C# minor", "D minor", "D# minor", "E minor", "F minor",
    "F# minor", "G minor", "G# minor", "A minor", "A# minor", "B minor",
]

FLAT_TO_SHARP = {
    "Db": "C#", "Eb": "D#", "Gb": "F#", "Ab": "G#", "Bb": "A#",
    "Cb": "B", "Fb": "E", "B#": "C", "E#": "F",
}

# Mirror of proof::corpus::camelot_to_key. nB is the major key whose tonic is
# CAMELOT_MAJOR[n-1]; nA is the relative minor three semitones below it.
CAMELOT_MAJOR = [11, 6, 1, 8, 3, 10, 5, 0, 7, 2, 9, 4]


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Import Rekordbox XML vendor opinions")
    parser.add_argument("--xml", required=True, type=Path)
    parser.add_argument("--mik-csv", type=Path, help="Optional MIK library CSV for agreement audit")
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--report", required=True, type=Path)
    return parser.parse_args()


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def atomic_json(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f"{path.name}.part.{os.getpid()}")
    temporary.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")
    os.replace(temporary, path)


def rekordbox_tonality(value: str) -> int | None:
    """Map Rekordbox standard notation ('Am', 'F#', 'Bbm') to canonical index."""
    value = value.strip()
    if not value:
        return None
    minor = value.endswith("m")
    tonic = value[:-1] if minor else value
    tonic = FLAT_TO_SHARP.get(tonic, tonic)
    label = f"{tonic} {'minor' if minor else 'major'}"
    return CANONICAL_LABELS.index(label) if label in CANONICAL_LABELS else None


def camelot_index(code: str) -> int | None:
    """Map a MIK Camelot code ('8A', '12B') to the canonical 24-key index."""
    code = code.strip()
    if len(code) < 2:
        return None
    try:
        number = int(code[:-1])
    except ValueError:
        return None
    if not 1 <= number <= 12:
        return None
    major_tonic = CAMELOT_MAJOR[number - 1]
    if code[-1] == "B":
        return CANONICAL_LABELS.index(f"{_tonic_name(major_tonic)} major")
    if code[-1] == "A":
        return CANONICAL_LABELS.index(f"{_tonic_name((major_tonic + 9) % 12)} minor")
    return None


def _tonic_name(tonic: int) -> str:
    return ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"][tonic]


def mirex_category(truth: int, predicted: int) -> str:
    """MIREX-style relation between two canonical key indices."""
    if truth == predicted:
        return "correct"
    truth_tonic, truth_minor = truth % 12, truth >= 12
    pred_tonic, pred_minor = predicted % 12, predicted >= 12
    if truth_tonic == pred_tonic:
        return "parallel"
    if truth_minor == pred_minor and (truth_tonic - pred_tonic) % 12 in (7, 5):
        return "fifth"
    if truth_minor != pred_minor and (truth_tonic - pred_tonic) % 12 == (9 if truth_minor else 3):
        return "relative"
    if truth_minor == pred_minor and (truth_tonic - pred_tonic) % 12 in (1, 11):
        return "semitone"
    return "other"


def location_to_path(location: str) -> str:
    parsed = urlparse(location)
    path = unquote(parsed.path)
    if path.startswith("/") and len(path) > 2 and path[2] == ":":
        path = path[1:]
    return path.replace("/", "\\")


def normalized_location(path: str) -> str:
    return path.casefold().replace("/", "\\").strip().rstrip("\\")


def parse_collection(xml_path: Path) -> tuple[list[dict[str, Any]], Counter, Counter]:
    records: list[dict[str, Any]] = []
    unmapped_tonality: Counter = Counter()
    kinds: Counter = Counter()
    inside_collection = False
    pending_tempos: list[dict[str, Any]] = []
    for event, element in ET.iterparse(xml_path, events=("start", "end")):
        if event == "start" and element.tag == "COLLECTION":
            inside_collection = True
        elif event == "end" and element.tag == "COLLECTION":
            inside_collection = False
            element.clear()
        elif event == "end" and element.tag == "TEMPO" and inside_collection:
            pending_tempos.append(
                {"bpm": float(element.get("Bpm", 0)), "start_seconds": float(element.get("Inizio", 0))}
            )
            element.clear()
        elif event == "end" and element.tag == "TRACK" and inside_collection:
            tonality_raw = element.get("Tonality", "").strip()
            index = rekordbox_tonality(tonality_raw)
            if tonality_raw and index is None:
                unmapped_tonality[tonality_raw] += 1
            kinds[element.get("Kind", "")] += 1
            records.append(
                {
                    "source": "rekordbox",
                    "track_id": element.get("TrackID", ""),
                    "title": element.get("Name", ""),
                    "artist": element.get("Artist", ""),
                    "genre": element.get("Genre", ""),
                    "location": location_to_path(element.get("Location", "")),
                    "kind": element.get("Kind", ""),
                    "total_seconds": int(element.get("TotalTime", "0") or 0),
                    "key_raw": tonality_raw or None,
                    "canonical_index": index,
                    "canonical_label": CANONICAL_LABELS[index] if index is not None else None,
                    "bpm": float(element.get("AverageBpm", "0") or 0) or None,
                    "beatgrid": pending_tempos or None,
                }
            )
            pending_tempos = []
            element.clear()
    return records, unmapped_tonality, kinds


def load_mik_keys(mik_csv: Path) -> dict[str, int]:
    import csv as csv_module

    keys: dict[str, int] = {}
    with mik_csv.open(newline="", encoding="utf-8-sig") as handle:
        for row in csv_module.DictReader(handle):
            index = camelot_index(row.get("Key", ""))
            location = row.get("Location", "").strip()
            if index is not None and location:
                keys[normalized_location(location)] = index
    return keys


def main() -> int:
    args = parse_args()
    for artifact in (args.output, args.report):
        if artifact.exists():
            raise FileExistsError(f"Refusing to overwrite existing artifact: {artifact}")

    records, unmapped_tonality, kinds = parse_collection(args.xml)
    with_key = [record for record in records if record["canonical_index"] is not None]
    with_bpm = [record for record in records if record["bpm"]]
    exists = sum(1 for record in records if Path(record["location"]).exists())

    report: dict[str, Any] = {
        "schema_version": 1,
        "experiment": "rekordbox-vendor-opinion-import",
        "xml_sha256": sha256(args.xml),
        "script_sha256": sha256(Path(__file__)),
        "collection_tracks": len(records),
        "with_key": len(with_key),
        "with_bpm": len(with_bpm),
        "files_on_disk": exists,
        "unmapped_tonality": dict(unmapped_tonality),
        "format_counts": dict(kinds.most_common()),
        "key_distribution": dict(
            Counter(record["canonical_label"] for record in with_key).most_common()
        ),
        "top_genres": dict(
            Counter(record["genre"] for record in records if record["genre"]).most_common(40)
        ),
        "warning": "Rekordbox keys/BPMs are vendor opinions for the assisted leaderboard "
        "and disagreement mining; they are never acoustic ground truth.",
    }

    if args.mik_csv is not None:
        mik = load_mik_keys(args.mik_csv)
        categories: Counter = Counter()
        matched = 0
        for record in with_key:
            mik_index = mik.get(normalized_location(record["location"]))
            if mik_index is None:
                continue
            matched += 1
            categories[mirex_category(mik_index, record["canonical_index"])] += 1
        report["mik_crosscheck"] = {
            "mik_csv_sha256": sha256(args.mik_csv),
            "mik_rows_with_key": len(mik),
            "matched_by_location": matched,
            "mirex_categories": dict(categories),
            "exact_agreement_pct": round(100.0 * categories["correct"] / matched, 2) if matched else None,
        }

    atomic_json(args.report, report)
    temporary = args.output.with_name(f"{args.output.name}.part.{os.getpid()}")
    with temporary.open("w", encoding="utf-8") as handle:
        metadata = {
            "type": "metadata",
            "schema_version": 1,
            "model": "rekordbox/vendor-opinions",
            "xml_sha256": report["xml_sha256"],
            "posterior_labels": CANONICAL_LABELS,
            "protocol": "Rekordbox XML collection export; vendor opinion, never acoustic truth",
        }
        handle.write(json.dumps(metadata, separators=(",", ":")) + "\n")
        for record in records:
            handle.write(json.dumps(record, separators=(",", ":")) + "\n")
    os.replace(temporary, args.output)
    print(
        f"tracks={len(records)} with_key={len(with_key)} with_bpm={len(with_bpm)} "
        f"on_disk={exists} unmapped_tonality={sum(unmapped_tonality.values())}"
    )
    if "mik_crosscheck" in report:
        print(f"mik_agreement={report['mik_crosscheck']['mirex_categories']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
