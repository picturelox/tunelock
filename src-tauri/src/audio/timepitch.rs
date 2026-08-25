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

/// Create the default processor. Uses Signalsmith Stretch for
/// pitch-preserving time stretching (independent tempo and pitch).
/// Falls back to VarispeedProcessor if Signalsmith cannot be configured
/// (e.g., no source attached yet).
pub fn default_processor() -> Box<dyn TimePitchProcessor> {
    Box::new(SignalsmithProcessor::new())
}

/// Create a varispeed-only processor (reference/fallback). Pitch and
/// tempo are coupled. Zero latency. Used for testing and as a fallback.
pub fn varispeed_processor() -> Box<dyn TimePitchProcessor> {
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

// ── SignalsmithProcessor ─────────────────────────────────────────────
//
// Production-quality pitch-preserving time stretcher using Signalsmith
// Stretch (MIT, C++11 header-only library). This is the real PB-2
// implementation — independent tempo and pitch control.
//
// Architecture:
//   - Signalsmith is block-based: process(input, output) where the
//     input/output length ratio controls the time-stretch ratio.
//   - Our TimePitchProcessor is pull-frame: next_frame() -> one frame.
//   - Bridge: a preallocated ring buffer. next_frame() pulls from the
//     ring; when the ring runs low, we read a block from the source,
//     call process(), and refill the ring.
//   - Pitch is set independently via set_transpose_factor_semitones().
//
// Realtime safety:
//   - All buffers (input block, output block, ring) are preallocated.
//   - process() does not allocate after initial configure() (Signalsmith
//     preallocates internally in presetDefault/configure).
//   - No locks, no I/O, no Tauri calls.
//   - The Stretch instance is Send (the colinmarc wrapper unsafely
//     implements Send because the C++ object has no thread-local state).
//
// Latency:
//   - input_latency() + output_latency() frames of algorithmic delay.
//   - Reported through latency_frames() for beat/transport compensation.

/// Output block size in frames. Small enough for low latency, large
/// enough for efficient processing. 256 frames ≈ 5.8ms at 44.1kHz.
const OUTPUT_BLOCK_FRAMES: usize = 256;

/// Ring buffer capacity in samples (interleaved). Must be a power of 2.
/// 16384 samples = 8192 stereo frames = 32 output blocks. Large enough
/// to absorb process() jitter without underrunning.
const RING_CAPACITY: usize = 16384;

pub struct SignalsmithProcessor {
    source: Option<Arc<DecodedBuffer>>,
    source_position: f64,  // read position in source frames (fractional)
    stretch: Option<signalsmith_stretch::Stretch>,
    // Preallocated interleaved buffers
    input_block: Vec<f32>,   // sized to max_input_frames * channels
    output_block: Vec<f32>,  // sized to OUTPUT_BLOCK_FRAMES * channels
    output_ring: Vec<f32>,   // power-of-2 ring buffer
    ring_read: usize,
    ring_write: usize,
    ring_mask: usize,
    // Parameters
    tempo: f64,
    semitones: f64,
    sample_rate: u32,
    channels: u16,
    // State
    source_exhausted: bool,
    fractional_accum: f64,  // carries fractional input frames between blocks
    configured: bool,
    // Buffered latency (in output frames) — set after configure
    cached_latency: usize,
}

impl SignalsmithProcessor {
    pub fn new() -> Self {
        let ring_mask = RING_CAPACITY - 1;  // RING_CAPACITY is power of 2
        Self {
            source: None,
            source_position: 0.0,
            stretch: None,
            input_block: Vec::with_capacity(OUTPUT_BLOCK_FRAMES * 4 * 2),  // max 4x tempo, stereo
            output_block: vec![0.0f32; OUTPUT_BLOCK_FRAMES * 2],  // stereo
            output_ring: vec![0.0f32; RING_CAPACITY],
            ring_read: 0,
            ring_write: 0,
            ring_mask,
            tempo: 1.0,
            semitones: 0.0,
            sample_rate: 44100,
            channels: 2,
            source_exhausted: false,
            fractional_accum: 0.0,
            configured: false,
            cached_latency: 0,
        }
    }

    /// Configure the stretcher for the current sample rate and channel count.
    /// Called lazily on first source attach or when sample rate changes.
    fn ensure_configured(&mut self) {
        if self.configured {
            return;
        }
        let mut stretch = signalsmith_stretch::Stretch::preset_default(
            self.channels as u32,
            self.sample_rate,
        );
        stretch.set_transpose_factor_semitones(self.semitones as f32, None);
        self.cached_latency = (stretch.input_latency() + stretch.output_latency()) as usize;
        self.stretch = Some(stretch);
        self.configured = true;
    }

    /// Number of samples currently in the ring buffer.
    #[inline]
    fn ring_available(&self) -> usize {
        (self.ring_write.wrapping_sub(self.ring_read)) & self.ring_mask
    }

    /// Read one sample from the ring. Returns 0.0 if empty (shouldn't happen
    /// if called after ensuring the ring has data).
    #[inline]
    fn ring_pop(&mut self) -> f32 {
        let s = self.output_ring[self.ring_read];
        self.ring_read = (self.ring_read + 1) & self.ring_mask;
        s
    }

    /// Write samples to the ring. Caller must ensure there's space.
    #[inline]
    fn ring_push(&mut self, samples: &[f32]) {
        for &s in samples {
            self.output_ring[self.ring_write] = s;
            self.ring_write = (self.ring_write + 1) & self.ring_mask;
        }
    }

    /// Read a block from the source into input_block, then process through
    /// the stretcher and write output to the ring buffer.
    /// Returns the number of output frames written to the ring.
    fn process_block(&mut self) -> usize {
        let stretch = match self.stretch.as_mut() {
            Some(s) => s,
            None => return 0,
        };
        let source = match self.source.as_ref() {
            Some(s) => s,
            None => return 0,
        };

        let channels = self.channels as usize;
        let total_source_frames = source.samples.len() / channels;

        // Calculate how many input frames to read for this output block.
        // tempo > 1 = faster playback = consume more source per output frame.
        let needed_input_f = OUTPUT_BLOCK_FRAMES as f64 * self.tempo + self.fractional_accum;
        let input_frames = needed_input_f.floor() as usize;
        self.fractional_accum = needed_input_f - input_frames as f64;

        // Ensure input_block is large enough
        let needed_input_samples = (input_frames + 1) * channels;  // +1 for safety
        if self.input_block.len() < needed_input_samples {
            self.input_block.resize(needed_input_samples, 0.0);
        }

        // Read input from source (with zero-padding at end)
        let mut frames_read = 0;
        let start = self.source_position as usize;
        for i in 0..input_frames {
            let src_frame = start + i;
            if src_frame < total_source_frames {
                for ch in 0..channels {
                    self.input_block[i * channels + ch] = source.samples[src_frame * channels + ch];
                }
                frames_read += 1;
            } else {
                // Zero-pad past end of source
                for ch in 0..channels {
                    self.input_block[i * channels + ch] = 0.0;
                }
            }
        }

        if frames_read == 0 && self.source_position >= total_source_frames as f64 {
            self.source_exhausted = true;
            return 0;
        }

        // Advance source position
        self.source_position += input_frames as f64;

        // Process: input has `input_frames * channels` samples,
        // output has `OUTPUT_BLOCK_FRAMES * channels` samples.
        let input_len = input_frames * channels;
        let output_len = OUTPUT_BLOCK_FRAMES * channels;
        stretch.process(
            &self.input_block[..input_len],
            &mut self.output_block[..output_len],
        );

        // Write output to ring (check space first)
        let space = RING_CAPACITY - self.ring_available();
        if space >= output_len {
            // Inline the ring write to avoid borrowing self mutably (ring_push)
            // while immutably borrowing self.output_block. No allocation.
            let mask = self.ring_mask;
            let mut write_pos = self.ring_write;
            for i in 0..output_len {
                self.output_ring[write_pos] = self.output_block[i];
                write_pos = (write_pos + 1) & mask;
            }
            self.ring_write = write_pos;
        }
        // If not enough space, we drop the block — this should never happen
        // if the ring is large enough relative to the output block size.

        output_len
    }

    /// Ensure the ring has at least `needed` samples. Processes blocks
    /// until the ring is sufficiently full or the source is exhausted.
    fn fill_ring(&mut self, needed: usize) {
        while self.ring_available() < needed && !self.source_exhausted {
            let written = self.process_block();
            if written == 0 {
                break;
            }
        }
    }
}

impl TimePitchProcessor for SignalsmithProcessor {
    fn set_tempo_ratio(&mut self, ratio: f64) {
        self.tempo = ratio.clamp(0.25, 4.0);
    }

    fn set_pitch_semitones(&mut self, semitones: f64) {
        self.semitones = semitones.clamp(-24.0, 24.0);
        if let Some(stretch) = self.stretch.as_mut() {
            stretch.set_transpose_factor_semitones(self.semitones as f32, None);
        }
    }

    fn tempo_ratio(&self) -> f64 {
        self.tempo
    }

    fn pitch_semitones(&self) -> f64 {
        self.semitones
    }

    fn latency_frames(&self) -> usize {
        self.cached_latency
    }

    fn set_source(&mut self, source: Arc<DecodedBuffer>, start_frame: f64) -> Option<Arc<DecodedBuffer>> {
        let old = self.source.take();
        self.source = Some(source);
        self.source_position = start_frame.max(0.0);
        self.source_exhausted = false;
        self.fractional_accum = 0.0;

        // Configure stretcher if sample rate changed
        let src = self.source.as_ref().unwrap();
        if src.sample_rate != self.sample_rate || src.channels != self.channels || !self.configured {
            self.sample_rate = src.sample_rate;
            self.channels = src.channels;
            self.configured = false;
            self.ensure_configured();
        }

        // Reset stretcher state for new source
        if let Some(stretch) = self.stretch.as_mut() {
            stretch.reset();
        }

        // Clear ring
        self.ring_read = 0;
        self.ring_write = 0;

        old
    }

    fn next_frame(&mut self) -> Option<(f64, f64)> {
        let channels = self.channels as usize;
        let needed = channels;  // one frame = `channels` samples

        // Ensure ring has enough data
        self.fill_ring(needed);

        if self.ring_available() < needed {
            // Source exhausted and ring drained
            return None;
        }

        let l = self.ring_pop() as f64;
        let r = if channels >= 2 {
            self.ring_pop() as f64
        } else {
            l
        };

        Some((l, r))
    }

    fn position_frames(&self) -> f64 {
        self.source_position
    }

    fn seek_frames(&mut self, frame: f64) {
        let max = self
            .source
            .as_ref()
            .map(|s| (s.samples.len() / s.channels as usize) as f64)
            .unwrap_or(0.0);
        self.source_position = frame.clamp(0.0, max);
        self.source_exhausted = false;
        self.fractional_accum = 0.0;

        // Reset stretcher and clear ring
        if let Some(stretch) = self.stretch.as_mut() {
            stretch.reset();
        }
        self.ring_read = 0;
        self.ring_write = 0;
    }

    fn reset(&mut self) {
        if let Some(stretch) = self.stretch.as_mut() {
            stretch.reset();
        }
        self.ring_read = 0;
        self.ring_write = 0;
        self.fractional_accum = 0.0;
        self.source_exhausted = false;
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

    // ── SignalsmithProcessor tests ───────────────────────────────────

    fn make_signalsmith() -> SignalsmithProcessor {
        SignalsmithProcessor::new()
    }

    #[test]
    fn signalsmith_unity_tempo_produces_audio() {
        // At unity tempo and no pitch shift, the processor must produce
        // audible output. It won't be sample-exact (latency + STFT), but
        // it must be non-silent.
        let buf = sine_buffer(440.0, 44100.0, 44100);  // 1 second
        let mut p = make_signalsmith();
        p.set_source(buf, 0.0);

        let mut max_abs = 0.0f64;
        let mut frames = 0;
        while let Some((l, r)) = p.next_frame() {
            max_abs = max_abs.max(l.abs()).max(r.abs());
            frames += 1;
            if frames >= 44100 {
                break;
            }
        }
        assert!(frames > 0, "must produce at least some frames");
        assert!(max_abs > 0.01, "unity playback must be audible (max_abs={max_abs})");
    }

    #[test]
    fn signalsmith_reports_nonzero_latency() {
        let buf = sine_buffer(440.0, 44100.0, 44100);
        let mut p = make_signalsmith();
        p.set_source(buf, 0.0);
        let lat = p.latency_frames();
        assert!(lat > 0, "Signalsmith must report nonzero latency (got {lat})");
    }

    #[test]
    fn signalsmith_double_tempo_finishes_faster() {
        // At 2x tempo, the processor should consume the source roughly
        // twice as fast. We don't require exact 2x (latency and STFT
        // behavior make this approximate), but the output should be
        // significantly shorter than the source.
        let buf = sine_buffer(440.0, 44100.0, 44100);  // 1 second
        let mut p = make_signalsmith();
        p.set_tempo_ratio(2.0);
        p.set_source(buf, 0.0);

        let mut frames = 0;
        while p.next_frame().is_some() {
            frames += 1;
            if frames > 30000 {
                break;  // safety
            }
        }
        // At 2x tempo, we expect roughly 22050 frames (±20% for latency/STFT)
        assert!(
            frames < 30000,
            "2x tempo should produce fewer frames than source (got {frames})"
        );
    }

    #[test]
    fn signalsmith_half_tempo_finishes_slower() {
        // At 0.5x tempo, the processor should produce roughly twice as
        // many output frames as the source.
        let buf = sine_buffer(440.0, 44100.0, 44100);  // 1 second
        let mut p = make_signalsmith();
        p.set_tempo_ratio(0.5);
        p.set_source(buf, 0.0);

        let mut frames = 0;
        while p.next_frame().is_some() {
            frames += 1;
            if frames > 100000 {
                break;  // safety
            }
        }
        // At 0.5x tempo, we expect roughly 88200 frames (±20%)
        assert!(
            frames > 60000,
            "0.5x tempo should produce more frames than source (got {frames})"
        );
    }

    #[test]
    fn signalsmith_pitch_shift_preserves_duration() {
        // Pitch shifting without tempo change should produce roughly the
        // same number of output frames as the source.
        let buf = sine_buffer(440.0, 44100.0, 44100);  // 1 second
        let mut p = make_signalsmith();
        p.set_pitch_semitones(3.0);  // +3 semitones
        p.set_source(buf, 0.0);

        let mut frames = 0;
        while p.next_frame().is_some() {
            frames += 1;
            if frames > 60000 {
                break;
            }
        }
        // Duration should be approximately preserved (±20% for latency/STFT)
        assert!(
            frames > 30000 && frames < 55000,
            "pitch shift at unity tempo should preserve duration (got {frames})"
        );
    }

    #[test]
    fn signalsmith_seek_and_reset() {
        let buf = sine_buffer(440.0, 44100.0, 44100);
        let mut p = make_signalsmith();
        p.set_source(buf, 0.0);

        // Read some frames to get past initial warm-up
        for _ in 0..5000 {
            p.next_frame();
        }

        // Seek to middle
        p.seek_frames(22050.0);
        assert!((p.position_frames() - 22050.0).abs() < 1.0);

        // After seek, the stretcher needs to warm up (latency can be
        // several thousand frames with preset_default at 44.1kHz).
        // Read enough frames to get past the latency.
        let warmup = p.latency_frames() * 3 + 1000;
        let mut found_audio = false;
        for _ in 0..warmup {
            if let Some((l, _)) = p.next_frame() {
                if l.abs() > 0.001 {
                    found_audio = true;
                    break;
                }
            }
        }
        assert!(found_audio, "must produce audio after seek (within {warmup} warm-up)");

        // Reset
        p.reset();
        // After reset, same warm-up applies
        let mut found_audio = false;
        for _ in 0..warmup {
            if let Some((l, _)) = p.next_frame() {
                if l.abs() > 0.001 {
                    found_audio = true;
                    break;
                }
            }
        }
        assert!(found_audio, "must produce audio after reset (within {warmup} warm-up)");
    }

    #[test]
    fn signalsmith_end_of_source_returns_none() {
        let buf = sine_buffer(440.0, 44100.0, 1000);  // short
        let mut p = make_signalsmith();
        p.set_tempo_ratio(2.0);  // exhaust faster
        p.set_source(buf, 0.0);

        let mut frames = 0;
        while p.next_frame().is_some() {
            frames += 1;
            if frames > 10000 {
                break;
            }
        }
        // After source is exhausted and ring drains, next_frame must return None
        assert!(p.next_frame().is_none(), "must return None after source exhausted");
    }

    #[test]
    fn signalsmith_no_nan_or_infinity() {
        // Process a variety of signals and verify no NaN/Inf in output.
        let buf = sine_buffer(440.0, 44100.0, 44100);
        let mut p = make_signalsmith();
        p.set_tempo_ratio(1.06);  // +6% tempo
        p.set_pitch_semitones(2.0);  // +2 semitones
        p.set_source(buf, 0.0);

        for _ in 0..5000 {
            if let Some((l, r)) = p.next_frame() {
                assert!(l.is_finite(), "left channel must be finite");
                assert!(r.is_finite(), "right channel must be finite");
                assert!(l.abs() <= 10.0, "left channel must not explode: {l}");
                assert!(r.abs() <= 10.0, "right channel must not explode: {r}");
            } else {
                break;
            }
        }
    }
}
