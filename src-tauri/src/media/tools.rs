use std::process::Command;

/// Check if a given executable is available on PATH.
fn on_path(name: &str) -> bool {
    Command::new(name)
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok()
}

/// Returns true if ffmpeg is available on PATH.
pub fn ffmpeg_available() -> bool {
    on_path("ffmpeg")
}

/// Returns true if yt-dlp is available on PATH.
pub fn ytdlp_available() -> bool {
    on_path("yt-dlp")
}

/// Returns true if fpcalc (Chromaprint) is available on PATH.
pub fn fpcalc_available() -> bool {
    on_path("fpcalc")
}

/// Which external tools are currently available.
pub struct ToolAvailability {
    pub ffmpeg: bool,
    pub yt_dlp: bool,
    pub fpcalc: bool,
}

impl ToolAvailability {
    /// Probe all supported external tools.
    pub fn detect() -> Self {
        Self {
            ffmpeg: ffmpeg_available(),
            yt_dlp: ytdlp_available(),
            fpcalc: fpcalc_available(),
        }
    }
}

/// File extensions that Symphonia can handle natively (with the current
/// feature set: mp3, flac, wav, ogg, pcm, aac, alac, isomp4, aiff).
pub const SYMPHONIA_EXTENSIONS: &[&str] = &[
    "mp3", "flac", "wav", "ogg", "oga", "opus", "aiff", "aif", "m4a", "mp4",
    "m4b", "m4p", "alac", "aac", "wma", "mkv",
];

/// File extensions that require the ffmpeg sidecar (video containers and
/// formats Symphonia doesn't support).
pub const FFMPEG_REQUIRED_EXTENSIONS: &[&str] = &[
    "mp4", "mov", "webm", "mkv", "m4v", "avi", "flv", "wmv", "mpg", "mpeg",
    "ts", "3gp",
];

/// Check if a file extension is a video container that requires ffmpeg.
pub fn is_video_extension(ext: &str) -> bool {
    FFMPEG_REQUIRED_EXTENSIONS.contains(&ext.to_lowercase().as_str())
}
