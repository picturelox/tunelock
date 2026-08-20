// MasterSection — the right-side master module of the console.
//
// Contains: master fader, master VU meters, bus A/B VU meters,
// crossfader, and the scene capture button. This is the control
// room monitor section, like the master module on an API console.

import Fader from './Fader';
import VuMeter from './VuMeter';
import { Camera } from 'lucide-react';

interface MasterSectionProps {
  masterGain: number;
  onMasterGain: (v: number) => void;
  crossfade: number;
  onCrossfade: (v: number) => void;
  masterRms: number;
  masterPeak: number;
  masterClip: boolean;
  busARms: number;
  busAPeak: number;
  busBRms: number;
  busBPeak: number;
  onCaptureScene: () => void;
}

export default function MasterSection({
  masterGain, onMasterGain,
  crossfade, onCrossfade,
  masterRms, masterPeak, masterClip,
  busARms, busAPeak,
  busBRms, busBPeak,
  onCaptureScene,
}: MasterSectionProps) {
  return (
    <div className="faceplate flex flex-col gap-2 p-2 w-32" style={{ height: '100%' }}>
      {/* Header */}
      <div className="text-center engraved">Master</div>

      {/* Master VU meters (stereo) */}
      <div className="flex justify-center gap-1">
        <VuMeter rms={masterRms} peak={masterPeak} size="md" label="L" />
        <VuMeter rms={masterRms} peak={masterPeak} size="md" label="R" />
      </div>

      {/* Clip indicator */}
      {masterClip && (
        <div className="cap cap-red lit text-center text-[8px] font-bold py-0.5">
          CLIP
        </div>
      )}

      {/* Master fader */}
      <div className="flex justify-center">
        <Fader
          value={masterGain}
          default={0.8}
          onChange={onMasterGain}
          height={100}
          label="MAIN"
        />
      </div>

      {/* Divider */}
      <div className="h-px bg-plate-darker/60" />

      {/* Bus A/B meters */}
      <div className="flex justify-around">
        <VuMeter rms={busARms} peak={busAPeak} size="sm" label="BUS A" />
        <VuMeter rms={busBRms} peak={busBPeak} size="sm" label="BUS B" />
      </div>

      {/* Crossfader */}
      <div className="flex flex-col gap-0.5">
        <div className="flex justify-between text-[7px] font-mono text-label-dim">
          <span>A</span>
          <span className="engraved-sm">XFADE</span>
          <span>B</span>
        </div>
        <input
          type="range"
          min="0"
          max="1"
          step="0.01"
          value={crossfade}
          onChange={(e) => onCrossfade(parseFloat(e.target.value))}
          className="w-full accent-cap-amber"
          style={{ height: 4 }}
        />
        <div className="text-center text-[7px] font-mono text-label-dim">
          {(crossfade * 100).toFixed(0)}%
        </div>
      </div>

      {/* Scene capture */}
      <div className="h-px bg-plate-darker/60" />
      <button
        onClick={onCaptureScene}
        className="cap cap-amber flex items-center justify-center gap-1 py-1.5"
      >
        <Camera className="w-3 h-3" />
        <span className="text-[8px] font-bold">SCENE</span>
      </button>
    </div>
  );
}
