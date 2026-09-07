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
    BusId, CommandAcknowledgement, CommandId, CommandQueue, DecodedBuffer, EngineCommand,
    EqBand, LoadGeneration, QueuedCommand, MAX_PLAYERS, PlayerId, Quantize, SourceHandle,
};
use super::meter::MeterSnapshot;
use super::player::Player;

/// The audio engine. Owns the CPAL stream and shared communication channels.
/// The real-time state is owned by the callback closure, not by this struct.
pub struct AudioEngine {
    frame_counter: Arc<AtomicU64>,
    command_queue: Arc<CommandQueue>,
    acknowledgements: Arc<crossbeam_queue::ArrayQueue<CommandAcknowledgement>>,
    next_command_id: AtomicU64,
    meter_snapshot: Arc<MeterSnapshot>,
    sample_rate: u32,
    output_channels: u16,
    stream: Option<SendStream>,
    /// Source registry — engine thread owns Arc<DecodedBuffer> keyed by
    /// SourceHandle. Launch commands carry an Arc clone; the callback never
    /// touches this registry.
    sources: HashMap<u64, Arc<DecodedBuffer>>,
    next_source_handle: u64,
    /// Registry ownership for the source most recently accepted for each
    /// player. Replacing a player evicts its prior registry reference.
    player_sources: [Option<SourceHandle>; MAX_PLAYERS],
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
    acknowledgements: Arc<crossbeam_queue::ArrayQueue<CommandAcknowledgement>>,
    meter_snapshot: Arc<MeterSnapshot>,
    #[cfg(test)]
    pub players: [Player; MAX_PLAYERS],
    #[cfg(not(test))]
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
    // Master metering across every callback in the reporting window
    // PB-6.2: Realtime true-peak meter (BS.1770 Annex 2, 4x oversampling).
    // Replaces the provisional sample-peak-only meter. Tracks sample peak,
    // true peak (dBTP), and RMS independently. Allocation-free in the callback.
    master_meter: super::master_meter::RealtimeMasterMeter,
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
    // Master/private-cue routing (TL-04)
    master_pair: (usize, usize),
    cue_pair: (usize, usize),
    cue_selected: [bool; MAX_PLAYERS],
    cue_gain: f64,
    cue_master_blend: f64,
    cue_available: bool,
}

