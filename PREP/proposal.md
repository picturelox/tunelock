# Project Proposal: NotMixedInKey

> A fast, accurate, cross-platform DJ music analysis and set-building tool  
> Codename: **NotMixedInKey**

---

## 1. Problem Statement

DJs need to analyze their music libraries for **key** and **BPM** to perform harmonic mixing — playing songs in compatible keys for smooth, professional-sounding transitions. The current landscape has clear gaps:

- **Mixed In Key** is accurate but expensive ($58), has no playback, no visual song relationship mapping, no built-in set building, and requires an internet connection.
- **beaTunes** is feature-rich but slow (Java-based), with mid-tier key accuracy and a dated UI.
- **Built-in DJ software detection** (Traktor, Rekordbox, Serato) varies wildly in accuracy (41–70%).
- **KeyFinder** is free and open-source but abandoned as a standalone app (mid-tier accuracy, no playlist features).
- **No tool exists** that combines accurate analysis with visual harmonic relationship mapping and a lightweight DJ preview mixer in one app.

---

## 2. Value Proposition

NotMixedInKey will be:

| Quality | How |
|---|---|
| **Fast** | Tauri (Rust backend) — small binary, native performance, instant startup |
| **Stable** | Rust's memory safety guarantees; no Electron bloat, no Java runtime |
| **Highly Accurate** | Multi-stage ensemble key detection (HPSS + multi-profile classical + multi-model CNN + MIK-validated self-tuning). Targeting ≥90% exact match — surpassing Mixed In Key |
| **Visual** | Interactive Camelot wheel visualization showing harmonic relationships between songs |
| **Set Builder** | Automated playlist suggestions based on Camelot wheel rules (+1, +2, A↔B, etc.) |
| **DJ Preview** | Built-in dual-deck player with crossfader, EQ, volume, and waveform display |
| **Scalable** | 3-pass streaming import: 50k files visible in <2s, results stream in real-time, smart priority queue analyzes what you're looking at first |
| **Cross-Platform** | Windows + macOS from a single codebase (Tauri 2) |
| **Free / Open** | Open-source core. No internet requirement. Your music stays local. |

---

## 3. Target Users

1. **DJs** (primary) — bedroom to professional. Need to prep sets, find compatible tracks, preview mixes.
2. **Music producers** — need to know the key of samples and references for production.
3. **Music curators / playlist builders** — Spotify playlist creators, radio programmers, event organizers.

---

## 4. MVP Scope (v1.0)

### 4.1 Must Have (MVP)

- [ ] **Audio file import** — drag-and-drop or file picker. Support: `.mp3`, `.wav`, `.flac`, `.ogg`, `.aiff`, `.m4a`, `.mp4`
- [ ] **Batch analysis** — analyze key + BPM for entire folders. Show progress.
- [ ] **Key detection** — hybrid classical + CNN. Display in standard notation AND Camelot notation.
- [ ] **BPM detection** — accurate tempo estimation.
- [ ] **Music library view** — sortable/filterable table showing: filename, artist, title, key, Camelot code, BPM, energy level, duration.
- [ ] **Camelot wheel visualization** — interactive wheel showing which songs fall on which key. Click a song to highlight compatible songs.
- [ ] **Harmonic playlist builder** — given a starting track, suggest a sequence using Camelot rules. User selects rules: same key, ±1, ±2, A↔B, energy curve (build up / wind down).
- [ ] **ID3 tag writing** — write detected key + BPM back into audio file metadata so results persist in Traktor/Serato/Rekordbox.
- [ ] **Dual-deck preview player** — two decks with:
  - Play/pause/stop
  - Waveform visualization (per deck)
  - Volume fader (per deck)
  - Crossfader
  - 3-band EQ with kill switches
  - Deck load (drag track from library to deck)
- [ ] **Cue point system** — set, name, color-code, and jump to cue points on each deck's waveform. Store cue data per track. Export cue points to Rekordbox XML, Serato markers, and Traktor NML formats.
- [ ] **MIK validation engine** — on import, read any existing Mixed In Key tags (key, Camelot code, energy) from the user's files. Compare our detection against MIK's classification. Generate a confidence/agreement report. Use agreement data to self-calibrate the ensemble weights over time.
- [ ] **Non-destructive file export** — after a playlist is finalized, export copies of the audio files (never move/modify originals) into an organized folder structure (numbered by playlist order, with metadata tags written). Ready for drag-and-drop import into Traktor, Serato, Rekordbox, or a USB stick for CDJs at a venue.
- [ ] **Song relationship visualization** — visual display showing how songs relate harmonically (inspired by mashupbreakdown.com). Lines/connections between compatible tracks.

