import { create } from 'zustand';
import type { MixClip, MixTransition, MixProject } from '../types/mix';
import { getCamelotRelationship } from '../lib/relationships';
import type { Track } from '../types';

interface MixStore {
  // Current project
  project: MixProject;
  // Track lookup (injected from library store)
  trackMap: Map<number, Track>;

  // Actions
  createProject: (name: string) => void;
  setTrackMap: (tracks: Map<number, Track>) => void;
  addTrack: (trackId: number, notes?: string) => void;
  removeClip: (clipId: string) => void;
  moveClip: (clipId: string, newPosition: number) => void;
  reorderClips: (clipIds: string[]) => void;
  selectClip: (clipId: string | null) => void;
  selectTransition: (transitionId: string | null) => void;
  updateClipNotes: (clipId: string, notes: string) => void;
  recalculateTransitions: () => void;
}

function genId(): string {
  return `${Date.now()}-${Math.random().toString(36).slice(2, 9)}`;
}

function emptyProject(name: string): MixProject {
  return {
    id: genId(),
    name,
    clips: [],
    transitions: [],
    createdAt: new Date().toISOString(),
    updatedAt: new Date().toISOString(),
    selectedClipId: null,
    selectedTransitionId: null,
  };
}

export const useMixStore = create<MixStore>((set, get) => ({
  project: emptyProject('New Mix'),
  trackMap: new Map(),

  createProject: (name) => {
    set({ project: emptyProject(name) });
  },

  setTrackMap: (trackMap) => set({ trackMap }),

  addTrack: (trackId, notes) => {
    set((state) => {
      const nextPos = state.project.clips.length;
      const newClip: MixClip = {
        id: genId(),
        trackId,
        position: nextPos,
        notes,
      };
      const clips = [...state.project.clips, newClip];
      const project = { ...state.project, clips };
      return { project };
    });
    get().recalculateTransitions();
  },

  removeClip: (clipId) => {
    set((state) => {
      const filtered = state.project.clips.filter((c) => c.id !== clipId);
      // Renumber positions
      const clips = filtered.map((c, i) => ({ ...c, position: i }));
      const project = { ...state.project, clips, selectedClipId: null };
      return { project };
    });
    get().recalculateTransitions();
  },

  moveClip: (clipId, newPosition) => {
    set((state) => {
      const clips = [...state.project.clips];
      const idx = clips.findIndex((c) => c.id === clipId);
      if (idx === -1) return state;
      const [moved] = clips.splice(idx, 1);
      clips.splice(newPosition, 0, moved);
      // Renumber positions
      const renumbered = clips.map((c, i) => ({ ...c, position: i }));
      const project = { ...state.project, clips: renumbered };
      return { project };
    });
    get().recalculateTransitions();
  },

  reorderClips: (clipIds) => {
    set((state) => {
      const map = new Map(state.project.clips.map((c) => [c.id, c]));
      const clips = clipIds
        .map((id) => map.get(id))
        .filter((c): c is MixClip => !!c)
        .map((c, i) => ({ ...c, position: i }));
      const project = { ...state.project, clips };
      return { project };
    });
    get().recalculateTransitions();
  },

  selectClip: (clipId) => {
    set((state) => ({
      project: { ...state.project, selectedClipId: clipId, selectedTransitionId: null },
    }));
  },

  selectTransition: (transitionId) => {
    set((state) => ({
      project: { ...state.project, selectedTransitionId: transitionId, selectedClipId: null },
    }));
  },

  updateClipNotes: (clipId, notes) => {
    set((state) => {
      const clips = state.project.clips.map((c) =>
        c.id === clipId ? { ...c, notes } : c
      );
      const project = { ...state.project, clips };
      return { project };
    });
  },

  recalculateTransitions: () => {
    set((state) => {
      const { project: currentProject, trackMap } = state;
      const clips = currentProject.clips;
      const transitions: MixTransition[] = [];

      for (let i = 0; i < clips.length - 1; i++) {
        const fromClip = clips[i];
        const toClip = clips[i + 1];
        const fromTrack = trackMap.get(fromClip.trackId);
        const toTrack = trackMap.get(toClip.trackId);

        const rel = getCamelotRelationship(
          fromTrack?.key_camelot ?? '',
          toTrack?.key_camelot ?? '',
          fromTrack?.bpm ?? undefined,
          toTrack?.bpm ?? undefined,
        );

        transitions.push({
          id: `trans-${fromClip.id}-${toClip.id}`,
          fromClipId: fromClip.id,
          toClipId: toClip.id,
          relationshipType: rel.type,
          score: rel.score,
          label: rel.label,
          explanation: rel.explanation,
          risk: rel.risk,
          bpmDeltaPercent: rel.bpmDeltaPercent,
        });
      }

      const nextProject = {
        ...currentProject,
        transitions,
        updatedAt: new Date().toISOString(),
      };
      return { project: nextProject };
    });
  },
}));
