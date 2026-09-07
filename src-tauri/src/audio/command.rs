// Commands from UI to the audio engine.
//
// All commands are frame-addressed: they specify the output frame at which
// they should take effect. The callback applies each command at the exact
// requested frame within the block (event-sliced rendering), not at block
// boundaries.
//
// The command queue is a bounded lock-free SPSC queue (crossbeam ArrayQueue).
// The UI thread is the producer; the audio callback is the consumer.
// If the queue is full, the command is dropped and the UI is notified via
// the meter snapshot (a `command_dropped` counter increments).
//
// Source ownership model (unequivocal):
//   - The engine thread (non-real-time) owns the source registry:
//     HashMap<u64, Arc<DecodedBuffer>>.
//   - Launch commands carry an Arc<DecodedBuffer> — a reference-counted
//     pointer clone (16 bytes), NOT a PCM buffer copy. No giant buffers
//     travel through the command queue.
//   - SourceHandle is the UI-facing identifier; the registry owns the
//     shared allocation. Unregistering drops the registry's Arc; a player
//     that still holds its own Arc keeps playing.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Maximum number of player slots in the engine.
/// Eight slots are available; the UI recommends 2-4 active layers.
pub const MAX_PLAYERS: usize = 8;

/// Identifier for a player slot (0..MAX_PLAYERS-1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PlayerId(pub u8);

impl PlayerId {
    pub fn as_index(&self) -> usize {
        self.0 as usize
    }
}

/// Identifier for a mix bus. The engine has Bus A, Bus B, and direct-to-master.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BusId {
    A,
    B,
    /// Direct to master, bypasses the crossfader.
    Master,
}

/// Handle to a decoded audio source stored in the engine's source registry.
/// The actual audio data lives in a worker-managed cache, not in the command.
/// This keeps commands lightweight and avoids large allocations in the
/// command queue or the audio callback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceHandle(pub u64);

/// Monotonic server-side identity for an asynchronous player load.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoadGeneration(pub u64);

/// Monotonic identity for a command accepted by one engine generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CommandId(pub u64);

/// Confirmation emitted after the realtime callback applies a tracked command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandAcknowledgement {
    pub command_id: CommandId,
    pub applied_frame: u64,
}

pub(crate) struct QueuedCommand {
    pub command_id: Option<CommandId>,
    pub command: EngineCommand,
}

/// Quantization point for launching a player.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quantize {
    /// Launch immediately at the requested frame.
    Immediate,
    /// Launch at the next beat boundary.
    NextBeat,
    /// Launch at the next bar boundary.
    NextBar,
    /// Launch at the next phrase boundary (typically 8 bars).
    NextPhrase,
}

