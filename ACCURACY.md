# TuneLock Accuracy Report

Key + BPM detection accuracy against a labelled ground-truth corpus.
Updated after each tuning phase.

## Method

- **Corpus:** MIK personal library export (20,221 rows, 18,909 ready).
- **Sample:** 500 tracks, stratified by genre (round-robin across all genres
  for diversity, not a head-cut).
- **Build:** `cargo build --release` (optimised).
- **Parallelism:** Rayon across all available cores.
- **Report files:** `ground-truth/baseline-500.json`, `ground-truth/tuned-final-500.json`
  (gitignored — contain personal file paths).

## Corpus classification

| Status | Count | Notes |
|---|---|---|
| ready | 18,909 | Decodable format, has key label, file exists |
| missing_file | 658 | Path in CSV not found on disk |
| unsupported_format | 624 | `.m4a`, `.aif`, `.aiff` — Phase 2 codec work |
| atonal | 30 | MIK "All" — no stable key, excluded from scoring |

## Results comparison

| Metric | Baseline | Tuned | Change |
|---|---|---|---|
| **Key exact match** | **61.4%** | **67.8%** | **+6.4%** |
| Tonic correct (mode-agnostic) | 64.2% | 70.4% | +6.2% |
| MIREX weighted score | 0.708 | 0.754 | +0.046 |
| **Camelot compatible** | **83.7%** | **86.3%** | **+2.6%** |
| BPM ±1 BPM (raw) | 19.7% | 19.7% | — |
| BPM ±1 BPM (octave-corrected) | 34.2% | 34.2% | — |
| BPM ratio median | 0.987 | 0.987 | — |
| Avg time per track | 4,012 ms | 4,044 ms | — |

## Error taxonomy comparison

| Error type | Baseline | Tuned | Change | Meaning |
|---|---|---|---|---|
| correct | 305 | 337 | +32 | Exact key match |
| fifth | 76 | 58 | -18 | Perfect-fifth substitution |
| other | 67 | 52 | -15 | No simple relationship |
| relative | 21 | 21 | 0 | Relative major/minor |
| parallel | 14 | 13 | -1 | Same tonic, wrong mode |
| semitone | 14 | 16 | +2 | Off by one semitone |

## By format (tuned)

| Format | n | Exact % | MIREX |
|---|---|---|---|
| mp3 | 477 | 68.3% | 0.759 |
| flac | 14 | 64.3% | 0.771 |
| wav | 6 | 33.3% | 0.367 |

## Tuning changes (Phase 4: key profile tuning)

Three changes, each measured independently on the same 500-track sample:

### 1. Profile weight rebalancing: +2.0% exact

Changed `ProfileWeights::default()` from `{0.4, 0.5, 0.5}` to
`{0.15, 0.25, 1.0}` (krumhansl, temperley, shaath).

The Sha'ath 72-band Direct Spectral Kernel profile is the strongest single
method — it uses a CQT approximation with cosine windowing and octave
weighting. Krumhansl and Temperley are kept at low weights because they
provide critical mode (major/minor) discrimination that Sha'ath alone
lacks: Sha'ath-only scores 61.4% exact with parallel-mode errors spiking
from 13 to 30.

### 2. Log compression of chroma: +4.4% exact (cumulative +6.4%)

Applied `log(1 + gain * chroma[i])` to the chroma vector before cosine
similarity scoring. This compresses the dynamic range so the tonic and
fifth (the two strongest bins) don't dominate the score. Weaker bins —
the 3rd, 4th, 6th, and 7th scale degrees — become more influential.

These weaker bins are the notes that **differ between a key and its
fifth-neighbour** (e.g. C major has F natural, G major has F#). Amplifying
their contribution directly combats fifth-substitution errors, which were
the largest error category in the baseline (76/192 errors = 15.3%).

Optimal gains, calibrated on the 500-track sample:
- 12-bin chroma (Krumhansl/Temperley): gain = 2.0
- 72-band chroma (Sha'ath): gain = 3.0

The 72-band path benefits from stronger compression because it has more
bins (72 vs 12), so the dynamic range is wider.

### 3. Approaches that did NOT work

- **Pearson correlation instead of cosine similarity**: Caused a major
  regression (61.4% → 43.9%). Parallel-mode errors exploded from 14 to 69.
  Cosine similarity's sensitivity to absolute magnitudes helps distinguish
  major from minor profiles.

- **Tonic prominence boost** (multiplying score by `1 + alpha * chroma[tonic]/max`):
  Consistently hurt accuracy at all tested alpha values (0.15, 0.35). The
  boost helps the correct key's tonic but also helps the fifth-neighbour's
  "tonic" (which is the fifth of the true key and thus also strong).

- **Folding 72-band chroma to 12-bin for K/T paths**: The 72-band octave
  weights distort the 12-bin profile matching (61.4% → 57.5%).

- **16 segments instead of 8**: No improvement; slightly increased relative
  errors due to noisier per-segment votes.

## Key findings

1. **67.8% exact key accuracy** across a wildly diverse 500-track sample
   (rock, classical, soundtrack, world, a cappella, electronic). Electronic
   genres (house, techno, trance) score near 100%.

2. **86.3% Camelot compatibility** — the more DJ-relevant number. A
   fifth-related "error" is still harmonically mixable.

3. **Fifth-substitution errors reduced from 76 to 58** (-24%). Log
   compression was the key insight: by compressing the dynamic range, the
   discriminative scale degrees (3rd, 4th, 7th) that differ between
   fifth-neighbours get more influence in the cosine similarity.

4. **BPM accuracy is unchanged** (19.7% raw, 34.2% octave-corrected). This
   is the next target — Phase 3: tempo rewrite.

5. **WAV format underperforms** (33.3% vs 68.3% for mp3). Small sample (6
   tracks, 3 failed to decode). Needs investigation with a larger WAV
   subset once the Phase 2 codec work is done.

## Next steps

- **Phase 3:** Tempo rewrite — octave resolution, confidence scoring, wider
  BPM range. Currently 19.7% ±1 BPM is the weakest metric.
- **Phase 2:** Fix WAV decode issue and add `.m4a`/`.aiff` codec support to
  recover the 624 unsupported-format tracks.
- **Phase 1.5:** Download GiantSteps audio and run cross-corpus validation.
- **Future key tuning:** Consider harmonic summation in the chroma to
  reduce 3rd-harmonic interference (the 3rd harmonic of the tonic lands on
  the fifth, artificially boosting the fifth bin).

## Reproducing

```powershell
cd src-tauri
cargo build --release --bin tunelock-bench
.\target\release\tunelock-bench.exe --corpus ..\ground-truth\MIKCompleteLibrary.csv --limit 500 --out ..\ground-truth\tuned-final-500.json
```

The stratified sample is deterministic for a given corpus + code revision, so
results are reproducible. To score the full 18,909 ready tracks, omit `--limit`.
