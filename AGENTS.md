# AGENTS.md — TuneLock working environment

Cargo is **not** on `PATH` in a fresh shell. Once per PowerShell session:

```powershell
$env:PATH = "C:\Users\louis.media\.cargo\bin;" + $env:PATH
```

## Commands

| Task | Command |
|---|---|
| Frontend only | `npm run dev` |
| Full app (dev) | `npm run tauri-dev` |
| Frontend typecheck | `npx tsc --noEmit` |
| Frontend prod build | `npm run build` |
| Rust check | `cd src-tauri; cargo check` |
| Rust tests | `cd src-tauri; cargo test` |
| Accuracy bench (**always `--release`** — debug is ~20× slower) | `cd src-tauri; cargo run --release --bin tunelock-bench -- <folder>` |
| Installer | `npm run tauri-build` |

## House rules

1. **The plan file is the source of truth:** `C:\Users\louis.media\.devin\plans\plan-dfdfe6627c43db0f.md`. Keep it current; do not work against memory of it.
2. **No engine change lands before a baseline exists** in `ACCURACY.md`, and every engine change is re-measured against it.
3. **The local result renders first, always.** No network call, model load, or LLM call is ever on the critical path to a key/BPM readout.
4. **One harmony vocabulary.** Rust `harmony/` and TS `lib/harmony.ts` are mirrors with shared test vectors. Do not add a third.
5. **Every `invoke(...)` in TS must resolve to a registered Rust command.** No phantom wrappers.
6. **Never bundle downloaders or GPL/AGPL code.** External tools (ffmpeg, yt-dlp, fpcalc) are detected on PATH or run as optional sidecars.
7. Non-destructive everywhere: originals are never modified, moved, or deleted.
