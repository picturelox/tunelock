# TuneLock Core Intelligence Plan

Updated: 2026-08-23
Branch: `core-intelligence`

## Product decision

TuneLock is now focused on one job:

> Drop or open an audio file and receive immediate, honest, extensive musical
> intelligence: global key, local key movement, tempo and beat-grid behavior,
> intensity over time, and useful directional relationships to other tracks.

The previous "ultimate mix planner" direction expanded outward before the
analysis and proof loops were finished. Console, Mix Canvas, Delivery, Assist,
Gold Set, piano, metronome, and other completed experiments are preserved, but
they are frozen behind an Experimental area. They are not active roadmap work.

## Non-negotiable principles

1. The local classical result renders first. Model loading, network access,
   enrichment, waveform generation, and relationship search never block the
   first key/BPM readout.
2. Accuracy claims are earned on frozen data. MIK and Traktor are opinion
   sources, never ground truth.
3. Confidence is empirical. Raw score mass, model agreement, and section vote
   counts must not be presented as calibrated correctness probabilities.
4. One decode feeds a versioned analysis graph. Derived artifacts are cached by
   audio fingerprint plus engine version and are never allowed to drift silently.
5. Originals are never modified, moved, or deleted.
6. Rust `harmony/` and TypeScript `lib/harmony.ts` remain the only mirrored
   harmony vocabulary, backed by shared vectors.
7. Every engine change starts from the baseline in `ACCURACY.md` and is
   re-measured before it lands.

## The product surface

### Empty state

- Analyze is the default view.
- Drag anywhere or use a visible Open File action.
- Multiple dropped files may quietly build the relationship catalog, but the
  first selected file is shown immediately.
- Recent analyses remain available without requiring a separate import flow.

### Result state

The first viewport contains:

- artwork, artist/title, duration;
- Camelot and standard key;
- BPM plus tempo stability / half-time ambiguity;
- honest evidence wording and close alternatives;
- one aligned musical map: waveform, key regions, beats/downbeats, and
  intensity curve;
- directional "works before / works after" recommendations with reasons.

Raw chroma, candidate tables, timings, Camelot wheel, piano, metronome, and
other diagnostics live under progressive disclosure.

## Current verified baseline

- Global key, GiantSteps 604: 64.4% exact, 0.725 MIREX, 85.1% Camelot-compatible.
- Best research-only acoustic candidate: 70.4% exact, 0.764 MIREX; best measured
  fixed blend 70.9%. It is not integrated into the release engine.
- MIK 500 agreement: 65.7% exact agreement; BPM +/-1 is 53.3%.
- Current release key-path smoke: roughly 0.7-1.6 seconds per track on five
  local files after compilation.
- Frontend typecheck and production build pass.
- Rust: 71 tests pass.

Important: GiantSteps has already been used to select HPSS kernel and analysis
window parameters. It is now a development benchmark, not an untouched final
test. A new final holdout is required before superiority claims.

## 75% exact-key stretch target and proof contract

The development stretch target is **453/604 exact (75.0%)** on GiantSteps-key.
The current 389/604 baseline therefore needs 64 net additional correct tracks.
This is a deliberately difficult engineering target, not a promised product
claim and not permission to tune on test answers.

Dataset roles are fixed as follows:

- GiantSteps-key 604 is the repeatable **development benchmark**. It may rank
  experiments, but it can no longer support an untouched-test claim because
  TuneLock's classical path was already tuned on it.
- GiantSteps-MTG is **training material only**. Train/validation splits must be
  grouped by normalized artist and checked for recording/preview duplicates
  before any fit. All 1,486 previews are present and match their published MD5;
  the historical clean slice contains 1,077 confidence-2, unambiguous,
  comment-free tracks.
- A new 300-500 track, artist/recording-family-disjoint, human-adjudicated set is
  the **frozen final holdout**. Its labels remain sealed until a candidate and
  fusion rule are locked.
- MIK, Traktor, file tags, vendor metadata, web knowledge bases, and LLM-derived
  assertions are opinions or enrichment. They get a separate assisted score
  and never enter the acoustic leaderboard as ground truth.

