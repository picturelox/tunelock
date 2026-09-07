# TuneLock product contract

Status: canonical product scope as of 2026-09-06

Baseline: `performance-build` at `e07d936b6a9bcd560edb4928b83dd242157818ce`

TuneLock is a native DJ instrument for discovering relationships in a music
library, preparing transitions, performing a set, and recording the result.
Its differentiator is that musical intelligence leads to an action: audition a
candidate, load it, inspect or correct its analysis, shape a transition, and
perform it without leaving the live session.

This document supersedes earlier analyzer-only, preparation-only, and
"ultimate mix planner" product definitions. Those documents remain useful
research and design history. They are not the active release contract.

## First usable release

The first release is complete only when a DJ can mix and record a complete set
at home with Decks A and B on Windows and macOS using both mouse/keyboard and a
Traktor Kontrol S3.

It includes:

- persistent two-deck waveforms, transport, mixer, recording status, and a
  docked library;
- load and immediate playback without waiting for analysis;
- trim, channel fader, crossfader assignment, three-band EQ and isolator kills,
  filters, delay, and reverb;
- hot cues, loops, tempo control, pitch control, tempo/beat Sync, and manual
  nudging;
- occasional expressive scratches and backspins, including hold, reverse, and
  a clean release/resume path;
- independent master and private-cue stereo outputs, with cue excluded from the
  recording;
- transparent output protection, honest metering, reversible loudness matching,
  and background master recording with normal file finalization;
- candidate discovery, local key and energy timelines, uncertain-key review and
  correction with provenance, and set-progression planning;
- transitions created by recording control movements, refined on a timeline,
  saved as versioned plans, and replayed from the native engine clock.

Decks C and D are exposed only after the two-deck release gate is reliable.
Stems begin only after four-deck full-track mixing passes its reliability gate.

## Explicit exclusions

The first release does not include advanced turntablism or motor/haptic
emulation, phrase-aware Sync, automatic support for drifting-tempo recordings,
an offline mastering suite, stem separation, or a claim that every controller
is supported. The S3 is the reference controller. Generic MIDI is a reusable
input path, not a blanket compatibility promise.

## Product invariants

1. The native Rust engine is the only performance playback authority.
2. A local classical key/BPM result renders first. Network calls, model loads,
   and LLM work never block it or playback.
3. Originals are never modified, moved, or deleted. Recordings and exports are
   new derivatives.
4. The audio callback never allocates, deallocates, locks, or performs I/O.
5. Rust `harmony/` and TypeScript `lib/harmony.ts` remain the only mirrored
   harmony vocabulary and share test vectors.
6. Every frontend `invoke(...)` resolves to a registered Rust command.
7. Engine behavior is promoted only with a recorded baseline and proportionate
   deterministic, performance, and listening evidence.
8. No visible control is called functional until its engine effect and failure
   response are demonstrated.

## Authoritative boundaries

| Domain | Responsibility | Contract |
|---|---|---|
| Library and analysis | Track identity, fingerprints, analysis revisions, corrected grids, uncertainty, and provenance | Publishes immutable versioned results and never blocks playback |
| Session service | Deck A-D mapping, load generations, desired controls, acknowledged state, save/restore | One application owner shared by every workspace |
| Playback engine | Source position, clock, transport, DSP, mixer, routing, and compact telemetry | Bounded commands; no UI, database, network, or file I/O in the callback |
| Presentation | Layout, focus, visualization, accessibility, pending and error feedback | Uses the shared session adapter; never invents a second playback state |
| Recording | Master capture, bounded writer handoff, finalization, and export | Captures a documented master tap, never private cue |

The contracts distinguish `DeckId`, `PlayerId`, `SourceId`, `LoadGeneration`,
`EngineGeneration`, `AnalysisRevision`, and `GridRevision`. User-facing Deck A/B
must not be confused with the engine's current crossfader Bus A/B vocabulary.

## Routing contract to prove

The intended signal path is:

`source -> time/pitch -> trim/match -> deck EQ -> channel fader -> deck effects -> crossfader/filter assignment -> master protection -> master output + recorder tap`

Private cue has a separately specified tap, cue sum, cue/master blend, level,
and output pair. For the S3, the initial hardware hypothesis is master on 1-2
and headphones on 3-4; TuneLock must prove actual device and driver behavior on
both target operating systems before claiming support.

## Evidence and historical documents

- `ROADMAP.md` is the delivery source of truth.
- `CAPABILITIES.md` records what is reachable, partial, or missing at the pinned
  baseline.
- `STATUS.md` records the current milestone and latest reproducible checks.
- `ACCURACY.md` remains the measurement ledger.
- `CORE_INTELLIGENCE.md` remains the frozen intelligence architecture and
  integration checklist.
- `PREP/` is historical or exploratory unless a current document links to a
  specific artifact.
