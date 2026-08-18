# Technical Blueprint: NotMixedInKey

> The definitive implementation guide for building the app from scratch.  
> A developer should be able to follow this document and produce a working application.

---

## 1. Tech Stack Decision

### 1.1 Framework: **Tauri 2** (Rust + Web Frontend)

**Why Tauri over Electron:**

| Factor | Tauri 2 | Electron |
|---|---|---|
| Binary size | ~3-10 MB | ~150+ MB |
| Memory usage | ~30-80 MB | ~200-400 MB |
| Startup time | < 1s | 2-5s |
| Backend language | Rust (fast, safe) | Node.js |
| Webview | System native (WebView2/WebKit) | Bundled Chromium |
| Cross-platform | Win/macOS/Linux (+ mobile) | Win/macOS/Linux |
| Audio processing | Rust crates (native speed) | Node addons (slower) |

**Tauri is the right choice** because our app does heavy audio processing (FFT, chromagram, CNN inference) — Rust gives us native performance without a garbage collector. The smaller binary and lower memory usage align with the "fast and stable" goals.

### 1.2 Frontend: **React + TypeScript + TailwindCSS + shadcn/ui**

| Choice | Why |
|---|---|
| **React 18+** | Largest ecosystem, best component libraries for data-heavy UIs |
| **TypeScript** | Type safety for complex state (library, playlists, audio routing) |
| **TailwindCSS** | Rapid UI development, consistent design system |
| **shadcn/ui** | High-quality, accessible components (tables, sliders, dialogs) |
| **Vite** | Fast dev server, HMR, Tauri-compatible bundler |
| **Zustand** | Lightweight state management (simpler than Redux for our needs) |
| **Lucide React** | Clean icon set |

### 1.3 Backend (Rust): Core Crates

| Crate | Purpose | Version |
|---|---|---|
| `tauri` | App framework, IPC, window management | 2.x |
| `symphonia` | Audio file decoding (MP3, FLAC, WAV, OGG, AAC, AIFF) | 0.5+ |
| `rustfft` | FFT computation for chromagram extraction | 6.x |
| `ndarray` | Numerical arrays for chroma vectors, matrix ops | 0.15+ |
| `ort` (ONNX Runtime) | CNN model inference for key detection | 2.x |
| `lofty` | ID3/metadata tag reading and writing | 0.18+ |
| `serde` / `serde_json` | Serialization for IPC and config | 1.x |
| `rayon` | Parallel batch processing | 1.x |
| `tokio` | Async runtime for file I/O | 1.x |
| `rodio` | Audio playback (backend alternative to Web Audio) | 0.19+ |
| `hound` | WAV reading (lightweight alternative for testing) | 3.x |

### 1.4 Frontend: Key Libraries

| Library | Purpose |
|---|---|
| `wavesurfer.js` | Waveform visualization for DJ decks |
| `d3.js` or `visx` | Camelot wheel visualization, song relationship graphs |
| `Web Audio API` | Audio playback, EQ (BiquadFilter), crossfading, gain control |
| `@tanstack/react-table` | Sortable/filterable music library table |
| `@tanstack/react-virtual` | Virtual scrolling for 50k+ track lists (only renders visible rows) |
| `react-dnd` | Drag-and-drop (tracks to decks) |
| `lru-cache` | LRU cache for waveform data (avoid re-fetching) |
| `essentia.js` (optional) | WASM-based BPM/key as validation layer |

---

## 2. Project Structure

```
notmixedinkey/
├── PREP/                          # Planning docs (this folder)
├── src-tauri/                     # Rust backend (Tauri)
│   ├── Cargo.toml
│   ├── tauri.conf.json
│   ├── src/
│   │   ├── main.rs                # Tauri entry point
│   │   ├── lib.rs                 # Library root
│   │   ├── commands/              # Tauri IPC commands
│   │   │   ├── mod.rs
│   │   │   ├── analyze.rs         # analyze_file, analyze_batch
│   │   │   ├── library.rs         # add_to_library, get_library, search
│   │   │   ├── playlist.rs        # generate_playlist, get_suggestions
│   │   │   ├── cuepoints.rs       # set_cue, delete_cue, get_cues, export_cues
│   │   │   ├── export.rs          # export_playlist_files (non-destructive copy)
│   │   │   ├── validation.rs      # compare_with_mik, get_validation_report
│   │   │   └── tags.rs            # write_tags, read_tags
│   │   ├── analysis/              # Audio analysis engine
│   │   │   ├── mod.rs
│   │   │   ├── decoder.rs         # Audio decoding (symphonia)
│   │   │   ├── hpss.rs            # Harmonic-Percussive Source Separation
│   │   │   ├── chromagram.rs      # STFT → chromagram extraction
│   │   │   ├── key_profiles.rs    # Krumhansl, Temperley, Sha'ath profiles
│   │   │   ├── key_classical.rs   # Classical key detection (chroma + profiles)
│   │   │   ├── key_cnn.rs         # CNN key detection (ONNX inference, multi-model)
│   │   │   ├── key_hybrid.rs      # 5-stage ensemble voting + MIK calibration
│   │   │   ├── key_temporal.rs    # Segmented temporal voting (per-segment analysis)
│   │   │   ├── bpm.rs             # BPM / tempo detection
│   │   │   └── energy.rs          # Energy level estimation
│   │   ├── validation/            # MIK comparison + self-calibration
│   │   │   ├── mod.rs
│   │   │   ├── mik_reader.rs      # Read existing MIK tags from files
│   │   │   ├── comparator.rs      # Compare our results vs MIK
│   │   │   └── calibrator.rs      # Adjust ensemble weights based on agreement
│   │   ├── export/                # Non-destructive file export
│   │   │   ├── mod.rs
│   │   │   ├── file_copier.rs     # Copy + rename files to export folder
│   │   │   ├── rekordbox_xml.rs   # Rekordbox XML playlist + cue export
│   │   │   ├── serato_markers.rs  # Serato cue marker writing
│   │   │   └── traktor_nml.rs     # Traktor NML collection export
│   │   ├── camelot/               # Camelot wheel logic
│   │   │   ├── mod.rs
│   │   │   ├── wheel.rs           # Key → Camelot mapping, compatibility checks
│   │   │   └── playlist.rs        # Harmonic playlist generation algorithm
│   │   ├── db/                    # Local database
│   │   │   ├── mod.rs
│   │   │   └── sqlite.rs          # SQLite via rusqlite
│   │   └── models/                # Data structures
│   │       ├── mod.rs
│   │       ├── track.rs           # Track struct (path, key, bpm, camelot, etc.)
│   │       ├── cuepoint.rs        # CuePoint struct (position_ms, name, color)
│   │       └── playlist.rs        # Playlist struct
│   ├── models/                    # Pre-trained ML models
│   │   ├── key_cnn_cqt.onnx       # CNN trained on CQT spectrograms (~5-10 MB)
│   │   ├── key_cnn_mel.onnx       # CNN trained on Mel spectrograms (~5-10 MB)
│   │   └── key_cnn_hpcp.onnx      # CNN trained on HPCP features (~3-5 MB)
│   └── migrations/                # SQLite migrations
│       └── 001_init.sql
├── src/                           # React frontend
│   ├── main.tsx                   # React entry point
│   ├── App.tsx                    # Root component + routing
│   ├── components/
│   │   ├── layout/
│   │   │   ├── Sidebar.tsx        # Navigation sidebar
│   │   │   ├── Header.tsx         # Top bar
│   │   │   └── MainLayout.tsx     # Layout wrapper
│   │   ├── library/
│   │   │   ├── LibraryTable.tsx   # Main track list (sortable, filterable)
│   │   │   ├── TrackRow.tsx       # Individual track row
│   │   │   ├── ImportDialog.tsx   # File/folder import dialog
│   │   │   └── AnalysisProgress.tsx # Batch analysis progress bar
│   │   ├── camelot/
│   │   │   ├── CamelotWheel.tsx   # Interactive SVG Camelot wheel
│   │   │   ├── HarmonicMap.tsx    # Song relationship visualization
│   │   │   └── KeyBadge.tsx       # Colored key/Camelot badge component
│   │   ├── playlist/
│   │   │   ├── PlaylistBuilder.tsx # Playlist creation interface
│   │   │   ├── PlaylistView.tsx   # Generated playlist display
│   │   │   └── RuleSelector.tsx   # Camelot rule selection (±1, ±2, A↔B)
│   │   ├── player/
│   │   │   ├── DualDeck.tsx       # Two-deck player layout
│   │   │   ├── Deck.tsx           # Single deck (waveform + controls)
│   │   │   ├── Waveform.tsx       # wavesurfer.js wrapper
│   │   │   ├── CuePoints.tsx      # Cue point markers on waveform
│   │   │   ├── CueButton.tsx      # Individual cue trigger button
│   │   │   ├── Mixer.tsx          # Crossfader + volume faders
│   │   │   ├── EQControl.tsx      # 3-band EQ knobs + kill switches
│   │   │   └── TransportBar.tsx   # Play/pause/stop/cue buttons
│   │   ├── export/
│   │   │   ├── ExportDialog.tsx    # Export playlist files dialog
│   │   │   └── ExportProgress.tsx  # Export progress indicator
│   │   ├── validation/
│   │   │   └── ValidationReport.tsx # MIK vs our results comparison view
│   │   └── ui/                    # shadcn/ui components
│   │       └── (generated by shadcn CLI)
│   ├── hooks/
│   │   ├── useAudioEngine.ts      # Web Audio API setup (context, nodes)
│   │   ├── useLibrary.ts          # Library CRUD via Tauri commands
│   │   ├── useAnalysis.ts         # Analysis trigger + progress tracking
│   │   └── usePlaylist.ts         # Playlist generation
│   ├── stores/
│   │   ├── libraryStore.ts        # Zustand: track list, filters, sort
│   │   ├── playerStore.ts         # Zustand: deck state, volumes, crossfader
│   │   └── playlistStore.ts       # Zustand: current playlist, rules
│   ├── lib/
│   │   ├── camelot.ts             # Camelot wheel utilities (JS side)
│   │   ├── audioEngine.ts         # Web Audio API graph setup
│   │   └── tauri.ts               # Tauri invoke wrappers (typed)
│   ├── types/
│   │   └── index.ts               # TypeScript interfaces (Track, Playlist, etc.)
│   └── styles/
│       └── globals.css            # Tailwind base + custom CSS
├── public/                        # Static assets
├── index.html                     # Vite entry HTML
├── package.json
├── tsconfig.json
├── tailwind.config.ts
├── vite.config.ts
└── README.md
```

