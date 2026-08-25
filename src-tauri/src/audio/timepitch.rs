// TimePitchProcessor — the replaceable time/pitch abstraction.
//
// The Player never talks to a specific stretching algorithm. It talks to
// this trait. An implementation can be swapped (varispeed today, a
// pitch-preserving stretcher later) without rebuilding the engine.
//
// Contract:
//   - Pull model: the player calls next_frame() once per output frame.
//   - The processor owns its source position internally.
//   - tempo_ratio and pitch_semitones are independent controls; how they
//     interact is the implementation's business (varispeed couples them;
//     a phase vocoder would not).
//   - latency_frames() reports algorithmic delay so the engine can
//     compensate. Varispeed: 0. A block-based stretcher: its window size.
//   - reset() clears all filter/FFT state (called on seek and loop wrap).

use std::sync::Arc;

use super::command::DecodedBuffer;

/// Replaceable time/pitch processor. One instance per player.
pub trait TimePitchProcessor: Send {
    /// Set the playback rate (1.0 = original speed). Ramped by the player.
    fn set_tempo_ratio(&mut self, ratio: f64);
    /// Set independent pitch shift in semitones (0.0 = none).
    fn set_pitch_semitones(&mut self, semitones: f64);
    /// Current tempo ratio.
    fn tempo_ratio(&self) -> f64;
    /// Current pitch shift in semitones.
    fn pitch_semitones(&self) -> f64;
    /// Algorithmic latency in output frames (for latency compensation).
    fn latency_frames(&self) -> usize;
    /// Attach a source and set the starting position (source frames).
    /// Returns the previous source if one was attached, so the caller can
    /// defer its destruction to a non-realtime thread (avoiding large
    /// Vec<f32> deallocation inside the audio callback).
    fn set_source(&mut self, source: Arc<DecodedBuffer>, start_frame: f64) -> Option<Arc<DecodedBuffer>>;
    /// Produce the next output frame (left, right). None at end of source.
    fn next_frame(&mut self) -> Option<(f64, f64)>;
    /// Current position in source frames (for UI position and loop logic).
    fn position_frames(&self) -> f64;
    /// Seek to an absolute source frame.
    fn seek_frames(&mut self, frame: f64);
    /// Clear all internal filter/history state (call on seek, loop wrap).
    fn reset(&mut self);
}

/// Create the default processor. Currently varispeed with cubic
/// interpolation; a pitch-preserving implementation can replace this
/// without touching Player.
pub fn default_processor() -> Box<dyn TimePitchProcessor> {
    Box::new(VarispeedProcessor::new())
}

/// Varispeed processor: high-quality cubic (Catmull-Rom) interpolation with
/// pitch coupled to tempo, plus an optional additional semitone shift that
/// compounds the read rate. Zero latency. This is the honest starting
/// engine — a pitch-preserving Master Tempo implementation will follow the
/// same trait.
pub struct VarispeedProcessor {
    source: Option<Arc<DecodedBuffer>>,
    position: f64,
    tempo: f64,
    semitones: f64,
}

impl VarispeedProcessor {
    pub fn new() -> Self {
        Self {
            source: None,
            position: 0.0,
            tempo: 1.0,
            semitones: 0.0,
        }
    }

    /// Effective read rate: tempo compounded with pitch shift.
    #[inline]
    fn read_rate(&self) -> f64 {
        self.tempo * 2.0f64.powf(self.semitones / 12.0)
    }

    /// Catmull-Rom cubic interpolation of an interleaved buffer.
    /// `frames` is total source frames; `channels` is the channel count.
    #[inline]
    fn cubic(samples: &[f32], channels: usize, frame: f64, ch: usize) -> f64 {
        let frames = samples.len() / channels;
        if frames == 0 {
            return 0.0;
        }
        let f = frame.clamp(0.0, (frames - 1) as f64);
        let i = f.floor() as usize;
        let frac = f - i as f64;

        let idx = |n: isize| -> f64 {
            let n = n.clamp(0, (frames - 1) as isize) as usize;
            samples[n * channels + ch.min(channels - 1)] as f64
        };

        let p0 = idx(i as isize - 1);
        let p1 = idx(i as isize);
        let p2 = idx(i as isize + 1);
        let p3 = idx(i as isize + 2);

        // Catmull-Rom spline
        p1 + 0.5 * frac * (p2 - p0
            + frac * (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3
            + frac * (3.0 * (p1 - p2) + p3 - p0)))
    }
}

impl TimePitchProcessor for VarispeedProcessor {
    fn set_tempo_ratio(&mut self, ratio: f64) {
        self.tempo = ratio.clamp(0.25, 4.0);
    }

    fn set_pitch_semitones(&mut self, semitones: f64) {
        self.semitones = semitones.clamp(-24.0, 24.0);
    }

    fn tempo_ratio(&self) -> f64 {
        self.tempo
    }

    fn pitch_semitones(&self) -> f64 {
        self.semitones
    }

    fn latency_frames(&self) -> usize {
        0
    }

    fn set_source(&mut self, source: Arc<DecodedBuffer>, start_frame: f64) -> Option<Arc<DecodedBuffer>> {
        let old = self.source.take();
        self.source = Some(source);
        self.position = start_frame.max(0.0);
        old
    }

