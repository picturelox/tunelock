// Commands from UI to the audio engine.
//
// All commands are frame-addressed: they specify the output frame at which
// they should take effect. The callback processes pending commands and
// applies them at the requested frame, not at block boundaries.
//
// The command queue is a bounded lock-free SPSC queue (crossbeam ArrayQueue).
// The UI thread is the producer; the audio callback is the consumer.
// If the queue is full, the command is dropped and the UI is notified via
// the meter snapshot (a `command_dropped` counter increments).

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
    /// The player starts playing at the quantized boundary nearest
    /// `at_frame`. `start_beat` is the position in the source in beats.
    Launch {
        player: PlayerId,
        at_frame: u64,
        source: SourceHandle,
        start_beat: f64,
        quantize: Quantize,
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
    /// Set player tempo rate (1.0 = original, 0.92-1.08 typical range).
    SetTempo {
        player: PlayerId,
        at_frame: u64,
        rate: f32,
    },
    /// Set player gain with a ramp to avoid clicks.
    SetGain {
        player: PlayerId,
        at_frame: u64,
        gain: f32,
        ramp_frames: u32,
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
    /// Set master gain.
    SetMasterGain {
        at_frame: u64,
        gain: f32,
    },
    /// Register a decoded source in the engine's source registry.
    /// Returns a SourceHandle that can be used in Launch commands.
    /// This is processed outside the audio callback (on the engine thread).
    RegisterSource {
        handle: SourceHandle,
        buffer: DecodedBuffer,
    },
    /// Unregister a source, freeing its memory.
    UnregisterSource {
        handle: SourceHandle,
    },
    /// Shutdown the engine.
    Shutdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EqBand {
    Low,
    Mid,
    High,
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
    queue: crossbeam_queue::ArrayQueue<EngineCommand>,
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
        match self.queue.push(cmd) {
            Ok(()) => true,
            Err(_) => {
                self.dropped_count.fetch_add(1, Ordering::Relaxed);
                false
            }
        }
    }

    /// Pop a command. Called from the audio callback.
    pub fn pop(&self) -> Option<EngineCommand> {
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
}
