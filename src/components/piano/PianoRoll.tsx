import { useMemo } from 'react';
import { Play } from 'lucide-react';
import { playNote, playSequence } from '../../lib/audio';
import { midiToFrequency, PITCH_NAMES_SHARP } from '../../lib/camelot';
import type { ScaleNote } from '../../lib/camelot';

interface PianoRollProps {
  /** MIDI of the lowest key (default C4 = 60). */
  startMidi?: number;
  /** Number of octaves to render (default 2). */
  octaves?: number;
  /** Scale notes to highlight on the keyboard. Pitch-class match (any octave). */
  highlightedScale?: ScaleNote[];
  /**
   * Optional accent color for highlighted keys. Defaults to the app accent.
   * Used so the wheel's hovered-key color can carry over.
   */
  highlightColor?: string;
}

/**
 * A lightweight, dependency-free piano roll.
 *
 * - White keys laid out edge-to-edge; black keys overlaid at their proper
 *   horizontal offsets relative to the white-key row.
 * - Click any key to hear it (Web Audio triangle wave with ADSR).
 * - When `highlightedScale` is passed, keys whose pitch class is in the scale
 *   are tinted; degree numbers (1, \u266d3, etc.) are drawn on top.
 *
 * No samples, no external libs. Designed as a learning aid for the Tuner.
 */
export default function PianoRoll({
  startMidi = 60, // C4
  octaves = 2,
  highlightedScale,
  highlightColor = '#a78bfa',
}: PianoRollProps) {
  const totalSemitones = octaves * 12;

  // Build all keys with their layout info.
  const keys = useMemo(() => {
    type KeyInfo = {
      midi: number;
      pitchClass: number;
      name: string;
      isBlack: boolean;
      /** 0-based column index for white keys; null for black keys. */
      whiteIndex: number | null;
    };
    const blackPattern = [false, true, false, true, false, false, true, false, true, false, true, false]; // C..B
    const out: KeyInfo[] = [];
    let whiteIdx = 0;
    for (let i = 0; i < totalSemitones; i++) {
      const midi = startMidi + i;
      const pc = midi % 12;
      const isBlack = blackPattern[pc];
      out.push({
        midi,
        pitchClass: pc,
        name: PITCH_NAMES_SHARP[pc],
        isBlack,
        whiteIndex: isBlack ? null : whiteIdx,
      });
      if (!isBlack) whiteIdx++;
    }
    return out;
  }, [startMidi, totalSemitones]);

  const whiteKeyCount = keys.filter((k) => !k.isBlack).length;
  const whiteKeyWidth = 100 / whiteKeyCount; // percent
  const blackKeyWidth = whiteKeyWidth * 0.6;

  // Map pitch class -> degree label, for the highlight overlay.
  const degreeByPitchClass = useMemo(() => {
    const map = new Map<number, string>();
    if (highlightedScale) {
      for (const n of highlightedScale) {
        map.set(n.pitchClass, n.degree);
      }
    }
    return map;
  }, [highlightedScale]);

  const isHighlighted = (pc: number) => degreeByPitchClass.has(pc);

  const handlePlay = (midi: number) => {
    playNote({ frequency: midiToFrequency(midi) });
  };

  const handlePlayScale = () => {
    if (!highlightedScale || highlightedScale.length === 0) return;
    // Play tonic in current octave, then ascending scale, then octave-up tonic.
    const midis = highlightedScale.map((n) => n.midi);
    const last = midis[midis.length - 1];
    const seq = [...midis, last + (12 - (last - midis[0]))]; // top octave
    playSequence(seq, 200, 480);
  };

  return (
    <div className="bg-surface/40 rounded-xl p-4">
      <div className="flex items-center justify-between mb-3">
        <div>
          <h3 className="text-sm font-semibold">Piano</h3>
          <div className="text-[10px] text-text-secondary">
            Click any key to hear it.
            {highlightedScale && highlightedScale.length > 0
              ? ' Highlighted keys are in the scale.'
              : ' Hover the wheel to highlight a scale.'}
          </div>
        </div>
        {highlightedScale && highlightedScale.length > 0 && (
          <button
            onClick={handlePlayScale}
            className="flex items-center gap-1.5 px-3 py-1.5 bg-accent-primary text-white rounded-md text-xs hover:opacity-90"
            title="Play the scale top-to-bottom"
          >
            <Play className="w-3 h-3" />
            Play scale
          </button>
        )}
      </div>

      <div className="relative w-full h-32 select-none">
        {/* White keys */}
        <div className="absolute inset-0 flex">
          {keys
            .filter((k) => !k.isBlack)
            .map((k) => {
              const highlighted = isHighlighted(k.pitchClass);
              const degree = degreeByPitchClass.get(k.pitchClass);
              const isTonic = degree === '1';
              return (
                <button
                  key={k.midi}
                  onMouseDown={() => handlePlay(k.midi)}
                  className="relative flex-1 h-full border border-neutral-700 first:rounded-l-md last:rounded-r-md bg-neutral-100 hover:bg-neutral-200 active:bg-neutral-300 transition-colors"
                  style={
                    highlighted
                      ? {
                          backgroundColor: isTonic ? highlightColor : `${highlightColor}55`,
                          borderColor: highlightColor,
                        }
                      : undefined
                  }
                  title={`${k.name}${Math.floor(k.midi / 12) - 1} \u00b7 ${midiToFrequency(k.midi).toFixed(2)} Hz`}
                >
                  {highlighted && degree && (
                    <span
                      className="absolute inset-x-0 bottom-1 text-center text-[10px] font-bold pointer-events-none"
                      style={{ color: isTonic ? '#fff' : '#111' }}
                    >
                      {degree}
                    </span>
                  )}
                  <span className="absolute inset-x-0 bottom-4 text-center text-[9px] text-neutral-500 pointer-events-none">
                    {k.name}
                  </span>
                </button>
              );
            })}
        </div>

        {/* Black keys overlaid */}
        <div className="absolute inset-0 pointer-events-none">
          {keys
            .filter((k) => k.isBlack)
            .map((k) => {
              // Find the white key directly to the left to anchor the position.
              const whiteLeftIdx = keys
                .filter((x) => !x.isBlack && x.midi < k.midi)
                .length - 1;
              const leftPct = (whiteLeftIdx + 1) * whiteKeyWidth - blackKeyWidth / 2;
              const highlighted = isHighlighted(k.pitchClass);
              const degree = degreeByPitchClass.get(k.pitchClass);
              const isTonic = degree === '1';
              return (
                <button
                  key={k.midi}
                  onMouseDown={() => handlePlay(k.midi)}
                  className="absolute top-0 h-[60%] rounded-b-md border border-black bg-neutral-900 hover:bg-neutral-700 active:bg-neutral-600 pointer-events-auto transition-colors"
                  style={{
                    left: `${leftPct}%`,
                    width: `${blackKeyWidth}%`,
                    ...(highlighted
                      ? {
                          backgroundColor: isTonic ? highlightColor : `${highlightColor}aa`,
                          borderColor: highlightColor,
                        }
                      : {}),
                  }}
                  title={`${k.name}${Math.floor(k.midi / 12) - 1} \u00b7 ${midiToFrequency(k.midi).toFixed(2)} Hz`}
                >
                  {highlighted && degree && (
                    <span
                      className="absolute inset-x-0 bottom-1 text-center text-[10px] font-bold text-white pointer-events-none"
                    >
                      {degree}
                    </span>
                  )}
                </button>
              );
            })}
        </div>
      </div>
    </div>
  );
}
