use anyhow::{Context, Result};
use std::path::Path;
use std::process::{Command, Stdio};

/// Decode any media file to mono f32 samples at the target sample rate
/// using ffmpeg as a sidecar.
///
/// ffmpeg reads the input file, decodes the first audio stream, downmixes
/// to mono, resamples to `target_rate`, and outputs raw 32-bit float
/// little-endian samples on stdout:
///
///   ffmpeg -i <input> -f f32le -acodec pcm_f32le -ac 1 -ar <rate> -
///
/// This handles:
///   - Video containers (.mp4, .mov, .webm, .mkv, etc.) — extracts audio
///   - WAV files with non-standard headers that Symphonia rejects
///   - Any codec ffmpeg supports that Symphonia doesn't
///
/// The caller is responsible for checking `tools::ffmpeg_available()`
/// before calling this function.
pub fn decode_to_pcm<P: AsRef<Path>>(path: P, target_rate: u32) -> Result<Vec<f32>> {
    let path_ref = path.as_ref();

    let mut cmd = Command::new("ffmpeg");
    cmd.args([
        "-i",
        path_ref
            .to_str()
            .context("Path is not valid UTF-8")?,
        "-f",
        "f32le",
        "-acodec",
        "pcm_f32le",
        "-ac",
        "1", // mono
        "-ar",
        &target_rate.to_string(),
        "-", // output to stdout
    ]);
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let output = cmd
        .output()
        .with_context(|| format!("Failed to spawn ffmpeg for {:?}", path_ref))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Trim to the last few lines for a concise error.
        let trimmed = stderr
            .lines()
            .filter(|l| l.starts_with("Error") || l.contains("Error") || l.contains("error"))
            .last()
            .unwrap_or("unknown ffmpeg error");
        return Err(anyhow::anyhow!("ffmpeg failed: {}", trimmed));
    }

    // Convert raw f32 LE bytes to Vec<f32>.
    let bytes = &output.stdout;
    if bytes.is_empty() {
        return Err(anyhow::anyhow!("ffmpeg produced no output"));
    }

    let samples: Vec<f32> = bytes
        .chunks_exact(4)
        .map(|chunk| {
            f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]])
        })
        .collect();

    if samples.is_empty() {
        return Err(anyhow::anyhow!("ffmpeg produced 0 samples"));
    }

    // ffmpeg outputs f32 in [-1, 1] already, but clamp out-of-range values
    // from edge-case codecs.
    let mut samples = samples;
    if let Some(max) = samples.iter().map(|&s| s.abs()).max_by(|a, b| a.partial_cmp(b).unwrap()) {
        if max > 1.0 {
            let scale = 1.0 / max;
            for sample in &mut samples {
                *sample *= scale;
            }
        }
    }

    Ok(samples)
}
