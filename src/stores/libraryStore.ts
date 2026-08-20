import { create } from 'zustand';
import type { Track, LibraryFilter, AnalysisProgress } from '../types';

interface LibraryStore {
  // Track data
  tracks: Map<number, Track>;
  pendingUpdates: Track[];
  
  // View state
  filter: LibraryFilter;
  sortBy: string;
  sortDir: 'asc' | 'desc';
  searchQuery: string;
  
  // Pagination
  currentPage: number;
  pageSize: number;
  totalCount: number;
  
  // Analysis
  analysisProgress: AnalysisProgress | null;
  isAnalyzing: boolean;
  isPaused: boolean;
  
  // Actions
  setTracks: (tracks: Track[]) => void;
  addOrUpdateTrack: (track: Track) => void;
  queueTrackUpdate: (track: Track) => void;
  flushPendingUpdates: () => void;
  deleteTrack: (id: number) => void;
  
  setFilter: (filter: LibraryFilter) => void;
  setSort: (sortBy: string, sortDir: 'asc' | 'desc') => void;
  setSearchQuery: (query: string) => void;
  
  setCurrentPage: (page: number) => void;
  setPageSize: (size: number) => void;
  setTotalCount: (count: number) => void;
  
  setAnalysisProgress: (progress: AnalysisProgress | null) => void;
  setIsAnalyzing: (isAnalyzing: boolean) => void;
  setIsPaused: (isPaused: boolean) => void;
}

export const useLibraryStore = create<LibraryStore>((set) => ({
  tracks: new Map(),
  pendingUpdates: [],
  
  filter: {},
  sortBy: 'filename',
  sortDir: 'asc',
  searchQuery: '',
  
  currentPage: 0,
  pageSize: 500,
  totalCount: 0,
  
  analysisProgress: null,
  isAnalyzing: false,
  isPaused: false,
  
  setTracks: (tracks) => {
    const trackMap = new Map<number, Track>();
    for (const track of tracks) {
      trackMap.set(track.id, track);
    }
    set({ tracks: trackMap });
  },
  
  addOrUpdateTrack: (track) => {
    set((state) => {
      const newTracks = new Map(state.tracks);
      newTracks.set(track.id, track);
      return { tracks: newTracks };
    });
  },
  
  queueTrackUpdate: (track) => {
    set((state) => ({
      pendingUpdates: [...state.pendingUpdates, track],
    }));
  },
  
  flushPendingUpdates: () => {
    set((state) => {
      if (state.pendingUpdates.length === 0) return state;
      
      const newTracks = new Map(state.tracks);
      for (const track of state.pendingUpdates) {
        newTracks.set(track.id, track);
      }
      return { 
        tracks: newTracks, 
        pendingUpdates: [] 
      };
    });
  },
  
  deleteTrack: (id) => {
    set((state) => {
      const newTracks = new Map(state.tracks);
      newTracks.delete(id);
      return { tracks: newTracks };
    });
  },
  
  setFilter: (filter) => set({ filter, currentPage: 0 }),
  setSort: (sortBy, sortDir) => set({ sortBy, sortDir }),
  setSearchQuery: (searchQuery) => set({ searchQuery, currentPage: 0 }),
  
  setCurrentPage: (currentPage) => set({ currentPage }),
  setPageSize: (pageSize) => set({ pageSize, currentPage: 0 }),
  setTotalCount: (totalCount) => set({ totalCount }),
  
  setAnalysisProgress: (analysisProgress) => set({ analysisProgress }),
  setIsAnalyzing: (isAnalyzing) => set({ isAnalyzing }),
  setIsPaused: (isPaused) => set({ isPaused }),
}));
