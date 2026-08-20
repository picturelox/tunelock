// Test fixture generator for Transition Workbench Slice A.
//
// Generates:
// - Click tracks at 120, 128, and 140 BPM (4/4, 16 bars = 64 beats)
// - Sine tone at 440Hz for pitch verification
// - Sine tone at 880Hz for pitch verification
//
// These fixtures are used to measure:
// - Start accuracy (deck-to-deck error)
// - Sustained drift over 2 minutes
// - Loop alignment over repeated passes
// - Pitch-preserving tempo change quality
//
// Usage: cargo run --release --bin gen-fixtures -- <output-dir>

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;

const SAMPLE_RATE: u32 = 44100;
const CHANNELS: u16 = 2;
const BITS_PER_SAMPLE: u16 = 16;

fn main() {
    let out_dir = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("prototypes/fixtures"));

    std::fs::create_dir_all(&out_dir).expect("Failed to create output dir");

    // Generate click tracks
    for bpm in [120u32, 128, 140] {
        let bars = 16;
        let beats_per_bar = 4;
        let total_beats = bars * beats_per_bar;
        let beat_duration = 60.0 / bpm as f64; // seconds per beat
        let total_duration = total_beats as f64 * beat_duration;
        let total_samples = (total_duration * SAMPLE_RATE as f64) as usize;

        let mut samples = vec![0.0f32; total_samples];

        // Place a click (short sine burst) on each beat
        let click_duration = 0.020; // 20ms click
        let click_samples = (click_duration * SAMPLE_RATE as f64) as usize;
        let freq = 1000.0; // 1kHz click

        for beat in 0..total_beats {
            let beat_start = (beat as f64 * beat_duration * SAMPLE_RATE as f64) as usize;
            let is_downbeat = beat % beats_per_bar == 0;
            let amplitude = if is_downbeat { 0.9 } else { 0.6 };

            for i in 0..click_samples {
                let idx = beat_start + i;
                if idx >= total_samples {
                    break;
                }
                // Envelope: quick attack, exponential decay
                let t = i as f64 / SAMPLE_RATE as f64;
                let env = (-t * 100.0).exp(); // 100ms decay
                samples[idx] = amplitude as f32 * env as f32
                    * (2.0 * std::f32::consts::PI * freq as f32 * t as f32).sin();
            }
        }

        // Label the BPM in the filename
        let filename = format!("click_{}bpm.wav", bpm);
        let path = out_dir.join(&filename);
        write_wav(&path, &samples);
        println!("Generated: {} ({}s, {} samples)", filename, total_duration, total_samples);
    }

    // Generate sine tones for pitch verification
    for freq in [440.0f64, 880.0] {
        let duration = 10.0; // 10 seconds
        let total_samples = (duration * SAMPLE_RATE as f64) as usize;
        let mut samples = vec![0.0f32; total_samples];

        for i in 0..total_samples {
            let t = i as f64 / SAMPLE_RATE as f64;
            // Slight fade in/out to avoid clicks
            let fade = if i < 1000 {
                i as f32 / 1000.0
            } else if i > total_samples - 1000 {
                (total_samples - i) as f32 / 1000.0
            } else {
                1.0
            };
            samples[i] = 0.3 * fade
                * (2.0 * std::f32::consts::PI * freq as f32 * t as f32).sin();
        }

        let filename = format!("sine_{}hz.wav", freq as u32);
        let path = out_dir.join(&filename);
        write_wav(&path, &samples);
        println!("Generated: {} ({}s, {} samples)", filename, duration, total_samples);
    }

    // Generate a 2-minute click track at 128 BPM for drift measurement
    let bpm = 128u32;
    let duration = 120.0; // 2 minutes
    let beat_duration = 60.0 / bpm as f64;
    let total_beats = (duration / beat_duration) as usize;
    let total_samples = (duration * SAMPLE_RATE as f64) as usize;
    let mut samples = vec![0.0f32; total_samples];

    let click_duration = 0.020;
    let click_samples = (click_duration * SAMPLE_RATE as f64) as usize;
    let freq = 1000.0;

    for beat in 0..total_beats {
        let beat_start = (beat as f64 * beat_duration * SAMPLE_RATE as f64) as usize;
        let is_downbeat = beat % 4 == 0;
        let amplitude = if is_downbeat { 0.9 } else { 0.6 };

        for i in 0..click_samples {
            let idx = beat_start + i;
            if idx >= total_samples {
                break;
            }
            let t = i as f64 / SAMPLE_RATE as f64;
            let env = (-t * 100.0).exp();
            samples[idx] = amplitude as f32 * env as f32
                * (2.0 * std::f32::consts::PI * freq as f32 * t as f32).sin();
        }
    }

    let filename = "click_128bpm_2min.wav";
    let path = out_dir.join(filename);
    write_wav(&path, &samples);
    println!(
        "Generated: {} ({}s, {} samples, {} beats)",
        filename, duration, total_samples, total_beats
    );

    println!("\nAll fixtures generated in: {}", out_dir.display());
}

fn write_wav(path: &PathBuf, samples: &[f32]) {
    let file = File::create(path).expect("Failed to create WAV file");
    let mut writer = BufWriter::new(file);

    let num_samples = samples.len();
    let data_size = (num_samples * CHANNELS as usize * (BITS_PER_SAMPLE / 8) as usize) as u32;
    let byte_rate = SAMPLE_RATE * CHANNELS as u32 * BITS_PER_SAMPLE as u32 / 8;
    let block_align = CHANNELS as u16 * BITS_PER_SAMPLE / 8;

    // RIFF header
    writer.write_all(b"RIFF").unwrap();
    writer.write_all(&(36 + data_size).to_le_bytes()).unwrap();
    writer.write_all(b"WAVE").unwrap();

    // fmt chunk
    writer.write_all(b"fmt ").unwrap();
    writer.write_all(&16u32.to_le_bytes()).unwrap();
    writer.write_all(&1u16.to_le_bytes()).unwrap(); // PCM
    writer.write_all(&CHANNELS.to_le_bytes()).unwrap();
    writer.write_all(&SAMPLE_RATE.to_le_bytes()).unwrap();
    writer.write_all(&byte_rate.to_le_bytes()).unwrap();
    writer.write_all(&block_align.to_le_bytes()).unwrap();
    writer.write_all(&BITS_PER_SAMPLE.to_le_bytes()).unwrap();

    // data chunk
    writer.write_all(b"data").unwrap();
    writer.write_all(&data_size.to_le_bytes()).unwrap();

    // Write interleaved stereo (duplicate mono to both channels)
    for &sample in samples {
        let value = (sample * i16::MAX as f32) as i16;
        writer.write_all(&value.to_le_bytes()).unwrap(); // Left
        writer.write_all(&value.to_le_bytes()).unwrap(); // Right
    }

    writer.flush().unwrap();
}
