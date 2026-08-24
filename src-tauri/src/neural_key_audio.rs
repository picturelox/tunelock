//! Real-file decode and resampling for the pinned Myna input contract.
//!
//! Unlike the classical analyzer's intentionally normalized 22.05 kHz path,
//! this path preserves decoded amplitude, averages channels in float32, and
//! mirrors torchaudio 2.7's default Hann-windowed sinc resampler at 16 kHz.

use std::fs::File;
use std::path::Path;

use anyhow::{bail, Context, Result};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use crate::neural_key_preprocess::MYNA_SAMPLE_RATE_HZ;

const LOWPASS_FILTER_WIDTH: f64 = 6.0;
const ROLLOFF: f64 = 0.99;

#[derive(Debug)]
pub struct MynaAudio {
    pub samples: Vec<f32>,
    pub source_sample_rate_hz: u32,
    pub source_channels: usize,
}

/// Decode the first native audio track, downmix it without normalization, and
/// reproduce Myna's torchaudio 16 kHz resampling contract.
pub fn decode_myna_audio(path: impl AsRef<Path>) -> Result<MynaAudio> {
    let path = path.as_ref();
    let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let source = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(extension) = path.extension().and_then(|value| value.to_str()) {
        hint.with_extension(extension);
    }
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            source,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .with_context(|| format!("probing {}", path.display()))?;
    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|track| track.codec_params.codec != CODEC_TYPE_NULL)
        .context("no native audio track found")?;
    let track_id = track.id;
    let source_sample_rate_hz = track
        .codec_params
        .sample_rate
        .context("audio track does not declare a sample rate")?;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .context("creating native audio decoder")?;
    let mut samples = Vec::new();
    let mut source_channels = None;

    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(symphonia::core::errors::Error::IoError(error))
                if error.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break
            }
            Err(error) => return Err(error).context("reading native audio packet"),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
            Err(error) => return Err(error).context("decoding native audio packet"),
        };
        if decoded.spec().rate != source_sample_rate_hz {
            bail!("audio sample rate changed within the stream");
        }
        let channels = decoded.spec().channels.count();
        if channels == 0 {
            bail!("decoded audio packet has no channels");
        }
        match source_channels {
            Some(expected) if expected != channels => {
                bail!("audio channel count changed within the stream")
            }
            None => source_channels = Some(channels),
            _ => {}
        }
        let spec = *decoded.spec();
        let mut buffer = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        buffer.copy_interleaved_ref(decoded);
        for frame in buffer.samples().chunks_exact(channels) {
            let mut sum = 0.0_f32;
            for sample in frame {
                sum += *sample;
            }
            samples.push(sum / channels as f32);
        }
    }

    let source_channels = source_channels.context("decoder produced no audio samples")?;
    if samples.is_empty() || samples.iter().any(|sample| !sample.is_finite()) {
        bail!("decoder produced empty or non-finite audio");
    }
    let samples = if source_sample_rate_hz == MYNA_SAMPLE_RATE_HZ as u32 {
        samples
    } else {
        torchaudio_sinc_resample(&samples, source_sample_rate_hz, MYNA_SAMPLE_RATE_HZ as u32)?
    };
    Ok(MynaAudio {
        samples,
        source_sample_rate_hz,
        source_channels,
    })
}

