# TuneLock key-model bakeoff

This folder is benchmark and artifact-export infrastructure, not an enabled
production model. Its purpose is to answer three questions before an engine
integration is considered:

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

- probability-averaged standalone: 426/604 (70.5%), MIREX 0.765,
  top-3 85.6%;
- logit-averaged standalone: 425/604 (70.4%);
- fixed equal blend observed: 428/604 (70.9%);
- paired TuneLock/Myna oracle: 453/604 (75.0%).

The oracle is only a ceiling. It says the pair contains enough complementary
correct answers to make the stretch target possible; it does not provide a
deployable way to choose those answers. No learned model is integrated into the
application in this checkpoint.

### Three-model selector checkpoint

The deployment-shaped v6 head evaluated with faithful native-equivalent views
reaches 424/604 (70.2%). A separately locked 384 -> 512 -> 24 faithful compact
head reaches 408/604 (67.5%) from a 1,681,279-byte checkpoint. Classical +
compact + deployment-shaped v6 raise the GiantSteps exact oracle to 462/604
(76.5%), but this remains hindsight evidence.

`train_three_model_selector.py` consumes one static classical source and two
five-shard neural OOF sources. It verifies each neural track's fixed
artist/recording-group fold, trains with nested MTG-only CV, and never reads
GiantSteps labels. The first candidate ranker reaches 864/1,340 (64.5%) nested
OOF and 410/604 (67.9%) on GiantSteps. Pitch-view stability plus classical
section evidence leaves exact accuracy unchanged; a direct model gate reaches
869/1,340 (64.9%) OOF but 408/604 (67.5%) on GiantSteps. These are locked
negative results. Do not tune another selector over the same three global
posterior streams; add a new information-bearing section/temporal signal first.

That section signal has now been tested in two leakage-controlled forms.
`evaluate_myna_temporal_pooling.py` selects one of seventeen auditable robust
pooling rules on MTG fold 0 and freezes it before development evaluation. The
selected 30% trimmed-logit rule improved MTG 169/266 versus 165/266, then fell
to 417/604 versus the 424/604 mean on GiantSteps. It is rejected.

`train_temporal_candidate_ranker.py` extracts 72 transposition-invariant
candidate statistics from nineteen ordered sections and thirteen aligned pitch
views. Its training inputs come from all five OOF neural heads; resumable caches
are contract-bound to the checkpoint, manifest, view cache, record order and
feature vocabulary. The tiny shared linear ranker improved MTG fold 0 to
168/266 but reached only 422/604 (69.9%) on GiantSteps. Its classical pair
oracle is 455/604 (75.3%), which records modest new diversity but no deployable
gain. Do not tune either temporal rule on GiantSteps. The next model must add a
different harmonic representation or truly beat/bar-aligned evidence.

The pinned 85M-parameter hybrid Myna was also evaluated. Head shapes and hybrid
branches were selected only on fixed MTG validation fold 0. Its best vertical-
branch candidate reaches 378/604 (62.6%), MIREX 0.701, top-3 83.8%, with a
439/604 (72.7%) pair oracle. It is a recorded negative result and will not
replace the smaller 70.5% candidate.

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
  ablation. `phase-vocoder-sparse-v1` is the faithful training path: it shares
  the STFT and stores only the non-negligible support of torchaudio's sinc/Hann
  resampler instead of constructing enormous dense kernels for coprime rates.
  It is a distinct cache identity. The completed v7 experiment is recorded
  below as a rejected accuracy path and a retained diversity study.
- Generated embeddings, checkpoints, posteriors, and external model checkouts
  remain gitignored research artifacts.
- Resumable caches are metadata-bound. A changed manifest, model/revision,
  sample window, embedding width, pitch method, role, or shift set must use a
  new cache directory; stale same-shaped embeddings are rejected.

Validate sparse-v1 against all twelve committed torchaudio 2.7.1 views before
extracting a cache:

```powershell
ml\venv\Scripts\python.exe ml\keybench\validate_sparse_sinc_resampler.py
```

The pinned CPU check currently measures 1.19e-7 maximum and 8.66e-9 mean
absolute waveform error. On the first shared training record, all twelve Myna
embedding tensors agreed with the older dense cache within 8.35e-7 maximum and
4.99e-8 mean absolute error. Generate the isolated, resumable training cache:

