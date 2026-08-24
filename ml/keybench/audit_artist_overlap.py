#!/usr/bin/env python3
"""Audit artist and recording-family overlap between GiantSteps corpora."""

from __future__ import annotations

import argparse
import csv
import json
from pathlib import Path
import re
import unicodedata
import xml.etree.ElementTree as ET
import zipfile

from train_myna_head import normalized_artist_tokens


XML_NS = {"x": "http://schemas.openxmlformats.org/spreadsheetml/2006/main"}
AUDIO_ID = re.compile(r"/([^/]+)\.mp3(?:\?.*)?$", flags=re.IGNORECASE)


def normalized_text(value: str) -> str:
    value = unicodedata.normalize("NFKD", value).encode("ascii", "ignore").decode("ascii")
    return "".join(character for character in value.casefold() if character.isalnum())


def column_index(reference: str) -> int:
    letters = "".join(character for character in reference if character.isalpha())
    value = 0
    for letter in letters.upper():
        value = value * 26 + ord(letter) - ord("A") + 1
    return value - 1


def read_xlsx_rows(path: Path) -> list[dict[str, str]]:
    with zipfile.ZipFile(path) as archive:
        shared_root = ET.fromstring(archive.read("xl/sharedStrings.xml"))
        shared = [
            "".join(text.text or "" for text in item.findall(".//x:t", XML_NS))
            for item in shared_root.findall("x:si", XML_NS)
        ]
        sheet = ET.fromstring(archive.read("xl/worksheets/sheet1.xml"))

    values: list[list[str]] = []
    for row in sheet.findall(".//x:sheetData/x:row", XML_NS):
        cells: dict[int, str] = {}
        for cell in row.findall("x:c", XML_NS):
            reference = cell.attrib.get("r", "")
            value_node = cell.find("x:v", XML_NS)
            value = "" if value_node is None else value_node.text or ""
            if cell.attrib.get("t") == "s" and value:
                value = shared[int(value)]
            elif cell.attrib.get("t") == "inlineStr":
                value = "".join(
                    text.text or "" for text in cell.findall(".//x:t", XML_NS)
                )
            cells[column_index(reference)] = value.strip()
        width = max(cells, default=-1) + 1
        values.append([cells.get(index, "") for index in range(width)])

    if not values:
        return []
    headers = values[0]
    return [
        {header: row[index] if index < len(row) else "" for index, header in enumerate(headers)}
        for row in values[1:]
    ]


def main() -> int:
    parser = argparse.ArgumentParser(description="Audit GiantSteps corpus overlap")
    parser.add_argument("--mtg-metadata", required=True, type=Path)
    parser.add_argument("--giantsteps-sources", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()

    with args.mtg_metadata.open(encoding="utf-8", newline="") as handle:
        mtg = list(csv.DictReader(handle, delimiter="\t"))
    development = read_xlsx_rows(args.giantsteps_sources)

    mtg_tokens = {
        token
        for row in mtg
        for token in normalized_artist_tokens(row.get("ARTIST", ""))
    }
    development_tokens = {
        token
        for row in development
        for token in normalized_artist_tokens(row.get("ARTIST", ""))
    }
    shared_tokens = sorted(mtg_tokens & development_tokens)

    development_shared_artist_ids = []
    for row in development:
        if set(normalized_artist_tokens(row.get("ARTIST", ""))) & set(shared_tokens):
            match = AUDIO_ID.search(row.get("AUDIO LINK", ""))
            if match:
                development_shared_artist_ids.append(match.group(1))

    mtg_families = {
        (normalized_text(row.get("ARTIST", "")), normalized_text(row.get("SONG TITLE", "")))
        for row in mtg
        if row.get("ARTIST") and row.get("SONG TITLE")
    }
    development_families = {
        (normalized_text(row.get("ARTIST", "")), normalized_text(row.get("TRACK", "")))
        for row in development
        if row.get("ARTIST") and row.get("TRACK")
    }
    shared_families = sorted(mtg_families & development_families)

    development_ids = []
    for row in development:
        match = AUDIO_ID.search(row.get("AUDIO LINK", ""))
        if match:
            development_ids.append(match.group(1))
    mtg_ids = {f"{row.get('ID', '').strip()}.LOFI" for row in mtg if row.get("ID")}
    shared_ids = sorted(mtg_ids & set(development_ids))

    report = {
        "schema_version": 1,
        "mtg_rows": len(mtg),
        "development_rows": len(development),
        "development_audio_ids_parsed": len(development_ids),
        "mtg_artist_tokens": len(mtg_tokens),
        "development_artist_tokens": len(development_tokens),
        "shared_artist_tokens": len(shared_tokens),
        "shared_artist_token_values": shared_tokens,
        "development_tracks_with_shared_artist": len(development_shared_artist_ids),
        "development_tracks_with_shared_artist_ids": sorted(development_shared_artist_ids),
        "exact_normalized_artist_title_families": len(shared_families),
        "exact_normalized_artist_title_values": shared_families,
        "shared_track_ids": len(shared_ids),
        "shared_track_id_values": shared_ids,
        "scope_note": "String audit only; acoustic fingerprinting is reported separately.",
    }
    if args.output.exists():
        raise FileExistsError(f"Refusing to overwrite audit artifact: {args.output}")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report, sort_keys=True), flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