---

## 3. Audio Analysis Engine (Detailed)

### 3.1 Audio Decoding Pipeline

```
Input File (.mp3/.wav/.flac/etc.)
        │
        ▼
  ┌─────────────┐
  │  symphonia   │  Decode to PCM f32 samples
  │  (Rust)      │  Resample to 44100 Hz mono
  └─────┬───────┘
        │
        ▼
  PCM Buffer: Vec<f32>
```

**Key implementation details:**
- Decode to **mono** (average L+R channels)
- Resample to **44100 Hz** (standard for analysis)
- Normalize amplitude to [-1.0, 1.0]
- For very long files (>10 min), analyze a representative sample (first 3 min + last 1 min) for speed

### 3.2 Stage 1: HPSS (Harmonic-Percussive Source Separation)

**Why this is the breakthrough:** Drums and percussive transients are the #1 source of error in key detection. They add broadband noise that pollutes the chromagram. By separating the harmonic content from percussive content *before* chroma extraction, we dramatically improve the signal that feeds both the classical and CNN paths.

```
PCM Buffer
    │
    ▼
┌──────────────────┐
│   STFT           │  Compute magnitude spectrogram
│   (4096 / 512)   │
└──────┬───────────┘
       │
       ▼
┌──────────────────────────────────┐
│ Median Filtering (HPSS)            │
│                                    │
│ • Horizontal median filter → H     │  (harmonic: sustained tones)
│ • Vertical median filter   → P     │  (percussive: transients)
│ • Soft mask: H_mask = H²/(H²+P²)  │
│ • Harmonic spectrogram = S * H_mask │
└──────┬───────────────────────────┘
       │
       ▼
  Clean Harmonic Spectrogram (drums removed)
       │
       ├──────► Classical Path (Stage 2)
       └──────► CNN Path (Stage 3)
```

**Implementation:** Median filter kernel sizes: horizontal = 31 frames (~0.7s), vertical = 31 bins. This is a well-established DSP technique (Fitzgerald 2010) that costs minimal CPU but yields ~5-10% accuracy improvement on percussive music.

### 3.3 Stage 2: Multi-Profile Classical Detection

```
Clean Harmonic Spectrogram
    │
    ▼
┌──────────────────┐
│ Chromagram        │  Map FFT bins → 12 pitch classes
│ Extraction        │  Sum magnitudes per pitch class per frame
└──────┬───────────┘
       │
       ▼
  Chromagram Matrix (12 × N frames)
       │
       ▼
┌──────────────────┐
│ Mean + Normalize  │  Average across time → 12-element vector
│                   │  L2 normalize
└──────┬───────────┘
       │
       ▼
  Chroma Vector (12 elements, unit length)
       │
       ├─────────────────┬─────────────────┐
       ▼                 ▼                 ▼
┌────────────┐  ┌────────────┐  ┌────────────┐
│ Krumhansl  │  │ Temperley  │  │ Sha'ath    │
│ profiles   │  │ profiles   │  │ profiles   │
│ (cos sim)  │  │ (cos sim)  │  │ (cos sim)  │
└─────┬──────┘  └─────┬──────┘  └─────┬──────┘
      │                │                │
      ▼                ▼                ▼
  Key_K (conf)     Key_T (conf)     Key_S (conf)
      │                │                │
      └────────────────┬────────────────┘
                       ▼
              Weighted vote across 3 profiles
              → Classical Result + Confidence
```

Instead of picking the single best profile, we **weighted-vote across all three**. Each profile's weight is calibrated (see Stage 5).

### 3.4 Stage 3: Multi-Model CNN Ensemble

Three separate CNN models, each trained on a **different input representation**:

