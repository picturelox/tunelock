# Contributing to TuneLock accuracy

The highest-value contribution is not another opinion scraped from the web. It
is a carefully blinded label on a legally usable or privately evaluated audio
recording, with enough independent review to know whether the label is sound.

## What the owner needs to do now

1. **Choose the data policy.** Decide whether model training will use only
   CC0/CC-BY and contributor-owned audio (recommended), or whether other
   licensed sources will be considered after legal review. A public label does
   not grant permission to train on its audio.
2. **Recruit a pilot panel.** Start with 5-10 experienced DJs, musicians, or
   engineers. Ask each person for 30-50 blind judgments; overlap at least 20%
   of assignments so agreement can be measured.
3. **Build a sealed final holdout.** Target 300-500 diverse recordings. Keep
   artists, remixes, edits, stems, duplicate previews, and recording families
   together in one split. Do not reveal its aggregate score until the model and
   fusion rule are frozen.
4. **Annotate, then repeat.** Use TuneLock's Gold Set view with the engine result
   hidden. Re-label 10-20% after at least two weeks. Aim for at least two strong
   independent agreements per accepted track and expert arbitration when they
   conflict.
5. **Cover the failure domains.** Include bass-heavy music, sparse intros,
   modal/ambiguous tracks, live music, key changes, short edits, and genres
   unlike GiantSteps. A comfortable EDM-only set will overstate general use.
6. **Preserve evidence.** Record tonic, mode, ambiguity/atonality, modulation,
   confidence, useful timestamps, method used, and an optional note. Never copy
   the answer from TuneLock, Mixed In Key, Beatport, Spotify, an LLM, or a tag.

## A clever crowdsourcing design

Use two contribution lanes, because privacy/evaluation and model-training
rights are different problems.

### Lane A: private library evaluation

- Audio stays on the contributor's computer.
- TuneLock creates a blind task from tracks where engines disagree, are
  uncertain, or lack genre coverage.
- The contributor submits the label, confidence, evidence, anonymous
  contributor ID, engine revision, and quality-check results—not the audio.
- This lane measures real-world failure rates and calibration. It does **not**
  automatically authorize centralized model training, because the service does
  not possess rights-cleared audio paired with the label.

### Lane B: rights-cleared training corpus

- Accept only contributor-owned audio with an explicit grant, or material whose
  license is verified as suitable for the intended commercial training and
  distribution. Preserve license URL, author, version, and proof at ingestion.
- Assign each recording family to train, validation, or sealed test before
  labels are examined. Never move a test family into training later.
- Give each item to at least three blind annotators when possible. Promote it to
  training only after consensus and automated quality gates pass.
- Publish the label set under an explicit license; keep audio distribution rules
  separate because they may differ per recording.

This separation lets a large DJ community help immediately without pretending
that a label submission also transfers audio rights.

## Spend human effort where it teaches the engine most

An active-learning queue should prioritize:

1. TuneLock and the learned model choose different top keys;
2. their top candidates are close or high-entropy;
3. transposing the audio by a known amount does not rotate predictions by the
   same amount;
4. a genre, production era, format, or acoustic condition is underrepresented;
5. a track resembles a known high-cost error such as parallel, relative,
   fifth, semitone, or unstable-key confusion.

Do not reward raw annotation volume. Reward agreement on difficult tasks,
consistency on hidden repeats, useful evidence, and successful adjudication.
This makes gaming the system less attractive and directs expert time to labels
that can actually change a model decision.

## Quality and anti-bias gates

- Keep every task blind to engine, vendor, web, and previous contributor labels.
- Insert hidden repeats and a small bank of calibration tracks.
- Add synthetic pitch-shift checks: a trusted label shifted by `n` semitones
  must shift by exactly `n`; this catches UI, vocabulary, and careless-label
  errors cheaply.
- Estimate contributor reliability from repeat consistency and agreement on
  adjudicated tasks. Use a probabilistic consensus method such as Dawid-Skene
  only as triage; a model-weighted vote is not a substitute for expert review.
- Quarantine suspicious batches, correlated accounts, implausibly fast work,
  and copied vendor labels. Keep an immutable audit trail rather than deleting
  inconvenient judgments.
- Store `ambiguous`, `atonal`, and `modulates` as valid outcomes. Forcing every
  recording into one of 24 keys poisons both training and evaluation.

## What can be shared safely

A contribution capsule should contain a versioned schema, pseudonymous
contributor ID, task ID, recording-family token, label, confidence, timestamps,
blind flag, evidence, repeat/calibration status, app/model revisions, and the
audio-rights class. It should exclude paths, tags, library history, and raw
audio unless the contributor explicitly uses Lane B.

A stable cross-user recording token is useful for duplicate control but can
be identifying. Start with per-install salted tokens for private metrics. Only
introduce cross-user matching after a privacy review—ideally using a blinded
matching protocol—rather than uploading raw acoustic fingerprints by default.

## Where OSINT and LLMs do help

They can find candidate public corpora, locate license records, normalize
artist/title metadata, surface likely duplicate releases, translate annotation
instructions, and summarize adjudicator notes. They cannot establish the heard
key, resolve a disputed label, prove audio rights, or become truth labels.

## Acceptance definition

A candidate release is promoted only when its data manifest, split audit,
artifact hash, model/data licenses, exact/MIREX/top-k metrics, calibration,
latency, and paired failures are reproducible. The 75% development target is a
checkpoint; the sealed holdout decides whether the improvement is real.
