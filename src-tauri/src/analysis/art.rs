//! Album-art extraction.
//!
//! Pulls the first embedded picture from a tagged audio file, decides on a
//! reasonable extension, and writes it to the app's artwork cache directory
//! as `<track_id>.<ext>`. Returns the absolute path on success.
//!
//! Design choices:
//!   * Cache by track_id, not by content hash. Tracks rarely change their
//!     embedded art and we don't need de-dup; making the path predictable
//!     means the frontend can rebuild it from the track id alone if needed.
//!   * Best-effort: any failure (no tag, no picture, IO error) returns
//!     `Ok(None)` rather than aborting the whole analysis. Album art is a
//!     nice-to-have, not a blocker.
//!   * Extension is sniffed from the picture's reported mime type with
//!     conservative fallback to `.jpg` (most embedded covers are JPEG).

use anyhow::Result;
use lofty::file::TaggedFileExt;
use lofty::picture::MimeType;
use lofty::probe::Probe;
use std::fs;
use std::path::{Path, PathBuf};

/// Map a lofty MimeType to a file extension. Defaults to "jpg" because the
/// vast majority of embedded covers are JPEG, and any image with a clean
/// JPEG body will display fine even if its sniffed mime is unknown.
fn ext_for_mime(mime: Option<&MimeType>) -> &'static str {
    match mime {
        Some(MimeType::Png) => "png",
        Some(MimeType::Jpeg) => "jpg",
        Some(MimeType::Tiff) => "tif",
        Some(MimeType::Bmp) => "bmp",
        Some(MimeType::Gif) => "gif",
        _ => "jpg",
    }
}

/// Extract the first picture embedded in `audio_path` and write it to
/// `cache_dir/<track_id>.<ext>`. Returns the absolute path of the written
/// file, or `Ok(None)` if there is no usable picture.
///
/// Side-effects: creates `cache_dir` if it doesn't exist; overwrites any
/// existing file at the target path (treats it as a cache).
pub fn extract_and_cache_artwork(
    audio_path: &Path,
    cache_dir: &Path,
    track_id: i64,
) -> Result<Option<PathBuf>> {
    // Probe for tags. If anything along this chain fails (file missing, no
    // tags, decode error) we treat that as "no art" rather than propagating
    // the error; analysis already handled decode/IO concerns upstream.
    let tagged = match Probe::open(audio_path).and_then(|p| p.read()) {
        Ok(t) => t,
        Err(_) => return Ok(None),
    };

    // Pick the first picture from the primary tag, then any tag, in that
    // order. Most files only have one cover, so this is effectively
    // "give me a cover if any exists".
    let picture = tagged
        .primary_tag()
        .and_then(|t| t.pictures().first().cloned())
        .or_else(|| {
            tagged
                .tags()
                .iter()
                .flat_map(|t| t.pictures())
                .next()
                .cloned()
        });

    let Some(pic) = picture else {
        return Ok(None);
    };

    let bytes = pic.data();
    if bytes.is_empty() {
        return Ok(None);
    }

    fs::create_dir_all(cache_dir)?;
    let ext = ext_for_mime(pic.mime_type());
    let out_path = cache_dir.join(format!("{}.{}", track_id, ext));
    fs::write(&out_path, bytes)?;
    Ok(Some(out_path))
}
