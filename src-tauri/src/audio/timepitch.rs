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

/// Create the default processor, pre-configured for the engine's sample
/// rate and channel count. Uses Signalsmith Stretch for pitch-preserving
/// time stretching (independent tempo and pitch). The Stretch instance is
/// fully configured HERE — no allocation occurs later in the audio callback.
pub fn default_processor(sample_rate: f64, channels: u16) -> Box<dyn TimePitchProcessor> {
    Box::new(SignalsmithProcessor::new(sample_rate as u32, channels))
}

/// Create a varispeed-only processor (reference/fallback). Pitch and
/// tempo are coupled. Zero latency. Used for testing and as a fallback.
pub fn varispeed_processor() -> Box<dyn TimePitchProcessor> {
    Box::new(VarispeedProcessor::new())
}

/// Create a bypass processor that reads directly from the source with no
/// time/pitch processing. Used by the Listening Lab as the true reference
/// path — the unprocessed original. Zero latency, sample-exact.
pub fn bypass_processor() -> Box<dyn TimePitchProcessor> {
    Box::new(BypassProcessor::new())
}

/// Bypass processor: reads samples directly from the source at the original
/// speed with no pitch or tempo modification. This is the true reference
/// path for the Listening Lab — what the listener compares Signalsmith
/// against. tempo_ratio and pitch_semitones are accepted but ignored.
pub struct BypassProcessor {
    source: Option<Arc<DecodedBuffer>>,
    position: f64,
}

impl BypassProcessor {
    pub fn new() -> Self {
        Self {
            source: None,
            position: 0.0,
        }
    }
}

impl TimePitchProcessor for BypassProcessor {
    fn set_tempo_ratio(&mut self, _ratio: f64) {}
    fn set_pitch_semitones(&mut self, _semitones: f64) {}
    fn tempo_ratio(&self) -> f64 { 1.0 }
    fn pitch_semitones(&self) -> f64 { 0.0 }
    fn latency_frames(&self) -> usize { 0 }

    fn set_source(&mut self, source: Arc<DecodedBuffer>, start_frame: f64) -> Option<Arc<DecodedBuffer>> {
        let old = self.source.take();
        self.source = Some(source);
        self.position = start_frame;
        old
    }

    fn next_frame(&mut self) -> Option<(f64, f64)> {
        let source = self.source.as_ref()?;
        let pos = self.position as usize;
        let channels = source.channels as usize;
        let total_samples = source.samples.len();
        if pos * channels + 1 >= total_samples {
            return None;
        }
        let l = source.samples[pos * channels] as f64;
        let r = source.samples[pos * channels + 1] as f64;
        self.position += 1.0;
        Some((l, r))
    }

    fn position_frames(&self) -> f64 {
        self.position
    }

    fn seek_frames(&mut self, frame: f64) {
        self.position = frame;
    }

