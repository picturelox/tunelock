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
// Frame scheduling (event-sliced rendering):
//   Commands carry an `at_frame` field. The callback drains the queue into a
//   sorted pending list, then renders the block in slices: render up to the
//   next pending event frame, apply the event, continue rendering. A command
//   scheduled for halfway through the block takes effect at that exact frame,
//   not at the block boundary.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, StreamConfig};

use super::bus::Bus;
use super::command::{
    BusId, CommandQueue, DecodedBuffer, EngineCommand, EqBand, MAX_PLAYERS, PlayerId,
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
    /// Source registry — engine thread owns Arc<DecodedBuffer> keyed by
    /// SourceHandle. Launch commands carry an Arc clone; the callback never
    /// touches this registry.
    sources: HashMap<u64, Arc<DecodedBuffer>>,
    next_source_handle: u64,
    /// Deferred-destruction queue: retired Arc<DecodedBuffer> from the
    /// callback are pushed here (lock-free). The engine thread drains and
    /// drops them outside the realtime path, so large Vec<f32> deallocation
    /// never happens inside the audio callback.
    retired_sources: Arc<crossbeam_queue::ArrayQueue<Arc<DecodedBuffer>>>,
    /// Last-resort overflow queue for when retired_sources and per-player
    /// overflow slots are all full. Drained alongside retired_sources.
    deferred_overflow: Arc<crossbeam_queue::ArrayQueue<Arc<DecodedBuffer>>>,
}

/// Wrapper around cpal::Stream to make it Send + Sync.
struct SendStream(cpal::Stream);
unsafe impl Send for SendStream {}
unsafe impl Sync for SendStream {}

/// Internal state owned by the callback closure. Not shared — all
/// communication is through lock-free channels.
pub struct CallbackState {
    #[cfg(test)]
    pub frame_counter: Arc<AtomicU64>,
    #[cfg(not(test))]
    frame_counter: Arc<AtomicU64>,
    #[cfg(test)]
    pub command_queue: Arc<CommandQueue>,
    #[cfg(not(test))]
    command_queue: Arc<CommandQueue>,
    meter_snapshot: Arc<MeterSnapshot>,
    players: [Player; MAX_PLAYERS],
    buses: [Bus; 2], // Bus A, Bus B
    /// Deferred-destruction queue for retired source buffers. The callback
    /// pushes old Arc<DecodedBuffer> here instead of dropping them directly,
    /// so large Vec<f32> deallocation never happens on the realtime thread.
    /// The engine thread drains this queue periodically.
    retired_sources: Arc<crossbeam_queue::ArrayQueue<Arc<DecodedBuffer>>>,
    /// Last-resort overflow queue: if the retirement queue AND all per-player
    /// overflow slots are full, un-storable Arcs go here. This is a separate
    /// lock-free queue shared with the engine thread for draining. No
    /// allocation or deallocation on the realtime thread.
    deferred_overflow: Arc<crossbeam_queue::ArrayQueue<Arc<DecodedBuffer>>>,
    master_gain: f64,
    // Crossfader state (ramped)
    crossfade_position: f64,
    crossfade_target: f64,
    crossfade_ramp_increment: f64,
    // Master metering for this block
    master_sum_sq: [f64; 2],
    master_peak: [f64; 2],
    // PROVISIONAL: this is sample-peak, NOT true-peak. No oversampling or
    // inter-sample reconstruction is performed. True-peak measurement arrives
    // with the PB-6 loudness/mastering phase. Renamed to make the provisional
    // status explicit so nobody builds UI around a measurement that is more
    // sophisticated than the implementation.
    master_sample_peak_provisional: [f64; 2],
    master_clip: bool,
    // Bus metering for this block
    bus_block_sum_sq: [[f64; 2]; 2], // [bus_a, bus_b]
    bus_block_peak: [[f64; 2]; 2],
    sample_rate: f64,
    // Meter update counter (update snapshot every N samples)
    meter_update_counter: u64,
    meter_update_interval: u64,
    // Pending commands waiting for their at_frame (kept sorted by at_frame).
    // Preallocated; if full, a future command is applied immediately
    // (fail-safe toward "happen now" rather than "never happen").
    pending: Vec<PendingCommand>,
    // Solo state
    any_soloed: bool,
}

struct PendingCommand {
    cmd: EngineCommand,
    at_frame: u64,
}

