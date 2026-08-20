// ContextInspector — the right-side panel showing musical intelligence.
//
// Rather than one opaque compatibility score, this visualizes separate
// musical dimensions:
//   - Harmonic relationship
//   - Beat and phrase alignment
//   - Bass conflict
//   - Lead-vocal overlap
//   - Transient density
//   - Spectral crowding
//   - Available headroom
//   - Energy contribution
//   - Local-key changes
//   - Beat-grid confidence
//
// Design language: Walnut Console — uses the charcoal data plane for
// the dimension readouts, framed by the bronze plate.

import { useMixStore } from '../../stores/mixStore';
import { useLibraryStore } from '../../stores/libraryStore';
import { parseCamelot, getRelationshipInfo } from '../../lib/harmony';

interface Dimension {
  label: string;
  value: string;
  status: 'good' | 'ok' | 'warn' | 'risk';
  detail?: string;
}

export default function ContextInspector() {
  const { project } = useMixStore();
  const { tracks } = useLibraryStore();
  const selectedTrans = project.transitions.find(t => t.id === project.selectedTransitionId);

  if (!selectedTrans) {
    return (
      <div className="bronze-plate p-3 h-full flex flex-col">
        <div className="engraved-label mb-2">Context Inspector</div>
        <div className="text-xs text-cream-label/40 flex-1 flex items-center justify-center">
          Select a transition to see compatibility analysis.
        </div>
      </div>
    );
  }

  const fromClip = project.clips.find(c => c.id === selectedTrans.fromClipId);
  const toClip = project.clips.find(c => c.id === selectedTrans.toClipId);
  const fromTrack = fromClip ? tracks.get(fromClip.trackId) : null;
  const toTrack = toClip ? tracks.get(toClip.trackId) : null;

  if (!fromTrack || !toTrack) {
    return (
      <div className="bronze-plate p-3 h-full flex flex-col">
        <div className="engraved-label mb-2">Context Inspector</div>
        <div className="text-xs text-cream-label/40 flex-1 flex items-center justify-center">
          Missing track data for this transition.
        </div>
      </div>
    );
  }

  // Compute dimensions
  const dimensions: Dimension[] = [];

  // Harmonic relationship
  if (fromTrack.key_camelot && toTrack.key_camelot) {
    const fromPos = parseCamelot(fromTrack.key_camelot);
    const toPos = parseCamelot(toTrack.key_camelot);
    if (fromPos && toPos) {
      const rel = getRelationshipInfo(fromPos, toPos);
      const riskLevel: 'good' | 'ok' | 'risk' =
        rel.kind === 'same' || rel.kind === 'plus_one' || rel.kind === 'minus_one' ? 'good' :
        rel.kind === 'mood_shift' || rel.kind === 'plus_two' || rel.kind === 'minus_two' ? 'ok' : 'risk';
      dimensions.push({
        label: 'Harmonic',
        value: rel.label,
        status: riskLevel,
        detail: rel.description,
      });
    } else {
      dimensions.push({
        label: 'Harmonic',
        value: 'Unknown',
        status: 'warn',
        detail: 'Could not parse one or both keys.',
      });
    }
  } else {
    dimensions.push({
      label: 'Harmonic',
      value: 'Unknown',
      status: 'warn',
      detail: 'One or both keys missing.',
    });
  }

  // BPM compatibility
  if (fromTrack.bpm && toTrack.bpm) {
    const bpmDiff = Math.abs(fromTrack.bpm - toTrack.bpm);
    const pct = bpmDiff / Math.min(fromTrack.bpm, toTrack.bpm);
    if (pct < 0.02) {
      dimensions.push({ label: 'Tempo', value: `${bpmDiff.toFixed(1)} BPM`, status: 'good', detail: 'Nearly identical tempo.' });
    } else if (pct < 0.08) {
      dimensions.push({ label: 'Tempo', value: `${bpmDiff.toFixed(1)} BPM`, status: 'ok', detail: 'Small tempo shift — pitch-preserving stretch recommended.' });
    } else {
      dimensions.push({ label: 'Tempo', value: `${bpmDiff.toFixed(1)} BPM`, status: 'warn', detail: 'Significant tempo change.' });
    }
  } else {
    dimensions.push({ label: 'Tempo', value: '—', status: 'warn' });
  }

  // Energy change
  if (fromTrack.energy_level && toTrack.energy_level) {
    const delta = toTrack.energy_level - fromTrack.energy_level;
    const status = delta > 2 ? 'good' : delta < -2 ? 'ok' : 'ok';
    dimensions.push({
      label: 'Energy',
      value: `${delta > 0 ? '+' : ''}${delta.toFixed(1)}`,
      status,
      detail: delta > 0 ? 'Energy lift.' : delta < 0 ? 'Energy drop.' : 'Energy maintained.',
    });
  } else {
    dimensions.push({ label: 'Energy', value: '—', status: 'warn' });
  }

  // Bass conflict (placeholder — needs spectral analysis)
  dimensions.push({
    label: 'Bass Conflict',
    value: 'Check by ear',
    status: 'ok',
    detail: 'Bass overlap analysis requires stems or spectral data.',
  });

  // Vocal overlap (placeholder)
  dimensions.push({
    label: 'Vocal Overlap',
    value: 'Check by ear',
    status: 'ok',
    detail: 'Vocal collision analysis requires stems or vocal detection.',
  });

  // Spectral crowding (placeholder)
  dimensions.push({
    label: 'Spectral Crowding',
    value: 'Unknown',
    status: 'warn',
    detail: 'Requires multi-source spectral analysis.',
  });

  // Headroom (placeholder)
  dimensions.push({
    label: 'Headroom',
    value: '~-6 dB',
    status: 'good',
    detail: 'Estimated for two sources. Four layers may need -3 dB more.',
  });

  // Beat-grid confidence (placeholder)
  dimensions.push({
    label: 'Beat Grid',
    value: 'Not analyzed',
    status: 'warn',
    detail: 'Run beat-grid detection for alignment information.',
  });

  return (
    <div className="bronze-plate p-3 h-full flex flex-col gap-2 overflow-auto scrollbar-thin">
      <div className="engraved-label">Context Inspector</div>

      {/* Transition header */}
      <div className="data-plane p-2">
        <div className="text-[10px] text-data-text-dim uppercase mb-1">Transition</div>
        <div className="text-xs text-data-text truncate">{fromTrack.title || fromTrack.filename}</div>
        <div className="text-[10px] text-data-text-dim text-center my-1">↓</div>
        <div className="text-xs text-data-text truncate">{toTrack.title || toTrack.filename}</div>
      </div>

      {/* Dimensions */}
      <div className="flex flex-col gap-1.5">
        {dimensions.map((dim, i) => (
          <DimensionRow key={i} dim={dim} />
        ))}
      </div>

      {/* Summary */}
      <div className="data-plane p-2 mt-1">
        <div className="text-[10px] text-data-text-dim uppercase mb-1">Summary</div>
        <div className="text-[11px] text-data-text leading-relaxed">
          {dimensions.find(d => d.status === 'risk') ? (
            'This transition has risks. Listen carefully before committing.'
          ) : dimensions.find(d => d.status === 'warn') ? (
            'Some information is missing. The transition may work but cannot be fully evaluated.'
          ) : (
            'This transition looks compatible. Trust your ears to confirm.'
          )}
        </div>
      </div>
    </div>
  );
}

function DimensionRow({ dim }: { dim: Dimension }) {
  const statusColor = {
    good: 'var(--lamp-green)',
    ok: 'var(--brass-accent)',
    warn: 'var(--lamp-amber)',
    risk: 'var(--lamp-red)',
  }[dim.status];

  return (
    <div className="data-plane p-2">
      <div className="flex items-center justify-between">
        <span className="text-[10px] text-data-text-dim uppercase">{dim.label}</span>
        <div className="flex items-center gap-1.5">
          <span className="w-1.5 h-1.5 rounded-full" style={{ backgroundColor: statusColor }} />
          <span className="text-[11px] text-data-text font-mono">{dim.value}</span>
        </div>
      </div>
      {dim.detail && (
        <div className="text-[10px] text-data-text-dim mt-0.5">{dim.detail}</div>
      )}
    </div>
  );
}
