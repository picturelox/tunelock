# BUG-001: Compile warnings & runtime analysis errors on first launch

- **Status**: fixed
- **Severity**: high
- **Reported**: 2026-04-22
- **Component**: backend
- **Reporter**: Cascade (first end-to-end run)

## Summary

On the first successful `npm run tauri dev` the app window opened and the scan command ran, but every track analysis emitted `"Track not found after analysis"` to stderr. Separately, `cargo check` produced 9 non-blocking warnings.

## Repro

1. `$env:PATH = "C:\Users\louis.media\.cargo\bin;" + $env:PATH`
2. `npm run tauri dev` from project root
3. Import a folder of audio files via the Import dialog
4. Observe stderr: repeated `Analysis error for <path>: Track not found after analysis`
5. Observe frontend: no `track-analyzed` events fire, so the library table never updates with key/BPM.

Expected: analyzed rows stream into the library view with key/BPM/Camelot badge. Actual: nothing streams; every track silently errors server-side.

## Root cause

Two distinct problems in one report:

### 1. Runtime: "Track not found after analysis" (high)
In `@c:\Users\louis.media\Desktop\notmixedinkey\src-tauri\src\commands\mod.rs:212-215` the post-analysis fetch was:

```rust
let track = db.get_library_page(0, 1, "id", "asc", None)?
    .tracks.into_iter()
    .find(|t| t.id == track_id)
    .ok_or_else(|| anyhow::anyhow!("Track not found after analysis"))?;
```

This fetches only page 0 (the single lowest-id track) and then filters to find `track_id` — which will virtually never match. The correct approach is a targeted `SELECT … WHERE id = ?` query. There is no `get_track_by_id` on `Database` yet.

### 2. Compile warnings (low, non-blocking)
From `cargo check`:

- `unused import: anyhow::Result` in `src/lib.rs` — *fixed during session*
- `unused import: ndarray::Array1` in `analysis/key_detector.rs` — *fixed*
- `unused imports: FftPlanner, num_complex::Complex` in `analysis/key_detector.rs` — *fixed*
- `unused import: Array` in `analysis/chromagram.rs` — *fixed*
- `unused imports: Path, AppHandle, KeyResult` in `commands/mod.rs` — *fixed*
- `unused import: OptionalExtension` in `db/mod.rs` — *fixed*
- 5 × `unused variable` warnings for placeholder params (`state`, `start_track_id`, `rules`, `max_length`, `track_id`) in the stub commands `generate_playlist` and `get_compatible_tracks`. These are intentional until Phase 4 is fully wired — will be resolved when those command bodies are implemented.

## Fix

### For issue #1
Add `get_track_by_id(&self, id: i64) -> Result<Option<Track>>` to `Database` and call that instead of paginating.

### For issue #2
Prefix the stub-command parameters with `_` (or implement the commands).

## Updates

- `2026-04-22 23:20` — **Resolved**. Added `Database::get_track_by_id(id)` (`@c:\Users\louis.media\Desktop\notmixedinkey\src-tauri\src\db\mod.rs:235-270`) and switched `analyze_single_track` to use it (`@c:\Users\louis.media\Desktop\notmixedinkey\src-tauri\src\commands\mod.rs:211-213`). Prefixed unused stub-command params with `_`. Removed unused `Accessor` import from `export/mod.rs`. `cargo check` now reports **0 errors, 0 warnings**.
- `2026-04-22 23:00` — Bug filed. App launched successfully; scan works; analysis pipeline runs per-file but the IPC event emission path is broken by the post-fetch query bug. Frontend never receives `track-analyzed` events.

## Resolution

- Root cause was a paginated lookup that couldn't find the just-analyzed track. Fix: targeted `SELECT … WHERE id = ?` via new `get_track_by_id` helper.
- All 9 original `cargo check` warnings are now cleaned up; `unused variable` warnings on stub commands silenced by underscore-prefixing.
- Verified clean build: `cargo check` → `Finished dev profile`, no errors, no warnings.
- **Still to verify live**: relaunch `npm run tauri dev`, re-scan the OCAAT/False_Banners_STEMS folder, confirm `track-analyzed` events now fire and the library table populates with key/BPM/Camelot badges.