/// Match `torchaudio.transforms.Resample` defaults: width 6, rolloff 0.99,
/// and `sinc_interp_hann`. Kernel construction is performed in float64 and
/// stored in float32, matching torchaudio's default cached transform.
pub fn torchaudio_sinc_resample(
    input: &[f32],
    original_rate: u32,
    target_rate: u32,
) -> Result<Vec<f32>> {
    if input.is_empty() {
        bail!("cannot resample empty audio");
    }
    if input.iter().any(|sample| !sample.is_finite()) {
        bail!("cannot resample non-finite audio");
    }
    if original_rate == 0 || target_rate == 0 {
        bail!("audio sample rates must be positive");
    }
    if original_rate == target_rate {
        return Ok(input.to_vec());
    }

    let divisor = greatest_common_divisor(original_rate, target_rate);
    let original = (original_rate / divisor) as usize;
    let target = (target_rate / divisor) as usize;
    let base_frequency = original.min(target) as f64 * ROLLOFF;
    let width = (LOWPASS_FILTER_WIDTH * original as f64 / base_frequency).ceil() as usize;
    let kernel_length = 2 * width + original;
    let scale = base_frequency / original as f64;
    let mut kernels = Vec::with_capacity(target);

    for phase in 0..target {
        let mut sparse = Vec::new();
        for kernel_index in 0..kernel_length {
            let index = kernel_index as f64 - width as f64;
            let mut time =
                (-(phase as f64) / target as f64 + index / original as f64) * base_frequency;
            time = time.clamp(-LOWPASS_FILTER_WIDTH, LOWPASS_FILTER_WIDTH);
            let window = (time * std::f64::consts::PI / LOWPASS_FILTER_WIDTH / 2.0)
                .cos()
                .powi(2);
            let radians = time * std::f64::consts::PI;
            let sinc = if radians == 0.0 {
                1.0
            } else {
                radians.sin() / radians
            };
            let weight = (sinc * window * scale) as f32;
            // Values outside the six-zero-crossing window are numerical dust
            // from cos(pi/2), far below one f32 ULP in the convolution sum.
            if weight.abs() >= 1.0e-12 {
                sparse.push((kernel_index, weight));
            }
        }
        kernels.push(sparse);
    }

    let output_numerator = target
        .checked_mul(input.len())
        .and_then(|value| value.checked_add(original - 1))
        .context("resampled audio length overflowed")?;
    let output_length = output_numerator / original;
    let mut output = Vec::with_capacity(output_length);
    for output_index in 0..output_length {
        let block = output_index / target;
        let phase = output_index % target;
        let padded_start = block * original;
        let mut sum = 0.0_f32;
        for &(kernel_index, weight) in &kernels[phase] {
            let padded_index = padded_start + kernel_index;
            if padded_index < width {
                continue;
            }
            let source_index = padded_index - width;
            if source_index >= input.len() {
                continue;
            }
            sum += input[source_index] * weight;
        }
        output.push(sum);
    }
    Ok(output)
}

fn greatest_common_divisor(mut left: u32, mut right: u32) -> u32 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_file_decode_downmix_and_resample_match_torchaudio() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/neural-key/myna-stereo-44100-pcm16.wav");
        let expected_bytes =
            include_bytes!("../tests/fixtures/neural-key/myna-stereo-44100-to-16000-f32le.bin");
        let expected: Vec<f32> = expected_bytes
            .chunks_exact(4)
            .map(|bytes| f32::from_le_bytes(bytes.try_into().unwrap()))
            .collect();
        let actual = decode_myna_audio(fixture).unwrap();
        assert_eq!(actual.source_sample_rate_hz, 44_100);
        assert_eq!(actual.source_channels, 2);
        assert_eq!(actual.samples.len(), 6_400);
        assert_eq!(actual.samples.len(), expected.len());

        let mut maximum_absolute_error = 0.0_f32;
        let mut mean_absolute_error = 0.0_f64;
        let expected_count = expected.len() as f64;
        for (actual, expected_sample) in actual.samples.iter().zip(&expected) {
            let error = (actual - expected_sample).abs();
            maximum_absolute_error = maximum_absolute_error.max(error);
            mean_absolute_error += error as f64 / expected_count;
        }
        eprintln!(
            "torchaudio real-file parity max_abs={maximum_absolute_error} mean_abs={mean_absolute_error}"
        );
        assert!(
            maximum_absolute_error <= 2.0e-5 && mean_absolute_error <= 2.0e-6,
            "real-file parity failed: max_abs={maximum_absolute_error} mean_abs={mean_absolute_error}"
        );
    }

    #[test]
    fn resampler_rejects_invalid_inputs_and_preserves_equal_rates() {
        assert!(torchaudio_sinc_resample(&[], 44_100, 16_000).is_err());
        assert!(torchaudio_sinc_resample(&[f32::NAN], 44_100, 16_000).is_err());
        assert!(torchaudio_sinc_resample(&[0.0], 0, 16_000).is_err());
        assert_eq!(
            torchaudio_sinc_resample(&[0.25, -0.5], 16_000, 16_000).unwrap(),
            [0.25, -0.5]
        );
    }
}