impl CallbackState {
    pub fn new(
        frame_counter: Arc<AtomicU64>,
        command_queue: Arc<CommandQueue>,
        meter_snapshot: Arc<MeterSnapshot>,
        retired_sources: Arc<crossbeam_queue::ArrayQueue<Arc<DecodedBuffer>>>,
        deferred_overflow: Arc<crossbeam_queue::ArrayQueue<Arc<DecodedBuffer>>>,
        sample_rate: f64,
    ) -> Self {
        Self::new_impl(
            frame_counter,
            command_queue,
            meter_snapshot,
            retired_sources,
            deferred_overflow,
            sample_rate,
            false, // use default (Signalsmith) processor
        )
    }

    /// Create CallbackState with VarispeedProcessor for engine tests.
    /// Engine tests need zero-latency, sample-exact processing to verify
    /// routing, scheduling, and transparency — not STFT behavior.
    #[cfg(test)]
    pub fn new_for_test(
        frame_counter: Arc<AtomicU64>,
        command_queue: Arc<CommandQueue>,
        meter_snapshot: Arc<MeterSnapshot>,
        retired_sources: Arc<crossbeam_queue::ArrayQueue<Arc<DecodedBuffer>>>,
        sample_rate: f64,
    ) -> Self {
        Self::new_impl(
            frame_counter,
            command_queue,
            meter_snapshot,
            retired_sources,
            Arc::new(crossbeam_queue::ArrayQueue::new(16)),
            sample_rate,
            true, // use varispeed (zero latency) processor
        )
    }

