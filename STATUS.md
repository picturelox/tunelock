# TuneLock status

Updated: 2026-09-06

Current milestone: TL-02 complete in the working tree; TL-03 is next

Integration baseline: `performance-build` at
`e07d936b6a9bcd560edb4928b83dd242157818ce`

## Product destination

The first usable release is a reliable two-deck DJ performance and recording
application for Windows and macOS, controlled by mouse/keyboard and the Traktor
Kontrol S3. It includes EQ/isolator/filter/delay/reverb mixing, cues and loops,
tempo/beat Sync, nudging, occasional scratches/backspins, private cue routing,
connected musical intelligence, and record/edit/save/replay transitions.

`PRODUCT.md` is the scope contract. `ROADMAP.md` is the plan source of truth.
`CAPABILITIES.md` separates reachable behavior from engine-only, partial,
missing, and unverified work.

## TL-00 result

- Verified remote heads on 2026-09-06: `main` and `core-intelligence` remain at
  `d0cfd4d`; `performance-build` remains at `e07d936`.
- Reconciled the supplied desktop blueprint and the legacy local Devin plan.
  The supplied file labels itself provisional and contains sections 1-10; the
  user's accompanying confirmed-scope message resolves the release choices and
  is reflected in `PRODUCT.md` and `ROADMAP.md`. The reviewed file SHA-256 is
  `1CD2BCE38DDAA2E9887BAE76E734B3C1D30374D784E39D9B1583779F89A8DC3B`.
- Preserved `ACCURACY.md`, `CORE_INTELLIGENCE.md`, all source, and all prior
  research evidence unchanged.
- Marked conflicting PREP product directions as historical instead of deleting
  them.
- No application or engine code changed in TL-00.

## Fresh baseline

Run on Windows at the pinned SHA before documentation edits:

| Check | Result |
|---|---|
| `npx tsc --noEmit` | Passed |
| `npm run build` | Passed; Vite transformed 1,605 modules |
| `cargo test` | Passed: 220 library + 8 binary tests; 0 failed; 7 ignored performance tests |
| Engine/accuracy benchmark | Not rerun because TL-00 changes no engine code; existing evidence remains authoritative in `ACCURACY.md` |
| Native listening and UI walkthrough | Not run in TL-00 |
| S3 hardware | Not available/proven in TuneLock |
| macOS | Not run; no current platform evidence recorded |

The first Rust attempt from a plain PowerShell shell failed while generating
Signalsmith bindings because Windows SDK/C++ include variables were absent. It
passed from the installed Visual Studio 2022 x64 developer environment. This is
a setup prerequisite, not a TuneLock test failure.

## Verified current behavior

The mounted application is the single `Workspace` view. It analyzes files,
auto-loads analyzed Deck A, directly loads Deck B, drives a subset of the native
engine's transport/sync controls, polls meters, compares loudness, and opens a
library drawer. The underlying engine has eight player slots, bounded commands,
time/pitch processors, EQ and filters, two crossfade buses, loops, sync commands,
device selection, and metering.

This does not yet establish release behavior. The current Workspace combines
session, analysis, engine, and presentation state; A/B are asymmetric; several
handlers optimistically update local state; multichannel output carries only
the stereo master; transition plans are storage-only; and key release features
are missing or unverified as detailed in `CAPABILITIES.md`.

## Next bounded ticket

Start TL-03: add bounded command identities/results and compact engine
acknowledgements so the interface can distinguish requested, queued, applied,
and failed state. Force queue-full and engine/device failures in focused tests.
Do not combine this with transport feature expansion or visual redesign.

## TL-01 implementation result

- Added branded `DeckId`, `PlayerId`, `SourceId`, `LoadGeneration`,
  `EngineGeneration`, `AnalysisRevision`, and `GridRevision` frontend contracts.
- Added one app-root session runtime and Zustand store. Deck A-D selection,
  desired state, engine-telemetry acknowledgement, load state, and errors survive
  Workspace remounts.
- Routed A/B load and transport through the same session adapter.
- Deck A loading and local analysis now start independently; neither waits for
  the other to finish.
- Serialized Rust engine initialization and device replacement, added monotonic
  engine generations to init results and meter telemetry, and kept the retired
  buffer drain under one application owner.
- Device replacement starts the candidate stream before retiring the current
  engine and invalidates the session's loaded-state acknowledgement when its
  engine generation changes. Reload automation belongs to a later package.

Fresh verification after TL-01:

| Check | Result |
|---|---|
| `npm run build` | Passed; 1,609 modules transformed |
| `cargo test` | Passed: 222 library tests + 8 binary tests; 0 failed; 7 ignored performance tests |
| Two-deck 48 kHz/256 release harness | Passed: max 1,839 µs, average 318 µs, p99 1,624 µs; 5,333 µs budget |
| Native interaction/listening | Not run; retained for integrated hardware/UI validation |

## TL-02 implementation result

- Replaced the separate launch and pause messages with one callback-side
  `LoadPaused` command. Source attachment and `playing = false` are now one
  atomic realtime operation, so a load cannot render an intervening frame.
- Added a server-side monotonic load coordinator per player. A newer request or
  an engine-generation change prevents older decode work from installing.
- Bound asynchronous beat-grid attachment to player, source handle, load
  generation, and engine generation. Both the async boundary and callback
  reject stale completion.
- Added explicit per-player registry ownership. Accepting a replacement source
  unregisters the prior registry reference while player-held buffers continue
  through the existing deferred-destruction queues.
- Propagated frontend request generations through IPC. Session completion is
  accepted only for the matching request; legacy Listening Lab calls receive a
  wrapper-generated request generation.

Fresh verification after TL-02:

| Check | Result |
|---|---|
| `npx tsc --noEmit` | Passed |
| `npm run build` | Passed; 1,609 modules transformed |
| `cargo test` | Passed: 228 library tests + 8 binary tests; 0 failed; 7 ignored performance tests |
| Focused lifecycle regressions | Passed: silent-until-resume with an allocation-audited callback, stale-grid rejection, latest-request invalidation, IPC identity serialization, and 100 repeated replacements with one registry source |
| Two-deck 48 kHz/256 release harness | Passed: max 1,216 µs, average 268 µs, p99 1,197 µs; 5,333 µs budget |
| Native interaction/listening | Not run; retained for integrated hardware/UI validation |
| Long-session process memory | Not yet measured interactively; deterministic registry ownership is bounded and the release soak remains a TL-12 gate |
