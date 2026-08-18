import { useState, useCallback, useEffect } from 'react';
import { useVirtualizer } from '@tanstack/react-virtual';
import { FolderPlus, Search, ArrowUpDown, RefreshCw, Filter } from 'lucide-react';
import { useLibraryStore } from '../../stores/libraryStore';
import { useMixStore } from '../../stores/mixStore';
import ImportDialog from './ImportDialog';
import TrackRow from './TrackRow';
import { scanFolder, getLibraryPage, startAnalysis } from '../../lib/tauri';

export default function LibraryTable() {
  const [showImport, setShowImport] = useState(false);
  const [search, setSearch] = useState('');
  const [sortBy, setSortBy] = useState('filename');
  const [sortDir, setSortDir] = useState<'asc' | 'desc'>('asc');
  
  const {
    tracks,
    pageSize,
    isAnalyzing,
    setTracks,
    setTotalCount,
    setIsAnalyzing,
  } = useLibraryStore();

  // Smart view filters
  const [activeFilter, setActiveFilter] = useState<string>('all');
  const { project } = useMixStore();
  const usedTrackIds = new Set(project.clips.map((c) => c.trackId));

  const trackList = Array.from(tracks.values());
  let filteredTracks = search
    ? trackList.filter(
        (t) =>
          t.filename.toLowerCase().includes(search.toLowerCase()) ||
          (t.title?.toLowerCase() ?? '').includes(search.toLowerCase()) ||
          (t.artist?.toLowerCase() ?? '').includes(search.toLowerCase())
      )
    : trackList;

  // Apply smart view filter
  switch (activeFilter) {
    case 'unanalyzed':
      filteredTracks = filteredTracks.filter((t) => !t.key_camelot);
      break;
    case 'low-confidence':
      filteredTracks = filteredTracks.filter((t) => (t.key_confidence ?? 0) < 0.7);
      break;
    case 'in-mix':
      filteredTracks = filteredTracks.filter((t) => usedTrackIds.has(t.id));
      break;
    case 'not-in-mix':
      filteredTracks = filteredTracks.filter((t) => !usedTrackIds.has(t.id));
      break;
    case 'high-confidence':
      filteredTracks = filteredTracks.filter((t) => (t.key_confidence ?? 0) >= 0.85);
      break;
  }

  const parentRef = useCallback((_node: HTMLDivElement | null) => {
    // ref callback
  }, []);

  const virtualizer = useVirtualizer({
    count: filteredTracks.length,
    getScrollElement: () => document.getElementById('library-scroll'),
    estimateSize: () => 40,
    overscan: 5,
  });

  const handleImport = async (path: string) => {
    try {
      const result = await scanFolder(path);
      console.log('Scanned:', result);
      await loadPage(0);
      // Auto-start analysis
      await startAnalysis();
      setIsAnalyzing(true);
    } catch (err) {
      console.error('Import failed:', err);
    }
  };

  const loadPage = async (page: number) => {
    try {
      const result = await getLibraryPage(page, pageSize, sortBy, sortDir, {
        search: search || undefined,
      });
      setTracks(result.tracks);
      setTotalCount(result.total_count);
    } catch (err) {
      console.error('Load page failed:', err);
    }
  };

  useEffect(() => {
    loadPage(0);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

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
          <span>{filteredTracks.length} tracks</span>
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

      {/* Virtual Scrolled List */}
      <div
        id="library-scroll"
        ref={parentRef}
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
