import { convertFileSrc } from '@tauri-apps/api/core';
import { useState } from 'react';
import { formatCamelotBadge, PITCH_NAMES_SHARP } from '../../lib/camelot';

/**
 * Visual identity tile for a single track.
 *
 * Layered design:
 *   1. A square frame.
 *   2. A glowing **ring** in the track's Camelot color — same palette as the
 *      wheel wedges, so the mosaic and the wheel speak the same visual
 *      language at a glance.
 *   3. The cover image, served via Tauri's asset protocol from the cached
 *      `artwork_path`.
 *   4. **Fallback**: when there is no cover, a 12-slice chroma swatch
 *      generated from the track's chroma vector. This is way more
 *      informative than a generic placeholder — the user can literally
 *      see which pitch classes dominate the track.
 *   5. Optional Camelot badge in the top-left corner.
 *
 * Accessibility: the whole tile is button-like only when `onClick` is set.
 * `title` shows track + key on hover.
 */
export interface TrackArtworkProps {
  /** Path on disk to the cached cover image, if any. */
  artworkPath?: string | null;
  /** Camelot code, used to color the ring + badge. */
  camelot?: string | null;
  /** 12-bin chroma vector (C..B) used for the fallback swatch. */
  chroma?: number[] | null;
  /** Display label under the tile. Pass title or filename. */
  label?: string | null;
  /** Optional secondary line (artist). */
  sublabel?: string | null;
  /** Tile pixel size. Defaults to 96. */
  size?: number;
  /** Highlight ring style. Default ring is colored Camelot. */
  ringStyle?: 'camelot' | 'subtle' | 'glow' | 'none';
  /** Make the tile clickable and keyboard-focusable. */
  onClick?: () => void;
  /** Hover handler used by HarmonicMosaic to coordinate with the wheel. */
  onMouseEnter?: () => void;
  onMouseLeave?: () => void;
  /** Show the Camelot code as a small overlay badge. */
  showBadge?: boolean;
}

export default function TrackArtwork({
  artworkPath,
  camelot,
  chroma,
  label,
  sublabel,
  size = 96,
  ringStyle = 'camelot',
  onClick,
  onMouseEnter,
  onMouseLeave,
  showBadge = true,
}: TrackArtworkProps) {
  // Lazily flip to fallback if the <img> errors (e.g. cached file deleted).
  const [imgFailed, setImgFailed] = useState(false);

  const badge = camelot ? formatCamelotBadge(camelot) : null;
  const ringColor = badge?.color ?? '#3a3a3a';

  // Tauri asset protocol path. Empty string is fine; convertFileSrc returns
  // a placeholder URL that just won't resolve.
  const src = artworkPath && !imgFailed ? convertFileSrc(artworkPath) : null;

  const ringClass = (() => {
    switch (ringStyle) {
      case 'none':   return '';
      case 'subtle': return 'ring-1 ring-white/15';
      case 'glow':   return 'ring-2 ring-offset-2 ring-offset-background';
      case 'camelot':
      default:       return 'ring-2 ring-offset-2 ring-offset-background';
    }
  })();

  const tileBody = (
    <div
      className={`relative rounded-xl overflow-hidden bg-surface-light ${ringClass}`}
      style={{
        width: size,
        height: size,
        boxShadow:
          ringStyle === 'camelot' || ringStyle === 'glow'
            ? `0 0 0 0 transparent, 0 0 18px ${ringColor}33`
            : undefined,
        // ring color via CSS variable (Tailwind `ring-2` reads --tw-ring-color).
        // We force it inline because we use dynamic Camelot palette colors.
        ['--tw-ring-color' as any]: ringColor,
      }}
      title={
        [label, camelot ? `(${camelot})` : null, sublabel]
          .filter(Boolean)
          .join(' ')
      }
    >
      {src ? (
        <img
          src={src}
          alt={label ?? 'cover'}
          className="w-full h-full object-cover select-none"
          draggable={false}
          onError={() => setImgFailed(true)}
        />
      ) : (
        <ChromaSwatch chroma={chroma ?? null} ringColor={ringColor} />
      )}

      {showBadge && badge && (
        <div
          className="absolute top-1 left-1 px-1.5 py-0.5 rounded-md text-[10px] font-bold text-white/95 backdrop-blur-sm"
          style={{ backgroundColor: `${badge.color}D9` }}
        >
          {badge.text}
        </div>
      )}
    </div>
  );

  const labelBlock = (label || sublabel) && (
    <div className="mt-1.5 text-center" style={{ width: size }}>
      {label && (
        <div className="text-[11px] text-text-primary truncate leading-tight">
          {label}
        </div>
      )}
      {sublabel && (
        <div className="text-[10px] text-text-secondary truncate leading-tight">
          {sublabel}
        </div>
      )}
    </div>
  );

  if (onClick) {
    return (
      <button
        type="button"
        onClick={onClick}
        onMouseEnter={onMouseEnter}
        onMouseLeave={onMouseLeave}
        className="flex flex-col items-center group focus:outline-none focus-visible:ring-2 focus-visible:ring-accent-primary rounded-xl transition-transform hover:scale-105 active:scale-100"
      >
        {tileBody}
        {labelBlock}
      </button>
    );
  }

  return (
    <div
      onMouseEnter={onMouseEnter}
      onMouseLeave={onMouseLeave}
      className="flex flex-col items-center"
    >
      {tileBody}
      {labelBlock}
    </div>
  );
}

