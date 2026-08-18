# Architecture Diagrams: NotMixedInKey

> ASCII system diagrams for developer reference.  
> These show how every component connects at the system, data, and UI level.

---

## 1. High-Level System Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│                        NotMixedInKey App                            │
│                                                                     │
│  ┌───────────────────────────────────────────────────────────────┐  │
│  │                    FRONTEND (Webview)                          │  │
│  │                  React + TypeScript + Tailwind                 │  │
│  │                                                               │  │
│  │  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌────────────────┐   │  │
│  │  │ Library  │ │ Camelot  │ │ Playlist │ │  DJ Preview    │   │  │
│  │  │ Table    │ │ Wheel    │ │ Builder  │ │  (Dual Deck)   │   │  │
│  │  └────┬─────┘ └────┬─────┘ └────┬─────┘ └───────┬────────┘   │  │
│  │       │             │            │               │            │  │
│  │  ┌────┴─────────────┴────────────┴───────────────┴────────┐   │  │
│  │  │              Zustand State Stores                       │   │  │
│  │  │    libraryStore / playerStore / playlistStore            │   │  │
│  │  └────────────────────────┬────────────────────────────────┘   │  │
│  │                           │                                    │  │
│  │  ┌────────────────────────┴────────────────────────────────┐   │  │
│  │  │              Tauri IPC (invoke / listen)                 │   │  │
│  │  └────────────────────────┬────────────────────────────────┘   │  │
│  │                           │                                    │  │
│  │  ┌────────────────────────┴────────────────────────────────┐   │  │
│  │  │           Web Audio API (playback + EQ + mixer)          │   │  │
│  │  └─────────────────────────────────────────────────────────┘   │  │
│  └───────────────────────────┬───────────────────────────────────┘  │
│                              │ IPC Bridge                           │
│  ┌───────────────────────────┴───────────────────────────────────┐  │
│  │                    BACKEND (Rust / Tauri)                      │  │
│  │                                                               │  │
│  │  ┌─────────────┐  ┌──────────────┐  ┌──────────────────────┐ │  │
│  │  │  Commands   │  │   Analysis   │  │   Data Layer         │ │  │
│  │  │  (IPC API)  │──│   Engine     │  │                      │ │  │
│  │  │             │  │              │  │  ┌────────────────┐  │ │  │
│  │  │ analyze_*   │  │ ┌──────────┐ │  │  │   SQLite DB    │  │ │  │
│  │  │ library_*   │  │ │ Decoder  │ │  │  │   (rusqlite)   │  │ │  │
│  │  │ playlist_*  │  │ │(symphonia│ │  │  └────────────────┘  │ │  │
│  │  │ tags_*      │  │ └──────────┘ │  │                      │ │  │
│  │  │ waveform_*  │  │ ┌──────────┐ │  │  ┌────────────────┐  │ │  │
│  │  └─────────────┘  │ │Chromagram│ │  │  │  File System   │  │ │  │
│  │                    │ │(rustfft) │ │  │  │  (audio files) │  │ │  │
│  │                    │ └──────────┘ │  │  └────────────────┘  │ │  │
│  │                    │ ┌──────────┐ │  │                      │ │  │
│  │                    │ │Key Detect│ │  │  ┌────────────────┐  │ │  │
│  │                    │ │(hybrid)  │ │  │  │  ID3 Tags      │  │ │  │
│  │                    │ └──────────┘ │  │  │  (lofty)       │  │ │  │
│  │                    │ ┌──────────┐ │  │  └────────────────┘  │ │  │
│  │                    │ │BPM Detect│ │  │                      │ │  │
│  │                    │ └──────────┘ │  └──────────────────────┘ │  │
│  │                    │ ┌──────────┐ │                           │  │
│  │                    │ │CNN Model │ │                           │  │
│  │                    │ │(ONNX/ort)│ │                           │  │
│  │                    │ └──────────┘ │                           │  │
│  │                    └──────────────┘                           │  │
│  └───────────────────────────────────────────────────────────────┘  │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
```

---

## 2. Audio Analysis Pipeline

```
┌──────────────┐
│  Audio File  │  .mp3 / .wav / .flac / .ogg / .aiff / .m4a / .mp4
│  (on disk)   │
└──────┬───────┘
       │
       ▼
┌──────────────┐
│   symphonia  │  Decode → PCM f32 mono @ 44100 Hz
│   decoder    │  Handle all formats via feature flags
└──────┬───────┘
       │
       ├──────────────────────────────────────────┐
       │                                          │
       ▼                                          ▼
┌──────────────┐                           ┌──────────────┐
│  KEY PATH    │                           │  BPM PATH    │
└──────┬───────┘                           └──────┬───────┘
       │                                          │
       ├─────────────┐                            │
       │             │                            │
       ▼             ▼                            ▼
┌────────────┐ ┌────────────┐              ┌────────────┐
│ CLASSICAL  │ │    CNN     │              │  Onset     │
│            │ │            │              │  Detection │
│ STFT       │ │ CQT/Mel   │              └──────┬─────┘
│ (4096/512) │ │ Spectro    │                     │
│     │      │ │     │      │                     ▼
│     ▼      │ │     ▼      │              ┌────────────┐
│ Chromagram │ │ ONNX Model │              │ Autocorrel │
│     │      │ │ Inference  │              │ + Tempogram│
│     ▼      │ │     │      │              └──────┬─────┘
│ Mean+Norm  │ │     ▼      │                     │
│     │      │ │ 24-class   │                     ▼
│     ▼      │ │ softmax    │              ┌────────────┐
│ Profile    │ │            │              │ BPM Refine │
│ Matching   │ │            │              │ (60-180)   │
│ (cos sim)  │ │            │              └──────┬─────┘
└──────┬─────┘ └──────┬─────┘                     │
       │              │                            │
       ▼              ▼                            │
┌─────────────────────────┐                        │
│   HYBRID ENSEMBLE       │                        │
│                         │                        │
│ Classical → Key A (0.82)│                        │
│ CNN       → Key B (0.91)│                        │
│                         │                        │
│ → Pick higher confidence│                        │
│ → Final: Key B          │                        │
└────────────┬────────────┘                        │
             │                                     │
             ▼                                     ▼
┌──────────────────────────────────────────────────────┐
│                   TrackAnalysis                       │
│                                                      │
│  key_standard: "A minor"                             │
│  key_camelot:  "8A"                                  │
│  key_confidence: 0.91                                │
│  bpm: 127.5                                          │
│  duration_ms: 245000                                 │
│  waveform_data: [0.12, 0.34, ...]                    │
└──────────────────────┬───────────────────────────────┘
                       │
                       ▼
              ┌────────────────┐
              │  SQLite DB     │  Persist results
              │  + ID3 Tags    │  Write back to file (optional)
              └────────────────┘
```

---

## 3. Frontend Component Tree

```
App
├── MainLayout
│   ├── Sidebar
│   │   ├── NavItem: "Library"
│   │   ├── NavItem: "Camelot Wheel"
│   │   ├── NavItem: "Playlists"
│   │   └── NavItem: "Settings"
│   │
│   ├── ContentArea (routes)
│   │   ├── /library
│   │   │   ├── ImportDialog
│   │   │   │   └── AnalysisProgress
│   │   │   └── LibraryTable
│   │   │       └── TrackRow (×N)
│   │   │           └── KeyBadge
│   │   │
│   │   ├── /camelot
│   │   │   ├── CamelotWheel (SVG)
│   │   │   │   ├── WheelSegment (×24)
│   │   │   │   └── TrackDot (×N)
│   │   │   └── HarmonicMap (D3 force graph)
│   │   │       ├── TrackNode (×N)
│   │   │       └── CompatibilityEdge (×M)
│   │   │
│   │   ├── /playlists
│   │   │   ├── PlaylistBuilder
│   │   │   │   ├── RuleSelector
│   │   │   │   └── EnergyCurveSelector
│   │   │   └── PlaylistView
│   │   │       ├── PlaylistTrackRow (×N, draggable)
│   │   │       └── ExportButton
│   │   │
│   │   ├── /validation
│   │   │   └── ValidationReport
│   │   │       ├── AccuracySummary (% agreement with MIK)
│   │   │       ├── DisagreementTable (tracks where we differ)
│   │   │       └── PerMethodBreakdown (accuracy per sub-method)
│   │   │
│   │   └── /settings
│   │       ├── AnalysisSettings
│   │       ├── TagSettings
│   │       └── ExportSettings
│   │
│   └── PlayerDock (always visible at bottom)
│       └── DualDeck
│           ├── Deck (A)
│           │   ├── Waveform (wavesurfer.js)
│           │   │   └── CueMarker (×8, positioned on waveform)
│           │   ├── CueButtonRow (8 color-coded hotcue pads)
│           │   ├── TransportBar (play/pause/stop)
│           │   └── TrackInfo (title, key, BPM)
│           │
│           ├── Mixer
│           │   ├── VolumeFader (A)
│           │   ├── Crossfader
│           │   ├── VolumeFader (B)
│           │   └── EQControl (×2: one per deck)
│           │       ├── KnobHi + KillSwitch
│           │       ├── KnobMid + KillSwitch
│           │       └── KnobLo + KillSwitch
│           │
│           └── Deck (B)
│               ├── Waveform (wavesurfer.js)
│               │   └── CueMarker (×8)
│               ├── CueButtonRow (8 hotcue pads)
│               ├── TransportBar
│               └── TrackInfo
```

---

## 4. Web Audio API Signal Graph

```
                        AudioContext
                            │
            ┌───────────────┼───────────────┐
            │                               │
     ┌──────┴──────┐                 ┌──────┴──────┐
     │  Deck A     │                 │  Deck B     │
     │ AudioBuffer │                 │ AudioBuffer │
     │ SourceNode  │                 │ SourceNode  │
     └──────┬──────┘                 └──────┬──────┘
            │                               │
            ▼                               ▼
     ┌──────────────┐                ┌──────────────┐
     │  GainNode    │                │  GainNode    │
     │  (Volume A)  │                │  (Volume B)  │
     └──────┬───────┘                └──────┬───────┘
            │                               │
            ▼                               ▼
     ┌──────────────┐                ┌──────────────┐
     │ BiquadFilter │                │ BiquadFilter │
     │ (Low Shelf)  │                │ (Low Shelf)  │
     │ freq: 320 Hz │                │ freq: 320 Hz │
     └──────┬───────┘                └──────┬───────┘
            │                               │
            ▼                               ▼
     ┌──────────────┐                ┌──────────────┐
     │ BiquadFilter │                │ BiquadFilter │
     │ (Peaking)    │                │ (Peaking)    │
     │ freq: 1kHz   │                │ freq: 1kHz   │
     │ Q: 1.0       │                │ Q: 1.0       │
     └──────┬───────┘                └──────┬───────┘
            │                               │
            ▼                               ▼
     ┌──────────────┐                ┌──────────────┐
     │ BiquadFilter │                │ BiquadFilter │
     │ (High Shelf) │                │ (High Shelf) │
     │ freq: 3.2kHz │                │ freq: 3.2kHz │
     └──────┬───────┘                └──────┬───────┘
            │                               │
            ▼                               ▼
     ┌──────────────┐                ┌──────────────┐
     │  GainNode    │                │  GainNode    │
     │ (Crossfade A)│                │ (Crossfade B)│
     │              │                │              │
     │ gain = f(x)  │                │ gain = f(x)  │
     │ where x =    │                │ where x =    │
     │ crossfader   │                │ crossfader   │
     │ position     │                │ position     │
     └──────┬───────┘                └──────┬───────┘
            │                               │
            └───────────┬───────────────────┘
                        │
                        ▼
                 ┌──────────────┐
                 │ AnalyserNode │  → waveform/spectrum data
                 │ (for viz)    │    for wavesurfer.js
                 └──────┬───────┘
                        │
                        ▼
                 ┌──────────────┐
                 │  Destination │  → speakers
                 │  (output)    │
                 └──────────────┘


Crossfade formula (equal-power):
  position = 0.0 (full left/A) to 1.0 (full right/B)
  gain_A = cos(position * π/2)
  gain_B = sin(position * π/2)
```

---

## 5. Data Flow: 3-Pass Import → Streaming Analysis → Progressive Display

```
USER drags folder onto app (50,000 files)
         │
═════════╪══════════════════════════════════════════════════════════
 PASS 1  │  SCAN (instant — <2 seconds)
═════════╪══════════════════════════════════════════════════════════
         ▼
┌─────────────────┐     Tauri invoke         ┌─────────────────────┐
│ ImportDialog.tsx │ ────────────────────────► │ scan_folder()       │
└─────────────────┘                           │ (filesystem only)   │
                                              │ Find audio files    │
                                              │ Dedup vs DB         │
                                              │ INSERT skeleton rows│
                                              └────────┬────────────┘
                                                       │
         ┌─────────────────────────────────────────────┘
         ▼
    Library table populates with 50k rows (filename + size only)
    Key/BPM columns show skeleton placeholders
    ► USER CAN ALREADY SCROLL AND BROWSE
         │
═════════╪══════════════════════════════════════════════════════════
 PASS 2  │  METADATA (fast — ~1 min for 50k files)
═════════╪══════════════════════════════════════════════════════════
         ▼
┌─────────────────┐     Tauri invoke         ┌─────────────────────┐
│  Auto-triggered │ ────────────────────────► │ read_metadata_batch │
│  after scan     │                           │ Read ID3 tags (lofty│
└─────────────────┘                           │ Read MIK tags       │
                                              │ Batch of 50 at once │
                                              └────────┬────────────┘
                                                       │
                                              emit("metadata-batch-complete")
                                              every 50 tracks
                                                       │
         ┌─────────────────────────────────────────────┘
         ▼
    Rows update: title, artist, album, duration fill in
    Tracks with existing MIK tags show those keys immediately
    ► USER CAN SORT, FILTER, SEARCH BY ARTIST/TITLE
         │
═════════╪══════════════════════════════════════════════════════════
 PASS 3  │  DEEP ANALYSIS (background — parallelized)
═════════╪══════════════════════════════════════════════════════════
         ▼
┌─────────────────┐     Tauri invoke         ┌─────────────────────┐
│  Auto-triggered │ ────────────────────────► │ start_analysis()    │
│  after metadata │                           │ (non-blocking)      │
└─────────────────┘                           └────────┬────────────┘
                                                       │
                                              ┌────────┴────────┐
                                              │  Priority Queue  │
                                              │                  │
                                              │ ┌──── Critical ──┤ (deck load)
                                              │ ├──── High ──────┤ (visible rows)
                                              │ ├──── Medium ────┤ (nearby scroll)
                                              │ └──── Low ───────┤ (everything else)
                                              └────────┬────────┘
                                                       │
         ┌─────────────────────────── rayon (N-1 cores) ──┐
         │              │              │                    │
         ▼              ▼              ▼                    ▼
    ┌─────────┐    ┌─────────┐    ┌─────────┐         ┌─────────┐
    │ Track A │    │ Track B │    │ Track C │   ...   │ Track N │
    │ decode  │    │ decode  │    │ decode  │         │ decode  │
    │ HPSS    │    │ HPSS    │    │ HPSS    │         │ HPSS    │
    │ chroma  │    │ chroma  │    │ chroma  │         │ chroma  │
    │ CNN ×3  │    │ CNN ×3  │    │ CNN ×3  │         │ CNN ×3  │
    │ temporal│    │ temporal│    │ temporal│         │ temporal│
    │ BPM     │    │ BPM     │    │ BPM     │         │ BPM     │
    └────┬────┘    └────┬────┘    └────┬────┘         └────┬────┘
         │              │              │                    │
         │ ◄──── EACH TRACK STREAMS RESULT IMMEDIATELY ────┤
         │              │              │                    │
         └──────────────┴──────┬───────┴────────────────────┘
                               │
                    ┌──────────┴──────────┐
                    │ Per-track:           │
                    │ 1. INSERT to SQLite  │
                    │ 2. emit("track-      │
                    │    analyzed",{id,    │
                    │    key,bpm,...})      │
                    └──────────┬──────────┘
                               │
                               ▼
                    ┌──────────────────────────────┐
                    │ Frontend (requestAnimationFrame)│
                    │ Buffer incoming events          │
                    │ Flush batch on next frame       │
                    │ → update only visible rows      │
                    │ → Camelot wheel adds dots       │
                    │ → progress bar updates           │
                    └──────────────────────────────────┘

    ► USER INTERACTS WITH COMPLETED TRACKS IMMEDIATELY
    ► PLAY, PLAYLIST BUILD, CUE POINTS — ALL WORK ON ANALYZED TRACKS
    ► UNANALYZED TRACKS SHOW ⏳ BUT ARE STILL BROWSABLE
```

---

## 6. Non-Destructive File Export Flow

```
User clicks "Export Playlist" in PlaylistView
         │
         ▼
┌────────────────────────┐
│  ExportDialog.tsx       │
│                         │
│  Choose:                │
│  • Destination folder    │
│  • Naming pattern        │
│  • Write tags? (yes/no)  │
│  • Include cue points?   │
│  • DJ software format:   │
│    [Rekordbox|Serato|   │
│     Traktor|None]       │
└────────────┬───────────┘
             │
             ▼  Tauri invoke: export_playlist_files()
┌────────────────────────────────────────┐
│  Rust: export_playlist_files()         │
│                                        │
│  For each track in playlist:            │
│    1. Copy file to destination           │
│       (NEVER modify original)            │
│    2. Rename: "01 - Artist - Title.mp3"  │
│    3. Write tags to COPY (lofty):        │
│       - Key, Camelot, BPM                │
│    4. If cue points enabled:              │
│       - Write cue data to copy            │
│    5. Emit progress event                 │
│                                        │
│  After all files copied:                │
│    If DJ software format selected:       │
│    - Generate rekordbox.xml, OR           │
│    - Write Serato markers to files, OR    │
│    - Generate collection.nml              │
└────────────────────┬───────────────────┘
                     │
                     ▼
         Output folder structure:

         MyDJSet/
         ├── 01 - Artist A - Track 1.mp3
         ├── 02 - Artist B - Track 2.wav
         ├── 03 - Artist C - Track 3.flac
         ├── ...
         └── rekordbox.xml  (if Rekordbox selected)
              or collection.nml (if Traktor selected)
```

---

## 7. Build & Release Pipeline

```
┌─────────────┐     push / PR      ┌──────────────────────────┐
│  Developer  │ ──────────────────► │  GitHub Actions CI       │
│  (local)    │                     │                          │
└─────────────┘                     │  ┌────────────────────┐  │
                                    │  │ 1. cargo fmt/clippy │  │
                                    │  │ 2. cargo test       │  │
                                    │  │ 3. pnpm install     │  │
                                    │  │ 4. pnpm test        │  │
                                    │  │ 5. pnpm tauri build │  │
                                    │  └─────────┬──────────┘  │
                                    │            │             │
                                    │    ┌───────┼───────┐     │
                                    │    │       │       │     │
                                    │    ▼       ▼       ▼     │
                                    │  Win     macOS   Linux   │
                                    │  .msi    .dmg    .deb    │
                                    │                          │
                                    │  on tag: upload to       │
                                    │  GitHub Releases         │
                                    └──────────────────────────┘
```