Every model bakeoff must emit all 24 labeled scores and report standalone
exact/MIREX/top-k, paired error overlap with TuneLock, the exact oracle union,
latency, failures, model/data license, and revision. Fusion parameters are fit
out-of-fold; no full-corpus best weight is presented as an honest result. If a
model pair's top-1 oracle is below 453 tracks, that pair cannot reach the stretch
target through selection alone and a more diverse model is required.

Promotion has two gates:

1. **Development gate:** a paired, reproducible improvement over 389/604 with
   no severe-error or latency regression; 453/604 is the stretch checkpoint.
2. **Product gate:** improvement repeats on the sealed final holdout, confidence
   calibration is measured there, the artifact is commercially usable, and the
   model runs asynchronously after the immediate classical result.

## Defects that directly block the focused product

1. The app lands on an eight-channel Console instead of Analyze.
2. The result page presents secondary experiments before a concise summary.
3. Soft-vote `confidence` is uncalibrated score mass. `agreement` mirrors that
   value and every candidate reports all valid sections as its own votes.
4. The runner-up UI uses an unreachable 0.35 absolute threshold.
5. Single-file Analyze returns no energy even though batch analysis computes it.
6. Local-key timeline is eight unsmoothed chunks, uses the older hard-vote
   confidence, treats any disagreement as modulation, and loses the centered
   window's absolute time offset.
7. Analyze relationship discovery only loads 500 library rows; backend helpers
   cap other relationship searches at 5,000.
8. Main Analyze uses the simple tempo detector while the more serious beat-grid
   engine is separate and validated only on synthetic clicks.
9. Beat-grid tempo maps always contain one segment; variable tempo is not yet
   implemented.
10. Waveform and local-key requests re-decode audio independently.
11. Analysis artifacts have no engine version or audio fingerprint staleness
   contract.
12. The corrected CNN experiment is still not a faithful experiment: feature
   extraction keeps 252 frames (about 5.85 s at the current hop), CV is not
   artist-aware, and only four pitch shifts are applied in `train_cv.py`.

## Delivery plan

### Slice A - Honest Analyzer

- Make Analyze the landing view.
- Reduce primary navigation to Analyze and Library; preserve other views under
  a clearly labeled Experimental section.
- Add a visible native Open File action.
- Put the musical summary before playback and secondary visualizations.
- Replace fake confidence percentages with evidence language, real section vote
  counts, and visible close alternatives.
- Compute and persist energy in single-file analysis using the existing pass.
- Keep raw diagnostics under a collapsed disclosure.

Checkpoint A: one file to an honest, useful first viewport with no extra mode
selection. Frontend build and Rust tests are clean.

### Slice B - Versioned analysis graph and proof reset

- Introduce `AnalysisResultV2`, `analysis_version`, audio fingerprint, artifact
  status, and per-stage readiness.
- Decode once; share PCM/features between key, tempo, waveform, intensity, and
  timeline work.
- Persist compact waveform, posterior, beat-grid, and curve artifacts.
- Establish frozen development and artist-disjoint final holdouts.
- Add calibration metrics, top-k recall, severe-error rate, latency percentiles,
  BPM ACC1/ACC2/AOE, beat F-measure, and downbeat F-measure.

Checkpoint B: every visible claim maps to a reproducible metric and stale
analysis is detectable.

### Slice C - Rhythm authority

- Benchmark the beat-grid engine on real annotated corpora.
- Improve octave selection, phase, downbeats, meter, and low-confidence behavior.
- Make beat-grid BPM authoritative only after it beats the current detector.
- Add real piecewise tempo segments and expose half/double alternatives.

Checkpoint C: DJ-usable BPM plus grid/phase, not merely a global number.

### Slice D - Global key model and calibrated ensemble

- Reproduce the Korzeniowski/Widmer protocol faithfully before custom changes:
  full-length time pooling, log-frequency input, all 12 transpositions,
  high-confidence GiantSteps-MTG training, and leakage-safe validation.
- Add diverse adjudicated material for genre-agnostic training.
- Export a small ONNX model and run it asynchronously after the classical result.
- Fuse models on validation data; calibrate the posterior on a separate split.
- Promote only with a measured paired improvement and acceptable latency/size.

Checkpoint D: higher key accuracy without sacrificing instant local output.

### Slice E - Key and intensity over time

