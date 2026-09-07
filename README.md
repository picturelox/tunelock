# TuneLock

TuneLock is a native DJ instrument for discovering relationships in a music
library, preparing transitions, performing a set, and recording the result.

The first usable release targets reliable two-deck home mixing on Windows and
macOS with mouse/keyboard and a Traktor Kontrol S3. Decks C/D follow after A/B
reliability; stems follow after four-deck full-track reliability.

## Current state

The repository already contains a substantial Rust/CPAL audio foundation,
classical and neural-analysis research, library persistence, and an integrated
analysis/playback workspace. It is not yet the first usable release: private cue
routing, S3 control, scratching/backspins, delay/reverb, output protection,
recording, executable transition replay, a shared session model, and macOS
validation remain open.

Start here:

- [Product contract](PRODUCT.md)
- [Delivery roadmap](ROADMAP.md)
- [Capability ledger](CAPABILITIES.md)
- [Current milestone and checks](STATUS.md)
- [Measured analysis and audio evidence](ACCURACY.md)
- [Frozen intelligence architecture](CORE_INTELLIGENCE.md)

Older documents in `PREP/` are retained as research, architecture, and design
history. They do not override the documents above where scope conflicts.

## Technical foundation

- Tauri 2, React 18, TypeScript, TailwindCSS, and Zustand
- Rust, CPAL, Symphonia, Rubato, and Signalsmith Stretch
- SQLite persistence with WAL mode
- immediate deterministic key/BPM analysis plus optional asynchronous
  intelligence work
- a bounded real-time command path and allocation/deallocation audit coverage

## Development

Prerequisites are Node.js 20+, current stable Rust, and the platform's native
build tools. On Windows, run Rust commands from a Visual Studio x64 developer
environment and add Cargo to `PATH` as documented in `AGENTS.md`.

```powershell
npm install
npm run dev
npm run tauri-dev
npx tsc --noEmit
npm run build
```

```powershell
$env:PATH = "C:\Users\louis.media\.cargo\bin;" + $env:PATH
Set-Location src-tauri
cargo test
```

See `AGENTS.md` for project rules and verification commands.

## Safety and licensing

TuneLock never modifies, moves, or deletes original media. External utilities
such as ffmpeg are detected on `PATH` or used as optional sidecars; downloaders
and GPL/AGPL code are not bundled.

License: MIT.
