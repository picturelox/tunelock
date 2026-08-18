# TuneLock Accuracy Baseline

First measured accuracy of TuneLock's key + BPM detection against a labelled
ground-truth corpus. This is the reference point for all future improvements.

## Method

- **Corpus:** MIK personal library export (20,221 rows, 18,909 ready).
- **Sample:** 500 tracks, stratified by genre (round-robin across all genres
  for diversity, not a head-cut).
- **Build:** `cargo build --release` (optimised).
- **Parallelism:** Rayon across all available cores.
- **Date:** 2025-01 baseline, commit `5d4ddde`.
- **Report file:** `ground-truth/baseline-500.json` (gitignored — contains
  personal file paths).

## Corpus classification

| Status | Count | Notes |
|---|---|---|
| ready | 18,909 | Decodable format, has key label, file exists |
| missing_file | 658 | Path in CSV not found on disk |
| unsupported_format | 624 | `.m4a`, `.aif`, `.aiff` — Phase 2 codec work |
| atonal | 30 | MIK "All" — no stable key, excluded from scoring |

## Overall results (497 scored, 3 failed)

| Metric | Value |
|---|---|
| **Key exact match** | **61.4%** |
| Tonic correct (mode-agnostic) | 64.2% |
| MIREX weighted score | 0.708 |
| **Camelot compatible** | **83.7%** |
| BPM ±1 BPM (raw) | 19.7% |
| BPM ±1 BPM (octave-corrected) | 34.2% |
| BPM ratio median | 0.987 |
| Avg time per track | 4,012 ms |

## Error taxonomy

| Error type | Count | % | Meaning |
|---|---|---|---|
| correct | 305 | 61.4% | Exact key match |
| fifth | 76 | 15.3% | Perfect-fifth substitution (harmonically close) |
| other | 67 | 13.5% | No simple relationship |
| relative | 21 | 4.2% | Relative major/minor (same notes, wrong mode) |
| parallel | 14 | 2.8% | Same tonic, wrong mode |
| semitone | 14 | 2.8% | Off by one semitone |

## By format

| Format | n | Exact % | MIREX |
|---|---|---|---|
| mp3 | 477 | 62.3% | 0.714 |
| flac | 14 | 50.0% | 0.664 |
| wav | 6 | 16.7% | 0.367 |

WAV performs anomalously poorly — small sample (6 tracks, 3 of which failed
to decode entirely due to a Symphonia probe issue). Needs investigation with
a larger WAV subset once the codec gap is addressed.

## Failures

All 3 failures were WAV files failing with `decode: Failed to probe audio
format`. This is a Symphonia codec limitation, not a key-detection error.

## Key findings

1. **61.4% exact key accuracy** is the starting point. MIK claims ~90%+ on
   electronic music; TuneLock is below that but the corpus here is extremely
   genre-diverse (rock, classical, soundtrack, world, a cappella, etc.).

2. **83.7% Camelot compatibility** is the more DJ-relevant number. A fifth-
   related "error" (15.3% of cases) is still harmonically mixable — the
   Camelot wheel treats perfect-fifth neighbours as compatible.

3. **The fifth-related error is the dominant failure mode** (76/192 errors).
   This suggests the chroma profiles sometimes lock onto the dominant rather
   than the tonic. Possible fixes:
   - Weight tonic vs dominant bins in the Krumhansl/Temperley/Sha'ath profiles.
   - Add a dominant-ambiguity flag to the diagnostic output.
   - Try the 72-band chroma with more selective bin weighting.

4. **BPM accuracy is poor** (19.7% raw, 34.2% octave-corrected). The tempo
   detector needs the Phase 3 rewrite: octave resolution, confidence scoring,
   and wider BPM range. The ratio median (0.987) shows no systematic half-
   tempo bias, but individual tracks are likely wrong in both directions.

5. **Genre diversity hurts accuracy.** The stratified sample includes rock,
   classical, soundtrack, world, and a cappella — genres where chroma-based
   key detection is known to struggle. Electronic music (house, techno,
   trance, tech house) shows near-100% accuracy in the per-genre breakdown.

## Next steps

- **Phase 1.5:** Download GiantSteps audio and run the second benchmark for
  cross-corpus validation.
- **Phase 2:** Fix the WAV decode issue and add `.m4a`/`.aiff` codec support
  to recover the 624 unsupported-format tracks.
- **Phase 3:** Rewrite the tempo detector with octave resolution.
- **Phase 4:** Investigate the dominant-vs-tonic ambiguity in the key profiles.
- **Re-run this baseline** after each phase to track improvement.

## Reproducing

```powershell
cd src-tauri
cargo build --release --bin tunelock-bench
.\target\release\tunelock-bench.exe --corpus ..\ground-truth\MIKCompleteLibrary.csv --limit 500 --out ..\ground-truth\baseline-500.json
```

The stratified sample is deterministic for a given corpus + code revision, so
results are reproducible. To score the full 18,909 ready tracks, omit `--limit`.
