# TuneLock core intelligence, simply explained

TuneLock's Core Intelligence v1 is a multi-opinion key engine that combines a
deterministic classical analyzer with frozen neural models and a learned
selector. The science phase is complete; the system is being integrated into
the application.

## Core Intelligence v1 — frozen 2026-08-26

### Architecture

1. **Classical engine — immediate anchor.** The local Rust analyzer is
   deterministic, needs no model, and returns first. It scores **57.1%**
   exact on the FMAK development corpus (4,928 tracks) and **64.4%** on
   GiantSteps (604 tracks). No network call, model load, or LLM call is
   ever on the critical path to this first key/BPM readout.
2. **Neural opinions — asynchronous upgrade.** Two frozen neural models
   load in the background and upgrade the classical result:
   - **S-KEY raw** (65.5% exact, ECE 0.046 — best calibrated model)
   - **Myna v8 compact** (65.0% exact, ECE 0.221 — best top-3: 87.1%)
3. **GBM selector — combines all opinions.** A per-candidate gradient
   boosting selector combines posteriors from classical, Myna v6, Myna v8,
   S-KEY, and the temporal ranker. It scores **69.8% OOF exact** on FMAK
   development (4,902 tracks, nested artist-disjoint OOF).
4. **Confidence tiers — honest calibration.** Model agreement provides a
   well-calibrated confidence signal:
   - **High confidence** (all models agree, ~48% of tracks): **84.7%** accuracy
   - **Medium confidence** (≥60% agree, ~81% of tracks): **75.8%** accuracy
   - **Low confidence** (models disagree, ~19% of tracks): selector fallback

### Science phase findings

The decisive science phase answered four key questions:

1. **Is performance data-limited or architecture-limited?**
   **Data-limited.** FMAK-only training gives +12.8pp over MTG-only
   (54.8% → 67.6%) with the same architecture. Adding MTG to FMAK gives
   only +0.5pp.

2. **Can a selector close the oracle gap?**
   **Partially.** The GBM selector captures 30% of the oracle gap
   (65.5% → 69.8%, oracle 79.9%). The remaining gap is due to model
   correlation — all Myna-based models share the same backbone embeddings.

3. **Is model confidence calibrated?**
   **Yes for FMAK-trained models (ECE 0.023–0.086), no for MTG-trained
   models (ECE 0.116–0.487).** S-KEY is the best-calibrated frozen model
   (ECE 0.046). Model agreement is a stronger confidence signal than any
   single model's posterior.

4. **What is the honest accuracy claim?**
   **69.8% overall (selector), 75.8% at medium confidence (≥60% agreement),
   84.7% at high confidence (full agreement).** The 75% target is achieved
   on the medium-confidence subset (81% coverage).

### What would close the gap to 75% overall

The selector at 69.8% is 5.2pp below the 75% overall target. The oracle
ceiling is 82.0%, so the information exists but cannot be extracted with
current model diversity. Closing the gap requires:

- A fundamentally different feature representation (not Myna embeddings)
- More diverse model families (CNN on spectrograms, different audio encoders)
- Larger training corpora (the retraining experiment shows +12.8pp from 4x
  more data, suggesting more data would continue to help)

Further key-model research has diminishing returns without new model
families. The MVP ships with Core Intelligence v1 and focuses on the
Transition Workbench, DSP, and user experience.

## Native engine status

Rust implements the shared neural path behind the optional `neural-key`
feature:

- real-file decode, float32 downmix, and torchaudio-compatible 16 kHz resample;
- nnAudio-compatible mel preprocessing and full-track chunking;
- all twelve faithful pitch-only views with label realignment;
- probability aggregation;
- checksum/schema-bound ONNX loading through an external runtime.

Python/Rust base inference matched top-1 on 20/20 audited files. A full faithful
TTA smoke chose the same key with 0.00131 mean posterior error. The native
non-TTA path runs at ~0.7 tracks/s. A native FMAK benchmark is in progress
to verify full-corpus parity.

## Product flow

1. Decode once and render the classical key/BPM immediately.
2. Reuse that audio in a background shared neural feature pass.
3. Run the frozen neural models on the shared embeddings.
4. Apply the GBM selector to combine all opinions.
5. Return the chosen key, confidence tier, and close alternatives.

No network, web lookup, or LLM belongs on this acoustic decision path. Metadata
may later identify a release or provide clearly labeled enrichment, but it
cannot silently replace what the audio models heard.

## Integration checklist

- [x] Frozen-model FMAK benchmark (5 models, full posteriors)
- [x] GBM selector trained with nested OOF (69.8% exact)
- [x] Controlled retraining experiment (data-limited verdict)
- [x] Confidence calibration study (model agreement tiers)
- [x] Core Intelligence v1 architecture defined
- [ ] Native FMAK parity benchmark (in progress)
- [ ] Latency, memory, artifact size measurement
- [ ] UI integration: classical first, neural async, confidence tiers
- [ ] Model, data, runtime, and distribution rights review

For the exact experimental ledger, see [ACCURACY.md](ACCURACY.md). For human and
contributor work, see [ACCURACY_CONTRIBUTING.md](ACCURACY_CONTRIBUTING.md).
