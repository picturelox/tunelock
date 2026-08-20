// Mix bus — sums players assigned to it, applies EQ and gain.
//
// The engine has two buses (A, B) that feed a crossfader, plus a direct-to-master
// path. Each bus has a DJ isolator EQ and gain. The crossfader blends Bus A and
// Bus B into the master output.
//
// This preserves familiar DJ behavior (A/B crossfader) while allowing multiple
// players on either side.

use super::command::BusId;
use super::eq::DjIsolator;

/// Ramped gain to avoid clicks.
struct RampedGain {
    current: f64,
    target: f64,
    ramp_increment: f64,
}

impl RampedGain {
    fn new(sample_rate: f64) -> Self {
        Self {
            current: 1.0,
            target: 1.0,
            ramp_increment: 1.0 / (0.005 * sample_rate), // 5ms ramp
        }
    }

    fn set_target(&mut self, target: f64) {
        self.target = target;
    }

    fn set_ramp(&mut self, ramp_frames: u32) {
        if ramp_frames > 0 {
            self.ramp_increment = (self.target - self.current).abs() / ramp_frames as f64;
            if self.ramp_increment < 1e-12 {
                self.ramp_increment = 1e-12;
            }
        }
    }

    #[inline]
    fn tick(&mut self) -> f64 {
        if (self.current - self.target).abs() <= self.ramp_increment {
            self.current = self.target;
        } else if self.current < self.target {
            self.current += self.ramp_increment;
        } else {
            self.current -= self.ramp_increment;
        }
        self.current
    }
}

/// A mix bus with EQ and gain.
pub struct Bus {
    pub id: BusId,
    eq: DjIsolator,
    gain: RampedGain,
    // Accumulated sum from all players on this bus (per block)
    block_sum_l: f64,
    block_sum_r: f64,
    // Crossfade gain (set by the engine based on crossfader position)
    crossfade_gain: RampedGain,
    sample_rate: f64,
}

impl Bus {
    pub fn new(id: BusId, sample_rate: f64) -> Self {
        Self {
            id,
            eq: DjIsolator::new(sample_rate),
            gain: RampedGain::new(sample_rate),
            block_sum_l: 0.0,
            block_sum_r: 0.0,
            crossfade_gain: RampedGain::new(sample_rate),
            sample_rate,
        }
    }

    pub fn set_gain(&mut self, gain: f32) {
        self.gain.set_target(gain as f64);
    }

    pub fn set_eq_gain(&mut self, band: super::command::EqBand, gain_db: f32) {
        self.eq.set_gain_db(band, gain_db);
    }

    pub fn set_eq_kill(&mut self, band: super::command::EqBand, killed: bool) {
        self.eq.set_kill(band, killed);
    }

    pub fn set_crossfade_gain(&mut self, gain: f32) {
        self.crossfade_gain.set_target(gain as f64);
    }

    /// Accumulate a player's output into this bus.
    #[inline]
    pub fn accumulate(&mut self, l: f64, r: f64) {
        self.block_sum_l += l;
        self.block_sum_r += r;
    }

    /// Process the bus for one sample: apply EQ, gain, and crossfade.
    /// Returns (left, right) output. Resets the block sum.
    #[inline]
    pub fn process_sample(&mut self) -> (f64, f64) {
        let l = self.block_sum_l;
        let r = self.block_sum_r;
        self.block_sum_l = 0.0;
        self.block_sum_r = 0.0;

        // Apply bus EQ
        let (eq_l, eq_r) = self.eq.process(l, r);

        // Apply bus gain
        let g = self.gain.tick();

        // For Bus A/B, apply crossfade gain. For Master direct, crossfade = 1.0.
        let xf_g = match self.id {
            BusId::A | BusId::B => self.crossfade_gain.tick(),
            BusId::Master => 1.0,
        };

        (eq_l * g * xf_g, eq_r * g * xf_g)
    }

    pub fn reset_eq(&mut self) {
        self.eq.reset();
    }
}
