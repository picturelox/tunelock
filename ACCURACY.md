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

### 75% exact-key development contract (2026-08-23)

The stretch target is **453/604 exact (75.0%)** on GiantSteps-key. The current
baseline is 389/604, so reaching it requires 64 net additional correct tracks.
This is a development objective, not a current accuracy claim.

GiantSteps-key is no longer an untouched test: it has already selected classical
engine settings. It may be used for repeatable model bakeoffs, paired error
analysis, and out-of-fold fusion experiments, but final superiority requires a
new frozen, artist/recording-family-disjoint, human-adjudicated holdout.

Required evidence for every external or learned model:

1. all 24 labeled scores, model/data revision, license, latency, and failures;
2. standalone exact, MIREX, top-3/top-5, and error taxonomy;
3. paired overlap with the 389/604 TuneLock baseline and exact oracle union;
4. fusion selected out-of-fold, never by reporting the best weight on all 604;
5. duplicate/fingerprint and artist-group checks between training and evaluation;
6. a separate assisted leaderboard for tags, OSINT, knowledge bases, or LLMs.

The first controlled bakeoff used Deezer's MIT-licensed S-KEY checkpoint at
revision `918b83d273568d5041569bb8068843d19a335726`. Subsequent runs used the
MIT-licensed `oriyonay/myna-vertical` backbone pinned at revision
`6b9e1e5aae0832335d61d7a38764114e496824d4`. The local result remains first and
no network/model call has been added to the production engine. A larger
MIT-licensed `oriyonay/myna-85m` ablation is pinned at revision
`f2c66dc432aa070bc9a82ed7a90a411c3b33f0eb` and is recorded below as a rejected
candidate, not as an upgrade.

#### 75% sprint results

All rows below score the same 604 development tracks with TuneLock's Rust
parsing and MIREX implementation. "Pair oracle" means either TuneLock or the
candidate was correct; it is a ceiling, not a selectable result.

| Candidate | Exact | MIREX | Top-3 | Pair oracle | Best measured fixed/OOF fusion |
|---|---:|---:|---:|---:|---:|
| TuneLock release baseline | 389/604 (64.4%) | 0.725 | 84.1% | — | — |
| S-KEY full-track control | 389/604 (64.4%) | 0.721 | 81.3% | 445/604 (73.7%) | 404/604 (66.9%) |
| KeyMyna Billboard ONNX control | 352/604 (58.3%) | 0.675 | 81.6% | 439/604 (72.7%) | 391/604 (64.7%) |
| Myna, published head, no pitch augmentation | 383/604 (63.4%) | 0.718 | 82.8% | 441/604 (73.0%) | 399/604 (66.1%) |
| Myna, clean 1,077 protocol + pitch/speed augmentation | 411/604 (68.0%) | 0.745 | 84.8% | 451/604 (74.7%) | 403/604 (66.7%) |
| Myna, 1,349 unambiguous-label ablation + pitch/speed augmentation | 415/604 (68.7%) | 0.753 | 85.1% | 450/604 (74.5%) | 417/604 (69.0%) |
| Myna v7, 1,349 faithful pitch-only augmentation, base view | 406/604 (67.2%) | 0.741 | 84.9% | 446/604 (73.8%) | 408/604 (67.5% OOF) |
| Myna v7 faithful + probability-averaged transpositions | 406/604 (67.2%) | 0.742 | 84.9% | 448/604 (74.2%) | 413/604 (68.4% OOF) |
| Myna v7 faithful + logit-averaged transpositions | 407/604 (67.4%) | 0.742 | 85.1% | 449/604 (74.3%) | 409/604 (67.7% OOF) |
| **Myna v6 fast + probability-averaged transpositions** | **426/604 (70.5%)** | **0.765** | **85.6%** | **453/604 (75.0%)** | 426/604 (70.5%) |
| Myna v6 fast + logit-averaged transpositions | 425/604 (70.4%) | 0.764 | 85.4% | 453/604 (75.0%) | **428/604 (70.9% fixed)** |
| Myna85M full hybrid embedding, no augmentation | 372/604 (61.6%) | 0.693 | 83.3% | 439/604 (72.7%) | 393/604 (65.1% OOF) |
| Myna85M vertical branch, no augmentation | 378/604 (62.6%) | 0.701 | 83.8% | 439/604 (72.7%) | 402/604 (66.6% OOF) |

