/**
 * Tiny in-browser tone generator for the Tuner's piano roll.
 *
 * Why not a sample library: the Tuner only needs to play a single note at a
 * time for ear-training, and we want zero extra dependencies. The Web Audio
 * oscillator + an ADSR envelope gives a clean enough "bell-ish" tone that the
 * user can compare against the track they just analysed.
 *
 * A single AudioContext is lazily created and reused.  We use `triangle`
 * waves because pure sines sound flute-like and saws sound harsh; triangles
 * sit in the middle and are the standard choice for a minimalist UI synth.
 */

let ctx: AudioContext | null = null;

function getContext(): AudioContext {
  if (!ctx) {
    // Some browsers gate audio creation behind a user gesture. The component
    // calls playNote only from click handlers so we're safe here.
    const Ctor = window.AudioContext || (window as any).webkitAudioContext;
    ctx = new Ctor();
  }
  return ctx;
}

export interface PlayOptions {
  /** Hz. */
  frequency: number;
  /** Total duration in ms. Defaults to 800. */
  durationMs?: number;
  /** Peak gain in 0..1. Defaults to 0.22 (mid volume, won't clip). */
  peakGain?: number;
}

/**
 * Play a single note at the given frequency with a piano-ish envelope.
 * Returns immediately; the note rings out asynchronously.
 */
export function playNote({ frequency, durationMs = 800, peakGain = 0.22 }: PlayOptions): void {
  const audio = getContext();
  // Browsers may suspend the context until the user interacts.
  if (audio.state === 'suspended') {
    audio.resume().catch(() => {});
  }

  const osc = audio.createOscillator();
  osc.type = 'triangle';
  osc.frequency.value = frequency;

  // Tiny lowpass to round off any harshness at higher pitches.
  const filter = audio.createBiquadFilter();
  filter.type = 'lowpass';
  filter.frequency.value = Math.max(2000, frequency * 6);

  const gain = audio.createGain();
  const now = audio.currentTime;
  const dur = durationMs / 1000;

  // ADSR-ish: 12 ms attack, 80 ms decay, sustain at 0.4 of peak, then release.
  gain.gain.setValueAtTime(0, now);
  gain.gain.linearRampToValueAtTime(peakGain, now + 0.012);
  gain.gain.exponentialRampToValueAtTime(peakGain * 0.4, now + 0.012 + 0.08);
  // exponentialRamp can't go to 0, ramp toward a tiny floor.
  gain.gain.exponentialRampToValueAtTime(0.0001, now + dur);

  osc.connect(filter);
  filter.connect(gain);
  gain.connect(audio.destination);

  osc.start(now);
  osc.stop(now + dur + 0.05);
}

/**
 * Play a sequence of MIDI notes ascending, useful for "preview this scale".
 * Each note overlaps slightly to feel legato instead of staccato.
 */
export function playSequence(
  midiNotes: number[],
  stepMs = 220,
  noteDurationMs = 600,
): void {
  midiNotes.forEach((midi, i) => {
    const freq = 440 * Math.pow(2, (midi - 69) / 12);
    setTimeout(() => playNote({ frequency: freq, durationMs: noteDurationMs }), i * stepMs);
  });
}
