// Fader — vertical channel fader.
//
// Renders a console-style fader with a track and a draggable cap.
// Vertical drag changes the value. Double-click resets to default.
// The cap sits at the value position along the track.

import { useState, useRef, useCallback } from 'react';

interface FaderProps {
  value: number;       // 0.0 to 1.0
  default?: number;
  onChange: (value: number) => void;
  height?: number;     // track height in px (default 120)
  label?: string;
}

export default function Fader({ value, default: defaultValue, onChange, height = 120, label }: FaderProps) {
  const [isDragging, setIsDragging] = useState(false);
  const trackRef = useRef<HTMLDivElement>(null);
  const dragStartRef = useRef<number | null>(null);

  const capBottom = value * height;

  const handlePointerDown = useCallback((e: React.PointerEvent) => {
    e.preventDefault();
    e.stopPropagation();
    setIsDragging(true);
    dragStartRef.current = e.clientY;
  }, []);

  const handlePointerMove = useCallback((e: React.PointerEvent) => {
    if (!isDragging || !trackRef.current) return;
    const rect = trackRef.current.getBoundingClientRect();
    const y = e.clientY - rect.top;
    const newValue = Math.max(0, Math.min(1, 1 - y / rect.height));
    onChange(newValue);
  }, [isDragging]);

  const handlePointerUp = useCallback(() => {
    setIsDragging(false);
    dragStartRef.current = null;
  }, []);

  const handleDoubleClick = useCallback(() => {
    if (defaultValue !== undefined) onChange(defaultValue);
  }, [defaultValue, onChange]);

  // dB scale markings
  const dbMarks = [
    { pos: 1.0, label: '+6' },
    { pos: 0.85, label: '+3' },
    { pos: 0.7, label: '0' },
    { pos: 0.5, label: '-6' },
    { pos: 0.3, label: '-12' },
    { pos: 0.1, label: '-24' },
    { pos: 0.0, label: '-∞' },
  ];

  return (
    <div className="flex flex-col items-center gap-0.5 no-select">
      <div className="flex items-stretch gap-1">
        {/* dB markings */}
        <div className="flex flex-col justify-between py-0.5" style={{ height }}>
          {dbMarks.map(mark => (
            <div key={mark.label} className="flex items-center gap-0.5">
              <div className="w-1.5 h-px bg-label-dim/40" />
              <span className="text-[7px] font-mono text-label-dim/60">{mark.label}</span>
            </div>
          ))}
        </div>

        {/* Track + cap */}
        <div
          ref={trackRef}
          className="fader-track relative"
          style={{ height, width: 6 }}
          onPointerDown={handlePointerDown}
          onPointerMove={handlePointerMove}
          onPointerUp={handlePointerUp}
          onPointerLeave={handlePointerUp}
          onDoubleClick={handleDoubleClick}
        >
          {/* Fill (from bottom to cap) */}
          <div
            className="absolute bottom-0 left-0 right-0"
            style={{
              height: capBottom,
              background: 'linear-gradient(0deg, rgba(212,160,76,0.15) 0%, rgba(212,160,76,0.05) 100%)',
            }}
          />
          {/* Cap */}
          <div
            className="fader-cap absolute left-1/2 -translate-x-1/2"
            style={{
              bottom: capBottom - 5,
              width: 22,
              height: 10,
              cursor: isDragging ? 'grabbing' : 'grab',
            }}
            onPointerDown={handlePointerDown}
            onPointerMove={handlePointerMove}
            onPointerUp={handlePointerUp}
            onDoubleClick={handleDoubleClick}
          >
            {/* Cap line indicator */}
            <div className="absolute left-0 right-0 top-1/2 h-px bg-plate-darker/60" />
          </div>
        </div>
      </div>
      {label && <span className="engraved-sm">{label}</span>}
    </div>
  );
}
