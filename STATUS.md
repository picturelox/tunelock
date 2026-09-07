# TuneLock status

Updated: 2026-09-07

Current milestone: TL-06 complete; TL-07 is in progress with a first Windows
S3 HID adapter checkpoint implemented and its remaining hardware gates open.

Integration baseline: `performance-build` at
`e07d936b6a9bcd560edb4928b83dd242157818ce`

Foundation checkpoint: `1fc95ba` contains TL-00 through TL-02.

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
handlers optimistically update local state; transition plans are storage-only;
and key release features are missing or unverified as detailed in
`CAPABILITIES.md`. The engine now routes master and private cue to explicit
channel pairs, but the performance desk has not yet exposed those controls.

## Next bounded ticket

TL-07 (shared action model and S3 adapter) is in progress. The first Windows
S3 HID input/LED path is implemented; TL-07 must not claim completion, S3 jog
timing, or scratch/backspin until the remaining controller and hardware gates
run.

## TL-07 S3 discovery evidence (2026-09-07)

The Traktor Kontrol S3 is connected and detected on Windows. It exposes its
control surface as a vendor-defined HID interface, not standard MIDI.

- VID `0x17CC`, PID `0x1900`, interface 3, usage page `0xff01`, serial
  `DE62E256`; product "Traktor Kontrol S3", manufacturer "Native Instruments".
- Interfaces: MI_00 audio (MEDIA class), MI_03 vendor-defined HID, MI_04 DFU.
- Input reports are 63 bytes. Byte 0 is the report ID: `0x01` carries
  buttons/jog and `0x02` carries continuous controls. Their histories are
  decoded independently so interleaved report types cannot create false edges.
- Jog values are four-byte little-endian fields at raw offset `0x0E` (Deck A)
  and `0x12` (Deck B): one distance-tick byte plus a 24-bit, 400 kHz timecode.
  TuneLock uses both wrapping deltas and the documented 768-ticks-per-rotation
  / 33 1/3 RPM relationship for scratch velocity; untouched turns become
  distance-based nudges. Native timing still needs controlled validation.
- Button map (byte index, bit mask), confirmed against the Mixxx S3 mapping:
  - Deck A: Play `(3, 0x01)`, Cue `(2, 0x80)`, Sync `(2, 0x08)`,
    Hot cue 1 `(3, 0x02)`, Hot cue 2 `(3, 0x04)`.
  - Deck B: Play `(6, 0x02)`, Cue `(6, 0x01)`, Sync `(5, 0x10)`.
  - Platter touch A: `(10, 0x10)`.
- A `s3-probe` binary (`cargo run --bin s3-probe`) and a `controller` module
  (`src-tauri/src/controller/mod.rs`) decode these into a semantic `S3Action`
  vocabulary (Play/Cue/Sync/HotCue/Touch/Jog/Control) with 11 passing focused
  regression tests. All eight hot-cue pads and platter touch are mapped on
  both physical decks.
- Long-report controls use named 16-bit little-endian offsets from the Mixxx
  mapping: Deck A/B tempo, volume, gain, and three-band EQ, plus crossfader,
  headphone mix, and headphone gain. They are decoded but intentionally not
  applied until TL-08 defines and measures control ranges and gain staging.
- The reader start is idempotent, keeps separate short/long predecessors, and
  retries after startup absence or disconnect. Native unplug/replug behavior
  still needs a controlled hardware run.
- LED output (reverse-engineered from the Mixxx S3 mapping, not a USB trace):
  - Output report `0x80` carries button/state LEDs; `0x81` carries VU meters.
  - Each LED is one byte. Palette LEDs encode `color + brightness` where color
    is `0x00..0x44` in steps of `0x04` (18 colors) and brightness is `0..3`.
    Single-color LEDs use `0x20` (off) / `0x77` (on).
  - Byte offsets: Play A `0x11`, Play B `0x2A`, Cue A `0x10`, Cue B `0x29`,
    Sync A `0x0C`, Sync B `0x25`. Deck base color is CARROT `0x08`; dim `1`,
    bright `3`.
  - `s3_set_leds` mirrors Play/Cue/Sync backlight from session transport state.
- Remaining: controlled jog/touch latency and unplug/replug runs, TL-08 control
  range/soft-takeover decisions, and verification of the 83-byte LED report
  length against the device's HID descriptor.

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

## TL-03 implementation result

- Added monotonic command IDs scoped by engine generation. A successful
  submission now means queued intent; completion requires a callback receipt.
- Added a bounded lock-free acknowledgement queue. The callback records each
  tracked command's exact application frame and increments explicit pressure
  telemetry when receipts cannot be retained, without allocating or blocking.
- Active load, transport, seek, loop, tempo, pitch, and loudness-gain actions
  now expose fixed per-deck pending state, wait up to two seconds for a receipt,
  and surface or roll back failed intent.
- Reconciliation preserves pending desired values until application rather than
  falsely treating optimistic local state as engine acknowledgement.
- The performance desk displays pending commands and cumulative command/receipt
  pressure, and serializes active per-deck control bursts.
- Master-gain and bus setup now return explicit missing-engine, invalid-input,
  and queue-full failures. Sync-specific receipts remain part of TL-05.

Fresh verification after TL-03:

| Check | Result |
|---|---|
| `npx tsc --noEmit` | Passed |
| `npm run build` | Passed; 1,609 modules transformed; 259.90 kB JavaScript and 33.25 kB CSS before gzip |
| `cargo test` | Passed: 233 library tests + 8 binary tests; 0 failed; 7 ignored performance tests |
| Focused acknowledgement regressions | Passed: exact callback-frame receipt, queue-full submission, bounded receipt overflow, wire serialization, and allocation-audited callback publication |
| Two-deck 48 kHz/256 release harness | Passed: max 1,227 microseconds, average 256 microseconds, p99 1,167 microseconds; 5,333 microsecond budget |
| Native interaction, device failure, S3, and macOS | Not run; retained for TL-04, TL-07, and TL-12 hardware/platform gates |

