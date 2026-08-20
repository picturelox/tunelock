# TuneLock CNN — Key Detection with Deep Learning

This is a standalone Python project for training a CNN-based key detector,
exported to ONNX and integrated into the TuneLock Rust app via the `ort` crate.

## Architecture

Following Korzeniowski & Widmer (EUSIPCO 2017, ISMIR 2018):

- **Input**: CQT (constant-Q transform) or Mel spectrogram, 80 mel bins × 252 frames
- **Model**: 4 conv layers (2D) → 2 dense layers → 24-way softmax (12 major + 12 minor)
- **Output**: key probability distribution over 24 keys
- **Size**: ~2-5 MB per model (INT8 quantized)

## Features

Three feature types are extracted and trained as separate models:
1. **CQT** — constant-Q transform, log-spaced frequency
2. **Mel** — mel spectrogram, perceptual frequency scale
3. **HPCP** — harmonic pitch class profile (chroma-like)

## Training data

- **GiantSteps** (~600 tracks with key labels) — downloaded by `ground-truth/download-giantsteps.ps1`
- **GiantSteps-MTG** (~1,500 tracks) — extended version
- **Adjudicated gold** — from TuneLock's adjudication queue (Phase 9)
- **Genre-stratified samples** — from the user's 19.5k library

## Usage

```bash
# Setup
pip install -r requirements.txt

# Extract features
python extract_features.py --audio_dir ../ground-truth/giantsteps/audio --output data/features.h5

# Train
python train.py --features data/features.h5 --model cqt --epochs 50 --output models/cqt_model.onnx

# Quantize
python quantize.py --model models/cqt_model.onnx --output models/cqt_model_int8.onnx

# Evaluate
python evaluate.py --model models/cqt_model_int8.onnx --test_set data/test.h5
```

## Integration

The trained ONNX models are loaded by the Rust app's `engine/key_cnn.rs`
module via the `ort` crate. The CNN is lazily loaded — startup stays under 2s.
The CNN's output is plugged into the existing ensemble as a weighted vote
alongside Krumhansl, Temperley, and Sha'ath.

## Decision criterion

The CNN is kept only if it beats the classical-only path on the user's corpus
by at least +3 points exact-match. Otherwise it is cut — 25 MB of model
weight is not justified by a marginal improvement.

## Status

**EXPERIMENT INVALID — DEFERRED (Step 7 fixes applied, pending re-run)**

The prior CNN experiment had three critical bugs that invalidated its
results:

1. **Windowing bug**: `librosa.load(duration=30.0)` truncated audio to
   the first 30 seconds. The centering code was dead. Fixed: now loads
   full audio and centers properly.

2. **Augmentation bug**: `np.roll(d, s, axis=0)` rolled the batch
   dimension (length N), not the time axis. Augmentation produced
   identical duplicates. Fixed: now rolls axis=2 (time frames) and
   adds pitch-shift augmentation (Korzeniowski protocol).

3. **Epoch selection bias**: Best epoch was selected on the same
   validation fold used for reporting. Fixed: now splits training into
   train + internal validation (90/10), selects epoch on internal
   validation only, and evaluates on the external fold once.

The prior 29.6% result is NOT a valid negative result — it was produced
by a broken experiment. The CNN must be re-run with the fixes before
any conclusion can be drawn.

## Corrected experiment protocol (Step 7)

1. Download GiantSteps-MTG from Zenodo (DOI 10.5281/zenodo.1101082)
   using `ground-truth/download-giantsteps-mtg.ps1`.
2. Extract features from MTG (training) and GiantSteps (test) separately.
3. Train on MTG with corrected augmentation (time-shift + pitch-shift).
4. Use leakage-safe artist-aware splits for MTG train/val.
5. Evaluate once on the untouched GiantSteps 604-track test set.
6. Compare paired predictions against the classical engine.
7. Keep CNN only if it beats classical by +3 points on GiantSteps.

## Prior (invalid) results — kept for reference only

| Model | Parameters | Accuracy | Notes |
|---|---|---|---|
| Full KeyCNN (424K) | 424,792 | 26.4% | INVALID — overfitting + windowing bug |
| SmallKeyCNN (26K) | 26,392 | 29.6% ± 2.3% | INVALID — no-op augmentation + epoch bias |
| Classical engine | N/A | 64.4% | Valid — current TuneLock ensemble |

**Do not cite the CNN results above. They were produced by a broken
experiment. The corrected experiment has not yet been run.**
