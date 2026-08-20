// Audio engine module — authoritative playback for the Transition Workbench.
//
// Architecture: Native Rust engine on CPAL. One output stream, one frame
// counter, lock-free command queue, atomic meter snapshots. The callback
// owns its state and never allocates, locks, or does I/O.
//
// DSP priority: sample-accurate transport → resampling → beat grid →
// pitch-preserving time stretch → mixing/metering → stems.

pub mod command;
pub mod deck;
pub mod engine;
pub mod eq;
pub mod meter;
pub mod ring_buffer;
pub mod worker;

pub use command::{CommandQueue, EngineCommand, DeckId, EqBand, LoopRegion, DecodedBuffer};
pub use engine::AudioEngine;
pub use meter::{MeterSnapshot, MeterReadout};
pub use ring_buffer::{RingBufferConsumer, RingBufferProducer, create_ring_buffer};
pub use worker::{decode_file, DecodeResult};
