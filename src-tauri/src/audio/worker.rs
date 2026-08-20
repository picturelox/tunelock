// Worker thread — background decoding and resampling.
//
// The worker thread runs on a normal (non-real-time) thread. It can:
//   - Open files
//   - Decode with Symphonia
//   - Resample with Rubato to the output sample rate
//   - Allocate memory freely
//
// The result is a DecodedBuffer sent to the audio callback via the command
// queue. For large files, future versions will stream through a ring buffer
// instead of loading the entire file at once.

use anyhow::{Context, Result};
use rubato::{Resampler, SincFixedIn, SincInterpolationType, SincInterpolationParameters, WindowFunction};

use super::command::DecodedBuffer;

/// Result of decoding a file — the decoded buffer plus metadata.
pub struct DecodeResult {
    pub buffer: DecodedBuffer,
}

/// Decode an audio file to interleaved f32 at the target sample rate.
/// This runs on a worker thread, not the audio callback.
pub fn decode_file(path: &str, target_sample_rate: u32) -> Result<DecodedBuffer> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    let file = std::fs::File::open(path)
        .with_context(|| format!("Failed to open file: {}", path))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = std::path::Path::new(path).extension() {
        if let Some(ext_str) = ext.to_str() {
            hint.with_extension(ext_str);
        }
    }

    let format_opts: FormatOptions = Default::default();
    let metadata_opts: MetadataOptions = Default::default();
    let decoder_opts: DecoderOptions = Default::default();

    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &format_opts, &metadata_opts)
        .context("Failed to probe audio format")?;

    let mut format = probed.format;

    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .context("No audio track found")?;

    let track_id = track.id;
    let source_sample_rate = track.codec_params.sample_rate.unwrap_or(44100);
    let channels = track
        .codec_params
        .channels
        .map(|c| c.count())
        .unwrap_or(2) as u16;

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &decoder_opts)
        .context("Failed to create decoder")?;

    let mut samples: Vec<f32> = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(symphonia::core::errors::Error::IoError(e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(_) => break,
        };

        if packet.track_id() != track_id {
            continue;
        }

        match decoder.decode(&packet) {
            Ok(decoded) => {
                let spec = *decoded.spec();
                let capacity = decoded.capacity() as u64;
                let mut sample_buf: SampleBuffer<f32> = SampleBuffer::new(capacity, spec);
                sample_buf.copy_interleaved_ref(decoded);
                samples.extend_from_slice(sample_buf.samples());
            }
            Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
            Err(_) => break,
        }
    }

    let duration_sec = samples.len() as f64 / (source_sample_rate as f64 * channels as f64);

    // Resample if source rate doesn't match target
    let final_samples = if source_sample_rate != target_sample_rate {
        resample(&samples, source_sample_rate, target_sample_rate, channels as usize)?
    } else {
        samples
    };

    Ok(DecodedBuffer {
        samples: final_samples,
        sample_rate: target_sample_rate,
        channels,
        duration_sec,
        bpm: None, // BPM is set separately from analysis results
    })
}

/// Resample interleaved audio using Rubato (band-limited sinc interpolation).
fn resample(
    samples: &[f32],
    source_rate: u32,
    target_rate: u32,
    channels: usize,
) -> Result<Vec<f32>> {
    let ratio = target_rate as f64 / source_rate as f64;

    let params = SincInterpolationParameters {
        sinc_len: 256,
        f_cutoff: 0.95,
        interpolation: SincInterpolationType::Linear,
        oversampling_factor: 256,
        window: WindowFunction::BlackmanHarris2,
    };

    // De-interleave into per-channel vectors
    let frames = samples.len() / channels;
    let mut input: Vec<Vec<f32>> = (0..channels).map(|_| Vec::with_capacity(frames)).collect();
    for i in 0..frames {
        for ch in 0..channels {
            input[ch].push(samples[i * channels + ch]);
        }
    }

    let mut resampler = SincFixedIn::<f32>::new(
        ratio,
        2.0,
        params,
        frames,
        channels,
    )?;

    // Process in chunks
    let chunk_size = frames;
    let output = resampler.process(&input, None)?;

    // Re-interleave
    let out_frames = output[0].len();
    let mut result = Vec::with_capacity(out_frames * channels);
    for i in 0..out_frames {
        for ch in 0..channels {
            result.push(output[ch][i]);
        }
    }

    Ok(result)
}
