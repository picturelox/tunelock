//! Phase 7 — non-destructive file export.
//!
//! Copies source audio files to a target folder, optionally:
//!   * renaming with a numeric position prefix (playlist order)
//!   * writing key/BPM/comment tags into the **copy** (never the source)
//!   * emitting an M3U8 playlist file (universally supported by Rekordbox,
//!     Serato, Traktor, Engine DJ, VirtualDJ, etc.)
//!
//! By design this module never mutates the original files.

use anyhow::{Context, Result};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::models::{ExportOptions, Track};

pub struct ExportReport {
    pub copied: usize,
    pub failed: usize,
    pub playlist_path: Option<PathBuf>,
}

/// Export a list of tracks to `target_dir`. Creates the folder if needed.
pub fn export_tracks(
    tracks: &[Track],
    target_dir: &Path,
    opts: &ExportOptions,
) -> Result<ExportReport> {
    fs::create_dir_all(target_dir)
        .with_context(|| format!("Failed to create target dir {:?}", target_dir))?;
    
    let mut copied = 0;
    let mut failed = 0;
    let mut copied_names: Vec<String> = Vec::with_capacity(tracks.len());
    
    for (idx, track) in tracks.iter().enumerate() {
        let src = Path::new(&track.file_path);
        let name = if opts.number_prefix {
            format!("{:03}_{}", idx + 1, track.filename)
        } else {
            track.filename.clone()
        };
        let dest = target_dir.join(&name);
        
        match fs::copy(src, &dest) {
            Ok(_) => {
                if opts.write_tags {
                    if let Err(e) = write_tags_to_copy(&dest, track) {
                        eprintln!("[export] tag write failed for {:?}: {}", dest, e);
                    }
                }
                copied += 1;
                copied_names.push(name);
            }
            Err(e) => {
                eprintln!("[export] copy failed {:?} -> {:?}: {}", src, dest, e);
                failed += 1;
            }
        }
    }
    
    // Write M3U8 playlist alongside the files (always in universal format).
    let playlist_path = if !copied_names.is_empty() {
        let p = target_dir.join("playlist.m3u8");
        write_m3u8(&p, &copied_names)?;
        Some(p)
    } else {
        None
    };
    
    Ok(ExportReport { copied, failed, playlist_path })
}

fn write_m3u8(path: &Path, names: &[String]) -> Result<()> {
    let mut f = fs::File::create(path)
        .with_context(|| format!("Failed to create M3U8 {:?}", path))?;
    writeln!(f, "#EXTM3U")?;
    for name in names {
        writeln!(f, "#EXTINF:-1,{}", name)?;
        writeln!(f, "{}", name)?;
    }
    Ok(())
}

/// Write key/BPM/Camelot into the **copy** using lofty. Never touches the source.
fn write_tags_to_copy(dest: &Path, track: &Track) -> Result<()> {
    use lofty::file::{TaggedFile, TaggedFileExt};
    use lofty::probe::Probe;
    use lofty::tag::{ItemKey, TagExt};
    
    let mut tagged: TaggedFile = Probe::open(dest)?.read()?;
    
    // Get or create the primary tag
    let tag = match tagged.primary_tag_mut() {
        Some(t) => t,
        None => {
            let tag_type = tagged.primary_tag_type();
            tagged.insert_tag(lofty::tag::Tag::new(tag_type));
            tagged.primary_tag_mut().unwrap()
        }
    };
    
    if let Some(key) = &track.key_standard {
        tag.insert_text(ItemKey::InitialKey, key.clone());
    }
    if let Some(camelot) = &track.key_camelot {
        // Many DJ apps read the Camelot notation out of the COMMENT field.
        tag.insert_text(ItemKey::Comment, format!("Camelot: {}", camelot));
    }
    if let Some(bpm) = track.bpm {
        tag.insert_text(ItemKey::Bpm, format!("{:.1}", bpm));
    }
    
    tag.save_to_path(dest, lofty::config::WriteOptions::default())?;
    Ok(())
}
