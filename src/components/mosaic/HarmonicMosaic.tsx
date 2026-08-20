import { useMemo } from 'react';
import type { Track } from '../../types';
import {
  parseCamelot,
  getRelationship,
  RELATIONSHIP_INFO,
  type CamelotRelationship,
} from '../../lib/harmony';
import TrackArtwork from '../artwork/TrackArtwork';

/**
 * Harmonic Mosaic — visual neighborhood of a focal track.
 *
 * Layout philosophy:
 *   - The focal track sits at the center as a large tile, color-ringed in
 *     its Camelot color.
 *   - Library tracks that have a real harmonic relationship to it
 *     ("same", "+1", "-1", "+2", "-2", "mood_shift") are grouped into
 *     **relationship buckets**, each rendered as a labeled row with its own
 *     swatch color from `RELATIONSHIP_INFO`.
 *   - Within a row, tiles are sorted by BPM proximity to the focal track
 *     (closest first) so the most "mixable" candidates are leftmost.
 *
 * Why rows instead of a literal radial wheel layout:
 *   - Producers scanning for a next track read top-to-bottom much faster
 *     than rotating around a hub.
 *   - Rows scale gracefully with library size; a circular layout becomes
 *     unreadable past ~12 tracks per relationship.
 *   - The Camelot wheel above already provides the radial mental model.
 *     The mosaic is the **detail view** of one wheel position, not a
 *     duplicate of the wheel.
 *
 * Interaction:
 *   - Hovering a tile fires `onHoverCandidate(camelot)` so the parent can
 *     light up the same wedge on the wheel — a single visual language across
 *     both surfaces.
 *   - Clicking a tile emits `onSelectCandidate(track)`. Parent decides what
 *     that means (re-center the mosaic, queue the track, etc.).
 */
export interface HarmonicMosaicProps {
  /** The focal track. Can be a real Track or a synthetic Track-like object
   *  built from a Tuner result that hasn't been saved yet. */
  focal: FocalTrack;
  /** Pool of candidate tracks to draw from (typically the library page). */
  library: Track[];
  /** Show this many candidates per relationship row, max. */
  perRowLimit?: number;
  /** Hover handler, lets parent coordinate with the Camelot wheel. */
  onHoverCandidate?: (camelot: string | null) => void;
  /** Click handler. */
  onSelectCandidate?: (track: Track) => void;
}

export interface FocalTrack {
  id?: number;
  /** Camelot code, e.g. "8A". Required for the layout to mean anything. */
  key_camelot: string;
  bpm: number | null;
  title: string | null;
  artist: string | null;
  filename: string | null;
  artwork_path: string | null;
  /** 12-bin chroma vector for the fallback swatch. Optional. */
  chroma?: number[] | null;
}

/** Relationships shown in the mosaic, in the user-friendly visual order. */
const VISIBLE_RELATIONSHIPS: CamelotRelationship[] = [
  'same',
  'plus_one',
  'minus_one',
  'mood_shift',
  'plus_two',
  'minus_two',
];

