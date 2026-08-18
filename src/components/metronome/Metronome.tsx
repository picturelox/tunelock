import { useEffect, useRef, useState } from 'react';
import { Play, Pause } from 'lucide-react';

/**
 * Lightweight click-track metronome.
 *
 * Design choices:
 * - Pure Web Audio (oscillator + tight gain envelope), no samples.
 * - Two distinct click pitches: a higher "downbeat" on beat 1, a softer
 *   tick on the rest of the beats. Helps the ear lock onto the meter.
 * - The scheduler runs in a 25 ms interval and pre-schedules clicks ~150 ms
 *   ahead of the audio clock. This is the standard MDN-recommended pattern
 *   for jitter-free Web Audio metronomes; setInterval alone drifts.
 * - BPM is owned by the parent so the Tuner can pre-fill it with the
 *   detected tempo of the analysed track.
 */
export interface MetronomeProps {
  /** Initial BPM. Tuner passes the detected tempo here. */
  initialBpm?: number;
  /** Min / max BPM allowed by the slider. */
  minBpm?: number;
  maxBpm?: number;
}

export default function Metronome({
  initialBpm = 120,
  minBpm = 40,
  maxBpm = 240,
}: MetronomeProps) {
  const [bpm, setBpm] = useState(Math.round(clamp(initialBpm, minBpm, maxBpm)));
  const [beatsPerBar, setBeatsPerBar] = useState(4);
  const [running, setRunning] = useState(false);
  /** Live beat indicator (1..beatsPerBar). 0 means not running. */
  const [currentBeat, setCurrentBeat] = useState(0);

  // === Web Audio scheduling state ==========================================
  const audioCtxRef = useRef<AudioContext | null>(null);
  const nextNoteTimeRef = useRef(0);
  const nextBeatRef = useRef(1);
  const schedulerIdRef = useRef<number | null>(null);

  // Mirror state into refs so the running scheduler picks up live changes
  // without having to re-create itself on every BPM tweak.
  const bpmRef = useRef(bpm);
  const beatsPerBarRef = useRef(beatsPerBar);
  useEffect(() => { bpmRef.current = bpm; }, [bpm]);
  useEffect(() => { beatsPerBarRef.current = beatsPerBar; }, [beatsPerBar]);

  // Update internal BPM if the parent passes a new initialBpm (e.g. a new
  // track is analysed). We treat initialBpm as a *suggestion* and only
  // adopt it when the user hasn't recently overridden it.
  useEffect(() => {
    if (!running) {
      setBpm(Math.round(clamp(initialBpm, minBpm, maxBpm)));
    }
  }, [initialBpm, minBpm, maxBpm, running]);

  const getCtx = (): AudioContext => {
    if (!audioCtxRef.current) {
      const Ctor = window.AudioContext || (window as any).webkitAudioContext;
      audioCtxRef.current = new Ctor();
    }
    return audioCtxRef.current;
  };

  /**
   * Schedule a click at audio-clock `time`. `accent` triggers a higher pitch.
   * Tight 0.05 s envelope = perceptually a "tick" rather than a tone.
   */
  const scheduleClick = (time: number, accent: boolean) => {
    const ctx = getCtx();
    const osc = ctx.createOscillator();
    const gain = ctx.createGain();
    osc.frequency.value = accent ? 1500 : 900;
    osc.type = 'square';
    gain.gain.setValueAtTime(0.0001, time);
    gain.gain.exponentialRampToValueAtTime(accent ? 0.4 : 0.22, time + 0.001);
    gain.gain.exponentialRampToValueAtTime(0.0001, time + 0.05);
    osc.connect(gain).connect(ctx.destination);
    osc.start(time);
    osc.stop(time + 0.06);
  };

  /**
   * Lookahead-based scheduler. Re-runs every 25 ms; whenever the audio
   * clock is within ~150 ms of the next beat, schedule it and advance.
   */
  const scheduler = () => {
    const ctx = getCtx();
    const lookahead = 0.15; // seconds
    while (nextNoteTimeRef.current < ctx.currentTime + lookahead) {
      const beat = nextBeatRef.current;
      const accent = beat === 1;
      scheduleClick(nextNoteTimeRef.current, accent);

      // Defer the visual tick to fire at audio-clock time.
      const delayMs = Math.max(0, (nextNoteTimeRef.current - ctx.currentTime) * 1000);
      window.setTimeout(() => setCurrentBeat(beat), delayMs);

      // Advance.
      const secondsPerBeat = 60.0 / bpmRef.current;
      nextNoteTimeRef.current += secondsPerBeat;
      nextBeatRef.current = (beat % beatsPerBarRef.current) + 1;
    }
  };

  const start = () => {
    const ctx = getCtx();
    if (ctx.state === 'suspended') ctx.resume().catch(() => {});
    nextNoteTimeRef.current = ctx.currentTime + 0.06;
    nextBeatRef.current = 1;
    schedulerIdRef.current = window.setInterval(scheduler, 25);
    setRunning(true);
  };

  const stop = () => {
    if (schedulerIdRef.current !== null) {
      window.clearInterval(schedulerIdRef.current);
      schedulerIdRef.current = null;
    }
    setRunning(false);
    setCurrentBeat(0);
  };

  // Cleanup on unmount.
  useEffect(() => {
    return () => {
      if (schedulerIdRef.current !== null) {
        window.clearInterval(schedulerIdRef.current);
      }
    };
  }, []);

  // Tap-tempo: average the last 4 inter-tap intervals.
  const tapTimes = useRef<number[]>([]);
  const handleTap = () => {
    const now = performance.now();
    tapTimes.current.push(now);
    if (tapTimes.current.length > 4) tapTimes.current.shift();
    if (tapTimes.current.length >= 2) {
      const intervals: number[] = [];
      for (let i = 1; i < tapTimes.current.length; i++) {
        intervals.push(tapTimes.current[i] - tapTimes.current[i - 1]);
      }
      const avgMs = intervals.reduce((a, b) => a + b, 0) / intervals.length;
      const detected = 60000 / avgMs;
      setBpm(Math.round(clamp(detected, minBpm, maxBpm)));
    }
  };

  return (
    <div className="bg-surface/40 rounded-xl p-4">
      <div className="flex items-center justify-between mb-3">
        <h3 className="text-sm font-semibold">Metronome</h3>
        <span className="text-[10px] text-text-secondary">
          Tap or set BPM, hit play
        </span>
      </div>

      {/* Beat indicator dots */}
      <div className="flex items-center justify-center gap-2 mb-4 h-8">
        {Array.from({ length: beatsPerBar }).map((_, i) => {
          const beat = i + 1;
          const active = running && currentBeat === beat;
          const isDownbeat = beat === 1;
          return (
            <div
              key={beat}
              className={`
                w-5 h-5 rounded-full border-2 transition-all duration-75
                ${active
                  ? isDownbeat
                    ? 'bg-accent-primary border-accent-primary scale-125'
                    : 'bg-accent-primary/60 border-accent-primary/60 scale-110'
                  : 'border-white/15 bg-transparent'
                }
              `}
            />
          );
        })}
      </div>

      {/* BPM big readout */}
      <div className="flex items-center justify-center gap-3 mb-4">
        <span className="text-4xl font-bold tabular-nums">{bpm}</span>
        <span className="text-xs text-text-secondary">BPM</span>
      </div>

      {/* BPM slider */}
      <input
        type="range"
        min={minBpm}
        max={maxBpm}
        value={bpm}
        onChange={(e) => setBpm(parseInt(e.target.value, 10))}
        className="w-full mb-3 accent-purple-400"
      />

      {/* Beats-per-bar selector */}
      <div className="flex items-center gap-2 mb-3 text-xs">
        <span className="text-text-secondary">Beats/bar</span>
        {[2, 3, 4, 6].map((n) => (
          <button
            key={n}
            onClick={() => setBeatsPerBar(n)}
            className={`
              px-2 py-1 rounded transition-colors
              ${beatsPerBar === n
                ? 'bg-accent-primary text-white'
                : 'bg-surface-light text-text-secondary hover:bg-white/10'
              }
            `}
          >
            {n}
          </button>
        ))}
      </div>

      {/* Controls */}
      <div className="flex items-center gap-2">
        <button
          onClick={running ? stop : start}
          className={`
            flex items-center gap-1.5 px-3 py-2 rounded-md text-sm font-medium transition-colors
            ${running
              ? 'bg-red-500/80 hover:bg-red-500 text-white'
              : 'bg-accent-primary text-white hover:opacity-90'
            }
          `}
        >
          {running ? <Pause className="w-4 h-4" /> : <Play className="w-4 h-4" />}
          {running ? 'Stop' : 'Start'}
        </button>
        <button
          onClick={handleTap}
          className="px-3 py-2 rounded-md bg-surface-light hover:bg-white/10 text-sm"
          title="Tap 2-4 times to set BPM by ear"
        >
          Tap
        </button>
        <button
          onClick={() => { tapTimes.current = []; }}
          className="px-2 py-2 rounded-md hover:bg-white/5 text-xs text-text-secondary"
          title="Reset tap-tempo memory"
        >
          reset tap
        </button>
      </div>
    </div>
  );
}

function clamp(v: number, lo: number, hi: number): number {
  return Math.max(lo, Math.min(hi, v));
}
