import { useMixStore } from '../../stores/mixStore';
import { useLibraryStore } from '../../stores/libraryStore';
import { formatCamelotBadge } from '../../lib/camelot';
import { relationshipColor } from '../../lib/relationships';

export default function MixTimeline() {
  const { project, selectClip } = useMixStore();
  const { tracks } = useLibraryStore();

  if (project.clips.length === 0) {
    return (
      <div className="flex items-center justify-center h-full text-text-secondary">
        <div className="text-center">
          <div className="text-lg font-light mb-2">Mix Canvas</div>
          <div className="text-sm max-w-md">
            Add tracks from the Library rail on the left to start building your set.
            Each transition will be analyzed for harmonic compatibility.
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="flex flex-col items-start gap-3 p-6 min-w-[600px]">
      {/* Key Journey Strip */}
      <KeyJourneyStrip />

      {/* Clips + transitions */}
      <div className="flex items-center gap-2 flex-wrap">
        {project.clips.map((clip, i) => {
          const track = tracks.get(clip.trackId);
          const isSelected = project.selectedClipId === clip.id;
          const badge = track?.key_camelot ? formatCamelotBadge(track.key_camelot) : null;

          return (
            <div key={clip.id} className="flex items-center gap-2">
              {/* Transition badge before this clip (except first) */}
              {i > 0 && (
                <TransitionBadge index={i - 1} />
              )}

              {/* Clip card */}
              <button
                onClick={(e) => {
                  e.stopPropagation();
                  selectClip(clip.id);
                }}
                className={`
                  flex flex-col gap-1 px-4 py-3 rounded-xl border transition-all min-w-[180px] text-left
                  ${isSelected
                    ? 'border-accent-primary bg-accent-primary/10'
                    : 'border-white/10 bg-surface/40 hover:border-white/20'
                  }
                `}
              >
                <div className="flex items-center gap-2">
                  {badge && (
                    <span
                      className="px-1.5 py-0.5 rounded text-[10px] font-bold text-white"
                      style={{ backgroundColor: badge.color }}
                    >
                      {badge.text}
                    </span>
                  )}
                  <span className="text-xs font-semibold text-text-primary truncate">
                    {track?.title || track?.filename || 'Unknown'}
                  </span>
                </div>
                <div className="text-[10px] text-text-secondary truncate">
                  {track?.artist}
                </div>
                <div className="text-[10px] text-text-secondary">
                  {track?.bpm?.toFixed(1)} BPM · {track?.key_standard} · {Math.round((track?.key_confidence ?? 0) * 100)}% confidence
                </div>
                {clip.notes && (
                  <div className="text-[10px] text-accent-primary italic truncate">
                    {clip.notes}
                  </div>
                )}
              </button>
            </div>
          );
        })}
      </div>
    </div>
  );
}

function KeyJourneyStrip() {
  const { project } = useMixStore();
  const { tracks } = useLibraryStore();

  const keys = project.clips
    .map((c) => {
      const t = tracks.get(c.trackId);
      return t?.key_camelot ?? null;
    })
    .filter((k): k is string => !!k);

  if (keys.length === 0) return null;

  return (
    <div className="flex items-center gap-2 text-xs text-text-secondary mb-2">
      <span className="font-semibold uppercase tracking-wide">Key journey:</span>
      {keys.map((k, i) => (
        <span key={i} className="flex items-center gap-2">
          <span className="font-mono px-2 py-0.5 rounded bg-surface-light">{k}</span>
          {i < keys.length - 1 && <span className="text-white/20">→</span>}
        </span>
      ))}
    </div>
  );
}

function TransitionBadge({ index }: { index: number }) {
  const { project, selectTransition } = useMixStore();
  const trans = project.transitions[index];
  if (!trans) return <span className="text-white/20">→</span>;

  const isSelected = project.selectedTransitionId === trans.id;
  const color = relationshipColor(trans.relationshipType);

  return (
    <button
      onClick={(e) => {
        e.stopPropagation();
        selectTransition(trans.id);
      }}
      className={`
        flex flex-col items-center px-2 py-1 rounded-lg border transition-all min-w-[80px]
        ${isSelected
          ? 'border-accent-primary bg-accent-primary/10'
          : 'border-transparent hover:border-white/10 hover:bg-white/5'
        }
      `}
      title={trans.explanation}
    >
      <span
        className="text-[10px] font-bold text-white px-1.5 py-0.5 rounded"
        style={{ backgroundColor: color }}
      >
        {trans.label}
      </span>
      <span className="text-[9px] text-text-secondary mt-0.5">
        score {trans.score}
      </span>
    </button>
  );
}
