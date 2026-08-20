// Knob — vintage console rotary control.
//
// Renders a chunky round knob with an indicator line, like the EQ
// knobs on an API 2500. Supports drag-to-change (vertical drag =
// value up/down) and double-click to reset to default.
//
// The knob has a 270-degree sweep (from -135deg to +135deg).

import { useState, useRef, useCallback, type ReactNode } from 'react';

interface KnobProps {
  value: number;       // current value
  min: number;
  max: number;
  default?: number;    // double-click resets to this
  onChange: (value: number) => void;
  label?: string;
  size?: number;       // diameter in px (default 36)
  format?: (v: number) => string;  // display format
  children?: ReactNode; // optional center content (e.g. value text)
}

export default function Knob({
  value, min, max, default: defaultValue, onChange, label, size = 36, format, children,
}: KnobProps) {
  const [isDragging, setIsDragging] = useState(false);
  const dragStartRef = useRef<{ y: number; value: number } | null>(null);

  const range = max - min;
  const normalized = range > 0 ? (value - min) / range : 0;
  // 270-degree sweep: -135 to +135
  const angle = -135 + normalized * 270;

  const handlePointerDown = useCallback((e: React.PointerEvent) => {
    e.preventDefault();
    e.stopPropagation();
    setIsDragging(true);
    dragStartRef.current = { y: e.clientY, value };
  }, [value]);

  const handlePointerMove = useCallback((e: React.PointerEvent) => {
    if (!isDragging || !dragStartRef.current) return;
    const dy = dragStartRef.current.y - e.clientY;
    const sensitivity = 0.005; // 200px = full range
    const delta = dy * sensitivity * range;
    const newValue = Math.max(min, Math.min(max, dragStartRef.current.value + delta));
    onChange(newValue);
  }, [isDragging, min, max, range, onChange]);

  const handlePointerUp = useCallback(() => {
    setIsDragging(false);
    dragStartRef.current = null;
  }, []);

  const handleDoubleClick = useCallback(() => {
    if (defaultValue !== undefined) onChange(defaultValue);
  }, [defaultValue, onChange]);

  return (
    <div className="flex flex-col items-center gap-0.5 no-select">
      <div
        className="knob"
        style={{ width: size, height: size, cursor: isDragging ? 'grabbing' : 'grab' }}
        onPointerDown={handlePointerDown}
        onPointerMove={handlePointerMove}
        onPointerUp={handlePointerUp}
        onPointerLeave={handlePointerUp}
        onDoubleClick={handleDoubleClick}
      >
        <div
          className="knob-indicator"
          style={{
            transform: `translate(-50%, -100%) rotate(${angle}deg)`,
            transformOrigin: 'bottom center',
            height: size * 0.35,
          }}
        />
        {children && (
          <div
            className="absolute inset-0 flex items-center justify-center"
            style={{ fontSize: size * 0.22 }}
          >
            {children}
          </div>
        )}
      </div>
      {label && <span className="engraved-sm">{label}</span>}
      {format && !children && (
        <span className="text-[8px] font-mono text-label-dim">{format(value)}</span>
      )}
    </div>
  );
}
