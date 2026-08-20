import type { TrackAnalysis } from '../../types';

export default function TimingsPanel({
  timings,
}: {
  timings: NonNullable<TrackAnalysis['timings']>;
}) {
  const rows: { label: string; ms: number }[] = [
    { label: 'Decode',       ms: timings.decode_ms },
    { label: 'Spectrogram',  ms: timings.spectrogram_ms },
    { label: 'HPSS',         ms: timings.hpss_ms },
    { label: 'Chromagram',   ms: timings.chromagram_ms },
    { label: 'Key ensemble', ms: timings.ensemble_ms },
    { label: 'Tempo',        ms: timings.tempo_ms },
    { label: 'Metadata',     ms: timings.metadata_ms },
  ];
  const maxMs = Math.max(...rows.map((r) => r.ms), 1);
  return (
    <div className="bg-surface/40 rounded-xl p-4">
      <div className="flex items-center justify-between mb-3">
        <h3 className="text-sm font-semibold">Stage timings</h3>
        <span className="font-mono text-xs text-text-secondary">
          total {timings.total_ms} ms
        </span>
      </div>
      <div className="flex flex-col gap-1.5">
        {rows.map((r) => (
          <div key={r.label} className="flex items-center gap-3 text-xs">
            <span className="w-28 text-text-secondary">{r.label}</span>
            <div className="flex-1 h-2 bg-surface-light rounded-full overflow-hidden">
              <div
                className="h-full bg-accent-primary/60"
                style={{ width: `${(r.ms / maxMs) * 100}%` }}
              />
            </div>
            <span className="w-14 text-right font-mono text-text-secondary">{r.ms} ms</span>
          </div>
        ))}
      </div>
    </div>
  );
}
