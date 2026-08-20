import { useState, useEffect } from 'react';
import { useMixStore } from '../../stores/mixStore';
import { useLibraryStore } from '../../stores/libraryStore';
import MixTimeline from './MixTimeline';
import RelationshipInspector from './RelationshipInspector';
import CandidateRail from './CandidateRail';
import DualAuditionPanel from './DualAuditionPanel';
import type { MixViewPanel } from '../../types/mix';

export default function MixWorkspace() {
  const [leftPanel, setLeftPanel] = useState<MixViewPanel>('library');
  const { project, selectClip, selectTransition } = useMixStore();
  const { tracks } = useLibraryStore();

  // Sync library tracks into the mix store's track map. Moved into an effect
  // to avoid setState-during-render, which breaks under concurrent rendering.
  const setTrackMap = useMixStore((s) => s.setTrackMap);
  const trackMapSize = useMixStore((s) => s.trackMap.size);
  useEffect(() => {
    if (trackMapSize !== tracks.size) {
      setTrackMap(new Map(tracks));
    }
  }, [tracks, trackMapSize, setTrackMap]);

  return (
    <div className="flex flex-col h-full">
      {/* Top bar: project name + left-panel toggles */}
      <div className="flex items-center gap-3 px-4 py-2 border-b border-white/5 bg-surface/30">
        <span className="text-sm font-semibold text-text-primary">{project.name}</span>
        <div className="flex-1" />
        <div className="flex gap-1">
          {(['library', 'candidates', 'inspector'] as MixViewPanel[]).map((p) => (
            <button
              key={p}
              onClick={() => setLeftPanel(p)}
              className={`
                px-3 py-1 rounded-md text-xs font-medium transition-colors
                ${leftPanel === p
                  ? 'bg-accent-primary text-white'
                  : 'text-text-secondary hover:text-text-primary hover:bg-white/5'
                }
              `}
            >
              {p === 'library' && 'Library'}
              {p === 'candidates' && 'Candidates'}
              {p === 'inspector' && 'Inspector'}
            </button>
          ))}
        </div>
      </div>

      {/* Main workspace */}
      <div className="flex flex-1 overflow-hidden">
        {/* Left panel */}
        <div className="w-72 border-r border-white/5 overflow-auto bg-surface/20">
          {leftPanel === 'library' && <LibraryRail />}
          {leftPanel === 'candidates' && <CandidateRail />}
          {leftPanel === 'inspector' && <RelationshipInspector />}
        </div>

        {/* Center: timeline */}
        <div
          className="flex-1 overflow-auto bg-surface/10"
          onClick={() => {
            selectClip(null);
            selectTransition(null);
          }}
        >
          <MixTimeline />
        </div>
      </div>

      {/* Bottom: dual audition deck */}
      <div className="h-32 border-t border-white/5 bg-surface/30">
        <DualAuditionPanel />
      </div>
    </div>
  );
}

function LibraryRail() {
  const { tracks } = useLibraryStore();
  const { addTrack } = useMixStore();
  const trackList = Array.from(tracks.values());

  return (
    <div className="flex flex-col h-full">
      <div className="px-3 py-2 text-xs font-semibold text-text-secondary uppercase tracking-wide">
        Library
      </div>
      <div className="flex-1 overflow-auto">
        {trackList.map((t) => (
          <button
            key={t.id}
            onClick={() => addTrack(t.id)}
            className="w-full text-left px-3 py-2 text-xs hover:bg-white/5 transition-colors border-b border-white/5"
            title={`Add ${t.title || t.filename} to mix`}
          >
            <div className="truncate text-text-primary">{t.title || t.filename}</div>
            <div className="text-text-secondary truncate">
              {t.artist} · {t.bpm?.toFixed(1)} BPM · {t.key_camelot}
            </div>
          </button>
        ))}
        {trackList.length === 0 && (
          <div className="p-4 text-xs text-text-secondary">
            Import tracks in the Library tab first.
          </div>
        )}
      </div>
    </div>
  );
}
