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

/// Snapshot of the engine state for UI display.
pub struct MeterSnapshot {
    // Transport
    pub playing: AtomicBool,
    pub current_frame: AtomicU64,
    pub deck_a_position_sec: AtomicU64,  // f64 bits
    pub deck_b_position_sec: AtomicU64,
    // Deck meters (RMS, 0.0-1.0)
    pub deck_a_rms: AtomicU64,           // f64 bits
    pub deck_b_rms: AtomicU64,
    // Deck peak (0.0-1.0)
    pub deck_a_peak: AtomicU64,
    pub deck_b_peak: AtomicU64,
    // Master meters
    pub master_rms: AtomicU64,
    pub master_peak: AtomicU64,
    pub master_true_peak: AtomicU64,
    // Clip indicators
    pub deck_a_clip: AtomicBool,
    pub deck_b_clip: AtomicBool,
    pub master_clip: AtomicBool,
    // Diagnostics
    pub underruns: AtomicU64,
    pub commands_dropped: AtomicU64,
}

impl MeterSnapshot {
    pub fn new() -> Self {
        Self {
            playing: AtomicBool::new(false),
            current_frame: AtomicU64::new(0),
            deck_a_position_sec: AtomicU64::new(0),
            deck_b_position_sec: AtomicU64::new(0),
            deck_a_rms: AtomicU64::new(0),
            deck_b_rms: AtomicU64::new(0),
            deck_a_peak: AtomicU64::new(0),
            deck_b_peak: AtomicU64::new(0),
            master_rms: AtomicU64::new(0),
            master_peak: AtomicU64::new(0),
            master_true_peak: AtomicU64::new(0),
            deck_a_clip: AtomicBool::new(false),
            deck_b_clip: AtomicBool::new(false),
            master_clip: AtomicBool::new(false),
            underruns: AtomicU64::new(0),
            commands_dropped: AtomicU64::new(0),
        }
    }

    // Write helpers (called from audio callback)

    pub fn write_f64(&self, slot: &AtomicU64, value: f64) {
        slot.store(value.to_bits(), Ordering::Relaxed);
    }

    pub fn write_position(&self, deck_a: f64, deck_b: f64) {
        self.deck_a_position_sec.store(deck_a.to_bits(), Ordering::Relaxed);
        self.deck_b_position_sec.store(deck_b.to_bits(), Ordering::Relaxed);
    }

    pub fn write_meters(
        &self,
        a_rms: f64, a_peak: f64, a_clip: bool,
        b_rms: f64, b_peak: f64, b_clip: bool,
        m_rms: f64, m_peak: f64, m_true_peak: f64, m_clip: bool,
    ) {
        self.deck_a_rms.store(a_rms.to_bits(), Ordering::Relaxed);
        self.deck_a_peak.store(a_peak.to_bits(), Ordering::Relaxed);
        self.deck_a_clip.store(a_clip, Ordering::Relaxed);
        self.deck_b_rms.store(b_rms.to_bits(), Ordering::Relaxed);
        self.deck_b_peak.store(b_peak.to_bits(), Ordering::Relaxed);
        self.deck_b_clip.store(b_clip, Ordering::Relaxed);
        self.master_rms.store(m_rms.to_bits(), Ordering::Relaxed);
        self.master_peak.store(m_peak.to_bits(), Ordering::Relaxed);
        self.master_true_peak.store(m_true_peak.to_bits(), Ordering::Relaxed);
        self.master_clip.store(m_clip, Ordering::Relaxed);
    }

    // Read helpers (called from UI thread)

    pub fn read_f64(&self, slot: &AtomicU64) -> f64 {
        f64::from_bits(slot.load(Ordering::Relaxed))
    }

    pub fn read_position(&self) -> (f64, f64) {
        (
            f64::from_bits(self.deck_a_position_sec.load(Ordering::Relaxed)),
            f64::from_bits(self.deck_b_position_sec.load(Ordering::Relaxed)),
        )
    }
}

/// A readable snapshot for the UI. Copied from the atomics at read time.
#[derive(Debug, Clone, Default)]
pub struct MeterReadout {
    pub playing: bool,
    pub current_frame: u64,
    pub deck_a_position_sec: f64,
    pub deck_b_position_sec: f64,
    pub deck_a_rms: f64,
    pub deck_b_rms: f64,
    pub deck_a_peak: f64,
    pub deck_b_peak: f64,
    pub master_rms: f64,
    pub master_peak: f64,
    pub master_true_peak: f64,
    pub deck_a_clip: bool,
    pub deck_b_clip: bool,
    pub master_clip: bool,
    pub underruns: u64,
    pub commands_dropped: u64,
}

impl MeterSnapshot {
    pub fn read_all(&self) -> MeterReadout {
        MeterReadout {
            playing: self.playing.load(Ordering::Relaxed),
            current_frame: self.current_frame.load(Ordering::Relaxed),
            deck_a_position_sec: self.read_f64(&self.deck_a_position_sec),
            deck_b_position_sec: self.read_f64(&self.deck_b_position_sec),
            deck_a_rms: self.read_f64(&self.deck_a_rms),
            deck_b_rms: self.read_f64(&self.deck_b_rms),
            deck_a_peak: self.read_f64(&self.deck_a_peak),
            deck_b_peak: self.read_f64(&self.deck_b_peak),
            master_rms: self.read_f64(&self.master_rms),
            master_peak: self.read_f64(&self.master_peak),
            master_true_peak: self.read_f64(&self.master_true_peak),
            deck_a_clip: self.deck_a_clip.load(Ordering::Relaxed),
            deck_b_clip: self.deck_b_clip.load(Ordering::Relaxed),
            master_clip: self.master_clip.load(Ordering::Relaxed),
            underruns: self.underruns.load(Ordering::Relaxed),
            commands_dropped: self.commands_dropped.load(Ordering::Relaxed),
        }
    }
}
