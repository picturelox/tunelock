// Commands from UI to the audio engine.
//
// All commands are frame-addressed: they specify the output frame at which
// they should take effect. The callback processes pending commands at the
// start of each block and applies them at the requested frame.
//
// The command queue is a bounded lock-free SPSC queue (crossbeam ArrayQueue).
// The UI thread is the producer; the audio callback is the consumer.
// If the queue is full, the command is dropped and the UI is notified via
// the meter snapshot (a `command_dropped` counter increments).

use std::sync::atomic::{AtomicU64, Ordering};

/// Monotonically increasing output frame counter.
/// Shared between the callback (writer) and all other threads (readers).
pub type FrameCounter = AtomicU64;

/// Commands sent from the UI to the audio engine.
#[derive(Debug, Clone)]
pub enum EngineCommand {
    /// Start playback at the given output frame.
    Play { at_frame: u64 },
    /// Pause playback at the given output frame.
    Pause { at_frame: u64 },
    /// Stop playback and reset positions to zero.
    Stop { at_frame: u64 },
    /// Seek both decks to the given position in seconds.
    Seek { at_frame: u64, position_sec: f64 },
    /// Set deck A tempo (0.92–1.08). Applied with a ramp.
    SetTempoA { at_frame: u64, rate: f32 },
    /// Set deck B tempo.
    SetTempoB { at_frame: u64, rate: f32 },
    /// Set crossfader position (0.0 = full A, 1.0 = full B). Ramped.
    SetCrossfade { at_frame: u64, position: f32 },
    /// Set deck gain. Ramped.
    SetDeckGain { at_frame: u64, deck: DeckId, gain: f32 },
    /// Set EQ band gain in dB. Ramped.
    SetEqGain { at_frame: u64, deck: DeckId, band: EqBand, gain_db: f32 },
    /// Kill an EQ band (full cut).
    SetEqKill { at_frame: u64, deck: DeckId, band: EqBand, killed: bool },
    /// Set loop region in beats. None disables looping.
    SetLoop { at_frame: u64, loop_region: Option<LoopRegion> },
    /// Load a decoded buffer into a deck (sent after worker thread finishes decoding).
    LoadDeck { deck: DeckId, buffer: DecodedBuffer },
    /// Shutdown the engine.
    Shutdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeckId {
    A,
    B,
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
