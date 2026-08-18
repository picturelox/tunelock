# Ground-truth corpora

TuneLock's Proof layer scores key/BPM detection against labelled reference
corpora. Two sources are supported; both are **kept out of git** because they
are either personal data or large downloaded media.

## 1. MIK personal library (`MIKCompleteLibrary.csv`)

A Mixed In Key CSV export of the user's DJ library.

- **Location (local only):** `ground-truth/MIKCompleteLibrary.csv`
- **Gitignored:** yes — contains absolute file paths and personal metadata.
- **Rows:** ~20,221  |  **Locally present audio:** ~19,563 (96.7%)
- **Columns:** `Title, Artist, Key, Tempo, Genre, Album, Grouping, Date Added,
  Location, Comment, Year, Overall Volume, Energy, CuePoints, ClippedPeaks`
- **Key column:** Camelot codes (`8A`, `12B`, …) plus `All` for MIK's
  atonal/no-stable-key verdict.
- **Tempo column:** BPM as a decimal string.

To run a benchmark against it:

```powershell
cd src-tauri
cargo run --release --bin tunelock-bench -- --corpus ..\ground-truth\MIKCompleteLibrary.csv --limit 500 --out ..\ground-truth\report.json
```

If you fork or clone this repo on a new machine, drop your own MIK export at
that path (or pass `--corpus <any-path>`) — the parser is format-driven, not
path-driven.

## 2. GiantSteps key dataset (`giantsteps-key/`)

The public GiantSteps + GiantStepsMTG key-annotation corpus from JKU:

- **Annotations:** committed (small `.key` / `.genre` text files).
- **Audio:** NOT committed — download separately with
  `download-giantsteps.ps1`:
  ```powershell
  pwsh ground-truth\download-giantsteps.ps1 ground-truth\giantsteps-key
  ```
- **Coverage:** 604 Beatport previews, ~76% minor keys.
- **License / provenance:** see `giantsteps-key/README` (Krumhansler et al. /
  Bogdanov et al.). Academic use; cite the original paper if you publish
  numbers derived from it.

To run a benchmark against it (after downloading audio):

```powershell
cd src-tauri
cargo run --release --bin tunelock-bench -- --giantsteps ..\ground-truth\giantsteps-key --out ..\ground-truth\gs-report.json
```

## 3. OUIE 7 reference set (`OUIE 7.csv`)

A small (~69-row) early reference set. Committed because it is tiny and
contains no personal paths. Useful for smoke-testing the parser.

## Reproducibility notes

- The bench is deterministic for a given corpus + TuneLock revision.
- `--limit N` performs a **stratified sample** across genres, not a head-cut,
  so a 500-track run covers the collection's diversity.
- Missing audio is reported as `missing_file`, never counted as a key error.
- MIK's `All` rows are classified `atonal` and excluded from key accuracy.
