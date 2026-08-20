// Transport — shared transport state and command definitions.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Instant;

pub enum TransportCommand {
    Play,
    Pause,
    Stop,
    Seek(f64),
    SetCrossfade(f32),
    SetTempoA(f32),
    SetTempoB(f32),
    SetGainA(f32),
    SetGainB(f32),
}

pub struct TransportState {
    playing: AtomicBool,
    start_time: Mutex<Option<Instant>>,
    drift_measuring: AtomicBool,
}

impl TransportState {
    pub fn new() -> Self {
        Self {
            playing: AtomicBool::new(false),
            start_time: Mutex::new(None),
            drift_measuring: AtomicBool::new(false),
        }
    }

    pub fn handle_command(&self, cmd: &TransportCommand) {
        match cmd {
            TransportCommand::Play => {
                self.playing.store(true, Ordering::Relaxed);
            }
            TransportCommand::Pause | TransportCommand::Stop => {
                self.playing.store(false, Ordering::Relaxed);
            }
            _ => {}
        }
    }

    pub fn set_start_time(&self, time: Instant) {
        *self.start_time.lock().unwrap() = Some(time);
    }

    pub fn is_playing(&self) -> bool {
        self.playing.load(Ordering::Relaxed)
    }

    pub fn start_drift_measurement(&self) {
        self.drift_measuring.store(true, Ordering::Relaxed);
    }

    pub fn stop_drift_measurement(&self) {
        self.drift_measuring.store(false, Ordering::Relaxed);
    }

    pub fn is_drift_measuring(&self) -> bool {
        self.drift_measuring.load(Ordering::Relaxed)
    }
}
