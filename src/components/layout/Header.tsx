import { useLibraryStore } from '../../stores/libraryStore';
import { Activity, Pause, Play } from 'lucide-react';

export default function Header() {
  const { analysisProgress, isAnalyzing, isPaused } = useLibraryStore();

  return (
    <header className="h-12 bg-surface border-b border-white/5 flex items-center justify-between px-4">
      <h1 className="text-lg font-semibold text-text-primary">TuneLock</h1>
      
      {analysisProgress && (
        <div className="flex items-center gap-4">
          <div className="flex items-center gap-2 text-sm text-text-secondary">
            <Activity className="w-4 h-4" />
            <span>
              {analysisProgress.completed} / {analysisProgress.total} analyzed
            </span>
            <span className="text-text-secondary/60">
              ({analysisProgress.speed_per_sec.toFixed(1)} tracks/sec)
            </span>
          </div>
          
          <div className="w-40 h-2 bg-surface-light rounded-full overflow-hidden">
            <div
              className="h-full bg-accent-primary transition-all duration-300"
              style={{
                width: `${(analysisProgress.completed / Math.max(analysisProgress.total, 1)) * 100}%`,
              }}
            />
          </div>
          
          {isAnalyzing && !isPaused ? (
            <Pause className="w-4 h-4 text-accent-primary" />
          ) : isPaused ? (
            <Play className="w-4 h-4 text-accent-primary" />
          ) : null}
        </div>
      )}
    </header>
  );
}
