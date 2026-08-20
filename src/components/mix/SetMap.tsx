// Set Map — the strategic view of the Mix Canvas.
//
// Shows the overall shape of the set: energy trajectory, key journey,
// tempo changes, and scene markers. This is TuneLock's clearest
// distinction from live DJ software — it answers "Where is this mix going?"
//
// The Set Map is the top level of the three-level workspace:
//   Set Map (strategic) → Layer Lab (exploratory) → Transition Workbench (precision)
//
// Design language: Walnut Console — the trajectory graph uses the
// charcoal data plane, framed by the walnut/bronze console shell.

import { useMixStore } from '../../stores/mixStore';
import { useLibraryStore } from '../../stores/libraryStore';
import { formatCamelotBadge } from '../../lib/harmony';

export default function SetMap() {
  const { project } = useMixStore();
  const { tracks } = useLibraryStore();
  const clips = project.clips;

  if (clips.length === 0) {
    return (
      <div className="walnut-frame p-6 flex items-center justify-center min-h-[120px]">
        <div className="text-center">
          <div className="engraved-label mb-2">Set Map</div>
          <div className="text-xs text-cream-label/40">
            Add tracks to the mix to see the energy trajectory, key journey, and scene markers.
          </div>
        </div>
      </div>
    );
  }

  // Compute per-clip data
  const clipData = clips.map((clip, i) => {
    const track = tracks.get(clip.trackId);
    return {
      clipId: clip.id,
      index: i,
      title: track?.title || track?.filename || 'Unknown',
      key_camelot: track?.key_camelot || null,
      bpm: track?.bpm || null,
      energy: track?.energy_level || null,
      color: track?.key_camelot ? formatCamelotBadge(track.key_camelot).color : '#666',
    };
  });

  // Energy trajectory: normalize energy 1-10 to 0-1
  const energyPoints = clipData.map((c, i) => ({
    x: (i / Math.max(clipData.length - 1, 1)) * 100,
    y: c.energy ? 100 - ((c.energy - 1) / 9) * 80 - 10 : 50,
    color: c.color,
    title: c.title,
  }));

  // BPM trajectory (values used for display in the strip below)

  return (
    <div className="walnut-frame p-3 flex flex-col gap-2">
      {/* Header */}
      <div className="flex items-center justify-between px-1">
        <span className="engraved-label">Set Map</span>
        <span className="text-[10px] text-cream-label/60">
          {clips.length} tracks · {clipData.filter(c => c.key_camelot).length} keyed
        </span>
      </div>

      {/* Energy trajectory graph */}
      <div className="data-plane p-2 relative" style={{ height: '80px' }}>
        <div className="absolute top-1 left-2 text-[9px] text-data-text-dim uppercase">Energy</div>
        <svg className="w-full h-full" viewBox="0 0 100 100" preserveAspectRatio="none">
          {/* Grid lines */}
          <line x1="0" y1="25" x2="100" y2="25" stroke="var(--data-grid)" strokeWidth="0.3" />
          <line x1="0" y1="50" x2="100" y2="50" stroke="var(--data-grid)" strokeWidth="0.3" />
          <line x1="0" y1="75" x2="100" y2="75" stroke="var(--data-grid)" strokeWidth="0.3" />

          {/* Energy line */}
          {energyPoints.length > 1 && (
            <polyline
              points={energyPoints.map(p => `${p.x},${p.y}`).join(' ')}
              fill="none"
              stroke="var(--brass-accent)"
              strokeWidth="0.8"
              opacity="0.8"
            />
          )}

          {/* Energy points */}
          {energyPoints.map((p, i) => (
            <g key={i}>
              <circle
                cx={p.x}
                cy={p.y}
                r="1.2"
                fill={p.color}
                stroke="var(--data-bg)"
                strokeWidth="0.3"
              />
            </g>
          ))}
        </svg>
      </div>

      {/* Key journey + BPM strip */}
      <div className="flex gap-2">
        {/* Key journey */}
        <div className="data-plane p-2 flex-1" style={{ minHeight: '40px' }}>
          <div className="text-[9px] text-data-text-dim uppercase mb-1">Key Journey</div>
          <div className="flex items-center gap-1 flex-wrap">
            {clipData.map((c, i) => (
              <div key={i} className="flex items-center gap-1">
                {i > 0 && <span className="text-data-text-dim text-[10px]">→</span>}
                <span
                  className="text-[10px] font-mono px-1.5 py-0.5 rounded"
                  style={{
                    backgroundColor: c.key_camelot ? c.color + '30' : 'transparent',
                    color: c.key_camelot ? c.color : 'var(--data-text-dim)',
                    border: c.key_camelot ? `1px solid ${c.color}40` : '1px solid var(--data-border)',
                  }}
                >
                  {c.key_camelot || '—'}
                </span>
              </div>
            ))}
          </div>
        </div>

        {/* BPM strip */}
        <div className="data-plane p-2 flex-1" style={{ minHeight: '40px' }}>
          <div className="text-[9px] text-data-text-dim uppercase mb-1">Tempo</div>
          <div className="flex items-center gap-1 flex-wrap">
            {clipData.map((c, i) => (
              <div key={i} className="flex items-center gap-1">
                {i > 0 && <span className="text-data-text-dim text-[10px]">→</span>}
                <span className="text-[10px] font-mono text-data-text">
                  {c.bpm ? c.bpm.toFixed(0) : '—'}
                </span>
              </div>
            ))}
          </div>
        </div>
      </div>

      {/* Track list with scene markers */}
      <div className="data-plane p-2">
        <div className="text-[9px] text-data-text-dim uppercase mb-1">Sequence</div>
        <div className="flex gap-1 overflow-x-auto scrollbar-thin">
          {clipData.map((c, i) => (
            <div
              key={i}
              className="flex-shrink-0 px-2 py-1 rounded text-[10px] data-plane border-l-2"
              style={{
                borderLeftColor: c.key_camelot ? c.color : 'var(--data-border)',
                minWidth: '80px',
                maxWidth: '120px',
              }}
              title={c.title}
            >
              <div className="text-data-text truncate">{i + 1}. {c.title}</div>
              <div className="flex gap-1.5 text-data-text-dim text-[9px]">
                {c.key_camelot && <span style={{ color: c.color }}>{c.key_camelot}</span>}
                {c.bpm && <span>{c.bpm.toFixed(0)} BPM</span>}
                {c.energy && <span>E{c.energy}</span>}
              </div>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