### 4.2 Nice to Have (v1.1+)

- [ ] Energy level detection (1–10 rating)
- [ ] Waveform color-coding by harmonic content
- [ ] Set export (M3U playlist, PDF set list)
- [ ] Cloud sync of library metadata
- [ ] Audio fingerprinting for duplicate detection
- [ ] Genre auto-tagging via ML model

### 4.3 Out of Scope (v1.0)

- Full DJ performance software (no beatmatching, no effects beyond EQ)
- Streaming service integration
- Mobile app
- Audio editing / mastering

---

## 5. Success Metrics

| Metric | Target |
|---|---|
| Key detection accuracy (exact match vs human consensus) | ≥ 90% |
| Key detection accuracy (within compatible key) | ≥ 96% |
| BPM detection accuracy (within ±1 BPM) | ≥ 95% |
| Analysis speed (per track, 4-minute song) | < 5 seconds |
| App startup time | < 2 seconds |
| Binary size (installer) | < 30 MB |
| Memory usage (1000-track library loaded) | < 200 MB |
| Time to first row visible (50k import) | < 2 seconds |
| Time to first analysis result (50k import) | < 8 seconds |
| Analysis throughput (8-core machine) | 8-15 tracks/sec |
| Library scroll performance (50k tracks) | 60 FPS, no jank |

---

## 6. Competitive Positioning

```
                    Accuracy
                       ▲
                       │
    NotMixedInKey ◆────┤
         Mixed In Key  ●
                       │
         Serato/RBox   ●
                       │
         beaTunes      ●
                       │
         KeyFinder     ●
                       │
         Traktor       ●
                       │
         Virtual DJ    ●
                       │
                       └──────────────────────────► Features
                       (key only)          (analysis + visual + mixer + export)
```

**Our differentiation:** We **surpass** MIK accuracy via a multi-stage ensemble with self-calibrating feedback, while offering a **significantly richer feature set** — visual harmonic mapping, playlist building, cue points, a built-in preview mixer, non-destructive DJ-software-ready file export — all for free.

---

## 7. Risks & Mitigations

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Key detection accuracy below target | Low | High | 5-stage ensemble (HPSS → multi-profile classical → multi-model CNN → temporal voting → MIK-calibrated feedback). Self-improving with user's own MIK-tagged library as ground truth. |
| CNN model too large for desktop app | Low | Medium | Use ONNX quantized model (~5-15 MB). Lazy load. |
| Cross-platform audio decoding issues | Medium | Medium | Symphonia (Rust) handles most formats. FFmpeg fallback for edge cases. |
| Tauri ecosystem maturity | Low | Low | Tauri 2 is stable. Large community. Well-documented. |
| Licensing conflicts (GPL libraries) | Medium | High | Careful dependency audit. Use AGPL Essentia via WASM isolation or implement key algorithms from scratch using research papers. |

---

## 8. Timeline Estimate (Solo Dev)

| Phase | Duration | Deliverable |
|---|---|---|
| **Phase 1: Foundation** | 2-3 weeks | Project scaffold, audio decoding, basic UI shell |
| **Phase 2: Analysis Engine** | 3-4 weeks | Key detection (classical), BPM detection, batch processing |
| **Phase 3: Library & Visualization** | 2-3 weeks | Music library view, Camelot wheel, harmonic mapping |
| **Phase 4: Playlist Builder** | 1-2 weeks | Harmonic playlist generation with Camelot rules |
| **Phase 5: DJ Preview** | 2-3 weeks | Dual-deck player, waveform, crossfader, EQ |
| **Phase 6: CNN + Accuracy Engine** | 3-4 weeks | Multi-model CNN ensemble, HPSS preprocessing, temporal voting, MIK validation engine |
| **Phase 7: Cue Points + File Export** | 2-3 weeks | Cue point system, non-destructive playlist file export, DJ software format export |
| **Phase 8: Polish & Ship** | 2 weeks | Installer, testing, docs |
| **Total** | **~18-24 weeks** | v1.0 release |

---

## 9. The Human Role

This tool is designed to **augment** the DJ, not replace them. The Camelot wheel rules and harmonic analysis provide **suggestions** — the human makes the creative decisions:

- Which energy curve to build
- When to break harmonic rules for dramatic effect
- Which tracks tell the story they want to tell
- The final ears-on approval of every transition

The app provides the data and the options. The DJ provides the artistry.
