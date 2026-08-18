import type { CamelotPosition, Track } from '../types';

// Camelot wheel constants
export const CAMELOT_NUMBERS = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12] as const;
export const CAMELOT_LETTERS = ['A', 'B'] as const;

// Standard key to Camelot mapping
const STANDARD_TO_CAMELOT: Record<string, string> = {
  'C major': '8B',
  'G major': '9B',
  'D major': '10B',
  'A major': '11B',
  'E major': '12B',
  'B major': '1B',
  'F# major': '2B',
  'C# major': '3B',
  'Ab major': '4B',
  'Eb major': '5B',
  'Bb major': '6B',
  'F major': '7B',
  'A minor': '8A',
  'E minor': '9A',
  'B minor': '10A',
  'F# minor': '11A',
  'C# minor': '12A',
  'G# minor': '1A',
  'D# minor': '2A',
  'A# minor': '3A',
  'F minor': '4A',
  'C minor': '5A',
  'G minor': '6A',
  'D minor': '7A',
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

// Get compatible Camelot positions based on harmonic mixing rules
export function getCompatiblePositions(pos: CamelotPosition): CamelotPosition[] {
  const compatibles: CamelotPosition[] = [];
  
  // Same key
  compatibles.push({ number: pos.number, letter: pos.letter });
  
  // +1 (clockwise)
  compatibles.push({ number: ((pos.number % 12) + 1), letter: pos.letter });
  
  // -1 (counter-clockwise)
  compatibles.push({ number: pos.number === 1 ? 12 : pos.number - 1, letter: pos.letter });
  
  // +2 (energy boost)
  compatibles.push({ number: ((pos.number + 1) % 12) + 1, letter: pos.letter });
  
  // -2
  compatibles.push({ number: pos.number <= 2 ? 12 + pos.number - 2 : pos.number - 2, letter: pos.letter });
  
  // Relative major/minor (A <-> B)
  compatibles.push({ number: pos.number, letter: pos.letter === 'A' ? 'B' : 'A' });
  
  return compatibles;
}

/**
 * Semantic label for the relationship between a reference Camelot position
 * and another position. Used by the wheel overlay and the playlist hint UI.
 *
 * Categories follow the standard Camelot mixing rules a DJ uses to build a set:
 *  - `same`        — exact match (energy-preserving identical-key blend)
 *  - `plus_one`    — clockwise step on the wheel (smooth, slightly brighter)
 *  - `minus_one`   — counter-clockwise step (smooth, slightly darker)
 *  - `plus_two`    — two clockwise (the classic "energy boost")
 *  - `minus_two`   — two counter-clockwise ("energy drop")
 *  - `mood_shift`  — A <-> B at the same number (relative minor/major, mood shift)
 *  - `incompatible` — none of the above; will clash without a transition
 */
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
  label: string;       // short, e.g. "+2 energy boost"
  description: string; // longer, e.g. "Two clockwise on the wheel — the classic energy boost."
  color: string;       // tailwind/CSS color for badges
}

/** Lookup table for relationship presentation. */
export const RELATIONSHIP_INFO: Record<CamelotRelationship, Omit<CamelotRelationshipInfo, 'kind'>> = {
  same:         { label: 'Same key',       description: 'Identical key. Seamless blend, no energy change.',                    color: '#22c55e' },
  plus_one:     { label: '+1 smooth',      description: 'One step clockwise on the wheel. Smooth, slightly brighter.',          color: '#84cc16' },
  minus_one:    { label: '-1 smooth',      description: 'One step counter-clockwise. Smooth, slightly darker.',                 color: '#84cc16' },
  plus_two:     { label: '+2 energy boost', description: 'Two steps clockwise. Classic energy lift used to build a set.',       color: '#f59e0b' },
  minus_two:    { label: '-2 energy drop', description: 'Two steps counter-clockwise. Wind-down move.',                         color: '#3b82f6' },
  mood_shift:   { label: 'A\u2194B mood shift', description: 'Same number, swap letter. Relative minor/major \u2014 mood shift.', color: '#a855f7' },
  incompatible: { label: 'Clash',          description: 'No standard Camelot relationship. Mix with care or use a bridge.',      color: '#6b7280' },
};

/**
 * Classify the relationship between `from` and `to` on the Camelot wheel.
 *
 * Distance on the wheel is computed modulo 12 in both directions.
 */
export function getRelationship(from: CamelotPosition, to: CamelotPosition): CamelotRelationship {
  if (from.number === to.number && from.letter === to.letter) return 'same';
  if (from.number === to.number && from.letter !== to.letter) return 'mood_shift';

  if (from.letter !== to.letter) return 'incompatible';

  const cw = (to.number - from.number + 12) % 12;   // clockwise distance
  const ccw = (from.number - to.number + 12) % 12;  // counter-clockwise distance

  if (cw === 1) return 'plus_one';
  if (ccw === 1) return 'minus_one';
  if (cw === 2) return 'plus_two';
  if (ccw === 2) return 'minus_two';
  return 'incompatible';
}

