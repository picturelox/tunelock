/**
 * TuneLock Harmony — the single source of truth for key/Camelot/relationship
 * logic in the frontend. Mirrors `src-tauri/src/harmony/` in Rust.
 *
 * One vocabulary for mixing relationships, one for detection ambiguity.
 * No other file should define harmony types or functions.
 */

import type { CamelotPosition, Track } from '../types';

// ============================================================================
// Camelot wheel constants & mappings
// ============================================================================

export const CAMELOT_NUMBERS = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12] as const;
export const CAMELOT_LETTERS = ['A', 'B'] as const;

const STANDARD_TO_CAMELOT: Record<string, string> = {
  'C major': '8B', 'G major': '9B', 'D major': '10B', 'A major': '11B',
  'E major': '12B', 'B major': '1B', 'F# major': '2B', 'C# major': '3B',
  'Ab major': '4B', 'Eb major': '5B', 'Bb major': '6B', 'F major': '7B',
  'A minor': '8A', 'E minor': '9A', 'B minor': '10A', 'F# minor': '11A',
  'C# minor': '12A', 'G# minor': '1A', 'D# minor': '2A', 'A# minor': '3A',
  'F minor': '4A', 'C minor': '5A', 'G minor': '6A', 'D minor': '7A',
};

const CAMELOT_TO_STANDARD: Record<string, string> = {
  '8B': 'C major', '9B': 'G major', '10B': 'D major', '11B': 'A major',
  '12B': 'E major', '1B': 'B major', '2B': 'F# major', '3B': 'C# major',
  '4B': 'Ab major', '5B': 'Eb major', '6B': 'Bb major', '7B': 'F major',
  '8A': 'A minor', '9A': 'E minor', '10A': 'B minor', '11A': 'F# minor',
  '12A': 'C# minor', '1A': 'G# minor', '2A': 'D# minor', '3A': 'A# minor',
  '4A': 'F minor', '5A': 'C minor', '6A': 'G minor', '7A': 'D minor',
};

export function standardKeyToCamelot(standardKey: string): string | null {
  return STANDARD_TO_CAMELOT[standardKey] || null;
}

export function camelotToStandardKey(camelot: string): string | null {
  return CAMELOT_TO_STANDARD[camelot] || null;
}

export function parseCamelot(camelotStr: string): CamelotPosition | null {
  const match = camelotStr.match(/^([1-9]|1[0-2])([AB])$/);
  if (!match) return null;
  return {
    number: parseInt(match[1], 10),
    letter: match[2] as 'A' | 'B',
  };
}

export function getAllCamelotPositions(): CamelotPosition[] {
  const positions: CamelotPosition[] = [];
  for (const letter of CAMELOT_LETTERS) {
    for (const number of CAMELOT_NUMBERS) {
      positions.push({ number, letter: letter as 'A' | 'B' });
    }
  }
  return positions;
}

export function getCompatiblePositions(pos: CamelotPosition): CamelotPosition[] {
  const compatibles: CamelotPosition[] = [];
  compatibles.push({ number: pos.number, letter: pos.letter });
  compatibles.push({ number: ((pos.number % 12) + 1), letter: pos.letter });
  compatibles.push({ number: pos.number === 1 ? 12 : pos.number - 1, letter: pos.letter });
  compatibles.push({ number: ((pos.number + 1) % 12) + 1, letter: pos.letter });
  compatibles.push({ number: pos.number <= 2 ? 12 + pos.number - 2 : pos.number - 2, letter: pos.letter });
  compatibles.push({ number: pos.number, letter: pos.letter === 'A' ? 'B' : 'A' });
  return compatibles;
}

// ============================================================================
// Mixing relationships — the unified vocabulary
// ============================================================================

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
  score: number;
  label: string;
  explanation: string;
  risk: 'low' | 'medium' | 'high';
  bpmDeltaPercent: number;
}

const RELATIONSHIP_META: Record<RelationshipType, {
  label: string;
  explanation: string;
  risk: 'low' | 'medium' | 'high';
  baseScore: number;
  color: string;
}> = {
  'same-key':      { label: 'Same key',      explanation: 'Stable, safe, seamless harmonic blend.', risk: 'low', baseScore: 95, color: '#22c55e' },
  'neighbor':      { label: 'Neighbor move', explanation: 'Smooth harmonic movement — one step on the wheel.', risk: 'low', baseScore: 85, color: '#84cc16' },
  'mood-shift':    { label: 'Mood shift',    explanation: 'Same tonic, major/minor swap. Changes emotional flavor.', risk: 'low', baseScore: 80, color: '#a855f7' },
  'energy-boost':  { label: 'Energy boost',  explanation: 'Two steps clockwise — a classic lift for building energy.', risk: 'medium', baseScore: 75, color: '#f59e0b' },
  'energy-drop':   { label: 'Energy drop',   explanation: 'Two steps counter-clockwise — useful for cooldown or reset.', risk: 'medium', baseScore: 70, color: '#3b82f6' },
  'tension':       { label: 'Tension jump',  explanation: 'Distant on the wheel — risky but can be creative.', risk: 'high', baseScore: 40, color: '#ef4444' },
  'bridge-needed': { label: 'Bridge needed', explanation: 'Far apart on the wheel. Consider an intermediate track.', risk: 'high', baseScore: 20, color: '#6b7280' },
  'unknown':       { label: 'Unknown',       explanation: 'One or both keys are missing or invalid.', risk: 'high', baseScore: 0, color: '#444444' },
};

