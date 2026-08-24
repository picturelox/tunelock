//! Real-file decode and resampling for the pinned Myna input contract.
//!
//! Unlike the classical analyzer's intentionally normalized 22.05 kHz path,
//! this path preserves decoded amplitude, averages channels in float32, and
//! mirrors torchaudio 2.7's default Hann-windowed sinc resampler at 16 kHz.

use std::fs::File;
use std::path::Path;

use anyhow::{bail, Context, Result};
use rustfft::num_complex::Complex32;
use rustfft::FftPlanner;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use crate::neural_key_preprocess::MYNA_SAMPLE_RATE_HZ;

const LOWPASS_FILTER_WIDTH: f64 = 6.0;
const ROLLOFF: f64 = 0.99;
const PITCH_N_FFT: usize = 512;
const PITCH_HOP_LENGTH: usize = 128;

#[derive(Debug)]
pub struct MynaAudio {
    pub samples: Vec<f32>,
    pub source_sample_rate_hz: u32,
    pub source_channels: usize,
}

/// Reusable shared STFT for the twelve pitch-preserving Myna views.
pub struct MynaPitchShifter {
    spectrum: Vec<Vec<Complex32>>,
    window: Vec<f32>,
    input_length: usize,
}

impl std::fmt::Debug for MynaPitchShifter {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MynaPitchShifter")
            .field("frames", &self.spectrum.len())
            .field("input_length", &self.input_length)
            .finish()
    }
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
        // The dense torchaudio kernel has `original` columns, which becomes
        // enormous when the rates are coprime. Only the roughly 2*width taps
        // inside the Hann support can be non-zero, so derive that interval
        // directly while preserving the same per-tap formula.
        let phase_offset = phase as f64 / target as f64;
        let first = (width as f64
            + (-LOWPASS_FILTER_WIDTH / base_frequency + phase_offset) * original as f64)
            .ceil()
            .max(0.0) as usize;
        let last = (width as f64
            + (LOWPASS_FILTER_WIDTH / base_frequency + phase_offset) * original as f64)
            .floor()
            .min((kernel_length - 1) as f64) as usize;
        for kernel_index in first..=last {
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

/// Generate pitch-preserving transposition views with the exact shape used by
/// the faithful Myna cache: a shared 512-point periodic-Hann STFT, torchaudio-
/// compatible phase vocoder, inverse STFT, and default sinc resampling. The
/// output order matches `semitones` and every view has the input length.
pub fn myna_pitch_shift_views(input: &[f32], semitones: &[i32]) -> Result<Vec<(i32, Vec<f32>)>> {
    if semitones.is_empty() {
        bail!("at least one pitch-shift view is required");
    }
    let mut seen = std::collections::HashSet::with_capacity(semitones.len());
    for shift in semitones {
        if *shift == 0 || !(-6..=6).contains(shift) || !seen.insert(*shift) {
            bail!("pitch shifts must be unique non-zero semitones in [-6, 6]");
        }
    }
    let shifter = MynaPitchShifter::new(input)?;
    semitones
        .iter()
        .map(|shift| Ok((*shift, shifter.shift(*shift)?)))
        .collect()
}

impl MynaPitchShifter {
    pub fn new(input: &[f32]) -> Result<Self> {
        if input.len() <= PITCH_N_FFT / 2 || input.iter().any(|sample| !sample.is_finite()) {
            bail!("pitch shifting requires more than 256 finite audio samples");
        }
        let window: Vec<f32> = (0..PITCH_N_FFT)
            .map(|index| {
                0.5 - 0.5 * (2.0 * std::f32::consts::PI * index as f32 / PITCH_N_FFT as f32).cos()
            })
            .collect();
        let spectrum = centered_stft(input, &window)?;
        Ok(Self {
            spectrum,
            window,
            input_length: input.len(),
        })
    }

    pub fn shift(&self, semitones: i32) -> Result<Vec<f32>> {
        if semitones == 0 || !(-6..=6).contains(&semitones) {
            bail!("pitch shift must be a non-zero semitone in [-6, 6]");
        }
        let rate = 2.0_f64.powf(-(semitones as f64) / 12.0);
        let stretched_spectrum = phase_vocoder(&self.spectrum, rate as f32);
        let stretched_length = (self.input_length as f64 / rate).round() as usize;
        let stretched = centered_istft(&stretched_spectrum, &self.window, stretched_length)?;
        let original_rate = (MYNA_SAMPLE_RATE_HZ as f64 / rate) as u32;
        let mut shifted =
            torchaudio_sinc_resample(&stretched, original_rate, MYNA_SAMPLE_RATE_HZ as u32)?;
        shifted.resize(self.input_length, 0.0);
        shifted.truncate(self.input_length);
        Ok(shifted)
    }
}

fn centered_stft(input: &[f32], window: &[f32]) -> Result<Vec<Vec<Complex32>>> {
    let padding = PITCH_N_FFT / 2;
    let padded_length = input
        .len()
        .checked_add(2 * padding)
        .context("pitch STFT input length overflowed")?;
    let mut padded = vec![0.0_f32; padded_length];
    for (index, value) in padded.iter_mut().enumerate() {
        let source = reflect_index(index as isize - padding as isize, input.len());
        *value = input[source];
    }
    let frame_count = 1 + (padded_length - PITCH_N_FFT) / PITCH_HOP_LENGTH;
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(PITCH_N_FFT);
    let mut spectrum = Vec::with_capacity(frame_count);
    for frame_index in 0..frame_count {
        let offset = frame_index * PITCH_HOP_LENGTH;
        let mut frame: Vec<Complex32> = padded[offset..offset + PITCH_N_FFT]
            .iter()
            .zip(window)
            .map(|(sample, weight)| Complex32::new(sample * weight, 0.0))
            .collect();
        fft.process(&mut frame);
        frame.truncate(PITCH_N_FFT / 2 + 1);
        spectrum.push(frame);
    }
    Ok(spectrum)
}

fn phase_vocoder(spectrum: &[Vec<Complex32>], rate: f32) -> Vec<Vec<Complex32>> {
    let frame_count = spectrum.len();
    let output_frames = (frame_count as f32 / rate).ceil() as usize;
    let bin_count = PITCH_N_FFT / 2 + 1;
    let mut output = vec![vec![Complex32::new(0.0, 0.0); bin_count]; output_frames];
    let mut accumulated_phase: Vec<f32> = spectrum[0].iter().map(|value| value.arg()).collect();
    let two_pi = 2.0 * std::f32::consts::PI;

    for output_index in 0..output_frames {
        let time = output_index as f32 * rate;
        let source_index = time.floor() as usize;
        let alpha = time - source_index as f32;
        for bin in 0..bin_count {
            let first = spectrum
                .get(source_index)
                .and_then(|frame| frame.get(bin))
                .copied()
                .unwrap_or_default();
            let second = spectrum
                .get(source_index + 1)
                .and_then(|frame| frame.get(bin))
                .copied()
                .unwrap_or_default();
            let magnitude = alpha * second.norm() + (1.0 - alpha) * first.norm();
            output[output_index][bin] = Complex32::from_polar(magnitude, accumulated_phase[bin]);

            let phase_advance = std::f32::consts::PI * PITCH_HOP_LENGTH as f32 * bin as f32
                / (bin_count - 1) as f32;
            let phase = second.arg() - first.arg() - phase_advance;
            let wrapped = phase - two_pi * round_ties_even(phase / two_pi);
            accumulated_phase[bin] += wrapped + phase_advance;
        }
    }
    output
}

fn centered_istft(
    spectrum: &[Vec<Complex32>],
    window: &[f32],
    output_length: usize,
) -> Result<Vec<f32>> {
    if spectrum.is_empty() {
        bail!("pitch inverse STFT received no frames");
    }
    let full_length = PITCH_N_FFT
        .checked_add((spectrum.len() - 1) * PITCH_HOP_LENGTH)
        .context("pitch inverse STFT length overflowed")?;
    let mut overlap = vec![0.0_f32; full_length];
    let mut envelope = vec![0.0_f32; full_length];
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_inverse(PITCH_N_FFT);
    for (frame_index, bins) in spectrum.iter().enumerate() {
        let mut frame = vec![Complex32::new(0.0, 0.0); PITCH_N_FFT];
        frame[..=PITCH_N_FFT / 2].copy_from_slice(bins);
        for bin in 1..PITCH_N_FFT / 2 {
            frame[PITCH_N_FFT - bin] = bins[bin].conj();
        }
        fft.process(&mut frame);
        let offset = frame_index * PITCH_HOP_LENGTH;
        for index in 0..PITCH_N_FFT {
            let weight = window[index];
            overlap[offset + index] += frame[index].re * weight / PITCH_N_FFT as f32;
            envelope[offset + index] += weight * weight;
        }
    }
    for (sample, weight) in overlap.iter_mut().zip(envelope) {
        if weight > 1.0e-11 {
            *sample /= weight;
        }
    }
    let start = PITCH_N_FFT / 2;
    let mut output = vec![0.0_f32; output_length];
    let available = overlap.len().saturating_sub(start).min(output_length);
    output[..available].copy_from_slice(&overlap[start..start + available]);
    Ok(output)
}

fn reflect_index(mut index: isize, length: usize) -> usize {
    let maximum = length as isize - 1;
    while index < 0 || index > maximum {
        index = if index < 0 {
            -index
        } else {
            2 * maximum - index
        };
    }
    index as usize
}

fn round_ties_even(value: f32) -> f32 {
    let lower = value.floor();
    let fraction = value - lower;
    if fraction < 0.5 {
        lower
    } else if fraction > 0.5 {
        lower + 1.0
    } else if lower as i64 % 2 == 0 {
        lower
    } else {
        lower + 1.0
    }
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

    #[test]
    fn pitch_views_match_pinned_torchaudio_phase_vocoder() {
        let source_bytes =
            include_bytes!("../tests/fixtures/neural-key/myna-stereo-44100-to-16000-f32le.bin");
        let expected_bytes =
            include_bytes!("../tests/fixtures/neural-key/myna-pitch-views-f32le.bin");
        let source: Vec<f32> = source_bytes
            .chunks_exact(4)
            .map(|bytes| f32::from_le_bytes(bytes.try_into().unwrap()))
            .collect();
        let expected: Vec<f32> = expected_bytes
            .chunks_exact(4)
            .map(|bytes| f32::from_le_bytes(bytes.try_into().unwrap()))
            .collect();
        let shifts = [-6, -5, -4, -3, -2, -1, 1, 2, 3, 4, 5, 6];
        let views = myna_pitch_shift_views(&source, &shifts).unwrap();
        assert_eq!(views.len(), shifts.len());
        assert_eq!(expected.len(), source.len() * shifts.len());

        let mut global_maximum_error = 0.0_f32;
        let mut global_mean_error = 0.0_f64;
        for ((actual_shift, actual), (expected_shift, expected)) in views
            .iter()
            .zip(shifts.iter().zip(expected.chunks_exact(source.len())))
        {
            assert_eq!(actual_shift, expected_shift);
            let mut shift_maximum_error = 0.0_f32;
            let mut shift_mean_error = 0.0_f64;
            for (actual, expected) in actual.iter().zip(expected) {
                let error = (actual - expected).abs();
                shift_maximum_error = shift_maximum_error.max(error);
                global_maximum_error = global_maximum_error.max(error);
                shift_mean_error += error as f64 / source.len() as f64;
                global_mean_error += error as f64 / expected_bytes.len() as f64 * 4.0;
            }
            eprintln!(
                "pitch shift {actual_shift:+} parity max_abs={shift_maximum_error} mean_abs={shift_mean_error}"
            );
        }
        eprintln!(
            "pitch parity global max_abs={global_maximum_error} mean_abs={global_mean_error}"
        );
        assert!(
            global_maximum_error <= 1.0e-3 && global_mean_error <= 1.0e-4,
            "pitch parity failed: max_abs={global_maximum_error} mean_abs={global_mean_error}"
        );
    }

    #[test]
    fn pitch_views_reject_invalid_requests() {
        let samples = vec![0.0_f32; 512];
        assert!(myna_pitch_shift_views(&samples, &[]).is_err());
        assert!(myna_pitch_shift_views(&samples, &[0]).is_err());
        assert!(myna_pitch_shift_views(&samples, &[1, 1]).is_err());
        assert!(myna_pitch_shift_views(&samples, &[7]).is_err());
        assert!(myna_pitch_shift_views(&[0.0; 256], &[1]).is_err());
    }
}
