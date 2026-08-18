import { useMixStore } from '../../stores/mixStore';
import { useLibraryStore } from '../../stores/libraryStore';
import { relationshipColor } from '../../lib/relationships';

export default function RelationshipInspector() {
  const { project } = useMixStore();
  const { tracks } = useLibraryStore();

  const selectedClip = project.clips.find((c) => c.id === project.selectedClipId);
  const selectedTrans = project.transitions.find((t) => t.id === project.selectedTransitionId);

  if (!selectedClip && !selectedTrans) {
    return (
      <div className="p-4 text-xs text-text-secondary">
        Select a clip or a transition on the timeline to inspect details.
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-4 p-4">
      {selectedClip && (
        <ClipDetails clip={selectedClip} tracks={tracks} />
      )}

      {selectedTrans && (
        <TransitionDetails trans={selectedTrans} tracks={tracks} />
      )}
    </div>
  );
}

function ClipDetails({ clip, tracks }: { clip: { trackId: number; notes?: string }; tracks: Map<number, any> }) {
  const track = tracks.get(clip.trackId);
  if (!track) return null;

  return (
    <div className="flex flex-col gap-2">
      <div className="text-xs font-semibold uppercase tracking-wide text-text-secondary">Track</div>
      <div className="text-sm font-medium text-text-primary">{track.title || track.filename}</div>
      <div className="text-xs text-text-secondary">{track.artist}</div>
      <div className="grid grid-cols-2 gap-2 text-xs mt-1">
        <div className="bg-surface-light rounded px-2 py-1">
          <div className="text-text-secondary">BPM</div>
          <div className="font-mono text-text-primary">{track.bpm?.toFixed(1)}</div>
        </div>
        <div className="bg-surface-light rounded px-2 py-1">
          <div className="text-text-secondary">Key</div>
          <div className="font-mono text-text-primary">{track.key_camelot}</div>
        </div>
        <div className="bg-surface-light rounded px-2 py-1">
          <div className="text-text-secondary">Confidence</div>
          <div className="font-mono text-text-primary">{Math.round((track.key_confidence ?? 0) * 100)}%</div>
        </div>
        <div className="bg-surface-light rounded px-2 py-1">
          <div className="text-text-secondary">Duration</div>
          <div className="font-mono text-text-primary">
            {track.duration_ms ? formatDuration(track.duration_ms) : '—'}
          </div>
        </div>
      </div>
      {clip.notes && (
        <div className="text-xs text-accent-primary italic mt-1">{clip.notes}</div>
      )}
    </div>
  );
}

function TransitionDetails({ trans, tracks }: { trans: { fromClipId: string; toClipId: string; relationshipType: string; score: number; label: string; explanation: string; risk: string; bpmDeltaPercent: number }; tracks: Map<number, any> }) {
  const { project } = useMixStore();
  const fromClip = project.clips.find((c) => c.id === trans.fromClipId);
  const toClip = project.clips.find((c) => c.id === trans.toClipId);
  const fromTrack = fromClip ? tracks.get(fromClip.trackId) : null;
  const toTrack = toClip ? tracks.get(toClip.trackId) : null;
  const color = relationshipColor(trans.relationshipType as any);

  return (
    <div className="flex flex-col gap-3">
      <div className="text-xs font-semibold uppercase tracking-wide text-text-secondary">
        Transition
      </div>

      {/* From → To */}
      <div className="flex items-center gap-2 text-xs">
        <span className="truncate flex-1 text-text-primary">{fromTrack?.title || fromTrack?.filename || '?'}</span>
        <span className="text-text-secondary">→</span>
        <span className="truncate flex-1 text-text-primary">{toTrack?.title || toTrack?.filename || '?'}</span>
      </div>

      {/* Relationship badge */}
      <div className="flex items-center gap-2">
        <span
          className="px-2 py-0.5 rounded text-xs font-bold text-white"
          style={{ backgroundColor: color }}
        >
          {trans.label}
        </span>
        <span className="text-xs text-text-secondary">score {trans.score}</span>
      </div>

      {/* Risk indicator */}
      <div className="flex items-center gap-2">
        <span className="text-[10px] uppercase tracking-wide text-text-secondary">Risk</span>
        <span
          className={`text-[10px] px-1.5 py-0.5 rounded font-medium ${
            trans.risk === 'low' ? 'bg-green-500/20 text-green-400'
            : trans.risk === 'medium' ? 'bg-amber-500/20 text-amber-400'
            : 'bg-red-500/20 text-red-400'
          }`}
        >
          {trans.risk}
        </span>
      </div>

      {/* Explanation */}
      <div className="text-xs text-text-secondary leading-relaxed">
        {trans.explanation}
      </div>

      {/* BPM delta */}
      {Math.abs(trans.bpmDeltaPercent) > 0.1 && (
        <div className="text-xs text-text-secondary">
          BPM delta: {trans.bpmDeltaPercent > 0 ? '+' : ''}
          {trans.bpmDeltaPercent.toFixed(1)}%
        </div>
      )}
    </div>
  );
}

function formatDuration(ms: number): string {
  const totalSeconds = Math.floor(ms / 1000);
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes}:${seconds.toString().padStart(2, '0')}`;
}
