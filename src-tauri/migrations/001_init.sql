-- tracks table: core metadata
CREATE TABLE IF NOT EXISTS tracks (
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
    artwork_path  TEXT,                  -- absolute path to cached cover-art image (nullable)
    created_at    TEXT DEFAULT (datetime('now')),
    updated_at    TEXT DEFAULT (datetime('now'))
);

-- playlists table
CREATE TABLE IF NOT EXISTS playlists (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    name        TEXT NOT NULL,
    description TEXT,
    rules       TEXT,           -- JSON: which Camelot rules were used
    created_at  TEXT DEFAULT (datetime('now'))
);

-- playlist_tracks join table (ordered)
CREATE TABLE IF NOT EXISTS playlist_tracks (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    playlist_id INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
    track_id    INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    position    INTEGER NOT NULL,  -- order in playlist
    UNIQUE(playlist_id, track_id)
);

-- cue_points table: per-track cue points
CREATE TABLE IF NOT EXISTS cue_points (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    track_id    INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    position_ms INTEGER NOT NULL,  -- millisecond position in track
    name        TEXT,              -- user label, e.g., "Drop", "Intro", "Vocal"
    color       TEXT,              -- hex color, e.g., "#FF0000"
    hotcue_index INTEGER,          -- 0-7 (for DJ software compatibility)
    created_at  TEXT DEFAULT (datetime('now'))
);

-- validation_results: MIK comparison data (self-calibration)
CREATE TABLE IF NOT EXISTS validation_results (
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
CREATE TABLE IF NOT EXISTS ensemble_weights (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    profile_name    TEXT NOT NULL UNIQUE,  -- e.g., "classical_temperley", "cnn_cqt"
    weight          REAL NOT NULL DEFAULT 1.0,
    accuracy_pct    REAL,          -- measured accuracy against MIK ground truth
    sample_count    INTEGER DEFAULT 0,
    updated_at      TEXT DEFAULT (datetime('now'))
);

-- Insert default ensemble weights (only if not already seeded)
INSERT OR IGNORE INTO ensemble_weights (profile_name, weight) VALUES
('classical_krumhansl', 0.4),
('classical_temperley', 0.5),
('classical_shaath', 0.5),
('cnn_cqt', 0.7),
('cnn_mel', 0.6),
('cnn_hpcp', 0.5),
('temporal', 0.6);

-- indexes for common queries
CREATE INDEX IF NOT EXISTS idx_tracks_key_camelot ON tracks(key_camelot);
CREATE INDEX IF NOT EXISTS idx_tracks_bpm ON tracks(bpm);
CREATE INDEX IF NOT EXISTS idx_tracks_artist ON tracks(artist);
CREATE INDEX IF NOT EXISTS idx_playlist_tracks_playlist ON playlist_tracks(playlist_id, position);
CREATE INDEX IF NOT EXISTS idx_cue_points_track ON cue_points(track_id);
CREATE INDEX IF NOT EXISTS idx_validation_track ON validation_results(track_id);
CREATE INDEX IF NOT EXISTS idx_tracks_status ON tracks(status);

-- track_opinions: multi-source key/BPM/energy opinions for consensus
CREATE TABLE IF NOT EXISTS track_opinions (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    track_id      INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    source        TEXT NOT NULL,           -- 'tunelock' | 'mik' | 'traktor' | 'acoustid'
    key_camelot   TEXT,
    key_standard  TEXT,
    bpm           REAL,
    energy        INTEGER,
    confidence    REAL DEFAULT 1.0,
    provenance    TEXT,                    -- e.g. "MIK CSV 2024-01-13"
    created_at    TEXT DEFAULT (datetime('now')),
    UNIQUE(track_id, source)
);
CREATE INDEX IF NOT EXISTS idx_track_opinions_track ON track_opinions(track_id);
CREATE INDEX IF NOT EXISTS idx_track_opinions_source ON track_opinions(source);

-- gold_annotations: user-adjudicated key labels, stored separately from
-- MIK opinions and engine predictions. Each track can have multiple
-- annotations (from different sessions or annotators) to measure
-- inter-annotator agreement and self-consistency.
CREATE TABLE IF NOT EXISTS gold_annotations (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    track_id        INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    -- The annotator's key judgment
    key_tonic       TEXT NOT NULL,          -- 'C', 'C#', 'D', ... 'B'
    key_mode        TEXT NOT NULL,          -- 'major', 'minor', 'ambiguous', 'atonal'
    -- If mode is 'ambiguous' or 'atonal', key_tonic may be a best guess
    -- or empty. The annotator can flag modulation:
    modulates       BOOLEAN DEFAULT 0,      -- does the track change key?
    modulation_note TEXT,                   -- free-text description of modulation
    -- Confidence of the annotator in their own judgment (1-5 scale)
    annotator_confidence INTEGER NOT NULL,  -- 1=guess, 5=certain
    -- Evidence the annotator used to reach their judgment
    evidence        TEXT,                   -- free-text: "bass line on C, melody emphasizes E minor"
    -- Annotator identity (for multi-annotator agreement)
    annotator_id    TEXT DEFAULT 'self',    -- 'self', or a named second annotator
    -- Whether this annotation was made blind (without seeing engine prediction)
    blind           BOOLEAN DEFAULT 1,
    created_at      TEXT DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_gold_annotations_track ON gold_annotations(track_id);
CREATE INDEX IF NOT EXISTS idx_gold_annotations_annotator ON gold_annotations(annotator_id);

-- training_sessions: tracks the user's key-identification training progress
CREATE TABLE IF NOT EXISTS training_sessions (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    session_type    TEXT NOT NULL,          -- 'pitch_id', 'tonic_id', 'mode_id', 'full_key'
    track_id        INTEGER REFERENCES tracks(id) ON DELETE SET NULL,
    -- What was presented
    presented_tonic TEXT,                   -- for pitch_id exercises
    presented_mode  TEXT,                   -- for mode_id exercises
    -- What the user answered
    user_answer     TEXT NOT NULL,
    -- Whether the answer was correct
    correct         BOOLEAN NOT NULL,
    -- Time taken in seconds
    response_time_s REAL,
    created_at      TEXT DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_training_sessions_type ON training_sessions(session_type);

-- Performance optimizations
PRAGMA journal_mode = WAL;
PRAGMA cache_size = -65536;
PRAGMA synchronous = NORMAL;
PRAGMA temp_store = MEMORY;
PRAGMA page_size = 4096;
