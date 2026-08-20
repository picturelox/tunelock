# TuneLock Accuracy Report

Key + BPM detection accuracy against labelled corpora.

**Important framing:**
- The MIK corpus is **Mixed In Key's opinion**, not ground truth. Results
  against it are "MIK agreement," not accuracy.
- The GiantSteps-key corpus (604 Beatport previews, independently
  annotated) is the primary **accuracy benchmark**.
- The 500-track MIK sample is not meaningfully stratified — see
  "Stratification issue" below.
- The CNN experiment was **invalid** (implementation bugs), not fairly
  evaluated. See "CNN experiment status" below.

Updated after each measured change.

## Method

- **MIK corpus:** Personal library export (20,221 rows, 18,909 ready).
  Labels are Mixed In Key's predictions, not human annotations. Used as
  a secondary agreement metric, never as a calibration target.
- **GiantSteps corpus:** 604 Beatport previews (~2 min each), independently
  annotated with standard key names. This is the primary accuracy
  benchmark. All 604 tracks scored (0 failures).
- **MIK sample:** 500 tracks, selected by round-robin across raw genre
  strings. This is **not** meaningful stratification — see below.
- **Build:** `cargo build --release` (optimised).
- **Parallelism:** Rayon across all available cores.

## Stratification issue (FIXED)

The prior 500-track MIK sample used `row.genre.to_lowercase()` as the
bucket key for round-robin selection (`corpus.rs`). The corpus contained
439 distinct raw genre strings, including typos, website names, combined
tags, emojis, and arbitrary metadata. 378 buckets contained one track;
61 contained two. The resulting sample was neither representative of the
library nor a clean macro-average across meaningful genres.

**Fix (Step 2):** Genre strings are now normalized to 16 meaningful
categories via `normalize_genre()`. Sampling uses a seeded xorshift64 PRNG
for reproducible shuffling within each normalized genre bucket, then
round-robin across buckets. The frozen manifest is at
`ground-truth/manifest-mik-500-seed42.json` (gitignored).

The new sample produces 61.6% MIK agreement vs the old 68.8% — the
difference is sample composition, not engine regression. The old sample
was biased toward easier tracks.

## Results — current release binary

### GiantSteps-key (primary accuracy benchmark)

| Metric | Value |
|---|---|
| **Scored** | 604 (0 failed) |
| **Key exact match** | **62.4%** (377/604) |
| Tonic correct (mode-agnostic) | 66.2% |
| MIREX weighted score | 0.704 |
| **Camelot compatible** | **82.3%** |
| Avg time per track | 5,519 ms |

**Historical context** (from the GiantSteps ISMIR 2015 paper):

| System | Exact match on GiantSteps |
|---|---|
| Mixed In Key (commercial) | 67.22% |
| Rekordbox (commercial) | 71.85% |
| **TuneLock (current)** | **62.4%** |

TuneLock is respectable but behind both historical commercial results on
this independently annotated dataset. The dataset is intentionally biased
toward difficult Beatport mistakes, though its authors manually checked
a random 15% of annotations and found them correct.

### MIK 500 sample (MIK agreement — not ground truth)

**Note:** The sample now uses normalized genre stratification (16 categories)
with seeded shuffling (seed=42). The prior sample used raw genre strings
(439 distinct values) and was not meaningfully stratified. The new sample
is more representative and produces a lower MIK agreement score (61.6% vs
the prior 68.8%) because the old sample was biased toward easier tracks.

| Metric | Value |
|---|---|
| **Scored** | 490 (10 failed) |
| **Key exact match (MIK agreement)** | **61.6%** (302/490) |
| Tonic correct (mode-agnostic) | 65.3% |
| MIREX weighted score | 0.706 |
| **Camelot compatible** | **84.3%** |
| **BPM ±1 BPM (raw)** | 53.3% |
| **BPM ±1 BPM (octave-corrected)** | 54.5% |
| BPM ratio median | 1.001 |

The 10 failed tracks are WAV files with non-standard fmt chunks. The
manifest is frozen at `ground-truth/manifest-mik-500-seed42.json`
(gitignored — contains personal paths).

**By normalized genre:**

| Genre | n | Exact % | MIREX |
|---|---|---|---|
| techno | 26 | 76.9% | 0.769 |
| ambient | 32 | 71.9% | 0.772 |
| electronic | 32 | 68.8% | 0.747 |
| other | 32 | 68.8% | 0.772 |
| jazz | 32 | 65.6% | 0.703 |
| reggae-latin | 31 | 64.5% | 0.745 |
| rock | 31 | 64.5% | 0.732 |
| world | 31 | 64.5% | 0.732 |
| classical | 32 | 59.4% | 0.697 |
| bass | 23 | 60.9% | 0.674 |
| r&b | 31 | 58.1% | 0.684 |
| trance | 31 | 54.8% | 0.665 |
| hip-hop | 32 | 53.1% | 0.672 |
| unknown | 31 | 61.3% | 0.726 |
| house | 31 | 48.4% | 0.565 |
| pop | 32 | 46.9% | 0.647 |