    fn reset(&mut self) {
        // No internal state to clear — bypass has no filters or buffers.
    }
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
//   - The Stretch instance is configured at CONSTRUCTION time, never
//     inside the audio callback. set_source() only swaps the source
//     reference, resets, and pre-rolls — no allocation.
//   - All buffers (input block, output block, ring, pre-roll buffer)
//     are preallocated to maximum capacity at construction.
//   - process() does not allocate after construction.
//   - No locks, no I/O, no Tauri calls.
//   - The Stretch instance is Send (the colinmarc wrapper unsafely
//     implements Send because the C++ object has no thread-local state).
//
// Latency and audible timing:
//   - input_latency() + output_latency() frames of algorithmic delay.
//   - On launch/seek, Signalsmith's seek() method is used to pre-roll
//     the internal state so the first process() output is immediately
//     audible (not silence).
//   - position_frames() returns the AUDIBLE source position (what the
//     listener is hearing), not the input feed position. This is
//     critical for loop detection, waveform cursor, and beat sync.
//   - feed_position_frames() returns the internal read-ahead position.
//
// Tempo range:
//   - Clamped to 0.5x–2.0x (covers all realistic DJ use).
//   - Signalsmith sounds best in 0.75x–1.5x; extreme values are
//     permitted but may produce artifacts.

/// Output block size in frames. Small enough for low latency, large
/// enough for efficient processing. 256 frames ≈ 5.8ms at 44.1kHz.
const OUTPUT_BLOCK_FRAMES: usize = 256;

/// Ring buffer capacity in samples (interleaved). Must be a power of 2.
/// 16384 samples = 8192 stereo frames = 32 output blocks. Large enough
/// to absorb process() jitter without underrunning.
const RING_CAPACITY: usize = 16384;

/// Maximum supported tempo ratio. Input buffers are preallocated for
/// this ratio so process_block() never needs to resize.
const MAX_TEMPO: f64 = 2.0;

/// Minimum supported tempo ratio.
const MIN_TEMPO: f64 = 0.5;

/// Maximum input frames in a single block (at MAX_TEMPO).
/// 256 * 2 + 1 = 513. We round up to 520 for safety.
const MAX_INPUT_FRAMES: usize = (OUTPUT_BLOCK_FRAMES as f64 * MAX_TEMPO) as usize + 8;

pub struct SignalsmithProcessor {
    source: Option<Arc<DecodedBuffer>>,
    /// Input feed position: how far ahead we've read from the source
    /// and fed into Signalsmith. This is NOT what the listener hears.
    feed_position: f64,
    /// Audible position: the source frame the listener is currently
    /// hearing. Drives UI cursor, loop detection, and beat sync.
    /// Computed as: start_frame + output_frames_served * tempo
    audible_position: f64,
    /// The source frame where playback started (for audible position calc).
    start_frame: f64,
    /// Number of output frames served via next_frame().
    output_frames_served: u64,
    /// The Stretch instance, configured at construction time.
    stretch: signalsmith_stretch::Stretch,
    // Preallocated interleaved buffers (sized at construction, never resized)
    input_block: Vec<f32>,   // MAX_INPUT_FRAMES * channels
    output_block: Vec<f32>,  // OUTPUT_BLOCK_FRAMES * channels
    pre_roll_buffer: Vec<f32>, // for seek() pre-roll
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
    fractional_accum: f64,
    cached_latency: usize,
}

impl SignalsmithProcessor {
    /// Create a fully configured processor. The Stretch instance is
    /// constructed and configured HERE, before the audio stream starts.
    /// This is the only place allocation occurs. After construction,
    /// set_source(), seek_frames(), and process_block() never allocate.
    pub fn new(sample_rate: u32, channels: u16) -> Self {
        let mut stretch = signalsmith_stretch::Stretch::preset_default(
            channels as u32,
            sample_rate,
        );
        stretch.set_transpose_factor_semitones(0.0, None);
        let cached_latency = (stretch.input_latency() + stretch.output_latency()) as usize;

        let ch = channels as usize;
        let ring_mask = RING_CAPACITY - 1;

        Self {
            source: None,
            feed_position: 0.0,
            audible_position: 0.0,
            start_frame: 0.0,
            output_frames_served: 0,
            stretch,
            input_block: vec![0.0f32; MAX_INPUT_FRAMES * ch],
            output_block: vec![0.0f32; OUTPUT_BLOCK_FRAMES * ch],
            pre_roll_buffer: vec![0.0f32; cached_latency * ch * 2], // generous
            output_ring: vec![0.0f32; RING_CAPACITY],
            ring_read: 0,
            ring_write: 0,
            ring_mask,
            tempo: 1.0,
            semitones: 0.0,
            sample_rate,
            channels,
            source_exhausted: false,
            fractional_accum: 0.0,
            cached_latency,
        }
    }

    /// Pre-roll the stretcher using seek(). This feeds source content
    /// through the stretcher's internal pipeline without producing output,
    /// so that the first process() call immediately produces audible content.
    /// Called from set_source() and seek_frames() — never allocates.
    fn pre_roll(&mut self) {
        let source = match self.source.as_ref() {
            Some(s) => s,
            None => return,
        };
        let ch = self.channels as usize;
        let total_frames = source.samples.len() / ch;

        // Number of input frames to feed as pre-roll.
        // We need enough to fill the latency pipeline.
        let pre_roll_input_frames = (self.cached_latency as f64 * self.tempo) as usize + 1;
        let pre_roll_samples = pre_roll_input_frames * ch;

        // Ensure pre_roll_buffer is large enough (should be preallocated,
        // but clamp to avoid any chance of indexing error)
        let buf_len = self.pre_roll_buffer.len().min(pre_roll_samples);
        let frames_to_feed = buf_len / ch;

        // Read source content into pre_roll_buffer
        let start = self.feed_position as usize;
        for i in 0..frames_to_feed {
            let src_frame = start + i;
            if src_frame < total_frames {
                for c in 0..ch {
                    self.pre_roll_buffer[i * ch + c] = source.samples[src_frame * ch + c];
                }
            } else {
                for c in 0..ch {
                    self.pre_roll_buffer[i * ch + c] = 0.0;
                }
            }
        }

        // Feed through seek() — warms up internal state, no output produced
        self.stretch.seek(
            &self.pre_roll_buffer[..frames_to_feed * ch],
            self.tempo,
        );
    }

    /// Number of samples currently in the ring buffer.
    #[inline]
    fn ring_available(&self) -> usize {
        (self.ring_write.wrapping_sub(self.ring_read)) & self.ring_mask
    }

    /// Read one sample from the ring.
    #[inline]
    fn ring_pop(&mut self) -> f32 {
        let s = self.output_ring[self.ring_read];
        self.ring_read = (self.ring_read + 1) & self.ring_mask;
        s
    }