/// Commands sent from the UI to the audio engine.
#[derive(Debug, Clone)]
pub enum EngineCommand {
    /// Launch a player with a source at a specific position.
    /// The player starts playing at `at_frame` (sample-accurate within the
    /// block). `start_beat` is the position in the source in beats.
    /// `buffer` is an Arc clone from the engine-thread source registry —
    /// the callback loads it directly into the player.
    /// Quantize is resolved by the caller into `at_frame`; the callback
    /// treats all scheduling as frame-addressed.
    Launch {
        player: PlayerId,
        at_frame: u64,
        source: SourceHandle,
        buffer: Arc<DecodedBuffer>,
        start_beat: f64,
        quantize: Quantize,
    },
    /// Replace a player's source and leave transport paused as one atomic
    /// callback operation. This prevents a load from rendering even one
    /// frame before a following Pause command can be observed.
    LoadPaused {
        player: PlayerId,
        at_frame: u64,
        source: SourceHandle,
        buffer: Arc<DecodedBuffer>,
        start_beat: f64,
        load_generation: LoadGeneration,
    },
    /// Stop a player at the given output frame.
    Stop {
        player: PlayerId,
        at_frame: u64,
    },
    /// Pause a player (stop playback, retain position).
    Pause {
        player: PlayerId,
        at_frame: u64,
    },
    /// Resume playback from current position.
    Resume {
        player: PlayerId,
        at_frame: u64,
    },
    /// Seek a player to a position in the source (in beats).
    Seek {
        player: PlayerId,
        at_frame: u64,
        source_beat: f64,
    },
    /// Seek a player to a position in the source (in seconds).
    /// Used by the Listening Lab's ABX cue positioning, where the
    /// cue is denominated in seconds rather than beats.
    SeekSourceSeconds {
        player: PlayerId,
        at_frame: u64,
        source_seconds: f64,
    },
    /// Set player tempo rate (1.0 = original, 0.92-1.08 typical range).
    SetTempo {
        player: PlayerId,
        at_frame: u64,
        rate: f32,
    },
    /// Set player pitch shift in semitones (independent of tempo).
    SetPitch {
        player: PlayerId,
        at_frame: u64,
        semitones: f32,
    },
    /// Set player gain with a ramp to avoid clicks.
    SetGain {
        player: PlayerId,
        at_frame: u64,
        gain: f32,
        ramp_frames: u32,
    },
    /// PB-6.1: Set loudness match gain (linear, separate from user gain).
    /// 1.0 = no match. Applied immediately (not ramped).
    SetLoudnessMatchGain {
        player: PlayerId,
        gain: f64,
    },
    /// Set player pan (-1.0 = full left, 0.0 = center, 1.0 = full right).
    SetPan {
        player: PlayerId,
        at_frame: u64,
        pan: f32,
    },
    /// Set player mute state.
    SetMute {
        player: PlayerId,
        at_frame: u64,
        muted: bool,
    },
    /// Set player solo state. When any player is soloed, only soloed
    /// players are audible.
    SetSolo {
        player: PlayerId,
        at_frame: u64,
        soloed: bool,
    },
    /// Assign a player to a bus.
    SetBus {
        player: PlayerId,
        at_frame: u64,
        bus: BusId,
    },
    /// Set EQ band gain in dB for a player. Ramped.
    SetEqGain {
        player: PlayerId,
        at_frame: u64,
        band: EqBand,
        gain_db: f32,
    },
    /// Kill an EQ band on a player (full cut).
    SetEqKill {
        player: PlayerId,
        at_frame: u64,
        band: EqBand,
        killed: bool,
    },
    /// Set loop region in beats for a player. None disables looping.
    SetLoop {
        player: PlayerId,
        at_frame: u64,
        loop_region: Option<LoopRegion>,
    },
    /// Set crossfader position (0.0 = full A, 1.0 = full B). Ramped.
    SetCrossfade {
        at_frame: u64,
        position: f32,
    },
    /// Set bus gain.
    SetBusGain {
        bus: BusId,
        at_frame: u64,
        gain: f32,
    },
    /// Set bus EQ band gain in dB.
    SetBusEq {
        bus: BusId,
        at_frame: u64,
        band: EqBand,
        gain_db: f32,
    },
    /// Set a bus's TuneLock filter mode (bypass/lp/bp/hp).
    SetFilterMode {
        bus: BusId,
        at_frame: u64,
        mode: FilterModeParam,
    },
    /// Set a bus's TuneLock filter cutoff (Hz). Swept logarithmically by UI.
    SetFilterCutoff {
        bus: BusId,
        at_frame: u64,
        hz: f32,
    },
    /// Set a bus's TuneLock filter resonance (0.0-1.0).
    SetFilterResonance {
        bus: BusId,
        at_frame: u64,
        resonance: f32,
    },
    /// Set a bus's TuneLock filter pre-drive (0.0 = off).
    SetFilterDrive {
        bus: BusId,
        at_frame: u64,
        drive: f32,
    },
    /// Set master gain.
    SetMasterGain {
        at_frame: u64,
        gain: f32,
    },
    /// Set per-deck private-cue selection (PFL). A cue-selected player's
    /// post-trim/deck-EQ signal is summed into the cue bus, independent of
    /// its channel fader and crossfader assignment.
    SetCueEnabled {
        player: PlayerId,
        at_frame: u64,
        enabled: bool,
    },
    /// Set the headphone/cue output level (linear, 0.0 = silent).
    SetCueGain {
        at_frame: u64,
        gain: f32,
    },
    /// Set the cue/master blend for the headphone output.
    /// 0.0 = cue only, 1.0 = master only.
    SetCueMasterBlend {
        at_frame: u64,
        blend: f32,
    },
    /// Set the explicit master and cue output channel pairs (0-based).
    /// Defaults: master (0,1), cue (2,3). Used to map the S3 hypothesis
    /// (master 1-2, headphones 3-4) or other multichannel layouts.
    SetOutputRouting {
        at_frame: u64,
        master_left: u8,
        master_right: u8,
        cue_left: u8,
        cue_right: u8,
    },
    /// Shutdown the engine.
    Shutdown,
    /// Set the processor type for a player. Used by the Listening Lab to
    /// switch between bypass (true reference), varispeed, and signalsmith.
    /// All three processors are preconstructed — no allocation in the
    /// callback. This only changes the mode enum and re-attaches source.
    SetProcessorType {
        player: PlayerId,
        at_frame: u64,
        processor_type: ProcessorType,
    },
    /// Atomic listening condition: set processor mode, tempo, and pitch
    /// in a single command. Used by the Listening Lab's ABX mode so all
    /// three parameters change deterministically at the same frame,
    /// with no IPC ordering ambiguity.
    SetListeningCondition {
        player: PlayerId,
        at_frame: u64,
        processor_type: ProcessorType,
        tempo_rate: f32,
        pitch_semitones: f32,
    },
    /// Beat Sync: tempo-match player B to player A's effective BPM and
    /// align their nearest beat-grid beats. Both players start at the
    /// same future frame. Used by the Listening Lab's two-deck mode.
    BeatSync {
        player_a: PlayerId,
        player_b: PlayerId,
        at_frame: u64,
    },
    /// Bar Sync: like BeatSync but additionally aligns downbeat/bar
    /// boundaries rather than just beat boundaries.
    BarSync {
        player_a: PlayerId,
        player_b: PlayerId,
        at_frame: u64,
    },
    /// Attach a beat grid to an already-loaded player. Used when beat-grid
    /// analysis completes asynchronously after the player is already loaded.
    /// Updates bpm, first_beat_sec, meter_numerator on the player without
    /// reloading the source. Does NOT affect playback position.
    AttachBeatGrid {
        player: PlayerId,
        at_frame: u64,
        source: SourceHandle,
        load_generation: LoadGeneration,
        bpm: f64,
        first_beat_sec: f64,
        meter_numerator: i32,
        downbeat_offset: usize,
    },
}

