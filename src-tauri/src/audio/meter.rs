// Meter snapshots from the audio engine to the UI.
//
// The audio callback writes meter data to a shared atomic snapshot. The UI
// reads it at 20-30 Hz for display. This is a single-producer (callback)
// single-consumer (UI) pattern using atomics for each field.
//
// For floating-point values, we use AtomicU64 with bits transmutation
// (f64 → u64 → f64) to avoid locks. The values may be slightly inconsistent
// across fields (torn reads), but this is acceptable for meter display.

use std::sync::atomic::{AtomicU64, AtomicBool, Ordering};

use super::command::MAX_PLAYERS;

/// Helper: store f64 as bits in an AtomicU64.
#[inline]
fn store_f64(slot: &AtomicU64, value: f64) {
    slot.store(value.to_bits(), Ordering::Relaxed);
}

#[inline]
fn load_f64(slot: &AtomicU64) -> f64 {
    f64::from_bits(slot.load(Ordering::Relaxed))
}

/// Snapshot of the engine state for UI display.
/// Uses fixed-size arrays for MAX_PLAYERS players.
pub struct MeterSnapshot {
    // Transport
    pub playing: AtomicBool,
    pub current_frame: AtomicU64,

    // Per-player meters (indexed by player slot)
    player_playing: [AtomicBool; MAX_PLAYERS],
    player_position_sec: [AtomicU64; MAX_PLAYERS], // f64 bits
    player_rms: [AtomicU64; MAX_PLAYERS],          // f64 bits
    player_peak: [AtomicU64; MAX_PLAYERS],         // f64 bits
    player_clip: [AtomicBool; MAX_PLAYERS],

    // Bus meters
    bus_a_rms: AtomicU64,
    bus_a_peak: AtomicU64,
    bus_b_rms: AtomicU64,
    bus_b_peak: AtomicU64,

    // Master meters
    master_rms: AtomicU64,
    master_peak: AtomicU64,
    master_true_peak: AtomicU64,
    master_clip: AtomicBool,

    // Crossfader position (0.0 = full A, 1.0 = full B)
    crossfade_position: AtomicU64, // f64 bits

    // Diagnostics
    pub underruns: AtomicU64,
    pub commands_dropped: AtomicU64,
}

impl MeterSnapshot {
    pub fn new() -> Self {
        Self {
            playing: AtomicBool::new(false),
            current_frame: AtomicU64::new(0),
            player_playing: Default::default(),
            player_position_sec: Default::default(),
            player_rms: Default::default(),
            player_peak: Default::default(),
            player_clip: Default::default(),
            bus_a_rms: AtomicU64::new(0),
            bus_a_peak: AtomicU64::new(0),
            bus_b_rms: AtomicU64::new(0),
            bus_b_peak: AtomicU64::new(0),
            master_rms: AtomicU64::new(0),
            master_peak: AtomicU64::new(0),
            master_true_peak: AtomicU64::new(0),
            master_clip: AtomicBool::new(false),
            crossfade_position: AtomicU64::new((0.5f64).to_bits()),
            underruns: AtomicU64::new(0),
            commands_dropped: AtomicU64::new(0),
        }
    }

    // Write helpers (called from audio callback)

    pub fn write_player(&self, index: usize, playing: bool, position_sec: f64, rms: f64, peak: f64, clip: bool) {
        if index < MAX_PLAYERS {
            self.player_playing[index].store(playing, Ordering::Relaxed);
            store_f64(&self.player_position_sec[index], position_sec);
            store_f64(&self.player_rms[index], rms);
            store_f64(&self.player_peak[index], peak);
            self.player_clip[index].store(clip, Ordering::Relaxed);
        }
    }

    pub fn write_buses(&self, a_rms: f64, a_peak: f64, b_rms: f64, b_peak: f64) {
        store_f64(&self.bus_a_rms, a_rms);
        store_f64(&self.bus_a_peak, a_peak);
        store_f64(&self.bus_b_rms, b_rms);
        store_f64(&self.bus_b_peak, b_peak);
    }

