# BUG-002: SQLite migration fails on second launch — tables already exist

- **Status**: fixed
- **Severity**: critical
- **Reported**: 2026-04-22
- **Component**: backend / db
- **Reporter**: HITL (second launch after BUG-001 fix)

## Summary

App crashes on launch with `Error code 1: SQL error or missing database` because `migrations/001_init.sql` uses `CREATE TABLE` and `CREATE INDEX` without `IF NOT EXISTS`. On the second launch the DB file already exists from the first session, so the schema re-execution hard-fails. First launch worked because the DB was empty.

## Repro

1. `npm run tauri dev` (app boots, DB created)
2. Close the app
3. `npm run tauri dev` again
4. Crash with:
   ```
   Error code 1: SQL error or missing database
   ```
   followed by the migration SQL dump.

## Root cause

`@c:\Users\louis.media\Desktop\notmixedinkey\src-tauri\migrations\001_init.sql` is not idempotent:

- `CREATE TABLE tracks (…)` — fails if `tracks` already exists
- 7 more `CREATE TABLE` statements — same issue
- `CREATE INDEX idx_…` — same issue
- `INSERT INTO ensemble_weights …` — would duplicate rows even if tables were guarded

`@c:\Users\louis.media\Desktop\notmixedinkey\src-tauri\src\db\mod.rs:28` calls `self.conn.execute_batch(schema)` unconditionally every time the `Database` opens, so these statements run on every boot.

## Fix

Make the migration idempotent:

1. `CREATE TABLE` → `CREATE TABLE IF NOT EXISTS`
2. `CREATE INDEX` → `CREATE INDEX IF NOT EXISTS`
3. `INSERT INTO ensemble_weights` → `INSERT OR IGNORE INTO ensemble_weights` (the `profile_name` UNIQUE constraint now blocks dupes)

A proper migration-versioning system (`user_version` pragma + ordered SQL files) is the longer-term answer, but this one-line-per-statement change unblocks day-to-day dev without restructuring the migration pipeline.

## Updates

- `2026-04-22 23:15` — **Resolved**. Added `IF NOT EXISTS` to all 6 `CREATE TABLE` and all 7 `CREATE INDEX` statements; changed the ensemble_weights seed to `INSERT OR IGNORE`. Migration is now safely idempotent. See `@c:\Users\louis.media\Desktop\notmixedinkey\src-tauri\migrations\001_init.sql`.
- `2026-04-22 23:10` — Filed. App crashes on second launch with migration SQL dump + `Error code 1`.

## Resolution

- Guarded every DDL statement with `IF NOT EXISTS`; the seed insert now uses `INSERT OR IGNORE` so re-running on a seeded DB is a no-op.
- A real migration versioning layer (`PRAGMA user_version`) is still desirable but out of scope for this fix.
