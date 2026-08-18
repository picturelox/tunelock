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
        self.conn.execute(
            "INSERT INTO tracks (file_path, filename, file_size, status) 
             VALUES (?1, ?2, ?3, 'pending')
             ON CONFLICT(file_path) DO UPDATE SET 
             file_size = excluded.file_size,
             updated_at = datetime('now')",
            params![path, filename, file_size],
        )?;
        Ok(self.conn.last_insert_rowid())
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
}