/** Full info object for a relationship. */
export function getRelationshipInfo(from: CamelotPosition, to: CamelotPosition): CamelotRelationshipInfo {
  const kind = getRelationship(from, to);
  return { kind, ...RELATIONSHIP_INFO[kind] };
}

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
  
  // Same key = strongest
  if (pos1.number === pos2.number && pos1.letter === pos2.letter) return 3;
  
  // Relative major/minor
  if (pos1.number === pos2.number && pos1.letter !== pos2.letter) return 2;
  
  // Adjacent on wheel (+/-1)
  const diff = Math.abs(pos1.number - pos2.number);
  if ((diff === 1 || diff === 11) && pos1.letter === pos2.letter) return 2;
  
  // +/-2
  if ((diff === 2 || diff === 10) && pos1.letter === pos2.letter) return 1;
  
  return 0;
}

export function formatCamelotBadge(camelot: string): { text: string; color: string } {
  const pos = parseCamelot(camelot);
  if (!pos) return { text: camelot, color: '#666' };
  
  // Hue based on position (0-360)
  const hue = ((pos.number - 1) * 30) % 360;
  const saturation = pos.letter === 'A' ? '70%' : '50%';
  const color = `hsl(${hue}, ${saturation}, 50%)`;
  
  return { text: camelot, color };
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

// ============================================================================
// Music-theory helpers (for the Tuner's educational panels)
// ============================================================================

/** Sharp spelling — used as the canonical pitch-class index. */
export const PITCH_NAMES_SHARP = ['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B'] as const;
/** Flat spelling — preferred for keys with flats in their signature. */
export const PITCH_NAMES_FLAT = ['C', 'Db', 'D', 'Eb', 'E', 'F', 'Gb', 'G', 'Ab', 'A', 'Bb', 'B'] as const;

/**
 * Keys whose signature is written with flats (in standard notation).
 * Includes both modes — natural minor inherits its relative major's sig.
 */
const FLAT_KEY_CAMELOT = new Set(['4B', '5B', '6B', '7B', '1A', '2A', '3A', '4A']);

/**
 * Whether this key is conventionally written with flats. The choice between
 * "Bb" and "A#" depends on the key signature, not on the pitch class alone.
 */
export function keyUsesFlats(camelot: string): boolean {
  return FLAT_KEY_CAMELOT.has(camelot);
}

/** Major scale intervals in semitones from the tonic. W-W-H-W-W-W-H. */
const MAJOR_INTERVALS = [0, 2, 4, 5, 7, 9, 11] as const;
/** Natural minor scale intervals. W-H-W-W-H-W-W. */
const MINOR_INTERVALS = [0, 2, 3, 5, 7, 8, 10] as const;

const MAJOR_DEGREES = ['1', '2', '3', '4', '5', '6', '7'] as const;
const MINOR_DEGREES = ['1', '2', '\u266d3', '4', '5', '\u266d6', '\u266d7'] as const;

/**
 * Resolve a Camelot key string to its `(tonic, isMajor)` pair.
 * Returns null if the string isn't a valid Camelot code.
 */
export function camelotToTonic(camelot: string): { tonic: number; isMajor: boolean } | null {
  const std = camelotToStandardKey(camelot);
  if (!std) return null;
  const isMajor = std.endsWith('major');
  const tonicName = std.replace(/\s+(major|minor)$/, '');
  // Convert "Ab", "Bb" etc. (flat) to pitch class via lookup.
  const flatIdx = (PITCH_NAMES_FLAT as readonly string[]).indexOf(tonicName);
  const sharpIdx = (PITCH_NAMES_SHARP as readonly string[]).indexOf(tonicName);
  const tonic = flatIdx >= 0 ? flatIdx : sharpIdx;
  if (tonic < 0) return null;
  return { tonic, isMajor };
}

export interface ScaleNote {
  /** 0..11 pitch class index. */
  pitchClass: number;
  /** Preferred spelling for this key (Bb vs A#). */
  name: string;
  /** Enharmonic alternative if it differs (A# when name is Bb, etc.). */
  altName?: string;
  /** Scale-degree label (e.g. "1", "\u266d3"). */
  degree: string;
  /** MIDI number at octave 4 (so the piano can play it). */
  midi: number;
  /** Reference frequency at octave 4, in Hz. */
  frequency: number;
}

/**
 * The 7 notes of the scale defined by a Camelot key, in ascending order
 * starting from the tonic. Includes spelling, degree, MIDI, frequency.
 *
 * Used by the Tuner wheel-hover panel and the piano-roll highlight overlay
 * so the user can SEE which notes belong to the key and HEAR them played.
 */
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
    const midi = 60 + pitchClass; // C4 = 60
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

/**
 * Equal-temperament MIDI -> frequency. A4 (MIDI 69) = 440 Hz.
 */
export function midiToFrequency(midi: number): number {
  return 440 * Math.pow(2, (midi - 69) / 12);
}

/**
 * Reference frequencies for each of the 12 pitch classes at octave 4.
 * Surfaced under each bin of the Tuner's chroma chart so the user learns
 * which Hz value corresponds to each pitch class.
 */
// ============================================================================
// Key-detection ambiguity relationships
// ============================================================================

/** Parse a standard key name like "A minor" or "C major" into pitch-class and mode. */
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
  same:           { label: 'Same key',     description: 'Identical key and mode.' },
  relative:       { label: 'Relative',     description: 'Same Camelot number, A↔B. Major/minor pair.' },
  parallel:       { label: 'Parallel',     description: 'Same tonic pitch class, different mode (e.g., C major vs C minor).' },
  dominant:       { label: 'Dominant',     description: "The dominant (V) of the winner. Common confusion in detection." },
  subdominant:    { label: 'Subdominant',  description: 'The subdominant (IV) of the winner.' },
  same_tonic_name:{ label: 'Same tonic',   description: 'Same note name, different mode (e.g., G# minor vs Ab major).' },
  enharmonic:     { label: 'Enharmonic',   description: 'Same pitch class, different spelling (e.g., D# minor vs Eb minor).' },
  close_wheel:    { label: 'Close wheel',  description: 'Adjacent on the Camelot wheel (+1/-1 step).' },
  distant:        { label: '',             description: 'No standard relationship detected.' },
};

