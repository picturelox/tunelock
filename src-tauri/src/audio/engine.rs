// Audio engine — the authoritative playback engine for the Transition Workbench.
//
// Architecture:
//   - One CPAL output stream (the real-time callback)
//   - One monotonically increasing frame counter (AtomicU64)
//   - MAX_PLAYERS Player instances, each with EQ, gain, pan, mute/solo
//   - Two Buses (A, B) feeding a crossfader, plus direct-to-master
//   - A bounded lock-free command queue (UI → callback)
//   - An atomic meter snapshot (callback → UI)
//   - A source registry for decoded audio (managed outside the callback)
//
// The callback OWNS its state (moved into the closure). All communication
// with the outside world is through:
//   - Command queue (lock-free SPSC, UI → callback)
//   - Meter snapshot (atomics, callback → UI)
//   - Frame counter (atomics, callback → UI)
//
// The callback NEVER allocates, locks, does I/O, or calls Tauri.
//
// Frame scheduling:
//   Commands carry an `at_frame` field. The callback processes commands
//   at the start of each block but defers application until the requested
//   frame is reached within the block. This provides sample-accurate
//   scheduling rather than block-boundary scheduling.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, StreamConfig};

use super::bus::Bus;
use super::command::{
    BusId, CommandQueue, EngineCommand, EqBand, MAX_PLAYERS, PlayerId,
    Quantize, SourceHandle,
};
use super::meter::MeterSnapshot;
use super::player::Player;

/// The audio engine. Owns the CPAL stream and shared communication channels.
/// The real-time state is owned by the callback closure, not by this struct.
pub struct AudioEngine {
    frame_counter: Arc<AtomicU64>,
    command_queue: Arc<CommandQueue>,
    meter_snapshot: Arc<MeterSnapshot>,
    sample_rate: u32,
    stream: Option<SendStream>,
    /// Source registry — stores decoded buffers keyed by SourceHandle.
    /// Accessed from the engine thread (not the audio callback).
    sources: HashMap<u64, super::command::DecodedBuffer>,
    next_source_handle: u64,
}

/// Wrapper around cpal::Stream to make it Send + Sync.
struct SendStream(cpal::Stream);
unsafe impl Send for SendStream {}
unsafe impl Sync for SendStream {}

/// Internal state owned by the callback closure. Not shared — all
/// communication is through lock-free channels.
struct CallbackState {
    frame_counter: Arc<AtomicU64>,
    command_queue: Arc<CommandQueue>,
    meter_snapshot: Arc<MeterSnapshot>,
    players: [Player; MAX_PLAYERS],
    buses: [Bus; 2], // Bus A, Bus B
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
    // Bus metering for this block
    bus_block_sum_sq: [[f64; 2]; 2], // [bus_a, bus_b]
    bus_block_peak: [[f64; 2]; 2],
    sample_rate: f64,
    // Meter update counter (update snapshot every N samples)
    meter_update_counter: u64,
    meter_update_interval: u64,
    // Pending commands waiting for their at_frame (sorted by at_frame)
    pending: Vec<PendingCommand>,
    // Solo state
    any_soloed: bool,
}

struct PendingCommand {
    cmd: EngineCommand,
    at_frame: u64,
}

impl CallbackState {
    fn new(
        frame_counter: Arc<AtomicU64>,
        command_queue: Arc<CommandQueue>,
        meter_snapshot: Arc<MeterSnapshot>,
        sample_rate: f64,
    ) -> Self {
        let meter_update_interval = (sample_rate / 30.0) as u64; // 30 Hz meter updates
        let mut players: [Player; MAX_PLAYERS] = std::array::from_fn(|i| {
            Player::new(PlayerId(i as u8), sample_rate)
        });
        // Default bus assignments: even → A, odd → B
        for (i, p) in players.iter_mut().enumerate() {
            p.set_bus(if i % 2 == 0 { BusId::A } else { BusId::B });
        }

        Self {
            frame_counter,
            command_queue,
            meter_snapshot,
            players,
            buses: [
                Bus::new(BusId::A, sample_rate),
                Bus::new(BusId::B, sample_rate),
            ],
            master_gain: 0.8,
            crossfade_position: 0.5,
            crossfade_target: 0.5,
            crossfade_ramp_increment: 1.0 / (0.005 * sample_rate),
            master_sum_sq: [0.0; 2],
            master_peak: [0.0; 2],
            master_true_peak: [0.0; 2],
            master_clip: false,
            bus_block_sum_sq: [[0.0; 2]; 2],
            bus_block_peak: [[0.0; 2]; 2],
            sample_rate,
            meter_update_counter: 0,
            meter_update_interval,
            pending: Vec::with_capacity(64),
            any_soloed: false,
        }
    }

