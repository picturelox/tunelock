import { useLibraryStore } from '../../stores/libraryStore';
import { Activity, Pause, Play } from 'lucide-react';

export default function Header() {
  const { analysisProgress, isAnalyzing, isPaused } = useLibraryStore();

  return (
    <header className="h-10 faceplate-flat flex items-center justify-between px-4 border-b border-plate-darker/60">
      <h1 className="text-sm font-bold text-label-cream tracking-wide">TuneLock</h1>

      {analysisProgress && (
        <div className="flex items-center gap-3">
          <div className="flex items-center gap-2 text-xs text-label-dim">
            <Activity className="w-3.5 h-3.5" />
            <span>
              {analysisProgress.completed} / {analysisProgress.total} analyzed
            </span>
            <span className="text-label-dim/60">
              ({analysisProgress.speed_per_sec.toFixed(1)} tracks/sec)
            </span>
          </div>

          <div className="w-32 h-1.5 data-plane rounded-full overflow-hidden">
            <div
              className="h-full bg-cap-amber transition-all duration-300"
              style={{
                width: `${(analysisProgress.completed / Math.max(analysisProgress.total, 1)) * 100}%`,
              }}
            />
          </div>

          {isAnalyzing && !isPaused ? (
            <Pause className="w-3.5 h-3.5 text-cap-amber" />
          ) : isPaused ? (
            <Play className="w-3.5 h-3.5 text-cap-amber" />
          ) : null}
        </div>
      )}
    </header>
  );
}