The verified acoustic top-1 result is therefore **70.5%**, 37 additional exact
matches over the 64.4% release baseline. A fixed equal blend using the
logit-averaged transposition variant reaches **70.9% (428/604)**. The exact
75.0% pair oracle proves sufficient complementary errors now exist, but the
remaining 25 oracle-only corrections cannot be claimed until a selector learns
them on data outside these 604 development labels.

The current fast augmentation is explicitly an ablation: linear resampling
changes pitch and speed together. The separately identified sparse-v1 faithful
path matches all twelve pinned torchaudio views within 1.19e-7 maximum waveform
error and matching-track Myna embeddings within 8.35e-7 maximum error. Its
clean training cache is now complete at 16,080/16,080 embeddings with zero
failures. Using the locked v6 head protocol, faithful augmentation improved the
artist/recording-disjoint MTG validation fold from 170/266 to 172/266, but its
base-view GiantSteps development result fell from 415/604 to 406/604. Faithful
probability TTA remained 406/604; logit TTA reached 407/604. Their classical
pair oracles (448 and 449) are below the 453-track stretch target, so v7 is
rejected as a standalone or ordinary blend candidate.

V7 is not useless evidence: classical + v7 logit TTA + the existing v6 models
have a 459/604 (76.0%) exact oracle, ten corrections beyond the classical/v7
pair. Yet the measured multi-model OOF convex blend is only 426/604 (70.5%),
and most folds assign v7 zero weight. This establishes diversity that a future
selector may study; it does not raise the verified score. The accuracy leader
therefore remains v6 probability TTA at 426/604 (70.5%), with the previously
measured fixed blend at 428/604 (70.9%). Neither candidate is eligible for the
app until it repeats on the sealed final holdout and passes latency,
calibration, data-rights, packaging, and commercial-license review.

#### Myna85M and production-artifact checkpoint

The larger hybrid backbone was evaluated without letting GiantSteps select its
head. Three head shapes were first compared only on fixed, artist/recording-
disjoint MTG fold 0. The compact 1,536 -> 2,048 -> 24 head won that round at
57.5% validation exact (published high-dropout head 49.6%; wide head 56.8%). A
second architecture-driven audit compared the hybrid branches: the vertical
128x2-patch half won at 59.0%, versus 49.6% for the horizontal half. Only the
locked winners were then scored on GiantSteps. Fold 0 has now selected several
head variants, so its percentages are comparative development evidence too,
not untouched-test estimates.

The vertical branch improves the full Myna85M result by six tracks, but its
439/604 TuneLock pair oracle remains fourteen tracks below the 453/604 stretch
target. The larger model is therefore rejected for integration in this
checkpoint. Model size alone did not improve key accuracy or error diversity.

Separately, the current Myna-Vertical research candidate now has a reproducible
deployment-shaped artifact path (updated 2026-08-24):

- one combined backbone + key-head ONNX graph, 162,782,585 bytes;
- ONNX Runtime CPU parity within 1.02e-6 maximum absolute logit error, with
  identical test-batch argmaxes;
- a schema-4 manifest binding the model SHA-256, exact input/output shapes,
  Rust-canonical 24-label order, aggregation rule, model revision, explicit
  research/data-rights status, and machine-readable mel plus real-file audio
  preprocessing parameters, twelve pitch views, label alignment, and TTA
  weighting;
- an opt-in Rust `neural-key` feature that validates the manifest, size, hash,
  labels, and finite logits, then loads an externally supplied ONNX Runtime;
- a native 16 kHz audio-to-mel implementation of the pinned nnAudio 0.3.3
  contract (2048-point periodic-Hann STFT, 512 hop, reflect centering, 128
  Slaney area-normalized bands, power 2, and 196-frame chunks);
