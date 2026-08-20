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
}
