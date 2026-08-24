import type { KeyCandidate } from '../../types';
import { formatCamelotBadge, getKeyAmbiguityRelationship } from '../../lib/harmony';

export interface CandidatesPanelProps {
  candidates: KeyCandidate[];
  sectionCount: number;
  selected: KeyCandidate;
  winnerStandard: string;
  onSelect: (c: KeyCandidate) => void;
}

export default function CandidatesPanel({
  candidates,
  sectionCount,
  selected,
  winnerStandard,
  onSelect,
}: CandidatesPanelProps) {
  return (
    <div className="bg-surface/40 rounded-xl p-4">
      <div className="flex items-center justify-between mb-3">
        <h3 className="text-sm font-semibold">Key candidates</h3>
        <div className="text-[10px] text-text-secondary">Click any row to override</div>
      </div>
      <div className="flex flex-col divide-y divide-white/5">
        {candidates.map((c, i) => {
          const isSelected =
            c.key_camelot === selected.key_camelot && c.key_standard === selected.key_standard;
          const badge = formatCamelotBadge(c.key_camelot);
          const rel = getKeyAmbiguityRelationship(winnerStandard, c.key_standard);
          return (
            <button
              key={`${c.key_camelot}-${i}`}
              onClick={() => onSelect(c)}
              className={`
                flex items-center gap-3 py-2 px-1 text-left text-sm transition-colors
                ${isSelected ? 'bg-accent-primary/10' : 'hover:bg-white/5'}
              `}
            >
              <span className="text-text-secondary w-4 text-xs">{i + 1}</span>
              <span
                className="px-2 py-0.5 rounded text-xs font-bold text-white min-w-[2.5rem] text-center"
                style={{ backgroundColor: badge.color }}
              >
                {badge.text}
              </span>
              <span className="flex-1 truncate">{c.key_standard}</span>
              {rel.label && (
                <span
                  className="text-[10px] px-1.5 py-0.5 rounded bg-white/10 text-text-secondary hidden sm:inline"
                  title={rel.description}
                >
                  {rel.label}
                </span>
              )}
              <span className="font-mono text-xs text-text-secondary">
                {c.segment_count}/{sectionCount || '—'} sections
              </span>
            </button>
          );
        })}
      </div>
    </div>
  );
}
