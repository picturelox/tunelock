#!/usr/bin/env python3
"""Evaluate raw S-KEY baseline (no harmonic head) on FMAK cached features."""

from __future__ import annotations

import json
from pathlib import Path

import numpy as np

from extract_skey_harmonic_features import feature_path

SKEY_TO_CANONICAL = (9, 10, 11, 0, 1, 2, 3, 4, 5, 6, 7, 8, 23, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22)


def main() -> int:
    import argparse
    parser = argparse.ArgumentParser()
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--feature-cache", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--role", default="development")
    args = parser.parse_args()

    manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
    records = [r for r in manifest["records"] if r["role"] == args.role]
    print(f"Records: {len(records)}", flush=True)

    all_results = []
    exact = 0
    for record in records:
        path = feature_path(args.feature_cache, record)
        if not path.exists():
            continue
        data = np.load(path, allow_pickle=False)
        raw_posterior = data["posterior"]
        canonical_posterior = np.zeros(24, dtype=np.float32)
        for s_idx, c_idx in enumerate(SKEY_TO_CANONICAL):
            canonical_posterior[c_idx] = raw_posterior[s_idx]
        pred_index = int(np.argmax(canonical_posterior))
        truth_index = record["truth_index"]
        if pred_index == truth_index:
            exact += 1
        all_results.append({
            "id": record["id"],
            "artist": record.get("artist", ""),
            "genre": record.get("genre", ""),
            "truth_index": truth_index,
            "truth_label": record["truth_label"],
            "pred_index": pred_index,
            "pred_label": manifest["canonical_labels"][pred_index],
            "posterior": canonical_posterior.tolist(),
        })

    total = len(all_results)
    print(f"\n=== Raw S-KEY baseline on FMAK ===", flush=True)
    print(f"Scored: {total}", flush=True)
    print(f"Exact: {exact}/{total} ({100*exact/total:.1f}%)", flush=True)

    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("w", encoding="utf-8") as f:
        for entry in all_results:
            f.write(json.dumps(entry, separators=(",", ":")) + "\n")
    print(f"Wrote: {args.output}", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
