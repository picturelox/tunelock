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

**TRAINED AND EVALUATED — CUT per plan's decision criterion.**

Cross-validated results on GiantSteps-key (604 tracks, 5-fold CV):

| Model | Parameters | Accuracy | Notes |
|---|---|---|---|
| Full KeyCNN (424K) | 424,792 | 26.4% | Massive overfitting (96% train, 20% val) |
| SmallKeyCNN (26K) | 26,392 | 29.6% ± 2.3% | Honest 5-fold CV result |
| Classical engine | N/A | 68.2% | TuneLock's Krumhansl/Temperley/Sha'ath ensemble |

The CNN scored less than half the classical engine's accuracy. Per the plan:
> "If it doesn't beat the classical path on your corpus, cut it."

**Root cause: insufficient training data.**
- GiantSteps-key: 604 tracks (downloaded successfully)
- GiantSteps-MTG: 1,486 tracks (download FAILED — Beatport preview URLs from 2015 are dead)
- Academic papers achieving 70-75% use ~2,100 tracks plus pretraining

**Reactivation path:**
1. Find an alternative mirror for GiantSteps-MTG audio
2. Or: use pretraining (e.g., transfer from a larger audio classification model)
3. Or: use the user's adjudicated gold labels (Phase 9 adjudication queue) as training data
4. Re-run `train_cv.py` with the larger dataset
5. If accuracy exceeds 68.2% + 3 points, proceed with ONNX export and Rust integration
