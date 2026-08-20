import { useEffect, useRef } from 'react';
import type { WaveformData } from '../../lib/tauri';

interface WaveformDisplayProps {
  data: WaveformData | null;
  /** Current playback position (0.0–1.0). Draws a playhead line. */
  progress?: number;
  /** Height of the canvas in pixels. Default 64. */
  height?: number;
  /** Whether to show the three bands as separate colors (true) or a
   * single blended waveform (false). Default true. */
  threeBand?: boolean;
  className?: string;
}

/**
 * Canvas-based waveform renderer. Draws a three-band (low/mid/high) waveform
 * using the Traktor convention: bass=red/orange, mid=green, high=blue.
 *
 * Performance: draws directly to a 2D canvas context, O(columns) per frame.
 * At 2000 columns this is sub-millisecond on any modern GPU.
 */
export default function WaveformDisplay({
  data,
  progress,
  height = 64,
  threeBand = true,
  className = '',
}: WaveformDisplayProps) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    // Set canvas resolution to device pixel ratio for crisp rendering
    const dpr = window.devicePixelRatio || 1;
    const rect = canvas.getBoundingClientRect();
    canvas.width = rect.width * dpr;
    canvas.height = height * dpr;
    ctx.scale(dpr, dpr);

    // Clear
    ctx.clearRect(0, 0, rect.width, height);

    if (!data || data.columns.length === 0) {
      // Draw a flat line
      ctx.strokeStyle = '#333';
    ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.moveTo(0, height / 2);
      ctx.lineTo(rect.width, height / 2);
      ctx.stroke();
      return;
    }

    const cols = data.columns;
    const colWidth = rect.width / cols.length;
    const midY = height / 2;
    const maxBarHeight = height / 2 - 2;

    // Draw each column
    for (let i = 0; i < cols.length; i++) {
      const x = i * colWidth;
      const col = cols[i];

      if (threeBand) {
        // Three-band: draw low (bottom), mid (middle), high (top)
        // Low band: warm color (red-orange)
        const lowH = col.low * maxBarHeight;
        ctx.fillStyle = '#ef4444'; // red
        ctx.fillRect(x, midY, colWidth + 0.5, lowH);

        // Mid band: green
        const midH = col.mid * maxBarHeight;
        ctx.fillStyle = '#22c55e'; // green
        ctx.fillRect(x, midY - midH, colWidth + 0.5, midH);

        // High band: blue (drawn as a thin line on top)
        const highH = col.high * maxBarHeight * 0.5;
        ctx.fillStyle = '#3b82f6'; // blue
        ctx.fillRect(x, midY - midH - highH, colWidth + 0.5, highH);
      } else {
        // Blended: sum all bands and draw as a single bar
        const total = Math.min(1.0, col.low + col.mid + col.high);
        const h = total * maxBarHeight;
        ctx.fillStyle = '#a78bfa'; // purple
        ctx.fillRect(x, midY - h, colWidth + 0.5, h * 2);
      }
    }

    // Draw playhead
    if (progress !== undefined && progress > 0 && progress < 1) {
      const playheadX = progress * rect.width;
      ctx.strokeStyle = '#ffffff';
      ctx.lineWidth = 1.5;
      ctx.beginPath();
      ctx.moveTo(playheadX, 0);
      ctx.lineTo(playheadX, height);
      ctx.stroke();
    }
  }, [data, progress, height, threeBand]);

  return (
    <canvas
      ref={canvasRef}
      style={{ width: '100%', height: `${height}px` }}
      className={className}
    />
  );
}
