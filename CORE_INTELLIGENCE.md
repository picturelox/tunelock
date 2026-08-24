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

The pair has a **453/604 (75.0%) oracle**: on exactly 75% of the tracks, at
least one of the two engines is right. An oracle knows the answer in advance;
software does not. This number proves the two engines contain enough
complementary answers to make 75% possible, but it is not a deployable score.

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
