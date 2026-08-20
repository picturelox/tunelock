import { Loader2 } from 'lucide-react';
import type { TunerProgress } from '../../types';

export const STAGE_LABELS: Record<string, string> = {
  decode: 'Decoding audio',
  spectrogram: 'Computing spectrogram',
  hpss: 'Separating harmonic content (HPSS)',
  chromagram: 'Building chromagram',
  ensemble: 'Ensemble key voting',
  tempo: 'Detecting tempo',
  done: 'Done',
};

export default function AnalysisProgressDisplay({
  progress,
  filename,
}: {
  progress: TunerProgress | null;
  filename: string | null;
}) {
  const percent = Math.round((progress?.percent ?? 0) * 100);
  const stageLabel = progress ? (STAGE_LABELS[progress.stage] ?? progress.stage) : 'Starting';
  return (
    <div className="w-full max-w-md flex flex-col items-center gap-4 px-8">
      <Loader2 className="w-10 h-10 text-accent-primary animate-spin" />
      {filename && <div className="text-sm text-text-secondary truncate max-w-full">{filename}</div>}
      <div className="w-full">
        <div className="flex justify-between text-xs text-text-secondary mb-1">
          <span>{stageLabel}</span>
          <span className="font-mono">{percent}%</span>
        </div>
        <div className="h-2 bg-surface-light rounded-full overflow-hidden">
          <div
            className="h-full bg-accent-primary transition-all duration-200"
            style={{ width: `${percent}%` }}
          />
        </div>
      </div>
    </div>
  );
}
