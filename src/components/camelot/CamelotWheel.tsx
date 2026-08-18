import { useEffect, useMemo, useRef, useState } from 'react';
import { useLibraryStore } from '../../stores/libraryStore';
import {
  getAllCamelotPositions,
  getRelationship,
  RELATIONSHIP_INFO,
  parseCamelot,
  type CamelotRelationship,
} from '../../lib/camelot';
import type { CamelotPosition } from '../../types';

/**
 * Visual order on the wheel:
 *  - Each ring places number 1 at the top (12 o'clock) and increments clockwise.
 *  - Outer ring = major keys (B), inner ring = minor keys (A).
 */
const WHEEL_ORDER = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];

export interface CamelotWheelProps {
  /**
   * Controlled selected key as a Camelot string (e.g. "11A"). When provided,
   * the wheel renders this as the "selected" position and ignores internal
   * click-to-select state. Pass null/undefined for uncontrolled mode.
   */
  selectedCamelot?: string | null;
  /** Fired when the user clicks a key. Pass null to clear. */
  onSelect?: (camelot: string | null) => void;
  /**
   * Fired with a delay (default 500ms) when the user hovers a key.
   * Used by the Tuner to coordinate the piano-roll + scale-notes panel.
   * Called with `null` when the cursor leaves the wheel without landing on a key.
   */
  onHover?: (camelot: string | null) => void;
  hoverDelayMs?: number;
  /** Show the right-side legend listing relationships. Default: true. */
  showLegend?: boolean;
  /** Show track-count chips overlaid on each wedge (Library mode). Default: true. */
  showTrackCounts?: boolean;
  /** Show "N/M tagged" stats in the center. Default: true. */
  showCenterStats?: boolean;
  /** Optional title shown above the wheel. */
  title?: string;
}