```powershell
ml\venv\Scripts\python.exe ml\keybench\extract_myna_pitch_embeddings.py `
  --manifest ml\data\keybench\key-corpus-manifest-v4-all-unambiguous.json `
  --cache-dir ml\data\keybench\myna-pitch-phase-sparse-v1-embeddings `
  --hf-cache ml\data\huggingface-cache `
  --pitch-method phase-vocoder-sparse-v1 `
  --role training --model-batch-size 32 --device cuda
```

Never point sparse-v1 at a `phase-vocoder-cached` directory. The metadata guard
rejects that mismatch; keeping the identities separate makes numerical drift
and partial-cache provenance auditable.

The head trainer rejects pitch caches unless every expected view is present,
the failure list is empty, the role is `training`, and the manifest/backbone
identity matches the base cache. TTA applies the same completeness checks and
also requires its pitch method to match the head's recorded training method.
Cross-method comparisons require the explicit
`--allow-training-pitch-method-mismatch` ablation flag.

Once metadata reports `complete == expected` with no failures, apply the
already-locked v6 head shape and optimizer to the new augmentation. First run
the validation-only gate; it never reads GiantSteps development embeddings:

```powershell
ml\venv\Scripts\python.exe ml\keybench\train_myna_head.py `
  --manifest ml\data\keybench\key-corpus-manifest-v4-all-unambiguous.json `
  --embedding-cache ml\data\keybench\myna-embeddings `
  --pitch-augmentation-cache ml\data\keybench\myna-pitch-phase-sparse-v1-embeddings `
  --report ml\data\keybench\myna-mtg-v7-faithful-validation-report.json `
  --checkpoint ml\data\models\myna-mtg-v7-faithful-validation.pt `
  --validation-only --validation-fold 0 --seeds 42 --epochs 100 --patience 15 `
  --batch-size 512 --learning-rate 0.0003 --weight-decay 0.0001 `
  --hidden-dims 4096 4096 --dropout 0.99 --amp --device cuda
```

The completed validation-only run selected epoch 2 at 172/266 (64.7%), versus
170/266 (63.9%) for v6. The deterministic final rerun reproduced that result.
Its base-view Rust bakeoff reached 406/604 (67.2%), 0.741 MIREX, 84.9% top-3,
and a 446/604 (73.8%) TuneLock pair oracle. This is below v6 base (415/604) and
below the 453-track oracle requirement.

Only after that result is recorded, rerun without `--validation-only` using
fresh `myna-mtg-v7-faithful` output/report/checkpoint paths. That second run
selects epochs on the same disjoint validation fold, retrains on all MTG
training records, and emits the 604 development posteriors for Rust scoring.

```powershell
ml\venv\Scripts\python.exe ml\keybench\train_myna_head.py `
  --manifest ml\data\keybench\key-corpus-manifest-v4-all-unambiguous.json `
  --embedding-cache ml\data\keybench\myna-embeddings `
  --pitch-augmentation-cache ml\data\keybench\myna-pitch-phase-sparse-v1-embeddings `
  --output ml\data\keybench\myna-mtg-v7-faithful.jsonl `
  --report ml\data\keybench\myna-mtg-v7-faithful-report.json `
  --checkpoint ml\data\models\myna-mtg-v7-faithful.pt `
  --validation-fold 0 --seeds 42 --epochs 100 --patience 15 `
  --batch-size 512 --learning-rate 0.0003 --weight-decay 0.0001 `
  --hidden-dims 4096 4096 --dropout 0.99 --amp --device cuda
```

The separate development cache completed at 7,248/7,248 shifted embeddings
with zero failures. Probability TTA remained 406/604 (67.2%); logit TTA reached
407/604 (67.4%). Their TuneLock pair oracles were only 448 and 449 tracks, so
v7 is rejected for standalone production and ordinary fusion. Adding v7 to the
classical and v6 TTA candidates raises the exact oracle to 459/604 (76.0%), but
the best measured OOF convex blend is 426/604 (70.5%) and generally gives v7
zero weight. Retain the artifacts for leakage-safe selector research; do not
replace the 426/604 v6 probability candidate or its 428/604 fixed-blend result.

## Leakage-safe head selection

Use `--validation-only` while comparing head families. This writes validation
states and a report but does not load development embeddings, retrain on all
MTG records, or emit GiantSteps posteriors. After locking one configuration,
rerun the same command without `--validation-only` and add `--output`.

