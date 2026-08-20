import type { ConsensusResult } from '../../lib/tauri';

interface ConsensusDotsProps {
  consensus: ConsensusResult | null;
}

const SOURCE_COLORS: Record<string, string> = {
  tunelock: '#a78bfa', // purple
  mik: '#3b82f6',      // blue
  traktor: '#ef4444',  // red
  acoustid: '#10b981', // green
};

const SOURCE_LABELS: Record<string, string> = {
  tunelock: 'TuneLock',
  mik: 'MIK',
  traktor: 'Traktor',
  acoustid: 'AcoustID',
};

/**
 * Four-dot consensus indicator. Each dot represents a source that has an
 * opinion on this track. Filled = source has an opinion, empty = no opinion.
 * Color matches the source. When all filled dots are the same color
 * (agreement), a subtle ring appears. When sources disagree, the dots pulse.
 */
export default function ConsensusDots({ consensus }: ConsensusDotsProps) {
  if (!consensus || consensus.status === 'unknown') {
    return <div className="flex gap-0.5 w-12 justify-center" title="No opinions available" />;
  }

  const sources = ['tunelock', 'mik', 'traktor', 'acoustid'];
  const opinionMap = new Map(
    consensus.opinions.map((o) => [o.source as string, o])
  );

  const isContested = consensus.status === 'contested';

  return (
    <div
      className="flex gap-0.5 w-12 justify-center"
      title={buildTooltip(consensus)}
    >
      {sources.map((src) => {
        const opinion = opinionMap.get(src);
        const hasOpinion = !!opinion;
        const color = SOURCE_COLORS[src];
        const agrees = hasOpinion && consensus.consensusKey === opinion!.keyCamelot;

        return (
          <div
            key={src}
            className={`
              w-1.5 h-1.5 rounded-full transition-all
              ${hasOpinion ? '' : 'opacity-20'}
              ${isContested && hasOpinion && !agrees ? 'animate-pulse' : ''}
            `}
            style={{
              backgroundColor: hasOpinion ? color : '#444',
              boxShadow: consensus.status === 'agreed' && hasOpinion
                ? `0 0 2px ${color}`
                : 'none',
            }}
          />
        );
      })}
    </div>
  );
}

function buildTooltip(consensus: ConsensusResult): string {
  if (consensus.status === 'unknown') return 'No opinions available';
  if (consensus.status === 'single') return 'Only one source has an opinion';

  const parts: string[] = [];
  for (const op of consensus.opinions) {
    const label = SOURCE_LABELS[op.source] ?? op.source;
    const key = op.keyCamelot ?? 'no key';
    const bpm = op.bpm ? ` ${op.bpm.toFixed(1)} BPM` : '';
    parts.push(`${label}: ${key}${bpm}`);
  }

  const statusLabel =
    consensus.status === 'agreed' ? 'All sources agree' :
    consensus.status === 'contested' ? 'Sources disagree' :
    'Single source';

  return `${statusLabel} (${consensus.keyAgreement}/${consensus.sourceCount} agree on key)\n${parts.join('\n')}`;
}