- deterministic parity against the committed nnAudio reference fixture:
  maximum absolute mel error 0.0002594 and maximum scaled relative error
  0.00000356;
- an amplitude-preserving Symphonia decode path, float32 channel mean, and a
  native implementation of torchaudio 2.7.1's default Hann sinc resampler;
- a committed deterministic stereo 44.1 kHz PCM16 fixture whose native 16 kHz
  output is within 4.92e-6 maximum and 4.83e-7 mean absolute sample error of
  the pinned torchaudio output;
- a release-mode audit of 20 real GiantSteps MP3s against cached Python
  posteriors: 20/20 top-1 agreement, zero failures, 0.000632 mean absolute
  posterior error, 0.0167 maximum posterior error, and 866 ms mean execution;
- a native shared-STFT phase vocoder and optimized sinc resampler for all
  twelve faithful pitch views. Across the committed torchaudio fixture its
  maximum waveform error is 0.000679 and global mean error is 0.0000558;
- end-to-end Rust TTA that runs the base plus twelve shifted views, preserves
  major/minor vocabulary alignment, and averages probabilities. A real-file
  Python/Rust smoke chose the same C-minor key with 0.00131 mean and 0.00606
  maximum final-posterior error; native release execution took 16.2 seconds
  including artifact/runtime load on this machine.

This is a production boundary, not production promotion. The default build
still downloads and bundles no model or runtime, and the release analyzer still
returns the classical result first. A sealed final holdout, calibration, rights
review, artifact distribution, and background lifecycle/UI integration remain
open gates. Rust now owns real-file decode through faithful twelve-view TTA,
but the exported head was trained using the earlier pitch+speed ablation. The
faithful cache must finish, the head must be retrained, and both accuracy and
latency/view-subset tradeoffs must be measured before this path can be promoted.
The parity results above are not ground-truth accuracy, so the key scores are
not raised by this infrastructure checkpoint. Separately, a consistent v6
probability-TTA rerun corrected the acoustic leaderboard from the previously
reported 425-track logit result to 426/604; both remain development results.

The full release-mode GiantSteps benchmark was rerun after this checkpoint:
604/604 scored with zero failures, 389 exact (64.4%), 0.725 MIREX, 69.0% tonic
accuracy, and 85.1% Camelot compatibility. This exactly reproduces the frozen
classical baseline; the local immediate result did not regress.

Dataset/protocol audit:

- all 1,486 GiantSteps-MTG previews are present and match their published MD5;
- there are 1,477 unique hashes, nine duplicate groups, no exact audio hashes
  shared with GiantSteps-key, and no shared track IDs;
- the historical clean protocol resolves exactly to 1,077 tracks: confidence 2,
  a single parseable key, and an empty annotator comment;
- exact-audio deduplication leaves 1,070 clean training recordings; the fixed
  artist/recording-component validation split has zero measured overlap;
- the 1,349-track all-confidence/unambiguous run is retained as a separately
  named ablation rather than silently replacing the published clean protocol.

### 2026-08-23 evidence-semantics verification

Slice A changed only the supporting diagnostics emitted by soft temporal
voting: `segment_count` is now the number of analyzed sections in which a
candidate was the strongest key, and `agreement` is that count divided by the
number of valid sections. Aggregate soft scores still determine the ranking.

The full 604-track GiantSteps release benchmark was rerun after this change:

| Metric | Before | After |
|---|---:|---:|
| Key exact match | 64.4% (389/604) | 64.4% (389/604) |
| Tonic correct | 69.0% | 69.0% |
| MIREX weighted | 0.725 | 0.725 |
| Camelot compatible | 85.1% | 85.1% |
| Failures | 0 | 0 |

This confirms that the user-facing evidence repair did not change key or BPM
predictions. A unit test also requires candidate section counts to sum to the
eight evaluated sections.

### GiantSteps-key (primary accuracy benchmark)

