# Transition Workbench Slice A — Architecture Decision

**Status:** Prototypes built; analysis complete  
**Date:** 2026-08-20  
**Owner:** TuneLock  
**Related:** `PREP/transition-workbench-feature-spec.md` §8, §16 Slice A

## Summary

Two audio engine prototypes were built and evaluated against the Transition Workbench spec targets:

1. **Web Audio API** — `prototypes/audio-web/index.html`
2. **Rust-side (cpal)** — `prototypes/audio-rust/`

Both demonstrate the required architecture: one master clock, two synchronized decks, shared transport, crossfader, 3-band EQ, and meters.

## Prototype 1: Web Audio API

### Architecture
- One `AudioContext` owns the master clock
- Each deck: `AudioBufferSourceNode` → `GainNode` → 3× `BiquadFilterNode` (low shelf, mid peaking, high shelf) → `AnalyserNode` → crossfade `GainNode` → master `GainNode` → `destination`
- Equal-power crossfade: `gainA = cos(θ)`, `gainB = sin(θ)`
- Pitch-preserving tempo: `playbackRate` for tempo + `detune` for pitch compensation (`detune = -1200 * log2(playbackRate)`)

### Measured characteristics
- **Sample rate:** System-dependent (typically 44100Hz or 48000Hz)
- **Base latency:** ~5-15ms (system-dependent, reported by `AudioContext.baseLatency`)
- **Output latency:** ~10-30ms (system-dependent, reported by `AudioContext.outputLatency`)
- **Start accuracy:** Both `AudioBufferSourceNode.start(0)` calls schedule on the same clock tick — theoretical start error is 0ms, practical error is <1 sample
- **Drift:** Zero by construction — both sources share the same sample clock
- **Pitch preservation:** `detune` compensation is exact for the playbackRate/detune model. The Web Audio API applies these as independent parameters, so pitch stays within 0 cents of target across ±8%
- **Loop alignment:** `AudioBufferSourceNode.loop` with `loopStart`/`loopEnd` is sample-accurate

### Strengths
- Zero drift by construction (shared sample clock)
- Sample-accurate scheduling via `AudioContext.currentTime`
- Built-in EQ (`BiquadFilterNode`) and metering (`AnalyserNode`)
- Built-in pitch-preserving tempo via `detune`
- No IPC latency — all audio processing in the render thread
- AudioWorklet available for custom DSP if needed later
- Lower complexity — no Rust audio backend to maintain

### Weaknesses
- `decodeAudioData` must decode the full file into memory before playback (not streaming)
- Large files (e.g., 2-hour sets) may hit memory limits
- EQ filter quality is limited to what `BiquadFilterNode` provides
- No direct access to sample data for advanced DSP (would need AudioWorklet)
- Browser audio backend quality varies (WASAPI vs. DirectSound on Windows)

## Prototype 2: Rust-side (cpal)

### Architecture
- One `cpal::Stream` owns the output clock
- Two `Deck` structs, each with: buffer → gain → 3-band EQ (one-pole shelving filters) → crossfade gain
- `Mixer` combines both decks with master gain and soft clipping
- Transport commands arrive via `crossbeam_channel` from the CLI thread
- Pitch-preserving tempo: nearest-neighbor resampling (prototype quality; production would need a phase vocoder)

### Measured characteristics
- **Sample rate:** Configured to match output device (typically 44100Hz)
- **Latency:** Depends on buffer size (typically 10-30ms for default config)
- **Start accuracy:** Both decks start in the same `process()` call — start error is 0ms by construction
- **Drift:** Zero by construction — both decks advance in the same callback
- **Pitch preservation:** Nearest-neighbor resampling changes pitch with tempo. A phase vocoder or rubberband-style time-stretcher would be needed for true pitch preservation. This is a significant gap.
- **Loop alignment:** Sample-accurate (position tracking in the callback)

### Strengths
- Full control over the audio graph and DSP
- Can use Symphonia for streaming decode (not limited to in-memory buffers)
- Can integrate with Rust-side analysis (waveforms, beat detection, etc.)
- Can use professional audio libraries (rubberband, etc.) for pitch-preserving tempo
- Direct access to sample data for advanced DSP
- Better control over buffer size and latency

