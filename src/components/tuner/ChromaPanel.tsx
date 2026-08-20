import {
  getScaleNotes,
  pitchClassFrequencies,
  PITCH_NAMES_SHARP,
} from '../../lib/harmony';
import { playNote } from '../../lib/audio';

export default function ChromaPanel({
  chroma,
  winnerCamelot,
}: {
  chroma: number[];
  winnerCamelot: string;
}) {
  const max = Math.max(...chroma, 1e-9);
  const refs = pitchClassFrequencies();
  const scaleNotes = getScaleNotes(winnerCamelot);
  const scalePitchClasses = new Set(scaleNotes.map((n) => n.pitchClass));
  const tonicPc = scaleNotes[0]?.pitchClass ?? -1;

  return (
    <div className="bg-surface/40 rounded-xl p-4">
      <h3 className="text-sm font-semibold mb-2">Chroma vector</h3>
      <div className="text-[10px] text-text-secondary mb-3 leading-snug">
        Average pitch-class energy across the track. Bars in the detected key
        are highlighted; the tonic bar is solid. The tonic should be tall — if
        it isn't, the key pick is suspect.
      </div>
      <div className="flex items-end gap-1 h-28">
        {chroma.map((v, i) => {
          const h = Math.max(2, (v / max) * 100);
          const inScale = scalePitchClasses.has(i);
          const isTonic = i === tonicPc;
          const color = isTonic
            ? '#a78bfa'
            : inScale
              ? '#a78bfa88'
              : '#7c7c8a55';
          return (
            <button
              key={i}
              onClick={() => playNote({ frequency: refs[i].frequency })}
              className="flex-1 flex flex-col items-center gap-1 hover:opacity-80"
              title={`Click to hear ${PITCH_NAMES_SHARP[i]} (${refs[i].frequency.toFixed(2)} Hz)`}
            >
              <div
                className="w-full rounded-t transition-colors"
                style={{ height: `${h}%`, backgroundColor: color }}
              />
              <div className="text-[10px] font-mono text-text-secondary">
                {PITCH_NAMES_SHARP[i]}
              </div>
              <div className="text-[8px] font-mono text-text-secondary/70 leading-none">
                {refs[i].frequency.toFixed(0)}
              </div>
            </button>
          );
        })}
      </div>
      <div className="text-[10px] text-text-secondary mt-3 leading-snug">
        Frequencies shown at octave 4. Click any bar to hear that pitch class.
      </div>
    </div>
  );
}