| Metric | Value |
|---|---|
| **Scored** | 604 (0 failed) |
| **Key exact match** | **64.4%** (389/604) |
| Tonic correct (mode-agnostic) | 69.0% |
| MIREX weighted score | 0.725 |
| **Camelot compatible** | **85.1%** |
| Avg time per track | ~5,500 ms |

**Historical context** (from the GiantSteps ISMIR 2015 paper):

| System | Exact match on GiantSteps |
|---|---|
| Mixed In Key (commercial) | 67.22% |
| Rekordbox (commercial) | 71.85% |
| **TuneLock (current)** | **64.4%** |

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
| **Key exact match (MIK agreement)** | **65.7%** (322/490) |
| Tonic correct (mode-agnostic) | 68.8% |
| MIREX weighted score | 0.730 |
| **Camelot compatible** | **84.1%** |
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
| correct | 389 | 64.4% | Exact key match |
| fifth | 70 | 11.6% | Perfect-fifth substitution |
| other | 72 | 11.9% | No simple relationship |
| parallel | 28 | 4.6% | Same tonic, wrong mode |
| relative | 27 | 4.5% | Relative major/minor |
| semitone | 18 | 3.0% | Off by one semitone |

### MIK 500 sample (490 scored, normalized stratification, seed=42)

| Error type | Count | % | Meaning |
|---|---|---|---|
| correct | 322 | 65.7% | Exact key match (MIK agreement) |
| fifth | 51 | 10.4% | Perfect-fifth substitution |
| other | 63 | 12.9% | No simple relationship |
| relative | 24 | 4.9% | Relative major/minor |
| parallel | 15 | 3.1% | Same tonic, wrong mode |
| semitone | 14 | 2.9% | Off by one semitone |

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
3. ✅ Return and aggregate all 24 key scores per segment; calibrate confidence
4. ✅ Run clean ablations: 12/72 paths, no-HPSS, kernel sweep, multiple windows
5. ✅ Implement braw/bgate HPCP experiment on a separate chroma path
6. ✅ Build gold set infrastructure + key-identification tooling
7. ✅ Download MTG dataset, fix CNN bugs, prepare corrected experiment

## Step 7: CNN experiment fixes

**Three critical bugs were fixed in the CNN training pipeline:**

1. **Windowing bug** (`extract_features.py`): `librosa.load(duration=30.0)`
   truncated audio to the first 30 seconds. The centering code was dead
   because `len(y)` could never exceed `sr * duration`. Fixed: now loads
   the full audio and centers properly.

2. **Augmentation bug** (`train_cv.py`): `np.roll(d, s, axis=0)` rolled
   the batch dimension (length N), not the time axis. Augmentation
   produced identical duplicates instead of time-shifted variants. Fixed:
   now rolls axis=2 (time frames) and adds pitch-shift augmentation
   following the Korzeniowski protocol (±4 semitone circular shifts with
   corresponding label shifts).

3. **Epoch selection bias** (`train_cv.py`): Best epoch was selected on
   the same validation fold used for reporting final accuracy. This
   inflates the reported accuracy because the model "peeked" at the test
   fold during training. Fixed: now splits training data into train +
   internal validation (90/10), selects the best epoch on internal
   validation only, and evaluates on the external fold exactly once.

**Current status:** All 1,486 MTG previews have now been acquired and checksum
verified. The legacy CNN itself has not been rerun; instead, the reproducible
Myna experiment above established a substantially stronger 70.5% acoustic
candidate with the same local training corpus. GiantSteps-key is now explicitly
a repeatedly observed development benchmark, not an untouched test; any CNN
revival must use the same leakage and final-holdout contract as the Myna work.

The prior 29.6% CNN result is INVALID — it was produced by a broken
experiment. No conclusion about CNN viability can be drawn until the
corrected experiment is run.

## Step 6: Gold set infrastructure + key-identification tooling

**Goal:** Build a 300-500 track gold set with user-adjudicated key labels,
stored separately from MIK opinions and engine predictions. Train the
user to identify keys accurately using scientific/musical methods.

