# TuneLock core intelligence, simply explained

TuneLock currently has two different things that must not be confused:

1. **The production engine** is the local Rust analyzer. It returns key and BPM
   without a network connection or model download. On the 604-track
   GiantSteps-key development benchmark, its global-key result is **389/604
   (64.4%)**.
2. **The strongest research candidate** adds a learned Myna audio embedding and
   a TuneLock-trained key head. It reaches **425/604 (70.4%)** by itself. A
   fixed equal-posterior blend with the production engine reaches **428/604
   (70.9%)**. It is not in the product yet.
3. **The production bridge now works in isolation.** The candidate can be
   exported as a checksum-bound ONNX artifact, loaded by opt-in Rust code through
   an external ONNX Runtime, and evaluated off the immediate result path. This
   is plumbing, not a claim that the model is ready to ship.

The pair has a **453/604 (75.0%) oracle**: on exactly 75% of the tracks, at
least one of the two engines is right. An oracle knows the answer in advance;
software does not. This number proves the two engines contain enough
complementary answers to make 75% possible, but it is not a deployable score.

## Latest checkpoint, in plain language

- We tested a much larger 85M-parameter Myna model. Its best locked branch
  scored **378/604 (62.6%)**, below both the production engine and the smaller
  70.4% research candidate. Its pair oracle was only **439/604 (72.7%)**, so it
  cannot reach the 75% target by choosing between its answer and TuneLock's.
  It was rejected instead of being promoted because it is newer or larger.
- We made model selection safer. Head variants can now run in
  `validation-only` mode, so GiantSteps audio/posteriors are not loaded until a
  configuration is locked on the artist-disjoint MTG validation fold.
- We exported the current candidate to a single **162,782,585-byte ONNX graph**.
  ONNX Runtime reproduced its PyTorch logits within **1.02e-6** and chose the
  same keys in the parity batch. This graph is the base acoustic view; the
  pitch-transposition/TTA orchestration that raises the research score to 70.4%
  is still outside the Rust runtime.
- Rust now verifies the artifact contract, file size, SHA-256, input/output
  shapes, canonical key order, aggregation rule, and finite outputs. A real
  external-runtime smoke produced a posterior summing to approximately 1.0.
- Nothing changed the user-visible release result. The model feature remains
  off by default; no model, downloader, or runtime binary is bundled.

The next accuracy work is not "use an even bigger model." It is to finish the
faithful pitch-only augmentation experiment, add rights-cleared diverse audio,
learn/calibrate fusion outside GiantSteps, reproduce gains on a sealed human
holdout, and match the model's mel preprocessing exactly in Rust.

## What happens to an audio file

The intended production flow is staged:

1. Decode the file locally and show the classical key/BPM result immediately.
2. In the background, run the learned acoustic model and produce all 24
   major/minor probabilities.
3. Apply a small selector or calibrated fusion rule trained without access to
   the evaluation answers.
4. Update the display only when the background result is ready, preserving the
original local result and its evidence.
5. Use overlapping windows for key-over-time and compare outgoing/incoming
   regions when ranking relationships to other tracks.

No web lookup or LLM belongs in steps 1-3. Web metadata can identify a release
or add context, but it cannot hear the audio and must never silently replace an
acoustic result or a human-adjudicated truth label.

The code currently implements the artifact/runtime boundary for step 2, behind
an opt-in build feature. It does not yet implement the production mel
preprocessor, background job lifecycle, calibrated step 3, or UI update in step
4. It also does not yet reproduce the winning multi-pitch TTA path. Those
omissions are deliberate promotion gates, not hidden completion.

## What the 75% target means

The target is **exact global key**, including mode, on GiantSteps-key. That set
has already influenced development, so it is a repeatable development board,
not an untouched final test. A model is production-eligible only after:

- its complete artifact and inference path are reproducible;
- its improvement repeats on a new sealed, artist/recording-family-disjoint
  human holdout;
- confidence calibration, latency, package size, failures, and commercial
  rights pass review; and
- the local classical readout remains first and available offline.

For the exact experimental ledger, see [ACCURACY.md](ACCURACY.md). For the work
TuneLock needs from its owner and contributors, see
[ACCURACY_CONTRIBUTING.md](ACCURACY_CONTRIBUTING.md).
