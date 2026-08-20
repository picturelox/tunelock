# Transition Workbench Slice A — Architecture Decision (Revised)

**Status:** Revised after technical review  
**Date:** 2026-08-20  
**Owner:** TuneLock  
**Related:** `PREP/transition-workbench-feature-spec.md` §8, §16 Slice A

## Revision summary

The initial decision (Web Audio API as primary engine) was based on flawed analysis. A technical review identified critical errors in both prototypes. This document corrects the decision.

**Revised decision: Native Rust engine on CPAL is the authoritative audio engine. Web Audio API is demoted to a UI interaction prototype/fallback only.**

## Flaws identified in the Web Audio API prototype

1. **Pitch preservation does not work.** Setting inverse `detune` alongside `playbackRate` does not preserve pitch while changing tempo. The Web Audio specification combines these into one computed playback rate — the inverse detune effectively cancels the speed change. True independent time stretching requires an AudioWorklet/WASM algorithm, not the built-in `detune` parameter.

2. **Start-error measurement is invalid.** The prototype compares the same JavaScript `now` value assigned to both decks, so it reports zero without measuring rendered audio. This is a measurement artifact, not a real result.

3. **Memory consumption.** `decodeAudioData` buffers complete tracks. Two five-minute stereo tracks consume ~212 MB of float PCM. Eight stems across two decks approach ~850 MB before other buffers.

## Flaws identified in the Rust prototype

1. **Nearest-neighbor resampling changes pitch and aliases.** Not suitable for production playback.
2. **Output sample rate is ignored.** Source buffers are not converted to the output device rate.
3. **Mutexes in the audio callback.** Multiple `Mutex` locks are acquired inside the real-time callback path, which can cause priority inversion, unbounded latency, or deadlocks.
4. **Per-block allocation.** The integer-output callback allocates a new `Vec` every block, which is a real-time violation.
5. **Stereo assumption.** Output channel count is hard-coded to stereo.
6. **EQ is approximate.** One-pole shelving filters do not provide flat reconstruction or convincing kills. A real DJ isolator needs complementary crossover filters.

## Corrected architecture

```
                    BACKGROUND / WORKER THREADS
 Source files ──► Symphonia decode ──► Rubato sample-rate conversion
 Stems       ──► alignment check  ──► per-deck source buffers
                                          │
                                 stem gain/mute/solo
                                          │
                                      deck sum
                                          │
                              Signalsmith time stretch
                                          │
                                  bounded ring buffer
                                          ▼
                         REAL-TIME CPAL OUTPUT CALLBACK
       Deck A buffer ──► EQ/gain ──┐
                                   ├─► crossfader ─► master meter/limiter ─► output
       Deck B buffer ──► EQ/gain ──┘
```

### Real-time constraints (non-negotiable)

The audio callback must **never**:
- Allocate memory
- Lock a mutex
- Decode a file
- Access SQLite or the filesystem
- Log
- Call Tauri
- Wait for another thread

UI actions enter a bounded lock-free command queue as frame-addressed commands. Meter snapshots and playhead position return to React at approximately 20–30 Hz.

### DSP priority order

1. **Sample-accurate transport** — one output stream, one monotonically increasing frame counter, all events scheduled against that counter
2. **Band-limited resampling** — Rubato (MIT/Apache-2.0), not nearest-neighbor
3. **Beat-grid DSP** — phase, downbeats, meter, DP beat tracking (not just BPM)
4. **Pitch-preserving time stretching** — Signalsmith Stretch (MIT), not Rubber Band (GPL)
5. **Mixer and gain staging** — real complementary crossover EQ, click-free ramps, 6 dB headroom
6. **Metering** — ITU-R BS.1770-5 true peak, per-deck peak/RMS, master meter
7. **Looping and seeking** — fractional-sample compensation, boundary crossfades, phase retention
8. **Multiresolution waveform pyramid** — replaces the current 2000-column overview
9. **Stem-specific DSP** — alignment verification, activity analysis

### Key dependencies

| Dependency | License | Purpose |
|---|---|---|
| `cpal` 0.15 | MIT/Apache-2.0 | Low-level audio output (WASAPI/CoreAudio/ALSA) |
| `rubato` | MIT/Apache-2.0 | Band-limited sample-rate conversion |
| `signalsmith-stretch` | MIT | Pitch-preserving time stretching |
| `symphonia` 0.5 | MPL-2.0 | Source file decoding (already used) |

Rubber Band is excluded because its open-source distribution is GPL, which conflicts with TuneLock's dependency rules unless a commercial license is acquired.

### Rust version compatibility

TuneLock currently declares Rust 1.70. CPAL 0.15 may require a newer compiler. The engine module will either pin a compatible CPAL release or intentionally upgrade the Rust baseline after testing — no unplanned dependency upgrade will decide this.

### Signal path separation

- TuneLock's 22.05 kHz mono analysis pipeline is for offline music analysis (key, BPM, energy, etc.)
- The Transition Workbench audio engine runs at full-quality stereo at the output device's rate (44.1 or 48 kHz)
- They share metadata and analysis results, but not the same signal path

## What the prototypes taught us

The Web Audio prototype validated:
- The UI interaction model (transport, crossfader, EQ, meters)
- The component hierarchy for the frontend

The Rust prototype validated:
- cpal works on Windows with WASAPI
- Symphonia can decode for the audio engine
- The mixer architecture concept is sound

Both prototypes are scaffolding, not production code. The real engine will be built as a proper `audio/` module in the main TuneLock crate with preallocated buffers, worker-thread decoding, and a lock-free real-time callback.