**Infrastructure:**
- `gold_annotations` table: stores user key judgments with tonic, mode
  (major/minor/ambiguous/atonal), modulation flag, annotator confidence
  (1-5), evidence notes, annotator ID, and blind flag.
- `training_sessions` table: tracks ear-training exercise results.
- Rust commands: `save_gold_annotation`, `get_gold_annotations`,
  `get_gold_annotation_summary`, `save_training_session`,
  `get_training_stats`.
- Frontend "Gold Set" view with 4 tabs: Overview, Ear Training,
  Annotate, Statistics.

**Key-identification training tools:**
- **Pitch Identification**: plays a sine wave at a random pitch class;
  user identifies it. Builds absolute pitch reference.
- **Major/Minor Identification**: plays a triad (root + third + fifth);
  user identifies the mode. Builds mode discrimination.
- Both exercises record accuracy and response time.

**Blind annotation workflow:**
- User selects a track from the library.
- The engine's prediction is hidden by default (blind mode).
- User listens to the track and selects tonic, mode, confidence, and
  optional evidence notes.
- User can optionally reveal the engine's prediction after annotating.
- Annotations are stored with the `blind` flag for later analysis.

**Self-agreement measurement:**
- The summary tracks how many tracks have 2+ annotations from the same
  annotator and whether they agree on (tonic, mode).
- This measures annotation reliability without requiring a second
  annotator (though a second annotator is supported via `annotator_id`).

**Status:** Infrastructure is complete. The user needs to:
1. Practice ear training exercises to build key-identification skills.
2. Annotate tracks blindly, aiming for 300+ tracks.
3. Re-annotate the same tracks after a delay to measure self-agreement.
4. Once the gold set is established, engine accuracy can be measured
   against it instead of MIK agreement.

## Step 5: HPCP + braw/bgate EDM experiment

**Goal:** Test whether the Faraldo braw/bgate profiles, designed
specifically for electronic dance music, outperform the existing
Krumhansl/Temperley/Sha'ath profiles on GiantSteps (which is heavily EDM).

**Implementation:**
- Added `hpcp_from_spec()` — Harmonic Pitch Class Profile chroma with
  4-harmonic summation and 200 Hz high-pass filter.
- Added braw and bgate profile sets from Essentia (Faraldo 2017).
  Each set has 3 profiles: major, minor, and "other" (amodal).
- Added `temporal_vote_edm_soft()` — soft voting on HPCP with braw/bgate.
- Tested both HPCP and plain chroma with both profile sets.

**Results on GiantSteps (604 tracks):**