    fn next_frame(&mut self) -> Option<(f64, f64)> {
        let source = self.source.as_ref()?;
        let channels = source.channels as usize;
        let total_frames = source.samples.len() / channels;

        if self.position >= total_frames as f64 {
            return None;
        }

        let l = Self::cubic(&source.samples, channels, self.position, 0);
        let r = if channels >= 2 {
            Self::cubic(&source.samples, channels, self.position, 1)
        } else {
            l
        };

        self.position += self.read_rate();
        Some((l, r))
    }

    fn position_frames(&self) -> f64 {
        self.position
    }

    fn seek_frames(&mut self, frame: f64) {
        let max = self
            .source
            .as_ref()
            .map(|s| (s.samples.len() / s.channels as usize) as f64)
            .unwrap_or(0.0);
        self.position = frame.clamp(0.0, max);
    }

    fn reset(&mut self) {
        // Varispeed is stateless — nothing to clear.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine_buffer(freq: f64, sr: f64, frames: usize) -> Arc<DecodedBuffer> {
        let mut samples = Vec::with_capacity(frames * 2);
        for i in 0..frames {
            let t = i as f64 / sr;
            let v = (2.0 * std::f64::consts::PI * freq * t).sin() as f32;
            samples.push(v);
            samples.push(v);
        }
        Arc::new(DecodedBuffer {
            samples,
            sample_rate: sr as u32,
            channels: 2,
            duration_sec: frames as f64 / sr,
            bpm: None,
            beat_grid: None,
        })
    }

    #[test]
    fn unity_tempo_reproduces_source() {
        let buf = sine_buffer(440.0, 44100.0, 4410);
        let mut p = VarispeedProcessor::new();
        p.set_source(buf.clone(), 0.0);

        for i in 0..4410 {
            let (l, r) = p.next_frame().expect("should produce frames");
            let expected = buf.samples[i * 2] as f64;
            assert!(
                (l - expected).abs() < 1e-6,
                "unity tempo frame {i}: expected {expected}, got {l}"
            );
            assert!((l - r).abs() < 1e-12);
        }
        assert!(p.next_frame().is_none(), "end of source must yield None");
    }

    #[test]
    fn double_tempo_reads_every_other_sample() {
        // Constant ramp: sample[i] = i / 1000. At 2x tempo, output frame n
        // reads source position 2n exactly — cubic on a ramp is exact.
        let frames = 1000;
        let mut samples = Vec::with_capacity(frames * 2);
        for i in 0..frames {
            let v = i as f32 / 1000.0;
            samples.push(v);
            samples.push(v);
        }
        let buf = Arc::new(DecodedBuffer {
            samples,
            sample_rate: 44100,
            channels: 2,
            duration_sec: frames as f64 / 44100.0,
            bpm: None,
            beat_grid: None,
        });

        let mut p = VarispeedProcessor::new();
        p.set_tempo_ratio(2.0);
        p.set_source(buf, 0.0);

        for n in 0..100 {
            let (l, _) = p.next_frame().unwrap();
            let expected = (2 * n) as f64 / 1000.0;
            assert!(
                (l - expected).abs() < 1e-3,
                "2x tempo frame {n}: expected ~{expected}, got {l}"
            );
        }
        assert!((p.position_frames() - 200.0).abs() < 1e-9);
    }

    #[test]
    fn pitch_shift_compounds_read_rate() {
        let buf = sine_buffer(440.0, 44100.0, 4410);
        let mut p = VarispeedProcessor::new();
        p.set_tempo_ratio(1.0);
        p.set_pitch_semitones(12.0); // +1 octave = 2x read rate
        p.set_source(buf, 0.0);

        let _ = p.next_frame();
        let _ = p.next_frame();
        assert!(
            (p.position_frames() - 4.0).abs() < 1e-9,
            "+12 semitones at 1x tempo must read at 2x: position {}",
            p.position_frames()
        );
    }

    #[test]
    fn seek_and_reset() {
        let buf = sine_buffer(440.0, 44100.0, 4410);
        let mut p = VarispeedProcessor::new();
        p.set_source(buf, 0.0);
        p.seek_frames(100.5);
        assert!((p.position_frames() - 100.5).abs() < 1e-9);
        p.seek_frames(-50.0);
        assert_eq!(p.position_frames(), 0.0);
        p.reset(); // must not panic; varispeed is stateless
    }

    #[test]
    fn interpolation_is_smooth_between_samples() {
        // Half-frame position on a sine should interpolate, not truncate.
        let buf = sine_buffer(440.0, 44100.0, 4410);
        let mut p = VarispeedProcessor::new();
        p.set_source(buf.clone(), 10.5);

        let (l, _) = p.next_frame().unwrap();
        let s10 = buf.samples[20] as f64;
        let s11 = buf.samples[22] as f64;
        let midpoint = (s10 + s11) / 2.0;
        assert!(
            (l - midpoint).abs() < 0.02,
            "half-frame position should interpolate (~{midpoint}), got {l}"
        );
    }
}