Hybrid embeddings can be audited with `--embedding-view full`, `first-half`, or
`second-half`. The source dimension, selected bounds, and head input dimension
are stored in every new checkpoint, consumed by TTA, and preserved in ONNX
artifacts.

## ONNX artifact and Rust runtime smoke

Export one pinned backbone + final head. Generated weights and manifests stay
below ignored `ml/data` paths:

```powershell
ml\venv\Scripts\python.exe ml\keybench\export_myna_onnx.py `
  --manifest ml\data\keybench\key-corpus-manifest-v4-all-unambiguous.json `
  --checkpoint ml\data\models\myna-mtg-v6-validation-state.pt `
  --hf-cache ml\data\huggingface-cache `
  --output-dir ml\data\models\myna-v6-onnx-v4
```

The exporter checks ONNX structure and CPU numerical/argmax parity before it
atomically publishes the directory. Schema 2 pins the nnAudio 0.3.3 mel
parameters; schema 3 additionally pins the torchaudio 2.7.1 decode/downmix/
resample reference contract; schema 4 pins all twelve phase-vocoder views,
harmony alignment, and probability averaging in machine-readable form. The
Rust probe validates the manifest, size, SHA-256, harmony and preprocessing
contracts, dynamically loads a user-supplied ONNX Runtime library, and executes
deterministic 16 kHz audio, a real file, or the full TTA path on a dedicated
background worker:

```powershell
cd src-tauri
C:\Users\louis.media\.cargo\bin\cargo.exe run `
  --features neural-key --bin tunelock-neural-key-probe -- `
  ..\ml\data\models\myna-v6-onnx-v4 `
  ..\ml\venv\Lib\site-packages\onnxruntime\capi\onnxruntime.dll `
  ..\ground-truth\giantsteps-key\audio\1004923.LOFI.mp3 --tta
```

The default application feature set does not compile the runtime adapter and
does not download or bundle ONNX Runtime or model weights. Native PCM-to-mel
parity is now fixture-tested against the pinned Python implementation (maximum
scaled relative error 3.56e-6). A second committed fixture covers stereo PCM16
decode, float32 channel averaging, amplitude preservation, and 44.1 -> 16 kHz
Hann sinc resampling; Rust differs from torchaudio by at most 4.92e-6 per
sample (mean 4.83e-7).

Audit real-file inference against cached Python posteriors without calling it
an accuracy score:

```powershell
cd src-tauri
C:\Users\louis.media\.cargo\bin\cargo.exe run --release `
  --features neural-key --bin tunelock-neural-key-parity -- `
  ..\ml\data\models\myna-v6-onnx-v4 `
  ..\ml\venv\Lib\site-packages\onnxruntime\capi\onnxruntime.dll `
  ..\ml\data\keybench\key-corpus-manifest-v4-all-unambiguous.json `
  ..\ml\data\keybench\myna-mtg-v6-validation-state.jsonl .. 20
```

The pinned base-view 20-file release audit had 20/20 top-1 agreement, no failures,
0.000632 mean absolute posterior error, 0.0167 maximum posterior error, and
866 ms mean per-track execution on this development machine.

`export_myna_pitch_fixture.py` pins the complete 12-view torchaudio reference.
Rust's phase-vocoder audio differs by at most 0.000679 per sample across all
views (global mean 0.0000558). `probe_myna_onnx_file.py` provides a real-file
Python reference for the complete TTA path. On the pinned smoke MP3, Rust and
Python chose the same C-minor top key; their final posteriors differed by
0.00131 mean absolute and 0.00606 maximum error. Native release TTA took 16.2
seconds including artifact/runtime load on this machine, so it remains an
asynchronous research path. The current graph's head was trained with the old
pitch+speed ablation: schema 4 proves faithful runtime execution, but a new
accuracy result requires completion of the faithful cache and retraining.

A validation-only latency sweep compared symmetric view sets on the existing
pitch+speed ablation. Base, six near views (±1/±2/±3), and all twelve scored
170/266, 171/266, and 169/266 respectively. The validation-locked six-view set
then reached 422/604 (69.9%) on GiantSteps versus 426/604 (70.5%) for all
twelve. TuneLock therefore retains the twelve-view accuracy-first research
contract; the six-view option is a measured latency tradeoff, not a promotion.