/// Processor type for the Listening Lab's reference comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessorType {
    /// True bypass — reads directly from source, no processing.
    Bypass,
    /// Varispeed — tempo and pitch coupled, cubic interpolation.
    Varispeed,
    /// Signalsmith Stretch — independent tempo and pitch.
    Signalsmith,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EqBand {
    Low,
    Mid,
    High,
}

/// Filter mode parameter (serializable across the command queue).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterModeParam {
    Bypass,
    Lowpass,
    Bandpass,
    Highpass,
}

#[derive(Debug, Clone, Copy)]
pub struct LoopRegion {
    pub start_beat: f64,
    pub length_beats: f64,
}

/// A decoded audio buffer ready for playback.
/// The samples are interleaved f32 at the output sample rate.
#[derive(Debug, Clone)]
pub struct DecodedBuffer {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub channels: u16,
    pub duration_sec: f64,
    /// BPM detected from analysis (for beat grid alignment).
    pub bpm: Option<f64>,
    /// Beat grid for this source (optional, for quantized launch).
    pub beat_grid: Option<BeatGridCompact>,
}

/// Compact beat grid stored with each source for quantized launch.
#[derive(Debug, Clone)]
pub struct BeatGridCompact {
    pub bpm: f64,
    pub first_beat_sec: f64,
    pub meter_numerator: i32,
    pub downbeat_offset: usize,
}

/// A bounded lock-free command queue.
/// Uses crossbeam's ArrayQueue which is lock-free (CAS-based).
pub struct CommandQueue {
    queue: crossbeam_queue::ArrayQueue<QueuedCommand>,
    dropped_count: AtomicU64,
}

impl CommandQueue {
    pub fn new(capacity: usize) -> Self {
        Self {
            queue: crossbeam_queue::ArrayQueue::new(capacity),
            dropped_count: AtomicU64::new(0),
        }
    }

    /// Push a command. Called from the UI thread.
    /// Returns false if the queue was full (command dropped).
    pub fn push(&self, cmd: EngineCommand) -> bool {
        self.push_queued(QueuedCommand {
            command_id: None,
            command: cmd,
        })
    }

    pub fn push_tracked(&self, command_id: CommandId, command: EngineCommand) -> bool {
        self.push_queued(QueuedCommand {
            command_id: Some(command_id),
            command,
        })
    }

    fn push_queued(&self, queued: QueuedCommand) -> bool {
        match self.queue.push(queued) {
            Ok(()) => true,
            Err(_) => {
                self.dropped_count.fetch_add(1, Ordering::Relaxed);
                false
            }
        }
    }

    /// Pop a command. Called from the audio callback.
    pub fn pop(&self) -> Option<EngineCommand> {
        self.queue.pop().map(|queued| queued.command)
    }

    pub(crate) fn pop_queued(&self) -> Option<QueuedCommand> {
        self.queue.pop()
    }

    /// Number of dropped commands (for UI diagnostics).
    pub fn dropped_count(&self) -> u64 {
        self.dropped_count.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn player_id_index() {
        assert_eq!(PlayerId(0).as_index(), 0);
        assert_eq!(PlayerId(7).as_index(), 7);
    }

    #[test]
    fn command_queue_push_pop() {
        let q = CommandQueue::new(4);
        assert!(q.push(EngineCommand::Stop { player: PlayerId(0), at_frame: 100 }));
        assert!(q.push(EngineCommand::Stop { player: PlayerId(1), at_frame: 200 }));
        let cmd = q.pop().unwrap();
        assert!(matches!(cmd, EngineCommand::Stop { player: PlayerId(0), at_frame: 100 }));
        let cmd = q.pop().unwrap();
        assert!(matches!(cmd, EngineCommand::Stop { player: PlayerId(1), at_frame: 200 }));
        assert!(q.pop().is_none());
    }

    #[test]
    fn command_queue_overflow() {
        let q = CommandQueue::new(2);
        assert!(q.push(EngineCommand::Shutdown));
        assert!(q.push(EngineCommand::Shutdown));
        assert!(!q.push(EngineCommand::Shutdown)); // full
        assert_eq!(q.dropped_count(), 1);
    }

    #[test]
    fn tracked_queue_entry_preserves_command_identity() {
        let q = CommandQueue::new(1);
        assert!(q.push_tracked(CommandId(9), EngineCommand::Shutdown));
        let queued = q.pop_queued().unwrap();
        assert_eq!(queued.command_id, Some(CommandId(9)));
        assert!(matches!(queued.command, EngineCommand::Shutdown));
    }
}