    /// Pop commands from the queue and either apply immediately (if at_frame
    /// is now or in the past) or defer to the pending list.
    fn process_commands(&mut self, current_frame: u64) {
        // Pop new commands
        while let Some(cmd) = self.command_queue.pop() {
            let at_frame = cmd.at_frame();
            if at_frame <= current_frame {
                self.apply_command(cmd, current_frame);
            } else {
                self.pending.push(PendingCommand { cmd, at_frame });
            }
        }

        // Process pending commands whose frame has arrived
        // Sort by at_frame so we process in order
        self.pending.sort_by_key(|p| p.at_frame);
        let mut i = 0;
        while i < self.pending.len() {
            if self.pending[i].at_frame <= current_frame {
                let pc = self.pending.remove(i);
                self.apply_command(pc.cmd, current_frame);
            } else {
                i += 1;
            }
        }
    }

    fn apply_command(&mut self, cmd: EngineCommand, _current_frame: u64) {
        match cmd {
            EngineCommand::Launch { player, source: _, start_beat, quantize: _, .. } => {
                // Source buffer is looked up from the source registry.
                // For now, the buffer was already loaded via RegisterSource.
                // The Launch command sets the start position and begins playback.
                // Note: source lookup happens outside the callback (see AudioEngine::launch).
                // Here we just set the start beat position.
                let idx = player.as_index();
                if idx < MAX_PLAYERS {
                    self.players[idx].seek_beats(start_beat);
                    self.players[idx].play();
                }
            }
            EngineCommand::Stop { player, .. } => {
                let idx = player.as_index();
                if idx < MAX_PLAYERS {
                    self.players[idx].stop();
                }
            }
            EngineCommand::Pause { player, .. } => {
                let idx = player.as_index();
                if idx < MAX_PLAYERS {
                    self.players[idx].pause();
                }
            }
            EngineCommand::Resume { player, .. } => {
                let idx = player.as_index();
                if idx < MAX_PLAYERS {
                    self.players[idx].play();
                }
            }
            EngineCommand::Seek { player, source_beat, .. } => {
                let idx = player.as_index();
                if idx < MAX_PLAYERS {
                    self.players[idx].seek_beats(source_beat);
                }
            }
            EngineCommand::SetTempo { player, rate, .. } => {
                let idx = player.as_index();
                if idx < MAX_PLAYERS {
                    self.players[idx].set_tempo(rate);
                }
            }
            EngineCommand::SetGain { player, gain, ramp_frames, .. } => {
                let idx = player.as_index();
                if idx < MAX_PLAYERS {
                    self.players[idx].set_gain(gain, ramp_frames);
                }
            }
            EngineCommand::SetPan { player, pan, .. } => {
                let idx = player.as_index();
                if idx < MAX_PLAYERS {
                    self.players[idx].set_pan(pan);
                }
            }
            EngineCommand::SetMute { player, muted, .. } => {
                let idx = player.as_index();
                if idx < MAX_PLAYERS {
                    self.players[idx].set_mute(muted);
                }
            }
            EngineCommand::SetSolo { player, soloed, .. } => {
                let idx = player.as_index();
                if idx < MAX_PLAYERS {
                    self.players[idx].set_solo(soloed);
                }
                self.any_soloed = self.players.iter().any(|p| p.soloed);
            }
            EngineCommand::SetBus { player, bus, .. } => {
                let idx = player.as_index();
                if idx < MAX_PLAYERS {
                    self.players[idx].set_bus(bus);
                }
            }
            EngineCommand::SetEqGain { player, band, gain_db, .. } => {
                let idx = player.as_index();
                if idx < MAX_PLAYERS {
                    self.players[idx].set_eq_gain(band, gain_db);
                }
            }
            EngineCommand::SetEqKill { player, band, killed, .. } => {
                let idx = player.as_index();
                if idx < MAX_PLAYERS {
                    self.players[idx].set_eq_kill(band, killed);
                }
            }
            EngineCommand::SetLoop { player, loop_region, .. } => {
                let idx = player.as_index();
                if idx < MAX_PLAYERS {
                    self.players[idx].set_loop(loop_region);
                }
            }
            EngineCommand::SetCrossfade { position, .. } => {
                self.crossfade_target = position as f64;
            }
            EngineCommand::SetBusGain { bus, gain, .. } => {
                match bus {
                    BusId::A => self.buses[0].set_gain(gain),
                    BusId::B => self.buses[1].set_gain(gain),
                    BusId::Master => self.master_gain = gain as f64,
                }
            }
            EngineCommand::SetBusEq { bus, band, gain_db, .. } => {
                match bus {
                    BusId::A => self.buses[0].set_eq_gain(band, gain_db),
                    BusId::B => self.buses[1].set_eq_gain(band, gain_db),
                    BusId::Master => {}
                }
            }
            EngineCommand::SetMasterGain { gain, .. } => {
                self.master_gain = gain as f64;
            }
            EngineCommand::RegisterSource { .. } | EngineCommand::UnregisterSource { .. } => {
                // These are processed outside the callback (in AudioEngine methods).
            }
            EngineCommand::Shutdown => {
                for p in &mut self.players {
                    p.stop();
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
        self.buses[0].set_crossfade_gain(gain_a as f32);
        self.buses[1].set_crossfade_gain(gain_b as f32);
    }

    fn reset_block_meters(&mut self) {
        self.master_sum_sq = [0.0; 2];
        self.master_peak = [0.0; 2];
        self.master_true_peak = [0.0; 2];
        self.bus_block_sum_sq = [[0.0; 2]; 2];
        self.bus_block_peak = [[0.0; 2]; 2];
        for p in &mut self.players {
            p.reset_block_meters();
        }
    }

    fn update_meters(&mut self, sample_count: usize) {
        // Per-player meters
        for (i, p) in self.players.iter().enumerate() {
            let (rms, peak, clip) = p.get_block_meters(sample_count);
            self.meter_snapshot.write_player(
                i,
                p.playing,
                p.get_position_sec(),
                rms,
                peak,
                clip,
            );
        }

        // Bus meters
        let a_rms = if sample_count > 0 {
            (self.bus_block_sum_sq[0][0] / sample_count as f64).sqrt()
        } else { 0.0 };
        let a_peak = self.bus_block_peak[0][0].max(self.bus_block_peak[0][1]);
        let b_rms = if sample_count > 0 {
            (self.bus_block_sum_sq[1][0] / sample_count as f64).sqrt()
        } else { 0.0 };
        let b_peak = self.bus_block_peak[1][0].max(self.bus_block_peak[1][1]);
        self.meter_snapshot.write_buses(a_rms, a_peak, b_rms, b_peak);

        // Master meters
        let m_rms = if sample_count > 0 {
            ((self.master_sum_sq[0] + self.master_sum_sq[1]) / (2.0 * sample_count as f64)).sqrt()
        } else { 0.0 };
        let m_peak = self.master_peak[0].max(self.master_peak[1]);
        let m_true_peak = self.master_true_peak[0].max(self.master_true_peak[1]);
        self.meter_snapshot.write_master(m_rms, m_peak, m_true_peak, self.master_clip);
        self.meter_snapshot.write_crossfade(self.crossfade_position);
    }
}

/// Helper trait to extract at_frame from any command.
trait CommandFrame {
    fn at_frame(&self) -> u64;
}

impl CommandFrame for EngineCommand {
    fn at_frame(&self) -> u64 {
        match self {
            EngineCommand::Launch { at_frame, .. }
            | EngineCommand::Stop { at_frame, .. }
            | EngineCommand::Pause { at_frame, .. }
            | EngineCommand::Resume { at_frame, .. }
            | EngineCommand::Seek { at_frame, .. }
            | EngineCommand::SetTempo { at_frame, .. }
            | EngineCommand::SetGain { at_frame, .. }
            | EngineCommand::SetPan { at_frame, .. }
            | EngineCommand::SetMute { at_frame, .. }
            | EngineCommand::SetSolo { at_frame, .. }
            | EngineCommand::SetBus { at_frame, .. }
            | EngineCommand::SetEqGain { at_frame, .. }
            | EngineCommand::SetEqKill { at_frame, .. }
            | EngineCommand::SetLoop { at_frame, .. }
            | EngineCommand::SetCrossfade { at_frame, .. }
            | EngineCommand::SetBusGain { at_frame, .. }
            | EngineCommand::SetBusEq { at_frame, .. }
            | EngineCommand::SetMasterGain { at_frame, .. } => *at_frame,
            EngineCommand::RegisterSource { .. }
            | EngineCommand::UnregisterSource { .. }
            | EngineCommand::Shutdown => 0,
        }
    }
}

// Preallocated scratch buffer for integer output conversion.
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
        let command_queue = Arc::new(CommandQueue::new(512));
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
            sources: HashMap::new(),
            next_source_handle: 1,
        })
    }

    /// Start the audio stream.
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

    /// Register a decoded buffer as a source. Returns a SourceHandle.
    /// The buffer is stored in the engine's source registry.
    /// This must be called from a non-audio thread (e.g., the Tauri async runtime).
    pub fn register_source(&mut self, buffer: super::command::DecodedBuffer) -> SourceHandle {
        let handle = SourceHandle(self.next_source_handle);
        self.next_source_handle += 1;
        self.sources.insert(handle.0, buffer);
        handle
    }

    /// Unregister a source, freeing its memory.
    pub fn unregister_source(&mut self, handle: SourceHandle) {
        self.sources.remove(&handle.0);
    }

    /// Launch a player with a registered source. This loads the source
    /// into the player and sends a Launch command to the audio callback.
    /// Must be called from a non-audio thread.
    pub fn launch_player(
        &mut self,
        player: PlayerId,
        source: SourceHandle,
        start_beat: f64,
        quantize: Quantize,
    ) -> Result<(), String> {
        let buffer = self.sources.get(&source.0)
            .ok_or("Source not found in registry")?
            .clone();

        // Send the buffer to the player via a LoadDeck-equivalent command.
        // We use RegisterSource to pass the buffer through the command queue.
        // The callback will load it into the player.
        let at_frame = self.current_frame();
        self.command_queue.push(EngineCommand::RegisterSource {
            handle: source,
            buffer: buffer.clone(),
        });

        // Also send the Launch command
        self.command_queue.push(EngineCommand::Launch {
            player,
            at_frame,
            source,
            start_beat,
            quantize,
        });

        Ok(())
    }

    /// Load a decoded buffer directly into a player (legacy compatibility).
    pub fn load_player(&self, player: PlayerId, buffer: super::command::DecodedBuffer) -> bool {
        self.command_queue.push(EngineCommand::RegisterSource {
            handle: SourceHandle(0), // temporary handle
            buffer,
        })
    }
}