export function relationshipColor(type: RelationshipType): string {
  return RELATIONSHIP_META[type].color;
}

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

  if (from.number === to.number && from.letter === to.letter) {
    return makeRel('same-key', fromBpm, toBpm);
  }

  if (from.number === to.number && from.letter !== to.letter) {
    return makeRel('mood-shift', fromBpm, toBpm);
  }

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
    return makeRel('tension', fromBpm, toBpm);
  }

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

  let score = meta.baseScore;
  const bpmAbs = Math.abs(bpmDeltaPercent);
  if (bpmAbs > 6) {
    score -= Math.min(20, Math.round((bpmAbs - 6) * 2));
  }
  score = Math.max(0, Math.min(100, score));

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

// ============================================================================
// Position-based relationships (for wheel/mosaic visualization)
// ============================================================================

export type CamelotRelationship =
  | 'same'
  | 'plus_one'
  | 'minus_one'
  | 'plus_two'
  | 'minus_two'
  | 'mood_shift'
  | 'incompatible';

export interface CamelotRelationshipInfo {
  kind: CamelotRelationship;
  label: string;
  description: string;
  color: string;
}

export const RELATIONSHIP_INFO: Record<CamelotRelationship, Omit<CamelotRelationshipInfo, 'kind'>> = {
  same:         { label: 'Same key',            description: 'Identical key. Seamless blend, no energy change.',                    color: '#22c55e' },
  plus_one:     { label: '+1 smooth',           description: 'One step clockwise on the wheel. Smooth, slightly brighter.',          color: '#84cc16' },
  minus_one:    { label: '-1 smooth',           description: 'One step counter-clockwise. Smooth, slightly darker.',                 color: '#84cc16' },
  plus_two:     { label: '+2 energy boost',     description: 'Two steps clockwise. Classic energy lift used to build a set.',       color: '#f59e0b' },
  minus_two:    { label: '-2 energy drop',      description: 'Two steps counter-clockwise. Wind-down move.',                         color: '#3b82f6' },
  mood_shift:   { label: 'A\u2194B mood shift', description: 'Same number, swap letter. Relative minor/major \u2014 mood shift.', color: '#a855f7' },
  incompatible: { label: 'Clash',               description: 'No standard Camelot relationship. Mix with care or use a bridge.',      color: '#6b7280' },
};

export function getRelationship(from: CamelotPosition, to: CamelotPosition): CamelotRelationship {
  if (from.number === to.number && from.letter === to.letter) return 'same';
  if (from.number === to.number && from.letter !== to.letter) return 'mood_shift';
  if (from.letter !== to.letter) return 'incompatible';
  const cw = (to.number - from.number + 12) % 12;
  const ccw = (from.number - to.number + 12) % 12;
  if (cw === 1) return 'plus_one';
  if (ccw === 1) return 'minus_one';
  if (cw === 2) return 'plus_two';
  if (ccw === 2) return 'minus_two';
  return 'incompatible';
}

export function getRelationshipInfo(from: CamelotPosition, to: CamelotPosition): CamelotRelationshipInfo {
  const kind = getRelationship(from, to);
  return { kind, ...RELATIONSHIP_INFO[kind] };
}

// ============================================================================
// Compatibility helpers (used by library/playlist code)
// ============================================================================

export function isCompatible(track1: Track, track2: Track): boolean {
  if (!track1.key_camelot || !track2.key_camelot) return false;
  const pos1 = parseCamelot(track1.key_camelot);
  const pos2 = parseCamelot(track2.key_camelot);
  if (!pos1 || !pos2) return false;
  const compatibles = getCompatiblePositions(pos1);
  return compatibles.some(p => p.number === pos2.number && p.letter === pos2.letter);
}