## TL-04 implementation result

- Added a cue (PFL) tap taken from each player's post-trim/deck-EQ output,
  before the channel fader and crossfader, so a cued deck is audible even when
  its channel is crossfaded out.
- Added per-deck cue selection, a cue sum, headphone level, and cue/master
  blend, all as frame-addressed engine commands with generation-scoped
  acknowledgements.
- Added explicit master and cue output channel pairs (defaults master 0/1,
  cue 2/3) with a `SetOutputRouting` command for other multichannel layouts.
- Routed master and cue to their pairs in the callback; unused device channels
  are zeroed. On stereo-only devices the cue monitor folds into the master pair,
  and mono devices downmix explicitly.
- Fixed the I16 conversion path to chunk at whole-frame boundaries so a scratch
  slice never splits an interleaved device frame mid-frame.
- Wired the session service and store with per-deck `cueEnabled`, engine-level
  `headphoneLevel` and `cueMasterBlend`, and re-applies them after device
  replacement.

Fresh verification after TL-04:

| Check | Result |
|---|---|
| `npx tsc --noEmit` | Passed |
| `npm run build` | Passed; 1,609 modules transformed; 261.27 kB JavaScript and 33.25 kB CSS before gzip |
| `cargo test` | Passed: 241 library tests + 8 binary tests; 0 failed; 7 ignored performance tests |
| Focused routing regressions | Passed: cue never reaches master on 4ch, stereo fallback folds cue, 6ch zeroes unused channels, mono downmix, whole-frame counts across 1/2/4/6/8ch, cue gain, per-deck cue selection, and explicit cue-pair rerouting |
| Two-deck 48 kHz/256 release harness | Passed: max 1,262 microseconds, average 271 microseconds, p99 1,187 microseconds; 5,333 microsecond budget |
| Native listening, S3 four-channel device, and macOS | Not run; retained for TL-07 and TL-12 hardware/platform gates |

## TL-05 implementation result

- Added eight hot-cue slots per player with `SetHotCue` and `JumpHotCue`
  commands; a jump seeks to the stored beat and starts playback.
- Added a signed fractional-beat `Nudge` command for manual beat alignment.
- Changed BeatSync to align B's fractional beat phase to A's exact beat
  position (no rounding), so both decks share sub-beat phase after Sync.
- Added a monotonic grid revision to `AttachBeatGrid`; stale (older) grid
  revisions are rejected so a corrected grid cannot be overwritten.
- Made BeatSync and BarSync tracked commands with generation-scoped
  acknowledgements, and wired hot-cue/nudge/sync through the session service.
- Equivalent A/B transport now covers load/play/pause/seek/cue/loop/nudge on
  both decks through the same session adapter.

Fresh verification after TL-05:

| Check | Result |
|---|---|
| `npx tsc --noEmit` | Passed |
| `npm run build` | Passed; 1,609 modules transformed; 261.94 kB JavaScript and 33.25 kB CSS before gzip |
| `cargo test` | Passed: 245 library tests + 8 binary tests; 0 failed; 7 ignored performance tests |
| Focused transport regressions | Passed: hot-cue set/jump, signed nudge, fractional beat-phase alignment, and stale grid-revision rejection |
| Two-deck 48 kHz/256 release harness | Passed: max 1,127 microseconds, average 262 microseconds, p99 1,108 microseconds; 5,333 microsecond budget |
| Native listening, S3 jog timing, and macOS | Not run; retained for TL-06, TL-07, and TL-12 hardware/platform gates |

## TL-06 implementation result

- Added signed jog/scratch read-rate support to the time/pitch processor
  abstraction: `set_jog_rate` (negative = reverse, 0 = hold) and `set_jogging`
  (engage/release). The varispeed processor implements reverse and hold; bypass
  and Signalsmith keep the default no-op.
- Added `JogTouch` and `JogRate` engine commands, wired through the Player's
  `engage_jog`/`set_jog_rate`/`release_jog` methods. Engaging scratch switches
  to the varispeed processor and starts from a hold at zero; release resumes
  normal tempo/pitch playback from the current position.
- Added `audio_engine_jog_touch` and `audio_engine_jog_rate` IPC commands plus
  session-service `jogTouch`/`jogRate` methods with generation-scoped
  acknowledgements.
- Pitch-lock interaction is varispeed (pitch follows rate) during scratch;
  Master Tempo reverse is out of scope. Crossfader and Sync handoff reuse the
  existing bus routing and BeatSync re-alignment.

Fresh verification after TL-06:

| Check | Result |
|---|---|
| `npx tsc --noEmit` | Passed |
| `npm run build` | Passed; 1,609 modules transformed; 262.33 kB JavaScript and 33.25 kB CSS before gzip |
| `cargo test` | Passed: 249 library tests + 8 binary tests; 0 failed; 7 ignored performance tests |
| Focused transport regressions | Passed: varispeed jog reverse, hold-at-zero silence, release/resume, and engine-level jog reverse/hold/resume |
| Two-deck 48 kHz/256 release harness | Passed: max 1,158 microseconds, average 268 microseconds, p99 1,122 microseconds; 5,333 microsecond budget |
| Native listening, S3 jog timing, and macOS | Not run; retained for TL-07 and TL-12 hardware/platform gates |
