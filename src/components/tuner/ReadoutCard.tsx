import { Copy, Check } from 'lucide-react';
import type { KeyCandidate } from '../../types';
import { formatCamelotBadge, getKeyAmbiguityRelationship } from '../../lib/harmony';

export interface ReadoutCardProps {
  displayed: KeyCandidate;
  candidates: KeyCandidate[];
  badge: { text: string; color: string } | null;
  bpm: number;
  overrideActive: boolean;
  onClearOverride: () => void;
  copied: boolean;
  onCopy: () => void;
  onReset: () => void;
}

export default function ReadoutCard({
  displayed,
  candidates,
  badge,
  bpm,
  overrideActive,
  onClearOverride,
  copied,
  onCopy,
  onReset,
}: ReadoutCardProps) {
  // Show up to 2 runner-ups that are "close" (conf >= 0.35) and not the winner.
  const runnerUps = candidates
    .filter(
      (c) =>
        c.key_camelot !== displayed.key_camelot &&
        c.confidence >= 0.35
    )
    .slice(0, 2);

  return (
    <div className="flex flex-col items-center gap-4 bg-surface/40 rounded-2xl p-6">
      {badge && (
        <div
          className="text-6xl font-bold px-10 py-5 rounded-3xl text-white shadow-2xl"
          style={{ backgroundColor: badge.color }}
        >
          {badge.text}
        </div>
      )}
      <div className="text-2xl font-light text-text-primary">{displayed.key_standard}</div>
      <div className="text-xl font-mono text-text-secondary">{bpm.toFixed(1)} BPM</div>

      <div className="w-full flex flex-col gap-1">
        <div className="flex justify-between text-xs text-text-secondary">
          <span>Confidence</span>
          <span className="font-mono">{Math.round(displayed.confidence * 100)}%</span>
        </div>
        <div className="h-2 bg-surface-light rounded-full overflow-hidden">
          <div
            className="h-full bg-accent-primary transition-all duration-500"
            style={{ width: `${displayed.confidence * 100}%` }}
          />
        </div>
        <div className="text-[10px] text-text-secondary mt-1">
          {displayed.segment_count}/8 segments agreed · profile-match{' '}
          {Math.round(displayed.avg_score * 100)}%
        </div>
      </div>

      {/* Secondary / ambiguous runner-up hint */}
      {runnerUps.length > 0 && (
        <div className="w-full flex flex-col gap-2 mt-1">
          <div className="text-[10px] uppercase tracking-wide text-text-secondary">
            Could also be
          </div>
          <div className="flex flex-wrap gap-2">
            {runnerUps.map((c) => {
              const rel = getKeyAmbiguityRelationship(displayed.key_standard, c.key_standard);
              const b = formatCamelotBadge(c.key_camelot);
              return (
                <div
                  key={c.key_camelot}
                  className="group relative flex items-center gap-2 px-3 py-1.5 rounded-lg bg-surface-light hover:bg-white/10 transition-colors"
                  title={rel.description}
                >
                  <span
                    className="px-1.5 py-0.5 rounded text-[10px] font-bold text-white"
                    style={{ backgroundColor: b.color }}
                  >
                    {b.text}
                  </span>
                  <span className="text-xs text-text-primary">{c.key_standard}</span>
                  {rel.label && (
                    <span className="text-[10px] px-1.5 py-0.5 rounded bg-white/10 text-text-secondary">
                      {rel.label}
                    </span>
                  )}
                </div>
              );
            })}
          </div>
        </div>
      )}

      {overrideActive && (
        <button
          onClick={onClearOverride}
          className="text-xs text-accent-primary hover:underline"
        >
          Revert to engine pick
        </button>
      )}

      <div className="flex gap-2 mt-2">
        <button
          onClick={onCopy}
          className="flex items-center gap-2 px-4 py-2 bg-accent-primary text-white rounded-md text-sm font-medium hover:opacity-90"
        >
          {copied ? <Check className="w-4 h-4" /> : <Copy className="w-4 h-4" />}
          {copied ? 'Copied' : 'Copy'}
        </button>
        <button
          onClick={onReset}
          className="px-4 py-2 bg-surface-light rounded-md text-sm hover:bg-white/10"
        >
          Analyze another
        </button>
      </div>
    </div>
  );
}
