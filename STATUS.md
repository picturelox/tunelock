# TuneLock — Status

> Replaces `progress.txt`, which predated Mix Canvas, Harmonic Mosaic, PianoRoll and Metronome. Authoritative as of the baseline commit.

## What the app is
The ultimate mix planner. Accurate key + BPM + energy analysis with honest confidence and ranked alternatives, exploration of harmonic relationships across a whole collection, set planning, and non-destructive DJ-ready delivery. Multimedia: video files are in scope. Target: **elite, all-genre accuracy that surpasses Mixed In Key.**

## What actually works today
- **Analyze (Tuner):** drop a file → live per-stage progress → key, Camelot, BPM, energy, confidence, ranked runner-ups with musical reasons, Camelot wheel, Harmonic Mosaic, piano roll, metronome, chroma, timings, three-band waveform.
- **Key engine:** HPSS → dual 12-bin + 72-band chroma → Krumhansl/Temperley/Sha'ath → 8-segment ranked vote. ~4 s/track in release (500-track sample).
- **Key timeline:** per-segment key detection with modulation boundaries + honest abstention for atonal material.
- **Genre-adaptive profiles:** electronic/classical/rock/hip-hop/jazz weight sets selected by genre metadata.
- **Energy detection:** loudness + spectral centroid + onset density + percussive ratio → 1–10 scale.
- **Consensus:** multi-source opinion model (TuneLock + MIK + Traktor + AcoustID) with four-dot agreement indicator.
- **Traktor NML import:** parse collection.nml, match by path/filename, store as opinions.
- **Media:** Symphonia (mp3/wav/flac/ogg/aac/alac/m4a/aiff) + ffmpeg sidecar fallback (video, malformed WAV). 0 decode failures on 20k library.
- **Waveforms:** three-band (low/mid/high) canvas renderer, 60 FPS, 2000 columns per track.
- **Library table:** server-side paging (500-row pages), infinite scroll, smart filters, sorting, MIK CSV import, Traktor NML import, consensus dots.
- **Playlist generation:** real harmonic compatibility scoring + BPM similarity. `generate_playlist` and `get_compatible_tracks` are functional.
- **Mix Canvas:** clip timeline + transition scoring. **Persists to database** — save/load across restarts via `save_mix`/`load_mix` commands. Clip notes stored in playlist `rules` JSON.
- **Delivery:** CSV/M3U8 as browser downloads only; real export unreachable from UI.
- **CNN (Phase 11):** Python ML project scaffolded and **trained, but the experiment was invalid** (not fairly evaluated). Multiple implementation bugs: windowing loaded only first 30s, augmentation was a no-op (rolled channel axis of length 1), best-epoch selection bias, no pitch-shift augmentation, insufficient training data (604 tracks vs 1,077 in the reference work). The 29.6% result diagnoses implementation problems, not CNN viability. Status: **experiment invalid; deferred.** The `ml/` scaffolding remains for a corrected re-run with MTG training data and the Korzeniowski protocol. The Rust `key_cnn.rs` stub returns `None` and is not wired into the ensemble.
- **Assist layer (Phase 11):** LLM-powered features via Ollama (local, offline). Four features built:
  1. DJ setlist analysis — paste a tracklist, LLM parses it, matches local library, shows harmonic flow with key/BPM/energy arc.
  2. Metadata repair — scan library for missing artist/title/genre, LLM parses filenames and infers metadata. User reviews and approves each change.
  3. Genre inference — LLM infers genre from artist/title, feeds adaptive profiles.
  4. NL set planning — describe a set in plain English ("90 min, start mellow, peak at 60"), LLM sequences tracks from library using harmonic compatibility.
  5. Transition explanations — LLM explains why a transition works (with deterministic template fallback when Ollama is absent).
  Never on the critical analysis path. All features are user-initiated. Degrades gracefully when Ollama is not installed.
- **Transition Workbench (Phase 7, Slice A):** Audio engine prototypes built and evaluated. Architecture decision: **Web Audio API** as the primary audio engine (zero drift by construction, built-in EQ/metering, `detune` for pitch preservation). Rust-side prototype validated cpal+Symphonia but requires a phase vocoder for pitch preservation. Test fixtures generated (click tracks at 120/128/140 BPM, sine tones at 440/880Hz, 2-minute drift fixture). Database infrastructure added: `beat_grids`, `transition_plans`, `stem_manifests` tables with migration 002. Tauri commands registered: `get_beat_grid`, `save_beat_grid_override`, `reset_beat_grid_override`, `get_transition_plan`, `save_transition_plan`, `get_stem_manifest`.

## Known defects (see plan, `C:\Users\louis.media\.devin\plans\plan-dfdfe6627c43db0f.md`)
- ~~`insert_track` returns a wrong id on re-import~~ **Fixed (Phase 5)** — uses `RETURNING id`.
- ~~StrictMode double-registers drag-drop → every Tuner analysis runs twice.~~ **Fixed (Phase 5)** — cancelled flag + late teardown.
- ~~Tempo detector is 98 lines, unnormalised autocorrelation, hard 60–180 clamp~~ **Fixed (Phase 3)** — octave resolution + wider range, 59.4% BPM ±1.
- ~~671 audio files (.m4a/.aif) + all 341 video files cannot be decoded~~ **Fixed (Phase 2)** — 0 unsupported, 0 decode failures.
- ~~18 frontend wrappers call Rust commands that don't exist.~~ **Fixed (Phase 5)** — all 18 phantom wrappers deleted.
- ~~Two competing harmony vocabularies~~ **Fixed (Phase 5)** — unified `lib/harmony.ts` + Rust `harmony/mod.rs` mirror.
- ~~HPSS kernel footprint is ~1.7 s (hop=4096), not the 210 ms the comments claim.~~ **Fixed (Phase 5)** — comments corrected to match actual parameters.
- ~~setState-during-render in `MixWorkspace` and `DualAuditionPanel`.~~ **Fixed (Phase 5)** — moved to useEffect.
- ~~`bundle.active = false` — no installer can be produced.~~ **Fixed** — bundle enabled with NSIS target for Windows installer.
- ~~No test infrastructure beyond one Camelot unit test.~~ **Improved** — 52 tests (harmony, metrics, tempo, consensus, waveform, energy, genre profiles, key timeline, CNN stub, NML parsing, gold annotations).

## Ground truth
- `ground-truth/MIKCompleteLibrary.csv` — 20,221 rows, 19,563 present on disk. Key (Camelot), Tempo, Energy, CuePoints, Genre, per track.
- `ground-truth/OUIE 7.csv` — 69 rows, 68 files, smoke corpus.
- `C:\Users\louis.media\Music\Tunelock Test Tracks` — 5-track smoke set.

## The plan
Fifteen phases, four hard checkpoints (A–D), in `C:\Users\louis.media\.devin\plans\plan-dfdfe6627c43db0f.md`. That file is the source of truth and is kept current.

## Verification commands
See `AGENTS.md`.
