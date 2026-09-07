# TuneLock capability ledger

Audited 2026-09-06 on `performance-build` at
`e07d936b6a9bcd560edb4928b83dd242157818ce`.

Status meanings: **Reachable** is available from the mounted application;
**Engine only** lacks a complete user workflow; **Partial** does not meet the
release contract; **Persistence only** stores data without executable behavior;
**Missing** has no production implementation; and **Unverified** lacks required
native or hardware evidence.

| Capability | Status | Baseline evidence and limitation | Next package |
|---|---|---|---|
| Active application surface | Reachable | `App.tsx` mounts the 749-line `Workspace`; legacy Console, Mix Canvas, and Listening Lab components are not navigable | TL-12 |
| Immediate local analysis | Reachable | File analysis reports key, BPM, energy, waveform, alternatives, and meters | TL-10 |
| Playback before analysis | Reachable | Deck A load and local analysis start independently; native source load returns before optional beat-grid analysis completes | TL-12 |
| Authoritative session model | Partial | An app-root session owns branded Deck A-D/player/source/load/engine/command identities, desired state, bounded pending commands, telemetry acknowledgement, and errors across Workspace remounts; legacy direct-control paths remain outside it | TL-12 |
| Native playback engine | Partial | CPAL engine, one clock, eight player slots, bounded command and acknowledgement queues, background decode/resample, server load generations, and one application-owned retired-buffer drain exist | TL-04/TL-05 |
| Source lifecycle | Reachable | Each accepted player replacement evicts its prior registry source; stale decode/grid work is generation checked; a deterministic 100-load test holds the registry at one source for that player | TL-12 |
| Atomic paused loading | Reachable | One `LoadPaused` callback command attaches the source and leaves transport paused; a regression test proves digital silence until explicit resume | TL-12 |
| Engine initialization ownership | Reachable | Initialization and device replacement share one lifecycle gate; installed engines have monotonic generations; a concurrency test covers serialized changes | TL-12 |
| Command acknowledgement | Partial | Active load, transport, seek, loop, tempo, pitch, and loudness-gain actions have generation-scoped callback receipts, bounded pressure telemetry, timeout/error handling, and rollback; Sync-specific and legacy direct-control paths remain | TL-05/TL-08 |
| Two-deck transport | Partial | A/B load/play/pause and synchronized launch use the same session adapter, but the current presentation remains asymmetric | TL-05/TL-12 |
| Cues and loops | Partial | Loop command and Deck A fixed-bar loop controls exist; a hot-cue data field exists but no production hot-cue workflow was found | TL-05 |
| Tempo, pitch, and Sync | Partial | Varispeed/Signalsmith, BeatSync, and BarSync exist with synthetic tests; fractional phase, grid revisions, nonzero downbeat origins, and manual takeover are not release-proven | TL-05 |
| Scratch and backspin | Missing | Current varispeed clamps the rate to positive values; no signed jog/hold/reverse transport exists | TL-06 |
| Deck mixer | Engine only | Per-player gain, pan, mute, solo, bus, EQ, kills, and loops are registered; active Workspace exposes only a subset | TL-08/TL-12 |
| Crossfader and filters | Engine only | Two buses, crossfade, and bus filters exist; deck identity versus crossfader-side vocabulary is not separated | TL-01/TL-08 |
| Delay and reverb | Missing | No production delay or reverb DSP/control path was found | TL-08 |
| Loudness matching and metering | Reachable | Reversible match gain, integrated loudness comparison, sample peak, and continuous oversampled true-peak metering exist | TL-08 |
| Output protection | Missing | A hard clamp exists and clipping is measured; UI states that no safety limiter is active | TL-08 |
| Multichannel output | Partial | Native multichannel devices can be selected, but only master channels 1-2 receive audio and higher channels are zeroed | TL-04 |
| Private headphone cue | Missing | No independent cue bus/output/tap or cue/master monitoring controls were found | TL-04 |
| Traktor Kontrol S3 | Unverified | No MIDI/HID adapter, mapping, event trace, feedback, or TuneLock master/cue proof exists | TL-07 |
| Mouse and keyboard action parity | Missing | Mouse controls call IPC directly; no shared semantic action model or complete keyboard map exists | TL-07 |
| Master recording | Missing | No callback-to-writer handoff, file writer, recorder state, or recording finalization workflow was found | TL-09 |
| Intelligence/performance contract | Partial | `TrackIntelligenceSnapshot` is defined and tested, but no active publication/consumption path was found; loudness defaults to `None` | TL-10 |
| Candidate discovery | Partial | Harmonic Mosaic and library data exist; active Mosaic selection is not wired into a complete audition/load flow and query scope is limited | TL-10 |
| Key/energy timeline and correction | Partial | Analysis and opinion infrastructure exist; source/analysis revision and persistent correction provenance are not joined to deck state | TL-10 |
| Set progression | Partial | Deterministic playlist/set tools and optional Ollama helpers exist; the connected performance-desk workflow is absent | TL-10 |
| Saved transition data | Persistence only | SQLite transition-plan commands exist; native-clock scheduling, executable replay, stale-plan checks, and manual takeover do not | TL-11 |
| Windows distribution | Partial | NSIS bundling is configured; native tests pass on Windows, but hardware and long-session gates have not run | TL-12 |
| macOS distribution | Unverified | Product target is confirmed, but no current macOS build, device, controller, or performance evidence is recorded | TL-12 |
| Four decks | Engine only | Eight player slots and an ignored synthetic four-deck harness exist; UI, routing, control banking, and reliability evidence do not | Post-release M5 |
| Stems | Deferred | Manifest persistence exists; work starts only after four-deck full-track reliability | Post-release M6 |

## Baseline checks

On 2026-09-06:

- `npx tsc --noEmit`: passed.
- `npm run build`: passed; Vite produced a 247.78 kB JavaScript bundle and a
  33.16 kB CSS bundle before gzip.
- `cargo test` from a Visual Studio x64 developer environment: 220 library
  tests and 8 binary tests passed; 0 failed; 7 release performance tests were
  ignored by design. The build emitted warnings but no errors.
- Current remote heads matched the review: `main` and `core-intelligence` at
  `d0cfd4d`; `performance-build` at `e07d936`.

TL-02 verification adds 228 passing library tests plus 8 binary tests, with 7
release performance tests ignored by default. The focused 48 kHz/256 release
harness passed at max 1,216 µs, average 268 µs, and p99 1,197 µs against a
5,333 µs callback budget. This remains deterministic evidence, not listening,
S3, macOS, or long-session process-memory evidence. `ACCURACY.md` remains the
authoritative measurement record.

TL-03 verification adds 233 passing library tests plus 8 binary tests, with 7
release performance tests ignored by default. The focused 48 kHz/256 release
harness passed at max 1,227 microseconds, average 256 microseconds, and p99
1,167 microseconds against a 5,333 microsecond callback budget. Focused tests
cover exact callback-frame receipts, command-queue rejection, bounded
acknowledgement overflow, IPC serialization, and allocation-audited
acknowledgement publication. Native listening, S3, macOS, and long-session
hardware evidence remain open.
