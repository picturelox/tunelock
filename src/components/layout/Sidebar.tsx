import { useState } from 'react';
import {
  Sliders,
  Activity,
  Library,
  Layers,
  PackageOpen,
  Disc3,
  GraduationCap,
  Sparkles,
  FlaskConical,
  ChevronDown,
} from 'lucide-react';
import type { View } from '../../App';

interface SidebarProps {
  currentView: View;
  onViewChange: (view: View) => void;
}

const primaryNavItems: { view: View; label: string; icon: typeof Activity }[] = [
  { view: 'analyze', label: 'Analyze', icon: Activity },
  { view: 'library', label: 'Library', icon: Library },
];

const experimentalNavItems: { view: View; label: string; icon: typeof Activity }[] = [
  { view: 'console', label: 'Console', icon: Sliders },
  { view: 'mix', label: 'Mix Canvas', icon: Layers },
  { view: 'delivery', label: 'Delivery', icon: PackageOpen },
  { view: 'gold', label: 'Gold Set', icon: GraduationCap },
  { view: 'assist', label: 'Assist', icon: Sparkles },
];

export default function Sidebar({ currentView, onViewChange }: SidebarProps) {
  const [experimentalOpen, setExperimentalOpen] = useState(false);

  const renderNavItem = ({ view, label, icon: Icon }: typeof primaryNavItems[number]) => (
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
      aria-label={label}
    >
      <Icon className="w-5 h-5" />
    </button>
  );

  return (
    <aside className="w-14 faceplate-flat flex flex-col items-center py-3 border-r border-plate-darker">
      <div className="mb-4">
        <Disc3 className="w-7 h-7 text-cap-amber" />
      </div>
      <nav className="flex-1 flex flex-col gap-1.5">
        {primaryNavItems.map(renderNavItem)}

        <div className="w-8 border-t border-plate-darker/80 my-1" />
        <button
          onClick={() => setExperimentalOpen((open) => !open)}
          className="w-10 min-h-10 rounded-md flex flex-col items-center justify-center gap-0.5 text-label-dim hover:text-label-cream hover:bg-plate-light/30 transition-colors"
          title="Experimental tools"
          aria-expanded={experimentalOpen}
          aria-label="Toggle experimental tools"
        >
          <div className="flex items-center">
            <FlaskConical className="w-4 h-4" />
            <ChevronDown className={`w-3 h-3 transition-transform ${experimentalOpen ? 'rotate-180' : ''}`} />
          </div>
          <span className="text-[7px] tracking-widest">LAB</span>
        </button>

        {experimentalOpen && experimentalNavItems.map(renderNavItem)}
      </nav>
    </aside>
  );
}
