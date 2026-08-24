# TuneLock key-model bakeoff

This folder is benchmark infrastructure, not a production model runtime. Its
purpose is to answer three questions before an engine integration is considered:

1. Does a candidate model beat TuneLock's 389/604 GiantSteps development
   baseline by itself?
2. Which errors does it correct that TuneLock misses?
3. Is the paired exact oracle at least 453/604, making the 75% stretch target
   possible for this model pair?

## Evaluation contract

- `ground-truth/giantsteps-key` is a development benchmark, not an untouched
  final test.
- External runners export all 24 probabilities with their original string
  labels. They do not implement Camelot or MIREX logic.
- `tunelock-key-bakeoff` parses those labels with TuneLock's Rust proof code and
  reports standalone, top-k, error overlap, oracle, and five-fold out-of-fold
  posterior fusion.
- A full-corpus best fusion weight is intentionally not reported.
- Generated posteriors, model checkouts, environments, and weights live below
  gitignored `ml/data`, `ml/venv`, or `ml/models` paths.
- No model is placed in the app until it wins reproducibly and passes the frozen
  final-holdout, license, latency, and packaging gates.

## Current result

The release baseline remains 389/604 (64.4%). The strongest reproducible
acoustic candidate so far is a pinned Myna-Vertical backbone with an
MTG-supervised head and Rust-aligned transposition averaging:

- standalone: 425/604 (70.4%), MIREX 0.764, top-3 85.4%;
- fixed equal blend observed: 428/604 (70.9%);
- paired TuneLock/Myna oracle: 453/604 (75.0%).

The oracle is only a ceiling. It says the pair contains enough complementary
correct answers to make the stretch target possible; it does not provide a
deployable way to choose those answers. No learned model is integrated into the
application in this checkpoint.

## S-KEY control

The first control is the MIT-licensed
[`deezer/skey`](https://github.com/deezer/skey) repository, pinned for the
recorded run at revision:

```text
918b83d273568d5041569bb8068843d19a335726
```

The checkout and its 765 KB checkpoint are external research inputs; they are
not vendored or bundled. With a compatible checkout at
`ml/data/external/skey`, export full-track posteriors:

```powershell
ml\venv\Scripts\python.exe ml\keybench\run_skey.py `
  --dataset-dir ground-truth\giantsteps-key `
  --skey-root ml\data\external\skey `
  --output ml\data\keybench\skey-fulltrack.jsonl `
  --device cuda
```

The JSONL writer is append-only and resumable. A changed model revision or crop
protocol must use a new output path.

Generate a fresh TuneLock report with all 24 soft-vote candidates:

```powershell
cd src-tauri
C:\Users\louis.media\.cargo\bin\cargo.exe run --release `
  --bin tunelock-bench -- `
  --giantsteps ..\ground-truth\giantsteps-key `
  --out ..\ml\data\keybench\tunelock-current.json
```

Then run the paired evaluator:

```powershell
C:\Users\louis.media\.cargo\bin\cargo.exe run --release `
  --bin tunelock-key-bakeoff -- `
  --giantsteps ..\ground-truth\giantsteps-key `
  --tunelock ..\ml\data\keybench\tunelock-current.json `
  --model ..\ml\data\keybench\skey-fulltrack.jsonl `
  --out ..\ml\data\keybench\skey-bakeoff.json
```

## Dataset audit

As of 2026-08-23:

- GiantSteps-key: 604/604 audio previews present.
- GiantSteps-MTG: 1,486 key annotations, 1,356 distinct raw artist strings,
  1,486/1,486 checksum-verified audio previews, and no track-ID or exact-audio
  overlap with GiantSteps-key.
- GiantSteps-MTG contains 1,477 unique audio hashes and nine exact-duplicate
  groups. The published clean subset is 1,077 tracks after requiring confidence
  2, one parseable key, and an empty annotator comment.
- The fixed clean split removes seven duplicate rows, leaves 1,070 unique
  recordings, and has zero artist-token, recording-hash, or connected-component
  overlap between training and validation.
- Compute: RTX 2070 SUPER (8 GB); PyTorch 2.7.1 + CUDA 12.8 in the ignored
  research environment.

The repository's historical statement that Zenodo provides a single ~2 GB MTG
audio ZIP is incorrect. The dataset release provides annotations, checksums, and
an old per-preview Beatport/backup download script; every acquired preview was
therefore verified against its published MD5.

## Myna experiment notes

- Backbone: `oriyonay/myna-vertical`, MIT, revision
  `6b9e1e5aae0832335d61d7a38764114e496824d4`.
- Head: published 384 -> 4096 -> 4096 -> 24 architecture, with the paper's
  0.99 dropout between hidden layers.
- Labels and every pitch-transposition target originate in the Rust corpus
  manifest; Python does not add another key/Camelot vocabulary.
- The 1,077-track clean protocol and the 1,349-track all-confidence,
  unambiguous-label ablation remain separately named and reported.
- The fast winning run uses linear pitch+speed resampling as an explicit
  ablation. A cached, pitch-only phase-vocoder path was numerically verified
  against torchaudio but has not completed a full training corpus.
- Generated embeddings, checkpoints, posteriors, and external model checkouts
  remain gitignored research artifacts.
