// Rust-side audio engine prototype for TuneLock Transition Workbench Slice A.
//
// Architecture: One cpal Stream owns the master clock. Two decks are mixed
// in Rust, each with gain, 3-band EQ, and crossfade. The audio graph is:
//
//   Deck A buffer → gain → low/mid/high EQ → crossfade gain ─┐
//                                                              ├→ master gain → cpal output
//   Deck B buffer → gain → low/mid/high EQ → crossfade gain ─┘
//
// Transport commands arrive via a crossbeam channel from the CLI.
// Pitch-preserving tempo is done by resampling (nearest-neighbor for the
// prototype; a phase vocoder would be needed for production quality).
//
// Usage: cargo run --release -- <file_a.wav> <file_b.wav>

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, StreamConfig};
use crossbeam_channel::{unbounded, Receiver};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

mod deck;
mod mixer;
mod transport;

use deck::Deck;
use mixer::Mixer;
use transport::{TransportCommand, TransportState};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: {} <file_a.wav> <file_b.wav>", args[0]);
        eprintln!("\nLoad two WAV files and test synchronized playback.");
        eprintln!("Commands: play, pause, stop, seek <seconds>, crossfade <0-1>, tempo_a <0.92-1.08>, tempo_b <0.92-1.08>, quit");
        std::process::exit(1);
    }

    let file_a = &args[1];
    let file_b = &args[2];

    println!("TuneLock Rust Audio Prototype (Slice A)");
    println!("=========================================");
    println!("Deck A: {}", file_a);
    println!("Deck B: {}", file_b);

    // Decode both files
    println!("\nDecoding files...");
    let buffer_a = decode_file(file_a);
    let buffer_b = decode_file(file_b);
    println!(
        "Deck A: {:.1}s, {}Hz, {}ch",
        buffer_a.duration,
        buffer_a.sample_rate,
        buffer_a.channels
    );
    println!(
        "Deck B: {:.1}s, {}Hz, {}ch",
        buffer_b.duration,
        buffer_b.sample_rate,
        buffer_b.channels
    );

    // Set up audio output
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .expect("No output device available");

    let supported_config = device.default_output_config().unwrap();
    let sample_format = supported_config.sample_format();
    let output_sample_rate = supported_config.sample_rate().0;

    println!("\nOutput device: {}", device.name().unwrap());
    println!("Output sample rate: {}Hz", output_sample_rate);
    println!("Sample format: {:?}", sample_format);

    // Create mixer with both decks
    let mixer = Arc::new(Mixer::new(buffer_a, buffer_b, output_sample_rate));

    // Transport command channel
    let (cmd_tx, cmd_rx) = unbounded::<TransportCommand>();

    // Transport state
    let transport = Arc::new(TransportState::new());

    // Build the audio stream
    let stream = build_stream(&device, &supported_config, mixer.clone(), cmd_rx, transport.clone());

    stream.play().expect("Failed to start stream");

    println!("\nAudio stream started. Enter commands:");
    println!("  play          - Start synchronized playback");
    println!("  pause         - Pause both decks");
    println!("  stop          - Stop and reset to beginning");
    println!("  seek <sec>    - Seek to position");
    println!("  crossfade <0-1> - Set crossfader (0=A, 1=B)");
    println!("  tempo_a <rate>  - Set deck A tempo (0.92-1.08)");
    println!("  tempo_b <rate>  - Set deck B tempo (0.92-1.08)");
    println!("  gain_a <0-1.5>  - Set deck A gain");
    println!("  gain_b <0-1.5>  - Set deck B gain");
    println!("  measure       - Run drift measurement (2 min)");
    println!("  quit          - Exit");

    // CLI loop
    let mut input = String::new();
    let running = Arc::new(AtomicBool::new(true));

    // Spawn drift measurement thread
    let mixer_for_measure = mixer.clone();
    let transport_for_measure = transport.clone();
    std::thread::spawn(move || {
        measure_drift(mixer_for_measure, transport_for_measure);
    });

    while running.load(Ordering::Relaxed) {
        input.clear();
        if std::io::stdin().read_line(&mut input).is_err() {
            break;
        }
        let input = input.trim();
        if input.is_empty() {
            continue;
        }

        let parts: Vec<&str> = input.split_whitespace().collect();
        let cmd = parts[0];

        match cmd {
            "quit" | "exit" | "q" => {
                running.store(false, Ordering::Relaxed);
                let _ = cmd_tx.send(TransportCommand::Stop);
            }
            "play" => {
                let start_time = Instant::now();
                let _ = cmd_tx.send(TransportCommand::Play);
                transport.set_start_time(start_time);
                println!("Play command sent.");
            }
            "pause" => {
                let _ = cmd_tx.send(TransportCommand::Pause);
                println!("Pause command sent.");
            }
            "stop" => {
                let _ = cmd_tx.send(TransportCommand::Stop);
                println!("Stop command sent.");
            }
            "seek" if parts.len() == 2 => {
                if let Ok(pos) = parts[1].parse::<f64>() {
                    let _ = cmd_tx.send(TransportCommand::Seek(pos));
                    println!("Seek to {:.1}s", pos);
                }
            }
            "crossfade" if parts.len() == 2 => {
                if let Ok(val) = parts[1].parse::<f32>() {
                    let _ = cmd_tx.send(TransportCommand::SetCrossfade(val));
                    println!("Crossfader: {:.2}", val);
                }
            }
            "tempo_a" if parts.len() == 2 => {
                if let Ok(rate) = parts[1].parse::<f32>() {
                    let _ = cmd_tx.send(TransportCommand::SetTempoA(rate));
                    println!("Deck A tempo: {:.3}", rate);
                }
            }
            "tempo_b" if parts.len() == 2 => {
                if let Ok(rate) = parts[1].parse::<f32>() {
                    let _ = cmd_tx.send(TransportCommand::SetTempoB(rate));
                    println!("Deck B tempo: {:.3}", rate);
                }
            }
            "gain_a" if parts.len() == 2 => {
                if let Ok(val) = parts[1].parse::<f32>() {
                    let _ = cmd_tx.send(TransportCommand::SetGainA(val));
                    println!("Deck A gain: {:.2}", val);
                }
            }
            "gain_b" if parts.len() == 2 => {
                if let Ok(val) = parts[1].parse::<f32>() {
                    let _ = cmd_tx.send(TransportCommand::SetGainB(val));
                    println!("Deck B gain: {:.2}", val);
                }
            }
            "measure" => {
                transport.start_drift_measurement();
                println!("Drift measurement started. Will report after 2 minutes.");
            }
            "status" => {
                let pos_a = mixer.deck_a.get_position();
                let pos_b = mixer.deck_b.get_position();
                println!(
                    "Deck A: {:.2}s | Deck B: {:.2}s | Playing: {}",
                    pos_a, pos_b, transport.is_playing()
                );
            }
            _ => println!("Unknown command: {}", cmd),
        }
    }

    println!("Shutting down...");
}

