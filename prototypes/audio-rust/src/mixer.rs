// Mixer — combines two decks with crossfader and master gain.

use std::sync::Arc;
use crate::deck::{AudioBuffer, Deck};
use crate::transport::TransportCommand;

pub struct Mixer {
    pub deck_a: Arc<Deck>,
    pub deck_b: Arc<Deck>,
    master_gain: std::sync::Mutex<f32>,
}

impl Mixer {
    pub fn new(buffer_a: AudioBuffer, buffer_b: AudioBuffer, _output_sample_rate: u32) -> Self {
        Self {
            deck_a: Arc::new(Deck::new(buffer_a)),
            deck_b: Arc::new(Deck::new(buffer_b)),
            master_gain: std::sync::Mutex::new(0.8), // Leave headroom
        }
    }

    pub fn handle_command(&self, cmd: &TransportCommand) {
        match cmd {
            TransportCommand::Play => {
                self.deck_a.play();
                self.deck_b.play();
            }
            TransportCommand::Pause => {
                self.deck_a.pause();
                self.deck_b.pause();
            }
            TransportCommand::Stop => {
                self.deck_a.stop();
                self.deck_b.stop();
            }
            TransportCommand::Seek(pos) => {
                self.deck_a.seek(*pos);
                self.deck_b.seek(*pos);
            }
            TransportCommand::SetCrossfade(val) => {
                // Equal-power crossfade
                let angle = val * std::f32::consts::PI / 2.0;
                let gain_a = angle.cos();
                let gain_b = angle.sin();
                self.deck_a.set_crossfade_gain(gain_a);
                self.deck_b.set_crossfade_gain(gain_b);
            }
            TransportCommand::SetTempoA(rate) => {
                self.deck_a.set_playback_rate(*rate);
            }
            TransportCommand::SetTempoB(rate) => {
                self.deck_b.set_playback_rate(*rate);
            }
            TransportCommand::SetGainA(gain) => {
                self.deck_a.set_gain(*gain);
            }
            TransportCommand::SetGainB(gain) => {
                self.deck_b.set_gain(*gain);
            }
        }
    }

    /// Process audio: mix both decks into the output buffer.
    pub fn process(&self, output: &mut [f32]) {
        // Zero the buffer first
        for s in output.iter_mut() {
            *s = 0.0;
        }

        // Process each deck (they add into the buffer)
        let channels = if output.len() >= 2 { 2 } else { 1 };
        self.deck_a.process(output, channels);
        self.deck_b.process(output, channels);

        // Apply master gain
        let master = *self.master_gain.lock().unwrap();
        for s in output.iter_mut() {
            *s *= master;
            // Soft clip to prevent harsh clipping
            if *s > 1.0 {
                *s = 1.0 - (1.0 - *s).abs().min(1.0) * 0.5;
            } else if *s < -1.0 {
                *s = -1.0 + (1.0 + *s).abs().min(1.0) * 0.5;
            }
        }
    }
}
