# TuneLock

> Fast, accurate musical key + BPM analysis for producers and DJs.
> Built for the MPC → Ableton Live → Tape 16 → Traktor workflow.

Built with **Tauri 2** (Rust backend) + **React** + **TypeScript** + **TailwindCSS**.

## The three modes

| Mode | What it does |
|---|---|
| **Tuner** | Drop a file or feed audio in (mic / line-in) and get the key + Camelot + BPM instantly. Like a guitar tuner, but for songs. |
| **Library** | Visualize and arrange your samples, stems, and tracks by key. Drag-and-drop playlists with live Camelot relationship hints (+1/-1, +2 energy boost, A↔B mood shift). |
| **Delivery** | Non-destructive export: copy, rename, optionally transcode, and emit `.m3u8` + `.csv` ready for Traktor, Ableton, Rekordbox, or USB to MPC / CDJs. |

## Features

- **Hybrid key detection**: HPSS source separation + 3-profile ensemble (Krumhansl, Temperley, Sha'ath) + temporal segment voting. Targeting ≥90% accuracy.
- **Camelot notation** alongside standard key (`8A`, `C minor`).
- **BPM detection** via onset energy + autocorrelation.
- **Camelot wheel** with live relationship overlay: same-key, ±1, ±2 (energy boost / drop), A↔B (mood shift).
- **Virtual-scrolling library** for large sample collections.
- **Non-destructive export** with M3U8 + CSV emission.

## Tech Stack

| Layer | Technology |
|-------|------------|
| Framework | Tauri 2 |
| Frontend | React 18, TypeScript, TailwindCSS |
| State | Zustand |
| Audio Decoding | Symphonia (Rust) |
| Analysis | rustfft, ndarray |
| Database | SQLite (rusqlite) |
| Metadata | lofty |

## Development

### Prerequisites

- Node.js 20+
- Rust (latest stable)
- Windows: Visual Studio Build Tools
- macOS: Xcode Command Line Tools

### Setup

```bash
# Install dependencies
npm install

# Run in development mode
npm run tauri-dev

# Build for production
npm run tauri-build
```

### Project Structure

```
notmixedinkey/
├── src/                    # React frontend
│   ├── components/         # React components
│   │   ├── camelot/        # Camelot wheel, HarmonicMap
│   │   ├── library/        # LibraryTable, TrackRow, ImportDialog
│   │   ├── layout/         # MainLayout, Sidebar, Header
│   │   ├── player/         # DualDeck, Deck, Mixer
│   │   └── playlist/       # PlaylistBuilder
│   ├── hooks/              # Custom React hooks
│   ├── lib/                # Utilities (camelot.ts, tauri.ts)
│   ├── stores/             # Zustand stores
│   ├── styles/             # Global CSS
│   └── types/              # TypeScript interfaces
├── src-tauri/              # Rust backend
│   ├── src/
│   │   ├── analysis/       # Audio analysis engine
│   │   ├── commands/       # Tauri IPC commands
│   │   ├── db/             # SQLite database
│   │   └── models/         # Rust data structures
│   ├── migrations/         # Database migrations
│   └── Cargo.toml
└── PREP/                   # Planning documents
```

## Audio Analysis Pipeline

The analysis engine uses a hybrid approach:

1. **Audio Decoding** (Symphonia): Decode any supported format to mono 44.1kHz PCM
2. **Chromagram Extraction** (rustfft): Convert audio to 12-dimensional pitch class representation
3. **Classical Key Detection**: Profile matching using Krumhansl, Temperley, and Sha'ath key profiles
4. **Tempo Detection**: Onset detection + autocorrelation for BPM

## Architecture

- **Frontend**: React with virtual scrolling for large libraries
- **Backend**: Rust with tokio for async operations
- **Database**: SQLite with WAL mode for concurrent read/write
- **IPC**: Tauri events for real-time analysis progress

## License

MIT
