import { useMixStore } from '../../stores/mixStore';
import { useLibraryStore } from '../../stores/libraryStore';
import { getCamelotRelationship, relationshipColor } from '../../lib/harmony';
import { formatCamelotBadge } from '../../lib/harmony';

export default function CandidateRail() {
  const { project, addTrack } = useMixStore();
  const { tracks } = useLibraryStore();

  const lastClip = project.clips[project.clips.length - 1];
  const lastTrack = lastClip ? tracks.get(lastClip.trackId) : null;

  if (!lastTrack) {
    return (
      <div className="p-4 text-xs text-text-secondary">
        Add at least one track to the mix to see candidate suggestions.
      </div>
    );
  }

  const candidates = getNextTrackCandidates(lastTrack, tracks, project.clips.map((c) => c.trackId));

  return (
    <div className="flex flex-col h-full">
      <div className="px-3 py-2 text-xs font-semibold text-text-secondary uppercase tracking-wide">
        Suggested next tracks
      </div>
      <div className="px-3 pb-1 text-[10px] text-text-secondary">
        From {lastTrack.title || lastTrack.filename} ({lastTrack.key_camelot})
      </div>
      <div className="flex-1 overflow-auto">
        {candidates.map((c) => (
          <button
            key={c.track.id}
            onClick={() => addTrack(c.track.id)}
            className="w-full text-left px-3 py-2 text-xs hover:bg-white/5 transition-colors border-b border-white/5"
            title={c.relationship.explanation}
          >
            <div className="flex items-center gap-2">
              {c.track.key_camelot && (
                <span
                  className="px-1.5 py-0.5 rounded text-[10px] font-bold text-white"
                  style={{ backgroundColor: formatCamelotBadge(c.track.key_camelot).color }}
                >
                  {c.track.key_camelot}
                </span>
              )}
              <span className="truncate flex-1 text-text-primary">{c.track.title || c.track.filename}</span>
            </div>
            <div className="text-text-secondary truncate">{c.track.artist}</div>
            <div className="flex items-center gap-2 mt-1">
              <span
                className="text-[10px] px-1.5 py-0.5 rounded text-white font-medium"
                style={{ backgroundColor: relationshipColor(c.relationship.type) }}
              >
                {c.relationship.label}
              </span>
              <span className="text-[10px] text-text-secondary">
                {c.track.bpm?.toFixed(1)} BPM · score {c.relationship.score}
              </span>
            </div>
            {c.reason && (
              <div className="text-[10px] text-text-secondary mt-0.5 italic">{c.reason}</div>
            )}
          </button>
        ))}
        {candidates.length === 0 && (
          <div className="p-4 text-xs text-text-secondary">
            No strong candidates found. Try a different seed track or expand your library.
          </div>
        )}
      </div>
    </div>
  );
}

interface Candidate {
  track: any;
  relationship: ReturnType<typeof getCamelotRelationship>;
  reason: string;
  alreadyUsed: boolean;
}

function getNextTrackCandidates(
  currentTrack: any,
  allTracks: Map<number, any>,
  usedTrackIds: number[],
): Candidate[] {
  const candidates: Candidate[] = [];
  const usedSet = new Set(usedTrackIds);

  for (const track of allTracks.values()) {
    if (track.id === currentTrack.id) continue;

    const rel = getCamelotRelationship(
      currentTrack.key_camelot ?? '',
      track.key_camelot ?? '',
      currentTrack.bpm ?? undefined,
      track.bpm ?? undefined,
    );

    // Only suggest relationships that are usable (not bridge-needed or unknown)
    if (rel.type === 'unknown') continue;

    const alreadyUsed = usedSet.has(track.id);

    // Build a reason string
    const reasons: string[] = [];
    if (rel.type === 'same-key') reasons.push('Same key — seamless blend');
    else if (rel.type === 'neighbor') reasons.push('Smooth harmonic step');
    else if (rel.type === 'mood-shift') reasons.push('Major/minor mood change');
    else if (rel.type === 'energy-boost') reasons.push('Classic energy lift');
    else if (rel.type === 'energy-drop') reasons.push('Cooldown move');
    else if (rel.type === 'tension') reasons.push('Creative risk');
    else if (rel.type === 'bridge-needed') reasons.push('Far apart — use with care');

    if (alreadyUsed) reasons.push('Already in mix');

    candidates.push({
      track,
      relationship: rel,
      reason: reasons.join('. '),
      alreadyUsed,
    });
  }

  // Sort: best relationships first, then by BPM closeness, then unused first
  candidates.sort((a, b) => {
    // Score descending
    if (b.relationship.score !== a.relationship.score) {
      return b.relationship.score - a.relationship.score;
    }
    // Already-used penalty
    if (a.alreadyUsed !== b.alreadyUsed) {
      return a.alreadyUsed ? 1 : -1;
    }
    // BPM delta ascending
    const aBpm = Math.abs(a.relationship.bpmDeltaPercent);
    const bBpm = Math.abs(b.relationship.bpmDeltaPercent);
    return aBpm - bBpm;
  });

  return candidates.slice(0, 20);
}