- Generate overlapping bar-aligned 24-key posteriors over the full recording.
- Decode 24 keys plus `no stable key` with persistence-aware temporal smoothing.
- Preserve absolute timestamps and distinguish chords from sustained modulation.
- Compute objective loudness/intensity descriptors and curves using a defensible
  loudness standard; fit any 1-10 DJ intensity score only to adjudicated labels.
- Validate local-key frames and boundaries independently of global key.

Checkpoint E: the musical map says what changes, when, and how certain it is.

### Slice F - Directional relationship intelligence

- Compare outgoing regions with incoming regions, not only global labels.
- Rank separate harmonic, rhythmic, phrase, intensity, and spectral/vocal risks.
- Integrate key uncertainty by scoring posterior distributions rather than hard
  labels alone.
- Query the complete catalog server-side.
- Learn ranking weights from blind DJ approval/rejection, never from an LLM.

Checkpoint F: useful before/after suggestions with audible, inspectable reasons.

## Stop rules

- No additional console, stem, scene, delivery, LLM, live-input, or mix-canvas
  work until Checkpoint F.
- No superiority claim from GiantSteps alone.
- No percent called confidence unless calibration demonstrates that meaning.
- No modulation label from one short-window disagreement.
- No single compatibility score without its component reasons and uncertainty.

## Progress log

### 2026-08-23 - Focus reset started

- Created branch `core-intelligence` from `c778129`.
- Repository reviewed against current code rather than status documents alone.
- Verified TypeScript, production frontend build, and all 70 Rust tests.
- Release smoke exposed the confidence/segment semantic defect directly.
- Slice A is complete:
  - Analyze is now the default and only file-analysis front door.
  - Primary navigation is Analyze + Library; frozen surfaces live under LAB.
  - The empty state has a native Open File action and accepts drag/drop.
  - The first result viewport leads with key, BPM, estimated intensity,
    playback, a musical-map foundation, and relationships.
  - Uncalibrated confidence percentages were replaced by evidence language,
    actual section winners, and always-visible alternatives.
  - Single-file analysis computes and persists the existing local energy
    estimate from the already-decoded PCM.
  - Secondary diagnostics are collapsed under `Why this result?`.
- Verification: TypeScript typecheck and production build pass; all 71 Rust
  tests pass.
- Full GiantSteps release recheck: 604/604 scored, 64.4% exact, 0.725 MIREX,
  85.1% Camelot compatible, identical to the established prediction baseline.
- Slice B is next; no new accuracy claim is made by Slice A.

### 2026-08-23 - 75% key-accuracy checkpoint

- Froze 453/604 as the GiantSteps development stretch target and separated the
  development, training, final-holdout, and OSINT-assisted evaluation roles.
- Acquired and checksum-verified all 1,486 MTG previews. The corpus has 1,477
  unique audio hashes, nine exact-duplicate groups, and no exact-audio or ID
  overlap with GiantSteps-key.
- Resolved the published 1,077-track protocol exactly: confidence 2, one
  parseable key, and an empty annotator comment. Exact-audio deduplication leaves
  1,070 recordings; the fixed artist/recording-group split has zero overlap.
- Provisioned an ignored Python 3.11 / PyTorch CUDA research environment on the
  local RTX 2070 SUPER. Nothing is added to the app's critical path.
- Added a resumable posterior adapter for the MIT-licensed S-KEY checkpoint and
  a Rust bakeoff that keeps parsing, MIREX, Camelot compatibility, oracle, and
  out-of-fold fusion inside TuneLock's existing harmony/proof vocabulary.
- S-KEY matches TuneLock at 389/604; its paired oracle is only 445/604, so that
  pair cannot reach the stretch target.
- Added Rust-canonical manifests, leakage audits, resumable Myna embedding and
  pitch caches, a reproducible MLP trainer, and pitch-equivariant TTA evaluation.
- The strongest Myna candidate reaches 425/604 (70.4%) standalone and 428/604
  (70.9%) in the best measured fixed blend. Its paired oracle with TuneLock is
  exactly 453/604 (75.0%): enough complementary answers now exist, but no honest
  selector has recovered the remaining oracle-only corrections.
- The winning fast path is a clearly labeled pitch+speed ablation. A cached
  pitch-only phase-vocoder path was numerically verified but has not completed a
  full corpus run.
- No learned model was integrated. Product promotion still requires the sealed
  final holdout, calibration, latency/size measurement, data-rights review, and
  an asynchronous runtime that never blocks the local classical result.