/**
 * Classify how `otherKey` relates to `winnerKey` from a *detection ambiguity*
 * perspective. This helps the UI explain WHY a runner-up appeared.
 */
export function getKeyAmbiguityRelationship(
  winnerKey: string,
  otherKey: string,
): KeyAmbiguityInfo {
  if (winnerKey === otherKey) {
    return { kind: 'same', ...AMBIGUITY_LABELS.same };
  }

  const w = parseStandardKey(winnerKey);
  const o = parseStandardKey(otherKey);

  // Fallback: use Camelot wheel proximity when names don't parse
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

  // Same tonic pitch class, different mode → parallel
  if (w.tonic === o.tonic && w.isMajor !== o.isMajor) {
    return { kind: 'parallel', ...AMBIGUITY_LABELS.parallel };
  }

  // Enharmonic spelling check: same pitch class, different name
  // (e.g., D# minor = pitch class 3, Eb minor = pitch class 3)
  const wCamelot = standardKeyToCamelot(winnerKey);
  const oCamelot = standardKeyToCamelot(otherKey);
  if (wCamelot && oCamelot) {
    const wp = parseCamelot(wCamelot);
    const op = parseCamelot(oCamelot);
    if (wp && op) {
      // Same Camelot number, different letter → relative major/minor
      if (wp.number === op.number && wp.letter !== op.letter) {
        return { kind: 'relative', ...AMBIGUITY_LABELS.relative };
      }

      // Adjacent on the wheel (same ring)
      if (wp.letter === op.letter) {
        const diff = Math.abs(wp.number - op.number);
        if (diff === 1 || diff === 11) {
          return { kind: 'close_wheel', ...AMBIGUITY_LABELS.close_wheel };
        }
      }
    }
  }

  // Dominant = +7 semitones (perfect fifth)
  const tonicDiff = (o.tonic + 12 - w.tonic) % 12;
  if (tonicDiff === 7 && w.isMajor === o.isMajor) {
    return { kind: 'dominant', ...AMBIGUITY_LABELS.dominant };
  }
  // Subdominant = +5 semitones (perfect fourth)
  if (tonicDiff === 5 && w.isMajor === o.isMajor) {
    return { kind: 'subdominant', ...AMBIGUITY_LABELS.subdominant };
  }

  // Same tonic name (e.g., G# minor vs G# major) — enharmonic edge case
  const wName = PITCH_NAMES_SHARP[w.tonic];
  const oName = PITCH_NAMES_SHARP[o.tonic];
  if (wName === oName && w.isMajor !== o.isMajor) {
    return { kind: 'same_tonic_name', ...AMBIGUITY_LABELS.same_tonic_name };
  }

  // If Camelot positions match (enharmonic equivalents like D# minor vs some other)
  if (wCamelot && oCamelot && wCamelot === oCamelot) {
    return { kind: 'enharmonic', ...AMBIGUITY_LABELS.enharmonic };
  }

  return { kind: 'distant', ...AMBIGUITY_LABELS.distant };
}

export function pitchClassFrequencies(): { name: string; frequency: number }[] {
  return PITCH_NAMES_SHARP.map((name, pc) => ({
    name,
    frequency: midiToFrequency(60 + pc),
  }));
}
