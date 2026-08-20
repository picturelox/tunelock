-- Migration 002: Beat grids and transition plans for the Transition Workbench.
--
-- Beat grids are track-level catalog data: BPM, first-beat time, meter,
-- downbeat offset, and confidence. Manual corrections are stored separately
-- from engine estimates and always win until explicitly reset.
--
-- Transition plans are mix-level data: overlap, loop, tempo master, deck
-- gains/EQ, stem states, crossfader state, and automation. They belong to
-- the persisted mix (playlist) and are versioned for safe migration.

-- beat_grids: per-track beat grid estimates and manual corrections
CREATE TABLE IF NOT EXISTS beat_grids (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    track_id        INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    -- Schema version for forward compatibility
    schema_version  INTEGER NOT NULL DEFAULT 1,
    -- Source: 'engine' (from BPM detection), 'manual' (user-set), 'imported' (from DJ software)
    source          TEXT NOT NULL DEFAULT 'engine',
    -- Beat grid parameters
    bpm             REAL NOT NULL,              -- detected or corrected BPM
    first_beat_ms   INTEGER NOT NULL DEFAULT 0, -- millisecond offset of first beat
    meter_numerator INTEGER NOT NULL DEFAULT 4, -- time signature numerator (4/4, 3/4, etc.)
    downbeat_offset_beats INTEGER NOT NULL DEFAULT 0, -- which beat is the downbeat (0-indexed)
    -- Confidence: null for manual, 0.0-1.0 for engine estimates
    confidence      REAL,
    -- Whether this is the active grid (manual overrides engine)
    is_override     BOOLEAN DEFAULT 0,
    created_at      TEXT DEFAULT (datetime('now')),
    updated_at      TEXT DEFAULT (datetime('now')),
    UNIQUE(track_id, source)
);
CREATE INDEX IF NOT EXISTS idx_beat_grids_track ON beat_grids(track_id);
CREATE INDEX IF NOT EXISTS idx_beat_grids_override ON beat_grids(track_id, is_override);

-- transition_plans: per-transition plan data stored alongside the mix
-- Each transition in a mix canvas has its own plan with overlap, loop,
-- mixer state, and automation. The plan is versioned for safe migration.
CREATE TABLE IF NOT EXISTS transition_plans (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    -- The mix (playlist) this transition belongs to
    playlist_id     INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
    -- The transition identifier within the mix (matches the MixProject JSON)
    transition_id   TEXT NOT NULL,
    -- Schema version for forward compatibility
    schema_version  INTEGER NOT NULL DEFAULT 1,
    -- The full transition plan as JSON (overlap, loop, tempo master, deck
    -- gains/EQ, stem states, crossfader state, automation points)
    plan_json       TEXT NOT NULL,
    -- Timestamps
    created_at      TEXT DEFAULT (datetime('now')),
    updated_at      TEXT DEFAULT (datetime('now')),
    UNIQUE(playlist_id, transition_id)
);
CREATE INDEX IF NOT EXISTS idx_transition_plans_playlist ON transition_plans(playlist_id);

-- stem_manifests: cached stem separation results
-- Tracks which tracks have been stem-separated, by which provider/model,
-- and where the stem files live. Stems are cached by source fingerprint.
CREATE TABLE IF NOT EXISTS stem_manifests (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    track_id        INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    -- Schema version
    schema_version  INTEGER NOT NULL DEFAULT 1,
    -- Source fingerprint (for staleness detection)
    source_fingerprint TEXT NOT NULL,
    -- Provider info
    provider        TEXT NOT NULL,              -- 'demucs', 'spleeter', 'stemdeck', etc.
    model           TEXT NOT NULL,              -- 'htdemucs', 'spleeter-4stems', etc.
    model_version   TEXT,                       -- version string
    -- Stem file paths (relative to app data dir or cache dir)
    vocals_path     TEXT,
    drums_path      TEXT,
    bass_path       TEXT,
    other_path      TEXT,
    -- Timing
    duration_ms     INTEGER,
    alignment_offset_ms INTEGER DEFAULT 0,      -- decoder/provider delay compensation
    -- Status: 'ready', 'processing', 'failed', 'stale'
    status          TEXT NOT NULL DEFAULT 'ready',
    -- Storage info
    storage_bytes   INTEGER,
    created_at      TEXT DEFAULT (datetime('now')),
    UNIQUE(track_id, provider, model)
);
CREATE INDEX IF NOT EXISTS idx_stem_manifests_track ON stem_manifests(track_id);
CREATE INDEX IF NOT EXISTS idx_stem_manifests_status ON stem_manifests(status);
