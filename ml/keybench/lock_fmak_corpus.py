#!/usr/bin/env python3
"""Lock the FMAK v1 experimental corpus before any model tuning.

This script freezes the corpus state:
  1. Quarantine recordings with conflicting labels (same MD5, different keys).
  2. Collapse exact-MD5 duplicates into recording families.
  3. Run a lightweight acoustic near-duplicate pass using librosa spectral
     signatures to catch re-encodings that MD5 misses.
  4. Build artist/recording-family-disjoint splits: ~10% sealed holdout +
     5 development folds.
  5. Separate research_training (all FMAK) from product_training
     (rights-cleared subset only).
  6. Write the full locked manifest to ml/data/ (gitignored) and a compact
     freeze file with hashes and fold summary to a tracked location.

Nothing in this script reads model scores. The corpus is frozen before any
FMAK model evaluation begins.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any

import numpy as np


CANONICAL_LABELS = [
    "C major", "C# major", "D major", "D# major", "E major", "F major",
    "F# major", "G major", "G# major", "A major", "A# major", "B major",
    "C minor", "C# minor", "D minor", "D# minor", "E minor", "F minor",
    "F# minor", "G minor", "G# minor", "A minor", "A# minor", "B minor",
]

PRODUCT_LICENSE_CLASSES = {"cc_by", "cc_by_sa", "public_domain"}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Lock FMAK v1 experimental corpus")
    parser.add_argument("--manifest", required=True, type=Path,
                        help="FMAK manifest v2 with recording_md5 backfilled")
    parser.add_argument("--verification-report", required=True, type=Path,
                        help="Audio verification report from verify_fmak_audio.py")
    parser.add_argument("--output-manifest", required=True, type=Path,
                        help="Output: full locked manifest (gitignored location)")
    parser.add_argument("--freeze-file", required=True, type=Path,
                        help="Output: compact freeze file (tracked location)")
    parser.add_argument("--report", required=True, type=Path,
                        help="Output: lock audit report")
    parser.add_argument("--n-folds", type=int, default=5)
    parser.add_argument("--holdout-fraction", type=float, default=0.10)
    parser.add_argument("--near-dup-threshold", type=float, default=0.995,
                        help="Cosine similarity threshold for near-duplicate flagging")
    parser.add_argument("--fingerprint-seconds", type=float, default=20.0,
                        help="Seconds of audio to decode for fingerprinting")
    parser.add_argument("--fingerprint-sr", type=int, default=8000,
                        help="Sample rate for fingerprint decoding")
    parser.add_argument("--skip-fingerprint", action="store_true",
                        help="Skip acoustic fingerprinting (MD5-only dedup)")
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


def normalized_artist_tokens(artist: str) -> frozenset[str]:
    """Normalized artist tokens for disjoint-fold grouping."""
    tokens = []
    for sep in ("&", ",", " x ", " feat.", " ft.", " feat ", " ft "):
        artist = artist.replace(sep, "|")
    for part in artist.split("|"):
        token = part.strip().casefold()
        if token and token not in ("the", "and", "a"):
            tokens.append(token)
    return frozenset(tokens) if tokens else frozenset({"__unknown__"})


def compute_spectral_fingerprint(audio_path: Path, seconds: float, sr: int) -> np.ndarray | None:
    """Compute a compact spectral signature for near-duplicate detection."""
    import librosa
    try:
        y, _ = librosa.load(str(audio_path), sr=sr, duration=seconds, mono=True)
    except Exception:
        return None
    if len(y) < sr * 0.5:
        return None
    # Trim silence
    y_trimmed, _ = librosa.effects.trim(y, top_db=40)
    if len(y_trimmed) < sr * 0.5:
        y_trimmed = y
    # Compute mel-spectrum mean (32 bins)
    mel = librosa.feature.melspectrogram(
        y=y_trimmed, sr=sr, n_fft=1024, hop_length=512,
        n_mels=32, fmin=0, fmax=sr // 2,
    )
    mel_mean = np.log1p(mel.mean(axis=1))
    # Normalize
    norm = np.linalg.norm(mel_mean)
    if norm < 1e-9:
        return None
    return mel_mean / norm


def find_near_duplicates(
    records: list[dict[str, Any]],
    threshold: float,
    seconds: float,
    sr: int,
) -> list[dict[str, Any]]:
    """Find near-duplicate recordings using spectral fingerprinting."""
    print(f"Computing spectral fingerprints for {len(records)} files...", flush=True)
    fingerprints: dict[str, np.ndarray] = {}
    for i, record in enumerate(records, 1):
        fp = compute_spectral_fingerprint(Path(record["audio_path"]), seconds, sr)
        if fp is not None:
            fingerprints[record["id"]] = fp
        if i % 500 == 0:
            print(f"  fingerprinted {i}/{len(records)}", flush=True)

    print(f"Comparing fingerprints ({len(fingerprints)} valid)...", flush=True)
    # Bucket by quantized first 8 bins for efficient candidate finding
    buckets: dict[tuple, list[str]] = defaultdict(list)
    for track_id, fp in fingerprints.items():
        key = tuple(np.digitize(fp[:8], bins=[-0.5, 0, 0.5]))
        buckets[key].append(track_id)

    near_dups: list[dict[str, Any]] = []
    ids = list(fingerprints.keys())
    compared = set()
    for i, id_a in enumerate(ids):
        fp_a = fingerprints[id_a]
        # Check same bucket and adjacent buckets
        key = tuple(np.digitize(fp_a[:8], bins=[-0.5, 0, 0.5]))
        candidates = set()
        for offset in [(0,)*8, (1,)+(0,)*7, (-1,)+(0,)*7]:
            adj_key = tuple(k + o for k, o in zip(key, offset))
            candidates.update(buckets.get(adj_key, []))
        for id_b in candidates:
            if id_a >= id_b:
                continue
            pair = (id_a, id_b)
            if pair in compared:
                continue
            compared.add(pair)
            sim = float(np.dot(fp_a, fingerprints[id_b]))
            if sim >= threshold:
                near_dups.append({
                    "track_a": id_a,
                    "track_b": id_b,
                    "cosine_similarity": round(sim, 6),
                })
    print(f"Found {len(near_dups)} near-duplicate pairs (threshold={threshold})", flush=True)
    return near_dups


def build_recording_families(
    records: list[dict[str, Any]],
    exact_dup_groups: list[dict[str, Any]],
    near_dups: list[dict[str, Any]],
    quarantined_ids: set[str],
) -> dict[str, str]:
    """Assign each track to a recording family ID.

    Family ID is the lexicographically smallest track ID in the family.
    Exact-MD5 duplicates and near-duplicates are merged into the same family.
    """
    # Union-Find
    parent: dict[str, str] = {}

    def find(x: str) -> str:
        if x not in parent:
            parent[x] = x
        if parent[x] != x:
            parent[x] = find(parent[x])
        return parent[x]

    def union(a: str, b: str) -> None:
        ra, rb = find(a), find(b)
        if ra != rb:
            # Keep smaller ID as root for deterministic family IDs
            root, child = (ra, rb) if ra < rb else (rb, ra)
            parent[child] = root

    # Initialize all records
    for record in records:
        find(record["id"])

    # Merge exact duplicates
    for group in exact_dup_groups:
        ids = group["track_ids"]
        for i in range(1, len(ids)):
            union(ids[0], ids[i])

    # Merge near duplicates
    for pair in near_dups:
        union(pair["track_a"], pair["track_b"])

    # Build family mapping
    family_map: dict[str, str] = {}
    for record in records:
        family_map[record["id"]] = find(record["id"])

    return family_map


def assign_folds(
    records: list[dict[str, Any]],
    family_map: dict[str, str],
    quarantined_ids: set[str],
    n_folds: int,
    holdout_fraction: float,
) -> dict[str, str]:
    """Assign each non-quarantined record to 'holdout' or 'fold_N'.

    Uses greedy artist/recording-disjoint assignment:
    - Sort families by size descending
    - Track which artists appear in which folds
    - Assign each family to the fold with fewest tracks that doesn't contain
      any of the family's artists
    - First assign holdout, then development folds
    """
    # Build family -> records mapping
    family_records: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for record in records:
        if record["id"] in quarantined_ids:
            continue
        family_records[family_map[record["id"]]].append(record)

    # Sort families by size descending
    sorted_families = sorted(family_records.items(), key=lambda x: -len(x[1]))

    # Track artists per fold
    fold_artists: dict[str, set[frozenset[str]]] = defaultdict(set)
    fold_counts: dict[str, int] = defaultdict(int)

    # Target holdout size
    total_tracks = sum(len(recs) for _, recs in sorted_families)
    holdout_target = int(total_tracks * holdout_fraction)

    # Assign folds
    assignment: dict[str, str] = {}
    fold_names = [f"fold_{i+1}" for i in range(n_folds)]

    for family_id, family_recs in sorted_families:
        # Get all artist tokens for this family
        family_artists = set()
        for rec in family_recs:
            family_artists.update(normalized_artist_tokens(rec.get("artist", "")))

        # Try holdout first if it's still under target
        if fold_counts["holdout"] < holdout_target:
            if not family_artists & fold_artists.get("holdout", set()):
                assignment[family_id] = "holdout"
                fold_artists["holdout"].update(family_artists)
                fold_counts["holdout"] += len(family_recs)
                for rec in family_recs:
                    assignment[rec["id"]] = "holdout"
                continue

        # Try development folds — pick the one with fewest tracks
        # that doesn't already have any of these artists
        candidates = []
        for fold_name in fold_names:
            if not family_artists & fold_artists.get(fold_name, set()):
                candidates.append((fold_counts[fold_name], fold_name))

        if candidates:
            candidates.sort()
            _, fold_name = candidates[0]
        else:
            # All folds have artist overlap — assign to the fold with
            # fewest tracks (least damage)
            fold_name = min(fold_names, key=lambda f: fold_counts[f])

        fold_artists[fold_name].update(family_artists)
        fold_counts[fold_name] += len(family_recs)
        assignment[family_id] = fold_name
        for rec in family_recs:
            assignment[rec["id"]] = fold_name

    return assignment


def main() -> int:
    args = parse_args()
    for artifact in (args.output_manifest, args.freeze_file, args.report):
        if artifact.exists():
            raise FileExistsError(f"Refusing to overwrite: {artifact}")

    manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
    records = manifest["records"]
    verification = json.loads(args.verification_report.read_text(encoding="utf-8"))

    print(f"Loaded {len(records)} records from manifest", flush=True)

    # Step 1: Quarantine conflicting recordings
    quarantined_ids: set[str] = set()
    for conflict in verification.get("label_conflicts", []):
        for track_id in conflict["track_ids"]:
            quarantined_ids.add(track_id)
    print(f"Quarantined {len(quarantined_ids)} tracks with conflicting labels: "
          f"{sorted(quarantined_ids)}", flush=True)

    # Step 2: Build recording families from exact duplicates
    exact_dup_groups = [
        g for g in verification.get("duplicates", [])
        if len(g.get("track_ids", [])) > 1
    ]

    # Step 3: Near-duplicate detection
    non_quarantined = [r for r in records if r["id"] not in quarantined_ids]
    if args.skip_fingerprint:
        near_dups: list[dict[str, Any]] = []
        print("Skipping acoustic fingerprinting (--skip-fingerprint)", flush=True)
    else:
        near_dups = find_near_duplicates(
            non_quarantined,
            args.near_dup_threshold,
            args.fingerprint_seconds,
            args.fingerprint_sr,
        )

    # Step 4: Build recording families
    family_map = build_recording_families(
        records, exact_dup_groups, near_dups, quarantined_ids
    )
    family_sizes = Counter(family_map.values())
    print(f"Recording families: {len(family_sizes)} "
          f"(singletons: {sum(1 for s in family_sizes.values() if s == 1)}, "
          f"multi: {sum(1 for s in family_sizes.values() if s > 1)})", flush=True)

    # Step 5: Assign folds
    fold_assignment = assign_folds(
        records, family_map, quarantined_ids,
        args.n_folds, args.holdout_fraction,
    )

    fold_counts = Counter(fold_assignment.values())
    print(f"Fold assignment: {dict(fold_counts)}", flush=True)

    # Step 6: Build locked records
    locked_records: list[dict[str, Any]] = []
    for record in records:
        locked = dict(record)
        locked["recording_family"] = family_map[record["id"]]
        locked["fold"] = fold_assignment.get(record["id"], "quarantined")
        locked["is_quarantined"] = record["id"] in quarantined_ids
        locked["is_product_eligible"] = (
            record.get("license_class", "unknown") in PRODUCT_LICENSE_CLASSES
            and record["id"] not in quarantined_ids
        )
        locked_records.append(locked)

    # Step 7: Build summary statistics
    non_quarantined_records = [r for r in locked_records if not r["is_quarantined"]]
    research_records = non_quarantined_records
    product_records = [r for r in non_quarantined_records if r["is_product_eligible"]]

    def fold_breakdown(recs: list[dict[str, Any]]) -> dict[str, int]:
        return dict(Counter(r["fold"] for r in recs).most_common())

    def label_distribution(recs: list[dict[str, Any]]) -> dict[str, int]:
        return dict(Counter(r["truth_label"] for r in recs).most_common())

    def license_distribution(recs: list[dict[str, Any]]) -> dict[str, int]:
        return dict(Counter(r.get("license_class", "unknown") for r in recs).most_common())

    def genre_distribution(recs: list[dict[str, Any]]) -> dict[str, int]:
        return dict(Counter(r.get("genre", "") for r in recs if r.get("genre")).most_common(20))

    # Step 8: Write full locked manifest
    locked_manifest = {
        "schema_version": 2,
        "experiment": "fmak-corpus-lock-v1",
        "canonical_labels": CANONICAL_LABELS,
        "lock_timestamp": str(Path(args.output_manifest).stat().st_mtime) if args.output_manifest.exists() else None,
        "input_manifest_sha256": sha256(args.manifest),
        "verification_report_sha256": sha256(args.verification_report),
        "lock_script_sha256": sha256(Path(__file__)),
        "near_dup_threshold": args.near_dup_threshold if not args.skip_fingerprint else None,
        "near_duplicate_pairs": near_dups,
        "quarantined_ids": sorted(quarantined_ids),
        "recording_family_count": len(family_sizes),
        "fold_assignment": {
            "n_folds": args.n_folds,
            "holdout_fraction": args.holdout_fraction,
            "counts": dict(fold_counts),
        },
        "records": locked_records,
    }
    atomic_json(args.output_manifest, locked_manifest)
    print(f"Wrote locked manifest: {args.output_manifest}", flush=True)

    # Step 9: Write compact freeze file (tracked in repo)
    freeze: dict[str, Any] = {
        "schema_version": 1,
        "experiment": "fmak-corpus-lock-v1-freeze",
        "input_manifest_sha256": sha256(args.manifest),
        "verification_report_sha256": sha256(args.verification_report),
        "locked_manifest_sha256": sha256(args.output_manifest),
        "lock_script_sha256": sha256(Path(__file__)),
        "total_records": len(records),
        "quarantined_ids": sorted(quarantined_ids),
        "recording_families": len(family_sizes),
        "exact_duplicate_groups": len(exact_dup_groups),
        "near_duplicate_pairs": len(near_dups),
        "near_dup_threshold": args.near_dup_threshold if not args.skip_fingerprint else None,
        "fold_counts": dict(fold_counts),
        "research_training": {
            "total": len(research_records),
            "folds": fold_breakdown(research_records),
            "license_distribution": license_distribution(research_records),
        },
        "product_training": {
            "total": len(product_records),
            "folds": fold_breakdown(product_records),
            "license_distribution": license_distribution(product_records),
        },
        "warning": "This freeze file records the corpus state before any FMAK model "
        "evaluation. The locked manifest (hashed above) contains the full per-record "
        "fold assignments. Research training includes all non-quarantined records; "
        "product training excludes NonCommercial-licensed audio.",
    }
    atomic_json(args.freeze_file, freeze)
    print(f"Wrote freeze file: {args.freeze_file}", flush=True)

    # Step 10: Write audit report
    report: dict[str, Any] = {
        "schema_version": 1,
        "experiment": "fmak-corpus-lock-v1-audit",
        "locked_manifest_sha256": sha256(args.output_manifest),
        "freeze_file_sha256": sha256(args.freeze_file),
        "total_records": len(records),
        "quarantined": len(quarantined_ids),
        "recording_families": len(family_sizes),
        "exact_duplicate_groups": len(exact_dup_groups),
        "near_duplicate_pairs": len(near_dups),
        "fold_counts": dict(fold_counts),
        "research_label_distribution": label_distribution(research_records),
        "product_label_distribution": label_distribution(product_records),
        "research_genre_distribution": genre_distribution(research_records),
        "product_genre_distribution": genre_distribution(product_records),
        "research_license_distribution": license_distribution(research_records),
        "product_license_distribution": license_distribution(product_records),
        "near_duplicate_details": near_dups[:50],
        "warning": "Corpus is frozen. No changes to fold assignments or quarantine "
        "status are permitted after this point without a new lock version.",
    }
    atomic_json(args.report, report)
    print(f"Wrote audit report: {args.report}", flush=True)

    print(
        f"\n=== FMAK Corpus Lock v1 ===\n"
        f"Total records: {len(records)}\n"
        f"Quarantined: {len(quarantined_ids)}\n"
        f"Recording families: {len(family_sizes)}\n"
        f"Near-duplicate pairs: {len(near_dups)}\n"
        f"Folds: {dict(fold_counts)}\n"
        f"Research training: {len(research_records)}\n"
        f"Product training: {len(product_records)}\n"
        f"Locked manifest: {args.output_manifest}\n"
        f"Freeze file: {args.freeze_file}",
        flush=True,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