    /// Read a block from the source into input_block, then process through
    /// the stretcher and write output to the ring buffer.
    /// Returns the number of output samples written to the ring.
    fn process_block(&mut self) -> usize {
        let source = match self.source.as_ref() {
            Some(s) => s,
            None => return 0,
        };

        let ch = self.channels as usize;
        let total_source_frames = source.samples.len() / ch;

        // Calculate how many input frames to read for this output block.
        let needed_input_f = OUTPUT_BLOCK_FRAMES as f64 * self.tempo + self.fractional_accum;
        let input_frames = needed_input_f.floor() as usize;
        self.fractional_accum = needed_input_f - input_frames as f64;

        // Clamp to preallocated capacity (should never exceed with clamped tempo)
        let input_frames = input_frames.min(MAX_INPUT_FRAMES);

        // Read input from source (with zero-padding at end)
        let mut frames_read = 0;
        let start = self.feed_position as usize;
        for i in 0..input_frames {
            let src_frame = start + i;
            if src_frame < total_source_frames {
                for c in 0..ch {
                    self.input_block[i * ch + c] = source.samples[src_frame * ch + c];
                }
                frames_read += 1;
            } else {
                for c in 0..ch {
                    self.input_block[i * ch + c] = 0.0;
                }
            }
        }

        if frames_read == 0 && self.feed_position >= total_source_frames as f64 {
            self.source_exhausted = true;
            return 0;
        }

        // Advance feed position
        self.feed_position += input_frames as f64;

        // Process: input has `input_frames * channels` samples,
        // output has `OUTPUT_BLOCK_FRAMES * channels` samples.
        let input_len = input_frames * ch;
        let output_len = OUTPUT_BLOCK_FRAMES * ch;
        self.stretch.process(
            &self.input_block[..input_len],
            &mut self.output_block[..output_len],
        );

        // Write output to ring (inline to avoid borrow conflict, no allocation)
        let space = RING_CAPACITY - self.ring_available();
        if space >= output_len {
            let mask = self.ring_mask;
            let mut write_pos = self.ring_write;
            for i in 0..output_len {
                self.output_ring[write_pos] = self.output_block[i];
                write_pos = (write_pos + 1) & mask;
            }
            self.ring_write = write_pos;
        }

        output_len
    }

    /// Ensure the ring has at least `needed` samples.
    fn fill_ring(&mut self, needed: usize) {
        while self.ring_available() < needed && !self.source_exhausted {
            let written = self.process_block();
            if written == 0 {
                break;
            }
        }
    }

    /// Update the audible position based on output frames served.
    #[inline]
    fn update_audible_position(&mut self) {
        self.audible_position = self.start_frame + self.output_frames_served as f64 * self.tempo;
    }
}

impl TimePitchProcessor for SignalsmithProcessor {
    fn set_tempo_ratio(&mut self, ratio: f64) {
        self.tempo = ratio.clamp(MIN_TEMPO, MAX_TEMPO);
    }

    fn set_pitch_semitones(&mut self, semitones: f64) {
        self.semitones = semitones.clamp(-24.0, 24.0);
        self.stretch.set_transpose_factor_semitones(self.semitones as f32, None);
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
        self.feed_position = start_frame.max(0.0);
        self.start_frame = start_frame.max(0.0);
        self.audible_position = start_frame.max(0.0);
        self.output_frames_served = 0;
        self.source_exhausted = false;
        self.fractional_accum = 0.0;

        // Reset stretcher state for new source (no allocation — just clears
        // internal buffers)
        self.stretch.reset();

        // Clear ring
        self.ring_read = 0;
        self.ring_write = 0;

        // Pre-roll: warm up the stretcher so the first output is audible
        self.pre_roll();

        old
    }

    fn next_frame(&mut self) -> Option<(f64, f64)> {
        let ch = self.channels as usize;
        let needed = ch;  // one frame = `channels` samples

        // Ensure ring has enough data
        self.fill_ring(needed);

        if self.ring_available() < needed {
            // Source exhausted and ring drained
            return None;
        }

        let l = self.ring_pop() as f64;
        let r = if ch >= 2 {
            self.ring_pop() as f64
        } else {
            l
        };

        self.output_frames_served += 1;
        self.update_audible_position();

        Some((l, r))
    }

    /// Returns the AUDIBLE source position — what the listener is hearing.
    /// This drives UI cursor, loop detection, and beat sync.
    fn position_frames(&self) -> f64 {
        self.audible_position
    }

    fn seek_frames(&mut self, frame: f64) {
        let max = self
            .source
            .as_ref()
            .map(|s| (s.samples.len() / s.channels as usize) as f64)
            .unwrap_or(0.0);
        let clamped = frame.clamp(0.0, max);
        self.feed_position = clamped;
        self.start_frame = clamped;
        self.audible_position = clamped;
        self.output_frames_served = 0;
        self.source_exhausted = false;
        self.fractional_accum = 0.0;

        // Reset stretcher and clear ring
        self.stretch.reset();
        self.ring_read = 0;
        self.ring_write = 0;

        // Pre-roll at the new position
        self.pre_roll();
    }

    fn reset(&mut self) {
        self.stretch.reset();
        self.ring_read = 0;
        self.ring_write = 0;
        self.fractional_accum = 0.0;
        self.source_exhausted = false;
        self.output_frames_served = 0;
        self.start_frame = self.feed_position;
        self.audible_position = self.feed_position;

        // Pre-roll after reset
        self.pre_roll();
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
        SignalsmithProcessor::new(44100, 2)
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
