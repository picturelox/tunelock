# TuneLock delivery roadmap

Status: canonical plan as of 2026-09-06

Product contract: `PRODUCT.md`

Capability baseline: `CAPABILITIES.md`

The checked-out integration baseline is `performance-build` at
`e07d936b6a9bcd560edb4928b83dd242157818ce`. `main` and
`core-intelligence` remain preserved at `d0cfd4d`. Do not merge branches merely
because one contains more features; move reviewed slices against this contract.

## Dependency path

`TL-00 -> TL-01 -> TL-02 -> TL-03 -> (TL-04, TL-05) -> TL-06 -> TL-07 -> TL-08 -> TL-09 -> TL-10 -> TL-11 -> TL-12`

Design exploration can run alongside TL-01 through TL-11, but audible UI
integration uses approved session contracts. Transition replay depends on stable
transport and scheduling. Controller feasibility begins as early as possible
because S3 access and jog timing can change implementation choices.

## Work packages

Every package must report changed files, actual behavior, focused checks,
listening or hardware evidence when relevant, unresolved risks, and the next
dependency. Test counts alone do not close an audio package.

### TL-00 - Canonical scope and baseline

**Outcome:** A fresh contributor can find the product destination, current
runtime behavior, required gates, baseline SHA, and next bounded ticket from the
repository alone.

**Scope:** Reconcile the supplied blueprint and legacy local plan; add
`PRODUCT.md`, `ROADMAP.md`, and `CAPABILITIES.md`; update README, STATUS, and
AGENTS; mark conflicting PREP documents historical; preserve research evidence.

**Acceptance:** Remote heads are re-read, working-tree state is recorded, TS and
Rust checks are rerun, no application code changes, and the documentation no
longer gives conflicting active product directions.

**Status:** Complete in the working tree; validation recorded in `STATUS.md`.

### TL-01 - Authoritative session model and engine ownership

**Outcome:** Deck A-D identities and one live engine/session survive workspace
changes, with playback independent of analysis completion.

**Allowed area:** New session modules and shared IPC types; minimal changes to
`App`, `Workspace`, engine state ownership, and registration needed to connect
them. Do not redesign DSP or the visual system.

**Contract:** Define `DeckId`, `PlayerId`, `SourceId`, `LoadGeneration`,
`EngineGeneration`, desired state, acknowledged state, and error state. Make
engine initialization single-flight under one owner.

**Failure cases:** Concurrent initialization, workspace remount, audio-device
rebuild, unavailable engine, and loading an unanalyzed source.

**Acceptance:** Concurrent init produces one owner; changing views does not
recreate the engine or unload playing decks; A/B use the same adapter; a file can
load and play while analysis continues asynchronously.

**Status:** Complete in the working tree. Rust serializes initialization and
device replacement and reports an engine generation; the app-root session owns
Deck A-D state and telemetry; A/B use the same adapter; Deck A load and analysis
start independently. Deterministic and release performance checks pass. A native
interactive walkthrough remains part of later integrated release validation.

### TL-02 - Source lifecycle and atomic loading

**Outcome:** Loading and replacing tracks is silent until requested, late work
cannot attach to a replacement source, and repeated loads have bounded memory.

**Contract:** Generation-checked prepare/load-paused, explicit source ownership,
eviction/unregister policy, cancellation, and deferred destruction away from the
callback.

**Failure cases:** Rapid A->B->C replacement, late grid/analysis completion,
decode failure, cancellation, repeated loading, and queue pressure.

**Acceptance:** No unintended launch during load; stale completions are rejected;
the source registry and process memory stabilize in a repeated-load test; the
audio callback remains allocation/deallocation/lock/I/O free.

**Status:** Complete in the working tree. A single realtime `LoadPaused`
operation replaces launch-then-pause; server-assigned load generations and
engine generations reject out-of-order decode/grid work; grid attachment also
checks the exact source handle; accepted replacement evicts the prior registry
reference; retired player buffers retain the existing non-realtime drain. The
focused silence, stale-grid, 100-reload ownership, full Rust, frontend build,
and release callback gates pass. Native long-session memory observation remains
part of TL-12 release qualification.

### TL-03 - Command acknowledgements and engine telemetry

**Outcome:** The interface distinguishes pending intent, accepted commands,
acknowledged engine state, and failure.

**Contract:** Bounded command IDs/results, compact telemetry snapshots, explicit
queue-full behavior, and reconciliation rules. React never treats a click alone
as proof that audio state changed.

**Acceptance:** Forced queue overflow and device/engine failures surface visibly;
load, stop, seek, loop, and gain controls cannot remain in a false success state;
control bursts remain bounded.

### TL-04 - Master and private-cue routing

**Outcome:** The S3 or another qualifying four-channel device plays master on one
stereo pair and private cue on another, with a truthful stereo-only fallback.

**Contract:** Explicit master pair, cue pair, cue tap, per-deck cue selection,
cue sum, headphone level, and cue/master blend. Hardware monitor level remains
separate from software master/recording gain.

**Acceptance:** 2/4/6/8-channel and supported sample formats have focused tests;
mono handling is explicit; frame conversion preserves whole device frames; cue
never reaches master. Device switching/reconnect behavior is documented.

### TL-05 - A/B transport, cues, loops, and beat Sync

**Outcome:** Two equivalent decks support dependable manual and synchronized
mixing.

