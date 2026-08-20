import { Activity, Library, Layers, PackageOpen, Disc3, GraduationCap } from 'lucide-react';
import type { View } from '../../App';

interface SidebarProps {
  currentView: View;
  onViewChange: (view: View) => void;
}

const navItems: { view: View; label: string; icon: typeof Activity }[] = [
  { view: 'analyze', label: 'Analyze', icon: Activity },
  { view: 'library', label: 'Library', icon: Library },
  { view: 'mix', label: 'Mix Canvas', icon: Layers },
  { view: 'delivery', label: 'Delivery', icon: PackageOpen },
  { view: 'gold', label: 'Gold Set', icon: GraduationCap },
];

export default function Sidebar({ currentView, onViewChange }: SidebarProps) {
  return (
    <aside className="w-16 bg-surface flex flex-col items-center py-4 border-r border-white/5">
      <div className="mb-6">
        <Disc3 className="w-8 h-8 text-accent-primary" />
      </div>
      <nav className="flex-1 flex flex-col gap-2">
        {navItems.map(({ view, label, icon: Icon }) => (
          <button
            key={view}
            onClick={() => onViewChange(view)}
            className={`
              w-10 h-10 rounded-lg flex items-center justify-center
              transition-colors duration-200
              ${currentView === view
                ? 'bg-accent-primary text-white'
                : 'text-text-secondary hover:text-text-primary hover:bg-white/5'
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