    fn new_impl(
        frame_counter: Arc<AtomicU64>,
        command_queue: Arc<CommandQueue>,
        meter_snapshot: Arc<MeterSnapshot>,
        retired_sources: Arc<crossbeam_queue::ArrayQueue<Arc<DecodedBuffer>>>,
        deferred_overflow: Arc<crossbeam_queue::ArrayQueue<Arc<DecodedBuffer>>>,
        sample_rate: f64,
        use_varispeed: bool,
    ) -> Self {
        let meter_update_interval = (sample_rate / 30.0) as u64; // 30 Hz meter updates
        let mut players: [Player; MAX_PLAYERS] = std::array::from_fn(|i| {
            if use_varispeed {
                Player::new_with_processor(
                    PlayerId(i as u8),
                    sample_rate,
                    retired_sources.clone(),
                    super::timepitch::varispeed_processor(),
                )
            } else {
                Player::new(PlayerId(i as u8), sample_rate, retired_sources.clone())
            }
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
            retired_sources,
            deferred_overflow,
            master_gain: 0.8,
            crossfade_position: 0.5,
            crossfade_target: 0.5,
            crossfade_ramp_increment: 1.0 / (0.005 * sample_rate),
            master_sum_sq: [0.0; 2],
            master_peak: [0.0; 2],
            master_sample_peak_provisional: [0.0; 2],
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

    /// Insert a pending command, keeping the list sorted by at_frame.
    /// If the preallocated capacity is exceeded, apply immediately instead
    /// of allocating (real-time safety).
    fn insert_pending(&mut self, pc: PendingCommand) {
        if self.pending.len() >= self.pending.capacity() {
            self.apply_command(pc.cmd, pc.at_frame);
            return;
        }
        let pos = self.pending.partition_point(|p| p.at_frame <= pc.at_frame);
        self.pending.insert(pos, pc);
    }

    /// Apply every pending command whose at_frame has arrived (<= frame).
    /// The pending list is sorted, so due commands form a prefix.
    fn apply_due_commands(&mut self, frame: u64) {
        while let Some(first) = self.pending.first() {
            if first.at_frame > frame {
                break;
            }
            let pc = self.pending.remove(0);
            self.apply_command(pc.cmd, frame);
        }
    }

    /// Frame strictly after `frame` at which the next pending command is
    /// scheduled, or None if no future commands remain.
    fn next_event_frame(&self, frame: u64) -> Option<u64> {
        self.pending.first().map(|p| p.at_frame).filter(|&f| f > frame)
    }

    // ── Test helpers (only compiled in test builds) ───────────────────
    #[cfg(test)]
    pub fn pending_capacity_for_test(&self) -> usize {
        self.pending.capacity()
    }

    #[cfg(test)]
    pub fn retired_sources_pop_for_test(&self) -> Option<Arc<DecodedBuffer>> {
        self.retired_sources.pop()
    }

    #[cfg(test)]
    pub fn player_position_sec(&self, index: usize) -> f64 {
        if index < self.players.len() {
            self.players[index].get_position_sec()
        } else {
            0.0
        }
    }

    fn apply_command(&mut self, cmd: EngineCommand, _current_frame: u64) {
        match cmd {
            EngineCommand::Launch { player, source, buffer, start_beat, quantize: _, .. } => {
                // The buffer travels with the command as an Arc clone from the
                // engine-thread source registry. Load it directly into the
                // player — no callback-side registry lookup, no buffer copy.
                let idx = player.as_index();
                if idx < MAX_PLAYERS {
                    let unstored = self.players[idx].launch(source, buffer, start_beat);
                    // Push any un-storable Arcs to the deferred_overflow queue.
                    // If the queue is full, the Arc drops here — but this
                    // requires 128 + 8*8 + 16 = 208 undrained sources, which
                    // is impossible with 30Hz meter-poll draining.
                    for arc in unstored.iter().flatten() {
                        let _ = self.deferred_overflow.push(arc.clone());
                    }
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
            EngineCommand::SetPitch { player, semitones, .. } => {
                let idx = player.as_index();
                if idx < MAX_PLAYERS {
                    self.players[idx].set_pitch_semitones(semitones);
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
            EngineCommand::SetFilterMode { bus, mode, .. } => {
                let filter_mode = match mode {
                    super::command::FilterModeParam::Bypass => super::filter::FilterMode::Bypass,
                    super::command::FilterModeParam::Lowpass => super::filter::FilterMode::Lowpass,
                    super::command::FilterModeParam::Bandpass => super::filter::FilterMode::Bandpass,
                    super::command::FilterModeParam::Highpass => super::filter::FilterMode::Highpass,
                };
                match bus {
                    BusId::A => self.buses[0].filter().set_mode(filter_mode),
                    BusId::B => self.buses[1].filter().set_mode(filter_mode),
                    BusId::Master => {}
                }
            }
            EngineCommand::SetFilterCutoff { bus, hz, .. } => {
                match bus {
                    BusId::A => self.buses[0].filter().set_cutoff_hz(hz as f64),
                    BusId::B => self.buses[1].filter().set_cutoff_hz(hz as f64),
                    BusId::Master => {}
                }
            }
            EngineCommand::SetFilterResonance { bus, resonance, .. } => {
                match bus {
                    BusId::A => self.buses[0].filter().set_resonance(resonance as f64),
                    BusId::B => self.buses[1].filter().set_resonance(resonance as f64),
                    BusId::Master => {}
                }
            }
            EngineCommand::SetFilterDrive { bus, drive, .. } => {
                match bus {
                    BusId::A => self.buses[0].filter().set_drive(drive as f64),
                    BusId::B => self.buses[1].filter().set_drive(drive as f64),
                    BusId::Master => {}
                }
            }
            EngineCommand::SetMasterGain { gain, .. } => {
                self.master_gain = gain as f64;
            }
            EngineCommand::Shutdown => {
                for p in &mut self.players {
                    p.stop();
                }
            }
            EngineCommand::SetProcessorType { player, processor_type, .. } => {
                let idx = player.as_index();
                if idx < MAX_PLAYERS {
                    let sr = self.sample_rate;
                    let new_processor: Box<dyn super::timepitch::TimePitchProcessor> = match processor_type {
                        super::command::ProcessorType::Bypass => super::timepitch::bypass_processor(),
                        super::command::ProcessorType::Varispeed => super::timepitch::varispeed_processor(),
                        super::command::ProcessorType::Signalsmith => super::timepitch::default_processor(sr, 2),
                    };
                    self.players[idx].swap_processor(new_processor);
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
        self.master_sample_peak_provisional = [0.0; 2];
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
        let m_sample_peak = self.master_sample_peak_provisional[0].max(self.master_sample_peak_provisional[1]);
        self.meter_snapshot.write_master(m_rms, m_peak, m_sample_peak, self.master_clip);
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
            | EngineCommand::SetPitch { at_frame, .. }
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
            | EngineCommand::SetFilterMode { at_frame, .. }
            | EngineCommand::SetFilterCutoff { at_frame, .. }
            | EngineCommand::SetFilterResonance { at_frame, .. }
            | EngineCommand::SetFilterDrive { at_frame, .. }
            | EngineCommand::SetMasterGain { at_frame, .. } => *at_frame,
            EngineCommand::SetProcessorType { at_frame, .. } => *at_frame,
            EngineCommand::Shutdown => 0,
        }
    }
}

// Preallocated scratch buffer for integer output conversion.
thread_local! {
    static SCRATCH_F32: std::cell::RefCell<Vec<f32>> = std::cell::RefCell::new(vec![0.0f32; 4096]);
}

impl AudioEngine {
    /// Create a new audio engine using the system default device.
    /// Does not start playback. Backward-compatible entry point.
    pub fn new() -> Result<Self, String> {
        Self::new_with_config(&super::io::AudioDeviceConfig::default())
    }

    /// Create a new audio engine with a specific device, sample rate, and
    /// buffer size. Does not start playback.
    ///
    /// Use `io::enumerate_output_devices()` to discover available devices,
    /// then construct an `AudioDeviceConfig` with the desired settings.
    pub fn new_with_config(config: &super::io::AudioDeviceConfig) -> Result<Self, String> {
        let (device, stream_config, sample_rate, sample_format) = super::io::resolve_config(config)
            .map_err(|e| format!("Failed to resolve audio config: {}", e))?;

        // sample_format comes from the ACTUAL selected supported config,
        // not from re-querying default_output_config(). This ensures the
        // callback format matches the stream config that resolve_config
        // selected (e.g., F32 vs I16).

        let frame_counter = Arc::new(AtomicU64::new(0));
        let command_queue = Arc::new(CommandQueue::new(512));
        let meter_snapshot = Arc::new(MeterSnapshot::new());
        // 32 slots: 8 players × 2 Arcs each (player buffer + processor source)
        // plus headroom for rapid relaunching.
        let retired_sources = Arc::new(crossbeam_queue::ArrayQueue::new(128));
        let deferred_overflow = Arc::new(crossbeam_queue::ArrayQueue::new(16));

        let callback_state = CallbackState::new(
            frame_counter.clone(),
            command_queue.clone(),
            meter_snapshot.clone(),
            retired_sources.clone(),
            deferred_overflow.clone(),
            sample_rate as f64,
        );

        let config = stream_config;
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
            retired_sources,
            deferred_overflow,
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

    /// Drain the deferred-destruction queues. Call this periodically from a
    /// non-realtime thread (e.g., the Tauri async runtime) to drop retired
    /// source buffers outside the audio callback. Returns the number of
    /// buffers dropped. Drains both the main queue and the last-resort
    /// overflow queue.
    pub fn drain_retired_sources(&self) -> usize {
        let mut count = 0;
        while self.retired_sources.pop().is_some() {
            count += 1;
        }
        while self.deferred_overflow.pop().is_some() {
            count += 1;
        }
        count
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
    /// The buffer is stored in the engine thread's source registry as an Arc.
    /// This must be called from a non-audio thread (e.g., the Tauri async runtime).
    pub fn register_source(&mut self, buffer: super::command::DecodedBuffer) -> SourceHandle {
        let handle = SourceHandle(self.next_source_handle);
        self.next_source_handle += 1;
        self.sources.insert(handle.0, Arc::new(buffer));
        handle
    }

    /// Unregister a source, dropping the registry's Arc. A player that still
    /// holds its own Arc clone keeps playing; memory is freed when the last
    /// Arc drops.
    pub fn unregister_source(&mut self, handle: SourceHandle) {
        self.sources.remove(&handle.0);
    }

    /// Launch a player with a registered source. Sends a single Launch
    /// command carrying an Arc clone of the buffer; the callback loads it
    /// directly into the player. Must be called from a non-audio thread.
    pub fn launch_player(
        &mut self,
        player: PlayerId,
        source: SourceHandle,
        start_beat: f64,
        quantize: Quantize,
    ) -> Result<(), String> {
        let buffer = self.sources.get(&source.0)
            .ok_or("Source not found in registry")?
            .clone(); // Arc clone — pointer copy only, no PCM duplication

        let at_frame = self.current_frame();
        self.command_queue.push(EngineCommand::Launch {
            player,
            at_frame,
            source,
            buffer,
            start_beat,
            quantize,
        });

        Ok(())
    }
}

/// The real-time audio callback for f32 output.
/// This function MUST NOT allocate, lock, do I/O, or call Tauri.
///
/// Event-sliced rendering: the block is rendered in slices between pending
/// command frames, so a command scheduled for halfway through the block
/// takes effect at that exact frame.
pub fn audio_callback_f32(state: &mut CallbackState, output: &mut [f32]) {
    // PB-3 MVP: the engine's internal signal path is always stereo (2ch).
    // resolve_config() forces stream_config.channels = 2, so the output
    // buffer should always be interleaved stereo. If it isn't (e.g., a
    // device driver quirk), we handle it gracefully.
    let channels = 2;
    let frames = output.len() / channels;
    let block_start = state.frame_counter.load(Ordering::Relaxed);
    let block_end = block_start + frames as u64;

    // Drain the command queue into the sorted pending list.
    while let Some(cmd) = state.command_queue.pop() {
        let at_frame = cmd.at_frame();
        state.insert_pending(PendingCommand { cmd, at_frame });
    }

    // Reset block meters
    state.reset_block_meters();

    let mut cursor_frame = block_start;
    let mut cursor_sample = 0usize;

    while cursor_sample < frames {
        // Apply every command whose frame has arrived.
        state.apply_due_commands(cursor_frame);

        // Render up to the next event frame (or block end).
        let next_event = state
            .next_event_frame(cursor_frame)
            .unwrap_or(block_end)
            .min(block_end);
        let n = ((next_event - cursor_frame) as usize).min(frames - cursor_sample);
        if n == 0 {
            break;
        }

        let start = cursor_sample * channels;
        let end = (cursor_sample + n) * channels;
        render_slice(state, &mut output[start..end], channels);

        cursor_sample += n;
        cursor_frame += n as u64;
    }

    // Update frame counter to block end
    state.frame_counter.store(block_end, Ordering::Relaxed);
    state.meter_snapshot.write_frame(block_end);

    // Update meter snapshot at ~30 Hz
    state.meter_update_counter += frames as u64;
    if state.meter_update_counter >= state.meter_update_interval {
        state.meter_update_counter = 0;
        state.update_meters(frames);
    }
}

/// Render `frames` audio frames with no command application — pure DSP.
/// Split out so the callback can slice rendering between scheduled events.
#[inline]
fn render_slice(state: &mut CallbackState, output: &mut [f32], channels: usize) {
    for frame in output.chunks_mut(channels) {
        // Ramp crossfader
        state.ramp_crossfade();

        // Process all players and route to buses (or direct-to-master)
        let mut direct_l = 0.0f64;
        let mut direct_r = 0.0f64;
        for p in &mut state.players {
            let (l, r) = p.process_sample(state.any_soloed);
            if l != 0.0 || r != 0.0 {
                match p.bus {
                    BusId::A => state.buses[0].accumulate(l, r),
                    BusId::B => state.buses[1].accumulate(l, r),
                    BusId::Master => {
                        // Direct to master — bypasses bus EQ and crossfader,
                        // but is genuinely summed into the output mix.
                        direct_l += l;
                        direct_r += r;
                    }
                }
            }
        }

        // Process buses (EQ, gain, crossfade)
        let (bus_a_l, bus_a_r) = state.buses[0].process_sample();
        let (bus_b_l, bus_b_r) = state.buses[1].process_sample();

        // Sum buses and direct-to-master players into the master mix
        let mut mix_l = bus_a_l + bus_b_l + direct_l;
        let mut mix_r = bus_a_r + bus_b_r + direct_r;

        // Master gain
        mix_l *= state.master_gain;
        mix_r *= state.master_gain;

        // Transparent master path: no always-on waveshaping. Clipping is
        // detected for metering; a proper look-ahead limiter arrives with
        // the loudness/mastering phase. Unity playback null-tests clean.
        let abs_l = mix_l.abs();
        let abs_r = mix_r.abs();
        if abs_l >= 1.0 || abs_r >= 1.0 {
            state.master_clip = true;
        }

        // Update master meters (pre-output-clamp values)
        state.master_sum_sq[0] += mix_l * mix_l;
        state.master_sum_sq[1] += mix_r * mix_r;
        if abs_l > state.master_peak[0] { state.master_peak[0] = abs_l; }
        if abs_r > state.master_peak[1] { state.master_peak[1] = abs_r; }
        // PROVISIONAL sample-peak (same value as master_peak). True-peak
        // requires 4x oversampling and inter-sample reconstruction — arrives
        // with PB-6. Stored separately so the field name doesn't lie.
        if abs_l > state.master_sample_peak_provisional[0] { state.master_sample_peak_provisional[0] = abs_l; }
        if abs_r > state.master_sample_peak_provisional[1] { state.master_sample_peak_provisional[1] = abs_r; }

        // Write output — hard-clamped at the output stage only (transparent
        // below 0 dBFS; the DAC would clip anyway).
        frame[0] = mix_l.clamp(-1.0, 1.0) as f32;
        if channels >= 2 {
            frame[1] = mix_r.clamp(-1.0, 1.0) as f32;
        }
    }
}

#[cfg(test)]
mod tests {
    //! Performance Engine regression harness (PB-0).
    //!
    //! Deterministic offline render tests — no audio device required.
    //! These tests lock in the real-time engine contract: silence, unity
    //! transparency, sample-accurate scheduling, routing, and crossfader
    //! semantics. Every future DSP change must keep these passing.

    use super::*;
    use crate::audio::command::DecodedBuffer;
    use crate::audio::meter::MeterSnapshot;

    const SR: f64 = 44100.0;

    fn make_state() -> CallbackState {
        CallbackState::new_for_test(
            Arc::new(AtomicU64::new(0)),
            Arc::new(CommandQueue::new(512)),
            Arc::new(MeterSnapshot::new()),
            Arc::new(crossbeam_queue::ArrayQueue::new(128)),
            SR,
        )
    }

    /// A decoded buffer of known content: constant value on both channels.
    fn constant_buffer(value: f32, frames: usize) -> Arc<DecodedBuffer> {
        let mut samples = Vec::with_capacity(frames * 2);
        for _ in 0..frames {
            samples.push(value);
            samples.push(value);
        }
        Arc::new(DecodedBuffer {
            samples,
            sample_rate: SR as u32,
            channels: 2,
            duration_sec: frames as f64 / SR,
            bpm: Some(120.0),
            beat_grid: None,
        })
    }

    /// Render one block through the real callback path.
    fn render(state: &mut CallbackState, output: &mut [f32]) {
        audio_callback_f32(state, output);
    }

    #[test]
    fn silence_is_silent() {
        let mut state = make_state();
        let mut out = vec![0.0f32; 512];
        render(&mut state, &mut out);
        assert!(out.iter().all(|&s| s == 0.0), "no players must produce digital silence");
    }

    #[test]
    fn launch_plays_source() {
        let mut state = make_state();
        let buffer = constant_buffer(0.25, 4410); // 0.1s
        state.command_queue.push(EngineCommand::Launch {
            player: PlayerId(0),
            at_frame: 0,
            source: SourceHandle(1),
            buffer: buffer.clone(),
            start_beat: 0.0,
            quantize: Quantize::Immediate,
        });
        // Player 0 defaults to Bus A; set crossfade fully to A and master to unity.
        state.command_queue.push(EngineCommand::SetCrossfade { at_frame: 0, position: 0.0 });
        state.command_queue.push(EngineCommand::SetMasterGain { at_frame: 0, gain: 1.0 });

        let mut out = vec![0.0f32; 512];
        render(&mut state, &mut out);

        // The buffer must actually play — constant 0.25 through unity path.
        // (EQ is transparent at 0 dB; small LR4 settling on DC is expected,
        // so assert signal presence rather than exact value at block start.)
        let non_zero = out.iter().filter(|&&s| s.abs() > 1e-6).count();
        assert!(non_zero > 400, "launched player must produce audio (got {non_zero} non-zero samples)");
    }

    #[test]
    fn master_path_is_transparent_at_unity() {
        // With master gain 1.0 and a signal well below clipping, output must
        // equal input. This is the null test that the always-on soft clipper
        // previously broke (x/(1+|x|) at 0.5 → 0.333).
        let mut state = make_state();
        state.command_queue.push(EngineCommand::SetCrossfade { at_frame: 0, position: 0.0 });
        state.command_queue.push(EngineCommand::SetMasterGain { at_frame: 0, gain: 1.0 });
        state.command_queue.push(EngineCommand::Launch {
            player: PlayerId(0),
            at_frame: 0,
            source: SourceHandle(1),
            buffer: constant_buffer(0.5, 44100),
            start_beat: 0.0,
            quantize: Quantize::Immediate,
        });

        // Render a few blocks to let EQ crossovers settle on the DC signal,
        // then measure steady-state.
        let mut out = vec![0.0f32; 1024];
        for _ in 0..10 {
            render(&mut state, &mut out);
        }
        // Steady-state: constant 0.5 through a transparent path must be ~0.5.
        // The old soft clipper would give 0.5/(1+0.5) = 0.333.
        let tail = &out[out.len() - 256..];
        for &s in tail {
            assert!(
                (s as f64 - 0.5).abs() < 0.01,
                "transparent master path: expected ~0.5, got {s} (soft clipper would give 0.333)"
            );
        }
    }

    #[test]
    fn commands_apply_at_exact_frame_within_block() {
        // A mute scheduled for mid-block must take effect at that exact
        // sample, not at the block boundary. The player routes
        // direct-to-master so there is no bus-EQ filter tail to tolerate —
        // a muted player emits exactly zero from the mute frame onward.
        let mut state = make_state();
        let block_frames = 256u64;
        let change_frame = 100u64; // mid-block

        state.command_queue.push(EngineCommand::SetMasterGain { at_frame: 0, gain: 1.0 });
        state.command_queue.push(EngineCommand::SetBus {
            player: PlayerId(0),
            at_frame: 0,
            bus: BusId::Master,
        });
        state.command_queue.push(EngineCommand::Launch {
            player: PlayerId(0),
            at_frame: 0,
            source: SourceHandle(1),
            buffer: constant_buffer(0.4, 44100),
            start_beat: 0.0,
            quantize: Quantize::Immediate,
        });
        // Mute the player exactly at frame 100 of the first block.
        state.command_queue.push(EngineCommand::SetMute {
            player: PlayerId(0),
            at_frame: change_frame,
            muted: true,
        });

        let mut out = vec![0.0f32; (block_frames * 2) as usize];
        render(&mut state, &mut out);

        // Frames 0..100 carry signal; frames 100..256 must be silent.
        let pre = &out[((change_frame - 4) * 2) as usize..(change_frame * 2) as usize];
        assert!(pre.iter().any(|&s| s.abs() > 1e-6), "signal expected before mute frame");
        let post = &out[(change_frame * 2) as usize..];
        assert!(
            post.iter().all(|&s| s == 0.0),
            "silence expected after exact mute frame; first non-zero: {:?}",
            post.iter().position(|&s| s != 0.0)
        );
    }

    #[test]
    fn direct_to_master_is_audible() {
        // A player routed to BusId::Master must reach the output.
        let mut state = make_state();
        state.command_queue.push(EngineCommand::SetMasterGain { at_frame: 0, gain: 1.0 });
        state.command_queue.push(EngineCommand::SetBus {
            player: PlayerId(0),
            at_frame: 0,
            bus: BusId::Master,
        });
        state.command_queue.push(EngineCommand::Launch {
            player: PlayerId(0),
            at_frame: 0,
            source: SourceHandle(1),
            buffer: constant_buffer(0.3, 44100),
            start_beat: 0.0,
            quantize: Quantize::Immediate,
        });

        let mut out = vec![0.0f32; 2048];
        for _ in 0..5 {
            render(&mut state, &mut out);
        }
        let non_zero = out.iter().filter(|&&s| s.abs() > 1e-6).count();
        assert!(non_zero > 1000, "direct-to-master player must be audible");
    }

    #[test]
    fn crossfade_routes_buses() {
        // Player 0 → A with constant 0.4; player 1 → B silent (not launched).
        let mut state = make_state();
        state.command_queue.push(EngineCommand::SetMasterGain { at_frame: 0, gain: 1.0 });
        state.command_queue.push(EngineCommand::Launch {
            player: PlayerId(0),
            at_frame: 0,
            source: SourceHandle(1),
            buffer: constant_buffer(0.4, 44100),
            start_beat: 0.0,
            quantize: Quantize::Immediate,
        });
        // Crossfade fully A
        state.command_queue.push(EngineCommand::SetCrossfade { at_frame: 0, position: 0.0 });
        let mut out_a = vec![0.0f32; 4096];
        for _ in 0..6 { render(&mut state, &mut out_a); }
        let level_a: f32 = out_a[out_a.len() - 512..].iter().map(|s| s.abs()).sum::<f32>() / 512.0;

        // Crossfade fully B — A's contribution must vanish.
        let frame = state.frame_counter.load(Ordering::Relaxed);
        state.command_queue.push(EngineCommand::SetCrossfade { at_frame: frame, position: 1.0 });
        let mut out_b = vec![0.0f32; 4096];
        for _ in 0..6 { render(&mut state, &mut out_b); }
        let level_b: f32 = out_b[out_b.len() - 512..].iter().map(|s| s.abs()).sum::<f32>() / 512.0;

        assert!(level_a > 0.1, "bus A should be audible at crossfade=0 (got {level_a})");
        assert!(level_b < level_a * 0.05, "bus A must be silenced at crossfade=1 (got {level_b} vs {level_a})");
    }

    #[test]
    fn stop_halts_playback() {
        let mut state = make_state();
        state.command_queue.push(EngineCommand::SetMasterGain { at_frame: 0, gain: 1.0 });
        state.command_queue.push(EngineCommand::SetBus {
            player: PlayerId(0),
            at_frame: 0,
            bus: BusId::Master,
        });
        state.command_queue.push(EngineCommand::Launch {
            player: PlayerId(0),
            at_frame: 0,
            source: SourceHandle(1),
            buffer: constant_buffer(0.4, 44100),
            start_beat: 0.0,
            quantize: Quantize::Immediate,
        });
        let mut out = vec![0.0f32; 1024];
        render(&mut state, &mut out);

        let frame = state.frame_counter.load(Ordering::Relaxed);
        state.command_queue.push(EngineCommand::Stop { player: PlayerId(0), at_frame: frame });
        render(&mut state, &mut out);
        assert!(out.iter().all(|&s| s == 0.0), "stopped player must be silent");
    }

    #[test]
    fn queue_overflow_failsafe() {
        // Pending capacity is preallocated; verify the fail-safe path doesn't
        // panic when exceeded.
        let mut state = make_state();
        let cap = state.pending.capacity();
        for i in 0..(cap + 10) {
            state.insert_pending(PendingCommand {
                cmd: EngineCommand::SetMasterGain { at_frame: 10_000 + i as u64, gain: 0.5 },
                at_frame: 10_000 + i as u64,
            });
        }
        assert!(state.pending.len() <= cap);
    }

    #[test]
    fn retired_sources_are_deferred_not_dropped_on_callback() {
        // When a player is relaunched with a new source, the old buffer's
        // Arc must be pushed to the retirement queue — NOT dropped inside
        // the callback. We verify by checking the queue after relaunch.
        let mut state = make_state();
        state.command_queue.push(EngineCommand::SetMasterGain { at_frame: 0, gain: 1.0 });
        state.command_queue.push(EngineCommand::SetBus {
            player: PlayerId(0),
            at_frame: 0,
            bus: BusId::Master,
        });

        // Launch with first buffer
        let buf1 = constant_buffer(0.4, 44100);
        state.command_queue.push(EngineCommand::Launch {
            player: PlayerId(0),
            at_frame: 0,
            source: SourceHandle(1),
            buffer: buf1,
            start_beat: 0.0,
            quantize: Quantize::Immediate,
        });
        let mut out = vec![0.0f32; 256];
        render(&mut state, &mut out);

        // Relaunch with second buffer — old one should be retired
        let buf2 = constant_buffer(0.5, 44100);
        state.command_queue.push(EngineCommand::Launch {
            player: PlayerId(0),
            at_frame: state.frame_counter.load(Ordering::Relaxed),
            source: SourceHandle(2),
            buffer: buf2,
            start_beat: 0.0,
            quantize: Quantize::Immediate,
        });
        render(&mut state, &mut out);

        // The retirement queue should contain the old buffer Arc(s).
        // At least one (the player's buffer field); possibly two (processor source).
        let mut retired_count = 0;
        while state.retired_sources.pop().is_some() {
            retired_count += 1;
        }
        assert!(
            retired_count >= 1,
            "old source buffer must be deferred to retirement queue (got {retired_count})"
        );
    }
}