**Contract:** Hot cues, quantized and unquantized loops, tempo/pitch, an explicit
Sync leader, fractional beat phase, valid grid identity/revision, and defined
manual takeover. Bar/phrase Sync is not a first-release claim.

**Acceptance:** Load/play/pause/seek/cue/loop/nudge sequences pass on both decks;
phase behavior is measured with nonzero grid origins and corrected grids; missing
or stale grids fail honestly; the other deck is unaffected.

### TL-06 - Jog, nudge, scratch, and backspin transport

**Outcome:** Mouse and the S3 can perform manual nudges plus occasional expressive
scratches and backspins without clicks or corrupting the other deck.

**Contract:** Signed jog motion, touch engagement, hold at zero, reverse, release
and clean resume, pitch-lock interaction, crossfader behavior, and Sync handoff.

**Acceptance:** A brief scratch, backspin, and release/resume work at measured
latency on the S3; master recording stays valid; advanced turntablism and motor
emulation remain excluded.

### TL-07 - Shared action model and S3 adapter

**Outcome:** Mouse, keyboard, generic MIDI, and the S3 drive the same semantic
actions and receive coherent state feedback.

**Prerequisite evidence:** OS/driver/firmware profile, discovered MIDI/HID
protocol, event trace, jog/touch timing, feedback capabilities, and audio-access
results. The user-supplied `NI-28953` is not assumed to be a verified USB ID.

**Acceptance:** Demonstrate load/play/cue/nudge/scratch on Windows and macOS;
focus and deck targeting are deterministic; disconnect/reconnect fails safely;
no bundled downloader or incompatible-license controller code is introduced.

### TL-08 - Mixer, effects, gain staging, and output protection

**Outcome:** Each deck has truthful trim/fader/EQ/isolator/filter controls plus
delay and reverb, while the master has transparent protection.

**Contract:** Specify control ranges, dB-to-linear mapping, smoothing, effect
routing, tails, bypass, crossfader assignment, headroom, and limiter behavior.
Do not conflate the current engine Bus A/B with Deck A/B.

**Acceptance:** Every visible control has an audible measured effect, unity and
bypass paths are characterized, rapid changes avoid unintended clicks, clipping
states are visible, and the limiter is tested as protection rather than marketed
as mastering.

### TL-09 - Master recording and finalization

**Outcome:** A complete performance produces a playable recording of the correct
duration without cue leakage.

**Contract:** Document the master tap point and format; use a preallocated bounded
handoff from callback to background writer; report overrun, disk, permission, and
finalization failures; never alter source files.

**Acceptance:** Start/stop/finalize/recover flows work; cue is absent; gain and
clipping match the documented tap; a long recording remains synchronized and
does not violate real-time rules.

### TL-10 - Connected intelligence workflows

**Outcome:** Intelligence is usable from the performance desk without becoming a
playback dependency.

**Contract:** Publish versioned `TrackIntelligenceSnapshot` data tied to source
fingerprint and analysis revision. Connect candidate audition/load, local
key/energy inspection, uncertain-key correction with provenance, and set
progression. Query the complete catalog server-side.

**Acceptance:** Each workflow reaches an action from a shared selection/deck
context; local results render before optional models; stale results cannot alter
a replacement source; uncertainty wording remains empirically honest.

### TL-11 - Record, edit, save, and replay transitions

**Outcome:** A DJ records control movements, refines supported automation on a
timeline, saves the transition, and replays it from an explicit action.

**Contract:** Version source fingerprints, grid revisions, cue/loop locations,
starting mixer/DSP/routing state, automation timebase, supported events, and plan
schema. Schedule via native clock with bounded lookahead. Define per-parameter
manual takeover and cancellation. Scratch automation is excluded until separately
accepted.

**Acceptance:** Save, restart, reopen, validate, replay, compare event timing,
interrupt manually, detect changed sources/grids, and repeat while recording;
cue remains excluded.

### TL-12 - Performance desk integration and release qualification

**Outcome:** The selected persistent performance desk delivers the complete A/B
home-set workflow on Windows and macOS.

**Scope:** Reusable deck components, waveforms, mixer, docked library, contextual
intelligence, timeline editor, recording status, keyboard/controller focus, and
empty/loading/analyzed/uncertain/playing/clipping/device-error states.

**Acceptance:** Complete discovery->load->cue->mix->transition replay->recording
on both operating systems and the S3; callback deadlines, long-session memory,
device recovery, controller focus, installation, accessibility, and listening
checks pass. The target set duration and minimum supported window size must be
frozen before this gate closes.

## Expansion after the first release

### M5 - Four full-track decks

Expose C/D through the same session and component contracts. Four real tracks,
sync leadership, cue selection, crossfader assignment, S3 bank mapping, effects,
and recording must meet deadline and listening gates without control jumps or
cross-talk.

### M6 - Stems

Only after M5, decide separation/import strategy, licenses, resource budget,
cache contract, and deck/voice hierarchy. Four decks times four stems implies at
least sixteen voices; the current eight player slots are not a stem architecture.

## Open release parameters

These choices do not block TL-01, but must be frozen before TL-12 closes:

- target duration for the long home-set acceptance run;
- minimum supported laptop/window size;
- recording file format and default destination;
- exact cue tap relative to deck EQ and channel fader;
- S3 firmware/driver versions present on the test Windows and macOS systems.
