#!/usr/bin/env python3
"""Audit FMAK/FMAKv2 key annotations and export a Rust-canonical corpus manifest.

The audit joins FMAKv2 labels with pinned FMA metadata, classifies per-track
Creative Commons licenses, measures artist-token overlap with the GiantSteps
corpora, and only then emits a schema-1 manifest. Audio download is a separate
step; records carry the expected FMA relative path pattern.
"""

from __future__ import annotations

import argparse
import csv
import hashlib
import json
import os
from pathlib import Path
from typing import Any

import pandas as pd

from train_myna_head import normalized_artist_tokens


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


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Audit/export FMAK key corpus")
    parser.add_argument("--fmak-csv", required=True, type=Path)
    parser.add_argument("--fma-tracks", required=True, type=Path)
    parser.add_argument("--fma-raw", required=True, type=Path)
    parser.add_argument("--mtg-manifest", required=True, type=Path)
    parser.add_argument("--report", required=True, type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument(
        "--audio-root",
        default="ground-truth\\fmak\\audio",
        help="Relative root prefix for expected per-record audio paths",
    )
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


def canonical_index(key_and_mode: str) -> int | None:
    """Map an FMAK label like 'F# Major' or 'Bb minor' to the canonical index."""
    parts = key_and_mode.strip().split()
    if len(parts) != 2:
        return None
    tonic, mode = parts
    tonic = FLAT_TO_SHARP.get(tonic, tonic)
    mode = mode.casefold()
    if mode not in ("major", "minor"):
        return None
    label = f"{tonic} {mode}"
    return CANONICAL_LABELS.index(label) if label in CANONICAL_LABELS else None


def license_class(title: str, url: str) -> str:
    """Classify a Creative Commons license for the commercial-rights audit."""
    text = f"{title} {url}".casefold()
    if "public domain" in text or "cc0" in text:
        return "public_domain"
    if "noncommercial" in text or "-nc" in text or "nc-" in text:
        return "noncommercial"
    if "sharealike" in text or "-sa" in text:
        return "cc_by_sa"
    if "attribution" in text or "by/4.0" in text or "by/3.0" in text:
        return "cc_by"
    return "unknown"


def load_fmak(path: Path) -> dict[int, str]:
    with path.open(newline="", encoding="utf-8") as handle:
        rows = {
            int(row["track_id"]): row["key_and_mode"].strip()
            for row in csv.DictReader(handle)
            if row.get("track_id") and row.get("key_and_mode")
        }
    if not rows:
        raise ValueError(f"No FMAK labels parsed from {path}")
    return rows


def main() -> int:
    args = parse_args()
    for artifact in (args.report, args.output):
        if artifact is not None and artifact.exists():
            raise FileExistsError(f"Refusing to overwrite existing artifact: {artifact}")

    labels = load_fmak(args.fmak_csv)
    raw = pd.read_csv(args.fma_raw)
    raw = raw.set_index("track_id")
    tracks = pd.read_csv(args.fma_tracks, header=[0, 1], index_col=0)
    genre_top = tracks[("track", "genre_top")]

    manifest_labels = json.loads(args.mtg_manifest.read_text(encoding="utf-8"))
    if manifest_labels.get("canonical_labels") != CANONICAL_LABELS:
        raise ValueError("MTG manifest canonical labels differ from this exporter")
    giantsteps_artists: dict[str, int] = {}
    for record in manifest_labels["records"]:
        for token in normalized_artist_tokens(str(record.get("artist", ""))):
            giantsteps_artists.setdefault(token, 0)
            giantsteps_artists[token] += 1

    records: list[dict[str, Any]] = []
    missing_metadata: list[int] = []
    unmapped_labels: list[dict[str, Any]] = []
    overlap_tracks: list[dict[str, Any]] = []
    for track_id in sorted(labels):
        index = canonical_index(labels[track_id])
        if index is None:
            unmapped_labels.append({"track_id": track_id, "label": labels[track_id]})
            continue
        if track_id not in raw.index:
            missing_metadata.append(track_id)
            continue
        row = raw.loc[track_id]
        if isinstance(row, pd.DataFrame):
            row = row.iloc[0]
        artist = str(row.get("artist_name", "")).strip()
        title = str(row.get("track_title", "")).strip()
        license_title = str(row.get("license_title", "")).strip()
        license_url = str(row.get("license_url", "")).strip()
        genre = ""
        if track_id in genre_top.index:
            value = genre_top.loc[track_id]
            if isinstance(value, pd.Series):
                value = value.iloc[0]
            genre = "" if pd.isna(value) else str(value)
        shared = sorted(set(normalized_artist_tokens(artist)) & set(giantsteps_artists))
        if shared:
            overlap_tracks.append({"track_id": track_id, "artist": artist, "tokens": shared})
        padded = f"{track_id:06d}"
        records.append(
            {
                "corpus": "fmak-v2",
                "role": "training",
                "id": padded,
                "audio_path": f"{args.audio_root}\\{padded[:3]}\\{padded}.mp3",
                "truth_index": index,
                "truth_label": CANONICAL_LABELS[index],
                "confidence": None,
                "artist": artist,
                "title": title,
                "genre": genre,
                "recording_md5": "",
                "license_title": license_title,
                "license_class": license_class(license_title, license_url),
            }
        )

    label_counts = {label: 0 for label in CANONICAL_LABELS}
    license_counts: dict[str, int] = {}
    genre_counts: dict[str, int] = {}
    for record in records:
        label_counts[record["truth_label"]] += 1
        license_counts[record["license_class"]] = license_counts.get(record["license_class"], 0) + 1
        if record["genre"]:
            genre_counts[record["genre"]] = genre_counts.get(record["genre"], 0) + 1

    report = {
        "schema_version": 1,
        "experiment": "fmak-corpus-audit",
        "inputs": {
            "fmak_csv_sha256": sha256(args.fmak_csv),
            "fma_tracks_sha256": sha256(args.fma_tracks),
            "fma_raw_sha256": sha256(args.fma_raw),
            "mtg_manifest_sha256": sha256(args.mtg_manifest),
        },
        "provenance": {
            "annotations": "FMAKv2, 10.5281/zenodo.12759100 (CC BY 4.0); "
            "218 labels corrected from FMAK v1, 10.5281/zenodo.10719860",
            "metadata": "FMA dataset metadata (mdeff/fma), per-track artist licenses",
            "script_sha256": sha256(Path(__file__)),
        },
        "fmak_labels": len(labels),
        "records_with_metadata": len(records),
        "missing_metadata": missing_metadata,
        "unmapped_labels": unmapped_labels,
        "label_counts": label_counts,
        "license_counts": license_counts,
        "genre_counts": dict(sorted(genre_counts.items(), key=lambda item: -item[1])),
        "giantsteps_artist_token_overlap_tracks": len(overlap_tracks),
        "giantsteps_artist_token_overlap": overlap_tracks[:200],
        "warning": "Audit only: no audio downloaded, no recording_md5 dedup, no fold assignment. "
        "Noncommercial-licensed records must be excluded from any commercial training path.",
    }
    atomic_json(args.report, report)
    print(
        f"labels={len(labels)} records={len(records)} missing={len(missing_metadata)} "
        f"unmapped={len(unmapped_labels)} overlap={len(overlap_tracks)} "
        f"license={license_counts}"
    )

    if args.output is not None:
        manifest = {
            "schema_version": 1,
            "canonical_labels": CANONICAL_LABELS,
            "training_protocol": "FMAKv2 expert key/mode annotations (CC BY 4.0) joined with "
            "per-track FMA artist licenses; audio downloaded separately; recording-family "
            "dedup and artist/recording-disjoint folds are computed at training time",
            "records": records,
        }
        atomic_json(args.output, manifest)
        print(f"wrote={args.output} records={len(records)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
