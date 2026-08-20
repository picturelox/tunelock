// MixWorkspace — the three-level Mix Canvas workspace.
//
// Three levels of magnification, all views of the same saved mix state:
//   1. Set Map — strategic trajectory (always visible at top)
//   2. Layer Lab — eight-slot exploratory grid (always visible in middle)
//   3. Transition Workbench — precision editing (expands when a transition is selected)
//
// Layout:
//   ┌─────────────────────────────────────────────────────────────┐
//   │ Top bar: project name, save/load, view toggles              │
//   ├──────────────────────────────────────────┬──────────────────┤
//   │ SET MAP (strategic trajectory)            │ CONTEXT          │
//   ├──────────────────────────────────────────┤ INSPECTOR        │
//   │ LAYER LAB (eight-slot grid)               │                  │
//   ├──────────────────────────────────────────┤                  │
//   │ TRANSITION WORKBENCH (expandable)         │                  │
//   ├──────────────────────────────────────────┴──────────────────┤
//   │ TRANSPORT BAR (crossfader, master, scene capture)           │
//   └─────────────────────────────────────────────────────────────┘
//
// Design language: Walnut Console — the frame uses walnut/bronze
// tokens, the data plane stays charcoal.

import { useState, useEffect } from 'react';
import { Save, FolderOpen, Plus, Check } from 'lucide-react';
import { useMixStore } from '../../stores/mixStore';
import { useLibraryStore } from '../../stores/libraryStore';
import SetMap from './SetMap';
import LayerLab from './LayerLab';
import TransportBar from './TransportBar';
import ContextInspector from './ContextInspector';
import MixTimeline from './MixTimeline';
import RelationshipInspector from './RelationshipInspector';
import CandidateRail from './CandidateRail';
import type { MixViewPanel } from '../../types/mix';