## Error taxonomy

### GiantSteps-key (604 tracks)

| Error type | Count | % | Meaning |
|---|---|---|---|
| correct | 377 | 62.4% | Exact key match |
| fifth | 73 | 12.1% | Perfect-fifth substitution |
| other | 91 | 15.1% | No simple relationship |
| relative | 24 | 4.0% | Relative major/minor |
| parallel | 23 | 3.8% | Same tonic, wrong mode |
| semitone | 16 | 2.6% | Off by one semitone |

### MIK 500 sample (490 scored, normalized stratification, seed=42)

| Error type | Count | % | Meaning |
|---|---|---|---|
| correct | 302 | 61.6% | Exact key match (MIK agreement) |
| fifth | 63 | 12.9% | Perfect-fifth substitution |
| other | 61 | 12.4% | No simple relationship |
| relative | 30 | 6.1% | Relative major/minor |
| parallel | 18 | 3.7% | Same tonic, wrong mode |
| semitone | 15 | 3.1% | Off by one semitone |

## By format (MIK sample)

| Format | n | Exact % | MIREX | Notes |
|---|---|---|---|---|
| mp3 | 469 | 68.7% | 0.759 | Primary format |
| m4a | 9 | 88.9% | 0.889 | AAC/ALAC via Symphonia isomp4 |
| flac | 13 | 69.2% | 0.792 | |
| wav | 6 | 50.0% | 0.500 | Intermittent decode failures (non-standard fmt) |

## By genre (GiantSteps — primary benchmark)

| Genre | n | Exact % | MIREX |
|---|---|---|---|
| indie-dance nu-disco | 14 | 85.7% | 0.893 |
| pop rock | 7 | 85.7% | 0.900 |
| glitch-hop | 6 | 83.3% | 0.867 |
| psy-trance | 5 | 80.0% | 0.800 |
| deep-house | 77 | 72.7% | 0.805 |
| electro-house | 51 | 70.6% | 0.786 |
| house | 47 | 68.1% | 0.732 |
| electronica | 20 | 65.0% | 0.725 |
| dubstep | 22 | 63.6% | 0.750 |
| breaks | 14 | 57.1% | 0.607 |
| techno | 34 | 55.9% | 0.638 |
| trance | 58 | 56.9% | 0.691 |
| drum-and-bass | 38 | 55.3% | 0.597 |
| chill-out | 11 | 54.5% | 0.645 |
| minimal | 11 | 54.5% | 0.545 |
| progressive-house | 88 | 53.4% | 0.650 |
| tech-house | 81 | 53.1% | 0.607 |

Progressive house and tech house are the weakest genres (53.4% and 53.1%).
These genres often use sustained pad chords and arpeggios that create
ambiguous chroma distributions — a known limitation of template methods
on electronic music.

## CNN experiment status: INVALID, deferred