fn decode_file(path: &str) -> deck::AudioBuffer {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    let file = std::fs::File::open(path).expect("Failed to open file");
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
        .expect("Failed to probe format");

    let mut format = probed.format;

    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .expect("No audio track found");

    let track_id = track.id;
    let sample_rate = track.codec_params.sample_rate.unwrap_or(44100);
    let channels = track
        .codec_params
        .channels
        .map(|c| c.count())
        .unwrap_or(2);

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &decoder_opts)
        .expect("Failed to create decoder");

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

    let duration = samples.len() as f64 / (sample_rate as f64 * channels as f64);

    println!("  Decoded: {} samples, {:.1}s", samples.len() / channels, duration);

    deck::AudioBuffer {
        samples,
        sample_rate,
        channels,
        duration,
    }
}

fn build_stream(
    device: &cpal::Device,
    config: &cpal::SupportedStreamConfig,
    mixer: Arc<Mixer>,
    cmd_rx: Receiver<TransportCommand>,
    transport: Arc<TransportState>,
) -> cpal::Stream {
    let sample_format = config.sample_format();
    let conf: StreamConfig = config.config().into();

    let err_fn = |err| eprintln!("Stream error: {}", err);

    match sample_format {
        SampleFormat::F32 => {
            let mut cmd_rx = cmd_rx;
            device
                .build_output_stream(
                    &conf,
                    move |buffer: &mut [f32], _| {
                        while let Ok(cmd) = cmd_rx.try_recv() {
                            mixer.handle_command(&cmd);
                            transport.handle_command(&cmd);
                        }
                        mixer.process(buffer);
                    },
                    err_fn,
                    None,
                )
                .unwrap()
        }
        SampleFormat::I16 => {
            let mut cmd_rx = cmd_rx;
            device
                .build_output_stream(
                    &conf,
                    move |buffer: &mut [i16], _| {
                        while let Ok(cmd) = cmd_rx.try_recv() {
                            mixer.handle_command(&cmd);
                            transport.handle_command(&cmd);
                        }
                        let mut float_buf = vec![0f32; buffer.len()];
                        mixer.process(&mut float_buf);
                        for (i, s) in float_buf.iter().enumerate() {
                            buffer[i] = (s * i16::MAX as f32).clamp(-1.0, 1.0) as i16;
                        }
                    },
                    err_fn,
                    None,
                )
                .unwrap()
        }
        _ => panic!("Unsupported sample format: {:?}", sample_format),
    }
}

fn measure_drift(mixer: Arc<Mixer>, transport: Arc<TransportState>) {
    loop {
        if !transport.is_drift_measuring() {
            std::thread::sleep(std::time::Duration::from_millis(100));
            continue;
        }

        let start = Instant::now();
        let start_pos_a = mixer.deck_a.get_position();
        let start_pos_b = mixer.deck_b.get_position();

        std::thread::sleep(std::time::Duration::from_secs(120));

        let elapsed = start.elapsed().as_secs_f64();
        let end_pos_a = mixer.deck_a.get_position();
        let end_pos_b = mixer.deck_b.get_position();

        let expected_a = start_pos_a + elapsed * mixer.deck_a.get_playback_rate() as f64;
        let expected_b = start_pos_b + elapsed * mixer.deck_b.get_playback_rate() as f64;

        let drift_a = (end_pos_a - expected_a).abs() * 1000.0;
        let drift_b = (end_pos_b - expected_b).abs() * 1000.0;
        let max_drift = drift_a.max(drift_b);

        println!("\n=== Drift Measurement (2 minutes) ===");
        println!("Deck A drift: {:.2} ms", drift_a);
        println!("Deck B drift: {:.2} ms", drift_b);
        println!("Max drift:   {:.2} ms", max_drift);
        if max_drift <= 20.0 {
            println!("PASS: Drift within 20ms target");
        } else {
            println!("FAIL: Drift exceeds 20ms target");
        }
        println!("======================================\n");

        transport.stop_drift_measurement();
    }
}
