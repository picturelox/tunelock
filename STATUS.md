# TuneLock — Status

> Replaces `progress.txt`, which predated Mix Canvas, Harmonic Mosaic, PianoRoll and Metronome. Authoritative as of the baseline commit.

## What the app is
The ultimate mix planner. Accurate key + BPM + energy analysis with honest confidence and ranked alternatives, exploration of harmonic relationships across a whole collection, set planning, and non-destructive DJ-ready delivery. Multimedia: video files are in scope. Target: **elite, all-genre accuracy that surpasses Mixed In Key.**

## What actually works today
- **Analyze (Tuner):** drop a file → live per-stage progress → key, Camelot, BPM, confidence, ranked runner-ups with musical reasons, Camelot wheel, Harmonic Mosaic, piano roll, metronome, chroma, timings.
- **Key engine:** HPSS → dual 12-bin + 72-band chroma → Krumhansl/Temperley/Sha'ath → 8-segment ranked vote. ~1 s/track in release.
- **Library table:** virtualised, smart filters — but only ever loads the first 200 rows.
- **Mix Canvas:** clip timeline + transition scoring — in-memory only, nothing persists.
- **Delivery:** CSV/M3U8 as browser downloads only; real export unreachable from UI.

## Known defects (see plan, `C:\Users\louis.media\.devin\plans\plan-dfdfe6627c43db0f.md`)
- `insert_track` returns a wrong id on re-import (`last_insert_rowid` after `ON CONFLICT DO UPDATE`) — re-analysis can write onto the wrong row.
- StrictMode double-registers drag-drop → every Tuner analysis runs twice.
- Tempo detector is 98 lines, unnormalised autocorrelation, hard 60–180 clamp; returns half-tempo (observed 76.0/69.8 vs MIK's 79 minimum).
- 671 audio files (.m4a/.aif) + all 341 video files cannot be decoded (missing symphonia codecs / no ffmpeg).
- 18 frontend wrappers call Rust commands that don't exist.
- Two competing harmony vocabularies (`lib/camelot.ts` vs `lib/relationships.ts`).
- HPSS kernel footprint is ~1.7 s (hop=4096), not the 210 ms the comments claim.
- setState-during-render in `MixWorkspace` and `DualAuditionPanel`.
- `bundle.active = false` — no installer can be produced.
- No test infrastructure beyond one Camelot unit test.

## Ground truth
- `ground-truth/MIKCompleteLibrary.csv` — 20,221 rows, 19,563 present on disk. Key (Camelot), Tempo, Energy, CuePoints, Genre, per track.
- `ground-truth/OUIE 7.csv` — 69 rows, 68 files, smoke corpus.
- `C:\Users\louis.media\Music\Tunelock Test Tracks` — 5-track smoke set.

## The plan
Fifteen phases, four hard checkpoints (A–D), in `C:\Users\louis.media\.devin\plans\plan-dfdfe6627c43db0f.md`. That file is the source of truth and is kept current.

## Verification commands
See `AGENTS.md`.