    pub fn write_master(&self, rms: f64, peak: f64, true_peak: f64, clip: bool) {
        store_f64(&self.master_rms, rms);
        store_f64(&self.master_peak, peak);
        store_f64(&self.master_true_peak, true_peak);
        self.master_clip.store(clip, Ordering::Relaxed);
    }

    pub fn write_crossfade(&self, position: f64) {
        store_f64(&self.crossfade_position, position);
    }

    pub fn write_frame(&self, frame: u64) {
        self.current_frame.store(frame, Ordering::Relaxed);
    }

    // Read helpers (called from UI thread)

    pub fn read_player(&self, index: usize) -> PlayerMeterReadout {
        if index < MAX_PLAYERS {
            PlayerMeterReadout {
                playing: self.player_playing[index].load(Ordering::Relaxed),
                position_sec: load_f64(&self.player_position_sec[index]),
                rms: load_f64(&self.player_rms[index]),
                peak: load_f64(&self.player_peak[index]),
                clip: self.player_clip[index].load(Ordering::Relaxed),
            }
        } else {
            PlayerMeterReadout::default()
        }
    }
}

/// Per-player meter readout for the UI.
#[derive(Debug, Clone, Default)]
pub struct PlayerMeterReadout {
    pub playing: bool,
    pub position_sec: f64,
    pub rms: f64,
    pub peak: f64,
    pub clip: bool,
}

/// A readable snapshot of all meters for the UI.
/// Copied from the atomics at read time.
#[derive(Debug, Clone)]
pub struct MeterReadout {
    pub playing: bool,
    pub current_frame: u64,
    pub players: [PlayerMeterReadout; MAX_PLAYERS],
    pub bus_a_rms: f64,
    pub bus_a_peak: f64,
    pub bus_b_rms: f64,
    pub bus_b_peak: f64,
    pub master_rms: f64,
    pub master_peak: f64,
    pub master_true_peak: f64,
    pub master_clip: bool,
    pub crossfade_position: f64,
    pub underruns: u64,
    pub commands_dropped: u64,
}

impl Default for MeterReadout {
    fn default() -> Self {
        Self {
            playing: false,
            current_frame: 0,
            players: Default::default(),
            bus_a_rms: 0.0,
            bus_a_peak: 0.0,
            bus_b_rms: 0.0,
            bus_b_peak: 0.0,
            master_rms: 0.0,
            master_peak: 0.0,
            master_true_peak: 0.0,
            master_clip: false,
            crossfade_position: 0.5,
            underruns: 0,
            commands_dropped: 0,
        }
    }
}

impl MeterSnapshot {
    pub fn read_all(&self) -> MeterReadout {
        let mut players: [PlayerMeterReadout; MAX_PLAYERS] = Default::default();
        for i in 0..MAX_PLAYERS {
            players[i] = self.read_player(i);
        }

        MeterReadout {
            playing: self.playing.load(Ordering::Relaxed),
            current_frame: self.current_frame.load(Ordering::Relaxed),
            players,
            bus_a_rms: load_f64(&self.bus_a_rms),
            bus_a_peak: load_f64(&self.bus_a_peak),
            bus_b_rms: load_f64(&self.bus_b_rms),
            bus_b_peak: load_f64(&self.bus_b_peak),
            master_rms: load_f64(&self.master_rms),
            master_peak: load_f64(&self.master_peak),
            master_true_peak: load_f64(&self.master_true_peak),
            master_clip: self.master_clip.load(Ordering::Relaxed),
            crossfade_position: load_f64(&self.crossfade_position),
            underruns: self.underruns.load(Ordering::Relaxed),
            commands_dropped: self.commands_dropped.load(Ordering::Relaxed),
        }
    }
}