| Configuration | Exact | Count | MIREX | vs default |
|---|---|---|---|---|
| **Default (dual, K+T+Sha'ath)** | **64.4%** | **389** | **0.725** | **baseline** |
| HPCP + braw | 44.0% | 266 | 0.544 | −20.4 |
| HPCP + bgate | 42.7% | 258 | 0.528 | −21.7 |
| Plain chroma + braw | 52.6% | 318 | 0.618 | −11.8 |
| Plain chroma + bgate | 52.5% | 317 | 0.608 | −11.9 |

**Conclusion:** The braw/bgate profiles perform significantly worse than
the existing profiles on GiantSteps, regardless of chroma representation.
The HPCP representation makes things worse, not better.

**Why the EDM path underperforms:**
1. Our simplified HPCP lacks spectral whitening — a critical preprocessing
   step in the Faraldo pipeline that normalises spectral peaks before PCP
   computation. Without it, HPCP is dominated by the strongest spectral
   peaks regardless of harmonic relationship.
2. The braw/bgate profiles were trained on Beatport metadata, which is
   heavily biased toward specific EDM subgenres. GiantSteps is a mixed
   EDM corpus with diverse subgenres.
3. The "other" (amodal) profile adds noise rather than capturing useful
   ambiguity — it competes with minor and pulls predictions away from
   the correct answer.
4. The existing Krumhansl/Temperley/Sha'ath profiles are more robust
   across genres because they capture universal tonal hierarchies.

**Decision:** The EDM path is retained as an ablation option
(`--edm-braw`, `--edm-bgate`, `--edm-braw-plain`, `--edm-bgate-plain`)
but is NOT promoted to the default path. The universal fallback
(Krumhansl/Temperley/Sha'ath dual chroma) remains the primary engine.

A future iteration could implement the full Faraldo pipeline with
spectral whitening and PCP gate, then re-evaluate. For now, the
simplified HPCP is insufficient.

## Step 4: Clean ablations

All ablations run against GiantSteps-key (604 tracks, primary benchmark)
with the soft voting from Step 3. Each row changes exactly one parameter.

### Component ablations

| Configuration | Exact | Count | MIREX | vs default |
|---|---|---|---|---|
| **Default (dual, HPSS k=17)** | **64.4%** | **389** | **0.725** | **baseline** |
| No HPSS (raw spectrogram) | 63.7% | 385 | 0.722 | −0.7 |
| 12-bin only (K+T+Sha'ath-12) | 49.3% | 298 | 0.609 | −15.1 |
| 72-band only (Sha'ath-72) | 60.4% | 365 | 0.692 | −4.0 |

**Key findings:**
- The 72-band Sha'ath path is the dominant contributor (+15.1 points over
  12-bin alone). The 12-bin Krumhansl/Temperley paths add +4.0 points of
  mode discrimination (parallel errors: 42 without vs 28 with).
- HPSS contributes +0.7 points. It helps, but the engine works nearly as
  well without it — the log compression and profile matching are doing
  most of the work.

### HPSS kernel sweep

| Kernel size | Coverage | Exact | Count | MIREX |
|---|---|---|---|---|
| 5 | ~0.93 s | 64.1% | 387 | 0.721 |
| 9 | ~1.67 s (prior default) | 63.4% | 383 | 0.715 |
| **17** | **~3.15 s (chosen)** | **64.4%** | **389** | **0.725** |
| 25 | ~4.65 s | 63.4% | 383 | 0.719 |

A larger kernel gives cleaner harmonic separation. The optimum is at 17
(~3.15 s). Beyond that, key changes get smeared and accuracy drops.

### Analysis window sweep

| Window | Exact | Count | MIREX |
|---|---|---|---|
| 30 s | 59.3% | 358 | 0.689 |
| 60 s | 62.9% | 380 | 0.710 |
| 90 s | 63.9% | 386 | 0.719 |
| **180 s (default)** | **64.4%** | **389** | **0.725** |

Longer windows are better. The 180-second centered window captures more
tonal content and gives the 8-segment voting more material. The 30-second
window is too short — fifth errors jump from 70 to 91.

## Step 3: Soft temporal voting

**Problem:** The ensemble discarded 23 scores per segment and retained only
that segment's winner. A key that came second in every segment was
completely invisible. On the stored GiantSteps report, the truth appeared
among the returned candidates for only 49.8% of wrong predictions.

**Fix:** Added `temporal_vote_ranked_dual_soft()` which:
1. Computes all 24 combined scores per segment (not just the winner).
2. Normalises each segment's scores to [0, 1] (min-max within the segment).
3. Sums the normalised scores across segments (soft aggregation).
4. Returns all 24 candidates ranked by aggregate score.
5. Confidence = winner's aggregate / total aggregate (proper probability-
   like measure, not an invented 0.6×agreement + 0.4×score blend).

**Results:**

| Benchmark | Before (hard) | After (soft) | Change |
|---|---|---|---|
| GiantSteps (604) | 62.4% (377) | 63.4% (383) | +1.0 (+6) |
| MIK 500 (490) | 61.6% (302) | 64.9% (318) | +3.3 (+16) |

The soft voting also fixed the confidence calibration issue: confidence
is now a proper fraction of total score (ranging from ~0.04 for a uniform
distribution to ~1.0 for a dominant key), not an invented blend with a
0.6 floor. Abstention can now trigger when no key dominates.