struct PendingCommand {
    queued: QueuedCommand,
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
        let acknowledgements = Arc::new(crossbeam_queue::ArrayQueue::new(512));
        Self::new_with_acknowledgements(
            frame_counter,
            command_queue,
            meter_snapshot,
            retired_sources,
            deferred_overflow,
            acknowledgements,
            sample_rate,
        )
    }

    fn new_with_acknowledgements(
        frame_counter: Arc<AtomicU64>,
        command_queue: Arc<CommandQueue>,
        meter_snapshot: Arc<MeterSnapshot>,
        retired_sources: Arc<crossbeam_queue::ArrayQueue<Arc<DecodedBuffer>>>,
        deferred_overflow: Arc<crossbeam_queue::ArrayQueue<Arc<DecodedBuffer>>>,
        acknowledgements: Arc<crossbeam_queue::ArrayQueue<CommandAcknowledgement>>,
        sample_rate: f64,
    ) -> Self {
        Self::new_impl(
            frame_counter,
            command_queue,
            meter_snapshot,
            retired_sources,
            deferred_overflow,
            acknowledgements,
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
            Arc::new(crossbeam_queue::ArrayQueue::new(512)),
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
        acknowledgements: Arc<crossbeam_queue::ArrayQueue<CommandAcknowledgement>>,
        sample_rate: f64,
        use_varispeed: bool,
    ) -> Self {
        let meter_update_interval = (sample_rate / 30.0) as u64; // 30 Hz meter updates
        let mut players: [Player; MAX_PLAYERS] = std::array::from_fn(|i| {
            if use_varispeed {
                Player::new_with_mode(
                    PlayerId(i as u8),
                    sample_rate,
                    retired_sources.clone(),
                    super::timepitch::ProcessorMode::Varispeed,
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
            acknowledgements,
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
            master_meter: super::master_meter::RealtimeMasterMeter::new(sample_rate as u32),
            master_clip: false,
            bus_block_sum_sq: [[0.0; 2]; 2],
            bus_block_peak: [[0.0; 2]; 2],
            sample_rate,
            meter_update_counter: 0,
            meter_update_interval,
            pending: Vec::with_capacity(64),
            any_soloed: false,
            master_pair: (0, 1),
            cue_pair: (2, 3),
            cue_selected: [false; MAX_PLAYERS],
            cue_gain: 1.0,
            cue_master_blend: 0.0,
            cue_available: false,
        }
    }

    /// Insert a pending command, keeping the list sorted by at_frame.
    /// If the preallocated capacity is exceeded, apply immediately instead
    /// of allocating (real-time safety).
    fn insert_pending(&mut self, pc: PendingCommand) {
        if self.pending.len() >= self.pending.capacity() {
            self.apply_queued_command(pc.queued, pc.at_frame);
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
            self.apply_queued_command(pc.queued, frame);
        }
    }

    fn apply_queued_command(&mut self, queued: QueuedCommand, current_frame: u64) {
        let command_id = queued.command_id;
        self.apply_command(queued.command, current_frame);
        if let Some(command_id) = command_id {
            if self
                .acknowledgements
                .push(CommandAcknowledgement {
                    command_id,
                    applied_frame: current_frame,
                })
                .is_err()
            {
                self.meter_snapshot
                    .acknowledgements_dropped
                    .fetch_add(1, Ordering::Relaxed);
            }
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
            EngineCommand::LoadPaused {
                player,
                source,
                buffer,
                start_beat,
                load_generation,
                ..
            } => {
                let idx = player.as_index();
                if idx < MAX_PLAYERS {
                    let unstored = self.players[idx].load_paused(
                        source,
                        buffer,
                        start_beat,
                        load_generation,
                    );
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
            EngineCommand::SeekSourceSeconds { player, source_seconds, .. } => {
                let idx = player.as_index();
                if idx < MAX_PLAYERS {
                    self.players[idx].seek_sec(source_seconds);
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
            EngineCommand::SetLoudnessMatchGain { player, gain } => {
                let idx = player.as_index();
                if idx < MAX_PLAYERS {
                    self.players[idx].set_loudness_match_gain(gain);
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
            EngineCommand::SetHotCue { player, slot, beat, .. } => {
                let idx = player.as_index();
                if idx < MAX_PLAYERS {
                    self.players[idx].set_hot_cue(slot, beat);
                }
            }
            EngineCommand::JumpHotCue { player, slot, .. } => {
                let idx = player.as_index();
                if idx < MAX_PLAYERS {
                    self.players[idx].jump_hot_cue(slot);
                }
            }
            EngineCommand::Nudge { player, beats, .. } => {
                let idx = player.as_index();
                if idx < MAX_PLAYERS {
                    self.players[idx].nudge(beats);
                }
            }
            EngineCommand::JogTouch { player, engaged, .. } => {
                let idx = player.as_index();
                if idx < MAX_PLAYERS {
                    if engaged {
                        self.players[idx].engage_jog();
                    } else {
                        self.players[idx].release_jog();
                    }
                }
            }
            EngineCommand::JogRate { player, rate, .. } => {
                let idx = player.as_index();
                if idx < MAX_PLAYERS {
                    self.players[idx].set_jog_rate(rate);
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
            EngineCommand::SetCueEnabled { player, enabled, .. } => {
                let idx = player.as_index();
                if idx < MAX_PLAYERS {
                    self.cue_selected[idx] = enabled;
                }
            }
            EngineCommand::SetCueGain { gain, .. } => {
                if gain.is_finite() && gain >= 0.0 {
                    self.cue_gain = gain as f64;
                }
            }
            EngineCommand::SetCueMasterBlend { blend, .. } => {
                if blend.is_finite() {
                    self.cue_master_blend = blend.clamp(0.0, 1.0) as f64;
                }
            }
            EngineCommand::SetOutputRouting {
                master_left,
                master_right,
                cue_left,
                cue_right,
                ..
            } => {
                self.master_pair = (master_left as usize, master_right as usize);
                self.cue_pair = (cue_left as usize, cue_right as usize);
            }
            EngineCommand::Shutdown => {
                for p in &mut self.players {
                    p.stop();
                }
            }
            EngineCommand::SetProcessorType { player, processor_type, .. } => {
                let idx = player.as_index();
                if idx < MAX_PLAYERS {
                    let mode = match processor_type {
                        super::command::ProcessorType::Bypass => super::timepitch::ProcessorMode::Bypass,
                        super::command::ProcessorType::Varispeed => super::timepitch::ProcessorMode::Varispeed,
                        super::command::ProcessorType::Signalsmith => super::timepitch::ProcessorMode::Signalsmith,
                    };
                    // No construction or destruction — all three processors
                    // are preconstructed at Player init. This only changes
                    // the mode enum and re-attaches the source.
                    self.players[idx].set_processor_mode(mode);
                }
            }
            EngineCommand::SetListeningCondition { player, processor_type, tempo_rate, pitch_semitones, .. } => {
                let idx = player.as_index();
                if idx < MAX_PLAYERS {
                    let mode = match processor_type {
                        super::command::ProcessorType::Bypass => super::timepitch::ProcessorMode::Bypass,
                        super::command::ProcessorType::Varispeed => super::timepitch::ProcessorMode::Varispeed,
                        super::command::ProcessorType::Signalsmith => super::timepitch::ProcessorMode::Signalsmith,
                    };
                    // Atomic: mode + tempo + pitch all change in one command.
                    self.players[idx].set_processor_mode(mode);
                    self.players[idx].set_tempo(tempo_rate);
                    self.players[idx].set_pitch_semitones(pitch_semitones);
                }
            }
            EngineCommand::BeatSync { player_a, player_b, .. } => {
                let idx_a = player_a.as_index();
                let idx_b = player_b.as_index();
                if idx_a < MAX_PLAYERS && idx_b < MAX_PLAYERS {
                    // Tempo-match B to A: B's tempo ratio = A's effective BPM / B's source BPM
                    let a_effective_bpm = self.players[idx_a].effective_bpm();
                    let b_source_bpm = self.players[idx_b].source_bpm();
                    if b_source_bpm > 0.0 {
                        let tempo_ratio = a_effective_bpm / b_source_bpm;
                        self.players[idx_b].set_tempo(tempo_ratio as f32);
                    }
                    // Align B's fractional beat phase to A's exact beat
                    // position (no rounding), so both decks share the same
                    // sub-beat phase after Sync.
                    let a_beat = self.players[idx_a].beat_position();
                    self.players[idx_b].seek_beats(a_beat);
                    // Start both players
                    self.players[idx_a].play();
                    self.players[idx_b].play();
                }
            }
            EngineCommand::BarSync { player_a, player_b, .. } => {
                let idx_a = player_a.as_index();
                let idx_b = player_b.as_index();
                if idx_a < MAX_PLAYERS && idx_b < MAX_PLAYERS {
                    // Tempo-match B to A (same as BeatSync)
                    let a_effective_bpm = self.players[idx_a].effective_bpm();
                    let b_source_bpm = self.players[idx_b].source_bpm();
                    if b_source_bpm > 0.0 {
                        let tempo_ratio = a_effective_bpm / b_source_bpm;
                        self.players[idx_b].set_tempo(tempo_ratio as f32);
                    }
                    // Align bars: seek B so its bar position matches A's.
                    let a_bar = self.players[idx_a].bar_position();
                    let b_bar = self.players[idx_b].bar_position();
                    let bar_diff = a_bar - b_bar;
                    let b_meter = self.players[idx_b].meter_numerator() as f64;
                    let b_beat = self.players[idx_b].beat_position();
                    // Convert bar difference to beat difference
                    let target_beat_b = b_beat + (bar_diff * b_meter).round();
                    self.players[idx_b].seek_beats(target_beat_b);
                    // Start both players
                    self.players[idx_a].play();
                    self.players[idx_b].play();
                }
            }
            EngineCommand::AttachBeatGrid {
                player,
                source,
                load_generation,
                grid_revision,
                bpm,
                first_beat_sec,
                meter_numerator,
                downbeat_offset,
                ..
            } => {
                let idx = player.as_index();
                if idx < MAX_PLAYERS {
                    self.players[idx].attach_beat_grid(
                        source,
                        load_generation,
                        grid_revision,
                        bpm,
                        first_beat_sec,
                        meter_numerator,
                        downbeat_offset,
                    );
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
        // PB-6.2: master_meter accumulates across callbacks in the
        // reporting window; it is reset only by finalize_block() at
        // meter update time. Bus meters remain per-callback.
        self.bus_block_sum_sq = [[0.0; 2]; 2];
        self.bus_block_peak = [[0.0; 2]; 2];
        for p in &mut self.players {
            p.reset_block_meters();
        }
    }

    fn update_meters(&mut self, sample_count: usize) {
        // Per-player meters + musical telemetry
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
            // Musical telemetry (PB-2.3 Musical Time Bridge)
            let mode_i32 = match p.processor_mode() {
                super::timepitch::ProcessorMode::Bypass => 0,
                super::timepitch::ProcessorMode::Varispeed => 1,
                super::timepitch::ProcessorMode::Signalsmith => 2,
            };
            self.meter_snapshot.write_player_musical(
                i,
                p.source_bpm(),
                p.effective_bpm(),
                p.tempo_ratio(),
                p.pitch_semitones(),
                p.beat_position(),
                p.bar_position(),
                p.meter_numerator(),
                mode_i32,
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

        // Master meters: finalize the reporting window.
        // The meter tracked sample peak, true peak, and RMS continuously
        // across all callbacks since the last finalize_block().
        let (m_sample_peak, m_true_peak, m_rms) = self.master_meter.finalize_block();
        self.meter_snapshot.write_master(m_rms, m_sample_peak, m_sample_peak, m_true_peak, self.master_clip);
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
            | EngineCommand::LoadPaused { at_frame, .. }
            | EngineCommand::Stop { at_frame, .. }
            | EngineCommand::Pause { at_frame, .. }
            | EngineCommand::Resume { at_frame, .. }
            | EngineCommand::Seek { at_frame, .. }
            | EngineCommand::SeekSourceSeconds { at_frame, .. }
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
            | EngineCommand::SetHotCue { at_frame, .. }
            | EngineCommand::JumpHotCue { at_frame, .. }
            | EngineCommand::Nudge { at_frame, .. }
            | EngineCommand::JogTouch { at_frame, .. }
            | EngineCommand::JogRate { at_frame, .. }
            | EngineCommand::SetCrossfade { at_frame, .. }
            | EngineCommand::SetBusGain { at_frame, .. }
            | EngineCommand::SetBusEq { at_frame, .. }
            | EngineCommand::SetFilterMode { at_frame, .. }
            | EngineCommand::SetFilterCutoff { at_frame, .. }
            | EngineCommand::SetFilterResonance { at_frame, .. }
            | EngineCommand::SetFilterDrive { at_frame, .. }
            | EngineCommand::SetMasterGain { at_frame, .. }
            | EngineCommand::SetCueEnabled { at_frame, .. }
            | EngineCommand::SetCueGain { at_frame, .. }
            | EngineCommand::SetCueMasterBlend { at_frame, .. }
            | EngineCommand::SetOutputRouting { at_frame, .. } => *at_frame,
            | EngineCommand::SetProcessorType { at_frame, .. }
            | EngineCommand::SetListeningCondition { at_frame, .. }
            | EngineCommand::BeatSync { at_frame, .. }
            | EngineCommand::BarSync { at_frame, .. }
            | EngineCommand::AttachBeatGrid { at_frame, .. } => *at_frame,
            EngineCommand::SetLoudnessMatchGain { .. } => 0,
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
        let acknowledgements = Arc::new(crossbeam_queue::ArrayQueue::new(512));

        let callback_state = CallbackState::new_with_acknowledgements(
            frame_counter.clone(),
            command_queue.clone(),
            meter_snapshot.clone(),
            retired_sources.clone(),
            deferred_overflow.clone(),
            acknowledgements.clone(),
            sample_rate as f64,
        );

        let config = stream_config;
        let output_channels = config.channels as usize;
        let err_fn = |err| eprintln!("Audio stream error: {}", err);

        let stream = match sample_format {
            SampleFormat::F32 => {
                let mut state = callback_state;
                device
                    .build_output_stream(
                        &config,
                        move |buffer: &mut [f32], _| {
                            audio_callback_f32(&mut state, buffer, output_channels);
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
                                // Chunk at whole-frame boundaries so a slice never
                                // splits an interleaved device frame mid-frame.
                                let channels = output_channels.max(1);
                                let frames_per_chunk = (scratch.len() / channels).max(1);
                                let chunk_size = frames_per_chunk * channels;
                                let mut offset = 0;
                                while offset < total {
                                    let n = chunk_size.min(total - offset);
                                    let scratch_slice = &mut scratch[..n];
                                    audio_callback_f32(&mut state, scratch_slice, output_channels);
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
            acknowledgements,
            next_command_id: AtomicU64::new(1),
            meter_snapshot,
            sample_rate,
            output_channels: output_channels as u16,
            stream: Some(SendStream(stream)),
            sources: HashMap::new(),
            next_source_handle: 1,
            player_sources: [None; MAX_PLAYERS],
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

    /// Submit a command that must be acknowledged after callback application.
    pub fn submit_command(&self, command: EngineCommand) -> Result<CommandId, String> {
        let command_id = CommandId(self.next_command_id.fetch_add(1, Ordering::Relaxed));
        if self.command_queue.push_tracked(command_id, command) {
            Ok(command_id)
        } else {
            Err("Audio command queue is full".to_string())
        }
    }

    /// Drain callback acknowledgements on a non-realtime thread.
    pub fn drain_command_acknowledgements(&self) -> Vec<CommandAcknowledgement> {
        let mut drained = Vec::with_capacity(self.acknowledgements.len());
        while let Some(acknowledgement) = self.acknowledgements.pop() {
            drained.push(acknowledgement);
        }
        drained
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

    /// Get the actual output channel count of the selected device.
    pub fn output_channels(&self) -> u16 {
        self.output_channels
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
        if player.as_index() >= MAX_PLAYERS {
            return Err(format!("Player {} is out of range", player.0));
        }
        let buffer = self.sources.get(&source.0)
            .ok_or("Source not found in registry")?
            .clone(); // Arc clone — pointer copy only, no PCM duplication

        let at_frame = self.current_frame();
        if !self.command_queue.push(EngineCommand::Launch {
            player,
            at_frame,
            source,
            buffer,
            start_beat,
            quantize,
        }) {
            return Err("Audio command queue is full".to_string());
        }

        self.adopt_player_source(player, source)?;

        Ok(())
    }

    /// Atomically attach a registered source and leave the player paused.
    pub fn load_player_paused(
        &mut self,
        player: PlayerId,
        source: SourceHandle,
        start_beat: f64,
        load_generation: LoadGeneration,
    ) -> Result<CommandId, String> {
        if player.as_index() >= MAX_PLAYERS {
            return Err(format!("Player {} is out of range", player.0));
        }
        let buffer = self
            .sources
            .get(&source.0)
            .ok_or("Source not found in registry")?
            .clone();
        let at_frame = self.current_frame();
        let command_id = self.submit_command(EngineCommand::LoadPaused {
            player,
            at_frame,
            source,
            buffer,
            start_beat,
            load_generation,
        })?;
        self.adopt_player_source(player, source)
            .map(|_| command_id)
    }

    fn adopt_player_source(
        &mut self,
        player: PlayerId,
        source: SourceHandle,
    ) -> Result<(), String> {
        let idx = player.as_index();
        if idx >= MAX_PLAYERS {
            return Err(format!("Player {} is out of range", player.0));
        }
        if let Some(previous) = self.player_sources[idx].replace(source) {
            if previous != source {
                self.unregister_source(previous);
            }
        }
        Ok(())
    }

    #[cfg(test)]
    pub fn source_count_for_test(&self) -> usize {
        self.sources.len()
    }
}

/// The real-time audio callback for f32 output.
/// This function MUST NOT allocate, lock, do I/O, or call Tauri.
///
/// Event-sliced rendering: the block is rendered in slices between pending
/// command frames, so a command scheduled for halfway through the block
/// takes effect at that exact frame.
///
/// The engine's internal signal path is always stereo (2 channels).
/// `output_channels` is the actual device channel count (may be 1, 2, 4, 6,
/// 8, etc.). Master is written to the configured master pair and the
/// headphone/cue monitor to the configured cue pair; unused channels are
/// zeroed. On stereo-only devices the cue monitor folds into the master pair.
pub fn audio_callback_f32(state: &mut CallbackState, output: &mut [f32], output_channels: usize) {
    // The engine mixes in stereo internally; the device channel count is the
    // actual frame width. Mono (1ch) is handled explicitly by downmixing.
    let channels = output_channels.max(1);
    let frames = output.len() / channels;
    let block_start = state.frame_counter.load(Ordering::Relaxed);
    let block_end = block_start + frames as u64;

    // Record whether the cue pair fits on this device so render_slice can
    // route master/cue pairs (or fold cue into master on stereo-only devices).
    state.cue_available = state.cue_pair.0 < channels
        && state.cue_pair.1 < channels
        && state.cue_pair != state.master_pair;

    // Drain the command queue into the sorted pending list.
    while let Some(queued) = state.command_queue.pop_queued() {
        let at_frame = queued.command.at_frame();
        state.insert_pending(PendingCommand { queued, at_frame });
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

        // Process all players and route to buses (or direct-to-master),
        // while tapping the cue (PFL) sum for cue-selected players.
        let mut direct_l = 0.0f64;
        let mut direct_r = 0.0f64;
        let mut cue_l = 0.0f64;
        let mut cue_r = 0.0f64;
        for p in &mut state.players {
            let (l, r) = p.process_sample(state.any_soloed);
            if l != 0.0 || r != 0.0 {
                if state.cue_selected[p.id.as_index()] {
                    cue_l += l;
                    cue_r += r;
                }
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
        // PB-6.2: The realtime meter tracks sample peak, true peak (dBTP),
        // and RMS continuously across the reporting window. No per-callback
        // reset needed; finalize_block() reports and resets at ~30 Hz.
        state.master_meter.process(mix_l, mix_r);

        // Headphone/cue monitor: blend cue (PFL) with master, then apply the
        // headphone level. The cue-only fold is used on stereo-only devices.
        let blend = state.cue_master_blend;
        let cue_gain = state.cue_gain;
        let monitor_l = (cue_l * (1.0 - blend) + mix_l * blend) * cue_gain;
        let monitor_r = (cue_r * (1.0 - blend) + mix_r * blend) * cue_gain;
        let cue_fold_l = cue_l * cue_gain;
        let cue_fold_r = cue_r * cue_gain;

        write_output_frame(
            frame,
            channels,
            state.master_pair,
            state.cue_pair,
            state.cue_available,
            mix_l,
            mix_r,
            monitor_l,
            monitor_r,
            cue_fold_l,
            cue_fold_r,
        );
    }
}

/// Write one output frame to the device buffer, routing master and cue to
/// their explicit channel pairs. Unused channels are zeroed. Mono devices
/// downmix master plus folded cue to a single channel. This never allocates.
#[inline]
fn write_output_frame(
    frame: &mut [f32],
    channels: usize,
    master_pair: (usize, usize),
    cue_pair: (usize, usize),
    cue_available: bool,
    master_l: f64,
    master_r: f64,
    monitor_l: f64,
    monitor_r: f64,
    cue_fold_l: f64,
    cue_fold_r: f64,
) {
    for sample in frame.iter_mut() {
        *sample = 0.0;
    }

    let ml = master_l.clamp(-1.0, 1.0) as f32;
    let mr = master_r.clamp(-1.0, 1.0) as f32;

    if channels == 1 {
        // Mono: explicit downmix of master plus folded cue/monitor.
        let fl = cue_fold_l.clamp(-1.0, 1.0);
        let fr = cue_fold_r.clamp(-1.0, 1.0);
        let mono = ((ml as f64 + mr as f64) * 0.5 + (fl + fr) * 0.5).clamp(-1.0, 1.0) as f32;
        frame[0] = mono;
        return;
    }

    if cue_available {
        let hl = monitor_l.clamp(-1.0, 1.0) as f32;
        let hr = monitor_r.clamp(-1.0, 1.0) as f32;
        if master_pair.0 < channels {
            frame[master_pair.0] = ml;
        }
        if master_pair.1 < channels {
            frame[master_pair.1] = mr;
        }
        if cue_pair.0 < channels {
            frame[cue_pair.0] = hl;
        }
        if cue_pair.1 < channels {
            frame[cue_pair.1] = hr;
        }
    } else {
        // Stereo-only fallback: fold the cue-only signal into the master pair
        // so monitoring remains audible without a private cue output.
        let fl = cue_fold_l.clamp(-1.0, 1.0) as f32;
        let fr = cue_fold_r.clamp(-1.0, 1.0) as f32;
        let fold_l = (ml + fl).clamp(-1.0, 1.0);
        let fold_r = (mr + fr).clamp(-1.0, 1.0);
        if master_pair.0 < channels {
            frame[master_pair.0] = fold_l;
        }
        if master_pair.1 < channels {
            frame[master_pair.1] = fold_r;
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

    #[test]
    fn pb62_master_rms_covers_whole_reporting_window() {
        let mut state = make_state();
        state.command_queue.push(EngineCommand::SetMasterGain { at_frame: 0, gain: 1.0 });
        state.command_queue.push(EngineCommand::SetBus { player: PlayerId(0), at_frame: 0, bus: BusId::Master });
        state.command_queue.push(EngineCommand::Launch {
            player: PlayerId(0), at_frame: 0, source: SourceHandle(1),
            buffer: constant_buffer(0.5, 44100), start_beat: 0.0, quantize: Quantize::Immediate,
        });
        let mut out = [0.0; 512];
        // Render enough callbacks to span at least one meter update (~30 Hz).
        // At 44100 Hz with 256-frame blocks, each callback is ~5.8 ms.
        // 12 callbacks ≈ 70 ms, which is at least two 33 ms meter windows.
        for _ in 0..12 {
            audio_callback_f32(&mut state, &mut out, 2);
        }
        let meters = state.meter_snapshot.read_all();
        // The meter should report a non-zero RMS and peak from the
        // constant 0.5 signal (gain 1.0, direct-to-master). The meter
        // tracks continuously across callbacks in the reporting window.
        // Varispeed interpolation may cause slight overshoot above 0.5.
        assert!(meters.master_rms > 0.4 && meters.master_rms < 0.6,
            "master RMS should be ~0.5, got {}", meters.master_rms);
        assert!(meters.master_sample_peak >= 0.5,
            "master sample peak should be >= 0.5, got {}", meters.master_sample_peak);
        assert!(meters.master_sample_peak < 0.6,
            "master sample peak should be < 0.6 (varispeed overshoot), got {}", meters.master_sample_peak);
        // True peak should be present and near 0 dBFS for a 0.5 amplitude signal
        // 20*log10(0.5) ≈ -6.02 dBFS. True peak may slightly exceed sample peak.
        let tp = meters.master_true_peak_dbtp;
        assert!(tp.is_finite() && tp > -7.0 && tp < -5.0,
            "true peak should be ~-6 dBTP, got {}", tp);
    }

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
        audio_callback_f32(state, output, 2);
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

    fn make_engine_without_stream(command_capacity: usize) -> AudioEngine {
        AudioEngine {
            frame_counter: Arc::new(AtomicU64::new(0)),
            command_queue: Arc::new(CommandQueue::new(command_capacity)),
            acknowledgements: Arc::new(crossbeam_queue::ArrayQueue::new(8)),
            next_command_id: AtomicU64::new(1),
            meter_snapshot: Arc::new(MeterSnapshot::new()),
            sample_rate: SR as u32,
            output_channels: 2,
            stream: None,
            sources: HashMap::new(),
            next_source_handle: 1,
            player_sources: [None; MAX_PLAYERS],
            retired_sources: Arc::new(crossbeam_queue::ArrayQueue::new(8)),
            deferred_overflow: Arc::new(crossbeam_queue::ArrayQueue::new(8)),
        }
    }

    #[test]
    fn load_paused_is_silent_until_resume() {
        let mut state = make_state();
        state.command_queue.push(EngineCommand::SetMasterGain {
            at_frame: 0,
            gain: 1.0,
        });
        state.command_queue.push(EngineCommand::SetBus {
            player: PlayerId(0),
            at_frame: 0,
            bus: BusId::Master,
        });
        state.command_queue.push(EngineCommand::LoadPaused {
            player: PlayerId(0),
            at_frame: 0,
            source: SourceHandle(1),
            buffer: constant_buffer(0.25, 4410),
            start_beat: 0.0,
            load_generation: LoadGeneration(1),
        });

        let mut out = vec![0.0f32; 512];
        render(&mut state, &mut out);
        assert!(out.iter().all(|&sample| sample == 0.0));

        state.command_queue.push(EngineCommand::Resume {
            player: PlayerId(0),
            at_frame: state.frame_counter.load(Ordering::Relaxed),
        });
        render(&mut state, &mut out);
        assert!(out.iter().any(|&sample| sample.abs() > 1e-6));
    }

    #[test]
    fn tracked_command_is_acknowledged_only_after_callback_application() {
        let mut state = make_state();
        assert!(state.command_queue.push_tracked(
            CommandId(42),
            EngineCommand::SetGain {
                player: PlayerId(0),
                at_frame: 0,
                gain: 0.5,
                ramp_frames: 1,
            },
        ));
        assert!(state.acknowledgements.is_empty());

        let mut out = [0.0f32; 2];
        render(&mut state, &mut out);
        assert_eq!(
            state.acknowledgements.pop(),
            Some(CommandAcknowledgement {
                command_id: CommandId(42),
                applied_frame: 0,
            }),
        );
    }

    #[test]
    fn acknowledgement_overflow_is_counted_without_blocking_callback() {
        let meter_snapshot = Arc::new(MeterSnapshot::new());
        let acknowledgements = Arc::new(crossbeam_queue::ArrayQueue::new(1));
        let mut state = CallbackState::new_impl(
            Arc::new(AtomicU64::new(0)),
            Arc::new(CommandQueue::new(8)),
            meter_snapshot.clone(),
            Arc::new(crossbeam_queue::ArrayQueue::new(8)),
            Arc::new(crossbeam_queue::ArrayQueue::new(8)),
            acknowledgements.clone(),
            SR,
            true,
        );
        for command_id in [CommandId(1), CommandId(2)] {
            assert!(state.command_queue.push_tracked(
                command_id,
                EngineCommand::Pause {
                    player: PlayerId(0),
                    at_frame: 0,
                },
            ));
        }

        let mut out = [0.0f32; 2];
        render(&mut state, &mut out);
        assert_eq!(acknowledgements.len(), 1);
        assert_eq!(meter_snapshot.read_all().acknowledgements_dropped, 1);
    }

    #[test]
    fn stale_beat_grid_cannot_modify_replacement_source() {
        let mut state = make_state();
        let mut replacement = constant_buffer(0.1, 4410);
        Arc::get_mut(&mut replacement).unwrap().bpm = Some(130.0);

        state.command_queue.push(EngineCommand::LoadPaused {
            player: PlayerId(0),
            at_frame: 0,
            source: SourceHandle(1),
            buffer: constant_buffer(0.1, 4410),
            start_beat: 0.0,
            load_generation: LoadGeneration(1),
        });
        state.command_queue.push(EngineCommand::LoadPaused {
            player: PlayerId(0),
            at_frame: 0,
            source: SourceHandle(2),
            buffer: replacement,
            start_beat: 0.0,
            load_generation: LoadGeneration(2),
        });
        state.command_queue.push(EngineCommand::AttachBeatGrid {
            player: PlayerId(0),
            at_frame: 0,
            source: SourceHandle(1),
            load_generation: LoadGeneration(1),
            grid_revision: 1,
            bpm: 77.0,
            first_beat_sec: 0.0,
            meter_numerator: 4,
            downbeat_offset: 0,
        });

        let mut out = [0.0f32; 2];
        render(&mut state, &mut out);
        assert_eq!(state.players[0].source_bpm(), 130.0);

        state.command_queue.push(EngineCommand::AttachBeatGrid {
            player: PlayerId(0),
            at_frame: state.frame_counter.load(Ordering::Relaxed),
            source: SourceHandle(2),
            load_generation: LoadGeneration(2),
            grid_revision: 1,
            bpm: 140.0,
            first_beat_sec: 0.0,
            meter_numerator: 4,
            downbeat_offset: 0,
        });
        render(&mut state, &mut out);
        assert_eq!(state.players[0].source_bpm(), 140.0);
    }

    #[test]
    fn repeated_player_loads_keep_one_registry_source() {
        let mut engine = make_engine_without_stream(8);

        for generation in 1..=100 {
            let buffer = Arc::try_unwrap(constant_buffer(0.1, 16)).unwrap();
            let source = engine.register_source(buffer);
            engine
                .load_player_paused(
                    PlayerId(0),
                    source,
                    0.0,
                    LoadGeneration(generation),
                )
                .unwrap();
            drop(engine.command_queue.pop());
            assert_eq!(engine.source_count_for_test(), 1);
        }
    }

    #[test]
    fn tracked_submission_reports_queue_full() {
        let engine = make_engine_without_stream(2);
        for _ in 0..2 {
            engine
                .submit_command(EngineCommand::Pause {
                    player: PlayerId(0),
                    at_frame: 0,
                })
                .unwrap();
        }
        assert_eq!(
            engine.submit_command(EngineCommand::Pause {
                player: PlayerId(0),
                at_frame: 0,
            }),
            Err("Audio command queue is full".to_string()),
        );
        assert_eq!(engine.command_queue.dropped_count(), 1);
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
                queued: QueuedCommand {
                    command_id: None,
                    command: EngineCommand::SetMasterGain {
                        at_frame: 10_000 + i as u64,
                        gain: 0.5,
                    },
                },
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

    // ── TL-04: master/private-cue routing ─────────────────────────────

    #[test]
    fn cue_never_reaches_master_on_four_channel_device() {
        let mut state = make_state();
        state.command_queue.push(EngineCommand::SetMasterGain { at_frame: 0, gain: 1.0 });
        // Crossfade fully B: player 0 (Bus A) is silent in the master mix.
        state.command_queue.push(EngineCommand::SetCrossfade { at_frame: 0, position: 1.0 });
        state.command_queue.push(EngineCommand::Launch {
            player: PlayerId(0),
            at_frame: 0,
            source: SourceHandle(1),
            buffer: constant_buffer(0.5, 44100),
            start_beat: 0.0,
            quantize: Quantize::Immediate,
        });
        state.command_queue.push(EngineCommand::SetCueEnabled {
            player: PlayerId(0),
            at_frame: 0,
            enabled: true,
        });

        let mut out = vec![0.0f32; 256 * 4];
        for _ in 0..10 {
            audio_callback_f32(&mut state, &mut out, 4);
        }

        let tail = &out[out.len() - 256 * 4..];
        for frame in tail.chunks(4) {
            assert!(
                frame[0].abs() < 1e-6 && frame[1].abs() < 1e-6,
                "master pair must stay silent when the deck is crossfaded out"
            );
        }
        let cue_nonzero = tail
            .chunks(4)
            .filter(|f| f[2].abs() > 1e-6 || f[3].abs() > 1e-6)
            .count();
        assert!(cue_nonzero > 200, "cue pair must carry the cued deck");
    }

    #[test]
    fn stereo_fallback_folds_cue_into_master() {
        let mut state = make_state();
        state.command_queue.push(EngineCommand::SetMasterGain { at_frame: 0, gain: 1.0 });
        state.command_queue.push(EngineCommand::SetCrossfade { at_frame: 0, position: 1.0 });
        state.command_queue.push(EngineCommand::Launch {
            player: PlayerId(0),
            at_frame: 0,
            source: SourceHandle(1),
            buffer: constant_buffer(0.5, 44100),
            start_beat: 0.0,
            quantize: Quantize::Immediate,
        });
        state.command_queue.push(EngineCommand::SetCueEnabled {
            player: PlayerId(0),
            at_frame: 0,
            enabled: true,
        });

        let mut out = vec![0.0f32; 256 * 2];
        for _ in 0..10 {
            audio_callback_f32(&mut state, &mut out, 2);
        }

        let tail = &out[out.len() - 256 * 2..];
        let nonzero = tail.iter().filter(|&&s| s.abs() > 1e-6).count();
        assert!(
            nonzero > 200,
            "stereo-only fallback must fold cue into the master pair"
        );
    }

    #[test]
    fn six_channel_device_zeroes_unused_channels() {
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

        let mut out = vec![0.0f32; 256 * 6];
        for _ in 0..5 {
            audio_callback_f32(&mut state, &mut out, 6);
        }

        let tail = &out[out.len() - 256 * 6..];
        for frame in tail.chunks(6) {
            assert!(
                frame[0].abs() > 1e-6 || frame[1].abs() > 1e-6,
                "master pair should carry signal"
            );
            assert!(frame[4].abs() < 1e-6 && frame[5].abs() < 1e-6, "channels 4,5 must be zeroed");
        }
    }

    #[test]
    fn mono_device_downmixes_explicitly() {
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

        let mut out = vec![0.0f32; 256];
        for _ in 0..5 {
            audio_callback_f32(&mut state, &mut out, 1);
        }

        let tail = &out[out.len() - 256..];
        let nonzero = tail.iter().filter(|&&s| s.abs() > 1e-6).count();
        assert!(nonzero > 200, "mono device must downmix master to one channel");
    }

    #[test]
    fn frame_counter_advances_by_whole_frames_across_channel_counts() {
        for channels in [1usize, 2, 4, 6, 8] {
            let mut state = make_state();
            let frames = 64usize;
            let mut out = vec![0.0f32; frames * channels];
            audio_callback_f32(&mut state, &mut out, channels);
            assert_eq!(
                state.frame_counter.load(Ordering::Relaxed),
                frames as u64,
                "channels={channels} must advance exactly {frames} frames"
            );
        }
    }

    #[test]
    fn cue_gain_scales_headphone_output() {
        let mut state = make_state();
        state.command_queue.push(EngineCommand::SetMasterGain { at_frame: 0, gain: 1.0 });
        state.command_queue.push(EngineCommand::SetCrossfade { at_frame: 0, position: 1.0 });
        state.command_queue.push(EngineCommand::Launch {
            player: PlayerId(0),
            at_frame: 0,
            source: SourceHandle(1),
            buffer: constant_buffer(0.5, 44100),
            start_beat: 0.0,
            quantize: Quantize::Immediate,
        });
        state.command_queue.push(EngineCommand::SetCueEnabled {
            player: PlayerId(0),
            at_frame: 0,
            enabled: true,
        });
        state.command_queue.push(EngineCommand::SetCueGain { at_frame: 0, gain: 0.5 });

        let mut out = vec![0.0f32; 256 * 4];
        for _ in 0..10 {
            audio_callback_f32(&mut state, &mut out, 4);
        }

        let tail = &out[out.len() - 256 * 4..];
        let cue_level = tail.chunks(4).map(|f| f[2].abs()).sum::<f32>() / 256.0;
        assert!(
            (cue_level - 0.25).abs() < 0.05,
            "cue gain 0.5 should halve the cue to ~0.25, got {cue_level}"
        );
    }

    #[test]
    fn only_cue_selected_decks_feed_cue_bus() {
        let mut state = make_state();
        state.command_queue.push(EngineCommand::SetMasterGain { at_frame: 0, gain: 1.0 });
        state.command_queue.push(EngineCommand::SetCrossfade { at_frame: 0, position: 1.0 });
        state.command_queue.push(EngineCommand::SetBus {
            player: PlayerId(0),
            at_frame: 0,
            bus: BusId::A,
        });
        state.command_queue.push(EngineCommand::SetBus {
            player: PlayerId(1),
            at_frame: 0,
            bus: BusId::A,
        });
        state.command_queue.push(EngineCommand::Launch {
            player: PlayerId(0),
            at_frame: 0,
            source: SourceHandle(1),
            buffer: constant_buffer(0.5, 44100),
            start_beat: 0.0,
            quantize: Quantize::Immediate,
        });
        state.command_queue.push(EngineCommand::Launch {
            player: PlayerId(1),
            at_frame: 0,
            source: SourceHandle(2),
            buffer: constant_buffer(0.3, 44100),
            start_beat: 0.0,
            quantize: Quantize::Immediate,
        });
        // Only player 1 is cue-selected.
        state.command_queue.push(EngineCommand::SetCueEnabled {
            player: PlayerId(1),
            at_frame: 0,
            enabled: true,
        });

        let mut out = vec![0.0f32; 256 * 4];
        for _ in 0..10 {
            audio_callback_f32(&mut state, &mut out, 4);
        }

        let tail = &out[out.len() - 256 * 4..];
        let cue_level = tail.chunks(4).map(|f| f[2].abs()).sum::<f32>() / 256.0;
        assert!(
            (cue_level - 0.3).abs() < 0.05,
            "cue must carry only the selected deck (~0.3), got {cue_level}"
        );
    }

    #[test]
    fn explicit_output_routing_moves_cue_pair() {
        let mut state = make_state();
        state.command_queue.push(EngineCommand::SetMasterGain { at_frame: 0, gain: 1.0 });
        state.command_queue.push(EngineCommand::SetCrossfade { at_frame: 0, position: 1.0 });
        state.command_queue.push(EngineCommand::Launch {
            player: PlayerId(0),
            at_frame: 0,
            source: SourceHandle(1),
            buffer: constant_buffer(0.5, 44100),
            start_beat: 0.0,
            quantize: Quantize::Immediate,
        });
        state.command_queue.push(EngineCommand::SetCueEnabled {
            player: PlayerId(0),
            at_frame: 0,
            enabled: true,
        });
        // Route cue to channels 4,5 on a 6-channel device.
        state.command_queue.push(EngineCommand::SetOutputRouting {
            at_frame: 0,
            master_left: 0,
            master_right: 1,
            cue_left: 4,
            cue_right: 5,
        });

        let mut out = vec![0.0f32; 256 * 6];
        for _ in 0..10 {
            audio_callback_f32(&mut state, &mut out, 6);
        }

        let tail = &out[out.len() - 256 * 6..];
        let cue_level = tail.chunks(6).map(|f| f[4].abs()).sum::<f32>() / 256.0;
        assert!(cue_level > 0.3, "cue must appear on the routed pair (4,5), got {cue_level}");
        for frame in tail.chunks(6) {
            assert!(
                frame[2].abs() < 1e-6 && frame[3].abs() < 1e-6,
                "default cue pair (2,3) must stay silent after rerouting"
            );
        }
    }

    // ── TL-05: hot cues, nudge, fractional phase, grid revision ───────

    #[test]
    fn hot_cue_set_and_jump() {
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
            buffer: constant_buffer(0.3, 44100 * 5),
            start_beat: 0.0,
            quantize: Quantize::Immediate,
        });
        // Store beat 4.0 in hot-cue slot 0.
        state.command_queue.push(EngineCommand::SetHotCue {
            player: PlayerId(0),
            at_frame: 0,
            slot: 0,
            beat: 4.0,
        });

        let mut out = vec![0.0f32; 512];
        for _ in 0..5 {
            render(&mut state, &mut out);
        }

        // Jump to hot cue 0: position should be ~4 beats = 2.0s at 120 BPM.
        state.command_queue.push(EngineCommand::JumpHotCue {
            player: PlayerId(0),
            at_frame: state.frame_counter.load(Ordering::Relaxed),
            slot: 0,
        });
        render(&mut state, &mut out);

        let pos = state.players[0].get_position_sec();
        assert!(
            (pos - 2.0).abs() < 0.05,
            "hot cue jump should position at ~2.0s, got {pos}"
        );
        assert!(state.players[0].playing, "hot cue jump should start playback");
    }

    #[test]
    fn nudge_shifts_beat_position() {
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

        let mut out = vec![0.0f32; 512];
        for _ in 0..5 {
            render(&mut state, &mut out);
        }
        let before = state.players[0].beat_position();

        state.command_queue.push(EngineCommand::Nudge {
            player: PlayerId(0),
            at_frame: state.frame_counter.load(Ordering::Relaxed),
            beats: 0.25,
        });
        render(&mut state, &mut out);

        let after = state.players[0].beat_position();
        // The nudge shifts by 0.25 beats, plus one block of playback advance
        // during the render that applies the command.
        assert!(
            (after - before - 0.25).abs() < 0.02,
            "nudge +0.25 should shift beat position by ~0.25 ({} -> {})",
            before,
            after
        );
    }

    #[test]
    fn beat_sync_aligns_fractional_phase() {
        let mut state = make_state();
        state.command_queue.push(EngineCommand::SetMasterGain { at_frame: 0, gain: 1.0 });
        state.command_queue.push(EngineCommand::Launch {
            player: PlayerId(0),
            at_frame: 0,
            source: SourceHandle(1),
            buffer: constant_buffer(0.3, 44100),
            start_beat: 0.0,
            quantize: Quantize::Immediate,
        });
        state.command_queue.push(EngineCommand::Launch {
            player: PlayerId(1),
            at_frame: 0,
            source: SourceHandle(2),
            buffer: constant_buffer(0.3, 44100),
            start_beat: 0.0,
            quantize: Quantize::Immediate,
        });
        // Different fractional phases: A at 2.3, B at 5.7 beats.
        state.command_queue.push(EngineCommand::Seek {
            player: PlayerId(0),
            at_frame: 0,
            source_beat: 2.3,
        });
        state.command_queue.push(EngineCommand::Seek {
            player: PlayerId(1),
            at_frame: 0,
            source_beat: 5.7,
        });

        let mut out = vec![0.0f32; 512];
        render(&mut state, &mut out);

        state.command_queue.push(EngineCommand::BeatSync {
            player_a: PlayerId(0),
            player_b: PlayerId(1),
            at_frame: state.frame_counter.load(Ordering::Relaxed),
        });
        render(&mut state, &mut out);

        let a_beat = state.players[0].beat_position();
        let b_beat = state.players[1].beat_position();
        assert!(
            (a_beat - b_beat).abs() < 0.05,
            "fractional beat phase should align: A={a_beat}, B={b_beat}"
        );
    }

    #[test]
    fn stale_grid_revision_is_rejected() {
        let mut state = make_state();
        state.command_queue.push(EngineCommand::LoadPaused {
            player: PlayerId(0),
            at_frame: 0,
            source: SourceHandle(1),
            buffer: constant_buffer(0.1, 4410),
            start_beat: 0.0,
            load_generation: LoadGeneration(1),
        });
        state.command_queue.push(EngineCommand::AttachBeatGrid {
            player: PlayerId(0),
            at_frame: 0,
            source: SourceHandle(1),
            load_generation: LoadGeneration(1),
            grid_revision: 2,
            bpm: 130.0,
            first_beat_sec: 0.0,
            meter_numerator: 4,
            downbeat_offset: 0,
        });

        let mut out = [0.0f32; 2];
        render(&mut state, &mut out);
        assert_eq!(state.players[0].source_bpm(), 130.0);

        // A stale (older) grid revision must not overwrite the newer grid.
        state.command_queue.push(EngineCommand::AttachBeatGrid {
            player: PlayerId(0),
            at_frame: state.frame_counter.load(Ordering::Relaxed),
            source: SourceHandle(1),
            load_generation: LoadGeneration(1),
            grid_revision: 1,
            bpm: 77.0,
            first_beat_sec: 0.0,
            meter_numerator: 4,
            downbeat_offset: 0,
        });
        render(&mut state, &mut out);
        assert_eq!(
            state.players[0].source_bpm(),
            130.0,
            "stale grid revision must be rejected"
        );
    }

    // ── TL-06: jog reverse, hold, and release/resume ──────────────────

    #[test]
    fn jog_reverse_hold_and_resume() {
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
            buffer: constant_buffer(0.3, 44100 * 5),
            start_beat: 0.0,
            quantize: Quantize::Immediate,
        });

        let mut out = vec![0.0f32; 512];
        for _ in 0..5 {
            render(&mut state, &mut out);
        }

        // Engage jog: hold at zero, position must not advance.
        state.command_queue.push(EngineCommand::JogTouch {
            player: PlayerId(0),
            at_frame: state.frame_counter.load(Ordering::Relaxed),
            engaged: true,
        });
        render(&mut state, &mut out);
        let held = state.players[0].get_position_sec();

        // Reverse: position must decrease.
        state.command_queue.push(EngineCommand::JogRate {
            player: PlayerId(0),
            at_frame: state.frame_counter.load(Ordering::Relaxed),
            rate: -1.0,
        });
        render(&mut state, &mut out);
        let reversed = state.players[0].get_position_sec();
        assert!(
            reversed < held,
            "reverse jog should decrease position: {held} -> {reversed}"
        );

        // Release: resume forward playback from the scratch position.
        state.command_queue.push(EngineCommand::JogTouch {
            player: PlayerId(0),
            at_frame: state.frame_counter.load(Ordering::Relaxed),
            engaged: false,
        });
        render(&mut state, &mut out);
        let resumed = state.players[0].get_position_sec();
        assert!(
            resumed > reversed,
            "release should resume forward playback: {reversed} -> {resumed}"
        );
    }
}