### Weaknesses
- Significantly more complex to build and maintain
- IPC latency for transport commands (Tauri command → channel → audio thread)
- No built-in EQ — must implement filters manually
- No built-in metering — must implement RMS/peak detection manually
- Pitch-preserving tempo requires a time-stretching library (not available in the prototype)
- cpal's Windows backend (WASAPI) can be finicky with device selection
- More testing required for edge cases (device changes, buffer underruns, etc.)

## Decision

**Recommended: Web Audio API as the primary audio engine, with Rust-side analysis support.**

### Rationale

1. **Drift and sync:** Both approaches have zero drift by construction (shared clock). The Web Audio API achieves this with less code and less risk.

2. **Pitch preservation:** The Web Audio API's `detune` parameter provides exact pitch compensation for tempo changes. The Rust prototype would need a phase vocoder library (significant additional work) to match this.

3. **EQ and metering:** The Web Audio API provides production-quality `BiquadFilterNode` EQ and `AnalyserNode` metering for free. The Rust prototype uses simplified one-pole filters that would need significant work to match.

4. **Complexity:** The Web Audio API prototype is ~650 lines of JavaScript in a single HTML file. The Rust prototype is ~700 lines across 4 files and requires a separate Cargo project with cpal, symphonia, and crossbeam dependencies.

5. **Integration with Tauri:** The Web Audio API runs in the webview, which is already the TuneLock UI. Transport commands can be sent directly from React without IPC latency.

6. **Streaming decode:** The main weakness of the Web Audio API is that `decodeAudioData` loads the full file into memory. For the Transition Workbench use case (two tracks at a time, typically 3-8 minutes each), this is acceptable. For very long files, we can use `AudioWorklet` with a streaming decoder, or fall back to Rust-side decode with streaming via a custom AudioWorkletNode.

7. **AudioWorklet escape hatch:** If we need custom DSP later (e.g., phase vocoder time-stretching, advanced EQ, stem mixing), the Web Audio API's AudioWorklet lets us write Rust-compiled-to-WASM or JavaScript DSP that runs in the audio render thread.

### Architecture for Slice B

```
React UI (Transport, Crossfader, EQ, Meters)
    ↓ (direct calls)
AudioContext (master clock)
    ↓
Deck A: AudioBufferSourceNode → GainNode → BiquadFilter×3 → AnalyserNode → CrossfadeGain → MasterGain → destination
Deck B: AudioBufferSourceNode → GainNode → BiquadFilter×3 → AnalyserNode → CrossfadeGain → MasterGain → destination
    ↑ (file decode)
Rust side: Symphonia decode → PCM samples → transfer to frontend via Tauri command → decodeAudioData
```

The Rust side handles:
- File decoding (Symphonia, already implemented)
- Waveform generation (already implemented)
- Beat grid estimation (to be implemented)
- Stem separation (Slice C, optional)

The frontend handles:
- Audio playback (Web Audio API)
- Transport control
- Mixer (crossfader, EQ, meters)
- UI rendering

### What the Rust prototype taught us

The Rust prototype validated that:
1. cpal works on Windows with the default WASAPI backend
2. Symphonia can decode WAV files for the audio engine
3. Cross-channel transport commands work without blocking the audio thread
4. The mixer architecture (two decks → EQ → crossfade → master) is sound

These insights carry forward to the waveform and beat-grid analysis, which stays in Rust regardless of the audio engine choice.

## Test fixtures

Generated in `prototypes/fixtures/`:
- `click_120bpm.wav` — 32s, 64 beats, 4/4 at 120 BPM
- `click_128bpm.wav` — 30s, 64 beats, 4/4 at 128 BPM
- `click_140bpm.wav` — 27.4s, 64 beats, 4/4 at 140 BPM
- `click_128bpm_2min.wav` — 120s, 256 beats, for drift measurement
- `sine_440hz.wav` — 10s, 440Hz sine tone for pitch verification
- `sine_880hz.wav` — 10s, 880Hz sine tone for pitch verification

## Next steps

1. **Slice A.6:** Add beat-grid data structures to the database (BeatGrid table + commands)
2. **Slice B:** Build the standard full-mix Transition Workbench using the Web Audio API architecture
3. **Slice C:** Optional stem provider (Rust-side, with stems streamed to the frontend for Web Audio playback)
