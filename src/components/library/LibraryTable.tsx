import { useState, useCallback, useEffect, useRef } from 'react';
import { useVirtualizer } from '@tanstack/react-virtual';
import { FolderPlus, Search, ArrowUpDown, RefreshCw, Filter, Loader2 } from 'lucide-react';
import { useLibraryStore } from '../../stores/libraryStore';
import { useMixStore } from '../../stores/mixStore';
import ImportDialog from './ImportDialog';
import TrackRow from './TrackRow';
import { scanFolder, getLibraryPage, startAnalysis, importMikCsv } from '../../lib/tauri';
import { open } from '@tauri-apps/plugin-dialog';

const PAGE_SIZE = 500;
const LOAD_MORE_THRESHOLD = 50; // rows from bottom to trigger next page

export default function LibraryTable() {
  const [showImport, setShowImport] = useState(false);
  const [search, setSearch] = useState('');
  const [sortBy, setSortBy] = useState('filename');
  const [sortDir, setSortDir] = useState<'asc' | 'desc'>('asc');
  const [activeFilter, setActiveFilter] = useState<string>('all');
  const [isLoadingMore, setIsLoadingMore] = useState(false);
  const [hasMore, setHasMore] = useState(true);
  const [mikStatus, setMikStatus] = useState<string | null>(null);

  const {
    tracks,
    setTracks,
    setTotalCount,
    setIsAnalyzing,
    isAnalyzing,
  } = useLibraryStore();

  const { project } = useMixStore();
  const usedTrackIds = new Set(project.clips.map((c) => c.trackId));

  // Track list from the store Map
  const trackList = Array.from(tracks.values());

  // "In-mix" / "not-in-mix" filters are client-side because they depend on
  // the current mix project state, not just track properties.
  // All other smart filters are server-side.
  const isMixFilter = activeFilter === 'in-mix' || activeFilter === 'not-in-mix';
  const serverSmartFilter =
    activeFilter === 'unanalyzed' || activeFilter === 'low-confidence' || activeFilter === 'high-confidence'
      ? activeFilter
      : undefined;

  let filteredTracks = trackList;
  if (isMixFilter) {
    filteredTracks = activeFilter === 'in-mix'
      ? trackList.filter((t) => usedTrackIds.has(t.id))
      : trackList.filter((t) => !usedTrackIds.has(t.id));
  }

  const scrollRef = useRef<HTMLDivElement | null>(null);

  const virtualizer = useVirtualizer({
    count: filteredTracks.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => 40,
    overscan: 10,
  });

  // Load a page from the server. If append=true, tracks are added to the
  // existing Map instead of replacing it.
  const loadPage = useCallback(async (page: number, append: boolean) => {
    const result = await getLibraryPage(page, PAGE_SIZE, sortBy, sortDir, {
      search: search || undefined,
      smart_filter: serverSmartFilter,
    });

    if (append) {
      // Merge new tracks into the existing Map
      const existing = useLibraryStore.getState().tracks;
      const newMap = new Map(existing);
      for (const t of result.tracks) {
        newMap.set(t.id, t);
      }
      setTracks(Array.from(newMap.values()));
    } else {
      setTracks(result.tracks);
    }
    setTotalCount(result.total_count);
    setHasMore(result.tracks.length === PAGE_SIZE);
  }, [sortBy, sortDir, search, serverSmartFilter, setTracks, setTotalCount]);

  // Initial load and reload on filter/sort/search change
  useEffect(() => {
    loadPage(0, false);
  }, [loadPage]);

  const handleImport = async (path: string) => {
    try {
      const result = await scanFolder(path);
      console.log('Scanned:', result);
      await loadPage(0, false);
      // Auto-start analysis
      await startAnalysis();
      setIsAnalyzing(true);
    } catch (err) {
      console.error('Import failed:', err);
    }
  };

  // Import a MIK CSV file to populate reference metadata (energy, genre, MIK key)
  // for consensus scoring. This is a read-only import — it doesn't modify the
  // original CSV or the audio files.
  const handleMikImport = async () => {
    try {
      const selected = await open({
        multiple: false,
        filters: [{ name: 'CSV', extensions: ['csv'] }],
      });
      if (typeof selected !== 'string') return;
      setMikStatus('Importing...');
      const result = await importMikCsv(selected);
      setMikStatus(
        `MIK: ${result.matched.toLocaleString()} matched, ${result.unmatched.toLocaleString()} unmatched of ${result.totalRows.toLocaleString()} rows`
      );
      // Reload to show updated genre/energy
      await loadPage(0, false);
    } catch (err) {
      setMikStatus(`MIK import failed: ${err}`);
    }
  };

  // Infinite scroll: detect when the user is near the bottom and load more
  const handleScroll = useCallback(() => {
    const el = scrollRef.current;
    if (!el || isLoadingMore || !hasMore) return;
    const nearBottom = el.scrollHeight - el.scrollTop - el.clientHeight < LOAD_MORE_THRESHOLD * 40;
    if (nearBottom) {
      setIsLoadingMore(true);
      const nextPage = Math.floor(tracks.size / PAGE_SIZE);
      loadPage(nextPage, true).finally(() => setIsLoadingMore(false));
    }
  }, [isLoadingMore, hasMore, tracks.size, loadPage]);

  const handleSort = (column: string) => {
    if (sortBy === column) {
      setSortDir(sortDir === 'asc' ? 'desc' : 'asc');
    } else {
      setSortBy(column);
      setSortDir('asc');
    }
  };

  const SortHeader = ({ column, label }: { column: string; label: string }) => (
    <button
      onClick={() => handleSort(column)}
      className="flex items-center gap-1 hover:text-text-primary transition-colors"
    >
      {label}
      {sortBy === column && <ArrowUpDown className="w-3 h-3" />}
    </button>
  );

  const virtualItems = virtualizer.getVirtualItems();

  return (
    <div className="flex flex-col h-full">
      {/* Toolbar */}
      <div className="flex items-center gap-2 p-3 border-b border-white/5">
        <button
          onClick={() => setShowImport(true)}
          className="flex items-center gap-2 px-3 py-1.5 bg-accent-primary text-white rounded-md text-sm hover:bg-accent-primary/90 transition-colors"
        >
          <FolderPlus className="w-4 h-4" />
          Import
        </button>

        <button
          onClick={handleMikImport}
          className="flex items-center gap-2 px-3 py-1.5 bg-surface-light text-text-secondary rounded-md text-sm hover:text-text-primary hover:bg-white/5 transition-colors"
          title="Import a Mixed In Key CSV to populate reference metadata"
        >
          Import MIK CSV
        </button>

        <div className="flex items-center gap-2 flex-1 max-w-md">
          <Search className="w-4 h-4 text-text-secondary" />
          <input
            type="text"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder="Search tracks..."
            className="flex-1 bg-surface-light text-text-primary text-sm rounded-md px-2 py-1.5 outline-none border border-white/5 focus:border-accent-primary/50"
          />
        </div>

        <div className="flex items-center gap-2 text-sm text-text-secondary">
          {mikStatus && <span className="text-xs">{mikStatus}</span>}
          <span>{tracks.size.toLocaleString()} loaded</span>
          {isAnalyzing && <RefreshCw className="w-4 h-4 animate-spin text-accent-primary" />}
        </div>
      </div>

      {/* Smart filter bar */}
      <div className="flex items-center gap-1 px-3 py-1.5 border-b border-white/5 overflow-auto">
        <Filter className="w-3 h-3 text-text-secondary mr-1 flex-shrink-0" />
        {[
          { id: 'all', label: 'All' },
          { id: 'unanalyzed', label: 'Unanalyzed' },
          { id: 'low-confidence', label: 'Low Confidence' },
          { id: 'high-confidence', label: 'High Confidence' },
          { id: 'in-mix', label: 'In Mix' },
          { id: 'not-in-mix', label: 'Not In Mix' },
        ].map((f) => (
          <button
            key={f.id}
            onClick={() => setActiveFilter(f.id)}
            className={`
              px-2 py-0.5 rounded text-[11px] font-medium transition-colors whitespace-nowrap
              ${activeFilter === f.id
                ? 'bg-accent-primary text-white'
                : 'text-text-secondary hover:text-text-primary hover:bg-white/5'
              }
            `}
          >
            {f.label}
          </button>
        ))}
      </div>

      {/* Table Header */}
      <div className="flex items-center px-4 py-2 bg-surface-light/50 text-xs font-medium text-text-secondary border-b border-white/5">
        <div className="w-8">#</div>
        <div className="flex-1 min-w-0"><SortHeader column="filename" label="Filename" /></div>
        <div className="w-32"><SortHeader column="artist" label="Artist" /></div>
        <div className="w-32"><SortHeader column="title" label="Title" /></div>
        <div className="w-16 text-center"><SortHeader column="key_camelot" label="Key" /></div>
        <div className="w-16 text-right"><SortHeader column="bpm" label="BPM" /></div>
        <div className="w-20 text-right"><SortHeader column="duration_ms" label="Duration" /></div>
        <div className="w-10 text-center">Status</div>
        <div className="w-8"></div>
      </div>

      {/* Virtual Scrolled List with infinite scroll */}
      <div
        ref={scrollRef}
        onScroll={handleScroll}
        className="flex-1 overflow-auto scrollbar-thin"
      >
        <div
          style={{
            height: `${virtualizer.getTotalSize()}px`,
            width: '100%',
            position: 'relative',
          }}
        >
          {virtualItems.map((virtualRow) => {
            const track = filteredTracks[virtualRow.index];
            if (!track) return null;
            return (
              <div
                key={track.id}
                style={{
                  position: 'absolute',
                  top: 0,
                  left: 0,
                  width: '100%',
                  height: `${virtualRow.size}px`,
                  transform: `translateY(${virtualRow.start}px)`,
                }}
              >
                <TrackRow
                  track={track}
                  index={virtualRow.index + 1}
                />
              </div>
            );
          })}
        </div>

        {/* Loading indicator at bottom */}
        {isLoadingMore && (
          <div className="flex items-center justify-center py-4">
            <Loader2 className="w-5 h-5 text-accent-primary animate-spin" />
            <span className="ml-2 text-xs text-text-secondary">Loading more...</span>
          </div>
        )}
        {!hasMore && tracks.size > 0 && (
          <div className="text-center py-3 text-xs text-text-secondary">
            All {tracks.size.toLocaleString()} tracks loaded
          </div>
        )}
      </div>

      {showImport && (
        <ImportDialog
          onClose={() => setShowImport(false)}
          onImport={handleImport}
        />
      )}
    </div>
  );
}
