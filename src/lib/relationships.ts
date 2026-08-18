import { parseCamelot } from './camelot';

export type RelationshipType =
  | 'same-key'
  | 'neighbor'
  | 'mood-shift'
  | 'energy-boost'
  | 'energy-drop'
  | 'tension'
  | 'bridge-needed'
  | 'unknown';

export interface HarmonicRelationship {
  type: RelationshipType;
  score: number; // 0-100
  label: string;
  explanation: string;
  risk: 'low' | 'medium' | 'high';
  bpmDeltaPercent: number; // percentage difference in BPM
}

const RELATIONSHIP_META: Record<RelationshipType, { label: string; explanation: string; risk: 'low' | 'medium' | 'high'; baseScore: number }> = {
  'same-key':       { label: 'Same key',      explanation: 'Stable, safe, seamless harmonic blend.', risk: 'low', baseScore: 95 },
  'neighbor':       { label: 'Neighbor move', explanation: 'Smooth harmonic movement — one step on the wheel.', risk: 'low', baseScore: 85 },
  'mood-shift':     { label: 'Mood shift',    explanation: 'Same tonic, major/minor swap. Changes emotional flavor.', risk: 'low', baseScore: 80 },
  'energy-boost':   { label: 'Energy boost',  explanation: 'Two steps clockwise — a classic lift for building energy.', risk: 'medium', baseScore: 75 },
  'energy-drop':    { label: 'Energy drop',   explanation: 'Two steps counter-clockwise — useful for cooldown or reset.', risk: 'medium', baseScore: 70 },
  'tension':        { label: 'Tension jump',  explanation: 'Distant on the wheel — risky but can be creative.', risk: 'high', baseScore: 40 },
  'bridge-needed':  { label: 'Bridge needed', explanation: 'Far apart on the wheel. Consider an intermediate track.', risk: 'high', baseScore: 20 },
  'unknown':        { label: 'Unknown',       explanation: 'One or both keys are missing or invalid.', risk: 'high', baseScore: 0 },
};

/**
 * Compute the harmonic relationship between two Camelot keys.
 * Returns a rich description including risk level and a composite score.
 */
export function getCamelotRelationship(
  fromKey: string,
  toKey: string,
  fromBpm?: number,
  toBpm?: number,
): HarmonicRelationship {
  const from = parseCamelot(fromKey);
  const to = parseCamelot(toKey);

  if (!from || !to) {
    return makeRel('unknown', fromBpm, toBpm);
  }

  // Same key
  if (from.number === to.number && from.letter === to.letter) {
    return makeRel('same-key', fromBpm, toBpm);
  }

  // Mood shift: same number, A/B swap
  if (from.number === to.number && from.letter !== to.letter) {
    return makeRel('mood-shift', fromBpm, toBpm);
  }

  // Must be same ring (both A or both B) for wheel moves
  if (from.letter === to.letter) {
    const cw = (to.number - from.number + 12) % 12;
    const ccw = (from.number - to.number + 12) % 12;

    if (cw === 1 || ccw === 1) {
      return makeRel('neighbor', fromBpm, toBpm);
    }
    if (cw === 2) {
      return makeRel('energy-boost', fromBpm, toBpm);
    }
    if (ccw === 2) {
      return makeRel('energy-drop', fromBpm, toBpm);
    }
  }

  // Anything else that's still on the same ring but >2 away is tension
  if (from.letter === to.letter) {
    return makeRel('tension', fromBpm, toBpm);
  }

  // Cross-ring, different number = bridge needed
  return makeRel('bridge-needed', fromBpm, toBpm);
}

function makeRel(
  type: RelationshipType,
  fromBpm?: number,
  toBpm?: number,
): HarmonicRelationship {
  const meta = RELATIONSHIP_META[type];

  let bpmDeltaPercent = 0;
  if (typeof fromBpm === 'number' && typeof toBpm === 'number' && fromBpm > 0) {
    bpmDeltaPercent = ((toBpm - fromBpm) / fromBpm) * 100;
  }

  // Score penalty for large BPM jumps
  let score = meta.baseScore;
  const bpmAbs = Math.abs(bpmDeltaPercent);
  if (bpmAbs > 6) {
    score -= Math.min(20, Math.round((bpmAbs - 6) * 2));
  }
  score = Math.max(0, Math.min(100, score));

  // Augment explanation with BPM note when relevant
  let explanation = meta.explanation;
  if (bpmAbs > 3) {
    const dir = bpmDeltaPercent > 0 ? '+' : '';
    explanation += ` BPM difference ${dir}${bpmDeltaPercent.toFixed(1)}%.`;
  }

  return {
    type,
    score,
    label: meta.label,
    explanation,
    risk: meta.risk,
    bpmDeltaPercent,
  };
}

/**
 * Color coding for relationship badges.
 */
export function relationshipColor(type: RelationshipType): string {
  switch (type) {
    case 'same-key':      return '#22c55e'; // green
    case 'neighbor':      return '#84cc16'; // lime
    case 'mood-shift':    return '#a855f7'; // purple
    case 'energy-boost':  return '#f59e0b'; // amber
    case 'energy-drop':   return '#3b82f6'; // blue
    case 'tension':       return '#ef4444'; // red
    case 'bridge-needed': return '#6b7280'; // gray
    case 'unknown':       return '#444444';
  }
}
