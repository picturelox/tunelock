import { Sliders, Activity, Library, Layers, PackageOpen, Disc3, GraduationCap, Sparkles } from 'lucide-react';
import type { View } from '../../App';

interface SidebarProps {
  currentView: View;
  onViewChange: (view: View) => void;
}

const navItems: { view: View; label: string; icon: typeof Activity }[] = [
  { view: 'console', label: 'Console', icon: Sliders },
  { view: 'analyze', label: 'Analyze', icon: Activity },
  { view: 'library', label: 'Library', icon: Library },
  { view: 'mix', label: 'Mix Canvas', icon: Layers },
  { view: 'delivery', label: 'Delivery', icon: PackageOpen },
  { view: 'gold', label: 'Gold Set', icon: GraduationCap },
  { view: 'assist', label: 'Assist', icon: Sparkles },
];

export default function Sidebar({ currentView, onViewChange }: SidebarProps) {
  return (
    <aside className="w-14 faceplate-flat flex flex-col items-center py-3 border-r border-plate-darker">
      <div className="mb-4">
        <Disc3 className="w-7 h-7 text-cap-amber" />
      </div>
      <nav className="flex-1 flex flex-col gap-1.5">
        {navItems.map(({ view, label, icon: Icon }) => (
          <button
            key={view}
            onClick={() => onViewChange(view)}
            className={`
              w-10 h-10 rounded-md flex items-center justify-center
              transition-all duration-100 no-select
              ${currentView === view
                ? 'cap cap-amber lit'
                : 'text-label-dim hover:text-label-cream hover:bg-plate-light/30'
              }
            `}
            title={label}
          >
            <Icon className="w-5 h-5" />
          </button>
        ))}
      </nav>
    </aside>
  );
}
