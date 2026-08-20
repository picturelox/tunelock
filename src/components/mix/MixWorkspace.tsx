import { useState, useEffect } from 'react';
import { Save, FolderOpen, Plus, Check } from 'lucide-react';
import { useMixStore } from '../../stores/mixStore';
import { useLibraryStore } from '../../stores/libraryStore';
import MixTimeline from './MixTimeline';
import RelationshipInspector from './RelationshipInspector';
import CandidateRail from './CandidateRail';
import DualAuditionPanel from './DualAuditionPanel';
import type { MixViewPanel } from '../../types/mix';

export default function MixWorkspace() {
  const [leftPanel, setLeftPanel] = useState<MixViewPanel>('library');
  const [showLoadMenu, setShowLoadMenu] = useState(false);
  const [savedFlash, setSavedFlash] = useState(false);
  const { project, selectClip, selectTransition } = useMixStore();
  const { tracks } = useLibraryStore();

  // Persistence actions
  const saveCurrentMix = useMixStore((s) => s.saveCurrentMix);
  const loadSavedMixes = useMixStore((s) => s.loadSavedMixes);
  const loadMixById = useMixStore((s) => s.loadMixById);
  const savedMixes = useMixStore((s) => s.savedMixes);
  const savedId = useMixStore((s) => s.savedId);
  const saving = useMixStore((s) => s.saving);
  const createProject = useMixStore((s) => s.createProject);

  // Sync library tracks into the mix store's track map. Moved into an effect
  // to avoid setState-during-render, which breaks under concurrent rendering.
  const setTrackMap = useMixStore((s) => s.setTrackMap);
  const trackMapSize = useMixStore((s) => s.trackMap.size);
  useEffect(() => {
    if (trackMapSize !== tracks.size) {
      setTrackMap(new Map(tracks));
    }
  }, [tracks, trackMapSize, setTrackMap]);

  // Load saved mixes list on mount
  useEffect(() => {
    loadSavedMixes();
  }, [loadSavedMixes]);

  const handleSave = async () => {
    await saveCurrentMix();
    setSavedFlash(true);
    setTimeout(() => setSavedFlash(false), 2000);
  };

  const handleLoad = async (id: number) => {
    await loadMixById(id);
    setShowLoadMenu(false);
  };

  const handleNew = () => {
    createProject('New Mix');
    setShowLoadMenu(false);
  };

  return (
    <div className="flex flex-col h-full">
      {/* Top bar: project name + save/load + left-panel toggles */}
      <div className="flex items-center gap-3 px-4 py-2 border-b border-white/5 bg-surface/30">
        <input
          type="text"
          value={project.name}
          onChange={(e) => {
            // Update project name inline
            useMixStore.setState((s) => ({
              project: { ...s.project, name: e.target.value },
            }));
          }}
          className="text-sm font-semibold text-text-primary bg-transparent border-none outline-none focus:bg-white/5 rounded px-1 py-0.5 transition-colors"
          style={{ width: `${Math.max(project.name.length * 8, 60)}px` }}
        />
        {savedId && (
          <span className="text-xs text-text-secondary">#{savedId}</span>
        )}
        <div className="flex items-center gap-1 ml-2">
          <button
            onClick={handleNew}
            className="p-1.5 rounded-md text-text-secondary hover:text-text-primary hover:bg-white/5 transition-colors"
            title="New mix"
          >
            <Plus className="w-4 h-4" />
          </button>
          <button
            onClick={handleSave}
            disabled={saving || project.clips.length === 0}
            className="p-1.5 rounded-md text-text-secondary hover:text-text-primary hover:bg-white/5 transition-colors disabled:opacity-40"
            title="Save mix"
          >
            {savedFlash ? <Check className="w-4 h-4 text-green-400" /> : <Save className="w-4 h-4" />}
          </button>
          <div className="relative">
            <button
              onClick={() => setShowLoadMenu(!showLoadMenu)}
              className="p-1.5 rounded-md text-text-secondary hover:text-text-primary hover:bg-white/5 transition-colors"
              title="Load mix"
            >
              <FolderOpen className="w-4 h-4" />
            </button>
            {showLoadMenu && (
              <div className="absolute top-full left-0 mt-1 w-64 bg-surface rounded-lg border border-white/10 shadow-xl z-50 max-h-80 overflow-y-auto">
                {savedMixes.length === 0 ? (
                  <div className="px-3 py-4 text-xs text-text-secondary text-center">
                    No saved mixes yet
                  </div>
                ) : (
                  savedMixes.map((mix) => (
                    <button
                      key={mix.id}
                      onClick={() => handleLoad(mix.id)}
                      className={`w-full text-left px-3 py-2 text-xs hover:bg-white/5 transition-colors border-b border-white/5 ${
                        savedId === mix.id ? 'bg-accent-primary/20' : ''
                      }`}
                    >
                      <div className="text-text-primary truncate">{mix.name}</div>
                      <div className="text-text-secondary">{mix.createdAt}</div>
                    </button>
                  ))
                )}
              </div>
            )}
          </div>
        </div>
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