export default function CamelotWheel({
  selectedCamelot,
  onSelect,
  onHover,
  hoverDelayMs = 500,
  showLegend = true,
  showTrackCounts = true,
  showCenterStats = true,
  title = 'Camelot Wheel',
}: CamelotWheelProps = {}) {
  const { tracks } = useLibraryStore();
  const trackList = Array.from(tracks.values());

  const tracksByPosition = useMemo(() => {
    const map = new Map<string, typeof trackList>();
    if (!showTrackCounts) return map;
    for (const track of trackList) {
      if (track.key_camelot) {
        const list = map.get(track.key_camelot) ?? [];
        list.push(track);
        map.set(track.key_camelot, list);
      }
    }
    return map;
  }, [trackList, showTrackCounts]);

  const allPositions = getAllCamelotPositions();
  const maxTracksInSegment = Math.max(
    ...allPositions.map((p) => tracksByPosition.get(`${p.number}${p.letter}`)?.length ?? 0),
    1
  );

  // Selection: controlled (prop) takes precedence over internal state.
  const [internalSelected, setInternalSelected] = useState<CamelotPosition | null>(null);
  const controlled = selectedCamelot !== undefined;
  const selectedPosition: CamelotPosition | null = controlled
    ? selectedCamelot
      ? parseCamelot(selectedCamelot)
      : null
    : internalSelected;

  // Map of every position -> its relationship to the current selection.
  const relationshipMap = useMemo(() => {
    const map = new Map<string, CamelotRelationship>();
    if (!selectedPosition) return map;
    for (const pos of allPositions) {
      map.set(`${pos.number}${pos.letter}`, getRelationship(selectedPosition, pos));
    }
    return map;
  }, [selectedPosition, allPositions]);

  const handleSelect = (pos: CamelotPosition) => {
    const same = selectedPosition?.number === pos.number && selectedPosition?.letter === pos.letter;
    const next = same ? null : pos;
    if (!controlled) setInternalSelected(next);
    onSelect?.(next ? `${next.number}${next.letter}` : null);
  };

  // === Hover-with-delay coordination =====================================
  // We don't want to fire onHover the instant the mouse crosses a wedge
  // because the user is often moving over multiple keys on the way to the
  // one they care about. Wait `hoverDelayMs` of dwell time before firing.
  const hoverTimer = useRef<number | null>(null);
  const lastHovered = useRef<string | null>(null);

  const clearHoverTimer = () => {
    if (hoverTimer.current !== null) {
      window.clearTimeout(hoverTimer.current);
      hoverTimer.current = null;
    }
  };

  /**
   * Schedule a hovered key to be announced after `hoverDelayMs` of dwell.
   * Wedge-to-wedge transitions just reset the timer; they don't clear the
   * previously locked-in hover. The full wheel `onMouseLeave` (below) is
   * what clears via `clearHover()`.
   */
  const handleHoverEnter = (camelot: string) => {
    if (!onHover) return;
    clearHoverTimer();
    if (lastHovered.current === camelot) return; // already locked in
    hoverTimer.current = window.setTimeout(() => {
      lastHovered.current = camelot;
      onHover(camelot);
    }, hoverDelayMs);
  };

  /** Called when the cursor exits the wheel entirely. */
  const clearHover = () => {
    if (!onHover) return;
    clearHoverTimer();
    if (lastHovered.current !== null) {
      lastHovered.current = null;
      onHover(null);
    }
  };

  /** Cancel the pending timer when leaving a wedge, but don't clear state. */
  const cancelPendingHover = () => {
    clearHoverTimer();
  };

  // Clean up timer on unmount.
  useEffect(() => () => clearHoverTimer(), []);

  const renderWedge = (pos: CamelotPosition, radius: number, fontSize: number) => {
    const orderIdx = WHEEL_ORDER.indexOf(pos.number);
    const angle = (orderIdx * 30 - 90) * (Math.PI / 180);
    const x = 200 + radius * Math.cos(angle);
    const y = 200 + radius * Math.sin(angle);
    const key = `${pos.number}${pos.letter}`;
    const tracksInSegment = tracksByPosition.get(key)?.length ?? 0;
    const trackOpacity =
      tracksInSegment > 0
        ? 0.4 + (tracksInSegment / maxTracksInSegment) * 0.6
        : showTrackCounts
          ? 0.15
          : 0.85;

    const relationship = relationshipMap.get(key);
    const isSelected =
      selectedPosition?.number === pos.number && selectedPosition?.letter === pos.letter;
    const hue = ((pos.number - 1) * 30) % 360;

    // Fill: when something is selected, color by relationship; otherwise color by key hue.
    let fill: string;
    if (isSelected) {
      fill = RELATIONSHIP_INFO.same.color;
    } else if (selectedPosition && relationship && relationship !== 'incompatible') {
      fill = RELATIONSHIP_INFO[relationship].color;
    } else {
      fill = `hsl(${hue}, ${pos.letter === 'A' ? '70%' : '50%'}, ${pos.letter === 'A' ? '40%' : '55%'})`;
    }

    const baseR = pos.letter === 'B' ? 22 : 18;
    const r = tracksInSegment > 0 ? baseR + 3 : baseR;

    return (
      <g key={key}>
        <circle
          cx={x}
          cy={y}
          r={r}
          fill={fill}
          opacity={isSelected ? 1 : selectedPosition ? (relationship === 'incompatible' ? 0.2 : 1) : trackOpacity}
          stroke={isSelected ? '#fff' : 'none'}
          strokeWidth={isSelected ? 2 : 0}
          className="cursor-pointer transition-all duration-200"
          onClick={() => handleSelect(pos)}
          onMouseEnter={() => handleHoverEnter(key)}
          onMouseLeave={cancelPendingHover}
        />
        <text
          x={x}
          y={y}
          textAnchor="middle"
          dominantBaseline="middle"
          fill="#fff"
          fontSize={fontSize}
          fontWeight="bold"
          pointerEvents="none"
        >
          {pos.number}{pos.letter}
        </text>
        {showTrackCounts && tracksInSegment > 0 && (
          <text
            x={x}
            y={y + fontSize + 4}
            textAnchor="middle"
            fill="#fff"
            fontSize={fontSize - 3}
            opacity={0.85}
            pointerEvents="none"
          >
            {tracksInSegment}
          </text>
        )}
      </g>
    );
  };

  return (
    <div className="flex flex-col lg:flex-row h-full p-4 gap-4 overflow-auto">
      {/* Wheel */}
      <div className="flex-1 min-w-0 flex flex-col items-center">
        {title && (
          <div className="w-full flex items-center justify-between mb-2">
            <h2 className="text-xl font-semibold">{title}</h2>
            {selectedPosition && !controlled ? (
              <button
                onClick={() => {
                  setInternalSelected(null);
                  onSelect?.(null);
                }}
                className="text-xs text-text-secondary hover:text-text-primary"
              >
                Clear selection
              </button>
            ) : !controlled ? (
              <span className="text-xs text-text-secondary">Click a key to see its relationships</span>
            ) : null}
          </div>
        )}

        <svg
          viewBox="0 0 400 400"
          className="w-full max-w-[520px] aspect-square"
          onMouseLeave={clearHover}
        >
          {/* Outer ring — major keys (B) */}
          {allPositions.filter((p) => p.letter === 'B').map((p) => renderWedge(p, 145, 12))}
          {/* Inner ring — minor keys (A) */}
          {allPositions.filter((p) => p.letter === 'A').map((p) => renderWedge(p, 80, 10))}

          {/* Center */}
          <circle cx={200} cy={200} r={32} fill="#1a1a2e" stroke="#333" strokeWidth={1} />
          {showCenterStats ? (
            <>
              <text x={200} y={196} textAnchor="middle" dominantBaseline="middle" fill="#fff" fontSize={10}>
                {trackList.filter((t) => t.key_camelot).length} / {trackList.length}
              </text>
              <text x={200} y={208} textAnchor="middle" dominantBaseline="middle" fill="#888" fontSize={7}>
                tagged
              </text>
            </>
          ) : selectedPosition ? (
            <text x={200} y={202} textAnchor="middle" dominantBaseline="middle" fill="#fff" fontSize={14} fontWeight="bold">
              {selectedPosition.number}{selectedPosition.letter}
            </text>
          ) : null}
        </svg>
      </div>

      {/* Relationship legend / detail panel */}
      {showLegend && (
        <aside className="w-full lg:w-72 shrink-0 bg-surface rounded-xl p-4 flex flex-col gap-3">
          <h3 className="text-sm font-semibold">
            {selectedPosition
              ? `From ${selectedPosition.number}${selectedPosition.letter}`
              : 'Mixing relationships'}
          </h3>

          <p className="text-xs text-text-secondary leading-relaxed">
            {selectedPosition
              ? 'The wheel is tinted by how each key relates to the one you picked. Use these moves to build a set.'
              : 'Click any key on the wheel to see which other keys it mixes well with, by relationship type.'}
          </p>

          <ul className="flex flex-col gap-2 mt-2">
            {(['same', 'plus_one', 'minus_one', 'plus_two', 'minus_two', 'mood_shift'] as const).map(
              (rel) => (
                <li key={rel} className="flex items-start gap-3">
                  <span
                    className="w-3 h-3 rounded-full mt-1 shrink-0"
                    style={{ backgroundColor: RELATIONSHIP_INFO[rel].color }}
                  />
                  <div className="text-xs">
                    <div className="font-semibold text-text-primary">{RELATIONSHIP_INFO[rel].label}</div>
                    <div className="text-text-secondary leading-snug">
                      {RELATIONSHIP_INFO[rel].description}
                    </div>
                  </div>
                </li>
              )
            )}
          </ul>
        </aside>
      )}
    </div>
  );
}
