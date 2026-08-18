import { useState } from 'react';
import { Table, CircleDot, ListMusic } from 'lucide-react';
import LibraryTable from './LibraryTable';
import CamelotWheel from '../camelot/CamelotWheel';
import PlaylistBuilder from '../playlist/PlaylistBuilder';

type LibrarySubView = 'table' | 'wheel' | 'playlists';

const tabs: { id: LibrarySubView; label: string; icon: typeof Table }[] = [
  { id: 'table', label: 'Tracks', icon: Table },
  { id: 'wheel', label: 'Camelot Wheel', icon: CircleDot },
  { id: 'playlists', label: 'Playlists', icon: ListMusic },
];

export default function LibraryView() {
  const [subView, setSubView] = useState<LibrarySubView>('table');

  return (
    <div className="flex flex-col h-full">
      <div className="flex items-center gap-1 px-4 pt-3 border-b border-white/5">
        {tabs.map(({ id, label, icon: Icon }) => (
          <button
            key={id}
            onClick={() => setSubView(id)}
            className={`
              flex items-center gap-2 px-3 py-2 text-sm rounded-t-md
              transition-colors border-b-2
              ${subView === id
                ? 'border-accent-primary text-text-primary'
                : 'border-transparent text-text-secondary hover:text-text-primary'
              }
            `}
          >
            <Icon className="w-4 h-4" />
            {label}
          </button>
        ))}
      </div>

      <div className="flex-1 overflow-hidden">
        {subView === 'table' && <LibraryTable />}
        {subView === 'wheel' && <CamelotWheel />}
        {subView === 'playlists' && <PlaylistBuilder />}
      </div>
    </div>
  );
}
