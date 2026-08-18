import type { ReactNode } from 'react';
import Sidebar from './Sidebar';
import Header from './Header';
import type { View } from '../../App';

interface MainLayoutProps {
  children: ReactNode;
  currentView: View;
  onViewChange: (view: View) => void;
}

export default function MainLayout({ children, currentView, onViewChange }: MainLayoutProps) {
  return (
    <div className="flex h-screen w-screen bg-background overflow-hidden">
      <Sidebar currentView={currentView} onViewChange={onViewChange} />
      <div className="flex-1 flex flex-col min-w-0">
        <Header />
        <main className="flex-1 overflow-hidden">
          {children}
        </main>
      </div>
    </div>
  );
}
