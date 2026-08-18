import { useState } from 'react';
import MainLayout from './components/layout/MainLayout';
import TunerView from './components/tuner/TunerView';
import LibraryView from './components/library/LibraryView';
import MixWorkspace from './components/mix/MixWorkspace';
import DeliveryView from './components/delivery/DeliveryView';
import { useLibraryStore } from './stores/libraryStore';
import { onTrackAnalyzed, onMetadataBatchComplete, onAnalysisProgress } from './lib/tauri';
import { useEffect } from 'react';

export type View = 'analyze' | 'library' | 'mix' | 'delivery';

function App() {
  const [currentView, setCurrentView] = useState<View>('analyze');
  const { queueTrackUpdate, flushPendingUpdates, setAnalysisProgress } = useLibraryStore();

  useEffect(() => {
    let rafId: number | null = null;

    const setupListeners = async () => {
      const unlistenTrack = await onTrackAnalyzed((track) => {
        queueTrackUpdate(track);
        if (!rafId) {
          rafId = requestAnimationFrame(() => {
            flushPendingUpdates();
            rafId = null;
          });
        }
      });

      const unlistenMetadata = await onMetadataBatchComplete((tracks) => {
        for (const track of tracks) {
          queueTrackUpdate(track);
        }
        if (!rafId) {
          rafId = requestAnimationFrame(() => {
            flushPendingUpdates();
            rafId = null;
          });
        }
      });

      const unlistenProgress = await onAnalysisProgress((progress) => {
        setAnalysisProgress(progress);
      });

      return () => {
        unlistenTrack();
        unlistenMetadata();
        unlistenProgress();
      };
    };

    const cleanup = setupListeners();
    return () => {
      cleanup.then((fn) => fn());
      if (rafId) cancelAnimationFrame(rafId);
    };
  }, [queueTrackUpdate, flushPendingUpdates, setAnalysisProgress]);

  const renderContent = () => {
    switch (currentView) {
      case 'analyze':
        return <TunerView />;
      case 'library':
        return <LibraryView />;
      case 'mix':
        return <MixWorkspace />;
      case 'delivery':
        return <DeliveryView />;
      default:
        return <TunerView />;
    }
  };

  return (
    <MainLayout currentView={currentView} onViewChange={setCurrentView}>
      <div className="flex flex-col h-full">
        <div className="flex-1 overflow-hidden">
          {renderContent()}
        </div>
      </div>
    </MainLayout>
  );
}

export default App;
