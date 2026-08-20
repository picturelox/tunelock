// TransportBar — the bottom transport controls for the Mix Canvas.
//
// Contains: play/pause, cue, loop, capture scene, A/B bus controls,
// crossfader, and master level. This is the shared transport for
// the three-level workspace.
//
// Design language: Walnut Console — faders and buttons use the
// bronze-plate and launch-pad styles.

import { useState, useEffect, useRef, useCallback } from 'react';
import { Play, Pause, Square, Repeat, Camera } from 'lucide-react';
import {
  audioEngineSetCrossfade,
  audioEngineSetMasterGain,
  audioEngineGetMeters,
  type AudioMeterReadout,
} from '../../lib/tauri';

export default function TransportBar() {
  const [crossfade, setCrossfade] = useState(0.5);
  const [masterGain, setMasterGain] = useState(0.8);
  const [meters, setMeters] = useState<AudioMeterReadout | null>(null);
  const meterRef = useRef<ReturnType<typeof setInterval> | null>(null);

  useEffect(() => {
    meterRef.current = setInterval(async () => {
      try {
        const m = await audioEngineGetMeters();
        setMeters(m);
      } catch {
        // engine might not be ready
      }
    }, 33);
    return () => { if (meterRef.current) clearInterval(meterRef.current); };
  }, []);

  const handleCrossfade = useCallback((value: number) => {
    setCrossfade(value);
    audioEngineSetCrossfade(value).catch(() => {});
  }, []);

  const handleMasterGain = useCallback((value: number) => {
    setMasterGain(value);
    audioEngineSetMasterGain(value).catch(() => {});
  }, []);

  return (
    <div className="bronze-plate px-4 py-2 flex items-center gap-4">
      {/* Transport buttons */}
      <div className="flex items-center gap-1">
        <button className="launch-pad p-2 text-cream-label/80 hover:text-brass-bright" title="Play">
          <Play className="w-4 h-4" />
        </button>
        <button className="launch-pad p-2 text-cream-label/60 hover:text-lamp-amber" title="Pause">
          <Pause className="w-4 h-4" />
        </button>
        <button className="launch-pad p-2 text-cream-label/60 hover:text-lamp-red" title="Stop">
          <Square className="w-4 h-4" />
        </button>
        <button className="launch-pad p-2 text-cream-label/60 hover:text-brass-bright" title="Loop">
          <Repeat className="w-4 h-4" />
        </button>
      </div>

      {/* Divider */}
      <div className="w-px h-8 bg-bronze-dark" />

      {/* Scene capture */}
      <button
        className="launch-pad px-3 py-2 text-[11px] text-cream-label/80 hover:text-brass-bright flex items-center gap-1.5"
        title="Capture current combination as a scene"
      >
        <Camera className="w-3.5 h-3.5" />
        <span className="engraved-label">Capture Scene</span>
      </button>

      {/* Divider */}
      <div className="w-px h-8 bg-bronze-dark" />

      {/* Crossfader */}
      <div className="flex items-center gap-2 flex-1 max-w-xs">
        <span className="text-[10px] font-mono text-cream-label/70 w-4">A</span>
        <input
          type="range"
          min="0"
          max="1"
          step="0.01"
          value={crossfade}
          onChange={(e) => handleCrossfade(parseFloat(e.target.value))}
          className="flex-1 accent-brass-accent"
        />
        <span className="text-[10px] font-mono text-cream-label/70 w-4">B</span>
        <span className="text-[10px] font-mono text-cream-label/50 w-10 text-right">
          {(crossfade * 100).toFixed(0)}%
        </span>
      </div>

      {/* Divider */}
      <div className="w-px h-8 bg-bronze-dark" />

      {/* Master gain + meter */}
      <div className="flex items-center gap-2">
        <span className="engraved-label">Master</span>
        <input
          type="range"
          min="0"
          max="1"
          step="0.01"
          value={masterGain}
          onChange={(e) => handleMasterGain(parseFloat(e.target.value))}
          className="w-20 accent-brass-accent"
        />
        {meters && (
          <div className="flex flex-col gap-px w-16">
            <MasterMeter
              rms={meters.masterRms}
              peak={meters.masterPeak}
              clip={meters.masterClip}
            />
            <div className="text-[9px] font-mono text-cream-label/50 text-center">
              {(meters.masterPeak * 100).toFixed(0)}{meters.masterClip && ' CLIP'}
            </div>
          </div>
        )}
      </div>

      {/* Bus meters */}
      {meters && (
        <div className="flex items-center gap-2 ml-auto">
          <BusMeter label="A" rms={meters.busARms} />
          <BusMeter label="B" rms={meters.busBRms} />
        </div>
      )}
    </div>
  );
}

function MasterMeter({ rms, peak, clip }: { rms: number; peak: number; clip: boolean }) {
  return (
    <div className="h-2 bg-walnut-dark rounded-full overflow-hidden relative">
      <div
        className={`h-full transition-all duration-75 ${clip ? 'bg-lamp-red' : 'bg-lamp-green'}`}
        style={{ width: `${Math.min(rms * 100, 100)}%` }}
      />
      <div
        className={`absolute top-0 h-full w-px ${clip ? 'bg-lamp-red' : 'bg-brass-bright'}`}
        style={{ left: `${Math.min(peak * 100, 100)}%` }}
      />
    </div>
  );
}

function BusMeter({ label, rms }: { label: string; rms: number }) {
  return (
    <div className="flex items-center gap-1">
      <span className="text-[10px] font-mono text-cream-label/60">{label}</span>
      <div className="h-1.5 w-12 bg-walnut-dark rounded-full overflow-hidden relative">
        <div
          className="h-full bg-brass-accent transition-all duration-75"
          style={{ width: `${Math.min(rms * 100, 100)}%` }}
        />
      </div>
    </div>
  );
}
