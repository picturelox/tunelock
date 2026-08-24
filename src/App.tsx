import { useState, useEffect } from 'react';
import MainLayout from './components/layout/MainLayout';
import ErrorBoundary from './components/layout/ErrorBoundary';
import ConsoleView from './components/console/ConsoleView';
import TunerView from './components/tuner/TunerView';
import LibraryView from './components/library/LibraryView';
import MixWorkspace from './components/mix/MixWorkspace';
import DeliveryView from './components/delivery/DeliveryView';
import GoldView from './components/gold/GoldView';
import AssistView from './components/assist/AssistView';
import { useLibraryStore } from './stores/libraryStore';
import { onTrackAnalyzed, onMetadataBatchComplete, onAnalysisProgress } from './lib/tauri';

export type View = 'console' | 'analyze' | 'library' | 'mix' | 'delivery' | 'gold' | 'assist';

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
      case 'console':
        return <ConsoleView />;
      case 'analyze':
        return <TunerView />;
      case 'library':
        return <LibraryView />;
      case 'mix':
        return <MixWorkspace />;
      case 'delivery':
        return <DeliveryView />;
      case 'gold':
        return <GoldView />;
      case 'assist':
        return <AssistView />;
      default:
        return <ConsoleView />;
    }
  };

  return (
    <ErrorBoundary>
      <MainLayout currentView={currentView} onViewChange={setCurrentView}>
        <ErrorBoundary>
          {renderContent()}
        </ErrorBoundary>
      </MainLayout>
    </ErrorBoundary>
  );
}

export default App;