export function getCompatibilityStrength(track1: Track, track2: Track): number {
  if (!track1.key_camelot || !track2.key_camelot) return 0;
  const pos1 = parseCamelot(track1.key_camelot);
  const pos2 = parseCamelot(track2.key_camelot);
  if (!pos1 || !pos2) return 0;
  if (pos1.number === pos2.number && pos1.letter === pos2.letter) return 3;
  if (pos1.number === pos2.number && pos1.letter !== pos2.letter) return 2;
  const diff = Math.abs(pos1.number - pos2.number);
  if ((diff === 1 || diff === 11) && pos1.letter === pos2.letter) return 2;
  if ((diff === 2 || diff === 10) && pos1.letter === pos2.letter) return 1;
  return 0;
}

export function formatCamelotBadge(camelot: string): { text: string; color: string } {
  const pos = parseCamelot(camelot);
  if (!pos) return { text: camelot, color: '#666' };
  const hue = ((pos.number - 1) * 30) % 360;
  const saturation = pos.letter === 'A' ? '70%' : '50%';
  return { text: camelot, color: `hsl(${hue}, ${saturation}, 50%)` };
}

// ============================================================================
// Music-theory helpers (for the Tuner's educational panels)
// ============================================================================

export const PITCH_NAMES_SHARP = ['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B'] as const;
export const PITCH_NAMES_FLAT = ['C', 'Db', 'D', 'Eb', 'E', 'F', 'Gb', 'G', 'Ab', 'A', 'Bb', 'B'] as const;

const FLAT_KEY_CAMELOT = new Set(['4B', '5B', '6B', '7B', '1A', '2A', '3A', '4A']);

export function keyUsesFlats(camelot: string): boolean {
  return FLAT_KEY_CAMELOT.has(camelot);
}

const MAJOR_INTERVALS = [0, 2, 4, 5, 7, 9, 11] as const;
const MINOR_INTERVALS = [0, 2, 3, 5, 7, 8, 10] as const;
const MAJOR_DEGREES = ['1', '2', '3', '4', '5', '6', '7'] as const;
const MINOR_DEGREES = ['1', '2', '\u266d3', '4', '5', '\u266d6', '\u266d7'] as const;

export function camelotToTonic(camelot: string): { tonic: number; isMajor: boolean } | null {
  const std = camelotToStandardKey(camelot);
  if (!std) return null;
  const isMajor = std.endsWith('major');
  const tonicName = std.replace(/\s+(major|minor)$/, '');
  const flatIdx = (PITCH_NAMES_FLAT as readonly string[]).indexOf(tonicName);
  const sharpIdx = (PITCH_NAMES_SHARP as readonly string[]).indexOf(tonicName);
  const tonic = flatIdx >= 0 ? flatIdx : sharpIdx;
  if (tonic < 0) return null;
  return { tonic, isMajor };
}

export interface ScaleNote {
  pitchClass: number;
  name: string;
  altName?: string;
  degree: string;
  midi: number;
  frequency: number;
}

export function getScaleNotes(camelot: string): ScaleNote[] {
  const t = camelotToTonic(camelot);
  if (!t) return [];
  const useFlats = keyUsesFlats(camelot);
  const intervals = t.isMajor ? MAJOR_INTERVALS : MINOR_INTERVALS;
  const degrees = t.isMajor ? MAJOR_DEGREES : MINOR_DEGREES;
  const primaryNames = useFlats ? PITCH_NAMES_FLAT : PITCH_NAMES_SHARP;
  const altNames = useFlats ? PITCH_NAMES_SHARP : PITCH_NAMES_FLAT;

  return intervals.map((interval, i) => {
    const pitchClass = (t.tonic + interval) % 12;
    const name = primaryNames[pitchClass];
    const alt = altNames[pitchClass];
    const midi = 60 + pitchClass;
    return {
      pitchClass,
      name,
      altName: alt !== name ? alt : undefined,
      degree: degrees[i],
      midi,
      frequency: midiToFrequency(midi),
    };
  });
}

export function midiToFrequency(midi: number): number {
  return 440 * Math.pow(2, (midi - 69) / 12);
}

export function pitchClassFrequencies(): { name: string; frequency: number }[] {
  return PITCH_NAMES_SHARP.map((name, pc) => ({
    name,
    frequency: midiToFrequency(60 + pc),
  }));
}