/**
 * 12-slice radial swatch from the chroma vector. When chroma is null we
 * draw an even rosette so the tile still feels like a music object rather
 * than an empty rectangle.
 */
function ChromaSwatch({
  chroma,
  ringColor,
}: {
  chroma: number[] | null;
  ringColor: string;
}) {
  const values = chroma && chroma.length === 12 ? chroma : Array(12).fill(1 / 12);
  // Hue spans 0..360 as we go around the chromatic circle. Saturation +
  // lightness driven by the chroma weight so dominant pitch classes pop.
  const max = Math.max(...values, 0.001);
  return (
    <svg viewBox="-50 -50 100 100" className="w-full h-full">
      <defs>
        <radialGradient id="chroma-grad" r="0.7">
          <stop offset="0%" stopColor="#000" stopOpacity="0.6" />
          <stop offset="100%" stopColor="#000" stopOpacity="0.1" />
        </radialGradient>
      </defs>
      {values.map((v, i) => {
        const w = v / max; // 0..1 normalised
        const startAngle = (i / 12) * 2 * Math.PI - Math.PI / 2;
        const endAngle = ((i + 1) / 12) * 2 * Math.PI - Math.PI / 2;
        const r = 50;
        const x1 = Math.cos(startAngle) * r;
        const y1 = Math.sin(startAngle) * r;
        const x2 = Math.cos(endAngle) * r;
        const y2 = Math.sin(endAngle) * r;
        const hue = (i * 30) % 360; // 12 evenly-spaced hues
        const sat = 35 + 50 * w;
        const light = 28 + 30 * w;
        return (
          <path
            key={i}
            d={`M 0 0 L ${x1} ${y1} A ${r} ${r} 0 0 1 ${x2} ${y2} Z`}
            fill={`hsl(${hue}, ${sat}%, ${light}%)`}
            stroke="rgba(0,0,0,0.25)"
            strokeWidth="0.5"
          />
        );
      })}
      {/* Center dot in the Camelot color */}
      <circle r="14" fill={ringColor} fillOpacity="0.85" stroke="rgba(0,0,0,0.4)" strokeWidth="1" />
      {/* No-art marker letter (note name of the strongest chroma bin). */}
      <text
        x="0"
        y="0"
        textAnchor="middle"
        dominantBaseline="central"
        fontSize="10"
        fontWeight="700"
        fill="white"
      >
        {dominantPitchClassName(values)}
      </text>
      <rect x="-50" y="-50" width="100" height="100" fill="url(#chroma-grad)" />
    </svg>
  );
}

function dominantPitchClassName(values: number[]): string {
  let bestIdx = 0;
  let bestVal = -Infinity;
  for (let i = 0; i < values.length; i++) {
    if (values[i] > bestVal) {
      bestVal = values[i];
      bestIdx = i;
    }
  }
  return PITCH_NAMES_SHARP[bestIdx] ?? '?';
}