```
Clean Harmonic Spectrogram
    │
    ├─────────────────┬─────────────────┐
    ▼                 ▼                 ▼
┌────────────┐  ┌────────────┐  ┌────────────┐
│ CQT       │  │ Mel        │  │ HPCP       │
│ Spectro   │  │ Spectro    │  │ Features   │
└────┬───────┘  └────┬───────┘  └────┬───────┘
     │                │                │
     ▼                ▼                ▼
┌────────────┐  ┌────────────┐  ┌────────────┐
│ CNN Model  │  │ CNN Model  │  │ CNN Model  │
│ key_cnn_   │  │ key_cnn_   │  │ key_cnn_   │
│ cqt.onnx   │  │ mel.onnx   │  │ hpcp.onnx  │
└────┬───────┘  └────┬───────┘  └────┬───────┘
     │                │                │
     ▼                ▼                ▼
  24-class          24-class          24-class
  softmax           softmax           softmax
     │                │                │
     └────────────────┬────────────────┘
                       ▼
              Average probability vectors
              → CNN Result + Confidence
```

**Why 3 models?** Each input representation captures different aspects of tonality. CQT has logarithmic frequency resolution (better for pitch). Mel captures perceptual energy distribution. HPCP is already a tonal feature. Their errors are **uncorrelated** — ensemble averaging reduces error significantly.

**Model training (pre-work):**
- Dataset: GiantSteps Key (~600 tracks) + GiantSteps MTG Key (~1500 tracks) + user's own MIK-tagged library (see Stage 5)
- Architecture per model: 4 conv layers (32→64→128→128 filters) + global avg pool + 256-unit dense + 24 softmax
- Training: Python + PyTorch → export to ONNX → quantize (INT8)
- Total model size: ~13-25 MB for all 3 models

### 3.5 Stage 4: Temporal Segment Voting

**The problem with global averaging:** A song may modulate keys, or have a long intro in a different key. Averaging the entire chroma destroys this information.

**Solution:** Analyze the song in **overlapping segments**, detect key per segment, then vote:

```
Full Track (e.g., 4 minutes)
│
├── Segment 1: 0:00 - 0:30   → Key estimate + confidence
├── Segment 2: 0:15 - 0:45   → Key estimate + confidence
├── Segment 3: 0:30 - 1:00   → Key estimate + confidence
│   ...                       (overlapping 30s windows, 15s hop)
└── Segment N: 3:30 - 4:00   → Key estimate + confidence
                               │
                               ▼
                    Confidence-Weighted Majority Vote
                    • Each segment casts a vote for its detected key
                    • Vote weight = segment's confidence score
                    • Segments with very low confidence (< 0.3) are discarded
                    • Final key = key with highest weighted vote total
                    • Final confidence = weighted_votes_for_winner / total_weighted_votes
```

**Bonus:** This also detects key modulations. If two keys each get >30% of votes, flag the track as "modulating" and report both keys.

### 3.6 Stage 5: Ensemble Fusion + MIK-Calibrated Self-Tuning

The final decision combines all paths with **learned weights**:

```
Stage 2 (Classical):  Key_C, Confidence_C, weight_C
Stage 3 (CNN):        Key_N, Confidence_N, weight_N
Stage 4 (Temporal):   Key_T, Confidence_T, weight_T
                         │
                         ▼
              ┌────────────────────────────────────┐
              │  WEIGHTED ENSEMBLE FUSION          │
              │                                    │
              │  For each of the 24 possible keys:  │
              │  score(key) =                       │
              │    w_C * P_C(key) * conf_C          │
              │  + w_N * P_N(key) * conf_N          │
              │  + w_T * P_T(key) * conf_T          │
              │                                    │
              │  Final key = argmax(score)          │
              └─────────────────┬──────────────────┘
                               │
                               ▼
                    Final Key + Confidence
```

**Self-calibration via MIK ground truth:**

This is the secret weapon. When the user imports tracks that already have MIK tags:

