import { useState, useEffect } from 'react';
import MainLayout from './components/layout/MainLayout';
import ErrorBoundary from './components/layout/ErrorBoundary';
import Workspace from './components/workspace/Workspace';
import { useLibraryStore } from './stores/libraryStore';
import { onTrackAnalyzed, onMetadataBatchComplete, onAnalysisProgress } from './lib/tauri';

function App() {
  const [libraryOpen, setLibraryOpen] = useState(false);
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

  return (
    <ErrorBoundary>
      <MainLayout onToggleLibrary={() => setLibraryOpen(o => !o)}>
        <ErrorBoundary>
          <Workspace libraryOpen={libraryOpen} setLibraryOpen={setLibraryOpen} />
        </ErrorBoundary>
      </MainLayout>
    </ErrorBoundary>
  );
}

export default App;
