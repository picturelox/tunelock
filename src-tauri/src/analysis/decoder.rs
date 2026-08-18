use anyhow::{Context, Result};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use std::fs::File;
use std::path::Path;

use super::SAMPLE_RATE;

/// Decode audio file to mono f32 samples at the analysis sample rate
/// (`super::SAMPLE_RATE`, currently 11.025 kHz).
///
/// Resampling from typical source rates (44.1 / 48 kHz) is a significant
/// **downsample**, so we apply a moving-average anti-alias lowpass before
/// the linear resampler to keep frequencies above the new Nyquist from
/// folding back as noise. See `resample` for details.
pub fn decode_audio<P: AsRef<Path>>(path: P) -> Result<Vec<f32>> {
    let file = File::open(path.as_ref())
        .with_context(|| format!("Failed to open file: {:?}", path.as_ref()))?;
    
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    
    let hint = Hint::new();
    let format_opts: FormatOptions = Default::default();
    let metadata_opts: MetadataOptions = Default::default();
    let decoder_opts: DecoderOptions = Default::default();
    
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &format_opts, &metadata_opts)
        .with_context(|| "Failed to probe audio format")?;
    
    let mut format = probed.format;
    
    // Find the first audio track
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .context("No audio track found")?;
    
    let track_id = track.id;
    
    // Get sample rate
    let sample_rate = track.codec_params.sample_rate.unwrap_or(44100);
    
    // Create decoder
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &decoder_opts)
        .context("Failed to create decoder")?;
    
    let mut samples = Vec::new();
    
    // Decode packets
    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(symphonia::core::errors::Error::IoError(e)) 
                if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e.into()),
        };
        
        if packet.track_id() != track_id {
            continue;
        }
        
        match decoder.decode(&packet) {
            Ok(decoded) => {
                // Convert any format to interleaved f32 samples
                let spec = *decoded.spec();
                let capacity = decoded.capacity() as u64;
                let mut sample_buf: SampleBuffer<f32> = SampleBuffer::new(capacity, spec);
                sample_buf.copy_interleaved_ref(decoded);
                
                let channels = spec.channels.count();
                let interleaved = sample_buf.samples();
                let frames = interleaved.len() / channels;
                
                for frame in 0..frames {
                    let mut sum = 0.0f32;
                    for ch in 0..channels {
                        sum += interleaved[frame * channels + ch];
                    }
                    samples.push(sum / channels as f32);
                }
            }
            Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
            Err(e) => return Err(e.into()),
        }
    }
    
    // Resample to the analysis sample rate if necessary.
    if sample_rate as usize != SAMPLE_RATE {
        samples = resample(&samples, sample_rate, SAMPLE_RATE as u32);
    }
    
    // Normalize
    if let Some(max) = samples.iter().map(|&s| s.abs()).max_by(|a, b| a.partial_cmp(b).unwrap()) {
        if max > 0.0 {
            let scale = 1.0 / max;
            for sample in &mut samples {
                *sample *= scale;
            }
        }
    }
    
    Ok(samples)
}

/// Resample mono audio from `from_rate` to `to_rate`.
///
/// For a significant **downsample** (the common case when going from a 44.1 /
/// 48 kHz source to our 11 kHz analysis rate), we first apply a centered
/// moving-average lowpass with window size `floor(from_rate / to_rate)`.
/// This is the simplest correct anti-alias filter \u2014 it attenuates
/// frequencies above the new Nyquist enough that they don\u2019t fold back into
/// the audible band and pollute the chroma. (Sharper filters exist; a boxcar
/// is good enough for MIR and is allocation-cheap.)
///
/// After the lowpass, a single-pass linear interpolation produces the output
/// at the target rate.
fn resample(input: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if from_rate == to_rate {
        return input.to_vec();
    }

    // ---- Anti-alias lowpass when downsampling -----------------------------
    // Only apply when from_rate is meaningfully higher than to_rate. The 11/10
    // factor avoids triggering for tiny rate mismatches (e.g. 48 -> 44.1).
    let filtered: Vec<f32>;
    let src: &[f32] = if from_rate > to_rate * 11 / 10 {
        let window = (from_rate as usize / to_rate as usize).max(2);
        filtered = moving_average(input, window);
        &filtered
    } else {
        input
    };

    let ratio = to_rate as f64 / from_rate as f64;
    let output_len = (src.len() as f64 * ratio) as usize;
    let mut output = Vec::with_capacity(output_len);

    for i in 0..output_len {
        let src_idx = i as f64 / ratio;
        let src_idx_floor = src_idx.floor() as usize;
        let frac = src_idx - src_idx.floor();

        if src_idx_floor + 1 >= src.len() {
            output.push(src[src.len() - 1]);
        } else {
            let a = src[src_idx_floor];
            let b = src[src_idx_floor + 1];
            output.push(a + (b - a) * frac as f32);
        }
    }

    output
}

/// Simple causal moving-average lowpass. Window is in samples.
///
/// Used as the anti-alias filter ahead of `resample` when significantly
/// reducing the sample rate. A boxcar isn\u2019t the sharpest possible filter
/// but it has a clean DC response, costs O(n), and is good enough that the
/// chroma is dominated by signal rather than aliased high-frequency hash.
fn moving_average(input: &[f32], window: usize) -> Vec<f32> {
    if window <= 1 || input.is_empty() {
        return input.to_vec();
    }
    let mut output = Vec::with_capacity(input.len());
    let mut sum: f32 = 0.0;
    let mut buf = vec![0.0_f32; window];
    for (i, &s) in input.iter().enumerate() {
        sum -= buf[i % window];
        sum += s;
        buf[i % window] = s;
        let n = (i + 1).min(window);
        output.push(sum / n as f32);
    }
    output
}