1. **Read MIK tags** from the audio file's ID3 comment/key fields (MIK writes Camelot codes like "8A" and standard keys like "Am")
2. **Run our full pipeline** on the same tracks
3. **Compare results** — for each sub-method (Krumhansl, Temperley, Sha'ath, CNN-CQT, CNN-Mel, CNN-HPCP, Temporal), compute accuracy vs MIK
4. **Update weights** — methods that agree more with MIK get higher weight in the ensemble
5. **Persist weights** to `ensemble_weights` table in SQLite
6. **Repeat** — as the user adds more MIK-tagged music, the weights refine further

```
Calibration formula:
  new_weight(method) = accuracy(method_vs_MIK) ^ 2
  // Squaring amplifies the difference between 85% and 70% accuracy
  // A method with 90% accuracy gets weight 0.81
  // A method with 60% accuracy gets weight 0.36

Default weights (before any MIK data):
  classical_krumhansl:  0.4
  classical_temperley:  0.5
  classical_shaath:     0.5
  cnn_cqt:              0.7
  cnn_mel:              0.6
  cnn_hpcp:             0.5
  temporal:             0.6
```

**Why this achieves >90% accuracy:**
- HPSS removes percussion noise: +5-10% over raw chromagram
- Multi-profile classical catches genre-specific patterns: +3-5%
- Multi-model CNN captures features classical can't: +10-15% over classical alone
- Temporal voting handles modulations and intros: +2-5%
- MIK calibration tunes weights to the user's actual music collection: +3-8%
- Ensemble diversity (uncorrelated errors) compounds these gains

**Conservative estimate: 88-93% exact match, 95-98% within compatible key.**

### 3.7 BPM Detection

```
PCM Buffer
    │
    ▼
┌─────────────────┐
│ Onset Detection  │  Spectral flux / energy-based onset function
└──────┬──────────┘
       │
       ▼
  Onset Strength Signal
       │
       ▼
┌──────────────────┐
│ Autocorrelation   │  Find dominant periodicity
│ or Tempogram      │
└──────┬───────────┘
       │
       ▼
  Raw BPM estimate
       │
       ▼
┌──────────────────┐
│ BPM Refinement   │  Constrain to 60-180 BPM range
│                  │  Resolve octave errors (half/double)
└──────┬───────────┘
       │
       ▼
  Final BPM (float, e.g., 127.5)
```

---

## 4. Database Schema (SQLite)

```sql
-- tracks table: core metadata
CREATE TABLE tracks (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    file_path     TEXT NOT NULL UNIQUE,
    filename      TEXT NOT NULL,
    title         TEXT,
    artist        TEXT,
    album         TEXT,
    duration_ms   INTEGER,
    -- Analysis results
    key_standard  TEXT,          -- e.g., "A minor", "C major"
    key_camelot   TEXT,          -- e.g., "8A", "8B"
    key_confidence REAL,         -- 0.0 to 1.0
    bpm           REAL,          -- e.g., 127.5
    energy_level  INTEGER,       -- 1-10 (nullable until v1.1)
    -- Metadata
    file_format   TEXT,          -- "mp3", "wav", "flac", etc.
    file_size     INTEGER,       -- bytes
    sample_rate   INTEGER,
    bit_depth     INTEGER,
    analyzed_at   TEXT,                  -- timestamp of last analysis
    status        TEXT DEFAULT 'pending', -- 'pending' | 'metadata_ready' | 'analyzing' | 'analyzed' | 'error'
    created_at    TEXT DEFAULT (datetime('now')),
    updated_at    TEXT DEFAULT (datetime('now'))
);

-- playlists table
CREATE TABLE playlists (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    name        TEXT NOT NULL,
    description TEXT,
    rules       TEXT,           -- JSON: which Camelot rules were used
    created_at  TEXT DEFAULT (datetime('now'))
);

-- playlist_tracks join table (ordered)
CREATE TABLE playlist_tracks (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    playlist_id INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
    track_id    INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    position    INTEGER NOT NULL,  -- order in playlist
    UNIQUE(playlist_id, track_id)
);

-- cue_points table: per-track cue points
CREATE TABLE cue_points (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    track_id    INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    position_ms INTEGER NOT NULL,  -- millisecond position in track
    name        TEXT,              -- user label, e.g., "Drop", "Intro", "Vocal"
    color       TEXT,              -- hex color, e.g., "#FF0000"
    hotcue_index INTEGER,          -- 0-7 (for DJ software compatibility)
    created_at  TEXT DEFAULT (datetime('now'))
);

-- validation_results: MIK comparison data (self-calibration)
CREATE TABLE validation_results (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    track_id        INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    mik_key         TEXT,          -- key from MIK tags (if present)
    mik_camelot     TEXT,          -- Camelot from MIK tags
    mik_energy      INTEGER,       -- energy from MIK tags
    our_key         TEXT,          -- our detected key
    our_camelot     TEXT,          -- our Camelot
    our_confidence  REAL,          -- our confidence
    match           BOOLEAN,       -- did we agree with MIK?
    validated_at    TEXT DEFAULT (datetime('now')),
    UNIQUE(track_id)
);

-- ensemble_weights: persisted calibration weights
CREATE TABLE ensemble_weights (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    profile_name    TEXT NOT NULL UNIQUE,  -- e.g., "classical_temperley", "cnn_cqt"
    weight          REAL NOT NULL DEFAULT 1.0,
    accuracy_pct    REAL,          -- measured accuracy against MIK ground truth
    sample_count    INTEGER DEFAULT 0,
    updated_at      TEXT DEFAULT (datetime('now'))
);

-- indexes for common queries
CREATE INDEX idx_tracks_key_camelot ON tracks(key_camelot);
CREATE INDEX idx_tracks_bpm ON tracks(bpm);
CREATE INDEX idx_tracks_artist ON tracks(artist);
CREATE INDEX idx_playlist_tracks_playlist ON playlist_tracks(playlist_id, position);
CREATE INDEX idx_cue_points_track ON cue_points(track_id);
CREATE INDEX idx_validation_track ON validation_results(track_id);
CREATE INDEX idx_tracks_status ON tracks(status);
```

---

## 5. Tauri IPC Commands (Rust → JS Bridge)

These are the Tauri commands the frontend calls. Each maps to a Rust function.

```rust
// === Analysis ===
#[tauri::command]
async fn analyze_file(path: String) -> Result<TrackAnalysis, String>;

#[tauri::command]
async fn analyze_batch(paths: Vec<String>, window: tauri::Window) -> Result<Vec<TrackAnalysis>, String>;
// ^ emits "analysis-progress" events to the window for progress tracking

// === Library ===
#[tauri::command]
async fn get_library(filter: Option<LibraryFilter>) -> Result<Vec<Track>, String>;

#[tauri::command]
async fn import_folder(path: String) -> Result<Vec<String>, String>;
// ^ returns list of audio file paths found

#[tauri::command]
async fn delete_tracks(ids: Vec<i64>) -> Result<(), String>;

// === Tags ===
#[tauri::command]
async fn write_tags(track_id: i64) -> Result<(), String>;
// ^ writes key + BPM back into the audio file's ID3/metadata

#[tauri::command]
async fn read_file_metadata(path: String) -> Result<FileMetadata, String>;

// === Playlist ===
#[tauri::command]
async fn generate_playlist(
    start_track_id: i64,
    rules: PlaylistRules,
    max_length: usize,
) -> Result<Vec<Track>, String>;

#[tauri::command]
async fn get_compatible_tracks(
    track_id: i64,
    rules: Vec<CamelotRule>,
) -> Result<Vec<Track>, String>;

// === Cue Points ===
#[tauri::command]
async fn set_cue_point(track_id: i64, position_ms: i64, name: Option<String>, color: Option<String>, hotcue_index: Option<u8>) -> Result<CuePoint, String>;

#[tauri::command]
async fn delete_cue_point(cue_id: i64) -> Result<(), String>;

#[tauri::command]
async fn get_cue_points(track_id: i64) -> Result<Vec<CuePoint>, String>;

#[tauri::command]
async fn export_cue_points(track_id: i64, format: CueExportFormat) -> Result<(), String>;
// ^ format: Rekordbox | Serato | Traktor

// === Validation (MIK Comparison) ===
#[tauri::command]
async fn run_mik_validation(track_ids: Vec<i64>) -> Result<ValidationReport, String>;
// ^ reads existing MIK tags, compares with our results, stores in validation_results

#[tauri::command]
async fn get_validation_report() -> Result<ValidationReport, String>;
// ^ aggregate stats: % agreement, disagreements list, per-method accuracy

#[tauri::command]
async fn recalibrate_ensemble() -> Result<EnsembleWeights, String>;
// ^ uses validation_results to update ensemble_weights table

// === File Export ===
#[tauri::command]
async fn export_playlist_files(
    playlist_id: i64,
    destination: String,       // output folder path
    options: ExportOptions,    // { write_tags, number_prefix, format, include_cues }
    window: tauri::Window,     // for progress events
) -> Result<ExportResult, String>;
// ^ copies files non-destructively, writes tags, emits progress

// === Audio ===
#[tauri::command]
async fn get_waveform_data(path: String, num_points: usize) -> Result<Vec<f32>, String>;
// ^ returns downsampled amplitude data for waveform rendering
```

---

## 6. Audio Playback Architecture (Frontend)

Audio playback uses the **Web Audio API** in the Tauri webview:

```
                    ┌─────────────────────────────────────┐
                    │          Web Audio Context            │
                    │                                       │
  ┌─────────┐      │  ┌──────┐   ┌─────┐   ┌──────────┐  │
  │ Deck A  │──────┼─►│ Gain │──►│ EQ  │──►│          │  │
  │ (source)│      │  │  A   │   │ 3band│  │          │  │
  └─────────┘      │  └──────┘   └─────┘   │ Crossfade│  │   ┌─────────┐
                    │                        │  Gain    │──┼──►│ Speakers│
  ┌─────────┐      │  ┌──────┐   ┌─────┐   │          │  │   └─────────┘
  │ Deck B  │──────┼─►│ Gain │──►│ EQ  │──►│          │  │
  │ (source)│      │  │  B   │   │ 3band│  │          │  │
  └─────────┘      │  └──────┘   └─────┘   └──────────┘  │
                    │                                       │
                    │  EQ = lo/mid/hi BiquadFilterNodes     │
                    └─────────────────────────────────────┘
```

**EQ Implementation (3-band):**
- **Low:** BiquadFilter type `lowshelf`, freq 320 Hz
- **Mid:** BiquadFilter type `peaking`, freq 1000 Hz, Q ~1.0
- **High:** BiquadFilter type `highshelf`, freq 3200 Hz
- **Kill switch:** Set gain to -40dB (effectively silent)

**Crossfader:** Adjusts gain of Deck A and Deck B inversely. At center, both play at equal volume. At left, only A. At right, only B. Use equal-power crossfade curve.

---

## 7. UI/UX Design Direction

### 7.1 Layout

```
┌─────────────────────────────────────────────────────────────┐
│  NotMixedInKey                              [settings] [?]  │
├──────────┬──────────────────────────────────────────────────┤
│          │                                                  │
│ Library  │  ┌─────────────────────────────────────────────┐│
│          │  │            Main Content Area                ││
│ Camelot  │  │                                             ││
│          │  │   [Library Table]                            ││
│ Playlists│  │   or [Camelot Wheel]                        ││
│          │  │   or [Playlist Builder]                     ││
│ Settings │  │   or [Harmonic Map]                         ││
│          │  │                                             ││
│          │  └─────────────────────────────────────────────┘│
│          ├──────────────────────────────────────────────────┤
│          │  ┌─────────────┐  ┌──────┐  ┌─────────────┐   │
│          │  │   DECK A    │  │MIXER │  │   DECK B    │   │
│          │  │ [waveform]  │  │      │  │ [waveform]  │   │
│          │  │ [controls]  │  │[xfdr]│  │ [controls]  │   │
│          │  └─────────────┘  └──────┘  └─────────────┘   │
└──────────┴──────────────────────────────────────────────────┘
```

### 7.2 Color Palette

Dark theme (DJ-friendly, reduces eye strain in dark environments):
- **Background:** `#0f0f0f` (near black)
- **Surface:** `#1a1a2e` (dark navy)
- **Accent primary:** `#e94560` (vibrant red-pink — energy)
- **Accent secondary:** `#0f3460` (deep blue — calm)
- **Text primary:** `#eaeaea`
- **Text secondary:** `#7a7a7a`
- **Camelot wheel colors:** 12 distinct hues mapped to the 12 clock positions (rainbow gradient)

### 7.3 Visual Inspiration

The **mashupbreakdown.com** concept (from the project spec): showing layered songs playing together visually. We adapt this as:
- In the **Harmonic Map** view, songs are nodes on a graph
- Lines connect harmonically compatible songs
- Line thickness = compatibility strength (same key = thick, ±1 = medium, ±2 = thin)
- Clicking a node highlights its connections and dims everything else
- Nodes are colored by their Camelot position

---

## 8. Development Environment Setup

A new developer should run these steps to get started:

### 8.1 Prerequisites

- **Rust** (latest stable via [rustup.rs](https://rustup.rs))
- **Node.js** 20+ (via [nvm](https://github.com/nvm-sh/nvm) or direct install)
- **pnpm** (package manager, faster than npm)
- **Visual Studio Build Tools** (Windows — for Rust compilation)
  - Or Xcode Command Line Tools (macOS)

### 8.2 Project Init (Step by Step)

```bash
# 1. Create the Tauri + React project
pnpm create tauri-app notmixedinkey --template react-ts
cd notmixedinkey

# 2. Install frontend dependencies
pnpm add wavesurfer.js d3 @tanstack/react-table zustand react-dnd react-dnd-html5-backend lucide-react
pnpm add -D @types/d3 tailwindcss @tailwindcss/vite

# 3. Install shadcn/ui
pnpm dlx shadcn@latest init
pnpm dlx shadcn@latest add button slider table dialog progress badge tabs tooltip

# 4. Add Rust dependencies (in src-tauri/Cargo.toml)
# See Section 1.3 for the full crate list

# 5. Run in development
pnpm tauri dev
```

### 8.3 Rust Crate Dependencies (Cargo.toml additions)

```toml
[dependencies]
tauri = { version = "2", features = ["shell-open"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
symphonia = { version = "0.5", features = ["mp3", "flac", "ogg", "aac", "wav", "aiff", "pcm"] }
rustfft = "6"
ndarray = "0.15"
ort = "2"                              # ONNX Runtime
lofty = "0.18"                         # Audio metadata tags
rusqlite = { version = "0.31", features = ["bundled"] }
rayon = "1"
tokio = { version = "1", features = ["full"] }
hound = "3"                            # WAV support
rodio = { version = "0.19", optional = true }

[features]
default = []
backend-playback = ["rodio"]           # Optional: use Rust for audio playback
```

---

## 9. Implementation Phases (Detailed)

### Phase 1: Foundation (Weeks 1-3)

**Goal:** Tauri app boots, loads audio files, displays basic UI.

- [ ] Scaffold Tauri 2 + React + TS project
- [ ] Set up Tailwind + shadcn/ui
- [ ] Create main layout (sidebar + content area + player dock)
- [ ] Implement `import_folder` command — recursively find audio files
- [ ] Implement `read_file_metadata` — read ID3 tags with `lofty`
- [ ] Create LibraryTable component — display imported tracks
- [ ] Set up SQLite database with schema
- [ ] Basic drag-and-drop file import

### Phase 2: Analysis Engine (Weeks 4-7)

**Goal:** Detect key and BPM of any supported audio file.

- [ ] Implement `decoder.rs` — decode audio to PCM with symphonia
- [ ] Implement `chromagram.rs` — STFT + chroma extraction with rustfft
- [ ] Implement `key_profiles.rs` — Krumhansl, Temperley, Sha'ath vectors
- [ ] Implement `key_classical.rs` — profile matching via cosine similarity
- [ ] Implement `bpm.rs` — onset detection + autocorrelation
- [ ] Implement `analyze_file` command — full pipeline
- [ ] Implement `analyze_batch` command — parallel with rayon, emit progress events
- [ ] Unit tests: test against known-key tracks
- [ ] Benchmark accuracy against a test set (50+ tracks with known keys)

### Phase 3: Library & Visualization (Weeks 8-10)

**Goal:** Interactive Camelot wheel + harmonic relationship view.

- [ ] Implement Camelot wheel mapping in Rust (`wheel.rs`)
- [ ] Create CamelotWheel SVG component (D3 or custom SVG)
- [ ] Populate wheel with analyzed tracks (colored dots per segment)
- [ ] Click a segment → highlight compatible segments
- [ ] Click a track → show all compatible tracks
- [ ] Create HarmonicMap (force-directed graph of song relationships)
- [ ] KeyBadge component (colored pill showing Camelot code)
- [ ] Enhanced LibraryTable with key/BPM columns, sorting, filtering

### Phase 4: Playlist Builder (Weeks 11-12)

**Goal:** Auto-generate DJ sets based on harmonic rules.

- [ ] Implement playlist generation algorithm in Rust
- [ ] Rule selector UI (checkboxes: same key, ±1, ±2, A↔B)
- [ ] Energy curve option (ascending, descending, peak-and-valley)
- [ ] Starting track selector
- [ ] Generated playlist view with drag-to-reorder
- [ ] Save/load playlists
- [ ] Export playlist (M3U format)

### Phase 5: DJ Preview Player + Cue Points (Weeks 13-16)

**Goal:** Dual-deck player with crossfader, EQ, and full cue point system.

- [ ] Set up Web Audio API graph (see Section 6)
- [ ] Integrate wavesurfer.js for waveform display
- [ ] Deck component: load track, play/pause/stop, seek
- [ ] Volume faders (per deck)
- [ ] Crossfader with equal-power curve
- [ ] 3-band EQ (BiquadFilter nodes)
- [ ] Kill switches (toggle to cut a band)
- [ ] Drag track from library → load into deck
- [ ] Visual: waveform scrolls during playback
- [ ] **Cue point system:**
  - [ ] Click on waveform to set cue point (visual marker)
  - [ ] Up to 8 hot cues per track (industry standard)
  - [ ] Name and color-code each cue
  - [ ] Click cue marker / press hotcue button to jump to that position
  - [ ] Cue data persisted in `cue_points` SQLite table
  - [ ] CuePoints.tsx: renders markers on waveform
  - [ ] CueButton.tsx: hotcue trigger pads (color-coded)

### Phase 6: CNN + Accuracy Engine (Weeks 17-20)

**Goal:** Achieve >90% key detection accuracy via 5-stage ensemble.

- [ ] Implement `hpss.rs` — Harmonic-Percussive Source Separation (median filtering)
- [ ] Train 3 CNN models (Python + PyTorch) on GiantSteps + MIK-tagged data:
  - [ ] CQT spectrogram model
  - [ ] Mel spectrogram model
  - [ ] HPCP feature model
- [ ] Export all 3 to ONNX, quantize to INT8
- [ ] Implement `key_cnn.rs` — multi-model loading, preprocess, inference
- [ ] Implement `key_temporal.rs` — segmented analysis + confidence-weighted voting
- [ ] Implement `key_hybrid.rs` — 5-stage ensemble fusion with weighted voting
- [ ] Implement `validation/mik_reader.rs` — read existing MIK tags from imported files
- [ ] Implement `validation/comparator.rs` — compare our results vs MIK per method
- [ ] Implement `validation/calibrator.rs` — update ensemble weights based on agreement
- [ ] ValidationReport.tsx — show user the accuracy comparison dashboard
- [ ] Benchmark accuracy against 200+ tracks (MIK-tagged + manual ground truth)
- [ ] Bundle all 3 model files with app

### Phase 7: File Export + DJ Software Integration (Weeks 21-23)

**Goal:** Non-destructive file export and DJ software cue point export.

- [ ] Implement `export/file_copier.rs` — copy files to destination, numbered by playlist order
- [ ] Write key/BPM/Camelot tags to copies (not originals) via lofty
- [ ] Implement `export/rekordbox_xml.rs` — Rekordbox XML collection format with cue points
- [ ] Implement `export/serato_markers.rs` — write Serato-compatible cue markers to ID3
- [ ] Implement `export/traktor_nml.rs` — Traktor NML collection export with cue points
- [ ] ExportDialog.tsx — choose destination, format, tag options
- [ ] ExportProgress.tsx — progress indicator for batch copy
- [ ] Folder structure options:
  - `{PlaylistName}/{01} - {Artist} - {Title}.{ext}`
  - `{PlaylistName}/{CamelotCode} - {Artist} - {Title}.{ext}`
  - Custom pattern
- [ ] USB export mode (flat folder for CDJ USB sticks)

### Phase 8: Polish & Ship (Weeks 24-26)

**Goal:** Production-ready release.

- [ ] Implement `write_tags` — write key/BPM to ID3 tags (on originals, user opt-in)
- [ ] Settings page (analysis preferences, tag format, Camelot vs standard notation, export defaults)
- [ ] Error handling and user-facing error messages
- [ ] Windows installer (MSI via Tauri)
- [ ] macOS installer (DMG via Tauri)
- [ ] README with screenshots and usage guide
- [ ] License selection and audit (GPL dependencies)
- [ ] Performance profiling and optimization
- [ ] End-to-end testing

---

## 10. Testing Strategy

| Level | Tool | What |
|---|---|---|
| **Rust unit tests** | `cargo test` | Key profile math, chroma extraction, Camelot mapping |
| **Rust integration tests** | `cargo test` | Full analysis pipeline against known-key audio files |
| **Accuracy benchmark** | Custom script | Run analyzer against 100+ tracks with known keys, measure % correct |
| **Frontend unit tests** | Vitest | Store logic, Camelot utilities, component rendering |
| **E2E tests** | Playwright / WebDriver | Full app flows (import → analyze → view → playlist → play) |

### Test Audio Files

Create a `tests/fixtures/` directory with:
- 24 synthetic audio files (one per key — generated with a synth playing the triad)
- 10-20 real-world tracks with manually verified keys (from DJ TechTools test set methodology)

---

## 11. Deployment & Distribution

- **Windows:** `.msi` installer via Tauri's WiX integration, or `.exe` via NSIS
- **macOS:** `.dmg` disk image via Tauri's bundler
- **Auto-update:** Tauri's built-in updater (checks GitHub releases)
- **CI/CD:** GitHub Actions — build on push to `main`, release on tag

---

## 12. Scalability & Large Library Handling

> **Design goal:** A user drops 50,000 audio files into the app. Within 2 seconds they see a populated library table. Within 5 seconds the first tracks show key/BPM results. The UI never freezes. They can browse, filter, play, and build playlists while analysis continues in the background.

### 12.1 Three-Pass Import Strategy

Instead of one monolithic "analyze everything" step, imports happen in **three fast passes**:

```
Pass 1: SCAN (instant — <2s for 50k files)
  ├── Recursively find all audio files by extension
  ├── Return file paths + file size + mtime
  ├── Deduplicate against existing DB entries (by file_path)
  └── INSERT skeleton rows: file_path, filename, file_format, file_size
      status = "pending"

  ► UI immediately shows all 50k rows in the library table
  ► Rows display filename, format, size — key/BPM columns show spinner

Pass 2: METADATA (fast — ~1-3ms per file, ~1 min for 50k)
  ├── Read ID3/metadata tags with lofty (title, artist, album, duration)
  ├── Read existing MIK tags if present (key, Camelot, energy)
  ├── UPDATE track rows with metadata
  ├── Emit Tauri events in batches of 50: "metadata-batch-complete"
  └── status = "metadata_ready"

  ► UI updates rows as batches complete (title, artist columns fill in)
  ► Tracks with existing MIK key tags show those immediately
  ► User can already sort, filter, search by artist/title

Pass 3: DEEP ANALYSIS (slow — ~2-5s per track, parallelized)
  ├── Decode audio, run 5-stage key detection, BPM detection
  ├── Process in parallel via rayon (N = num_cpu_cores - 1)
  ├── Emit Tauri event per completed track: "track-analyzed"
  ├── INSERT analysis results immediately per track
  └── status = "analyzed"

  ► UI updates individual rows as each track finishes
  ► User can interact with analyzed tracks immediately
  ► Progress bar shows: "Analyzing: 342 / 50,000 (12 tracks/sec)"
```

### 12.2 Smart Queue Prioritization

Not all tracks should be analyzed in the same order. The queue is **priority-sorted**:

```rust
enum AnalysisPriority {
    Critical,   // User loaded this track into a deck — analyze NOW
    High,       // User is currently viewing this page of the library table
    Medium,     // Tracks in the viewport's nearby scroll range
    Low,        // Everything else (background batch)
}
```

**Priority rules:**
1. **Deck load** — if a user drags an unanalyzed track to a deck, it jumps to front of queue
2. **Viewport-aware** — the frontend sends the currently visible row range to the backend; those tracks get `High` priority
3. **Playlist seed** — if user starts building a playlist from a track, its neighbors (by artist, folder, or filename) get bumped
4. **User click** — clicking an unanalyzed track row bumps it to `High`

**Implementation:**
```rust
// Priority queue (min-heap by priority, FIFO within same priority)
use std::collections::BinaryHeap;

struct AnalysisJob {
    track_id: i64,
    file_path: String,
    priority: AnalysisPriority,
    queued_at: Instant,
}

// IPC command: frontend tells backend what's visible
#[tauri::command]
async fn set_visible_range(start_idx: usize, end_idx: usize) -> Result<(), String>;
// ^ Bumps tracks in this range to High priority
```

### 12.3 Parallel Processing Pipeline

```
                    Analysis Thread Pool (rayon)
                    ┌─────────────────────────────────┐
                    │  Cores: N = num_cpus - 1         │
                    │  (leave 1 core for UI + OS)       │
                    │                                   │
   Priority Queue   │  ┌────────┐ ┌────────┐ ┌────────┐│
   ──────────────►  │  │ Core 1 │ │ Core 2 │ │ Core N ││
                    │  │Track A │ │Track B │ │Track C ││
                    │  │ decode │ │ decode │ │ decode ││
                    │  │ HPSS   │ │ HPSS   │ │ HPSS   ││
                    │  │ chroma │ │ chroma │ │ chroma ││
                    │  │ CNN    │ │ CNN    │ │ CNN    ││
                    │  │ BPM    │ │ BPM    │ │ BPM    ││
                    │  └───┬────┘ └───┬────┘ └───┬────┘│
                    │      │          │          │      │
                    └──────┼──────────┼──────────┼──────┘
                           │          │          │
                           ▼          ▼          ▼
                    ┌─────────────────────────────────┐
                    │  Results Channel (mpsc)           │
                    │  Each completed track is sent     │
                    │  immediately — no waiting for     │
                    │  the batch to finish               │
                    └──────────────┬────────────────────┘
                                   │
                           ┌───────┼───────┐
                           ▼               ▼
                    ┌────────────┐  ┌────────────────┐
                    │ SQLite     │  │ Tauri Event     │
                    │ INSERT     │  │ "track-analyzed"│
                    │ (per track)│  │ {id, key, bpm}  │
                    └────────────┘  └────────────────┘
                                          │
                                          ▼
                                   Frontend receives
                                   → updates single row
                                   → no full re-render
```

**Key design decisions:**
- **N-1 cores:** Always leave one core free for the UI thread and OS. On an 8-core machine, 7 tracks analyze simultaneously.
- **Streaming results:** Each completed track is sent over an `mpsc` channel immediately. No batching delay.
- **Per-track DB write:** Each track is `INSERT`ed as it finishes. If the app crashes mid-batch, all completed work is preserved.
- **CNN model sharing:** The ONNX models are loaded once and shared across threads (read-only inference is thread-safe with `ort`).

### 12.4 Memory Management for Large Libraries

**Problem:** 50,000 tracks × ~2 KB metadata each = ~100 MB. This is fine for RAM. But loading 50,000 audio files simultaneously is not.

**Solutions:**

| Concern | Strategy |
|---|---|
| **Audio decoding** | Decode one track at a time per thread. Release PCM buffer immediately after analysis. Peak memory per thread: ~50 MB (4-min track at 44.1kHz mono). |
| **CNN inference** | Pre-allocate spectrogram tensor buffer, reuse across tracks (no alloc per track). |
| **Library metadata** | Load all track metadata into Zustand store. 50k tracks × ~2 KB = ~100 MB. Acceptable. |
| **Waveform data** | Do NOT store waveform data in the library store. Generate on demand when a track is loaded into a deck. Cache the last 10 waveforms in an LRU cache. |
| **SQLite** | Use WAL mode (write-ahead logging) for concurrent read+write. Frontend reads don't block backend writes. |

```rust
// Memory-conscious analysis loop
fn analyze_track(path: &str) -> Result<TrackAnalysis> {
    // 1. Decode — allocates PCM buffer
    let pcm = decode_to_mono_44100(path)?;  // ~50 MB for 4-min track
    
    // 2. HPSS + Chromagram — allocates spectrogram
    let spectrogram = compute_stft(&pcm);    // ~20 MB
    let (harmonic, _percussive) = hpss(&spectrogram);
    drop(spectrogram);  // Free immediately
    
    // 3. Key detection (reuses harmonic spectrogram)
    let key_result = detect_key(&harmonic, &pcm);
    drop(harmonic);     // Free immediately
    
    // 4. BPM detection (uses PCM directly)
    let bpm_result = detect_bpm(&pcm);
    drop(pcm);          // Free immediately — biggest allocation gone
    
    // 5. Return small result struct (~200 bytes)
    Ok(TrackAnalysis { key_result, bpm_result, ... })
}
```

### 12.5 Frontend: Virtual Scrolling + Progressive Rendering

**Problem:** React cannot render 50,000 `<tr>` elements. The DOM would freeze.

**Solution:** `@tanstack/react-virtual` — only renders the rows currently visible in the viewport (~30-50 rows).

```
Library Table (50,000 tracks)
┌───────────────────────────────────────────────────────┐
│  Header: # | Title | Artist | Key | Camelot | BPM    │
├───────────────────────────────────────────────────────┤
│  ... (rows 1-240 are above viewport, NOT rendered)    │
│                                                       │
│  ┌─────────────────────────────────────────────────┐  │  ← Visible
│  │ 241 | Strobe        | deadmau5 | [8A] | 128.0  │  │    viewport
│  │ 242 | Ghosts n Stuff | deadmau5 | [⏳] | [⏳]   │  │    (only these
│  │ 243 | Levels        | Avicii   | [6B] | 126.0  │  │     ~30 rows
│  │ 244 | Titanium      | David G  | [4B] | 126.0  │  │     exist in
│  │ ...                                            │  │     the DOM)
│  │ 270 | Clarity       | Zedd     | [9A] | 128.0  │  │
│  └─────────────────────────────────────────────────┘  │  ← End visible
│                                                       │
│  ... (rows 271-50000 are below viewport, NOT rendered)│
└───────────────────────────────────────────────────────┘

Scroll position → determines which rows to render
Total height = row_count × row_height (faked via CSS)
```

**Row states (progressive rendering):**

```tsx
function TrackRow({ track }: { track: Track }) {
  return (
    <tr>
      <td>{track.filename}</td>
      <td>{track.title ?? '—'}</td>
      <td>{track.artist ?? '—'}</td>
      <td>
        {track.status === 'analyzed' && <KeyBadge camelot={track.key_camelot} />}
        {track.status === 'metadata_ready' && <span className="text-muted">⏳</span>}
        {track.status === 'pending' && <Skeleton className="w-10 h-5" />}
      </td>
      <td>
        {track.status === 'analyzed' ? track.bpm?.toFixed(1) : '—'}
      </td>
    </tr>
  );
}
```

### 12.6 Batched State Updates (Preventing React Meltdown)

**Problem:** If analysis completes 12 tracks/second and each emits a Tauri event, that's 12 re-renders/second of a 50k-row table. React will choke.

**Solution:** Batch incoming events and flush on an animation frame:

```typescript
// stores/libraryStore.ts
interface LibraryStore {
  tracks: Map<number, Track>;     // Map for O(1) updates by ID
  pendingUpdates: Track[];         // Buffer for incoming results
  flushUpdates: () => void;        // Apply all pending updates at once
}

// In the Tauri event listener:
let rafId: number | null = null;

listen('track-analyzed', (event) => {
  const track = event.payload as Track;
  libraryStore.getState().pendingUpdates.push(track);
  
  // Coalesce: only schedule one flush per animation frame
  if (!rafId) {
    rafId = requestAnimationFrame(() => {
      libraryStore.getState().flushUpdates();  // Apply all buffered updates at once
      rafId = null;
    });
  }
});

// flushUpdates applies all pending changes in a single state update
flushUpdates: () => {
  set((state) => {
    const newTracks = new Map(state.tracks);
    for (const track of state.pendingUpdates) {
      newTracks.set(track.id, track);
    }
    return { tracks: newTracks, pendingUpdates: [] };
  });
}
```

**Result:** Even if 100 tracks complete in one second, React only re-renders ~60 times/sec max (one per frame), and each render only updates the ~30 visible rows.

### 12.7 SQLite Optimization for 50k+ Tracks

```sql
-- Enable WAL mode (concurrent reads + writes)
PRAGMA journal_mode = WAL;

-- Increase cache size (default 2MB → 64MB)
PRAGMA cache_size = -65536;

-- Synchronous mode: NORMAL is safe for WAL, much faster than FULL
PRAGMA synchronous = NORMAL;

-- Temp store in memory
PRAGMA temp_store = MEMORY;

-- Page size optimization
PRAGMA page_size = 4096;
```

**Batch operations:**
```rust
// Insert 50 tracks in a single transaction (10x faster than individual inserts)
fn insert_metadata_batch(tracks: &[TrackMetadata], conn: &Connection) -> Result<()> {
    let tx = conn.transaction()?;
    {
        let mut stmt = tx.prepare_cached(
            "INSERT OR IGNORE INTO tracks (file_path, filename, title, artist, album, 
             duration_ms, file_format, file_size, status) 
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'metadata_ready')"
        )?;
        for t in tracks {
            stmt.execute(params![t.file_path, t.filename, t.title, t.artist, 
                                 t.album, t.duration_ms, t.format, t.size])?;
        }
    }
    tx.commit()
}
```

**Pagination for initial load:**
```rust
#[tauri::command]
async fn get_library_page(
    page: usize, 
    page_size: usize,      // default 200
    sort_by: String,
    sort_dir: String,
    filter: Option<LibraryFilter>,
) -> Result<LibraryPage, String>;

// LibraryPage { tracks: Vec<Track>, total_count: usize, page: usize }
```

The frontend loads page 0 immediately, then prefetches pages ±1 from the current scroll position. `@tanstack/react-virtual` drives which page to request.

### 12.8 Skip & Cache Strategy

**Don't re-analyze what's already done:**

```rust
fn should_analyze(file_path: &str, db: &Connection) -> bool {
    let result = db.query_row(
        "SELECT status, file_size, analyzed_at FROM tracks WHERE file_path = ?1",
        [file_path],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, Option<String>>(2)?)),
    );
    
    match result {
        Ok(("analyzed", db_size, Some(_))) => {
            // Already analyzed — but has the file changed?
            let current_size = std::fs::metadata(file_path).map(|m| m.len() as i64).unwrap_or(0);
            current_size != db_size  // Re-analyze only if file size changed
        }
        Ok(("pending" | "metadata_ready", _, _)) => true,  // Not yet analyzed
        Err(_) => true,  // Not in DB at all
        _ => true,
    }
}
```

**Waveform LRU cache (frontend):**
```typescript
// Cache last 20 waveform datasets to avoid re-fetching when switching between decks
const waveformCache = new LRUCache<string, Float32Array>({ max: 20 });

async function getWaveform(path: string): Promise<Float32Array> {
  const cached = waveformCache.get(path);
  if (cached) return cached;
  
  const data = await invoke<number[]>('get_waveform_data', { path, numPoints: 4000 });
  const arr = new Float32Array(data);
  waveformCache.set(path, arr);
  return arr;
}
```

### 12.9 Performance Targets

| Metric | Target | Strategy |
|---|---|---|
| **Time to first row visible** | < 2 seconds (50k files) | Pass 1 scan is filesystem-only, no audio decoding |
| **Time to first analysis result** | < 8 seconds | Highest-priority track starts analyzing in Pass 3 while Pass 2 runs |
| **Analysis throughput** | 8-15 tracks/sec (8-core machine) | 7 parallel workers, ~2-5s per track |
| **50k library full analysis** | ~1-2 hours | 50000 / 12 tracks/sec ≈ 70 min |
| **Library table scroll** | 60 FPS, no jank | Virtual scrolling, only 30-50 DOM rows |
| **Memory usage (50k lib)** | < 400 MB | Metadata in store, audio buffers released per-track |
| **Re-import (no changes)** | < 10 seconds (50k files) | Skip cache: check file_path + file_size only |
| **DB query (filter/sort 50k)** | < 50ms | Indexed columns, prepared statements, WAL mode |

### 12.10 User Experience During Analysis

The UI communicates state clearly at all times:

```
┌──────────────────────────────────────────────────────────────┐
│  ⚡ Analyzing: 1,247 / 50,000  │  ██████░░░░░  2.5%         │
│     Speed: 12.3 tracks/sec     │  ETA: ~66 min remaining     │
│     [Pause]  [Cancel]          │  [Prioritize: Current View]  │
└──────────────────────────────────────────────────────────────┘
```

**Key UX principles:**
- **Analyzed tracks are fully functional** — user can play them, add to playlists, see them on the Camelot wheel
- **Unanalyzed tracks are still browsable** — title, artist, duration are visible from Pass 2
- **Sorting by key/BPM** groups analyzed tracks at top, unanalyzed at bottom (with a visual separator)
- **Pause/resume** — user can pause analysis (e.g., during a DJ performance) and resume later
- **Cancel** — stops analysis, preserves all completed results
- **Priority boost** — "Analyze This First" context menu option on any track/selection
- **Background mode** — analysis continues at lower priority if the app is minimized

### 12.11 IPC Commands: Scalability

```rust
// === Scalability ===
#[tauri::command]
async fn scan_folder(path: String) -> Result<ScanResult, String>;
// ^ Pass 1: filesystem scan only. Returns file count + inserts skeleton rows.
// Returns immediately with: { total_files: 50000, new_files: 48200, skipped: 1800 }

#[tauri::command]
async fn read_metadata_batch(track_ids: Vec<i64>, window: tauri::Window) -> Result<(), String>;
// ^ Pass 2: reads ID3 tags for given tracks. Emits "metadata-batch-complete" events.

#[tauri::command]  
async fn start_analysis(window: tauri::Window) -> Result<(), String>;
// ^ Pass 3: starts background analysis of all pending tracks. Non-blocking.

#[tauri::command]
async fn pause_analysis() -> Result<(), String>;

#[tauri::command]
async fn resume_analysis() -> Result<(), String>;

#[tauri::command]
async fn cancel_analysis() -> Result<(), String>;

#[tauri::command]
async fn set_visible_range(start_idx: usize, end_idx: usize) -> Result<(), String>;
// ^ Frontend tells backend which library rows are visible → priority boost

#[tauri::command]
async fn prioritize_tracks(track_ids: Vec<i64>) -> Result<(), String>;
// ^ Bump specific tracks to front of analysis queue

#[tauri::command]
async fn get_library_page(
    page: usize, 
    page_size: usize, 
    sort: SortOptions, 
    filter: Option<LibraryFilter>,
) -> Result<LibraryPage, String>;

#[tauri::command]
async fn get_analysis_status() -> Result<AnalysisStatus, String>;
// ^ Returns: { total, completed, in_progress, speed_per_sec, eta_seconds }
```
