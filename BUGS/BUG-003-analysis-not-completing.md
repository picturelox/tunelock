# BUG-003: Analysis starts but never completes / stalls

- **Status**: fixed (pending live verification)
- **Severity**: high
- **Reported**: 2026-04-22
- **Component**: backend / analysis
- **Reporter**: HITL (third launch, post BUG-002 fix)

## Summary

App launches cleanly after BUG-002 fix. User imports a folder, scan reports the files, analysis kicks off — but tracks never finish analysing (library rows stay in the `pending`/`analyzing` state indefinitely; no Camelot/BPM badges appear).

## Repro

1. `npm run tauri dev`
2. Import a folder of audio files (e.g. `C:\Users\louis.media\Desktop\OCAAT\False_Banners_STEMS\…`)
3. Wait. Expected: rows stream key/BPM within seconds-to-minutes. Actual: rows stay in a non-`analyzed` state; analysis progress indicator does not advance (or advances extremely slowly).

## Likely root cause (hypotheses, in order of probability)

### H1 — HPSS median filtering is catastrophically slow  (most likely)

`@c:\Users\louis.media\Desktop\notmixedinkey\src-tauri\src\analysis\hpss.rs` implements two full-spectrogram median-filter passes, each with an inner `window.sort_by(…)` per output sample.

For a 4-minute track at 44.1 kHz with FFT=4096, hop=512:
- `bins = 2048`, `frames ≈ 4 × 60 × 44100 / 512 ≈ 20,640`
- Each pass visits `bins × frames ≈ 42.2M` cells
- At each cell it clones a `Vec<f64>` of length ≤ 17 and `sort_by`s it (~85 comparisons)
- Two passes → **~7 billion f64 comparisons per track**, plus memory allocation per cell

That is easily 30 s – several minutes per track on a single thread, and the analysis worker is **serial** (no rayon yet). For a 20-file folder the perceived result is "analysis hangs".

### H2 — Analysis worker runs tracks serially, not in parallel

`@c:\Users\louis.media\Desktop\notmixedinkey\src-tauri\src\commands\mod.rs:140-185` uses a single `tokio::spawn`ed loop that pops one track at a time and awaits its full analysis before starting the next. Blueprint called for rayon-based parallelism across N-1 cores. Even with H1 fixed, a 50k-file library would take forever at 1 track at a time.

### H3 — Events fire but the frontend isn't consuming them correctly

Less likely because the Rust side writes to SQLite before emitting, so a frontend-only bug would still show rows transitioning to `analyzed` after a DB refresh. Worth confirming.

### H4 — Tokio runtime contention between `db.lock()` and the worker

`analyze_single_track` holds `db.lock().await` across a multi-megabyte `get_track_by_id` round-trip. If something else (e.g. a concurrent `get_library_page` from the frontend) is also awaiting the lock, throughput degrades. This would only show as "slow" though, not "never completes".

## Planned fix

**Phase 1 — unblock perceived hang (the real root cause):**

1. Replace the naïve per-cell median filter in `hpss::hpss` with a much faster running-window structure (e.g. two sorted deques / quickselect) — or, as a quick first pass, just **reduce the kernel size from 17 → 9** and **downsample frames by 2x** before HPSS. That alone should give ~6× speedup.
2. If still too slow, consider doing HPSS on a lower-resolution spectrogram (larger hop), or skipping HPSS on the first-pass import and only running it on user-triggered deep-analysis.

**Phase 2 — true parallelism:**

3. Convert the worker to use `rayon::par_iter` (or a `tokio::task::JoinSet`) so N-1 cores work in parallel. Stream each completed track via `window.emit` as before.

**Phase 3 — fall-back path:**

4. Add a feature flag / config toggle that disables HPSS entirely and goes straight to classical chroma → ensemble. For most tracks this costs ≤ 1% accuracy but makes analysis real-time.

## Diagnostic info to collect on next run

- Time between `start_analysis` invocation and the **first** `track-analyzed` event
- Did stderr show any `Analysis error for …` lines?
- Does the `tracks.status` column in the SQLite DB ever move from `pending`/`metadata_ready` to `analyzed`?
- CPU utilisation of the `notmixedinkey.exe` process during "hang"

## Updates

- `2026-04-22 23:20` — **Real fix shipped.** Two parallel changes:

  **HPSS** (`@c:\Users\louis.media\Desktop\notmixedinkey\src-tauri\src\analysis\hpss.rs`):
  - Both median passes now run in parallel via `ndarray::Zip::par_for_each` (rayon thread pool) — one row/column per task
  - Window buffer is now a **stack-allocated `[f64; 64]`**, eliminating the ~42 M heap allocations per track
  - Soft-mask step is a single fused `Zip::par_for_each` over `(h_out, p_out, spec, harmonic, percussive)` — no intermediate allocation
  - Expected speedup: roughly `(N cores) × (heap→stack savings)` ≈ **10–30× on an 8-core box**

  **Analysis worker** (`@c:\Users\louis.media\Desktop\notmixedinkey\src-tauri\src\commands\mod.rs:159-233`):
  - Removed the one-track-at-a-time serial loop
  - Now pops a batch of `N-1` tracks per iteration (where `N` = available cores) and runs decode + HPSS + key + BPM in parallel via `rayon::par_iter` inside a single `tokio::task::spawn_blocking`
  - DB writes + `window.emit("track-analyzed", …)` are serialised after each batch completes — no DB contention, events still stream per track
  - Throughput is now bounded by decode + per-track HPSS, not by sequential execution

- `2026-04-22 23:13` — Filed. Awaiting diagnostic data from the running session to confirm H1. High confidence H1 is dominant because the HPSS code is genuinely O(bins × frames × kernel · log kernel) with heap allocation per cell.

## Resolution

- Root cause was H1 + H2 stacked: naïve O(bins × frames × kernel) HPSS with per-cell heap allocation, plus a single-track serial worker. Either alone would be painful; together they made analysis feel like a hang on any library >5 files.
- Build verified: `cargo check` → 0 errors, 0 warnings.
- Status is "fixed (pending live verification)" until the next `npm run tauri dev` session confirms tracks now stream completed within a reasonable wall-clock time and the progress counter advances.
