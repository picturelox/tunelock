// VuMeter — vintage analog VU meter.
//
// Renders a backlit scale with a moving needle, like the meters on
// an API 2500 or Neve console. The needle responds to the RMS level
// with a slight ballistic delay (smoothed) for that analog feel.
//
// The scale goes from -20 to +3 dB, with the 0 dB mark around 70%
// of the arc. Above 0 dB, the scale turns red.

import { useState, useEffect, useRef } from 'react';

interface VuMeterProps {
  rms: number;        // 0.0 to 1.0
  peak?: number;      // 0.0 to 1.0
  label?: string;
  size?: 'sm' | 'md' | 'lg';
}

export default function VuMeter({ rms, peak, label, size = 'md' }: VuMeterProps) {
  const [smoothedRms, setSmoothedRms] = useState(0);
  const targetRef = useRef(0);
  const rafRef = useRef<number | null>(null);

  // Ballistic smoothing — analog VU meters have ~300ms response time
  useEffect(() => {
    targetRef.current = Math.min(rms, 1.2);

    if (rafRef.current === null) {
      const tick = () => {
        setSmoothedRms(prev => {
          const target = targetRef.current;
          // Attack faster than release (like a real VU)
          const speed = target > prev ? 0.15 : 0.06;
          const next = prev + (target - prev) * speed;
          if (Math.abs(next - target) < 0.001) {
            rafRef.current = null;
            return target;
          }
          rafRef.current = requestAnimationFrame(tick);
          return next;
        });
      };
      rafRef.current = requestAnimationFrame(tick);
    }

    return () => {
      if (rafRef.current !== null) {
        cancelAnimationFrame(rafRef.current);
        rafRef.current = null;
      }
    };
  }, [rms]);

  // Convert RMS (0-1) to needle angle (-20dB to +3dB scale)
  // 0 dB ≈ 0.7 on the scale, -20 dB ≈ 0, +3 dB ≈ 1.0
  const dbValue = smoothedRms > 0 ? 20 * Math.log10(smoothedRms) : -60;
  // Map -20..+3 dB to -50..+50 degrees
  const angle = Math.max(-50, Math.min(50, ((dbValue + 20) / 23) * 100 - 50));

  const dimensions = {
    sm: { w: 40, h: 28 },
    md: { w: 56, h: 38 },
    lg: { w: 80, h: 52 },
  }[size];

  const peakDb = peak && peak > 0 ? 20 * Math.log10(peak) : -60;
  const isClip = peakDb > 0;

  return (
    <div className="flex flex-col items-center gap-0.5">
      <div
        className="vu-meter flex items-end justify-center"
        style={{ width: dimensions.w, height: dimensions.h }}
      >
        <svg
          width={dimensions.w}
          height={dimensions.h}
          viewBox="0 0 56 38"
          className="overflow-visible"
        >
          {/* Scale arc ticks */}
          {[-20, -10, -7, -5, -3, 0, 1, 2, 3].map(db => {
            const tickAngle = Math.max(-50, Math.min(50, ((db + 20) / 23) * 100 - 50));
            const rad = (tickAngle - 90) * Math.PI / 180;
            const cx = 28, cy = 34;
            const r1 = 26, r2 = 22;
            const x1 = cx + Math.cos(rad) * r1;
            const y1 = cy + Math.sin(rad) * r1;
            const x2 = cx + Math.cos(rad) * r2;
            const y2 = cy + Math.sin(rad) * r2;
            const isRed = db > 0;
            return (
              <line
                key={db}
                x1={x1} y1={y1} x2={x2} y2={y2}
                stroke={isRed ? '#c45c3c' : '#c0c0a0'}
                strokeWidth={db === 0 ? 0.8 : 0.4}
                opacity={0.7}
              />
            );
          })}

          {/* Scale labels (only on md/lg) */}
          {size !== 'sm' && [-20, 0, 3].map(db => {
            const tickAngle = Math.max(-50, Math.min(50, ((db + 20) / 23) * 100 - 50));
            const rad = (tickAngle - 90) * Math.PI / 180;
            const cx = 28, cy = 34;
            const r = 19;
            const x = cx + Math.cos(rad) * r;
            const y = cy + Math.sin(rad) * r;
            return (
              <text
                key={db}
                x={x} y={y}
                fill={db > 0 ? '#c45c3c' : '#a0a080'}
                fontSize={3}
                textAnchor="middle"
                dominantBaseline="middle"
                fontFamily="monospace"
              >
                {db > 0 ? `+${db}` : db}
              </text>
            );
          })}

          {/* Needle */}
          <g transform={`rotate(${angle} 28 34)`}>
            <line
              x1="28" y1="34"
              x2="28" y2="6"
              stroke={isClip ? '#e85c3c' : '#d4a04c'}
              strokeWidth="0.6"
              strokeLinecap="round"
              opacity={0.85}
            />
            <circle cx="28" cy="34" r="1.5" fill="#8a7a40" />
          </g>
        </svg>
      </div>
      {label && (
        <span className="engraved-sm">{label}</span>
      )}
    </div>
  );
}
