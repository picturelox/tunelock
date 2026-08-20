pub mod tools;
pub mod ffmpeg;

use anyhow::Result;
use std::path::Path;

use crate::analysis::SAMPLE_RATE;

/// Decode any media file to mono f32 samples at the analysis sample rate.
///
/// This is the unified entry point for audio/video decode. It tries the
/// native Symphonia path first (fast, no external dependencies), then
/// falls back to the ffmpeg sidecar for formats Symphonia cannot handle:
///
///   - Video containers (.mp4, .mov, .webm, .mkv, .m4v)
///   - WAV files with non-standard fmt chunks
///   - Any other format Symphonia doesn't have a codec for
///
/// If ffmpeg is not on PATH and Symphonia fails, the original error is
/// returned so the caller can report a meaningful failure reason.
pub fn decode_media<P: AsRef<Path>>(path: P) -> Result<Vec<f32>> {
    let path_ref = path.as_ref();

    // Try Symphonia first.
    match crate::analysis::decoder::decode_audio(path_ref) {
        Ok(samples) => return Ok(samples),
        Err(symphonia_err) => {
            // If ffmpeg is available, try it as a fallback.
            if tools::ffmpeg_available() {
                match ffmpeg::decode_to_pcm(path_ref, SAMPLE_RATE as u32) {
                    Ok(samples) => return Ok(samples),
                    Err(ffmpeg_err) => {
                        // Both failed — return the more informative error.
                        return Err(anyhow::anyhow!(
                            "Symphonia: {} | ffmpeg: {}",
                            symphonia_err,
                            ffmpeg_err
                        ));
                    }
                }
            }
            // ffmpeg not available — return the Symphonia error.
            return Err(symphonia_err);
        }
    }
}