export function parseStandardKey(standardKey: string): { tonic: number; isMajor: boolean } | null {
  const match = standardKey.match(/^([A-G][#b]?)(?:\s+(major|minor))?$/i);
  if (!match) return null;
  const name = match[1];
  const mode = (match[2] || 'major').toLowerCase();
  const flatIdx = (PITCH_NAMES_FLAT as readonly string[]).indexOf(name);
  const sharpIdx = (PITCH_NAMES_SHARP as readonly string[]).indexOf(name);
  const tonic = flatIdx >= 0 ? flatIdx : sharpIdx;
  if (tonic < 0) return null;
  return { tonic, isMajor: mode === 'major' };
}

// ============================================================================
// Detection ambiguity relationships (explains WHY a runner-up appeared)
// ============================================================================

export type KeyAmbiguityRelationship =
  | 'same'
  | 'relative'
  | 'parallel'
  | 'dominant'
  | 'subdominant'
  | 'same_tonic_name'
  | 'enharmonic'
  | 'close_wheel'
  | 'distant';

export interface KeyAmbiguityInfo {
  kind: KeyAmbiguityRelationship;
  label: string;
  description: string;
}

const AMBIGUITY_LABELS: Record<KeyAmbiguityRelationship, { label: string; description: string }> = {
  same:            { label: 'Same key',     description: 'Identical key and mode.' },
  relative:        { label: 'Relative',     description: 'Same Camelot number, A\u2194B. Major/minor pair.' },
  parallel:        { label: 'Parallel',     description: 'Same tonic pitch class, different mode (e.g., C major vs C minor).' },
  dominant:        { label: 'Dominant',     description: 'The dominant (V) of the winner. Common confusion in detection.' },
  subdominant:     { label: 'Subdominant',  description: 'The subdominant (IV) of the winner.' },
  same_tonic_name: { label: 'Same tonic',   description: 'Same note name, different mode (e.g., G# minor vs Ab major).' },
  enharmonic:      { label: 'Enharmonic',   description: 'Same pitch class, different spelling (e.g., D# minor vs Eb minor).' },
  close_wheel:     { label: 'Close wheel',  description: 'Adjacent on the Camelot wheel (+1/-1 step).' },
  distant:         { label: '',             description: 'No standard relationship detected.' },
};

export function getKeyAmbiguityRelationship(
  winnerKey: string,
  otherKey: string,
): KeyAmbiguityInfo {
  if (winnerKey === otherKey) {
    return { kind: 'same', ...AMBIGUITY_LABELS.same };
  }

  const w = parseStandardKey(winnerKey);
  const o = parseStandardKey(otherKey);

  if (!w || !o) {
    const wc = standardKeyToCamelot(winnerKey);
    const oc = standardKeyToCamelot(otherKey);
    if (wc && oc) {
      const wp = parseCamelot(wc);
      const op = parseCamelot(oc);
      if (wp && op && wp.letter === op.letter) {
        const diff = Math.abs(wp.number - op.number);
        if (diff === 1 || diff === 11) {
          return { kind: 'close_wheel', ...AMBIGUITY_LABELS.close_wheel };
        }
        if (wp.number === op.number && wp.letter !== op.letter) {
          return { kind: 'relative', ...AMBIGUITY_LABELS.relative };
        }
      }
    }
    return { kind: 'distant', ...AMBIGUITY_LABELS.distant };
  }

  if (w.tonic === o.tonic && w.isMajor !== o.isMajor) {
    return { kind: 'parallel', ...AMBIGUITY_LABELS.parallel };
  }

  const wCamelot = standardKeyToCamelot(winnerKey);
  const oCamelot = standardKeyToCamelot(otherKey);
  if (wCamelot && oCamelot) {
    const wp = parseCamelot(wCamelot);
    const op = parseCamelot(oCamelot);
    if (wp && op) {
      if (wp.number === op.number && wp.letter !== op.letter) {
        return { kind: 'relative', ...AMBIGUITY_LABELS.relative };
      }
      if (wp.letter === op.letter) {
        const diff = Math.abs(wp.number - op.number);
        if (diff === 1 || diff === 11) {
          return { kind: 'close_wheel', ...AMBIGUITY_LABELS.close_wheel };
        }
      }
    }
  }

  const tonicDiff = (o.tonic + 12 - w.tonic) % 12;
  if (tonicDiff === 7 && w.isMajor === o.isMajor) {
    return { kind: 'dominant', ...AMBIGUITY_LABELS.dominant };
  }
  if (tonicDiff === 5 && w.isMajor === o.isMajor) {
    return { kind: 'subdominant', ...AMBIGUITY_LABELS.subdominant };
  }

  const wName = PITCH_NAMES_SHARP[w.tonic];
  const oName = PITCH_NAMES_SHARP[o.tonic];
  if (wName === oName && w.isMajor !== o.isMajor) {
    return { kind: 'same_tonic_name', ...AMBIGUITY_LABELS.same_tonic_name };
  }

  if (wCamelot && oCamelot && wCamelot === oCamelot) {
    return { kind: 'enharmonic', ...AMBIGUITY_LABELS.enharmonic };
  }

  return { kind: 'distant', ...AMBIGUITY_LABELS.distant };
}
