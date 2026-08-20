// Audio engine — the authoritative playback engine for the Transition Workbench.
//
// Architecture:
//   - One CPAL output stream (the real-time callback)
//   - One monotonically increasing frame counter (AtomicU64)
//   - Two Deck instances, each with EQ, gain, and crossfade
//   - A bounded lock-free command queue (UI → callback)
//   - An atomic meter snapshot (callback → UI)
//   - Worker threads for decode/resample (future: stretch)
//
// The callback OWNS its state (moved into the closure). All communication
// with the outside world is through:
//   - Command queue (lock-free SPSC, UI → callback)
//   - Meter snapshot (atomics, callback → UI)
//   - Frame counter (atomics, callback → UI)
//
// The callback NEVER allocates, locks, does I/O, or calls Tauri.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, StreamConfig};

use super::command::{CommandQueue, EngineCommand, DeckId, EqBand, DecodedBuffer};
use super::deck::Deck;
use super::meter::MeterSnapshot;

/// The audio engine. Owns the CPAL stream and shared communication channels.
/// The real-time state is owned by the callback closure, not by this struct.
pub struct AudioEngine {
    frame_counter: Arc<AtomicU64>,
    command_queue: Arc<CommandQueue>,
    meter_snapshot: Arc<MeterSnapshot>,
    sample_rate: u32,
    stream: Option<SendStream>,
}

/// Wrapper around cpal::Stream to make it Send + Sync.
/// This is sound because:
/// - The stream is created on one thread and stored in AppState
/// - The stream's play/pause methods are called from the Tauri async runtime
/// - The actual audio processing happens on the audio thread, not the owning thread
/// - No two threads access the stream handle simultaneously (Tauri's Mutex ensures this)
struct SendStream(cpal::Stream);
unsafe impl Send for SendStream {}
unsafe impl Sync for SendStream {}

/// Internal state owned by the callback closure. Not shared — all
/// communication is through lock-free channels.
struct CallbackState {
    frame_counter: Arc<AtomicU64>,
    command_queue: Arc<CommandQueue>,
    meter_snapshot: Arc<MeterSnapshot>,
    decks: [Deck; 2],
    master_gain: f64,
    // Crossfader state (ramped)
    crossfade_position: f64,
    crossfade_target: f64,
    crossfade_ramp_increment: f64,
    // Master metering for this block
    master_sum_sq: [f64; 2],
    master_peak: [f64; 2],
    master_true_peak: [f64; 2],
    master_clip: bool,
    sample_rate: f64,
    // Meter update counter (update snapshot every N samples)
    meter_update_counter: u64,
    meter_update_interval: u64,
}

impl CallbackState {
    fn new(
        frame_counter: Arc<AtomicU64>,
        command_queue: Arc<CommandQueue>,
        meter_snapshot: Arc<MeterSnapshot>,
        sample_rate: f64,
    ) -> Self {
        let meter_update_interval = (sample_rate / 30.0) as u64; // 30 Hz meter updates
        Self {
            frame_counter,
            command_queue,
            meter_snapshot,
            decks: [
                Deck::new(DeckId::A, sample_rate),
                Deck::new(DeckId::B, sample_rate),
            ],
            master_gain: 0.8, // ~-2 dB headroom
            crossfade_position: 0.5,
            crossfade_target: 0.5,
            crossfade_ramp_increment: 1.0 / (0.005 * sample_rate),
            master_sum_sq: [0.0; 2],
            master_peak: [0.0; 2],
            master_true_peak: [0.0; 2],
            master_clip: false,
            sample_rate,
            meter_update_counter: 0,
            meter_update_interval,
        }
    }

    #[inline]
    fn deck_idx(&self, id: DeckId) -> usize {
        match id {
            DeckId::A => 0,
            DeckId::B => 1,
        }
    }

