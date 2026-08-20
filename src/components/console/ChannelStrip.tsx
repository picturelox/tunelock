// ChannelStrip — one channel of the API 2500-style console.
//
// Each strip has:
//   - Track name display
//   - EQ knobs (HI, MID, LO)
//   - Pan knob
//   - Mute / Solo button caps
//   - VU meter
//   - Fader
//   - Bus assignment (A / B / Master)
//   - Player number
//
// Empty strips show a striped pattern and can be loaded by clicking.

import { useCallback } from 'react';
import { Play, Pause, Square, X } from 'lucide-react';
import Knob from './Knob';
import Fader from './Fader';
import VuMeter from './VuMeter';

export interface ChannelState {
  trackId: number | null;
  trackTitle: string;
  trackArtist: string;
  keyCamelot: string | null;
  bpm: number | null;
  gain: number;       // 0-1
  pan: number;        // -1 to 1
  eqLow: number;      // -12 to +12 dB
  eqMid: number;
  eqHigh: number;
  muted: boolean;
  soloed: boolean;
  playing: boolean;
  bus: 'a' | 'b' | 'master';
  rms: number;
  peak: number;
  clip: boolean;
}

export const EMPTY_CHANNEL: ChannelState = {
  trackId: null,
  trackTitle: '',
  trackArtist: '',
  keyCamelot: null,
  bpm: null,
  gain: 0.75,
  pan: 0,
  eqLow: 0,
  eqMid: 0,
  eqHigh: 0,
  muted: false,
  soloed: false,
  playing: false,
  bus: 'master',
  rms: 0,
  peak: 0,
  clip: false,
};

interface ChannelStripProps {
  index: number;
  channel: ChannelState;
  selected: boolean;
  onSelect: () => void;
  onPlayPause: () => void;
  onStop: () => void;
  onRemove: () => void;
  onChange: (partial: Partial<ChannelState>) => void;
}

export default function ChannelStrip({
  index, channel, selected, onSelect, onPlayPause, onStop, onRemove, onChange,
}: ChannelStripProps) {
  const isEmpty = channel.trackId === null;

  const handleChange = useCallback((partial: Partial<ChannelState>) => {
    onChange(partial);
  }, [onChange]);

  if (isEmpty) {
    return (
      <div
        className={`channel-strip flex flex-col items-center w-16 ${selected ? 'ring-1 ring-cap-amber/50' : ''}`}
        style={{ height: '100%' }}
        onClick={onSelect}
      >
        <div className="engraved-sm py-1">{index + 1}</div>
        <div className="flex-1 slot-empty flex items-center justify-center">
          <span className="text-[8px] text-label-dim/40 rotate-90 whitespace-nowrap tracking-widest">
            EMPTY
          </span>
        </div>
      </div>
    );
  }

  return (
    <div
      className={`channel-strip flex flex-col items-center w-16 gap-1 py-1 ${
        selected ? 'ring-1 ring-cap-amber/60' : ''
      }`}
      style={{ height: '100%' }}
      onClick={onSelect}
    >
      {/* Channel number */}
      <div className="engraved-sm">{index + 1}</div>

      {/* Track name (truncated, vertical) */}
      <div
        className="w-full text-center text-[8px] text-label-cream px-0.5 truncate"
        title={`${channel.trackTitle} — ${channel.trackArtist}`}
      >
        {channel.trackTitle}
      </div>

      {/* Key + BPM */}
      <div className="flex gap-0.5 text-[8px] font-mono">
        {channel.keyCamelot && (
          <span className="text-cap-amber">{channel.keyCamelot}</span>
        )}
        {channel.bpm && (
          <span className="text-label-dim">{channel.bpm.toFixed(0)}</span>
        )}
      </div>

      {/* Transport buttons */}
      <div className="flex gap-0.5">
        <button
          onClick={(e) => { e.stopPropagation(); onPlayPause(); }}
          className={`cap ${channel.playing ? 'cap-green lit' : 'cap-dark'} w-5 h-5 flex items-center justify-center`}
          title={channel.playing ? 'Pause' : 'Play'}
        >
          {channel.playing ? <Pause className="w-2.5 h-2.5" /> : <Play className="w-2.5 h-2.5" />}
        </button>
        <button
          onClick={(e) => { e.stopPropagation(); onStop(); }}
          className="cap cap-red w-5 h-5 flex items-center justify-center"
          title="Stop"
        >
          <Square className="w-2.5 h-2.5" />
        </button>
      </div>

      {/* EQ knobs */}
      <div className="flex flex-col items-center gap-0.5">
        <Knob
          value={channel.eqHigh}
          min={-12} max={12} default={0}
          onChange={(v) => handleChange({ eqHigh: v })}
          label="HI"
          size={24}
        />
        <Knob
          value={channel.eqMid}
          min={-12} max={12} default={0}
          onChange={(v) => handleChange({ eqMid: v })}
          label="MID"
          size={24}
        />
        <Knob
          value={channel.eqLow}
          min={-12} max={12} default={0}
          onChange={(v) => handleChange({ eqLow: v })}
          label="LO"
          size={24}
        />
      </div>

      {/* Pan knob */}
      <Knob
        value={channel.pan}
        min={-1} max={1} default={0}
        onChange={(v) => handleChange({ pan: v })}
        label="PAN"
        size={22}
      />

      {/* Mute / Solo */}
      <div className="flex gap-0.5">
        <button
          onClick={(e) => { e.stopPropagation(); handleChange({ muted: !channel.muted }); }}
          className={`cap ${channel.muted ? 'cap-red lit' : 'cap-dark'} w-6 h-4 flex items-center justify-center`}
        >
          <span className="text-[7px] font-bold">M</span>
        </button>
        <button
          onClick={(e) => { e.stopPropagation(); handleChange({ soloed: !channel.soloed }); }}
          className={`cap ${channel.soloed ? 'cap-amber lit' : 'cap-dark'} w-6 h-4 flex items-center justify-center`}
        >
          <span className="text-[7px] font-bold">S</span>
        </button>
      </div>

      {/* Bus selector */}
      <div className="flex gap-0.5">
        {(['a', 'b', 'master'] as const).map(bus => (
          <button
            key={bus}
            onClick={(e) => { e.stopPropagation(); handleChange({ bus }); }}
            className={`cap ${channel.bus === bus ? 'cap-amber lit' : 'cap-dark'} px-1 h-4 flex items-center justify-center`}
          >
            <span className="text-[6px] font-bold uppercase">{bus === 'master' ? 'MST' : bus}</span>
          </button>
        ))}
      </div>

      {/* VU meter */}
      <VuMeter rms={channel.rms} peak={channel.peak} size="sm" />

      {/* Fader */}
      <Fader
        value={channel.gain}
        default={0.75}
        onChange={(v) => handleChange({ gain: v })}
        height={100}
      />

      {/* Remove button */}
      <button
        onClick={(e) => { e.stopPropagation(); onRemove(); }}
        className="cap cap-dark w-5 h-4 flex items-center justify-center mt-auto"
        title="Unload"
      >
        <X className="w-2.5 h-2.5" />
      </button>
    </div>
  );
}
