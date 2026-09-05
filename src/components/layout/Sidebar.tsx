import { Disc3, Library } from 'lucide-react';

interface SidebarProps {
  onToggleLibrary: () => void;
}

export default function Sidebar({ onToggleLibrary }: SidebarProps) {
  return (
    <aside className="w-14 faceplate-flat flex flex-col items-center py-3 border-r border-plate-darker">
      <div className="mb-4">
        <Disc3 className="w-7 h-7 text-cap-amber" />
      </div>
      <nav className="flex-1 flex flex-col gap-1.5">
        <button
          onClick={onToggleLibrary}
          className="w-10 h-10 rounded-md flex items-center justify-center transition-all duration-100 no-select text-label-dim hover:text-label-cream hover:bg-plate-light/30"
          title="Library"
          aria-label="Library"
        >
          <Library className="w-5 h-5" />
        </button>
      </nav>
    </aside>
  );
}
