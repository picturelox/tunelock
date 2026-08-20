// LibraryDrawer — slide-out library panel for loading tracks into channels.
//
// Slides in from the left when activated. Shows the track library
// with search/filter. Clicking a track loads it into the selected
// channel slot.

import { useState, useMemo } from 'react';
import { X, Search } from 'lucide-react';
import { useLibraryStore } from '../../stores/libraryStore';

interface LibraryDrawerProps {
  open: boolean;
  onClose: () => void;
  selectedChannel: number | null;
  onLoadTrack: (channelIndex: number, trackId: number) => void;
}

export default function LibraryDrawer({ open, onClose, selectedChannel, onLoadTrack }: LibraryDrawerProps) {
  const { tracks } = useLibraryStore();
  const [search, setSearch] = useState('');

  const filteredTracks = useMemo(() => {
    const trackList = Array.from(tracks.values());
    if (!search) return trackList;
    const q = search.toLowerCase();
    return trackList.filter(t =>
      (t.title || '').toLowerCase().includes(q) ||
      (t.artist || '').toLowerCase().includes(q) ||
      (t.filename || '').toLowerCase().includes(q) ||
      (t.key_camelot || '').toLowerCase().includes(q)
    );
  }, [tracks, search]);

  return (
    <>
      {/* Backdrop */}
      {open && (
        <div
          className="fixed inset-0 bg-black/50 z-40"
          onClick={onClose}
        />
      )}

      {/* Drawer */}
      <div
        className={`fixed left-0 top-0 bottom-0 w-80 faceplate z-50 transition-transform duration-200 flex flex-col ${
          open ? 'translate-x-0' : '-translate-x-full'
        }`}
      >
        {/* Header */}
        <div className="flex items-center justify-between p-3 border-b border-plate-darker/60">
          <span className="engraved">Library</span>
          <button onClick={onClose} className="cap cap-dark w-6 h-6 flex items-center justify-center">
            <X className="w-3 h-3" />
          </button>
        </div>

        {/* Channel indicator */}
        {selectedChannel !== null && (
          <div className="px-3 py-2 bg-plate-darker/40 text-center">
            <span className="text-[10px] text-label-cream">
              Loading into Channel {selectedChannel + 1}
            </span>
          </div>
        )}

        {/* Search */}
        <div className="p-2">
          <div className="flex items-center gap-1 data-plane px-2 py-1">
            <Search className="w-3 h-3 text-label-dim" />
            <input
              type="text"
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              placeholder="Search tracks..."
              className="flex-1 bg-transparent text-xs text-label-cream outline-none placeholder:text-label-dim/50"
            />
          </div>
        </div>

        {/* Track list */}
        <div className="flex-1 overflow-auto">
          {filteredTracks.length === 0 ? (
            <div className="p-4 text-center text-xs text-label-dim">
              {tracks.size === 0
                ? 'No tracks in library. Import from the Library tab.'
                : 'No tracks match your search.'}
            </div>
          ) : (
            filteredTracks.map(track => (
              <button
                key={track.id}
                onClick={() => {
                  if (selectedChannel !== null) {
                    onLoadTrack(selectedChannel, track.id);
                    onClose();
                  }
                }}
                className="w-full text-left px-3 py-2 hover:bg-plate-light/30 transition-colors border-b border-plate-darker/30"
              >
                <div className="text-xs text-label-cream truncate">
                  {track.title || track.filename}
                </div>
                <div className="flex gap-2 text-[9px] text-label-dim">
                  <span className="truncate">{track.artist}</span>
                  {track.key_camelot && (
                    <span className="text-cap-amber font-mono">{track.key_camelot}</span>
                  )}
                  {track.bpm && (
                    <span className="font-mono">{track.bpm.toFixed(0)} BPM</span>
                  )}
                </div>
              </button>
            ))
          )}
        </div>

        {/* Footer */}
        <div className="p-2 border-t border-plate-darker/60 text-center">
          <span className="engraved-sm">{filteredTracks.length} tracks</span>
        </div>
      </div>
    </>
  );
}