export default function MixWorkspace() {
  const [leftPanel, setLeftPanel] = useState<MixViewPanel>('library');
  const [showLoadMenu, setShowLoadMenu] = useState(false);
  const [savedFlash, setSavedFlash] = useState(false);
  const { project } = useMixStore();
  const { tracks } = useLibraryStore();

  const saveCurrentMix = useMixStore((s) => s.saveCurrentMix);
  const loadSavedMixes = useMixStore((s) => s.loadSavedMixes);
  const loadMixById = useMixStore((s) => s.loadMixById);
  const savedMixes = useMixStore((s) => s.savedMixes);
  const savedId = useMixStore((s) => s.savedId);
  const saving = useMixStore((s) => s.saving);
  const createProject = useMixStore((s) => s.createProject);
  const setTrackMap = useMixStore((s) => s.setTrackMap);
  const trackMapSize = useMixStore((s) => s.trackMap.size);

  useEffect(() => {
    if (trackMapSize !== tracks.size) {
      setTrackMap(new Map(tracks));
    }
  }, [tracks, trackMapSize, setTrackMap]);

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

  const hasTransition = project.selectedTransitionId !== null;

  return (
    <div className="flex flex-col h-full bg-background">
      {/* Top bar: project name + save/load + left-panel toggles */}
      <div className="flex items-center gap-3 px-4 py-2 border-b border-walnut-light/30 bg-walnut-dark/40">
        <input
          type="text"
          value={project.name}
          onChange={(e) => {
            useMixStore.setState((s) => ({
              project: { ...s.project, name: e.target.value },
            }));
          }}
          className="text-sm font-semibold text-cream-label bg-transparent border-none outline-none focus:bg-walnut-light/30 rounded px-1 py-0.5 transition-colors"
          style={{ width: `${Math.max(project.name.length * 8, 60)}px` }}
        />
        {savedId && (
          <span className="text-xs text-cream-label/50">#{savedId}</span>
        )}
        <div className="flex items-center gap-1 ml-2">
          <button
            onClick={handleNew}
            className="p-1.5 rounded-md text-cream-label/60 hover:text-brass-bright hover:bg-walnut-light/30 transition-colors"
            title="New mix"
          >
            <Plus className="w-4 h-4" />
          </button>
          <button
            onClick={handleSave}
            disabled={saving || project.clips.length === 0}
            className="p-1.5 rounded-md text-cream-label/60 hover:text-brass-bright hover:bg-walnut-light/30 transition-colors disabled:opacity-40"
            title="Save mix"
          >
            {savedFlash ? <Check className="w-4 h-4 text-lamp-green" /> : <Save className="w-4 h-4" />}
          </button>
          <div className="relative">
            <button
              onClick={() => setShowLoadMenu(!showLoadMenu)}
              className="p-1.5 rounded-md text-cream-label/60 hover:text-brass-bright hover:bg-walnut-light/30 transition-colors"
              title="Load mix"
            >
              <FolderOpen className="w-4 h-4" />
            </button>
            {showLoadMenu && (
              <div className="absolute top-full left-0 mt-1 w-64 walnut-frame rounded-lg shadow-xl z-50 max-h-80 overflow-y-auto">
                {savedMixes.length === 0 ? (
                  <div className="px-3 py-4 text-xs text-cream-label/40 text-center">
                    No saved mixes yet
                  </div>
                ) : (
                  savedMixes.map((mix) => (
                    <button
                      key={mix.id}
                      onClick={() => handleLoad(mix.id)}
                      className={`w-full text-left px-3 py-2 text-xs hover:bg-walnut-light/30 transition-colors border-b border-walnut-light/20 ${
                        savedId === mix.id ? 'bg-brass-accent/20' : ''
                      }`}
                    >
                      <div className="text-cream-label truncate">{mix.name}</div>
                      <div className="text-cream-label/50">{mix.createdAt}</div>
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
                  ? 'bg-brass-accent text-walnut-dark'
                  : 'text-cream-label/60 hover:text-cream-label hover:bg-walnut-light/30'
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

      {/* Main workspace: left rail + center (Set Map + Layer Lab + Workbench) + right inspector */}
      <div className="flex flex-1 overflow-hidden">
        {/* Left rail */}
        <div className="w-64 border-r border-walnut-light/20 overflow-auto bg-walnut-dark/20">
          {leftPanel === 'library' && <LibraryRail />}
          {leftPanel === 'candidates' && <CandidateRail />}
          {leftPanel === 'inspector' && <RelationshipInspector />}
        </div>

        {/* Center: three-level workspace */}
        <div className="flex-1 flex flex-col overflow-auto bg-background">
          {/* Level 1: Set Map */}
          <div className="p-2">
            <SetMap />
          </div>

          {/* Level 2: Layer Lab */}
          <div className="p-2">
            <LayerLab />
          </div>

          {/* Level 3: Transition Workbench (expandable) */}
          <div className="p-2 flex-1">
            {hasTransition ? (
              <div className="data-plane p-2 h-full">
                <div className="text-[10px] text-data-text-dim uppercase mb-2">
                  Transition Workbench — select a transition to expand
                </div>
                <MixTimeline />
              </div>
            ) : (
              <div className="data-plane p-4 h-full flex items-center justify-center">
                <div className="text-center">
                  <div className="text-[10px] text-data-text-dim uppercase mb-2">
                    Transition Workbench
                  </div>
                  <div className="text-xs text-data-text-dim">
                    Select a transition between clips to open the precision editing view.
                  </div>
                </div>
              </div>
            )}
          </div>
        </div>

        {/* Right: Context Inspector */}
        <div className="w-72 border-l border-walnut-light/20 overflow-auto bg-walnut-dark/20">
          <ContextInspector />
        </div>
      </div>

      {/* Bottom: Transport Bar */}
      <div className="border-t border-walnut-light/30">
        <TransportBar />
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
      <div className="px-3 py-2 engraved-label">
        Library
      </div>
      <div className="flex-1 overflow-auto scrollbar-thin">
        {trackList.map((t) => (
          <button
            key={t.id}
            onClick={() => addTrack(t.id)}
            className="w-full text-left px-3 py-2 text-xs hover:bg-walnut-light/20 transition-colors border-b border-walnut-light/10"
            title={`Add ${t.title || t.filename} to mix`}
          >
            <div className="truncate text-cream-label/90">{t.title || t.filename}</div>
            <div className="text-cream-label/50 truncate">
              {t.artist} · {t.bpm?.toFixed(1)} BPM · {t.key_camelot}
            </div>
          </button>
        ))}
        {trackList.length === 0 && (
          <div className="p-4 text-xs text-cream-label/40">
            Import tracks in the Library tab first.
          </div>
        )}
      </div>
    </div>
  );
}