export default function HarmonicMosaic({
  focal,
  library,
  perRowLimit = 8,
  onHoverCandidate,
  onSelectCandidate,
}: HarmonicMosaicProps) {
  const focalPos = parseCamelot(focal.key_camelot);

  // Group library tracks by their relationship to the focal track.
  const groups = useMemo(() => {
    if (!focalPos) return new Map<CamelotRelationship, Track[]>();

    const buckets = new Map<CamelotRelationship, Track[]>();
    for (const rel of VISIBLE_RELATIONSHIPS) buckets.set(rel, []);

    for (const t of library) {
      if (!t.key_camelot || t.id === focal.id) continue;
      const pos = parseCamelot(t.key_camelot);
      if (!pos) continue;
      const rel = getRelationship(focalPos, pos);
      if (rel === 'incompatible') continue;
      const bucket = buckets.get(rel);
      if (bucket) bucket.push(t);
    }

    // Sort each bucket by BPM proximity (NaN safe).
    const focalBpm = focal.bpm ?? null;
    for (const [, ts] of buckets) {
      ts.sort((a, b) => {
        const da = focalBpm == null || a.bpm == null ? Infinity : Math.abs(a.bpm - focalBpm);
        const db = focalBpm == null || b.bpm == null ? Infinity : Math.abs(b.bpm - focalBpm);
        return da - db;
      });
    }
    return buckets;
  }, [focalPos, library, focal.id, focal.bpm]);

  if (!focalPos) {
    return (
      <div className="bg-surface/40 rounded-2xl p-6 text-sm text-text-secondary">
        Mosaic needs a valid Camelot key. (Got: {focal.key_camelot ?? 'none'})
      </div>
    );
  }

  const totalCandidates = Array.from(groups.values()).reduce((n, ts) => n + ts.length, 0);

  return (
    <div className="bg-surface/40 rounded-2xl p-5">
      <header className="flex items-baseline justify-between mb-4">
        <h3 className="text-sm font-semibold">Harmonic Mosaic</h3>
        <span className="text-[11px] text-text-secondary">
          {totalCandidates} compatible tracks in your library
        </span>
      </header>

      <div className="flex flex-col lg:flex-row gap-6">
        {/* Focal track — the anchor of the whole view */}
        <div className="flex flex-col items-center shrink-0">
          <span className="text-[10px] uppercase tracking-wider text-text-secondary mb-2">
            Now exploring
          </span>
          <TrackArtwork
            artworkPath={focal.artwork_path ?? null}
            camelot={focal.key_camelot}
            chroma={focal.chroma ?? null}
            label={focal.title ?? focal.filename ?? 'Untitled'}
            sublabel={focal.artist ?? (focal.bpm ? `${focal.bpm.toFixed(1)} BPM` : null)}
            size={144}
            ringStyle="camelot"
          />
        </div>

        {/* Relationship rows */}
        <div className="flex-1 min-w-0 flex flex-col gap-3">
          {VISIBLE_RELATIONSHIPS.map((rel) => {
            const tracks = groups.get(rel) ?? [];
            const info = RELATIONSHIP_INFO[rel];
            return (
              <RelationshipRow
                key={rel}
                title={info.label}
                description={info.description}
                color={info.color}
                tracks={tracks}
                limit={perRowLimit}
                onHover={onHoverCandidate}
                onSelect={onSelectCandidate}
              />
            );
          })}
        </div>
      </div>
    </div>
  );
}

function RelationshipRow({
  title,
  description,
  color,
  tracks,
  limit,
  onHover,
  onSelect,
}: {
  title: string;
  description: string;
  color: string;
  tracks: Track[];
  limit: number;
  onHover?: (camelot: string | null) => void;
  onSelect?: (track: Track) => void;
}) {
  const visible = tracks.slice(0, limit);
  const overflow = tracks.length - visible.length;

  return (
    <div className="flex items-start gap-3">
      <div
        className="shrink-0 w-28 pr-2 border-r"
        style={{ borderColor: `${color}55` }}
      >
        <div className="flex items-center gap-1.5 mb-0.5">
          <span
            className="w-2 h-2 rounded-full"
            style={{ backgroundColor: color }}
          />
          <span className="text-xs font-semibold" style={{ color }}>
            {title}
          </span>
        </div>
        <div className="text-[9px] text-text-secondary leading-tight">
          {description}
        </div>
        <div className="text-[10px] text-text-secondary mt-1">
          {tracks.length} track{tracks.length === 1 ? '' : 's'}
        </div>
      </div>

      <div className="flex-1 min-w-0">
        {visible.length === 0 ? (
          <div className="text-[11px] text-text-secondary/60 italic py-3">
            No tracks in this relationship yet.
          </div>
        ) : (
          <div className="flex flex-wrap gap-3">
            {visible.map((t) => (
              <TrackArtwork
                key={t.id}
                artworkPath={t.artwork_path}
                camelot={t.key_camelot}
                label={t.title ?? t.filename ?? 'Untitled'}
                sublabel={t.bpm ? `${t.bpm.toFixed(1)} BPM` : t.artist ?? null}
                size={80}
                ringStyle="camelot"
                onClick={onSelect ? () => onSelect(t) : undefined}
                onMouseEnter={onHover ? () => onHover(t.key_camelot) : undefined}
                onMouseLeave={onHover ? () => onHover(null) : undefined}
              />
            ))}
            {overflow > 0 && (
              <div className="flex items-center px-2 text-[11px] text-text-secondary">
                +{overflow} more
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
