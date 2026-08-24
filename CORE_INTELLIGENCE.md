# TuneLock core intelligence, simply explained

TuneLock is building a three-opinion key engine on `core-intelligence`. It is
still research code and is not enabled in the application.

## What exists now

1. **Classical engine — immediate anchor.** The current local Rust analyzer is
   deterministic, needs no model, and returns first. It scores **389/604
   (64.4%)** exact and 0.725 MIREX on the GiantSteps-key development benchmark.
2. **Neural accuracy head — strongest opinion.** The Myna v6 head evaluated
   with the faithful pitch-only views that native Rust can generate scores
   **424/604 (70.2%)**, 0.762 MIREX, and 85.4% top-3. The older fast
   pitch-and-speed research ablation scores 426/604 (70.5%), but 70.2% is the
   relevant deployment-shaped result.
3. **Compact diversity head — cheap second neural opinion.** A newly locked
   384 -> 512 -> 24 faithful-training head scores **408/604 (67.5%)**, 0.746
   MIREX, and 86.4% top-3. Its PyTorch checkpoint is 1,681,279 bytes versus
   roughly 147.7 MB for the previous wide faithful checkpoint.

The compact head does not add another audio pipeline. Both neural heads use the
same decoded audio, pitch views, mel features, and pinned Myna backbone. The
backbone runs once per view; the embeddings then pass through two heads. A tiny
candidate selector combines their 24-key posteriors with the classical
posterior.

## Why the third model matters

On GiantSteps, the classical engine plus the compact head have a 447/604
(74.0%) exact oracle. Adding the 70.2% accuracy head raises the three-model
oracle to **462/604 (76.5%)**. In other words, at least one of the three is
correct on 76.5% of these tracks.

That is not a deployable 76.5% result. An oracle knows the answer after the
fact. The measured ordinary three-way out-of-fold convex blend is only 69.4%,
below the accuracy head alone. The next problem is learning when each opinion
is trustworthy, not averaging them indiscriminately.

## The selector being built

The selector treats each of the 24 keys as a candidate and uses only
transposition-invariant evidence:

- each model's posterior, rank, top-1/top-3/top-5 status, margin, and entropy;
- whether model winners agree;
- pairwise posterior distance and candidate-score disagreement;
- aggregate support for the candidate across all three models.

It does not use a key name, candidate index, GiantSteps answer, genre lookup,
web result, or LLM assertion. The exported selector is a scaled logistic model:
small, deterministic, auditable, and cheap enough to run after every analysis.

Its training contract is strict:

- all selector fitting uses the 1,340 exact-audio-deduplicated MTG records;
- the five folds are disjoint by artist/recording connected component;
- every neural training posterior must come from a head trained without that
  track, and must carry the matching fold marker;
- selector evaluation is nested out-of-fold;
- GiantSteps predictions are generated only after the selector is locked and
  GiantSteps labels are never read by the Python trainer.

All selector inputs are now complete. The first candidate ranker reaches
864/1,340 (64.5%) nested OOF on MTG and 410/604 (67.9%) on GiantSteps. Adding
pitch-view stability and classical section evidence leaves exact accuracy
unchanged. A direct hard model gate reaches 869/1,340 (64.9%) on MTG but only
408/604 (67.5%) on GiantSteps. All three are rejected for promotion; the 70.2%
accuracy head remains the best deployment-shaped result.

A genuinely independent fourth opinion was then tried: a small supervised head
on the pinned S-KEY ChromaNet harmonic map, trained under the same MTG-only
contract. It scores 395/604 (65.4%) on GiantSteps—too weak alone, but its
errors are different, and the five-opinion oracle (classical, both Myna heads,
the temporal ranker, and this harmonic head) is **475/604 (78.6%)**. A fourth
selector over classical + accuracy head + harmonic head reached only 826/1,340
(61.6%) nested OOF and 410/604 (67.9%) on GiantSteps, and is also rejected.

The pattern is now conclusive: the oracle keeps rising while every
leakage-safe selector stays below the single accuracy head. The bottleneck is
the 1,340-track adjudicated training corpus, not candidate diversity or
selection features. Selector research over track-global posteriors is parked
until broader adjudicated key-labeled training data is acquired; repeatedly
changing the selector on these same posteriors remains a stop condition
because it would invite development-set tuning without adding information.

## Native engine status

Rust already implements the difficult shared neural path behind the optional
`neural-key` feature:

- real-file decode, float32 downmix, and torchaudio-compatible 16 kHz resample;
- nnAudio-compatible mel preprocessing and full-track chunking;
- all twelve faithful pitch-only views with label realignment;
- probability aggregation;
- checksum/schema-bound ONNX loading through an external runtime.

Python/Rust base inference matched top-1 on 20/20 audited files. A full faithful
TTA smoke chose the same key with 0.00131 mean posterior error. The current cold
all-view path took about 16.2 seconds, so model reuse, caching, and view/runtime
optimization remain required before product promotion.

## Intended eventual product flow

1. Decode once and render the classical key/BPM immediately.
2. Reuse that audio in a background shared neural feature pass.
3. Run the accuracy and compact diversity heads on the shared embeddings.
4. Apply the frozen selector and calibrator.
5. Return the chosen key, qualitative evidence tier, and close alternatives.

No network, web lookup, or LLM belongs on this acoustic decision path. Metadata
may later identify a release or provide clearly labeled enrichment, but it
cannot silently replace what the audio models heard.

## What must be true before integration

- A future selector must beat the 70.2% deployment-shaped neural
  result under the MTG-only selection contract. The four locked
  selectors did not, and further attempts require broader adjudicated
  training data first.
- The sealed final holdout's labels are sourced from existing labeled corpora
  or paid annotation, not an in-house panel.
- Latency engineering on the native neural path is in scope now; calibration,
  packaging, and UI integration remain gated.
- The complete native artifact path must reproduce the locked result over all
  604 development files, not only parity samples.
- Latency, memory, artifact size, cancellation, caching, and failures must be
  measured.
- Confidence tiers and alternatives must be calibrated on held-out data.
- Improvement must repeat on a new sealed, artist/recording-family-disjoint
  human-adjudicated holdout.
- Model, data, runtime, and distribution rights must pass review.

For the exact experimental ledger, see [ACCURACY.md](ACCURACY.md). For human and
contributor work, see [ACCURACY_CONTRIBUTING.md](ACCURACY_CONTRIBUTING.md).
