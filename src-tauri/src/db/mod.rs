use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use std::path::Path;

use crate::models::*;

pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self> {
        let conn = Connection::open(path).context("Failed to open database")?;
        
        // Apply performance optimizations
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA cache_size = -65536;
             PRAGMA synchronous = NORMAL;
             PRAGMA temp_store = MEMORY;
             PRAGMA page_size = 4096;"
        )?;
        
        let db = Self { conn };
        db.init_schema()?;
        Ok(db)
    }
    
    fn init_schema(&self) -> Result<()> {
        let schema = include_str!("../../migrations/001_init.sql");
        self.conn.execute_batch(schema)?;

        // Migration 002: Transition Workbench tables (beat grids, transition plans, stem manifests)
        let migration_002 = include_str!("../../migrations/002_transition_workbench.sql");
        self.conn.execute_batch(migration_002)?;

        // Migration 003: Loudness analysis (PB-6.0)
        let migration_003 = include_str!("../../migrations/003_loudness.sql");
        self.conn.execute_batch(migration_003)?;

        // Idempotent column additions for existing databases that pre-date
        // a schema change. SQLite doesn't have `ADD COLUMN IF NOT EXISTS`,
        // so we attempt the ALTER and swallow the specific "duplicate column"
        // error. Any other error still propagates.
        self.add_column_if_missing("tracks", "artwork_path", "TEXT")?;
        // Phase 6: genre and MIK reference metadata for consensus/import.
        self.add_column_if_missing("tracks", "genre", "TEXT")?;
        self.add_column_if_missing("tracks", "mik_key", "TEXT")?;
        self.add_column_if_missing("tracks", "mik_energy", "INTEGER")?;

        Ok(())
    }

    /// Run `ALTER TABLE <table> ADD COLUMN <col> <type>`, treating a
    /// "duplicate column name" error as success. All other errors propagate.
    fn add_column_if_missing(&self, table: &str, column: &str, col_type: &str) -> Result<()> {
        let sql = format!("ALTER TABLE {} ADD COLUMN {} {}", table, column, col_type);
        match self.conn.execute(&sql, []) {
            Ok(_) => Ok(()),
            Err(rusqlite::Error::SqliteFailure(_, Some(msg)))
                if msg.contains("duplicate column name") =>
            {
                Ok(())
            }
            Err(e) => Err(e.into()),
        }
    }
    
    pub fn insert_track(&self, path: &str, filename: &str, file_size: i64) -> Result<i64> {
        // Use RETURNING id to get the correct row ID on both insert and
        // conflict paths. The previous version used last_insert_rowid(),
        // which returns a stale ID on the ON CONFLICT DO UPDATE path
        // (nothing was inserted, so the rowid belongs to a prior insert).
        let id: i64 = self.conn.query_row(
            "INSERT INTO tracks (file_path, filename, file_size, status)
             VALUES (?1, ?2, ?3, 'pending')
             ON CONFLICT(file_path) DO UPDATE SET
             file_size = excluded.file_size,
             updated_at = datetime('now')
             RETURNING id",
            params![path, filename, file_size],
            |row| row.get(0),
        )?;
        Ok(id)
    }
    
    pub fn update_track_metadata(
        &self,
        track_id: i64,
        title: Option<&str>,
        artist: Option<&str>,
        album: Option<&str>,
        duration_ms: Option<i64>,
        file_format: &str,
        sample_rate: Option<i64>,
        bit_depth: Option<i64>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE tracks SET
             title = ?2,
             artist = ?3,
             album = ?4,
             duration_ms = ?5,
             file_format = ?6,
             sample_rate = ?7,
             bit_depth = ?8,
             status = 'metadata_ready',
             updated_at = datetime('now')
             WHERE id = ?1",
            params![
                track_id,
                title,
                artist,
                album,
                duration_ms,
                file_format,
                sample_rate,
                bit_depth,
            ],
        )?;
        Ok(())
    }
    
    pub fn update_track_analysis(
        &self,
        track_id: i64,
        key_standard: &str,
        key_camelot: &str,
        key_confidence: f64,
        bpm: f64,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE tracks SET
             key_standard = ?2,
             key_camelot = ?3,
             key_confidence = ?4,
             bpm = ?5,
             status = 'analyzed',
             analyzed_at = datetime('now'),
             updated_at = datetime('now')
             WHERE id = ?1",
            params![
                track_id,
                key_standard,
                key_camelot,
                key_confidence,
                bpm,
            ],
        )?;
        Ok(())
    }
    
    pub fn get_library_page(
        &self,
        page: usize,
        page_size: usize,
        sort_by: &str,
        sort_dir: &str,
        filter: Option<&LibraryFilter>,
    ) -> Result<LibraryPage> {
        let offset = page * page_size;
        
        // Build WHERE clause using owned Values to avoid borrow issues
        let mut conditions = vec![];
        let mut param_values: Vec<rusqlite::types::Value> = vec![];
        
        if let Some(f) = filter {
            if let Some(search) = &f.search {
                conditions.push("(filename LIKE ? OR title LIKE ? OR artist LIKE ?)");
                let pattern = format!("%{}%", search);
                param_values.push(rusqlite::types::Value::Text(pattern.clone()));
                param_values.push(rusqlite::types::Value::Text(pattern.clone()));
                param_values.push(rusqlite::types::Value::Text(pattern));
            }
            if let Some(artist) = &f.artist {
                conditions.push("artist = ?");
                param_values.push(rusqlite::types::Value::Text(artist.clone()));
            }
            if let Some(key) = &f.key_camelot {
                conditions.push("key_camelot = ?");
                param_values.push(rusqlite::types::Value::Text(key.clone()));
            }
            if let Some(min_bpm) = f.min_bpm {
                conditions.push("bpm >= ?");
                param_values.push(rusqlite::types::Value::Real(min_bpm));
            }
            if let Some(max_bpm) = f.max_bpm {
                conditions.push("bpm <= ?");
                param_values.push(rusqlite::types::Value::Real(max_bpm));
            }
            if let Some(status) = &f.status {
                conditions.push("status = ?");
                param_values.push(rusqlite::types::Value::Text(status.clone()));
            }
            // Smart filters — applied server-side so they work across all 20k
            // tracks, not just the currently loaded page.
            if let Some(smart) = &f.smart_filter {
                match smart.as_str() {
                    "unanalyzed" => {
                        conditions.push("key_camelot IS NULL");
                    }
                    "low-confidence" => {
                        conditions.push("key_confidence IS NOT NULL AND key_confidence < 0.7");
                    }
                    "high-confidence" => {
                        conditions.push("key_confidence IS NOT NULL AND key_confidence >= 0.85");
                    }
                    _ => {}
                }
            }
        }
        
        let where_clause = if conditions.is_empty() {
            ""
        } else {
            &format!("WHERE {}", conditions.join(" AND "))
        };
        
        // Create parameter references after all values are collected
        let param_refs: Vec<&dyn rusqlite::ToSql> = param_values.iter().map(|v| v as &dyn rusqlite::ToSql).collect();
        
        // Get total count
        let count_sql = format!("SELECT COUNT(*) FROM tracks {}", where_clause);
        let total_count: usize = self.conn.query_row(&count_sql, &*param_refs, |row| row.get(0))?;
        
        // Get tracks
        let sort_column = match sort_by {
            "filename" => "filename",
            "artist" => "artist",
            "title" => "title",
            "key_camelot" => "key_camelot",
            "bpm" => "bpm",
            "duration_ms" => "duration_ms",
            _ => "filename",
        };
        let dir = if sort_dir == "desc" { "DESC" } else { "ASC" };
        
        // Add limit and offset to param_values
        param_values.push(rusqlite::types::Value::Integer(page_size as i64));
        param_values.push(rusqlite::types::Value::Integer(offset as i64));
        
        // Recreate param_refs with updated values
        let param_refs: Vec<&dyn rusqlite::ToSql> = param_values.iter().map(|v| v as &dyn rusqlite::ToSql).collect();
        
        let sql = format!(
            "SELECT id, file_path, filename, title, artist, album, duration_ms,
             key_standard, key_camelot, key_confidence, bpm, energy_level,
             file_format, file_size, sample_rate, bit_depth, analyzed_at, status,
             artwork_path, created_at, updated_at
             FROM tracks {}
             ORDER BY {} {}
             LIMIT ? OFFSET ?",
            where_clause,
            sort_column,
            dir
        );
        
        let mut stmt = self.conn.prepare(&sql)?;
        
        let tracks: Vec<Track> = stmt
            .query_map(&*param_refs, |row| {
                Ok(Track {
                    id: row.get(0)?,
                    file_path: row.get(1)?,
                    filename: row.get(2)?,
                    title: row.get(3)?,
                    artist: row.get(4)?,
                    album: row.get(5)?,
                    duration_ms: row.get(6)?,
                    key_standard: row.get(7)?,
                    key_camelot: row.get(8)?,
                    key_confidence: row.get(9)?,
                    bpm: row.get(10)?,
                    energy_level: row.get(11)?,
                    file_format: row.get(12)?,
                    file_size: row.get(13)?,
                    sample_rate: row.get(14)?,
                    bit_depth: row.get(15)?,
                    analyzed_at: row.get(16)?,
                    status: TrackStatus::from(row.get::<_, String>(17)?),
                    artwork_path: row.get(18)?,
                    created_at: row.get(19)?,
                    updated_at: row.get(20)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        
        Ok(LibraryPage {
            tracks,
            total_count,
            page,
            page_size,
        })
    }
    
    pub fn get_track_by_id(&self, id: i64) -> Result<Option<Track>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, file_path, filename, title, artist, album, duration_ms,
             key_standard, key_camelot, key_confidence, bpm, energy_level,
             file_format, file_size, sample_rate, bit_depth, analyzed_at, status,
             artwork_path, created_at, updated_at
             FROM tracks WHERE id = ?1"
        )?;
        let track = stmt
            .query_row(params![id], |row| {
                Ok(Track {
                    id: row.get(0)?,
                    file_path: row.get(1)?,
                    filename: row.get(2)?,
                    title: row.get(3)?,
                    artist: row.get(4)?,
                    album: row.get(5)?,
                    duration_ms: row.get(6)?,
                    key_standard: row.get(7)?,
                    key_camelot: row.get(8)?,
                    key_confidence: row.get(9)?,
                    bpm: row.get(10)?,
                    energy_level: row.get(11)?,
                    file_format: row.get(12)?,
                    file_size: row.get(13)?,
                    sample_rate: row.get(14)?,
                    bit_depth: row.get(15)?,
                    analyzed_at: row.get(16)?,
                    status: TrackStatus::from(row.get::<_, String>(17)?),
                    artwork_path: row.get(18)?,
                    created_at: row.get(19)?,
                    updated_at: row.get(20)?,
                })
            })
            .ok();
        Ok(track)
    }

    /// Persist the path to a track's cached artwork file. Pass `None` to clear.
    pub fn update_track_artwork(&self, track_id: i64, artwork_path: Option<&str>) -> Result<()> {
        self.conn.execute(
            "UPDATE tracks SET artwork_path = ?2, updated_at = datetime('now') WHERE id = ?1",
            params![track_id, artwork_path],
        )?;
        Ok(())
    }

    /// Update a track's detected energy level (1–10).
    pub fn update_track_energy(&self, track_id: i64, energy_level: i32) -> Result<()> {
        self.conn.execute(
            "UPDATE tracks SET energy_level = ?2, updated_at = datetime('now') WHERE id = ?1",
            params![track_id, energy_level],
        )?;
        Ok(())
    }

    /// Get a track's genre (from MIK CSV import or manual entry).
    pub fn get_track_genre(&self, track_id: i64) -> Result<Option<String>> {
        let genre: Option<String> = self.conn.query_row(
            "SELECT genre FROM tracks WHERE id = ?1",
            params![track_id],
            |row| row.get(0),
        )?;
        Ok(genre)
    }
    
    pub fn get_tracks_pending_analysis(&self, limit: usize) -> Result<Vec<(i64, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, file_path FROM tracks 
             WHERE status IN ('pending', 'metadata_ready')
             ORDER BY id
             LIMIT ?"
        )?;
        
        let tracks = stmt
            .query_map([limit as i64], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        
        Ok(tracks)
    }
    
    pub fn get_analysis_stats(&self) -> Result<(usize, usize)> {
        let total: usize = self.conn.query_row(
            "SELECT COUNT(*) FROM tracks",
            [],
            |row| row.get(0)
        )?;
        
        let analyzed: usize = self.conn.query_row(
            "SELECT COUNT(*) FROM tracks WHERE status = 'analyzed'",
            [],
            |row| row.get(0)
        )?;
        
        Ok((total, analyzed))
    }

    // ========================================================================
    // Playlist methods
    // ========================================================================

    pub fn create_playlist(&self, name: &str, description: Option<&str>) -> Result<Playlist> {
        self.conn.execute(
            "INSERT INTO playlists (name, description) VALUES (?1, ?2)",
            params![name, description],
        )?;
        let id = self.conn.last_insert_rowid();
        let created_at: String = self.conn.query_row(
            "SELECT created_at FROM playlists WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )?;
        Ok(Playlist {
            id,
            name: name.to_string(),
            description: description.map(|s| s.to_string()),
            rules: None,
            created_at,
        })
    }

    pub fn get_playlists(&self) -> Result<Vec<Playlist>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, description, rules, created_at FROM playlists ORDER BY id DESC"
        )?;
        let playlists = stmt
            .query_map([], |row| {
                // rules is stored as TEXT (JSON); parse it if present.
                let rules_text: Option<String> = row.get(3)?;
                let rules = rules_text
                    .as_deref()
                    .and_then(|s| serde_json::from_str(s).ok());
                Ok(Playlist {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    description: row.get(2)?,
                    rules,
                    created_at: row.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(playlists)
    }

    pub fn delete_playlist(&self, id: i64) -> Result<()> {
        self.conn.execute(
            "DELETE FROM playlists WHERE id = ?1",
            params![id],
        )?;
        Ok(())
    }

    pub fn add_track_to_playlist(&self, playlist_id: i64, track_id: i64, position: i64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO playlist_tracks (playlist_id, track_id, position) VALUES (?1, ?2, ?3)
             ON CONFLICT(playlist_id, track_id) DO UPDATE SET position = excluded.position",
            params![playlist_id, track_id, position],
        )?;
        Ok(())
    }

    pub fn remove_track_from_playlist(&self, playlist_id: i64, track_id: i64) -> Result<()> {
        self.conn.execute(
            "DELETE FROM playlist_tracks WHERE playlist_id = ?1 AND track_id = ?2",
            params![playlist_id, track_id],
        )?;
        Ok(())
    }

    /// Save a mix project: updates an existing playlist or creates a new one.
    /// The clip metadata (notes, etc.) is stored as JSON in the `rules` column.
    /// Returns the playlist ID.
    pub fn save_mix(
        &self,
        id: Option<i64>,
        name: &str,
        description: Option<&str>,
        track_ids: &[i64],
        clip_notes: &[(usize, String)], // (position, notes) per clip
    ) -> Result<i64> {
        // Build the mix metadata JSON to store in the rules column
        let mix_meta = serde_json::json!({
            "type": "mix",
            "clipNotes": clip_notes.iter().map(|(pos, notes)| {
                serde_json::json!({"position": pos, "notes": notes})
            }).collect::<Vec<_>>(),
        });
        let rules_json = serde_json::to_string(&mix_meta)?;

        let playlist_id = if let Some(existing_id) = id {
            // Update existing playlist
            self.conn.execute(
                "UPDATE playlists SET name = ?1, description = ?2, rules = ?3 WHERE id = ?4",
                params![name, description, &rules_json, existing_id],
            )?;
            // Clear existing tracks and re-add
            self.conn.execute(
                "DELETE FROM playlist_tracks WHERE playlist_id = ?1",
                params![existing_id],
            )?;
            existing_id
        } else {
            // Create new playlist
            self.conn.execute(
                "INSERT INTO playlists (name, description, rules) VALUES (?1, ?2, ?3)",
                params![name, description, &rules_json],
            )?;
            self.conn.last_insert_rowid()
        };

        // Add tracks in order
        for (i, track_id) in track_ids.iter().enumerate() {
            self.conn.execute(
                "INSERT INTO playlist_tracks (playlist_id, track_id, position) VALUES (?1, ?2, ?3)",
                params![playlist_id, track_id, i as i64],
            )?;
        }

        Ok(playlist_id)
    }

    /// Load a mix project: returns the playlist plus ordered track IDs and clip notes.
    pub fn load_mix(&self, playlist_id: i64) -> Result<(Playlist, Vec<i64>, Vec<Option<String>>)> {
        let playlist: Playlist = self.conn.query_row(
            "SELECT id, name, description, rules, created_at FROM playlists WHERE id = ?1",
            params![playlist_id],
            |row| {
                let rules_text: Option<String> = row.get(3)?;
                let rules = rules_text
                    .as_deref()
                    .and_then(|s| serde_json::from_str(s).ok());
                Ok(Playlist {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    description: row.get(2)?,
                    rules,
                    created_at: row.get(4)?,
                })
            },
        )?;

        // Get ordered track IDs
        let mut stmt = self.conn.prepare(
            "SELECT track_id, position FROM playlist_tracks WHERE playlist_id = ?1 ORDER BY position"
        )?;
        let track_rows: Vec<(i64, i64)> = stmt
            .query_map(params![playlist_id], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;

        let track_ids: Vec<i64> = track_rows.iter().map(|(id, _)| *id).collect();

        // Extract clip notes from the rules JSON
        let mut clip_notes = vec![None; track_ids.len()];
        if let Some(serde_json::Value::Object(ref meta)) = playlist.rules {
            if let Some(serde_json::Value::Array(notes_arr)) = meta.get("clipNotes") {
                for note_entry in notes_arr {
                    if let Some(pos) = note_entry.get("position").and_then(|p| p.as_u64()) {
                        if let Some(notes) = note_entry.get("notes").and_then(|n| n.as_str()) {
                            let pos = pos as usize;
                            if pos < clip_notes.len() {
                                clip_notes[pos] = Some(notes.to_string());
                            }
                        }
                    }
                }
            }
        }

        Ok((playlist, track_ids, clip_notes))
    }

    pub fn get_playlist_tracks(&self, playlist_id: i64) -> Result<Vec<Track>> {        let mut stmt = self.conn.prepare(
            "SELECT t.id, t.file_path, t.filename, t.title, t.artist, t.album,
             t.duration_ms, t.key_standard, t.key_camelot, t.key_confidence, t.bpm,
             t.energy_level, t.file_format, t.file_size, t.sample_rate, t.bit_depth,
             t.analyzed_at, t.status, t.artwork_path, t.created_at, t.updated_at
             FROM tracks t
             JOIN playlist_tracks pt ON pt.track_id = t.id
             WHERE pt.playlist_id = ?1
             ORDER BY pt.position"
        )?;
        let tracks = stmt
            .query_map(params![playlist_id], |row| {
                Ok(Track {
                    id: row.get(0)?,
                    file_path: row.get(1)?,
                    filename: row.get(2)?,
                    title: row.get(3)?,
                    artist: row.get(4)?,
                    album: row.get(5)?,
                    duration_ms: row.get(6)?,
                    key_standard: row.get(7)?,
                    key_camelot: row.get(8)?,
                    key_confidence: row.get(9)?,
                    bpm: row.get(10)?,
                    energy_level: row.get(11)?,
                    file_format: row.get(12)?,
                    file_size: row.get(13)?,
                    sample_rate: row.get(14)?,
                    bit_depth: row.get(15)?,
                    analyzed_at: row.get(16)?,
                    status: TrackStatus::from(row.get::<_, String>(17)?),
                    artwork_path: row.get(18)?,
                    created_at: row.get(19)?,
                    updated_at: row.get(20)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(tracks)
    }

    // ========================================================================
    // MIK reference metadata import
    // ========================================================================

    /// Update a track's MIK reference data (key, energy, genre) by file path.
    /// Used by the MIK CSV importer to populate reference metadata for
    /// consensus scoring and energy display.
    /// Returns true if a track was found and updated, false if no match.
    pub fn update_mik_reference(
        &self,
        file_path: &str,
        mik_key: Option<&str>,
        mik_energy: Option<i32>,
        genre: Option<&str>,
    ) -> Result<bool> {
        // Try exact path match first, then try a normalized match
        // (MIK paths may use different drive letters or separators).
        let normalized = file_path.replace('/', "\\");
        let affected = self.conn.execute(
            "UPDATE tracks SET mik_key = ?1, mik_energy = ?2, genre = ?3 WHERE file_path = ?4 OR file_path = ?5",
            params![mik_key, mik_energy, genre, file_path, normalized],
        )?;
        if affected > 0 {
            return Ok(true);
        }

        // Try matching by filename only as a last resort.
        let filename = file_path
            .rsplit(|c| c == '/' || c == '\\')
            .next()
            .unwrap_or(file_path);
        let pattern = format!("%\\{}", filename);
        let affected2 = self.conn.execute(
            "UPDATE tracks SET mik_key = ?1, mik_energy = ?2, genre = ?3 WHERE file_path LIKE ?4",
            params![mik_key, mik_energy, genre, pattern],
        )?;
        Ok(affected2 > 0)
    }

    // ========================================================================
    // Track opinions (consensus)
    // ========================================================================

    /// Look up a track by exact file_path match.
    pub fn get_track_by_path(&self, path: &str) -> Result<Option<Track>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, file_path, filename, title, artist, album, duration_ms,
             key_standard, key_camelot, key_confidence, bpm, energy_level,
             file_format, file_size, sample_rate, bit_depth, analyzed_at, status,
             artwork_path, created_at, updated_at
             FROM tracks WHERE file_path = ?1"
        )?;
        let track = stmt
            .query_map(params![path], |row| {
                Ok(Track {
                    id: row.get(0)?,
                    file_path: row.get(1)?,
                    filename: row.get(2)?,
                    title: row.get(3)?,
                    artist: row.get(4)?,
                    album: row.get(5)?,
                    duration_ms: row.get(6)?,
                    key_standard: row.get(7)?,
                    key_camelot: row.get(8)?,
                    key_confidence: row.get(9)?,
                    bpm: row.get(10)?,
                    energy_level: row.get(11)?,
                    file_format: row.get(12)?,
                    file_size: row.get(13)?,
                    sample_rate: row.get(14)?,
                    bit_depth: row.get(15)?,
                    analyzed_at: row.get(16)?,
                    status: TrackStatus::from(row.get::<_, String>(17)?),
                    artwork_path: row.get(18)?,
                    created_at: row.get(19)?,
                    updated_at: row.get(20)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(track.into_iter().next())
    }

    /// Look up a track by filename only (case-insensitive).
    pub fn get_track_by_filename(&self, filename: &str) -> Result<Option<Track>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, file_path, filename, title, artist, album, duration_ms,
             key_standard, key_camelot, key_confidence, bpm, energy_level,
             file_format, file_size, sample_rate, bit_depth, analyzed_at, status,
             artwork_path, created_at, updated_at
             FROM tracks WHERE filename = ?1 COLLATE NOCASE LIMIT 1"
        )?;
        let track = stmt
            .query_map(params![filename], |row| {
                Ok(Track {
                    id: row.get(0)?,
                    file_path: row.get(1)?,
                    filename: row.get(2)?,
                    title: row.get(3)?,
                    artist: row.get(4)?,
                    album: row.get(5)?,
                    duration_ms: row.get(6)?,
                    key_standard: row.get(7)?,
                    key_camelot: row.get(8)?,
                    key_confidence: row.get(9)?,
                    bpm: row.get(10)?,
                    energy_level: row.get(11)?,
                    file_format: row.get(12)?,
                    file_size: row.get(13)?,
                    sample_rate: row.get(14)?,
                    bit_depth: row.get(15)?,
                    analyzed_at: row.get(16)?,
                    status: TrackStatus::from(row.get::<_, String>(17)?),
                    artwork_path: row.get(18)?,
                    created_at: row.get(19)?,
                    updated_at: row.get(20)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(track.into_iter().next())
    }

    /// Upsert an opinion for a track. If an opinion from the same source
    /// already exists, it is replaced.
    pub fn upsert_opinion(
        &self,
        track_id: i64,
        source: &str,
        key_camelot: Option<&str>,
        key_standard: Option<&str>,
        bpm: Option<f64>,
        energy: Option<i32>,
        confidence: f64,
        provenance: &str,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO track_opinions (track_id, source, key_camelot, key_standard, bpm, energy, confidence, provenance)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(track_id, source) DO UPDATE SET
               key_camelot = excluded.key_camelot,
               key_standard = excluded.key_standard,
               bpm = excluded.bpm,
               energy = excluded.energy,
               confidence = excluded.confidence,
               provenance = excluded.provenance",
            params![track_id, source, key_camelot, key_standard, bpm, energy, confidence, provenance],
        )?;
        Ok(())
    }

    /// Get all opinions for a track.
    pub fn get_opinions_for_track(&self, track_id: i64) -> Result<Vec<crate::consensus::TrackOpinion>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, track_id, source, key_camelot, key_standard, bpm, energy, confidence, provenance, created_at
             FROM track_opinions WHERE track_id = ?1 ORDER BY id"
        )?;
        let opinions = stmt
            .query_map(params![track_id], |row| {
                let source_str: String = row.get(2)?;
                let source = crate::consensus::OpinionSource::from_str(&source_str)
                    .unwrap_or(crate::consensus::OpinionSource::Tunelock);
                Ok(crate::consensus::TrackOpinion {
                    id: row.get(0)?,
                    track_id: row.get(1)?,
                    source,
                    key_camelot: row.get(3)?,
                    key_standard: row.get(4)?,
                    bpm: row.get(5)?,
                    energy: row.get(6)?,
                    confidence: row.get(7)?,
                    provenance: row.get(8)?,
                    created_at: row.get(9)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(opinions)
    }

    /// Get opinions for multiple tracks (batch, for library display).
    pub fn get_opinions_batch(&self, track_ids: &[i64]) -> Result<std::collections::HashMap<i64, Vec<crate::consensus::TrackOpinion>>> {
        if track_ids.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        let placeholders = track_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!(
            "SELECT id, track_id, source, key_camelot, key_standard, bpm, energy, confidence, provenance, created_at
             FROM track_opinions WHERE track_id IN ({}) ORDER BY id",
            placeholders
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let params: Vec<&dyn rusqlite::ToSql> = track_ids.iter().map(|id| id as &dyn rusqlite::ToSql).collect();
        let opinions = stmt
            .query_map(&*params, |row| {
                let source_str: String = row.get(2)?;
                let source = crate::consensus::OpinionSource::from_str(&source_str)
                    .unwrap_or(crate::consensus::OpinionSource::Tunelock);
                Ok((
                    row.get::<_, i64>(1)?,
                    crate::consensus::TrackOpinion {
                        id: row.get(0)?,
                        track_id: row.get(1)?,
                        source,
                        key_camelot: row.get(3)?,
                        key_standard: row.get(4)?,
                        bpm: row.get(5)?,
                        energy: row.get(6)?,
                        confidence: row.get(7)?,
                        provenance: row.get(8)?,
                        created_at: row.get(9)?,
                    },
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let mut map: std::collections::HashMap<i64, Vec<crate::consensus::TrackOpinion>> = std::collections::HashMap::new();
        for (track_id, opinion) in opinions {
            map.entry(track_id).or_default().push(opinion);
        }
        Ok(map)
    }

    /// Get tracks that have contested opinions (disagreement between sources).
    /// Ordered by disagreement count descending, then by track id.
    pub fn get_contested_tracks(&self, limit: usize) -> Result<Vec<i64>> {
        // Find tracks where at least two sources disagree on the key.
        let mut stmt = self.conn.prepare(
            "SELECT track_id, COUNT(DISTINCT key_camelot) as distinct_keys
             FROM track_opinions
             WHERE key_camelot IS NOT NULL
             GROUP BY track_id
             HAVING distinct_keys > 1
             ORDER BY distinct_keys DESC, track_id
             LIMIT ?1"
        )?;
        let track_ids = stmt
            .query_map(params![limit as i64], |row| row.get::<_, i64>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(track_ids)
    }

    // ========================================================================
    // Gold set annotation methods (Step 6)
    // ========================================================================

    pub fn save_gold_annotation(&self, ann: &GoldAnnotation) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO gold_annotations
             (track_id, key_tonic, key_mode, modulates, modulation_note,
              annotator_confidence, evidence, annotator_id, blind)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                ann.track_id,
                ann.key_tonic,
                ann.key_mode,
                ann.modulates,
                ann.modulation_note,
                ann.annotator_confidence,
                ann.evidence,
                ann.annotator_id,
                ann.blind,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn get_gold_annotations(&self, track_id: i64) -> Result<Vec<GoldAnnotation>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, track_id, key_tonic, key_mode, modulates, modulation_note,
                    annotator_confidence, evidence, annotator_id, blind, created_at
             FROM gold_annotations WHERE track_id = ?1 ORDER BY created_at"
        )?;
        let rows = stmt.query_map(params![track_id], |row| {
            Ok(GoldAnnotation {
                id: Some(row.get(0)?),
                track_id: row.get(1)?,
                key_tonic: row.get(2)?,
                key_mode: row.get(3)?,
                modulates: row.get(4)?,
                modulation_note: row.get(5)?,
                annotator_confidence: row.get(6)?,
                evidence: row.get(7)?,
                annotator_id: row.get(8)?,
                blind: row.get(9)?,
                created_at: Some(row.get(10)?),
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn get_gold_annotation_summary(&self) -> Result<GoldAnnotationSummary> {
        let total_tracks: usize = self.conn.query_row(
            "SELECT COUNT(*) FROM tracks", [], |row| row.get(0)
        )?;
        let annotated_tracks: usize = self.conn.query_row(
            "SELECT COUNT(DISTINCT track_id) FROM gold_annotations", [], |row| row.get(0)
        )?;
        let total_annotations: usize = self.conn.query_row(
            "SELECT COUNT(*) FROM gold_annotations", [], |row| row.get(0)
        )?;

        // Self-agreement: for tracks with 2+ annotations from 'self',
        // what fraction have the same (key_tonic, key_mode)?
        let self_agreement: Option<f64> = self.conn.query_row(
            "SELECT
                CASE WHEN COUNT(*) = 0 THEN NULL
                     ELSE CAST(SUM(CASE WHEN agree = 1 THEN 1 ELSE 0 END) AS REAL) / COUNT(*)
                END
             FROM (
                SELECT track_id,
                       CASE WHEN COUNT(DISTINCT key_tonic || key_mode) = 1 THEN 1 ELSE 0 END AS agree
                FROM gold_annotations
                WHERE annotator_id = 'self'
                GROUP BY track_id
                HAVING COUNT(*) >= 2
             )",
            [],
            |row| row.get(0),
        ).unwrap_or(None);

        // Mode distribution
        let mut stmt = self.conn.prepare(
            "SELECT key_mode, COUNT(*) FROM gold_annotations GROUP BY key_mode"
        )?;
        let mode_dist: std::collections::HashMap<String, usize> = stmt
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, usize>(1)?)))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(GoldAnnotationSummary {
            total_tracks,
            annotated_tracks,
            total_annotations,
            self_agreement_pct: self_agreement,
            mode_distribution: mode_dist,
        })
    }

    pub fn save_training_session(&self, session: &TrainingSession) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO training_sessions
             (session_type, track_id, presented_tonic, presented_mode,
              user_answer, correct, response_time_s)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                session.session_type,
                session.track_id,
                session.presented_tonic,
                session.presented_mode,
                session.user_answer,
                session.correct,
                session.response_time_s,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn get_training_stats(&self) -> Result<TrainingStats> {
        let total: usize = self.conn.query_row(
            "SELECT COUNT(*) FROM training_sessions", [], |row| row.get(0)
        )?;
        let correct: usize = self.conn.query_row(
            "SELECT COUNT(*) FROM training_sessions WHERE correct = 1", [], |row| row.get(0)
        )?;

        let mut stmt = self.conn.prepare(
            "SELECT session_type, COUNT(*), SUM(CASE WHEN correct = 1 THEN 1 ELSE 0 END)
             FROM training_sessions GROUP BY session_type"
        )?;
        let by_type: std::collections::HashMap<String, (usize, usize)> = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, (row.get::<_, usize>(1)?, row.get::<_, usize>(2)?)))
            })?
            .filter_map(|r| r.ok())
            .collect();

        let accuracy = if total > 0 { correct as f64 / total as f64 * 100.0 } else { 0.0 };

        Ok(TrainingStats {
            total_sessions: total,
            correct_count: correct,
            accuracy_pct: accuracy,
            by_type,
        })
    }

    // ========================================================================
    // Transition Workbench: Beat grid methods
    // ========================================================================

    pub fn get_beat_grid(&self, track_id: i64) -> Result<Option<BeatGrid>> {
        let row = self.conn.query_row(
            "SELECT track_id, source, bpm, first_beat_ms, meter_numerator,
                    downbeat_offset_beats, confidence, is_override
             FROM beat_grids
             WHERE track_id = ?
             ORDER BY is_override DESC, created_at DESC LIMIT 1",
            params![track_id],
            |row| {
                Ok(BeatGrid {
                    track_id: row.get(0)?,
                    source: row.get(1)?,
                    bpm: row.get(2)?,
                    first_beat_ms: row.get(3)?,
                    meter_numerator: row.get(4)?,
                    downbeat_offset_beats: row.get(5)?,
                    confidence: row.get(6)?,
                    is_override: row.get::<_, i32>(7)? != 0,
                })
            },
        );
        match row {
            Ok(grid) => Ok(Some(grid)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn save_beat_grid(&self, grid: &BeatGrid) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO beat_grids
             (track_id, source, bpm, first_beat_ms, meter_numerator,
              downbeat_offset_beats, confidence, is_override, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, datetime('now'))",
            params![
                grid.track_id, grid.source, grid.bpm, grid.first_beat_ms,
                grid.meter_numerator, grid.downbeat_offset_beats,
                grid.confidence, grid.is_override as i32,
            ],
        )?;
        Ok(())
    }

    pub fn save_beat_grid_override(
        &self, track_id: i64, bpm: f64, first_beat_ms: i64,
        meter_numerator: i32, downbeat_offset_beats: i32,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO beat_grids
             (track_id, source, bpm, first_beat_ms, meter_numerator,
              downbeat_offset_beats, confidence, is_override, updated_at)
             VALUES (?, 'manual', ?, ?, ?, ?, NULL, 1, datetime('now'))",
            params![track_id, bpm, first_beat_ms, meter_numerator, downbeat_offset_beats],
        )?;
        Ok(())
    }

    pub fn reset_beat_grid_override(&self, track_id: i64) -> Result<()> {
        self.conn.execute(
            "DELETE FROM beat_grids WHERE track_id = ? AND is_override = 1",
            params![track_id],
        )?;
        Ok(())
    }

    // ========================================================================
    // PB-6.0: Loudness analysis methods
    // ========================================================================

    pub fn get_loudness(&self, track_id: i64) -> Result<Option<LoudnessAnalysis>> {
        let row = self.conn.query_row(
            "SELECT track_id, integrated_lufs, true_peak_dbtp, sample_peak_dbfs,
                    analysis_version, sample_rate, duration_sec
             FROM loudness_analysis WHERE track_id = ?",
            params![track_id],
            |row| {
                Ok(LoudnessAnalysis {
                    track_id: row.get(0)?,
                    integrated_lufs: row.get(1)?,
                    true_peak_dbtp: row.get(2)?,
                    sample_peak_dbfs: row.get(3)?,
                    analysis_version: row.get(4)?,
                    sample_rate: row.get(5)?,
                    duration_sec: row.get(6)?,
                })
            },
        );
        match row {
            Ok(analysis) => Ok(Some(analysis)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn save_loudness(&self, analysis: &LoudnessAnalysis) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO loudness_analysis
             (track_id, integrated_lufs, true_peak_dbtp, sample_peak_dbfs,
              analysis_version, sample_rate, duration_sec, analyzed_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, datetime('now'))",
            params![
                analysis.track_id,
                analysis.integrated_lufs,
                analysis.true_peak_dbtp,
                analysis.sample_peak_dbfs,
                analysis.analysis_version,
                analysis.sample_rate,
                analysis.duration_sec,
            ],
        )?;
        Ok(())
    }

    // ========================================================================
    // Transition Workbench: Transition plan methods
    // ========================================================================

    pub fn get_transition_plan(&self, playlist_id: i64, transition_id: &str) -> Result<Option<TransitionPlan>> {
        let row = self.conn.query_row(
            "SELECT playlist_id, transition_id, schema_version, plan_json
             FROM transition_plans WHERE playlist_id = ? AND transition_id = ?",
            params![playlist_id, transition_id],
            |row| Ok(TransitionPlan {
                playlist_id: row.get(0)?,
                transition_id: row.get(1)?,
                schema_version: row.get(2)?,
                plan_json: row.get(3)?,
            }),
        );
        match row {
            Ok(plan) => Ok(Some(plan)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn save_transition_plan(&self, plan: &TransitionPlan) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO transition_plans
             (playlist_id, transition_id, schema_version, plan_json, updated_at)
             VALUES (?, ?, ?, ?, datetime('now'))",
            params![plan.playlist_id, plan.transition_id, plan.schema_version, plan.plan_json],
        )?;
        Ok(())
    }

    // ========================================================================
    // Transition Workbench: Stem manifest methods
    // ========================================================================

    pub fn get_stem_manifest(&self, track_id: i64) -> Result<Option<StemManifest>> {
        let row = self.conn.query_row(
            "SELECT track_id, source_fingerprint, provider, model, model_version,
                    vocals_path, drums_path, bass_path, other_path,
                    duration_ms, alignment_offset_ms, status, storage_bytes
             FROM stem_manifests WHERE track_id = ? AND status = 'ready'
             ORDER BY created_at DESC LIMIT 1",
            params![track_id],
            |row| Ok(StemManifest {
                track_id: row.get(0)?,
                source_fingerprint: row.get(1)?,
                provider: row.get(2)?,
                model: row.get(3)?,
                model_version: row.get(4)?,
                vocals_path: row.get(5)?,
                drums_path: row.get(6)?,
                bass_path: row.get(7)?,
                other_path: row.get(8)?,
                duration_ms: row.get(9)?,
                alignment_offset_ms: row.get(10)?,
                status: row.get(11)?,
                storage_bytes: row.get(12)?,
            }),
        );
        match row {
            Ok(manifest) => Ok(Some(manifest)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn save_stem_manifest(&self, manifest: &StemManifest) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO stem_manifests
             (track_id, source_fingerprint, provider, model, model_version,
              vocals_path, drums_path, bass_path, other_path,
              duration_ms, alignment_offset_ms, status, storage_bytes, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, datetime('now'))",
            params![
                manifest.track_id, manifest.source_fingerprint, manifest.provider,
                manifest.model, manifest.model_version, manifest.vocals_path,
                manifest.drums_path, manifest.bass_path, manifest.other_path,
                manifest.duration_ms, manifest.alignment_offset_ms,
                manifest.status, manifest.storage_bytes,
            ],
        )?;
        Ok(())
    }

    pub fn delete_stem_manifest(&self, track_id: i64) -> Result<()> {
        self.conn.execute(
            "DELETE FROM stem_manifests WHERE track_id = ?",
            params![track_id],
        )?;
        Ok(())
    }

    // ── PB-2 Listening Lab ───────────────────────────────────────────

    pub fn save_listening_lab_result(
        &self,
        timestamp: &str,
        processor: &str,
        tempo_percent: f64,
        pitch_semitones: f64,
        material: &str,
        track_name: Option<&str>,
        transients: u8,
        bass: u8,
        vocals: u8,
        stereo: u8,
        artifacts: u8,
        overall: u8,
        abx_correct: Option<u32>,
        abx_trials: Option<u32>,
        notes: Option<&str>,
        git_revision: Option<&str>,
    ) -> Result<i64> {
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS listening_lab_results (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp TEXT NOT NULL,
                processor TEXT NOT NULL,
                tempo_percent REAL NOT NULL,
                pitch_semitones REAL NOT NULL,
                material TEXT NOT NULL,
                track_name TEXT,
                transients INTEGER NOT NULL,
                bass INTEGER NOT NULL,
                vocals INTEGER NOT NULL,
                stereo INTEGER NOT NULL,
                artifacts INTEGER NOT NULL,
                overall INTEGER NOT NULL,
                abx_correct INTEGER,
                abx_trials INTEGER,
                notes TEXT,
                git_revision TEXT
            )",
            [],
        )?;
        // Add column if upgrading from older schema
        let _ = self.conn.execute(
            "ALTER TABLE listening_lab_results ADD COLUMN git_revision TEXT",
            [],
        );
        self.conn.execute(
            "INSERT INTO listening_lab_results (
                timestamp, processor, tempo_percent, pitch_semitones, material,
                track_name, transients, bass, vocals, stereo, artifacts, overall,
                abx_correct, abx_trials, notes, git_revision
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
            params![
                timestamp,
                processor,
                tempo_percent,
                pitch_semitones,
                material,
                track_name,
                transients as i64,
                bass as i64,
                vocals as i64,
                stereo as i64,
                artifacts as i64,
                overall as i64,
                abx_correct.map(|v| v as i64),
                abx_trials.map(|v| v as i64),
                notes,
                git_revision,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn get_listening_lab_results(&self) -> Result<Vec<crate::commands::ListeningLabResult>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, timestamp, processor, tempo_percent, pitch_semitones,
             material, track_name, transients, bass, vocals, stereo, artifacts,
             overall, abx_correct, abx_trials, notes, git_revision
             FROM listening_lab_results ORDER BY timestamp DESC LIMIT 500",
        )?;
        let results = stmt.query_map([], |row| {
            Ok(crate::commands::ListeningLabResult {
                id: row.get(0)?,
                timestamp: row.get(1)?,
                processor: row.get(2)?,
                tempo_percent: row.get(3)?,
                pitch_semitones: row.get(4)?,
                material: row.get(5)?,
                track_name: row.get(6)?,
                transients: row.get::<_, i64>(7)? as u8,
                bass: row.get::<_, i64>(8)? as u8,
                vocals: row.get::<_, i64>(9)? as u8,
                stereo: row.get::<_, i64>(10)? as u8,
                artifacts: row.get::<_, i64>(11)? as u8,
                overall: row.get::<_, i64>(12)? as u8,
                abx_correct: row.get::<_, Option<i64>>(13)?.map(|v| v as u32),
                abx_trials: row.get::<_, Option<i64>>(14)?.map(|v| v as u32),
                notes: row.get(15)?,
                git_revision: row.get(16)?,
            })
        })?;
        let mut out = Vec::new();
        for r in results {
            out.push(r?);
        }
        Ok(out)
    }
}