The CNN experiment (commit c9bb494, "Phase 11 verdict: CNN trained,
evaluated, and CUT — 29.6% vs 68.2% classical") reported 29.6% ± 2.3%
five-fold cross-validation accuracy. This result does **not** constitute
a fair evaluation. The experiment had multiple implementation bugs:

1. **Windowing bug** (`extract_features.py:68-74`): `load_audio` loads
   only the first 30 seconds (`duration=30.0`), making the centering
   branch unreachable. Features are then truncated to 252 frames (~5.85
   seconds at hop 512). The model effectively learns from the beginning
   of each track, not a representative sample.

2. **Augmentation is a no-op** (`train_cv.py:64-73,141`): After adding
   the channel dimension (`data[:, np.newaxis, :, :]`), `np.roll(d, s,
   axis=0)` rolls the channel dimension (length 1). Every "augmented"
   sample is identical to its original.

3. **Model selection bias** (`train_cv.py:109-111`): Each fold selects
   and reports its best epoch on the same validation fold, introducing
   optimistic bias.

4. **Missing pitch-shift augmentation**: The reference CNN work
   (Korzeniowski & Widmer, 2017) uses pitch shifts from −4 to +7
   semitones. Our experiment omits this entirely.

5. **Insufficient training data**: The experiment used only 604
   GiantSteps tracks. The published CNN work trained on 1,077
   high-confidence GiantSteps-MTG tracks and achieved 67.9% exact on the
   held-out 604-track GiantSteps set.

**Verdict:** The CNN was not fairly evaluated. "CNN cut" should be
changed to "experiment invalid; deferred." A corrected experiment
requires: (a) the GiantSteps-MTG training audio (1,486 tracks, Zenodo
DOI 10.5281/zenodo.1101082), (b) fixed windowing/augmentation/epoch
selection, and (c) the Korzeniowski pitch-shift protocol. The local
classical result can still render first while an optional model upgrades
it asynchronously.

## Engine issues identified

The following issues were identified during the accuracy review and are
prioritized for remediation:

1. **Ranked alternatives are not global runner-ups.** The ensemble
   discards 23 scores per segment and retains only that segment's winner
   (`ensemble.rs:298-329`). A key that comes second in every segment is
   omitted completely. On the stored GiantSteps report, the truth
   appeared among the returned candidates for only 49.8% of wrong
   predictions.

2. **Confidence is not calibrated.** The formula `0.6 × agreement + 0.4
   × cosine_score` is invented, not measured. With one segment,
   agreement is always 1.0, giving confidence a 0.6 floor. Even
   predictions above 0.90 confidence were only 78.2% accurate on
   GiantSteps.

3. **Abstention cannot trigger.** The timeline abstention threshold is
   0.35 (`key_timeline.rs:21`), but confidence has a 0.6 floor, so
   per-segment abstention never fires. On the stored GiantSteps run, a
   0.35 threshold retained all 604 tracks.

4. **Timeline modulation detection is naive.** Any disagreement among
   eight independently classified chunks is called a "modulation"
   (`key_timeline.rs`). There is no smoothing, persistence requirement,
   or boundary detection.

5. **Genre profiles are guessed and backwards.** `genre_profiles.rs:31-43`
   comments claim Krumhansl was designed for classical and Temperley for
   rock/pop, but Essentia documentation says the reverse. The weights
   are guesses, not measurements, and the module is not wired into the
   main ensemble path.

6. **MIK used as calibration target.** The plan's own rule says MIK must
   be an opinion and disagreement source, never a calibration target
   (`plan-dfdfe6627c43db0f.md:238`). The log-gain and ensemble weights
   were selected on MIK agreement, violating this rule.

## Tuning history

### Phase 4: Profile weight rebalancing + log compression

Changed `ProfileWeights::default()` from `{0.4, 0.5, 0.5}` to
`{0.15, 0.25, 1.0}` (krumhansl, temperley, shaath). Applied
`log(1 + gain * chroma[i])` before cosine similarity. These changes
were calibrated on the MIK 500-sample, which violates the plan's
calibration rule. They need re-evaluation against GiantSteps with
frozen validation/test splits.

### Phase 5: Log compression gain tuning

Increased 12-bin `LOG_GAIN` from 2.0 to 5.0. On the MIK sample this
changed 11 predictions (fixed 4, broke 2, net +2 after WAV decode
instability). On GiantSteps it changed 20 predictions (fixed 10, broke
5, net +5). The paired exact-test p-value is approximately 0.30, so
this is a provisional improvement, not a demonstrated optimum.

### Approaches that did NOT work

- **HPCP subharmonic summation**: 66.2% MIK (−2.0). Profiles expect
  plain chroma, not HPCP.
- **Tuning estimation**: 67.8% MIK (−0.4). MIK corpus is A=440.
- **Mode-flip heuristic**: No effect at 1.5× threshold; harmful at 1.2×.
  EDM has both thirds coexisting (Faraldo 2017).
- **Fifth-disambiguation via tonic prominence**: 66.4% MIK (−1.8). Bass
  plays chord root, not key tonic in electronic music.
- **Pearson correlation**: 43.9% MIK (−24.3%). Parallel-mode errors
  exploded from 14 to 69.

## Reproducing

```powershell
cd src-tauri
$env:PATH = "C:\Users\louis.media\.cargo\bin;" + $env:PATH

# GiantSteps (primary accuracy benchmark, 604 tracks)
cargo run --release --bin tunelock-bench -- --giantsteps ..\ground-truth\giantsteps-key

# MIK 500 sample (MIK agreement, normalized stratification, seed=42)
cargo run --release --bin tunelock-bench -- --corpus ..\ground-truth\MIKCompleteLibrary.csv --limit 500 --seed 42 --manifest ..\ground-truth\manifest-mik-500-seed42.json
```

## Remediation plan

1. ✅ Repair benchmark and documentation (this file)
2. ✅ Freeze train/validation/test manifests with seeded, normalized sampling
3. Return and aggregate all 24 key scores per segment; calibrate confidence
4. Run clean ablations: 12/72 paths, no-HPSS, kernel sweep, multiple windows
5. Implement braw/bgate HPCP experiment on a separate chroma path
6. Build gold set infrastructure + key-identification tooling
7. Download MTG dataset, fix CNN bugs, re-run corrected experiment
