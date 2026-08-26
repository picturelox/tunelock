-- PB-6.0: Loudness analysis storage.
-- Stores per-track Integrated LUFS, true peak, and sample peak.
-- Versioned so results can be recomputed when the analysis engine changes.

CREATE TABLE IF NOT EXISTS loudness_analysis (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    track_id          INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    -- Integrated loudness in LUFS (BS.1770-4 gated). NULL if the track
    -- is too quiet to pass the -70 LUFS absolute gate.
    integrated_lufs   REAL,
    -- True peak in dBTP (BS.1770 Annex 2, 4x oversampling).
    true_peak_dbtp    REAL NOT NULL,
    -- Sample peak in dBFS (maximum absolute sample value).
    sample_peak_dbfs  REAL NOT NULL,
    -- Analysis engine version. When this doesn't match the current
    -- LOUDNESS_ANALYSIS_VERSION, the result should be recomputed.
    analysis_version  INTEGER NOT NULL,
    -- Sample rate used for analysis (Hz).
    sample_rate       INTEGER NOT NULL,
    -- Duration in seconds (for reference; tracks may also have duration_ms).
    duration_sec      REAL NOT NULL,
    analyzed_at       TEXT DEFAULT (datetime('now')),
    UNIQUE(track_id)
);

CREATE INDEX IF NOT EXISTS idx_loudness_track ON loudness_analysis(track_id);
CREATE INDEX IF NOT EXISTS idx_loudness_lufs ON loudness_analysis(integrated_lufs);