    fn process_commands(&mut self) {
        while let Some(cmd) = self.command_queue.pop() {
            match cmd {
                EngineCommand::Play { at_frame: _ } => {
                    self.decks[0].play();
                    self.decks[1].play();
                    self.meter_snapshot.playing.store(true, Ordering::Relaxed);
                }
                EngineCommand::Pause { at_frame: _ } => {
                    self.decks[0].pause();
                    self.decks[1].pause();
                    self.meter_snapshot.playing.store(false, Ordering::Relaxed);
                }
                EngineCommand::Stop { at_frame: _ } => {
                    self.decks[0].stop();
                    self.decks[1].stop();
                    self.meter_snapshot.playing.store(false, Ordering::Relaxed);
                }
                EngineCommand::Seek { at_frame: _, position_sec } => {
                    self.decks[0].seek(position_sec);
                    self.decks[1].seek(position_sec);
                }
                EngineCommand::SetTempoA { at_frame: _, rate } => {
                    self.decks[0].set_tempo(rate);
                }
                EngineCommand::SetTempoB { at_frame: _, rate } => {
                    self.decks[1].set_tempo(rate);
                }
                EngineCommand::SetCrossfade { at_frame: _, position } => {
                    self.crossfade_target = position as f64;
                }
                EngineCommand::SetDeckGain { at_frame: _, deck, gain } => {
                    let idx = self.deck_idx(deck);
                    self.decks[idx].set_gain(gain);
                }
                EngineCommand::SetEqGain { at_frame: _, deck, band, gain_db } => {
                    let idx = self.deck_idx(deck);
                    self.decks[idx].set_eq_gain(band, gain_db);
                }
                EngineCommand::SetEqKill { at_frame: _, deck, band, killed } => {
                    let idx = self.deck_idx(deck);
                    self.decks[idx].set_eq_kill(band, killed);
                }
                EngineCommand::SetLoop { at_frame: _, loop_region } => {
                    self.decks[0].set_loop(loop_region);
                    self.decks[1].set_loop(loop_region);
                }
                EngineCommand::LoadDeck { deck, buffer } => {
                    let idx = self.deck_idx(deck);
                    self.decks[idx].load_buffer(buffer);
                }
                EngineCommand::Shutdown => {
                    self.decks[0].stop();
                    self.decks[1].stop();
                }
            }
        }
    }

    #[inline]
    fn ramp_crossfade(&mut self) {
        if (self.crossfade_position - self.crossfade_target).abs() <= self.crossfade_ramp_increment {
            self.crossfade_position = self.crossfade_target;
        } else if self.crossfade_position < self.crossfade_target {
            self.crossfade_position += self.crossfade_ramp_increment;
        } else {
            self.crossfade_position -= self.crossfade_ramp_increment;
        }

        // Equal-power crossfade
        let angle = self.crossfade_position * std::f64::consts::PI / 2.0;
        let gain_a = angle.cos();
        let gain_b = angle.sin();
        self.decks[0].set_crossfade_gain(gain_a as f32);
        self.decks[1].set_crossfade_gain(gain_b as f32);
    }

    fn reset_block_meters(&mut self) {
        self.master_sum_sq = [0.0; 2];
        self.master_peak = [0.0; 2];
        self.master_true_peak = [0.0; 2];
        self.decks[0].reset_block_meters();
        self.decks[1].reset_block_meters();
    }

    fn update_meters(&mut self, sample_count: usize) {
        let (a_rms, a_peak, a_clip) = self.decks[0].get_block_meters(sample_count);
        let (b_rms, b_peak, b_clip) = self.decks[1].get_block_meters(sample_count);

        let m_rms = if sample_count > 0 {
            ((self.master_sum_sq[0] + self.master_sum_sq[1]) / (2.0 * sample_count as f64)).sqrt()
        } else {
            0.0
        };
        let m_peak = self.master_peak[0].max(self.master_peak[1]);
        let m_true_peak = self.master_true_peak[0].max(self.master_true_peak[1]);

        self.meter_snapshot.write_meters(
            a_rms, a_peak, a_clip,
            b_rms, b_peak, b_clip,
            m_rms, m_peak, m_true_peak, self.master_clip,
        );
        self.meter_snapshot.write_position(
            self.decks[0].get_position_sec(),
            self.decks[1].get_position_sec(),
        );
    }
}

// Preallocated scratch buffer for integer output conversion.
// This is a thread-local to avoid allocation in the callback.
thread_local! {
    static SCRATCH_F32: std::cell::RefCell<Vec<f32>> = std::cell::RefCell::new(vec![0.0f32; 4096]);
}

impl AudioEngine {
    /// Create a new audio engine. Does not start playback.
    pub fn new() -> Result<Self, String> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or("No audio output device available")?;

        let supported_config = device
            .default_output_config()
            .map_err(|e| format!("Failed to get output config: {}", e))?;

        let sample_format = supported_config.sample_format();
        let sample_rate = supported_config.sample_rate().0;

        let frame_counter = Arc::new(AtomicU64::new(0));
        let command_queue = Arc::new(CommandQueue::new(256));
        let meter_snapshot = Arc::new(MeterSnapshot::new());

        let callback_state = CallbackState::new(
            frame_counter.clone(),
            command_queue.clone(),
            meter_snapshot.clone(),
            sample_rate as f64,
        );

        let config: StreamConfig = supported_config.config();
        let err_fn = |err| eprintln!("Audio stream error: {}", err);

