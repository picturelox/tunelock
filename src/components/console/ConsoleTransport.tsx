// ConsoleTransport — the bottom transport bar.
//
// Play/pause/stop, cue, loop, and the current position display.
// Styled as the transport section of a tape machine / console.

import { Play, Pause, Square, Repeat, SkipBack, SkipForward } from 'lucide-react';

interface ConsoleTransportProps {
  playing: boolean;
  onPlayPause: () => void;
  onStop: () => void;
  onLoop: () => void;
  looping: boolean;
  positionSec: number;
  totalSec: number;
}

function formatTime(sec: number): string {
  const m = Math.floor(sec / 60);
  const s = Math.floor(sec % 60);
  const ms = Math.floor((sec % 1) * 1000);
  return `${m.toString().padStart(2, '0')}:${s.toString().padStart(2, '0')}.${ms.toString().padStart(3, '0')}`;
}

export default function ConsoleTransport({
  playing, onPlayPause, onStop, onLoop, looping, positionSec, totalSec,
}: ConsoleTransportProps) {
  return (
    <div className="faceplate flex items-center gap-3 px-4 py-2">
      {/* Transport buttons */}
      <div className="flex items-center gap-1">
        <button className="cap cap-dark w-8 h-8 flex items-center justify-center" title="Previous">
          <SkipBack className="w-4 h-4" />
        </button>
        <button
          onClick={onPlayPause}
          className={`cap ${playing ? 'cap-green lit' : 'cap-amber'} w-10 h-8 flex items-center justify-center`}
          title={playing ? 'Pause' : 'Play'}
        >
          {playing ? <Pause className="w-4 h-4" /> : <Play className="w-4 h-4" />}
        </button>
        <button
          onClick={onStop}
          className="cap cap-red w-8 h-8 flex items-center justify-center"
          title="Stop"
        >
          <Square className="w-4 h-4" />
        </button>
        <button
          onClick={onLoop}
          className={`cap ${looping ? 'cap-amber lit' : 'cap-dark'} w-8 h-8 flex items-center justify-center`}
          title="Loop"
        >
          <Repeat className="w-4 h-4" />
        </button>
        <button className="cap cap-dark w-8 h-8 flex items-center justify-center" title="Next">
          <SkipForward className="w-4 h-4" />
        </button>
      </div>

      {/* Divider */}
      <div className="w-px h-8 bg-plate-darker/60" />

      {/* Position display */}
      <div className="data-plane px-3 py-1 flex items-center gap-2">
        <span className="engraved-sm">POS</span>
        <span className="font-mono text-sm text-cap-amber tabular-nums">
          {formatTime(positionSec)}
        </span>
        <span className="font-mono text-xs text-label-dim">
          / {formatTime(totalSec)}
        </span>
      </div>

      {/* Status lamps */}
      <div className="flex items-center gap-2 ml-auto">
        <div className="flex items-center gap-1">
          <div className={`w-2 h-2 rounded-full ${playing ? 'bg-cap-green' : 'bg-plate-darker'}`}
            style={playing ? { boxShadow: '0 0 4px rgba(92,138,92,0.6)' } : {}} />
          <span className="engraved-sm">PLAY</span>
        </div>
        <div className="flex items-center gap-1">
          <div className={`w-2 h-2 rounded-full ${looping ? 'bg-cap-amber' : 'bg-plate-darker'}`}
            style={looping ? { boxShadow: '0 0 4px rgba(212,160,76,0.6)' } : {}} />
          <span className="engraved-sm">LOOP</span>
        </div>
      </div>
    </div>
  );
}
