// Audio engine module — authoritative playback for the Transition Workbench.
//
// Architecture: Native Rust engine on CPAL. One output stream, one frame
// counter, lock-free command queue, atomic meter snapshots. The callback
// owns its state and never allocates, locks, or does I/O.
//
// Engine vocabulary:
//   - MAX_PLAYERS (8) player slots
//   - Two buses (A, B) feeding a crossfader, plus direct-to-master
//   - Frame-addressed commands with real scheduling (not block-boundary)
//   - Source handles instead of raw buffers in commands
//
// DSP priority: sample-accurate transport → resampling → beat grid →
// pitch-preserving time stretch → mixing/metering → stems.

pub mod bus;
pub mod command;
pub mod engine;
pub mod eq;
pub mod filter;
#[cfg(test)]
pub mod dsp_char;
pub mod intelligence;
pub mod io;
pub mod meter;
pub mod player;
#[cfg(test)]
pub mod qg_pb2;
#[cfg(test)]
pub mod perf_harness;
#[cfg(test)]
pub mod rt_audit;
#[cfg(test)]
pub mod sync_tests;
pub mod ring_buffer;
pub mod timepitch;
pub mod worker;

pub use bus::Bus;
pub use command::{
    BusId, CommandQueue, EngineCommand, EqBand, FilterModeParam, LoopRegion,
    DecodedBuffer, BeatGridCompact, PlayerId, SourceHandle, Quantize,
    MAX_PLAYERS,
};
pub use engine::AudioEngine;
pub use filter::{FilterMode, TuneLockFilter};
pub use intelligence::{
    ConfidenceTier, KeyAlternative, TrackIntelligenceSnapshot,
    SNAPSHOT_SCHEMA_VERSION, compatibility,
};
pub use meter::{MeterSnapshot, MeterReadout, PlayerMeterReadout};
pub use player::Player;
pub use ring_buffer::{RingBufferConsumer, RingBufferProducer, create_ring_buffer};
pub use timepitch::{TimePitchProcessor, default_processor};
pub use worker::{decode_file, DecodeResult};
