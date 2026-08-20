import { formatCamelotBadge, type ScaleNote } from '../../lib/harmony';
import { playNote } from '../../lib/audio';
import PianoRoll from '../piano/PianoRoll';
import Metronome from '../metronome/Metronome';

export interface ExplorePanelProps {
  activeCamelot: string | null;
  activeStandard: string | null;
  activeScale: ScaleNote[];
  isHovered: boolean;
  bpm: number;
}

export default function ExplorePanel({
  activeCamelot,
  activeStandard,
  activeScale,
  isHovered,
  bpm,
}: ExplorePanelProps) {
  if (!activeCamelot || activeScale.length === 0) return null;
  const badge = formatCamelotBadge(activeCamelot);

  return (
    <div className="grid grid-cols-1 lg:grid-cols-3 gap-6">
      {/* Notes in this key */}
      <div className="bg-surface/40 rounded-2xl p-4">
        <div className="flex items-center justify-between mb-2">
          <h3 className="text-sm font-semibold">
            Notes in <span style={{ color: badge.color }}>{activeCamelot}</span>
          </h3>
          {isHovered && (
            <span className="text-[10px] uppercase tracking-wide text-accent-primary">
              hovered
            </span>
          )}
        </div>
        <div className="text-xs text-text-secondary mb-3">{activeStandard}</div>
        <div className="flex flex-wrap gap-2">
          {activeScale.map((n) => (
            <button
              key={n.midi}
              onClick={() => playNote({ frequency: n.frequency })}
              className="flex flex-col items-center px-3 py-2 rounded-lg bg-surface-light hover:bg-white/10 transition-colors min-w-[3.5rem]"
              title={`Play ${n.name}4 (${n.frequency.toFixed(2)} Hz)`}
            >
              <span className="text-[10px] text-text-secondary">{n.degree}</span>
              <span className="text-lg font-bold">{n.name}</span>
              {n.altName && (
                <span className="text-[9px] text-text-secondary">/ {n.altName}</span>
              )}
              <span className="text-[9px] font-mono text-text-secondary mt-0.5">
                {n.frequency.toFixed(1)} Hz
              </span>
            </button>
          ))}
        </div>
        <div className="text-[10px] text-text-secondary mt-3 leading-snug">
          Click any note to hear it. Hover a different key on the wheel to swap
          the scale.
        </div>
      </div>

      {/* Piano roll spans 2/3 */}
      <div className="lg:col-span-2 flex flex-col gap-6">
        <PianoRoll highlightedScale={activeScale} />
        <Metronome initialBpm={bpm} />
      </div>
    </div>
  );
}