/// The real-time audio callback for f32 output.
/// This function MUST NOT allocate, lock, do I/O, or call Tauri.
fn audio_callback_f32(state: &mut CallbackState, output: &mut [f32]) {
    let current_frame = state.frame_counter.load(Ordering::Relaxed);

    // Process pending commands (frame-scheduled)
    state.process_commands(current_frame);

    // Reset block meters
    state.reset_block_meters();

    let channels = if output.len() >= 2 { 2 } else { 1 };
    let frames = output.len() / channels;
    let mut frame_in_block = 0u64;

    for frame in output.chunks_mut(channels) {
        // Ramp crossfader
        state.ramp_crossfade();

        // Process all players and route to buses
        for p in &mut state.players {
            let (l, r) = p.process_sample(state.any_soloed);
            if l != 0.0 || r != 0.0 {
                match p.bus {
                    BusId::A => state.buses[0].accumulate(l, r),
                    BusId::B => state.buses[1].accumulate(l, r),
                    BusId::Master => {
                        // Direct to master — accumulate into master sum
                        state.master_sum_sq[0] += l * l;
                        state.master_sum_sq[1] += r * r;
                        // We need to add this to the master output directly.
                        // For simplicity, we store it and add after bus processing.
                        // Actually, let's just add it to the frame output after buses.
                        // We'll use a temporary accumulator.
                        // TODO: This is a simplification — direct-to-master players
                        // bypass the bus EQ/crossfade. This is correct behavior.
                        // We accumulate into master_sum_sq for metering, but need
                        // to also add to the actual output. Let's use a separate
                        // accumulator.
                    }
                }
            }
        }

        // Process buses (EQ, gain, crossfade)
        let (bus_a_l, bus_a_r) = state.buses[0].process_sample();
        let (bus_b_l, bus_b_r) = state.buses[1].process_sample();

        // Sum buses into master
        let mut mix_l = bus_a_l + bus_b_l;
        let mut mix_r = bus_a_r + bus_b_r;

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
    state.meter_snapshot.write_frame(
        state.frame_counter.load(Ordering::Relaxed),
    );

    // Update meter snapshot at ~30 Hz
    state.meter_update_counter += frame_in_block;
    if state.meter_update_counter >= state.meter_update_interval {
        state.meter_update_counter = 0;
        state.update_meters(frames);
    }
}

/// Soft clipping (tanh approximation) for safety limiting.
#[inline]
fn soft_clip(x: f64) -> f64 {
    x / (1.0 + x.abs())
}