        let stream = match sample_format {
            SampleFormat::F32 => {
                let mut state = callback_state;
                device
                    .build_output_stream(
                        &config,
                        move |buffer: &mut [f32], _| {
                            audio_callback_f32(&mut state, buffer);
                        },
                        err_fn,
                        None,
                    )
                    .map_err(|e| format!("Failed to build stream: {}", e))?
            }
            SampleFormat::I16 => {
                let mut state = callback_state;
                device
                    .build_output_stream(
                        &config,
                        move |buffer: &mut [i16], _| {
                            // Use thread-local scratch buffer — no allocation
                            SCRATCH_F32.with(|scratch| {
                                let mut scratch = scratch.borrow_mut();
                                let total = buffer.len();
                                let chunk_size = scratch.len().min(total);

                                let mut offset = 0;
                                while offset < total {
                                    let n = chunk_size.min(total - offset);
                                    let scratch_slice = &mut scratch[..n];
                                    audio_callback_f32(&mut state, scratch_slice);
                                    for i in 0..n {
                                        buffer[offset + i] = (scratch_slice[i] * i16::MAX as f32)
                                            .clamp(-32768.0, 32767.0) as i16;
                                    }
                                    offset += n;
                                }
                            });
                        },
                        err_fn,
                        None,
                    )
                    .map_err(|e| format!("Failed to build stream: {}", e))?
            }
            _ => return Err(format!("Unsupported sample format: {:?}", sample_format)),
        };

        Ok(Self {
            frame_counter,
            command_queue,
            meter_snapshot,
            sample_rate,
            stream: Some(SendStream(stream)),
        })
    }

    /// Start the audio stream (begins processing but decks are silent until Play).
    pub fn start(&self) -> Result<(), String> {
        if let Some(stream) = &self.stream {
            stream.0.play().map_err(|e| format!("Failed to start stream: {}", e))?;
        }
        Ok(())
    }

    /// Send a command to the engine.
    pub fn send_command(&self, cmd: EngineCommand) -> bool {
        self.command_queue.push(cmd)
    }

    /// Get the current meter snapshot.
    pub fn get_meters(&self) -> super::meter::MeterReadout {
        self.meter_snapshot.read_all()
    }

    /// Get the current output frame.
    pub fn current_frame(&self) -> u64 {
        self.frame_counter.load(Ordering::Relaxed)
    }

    /// Get the output sample rate.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Get the command queue for direct access.
    pub fn command_queue(&self) -> &Arc<CommandQueue> {
        &self.command_queue
    }

    /// Get the meter snapshot for direct access.
    pub fn meter_snapshot(&self) -> &Arc<MeterSnapshot> {
        &self.meter_snapshot
    }
}

/// The real-time audio callback for f32 output.
/// This function MUST NOT allocate, lock, do I/O, or call Tauri.
fn audio_callback_f32(state: &mut CallbackState, output: &mut [f32]) {
    // Process pending commands (lock-free queue)
    state.process_commands();

    // Reset block meters
    state.reset_block_meters();

    let channels = if output.len() >= 2 { 2 } else { 1 };
    let frames = output.len() / channels;
    let mut frame_in_block = 0u64;

    for frame in output.chunks_mut(channels) {
        // Ramp crossfader
        state.ramp_crossfade();

        // Process both decks
        let (a_l, a_r) = state.decks[0].process_sample();
        let (b_l, b_r) = state.decks[1].process_sample();

        // Sum decks
        let mut mix_l = a_l + b_l;
        let mut mix_r = a_r + b_r;

        // Master gain
        mix_l *= state.master_gain;
        mix_r *= state.master_gain;

        // Soft clip (safety limiter)
        mix_l = soft_clip(mix_l);
        mix_r = soft_clip(mix_r);

        // Update master meters
        state.master_sum_sq[0] += mix_l * mix_l;
        state.master_sum_sq[1] += mix_r * mix_r;
        let abs_l = mix_l.abs();
        let abs_r = mix_r.abs();
        if abs_l > state.master_peak[0] { state.master_peak[0] = abs_l; }
        if abs_r > state.master_peak[1] { state.master_peak[1] = abs_r; }
        // True peak approximation (would need 4x oversampling for true BS.1770)
        if abs_l > state.master_true_peak[0] { state.master_true_peak[0] = abs_l; }
        if abs_r > state.master_true_peak[1] { state.master_true_peak[1] = abs_r; }
        if abs_l >= 1.0 || abs_r >= 1.0 {
            state.master_clip = true;
        }

        // Write output
        frame[0] = mix_l as f32;
        if channels >= 2 {
            frame[1] = mix_r as f32;
        }

        frame_in_block += 1;
    }

    // Update frame counter
    state.frame_counter.fetch_add(frame_in_block, Ordering::Relaxed);
    state.meter_snapshot.current_frame.store(
        state.frame_counter.load(Ordering::Relaxed),
        Ordering::Relaxed,
    );

    // Update meter snapshot at ~30 Hz
    state.meter_update_counter += frame_in_block;
    if state.meter_update_counter >= state.meter_update_interval {
        state.meter_update_counter = 0;
        state.update_meters(frames);
    }
}

/// Soft clipping (tanh approximation) for safety limiting.
/// x / (1 + |x|) — smooth saturation, asymptotes at ±1.
#[inline]
fn soft_clip(x: f64) -> f64 {
    x / (1.0 + x.abs())
}
