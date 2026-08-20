// Layer Lab — eight-slot exploratory grid for the Mix Canvas.
//
// Each slot represents one player in the generalized audio engine.
// Slots 0 and 1 map to the Transition Workbench's A and B decks.
// Up to 8 slots are available; 2-4 are typically active.
//
// Compact slot display:
//   - Track/sample name and musical role
//   - Miniature waveform
//   - Playback position and beat phase
//   - Camelot key and source BPM
//   - Gain meter
//   - Mute, solo, loop, launch state
//   - A/B/Master bus assignment
//   - Ready, queued, playing, or stopped state
//
// Only the selected slot expands to show EQ, cue regions, detailed
// waveform, loop length, stems, and routing.
//
// Design language: Walnut Console — slot cards use the charcoal data
// plane with brass-focus selection. The frame around the grid uses
// the walnut/bronze frame tokens.

import { useState, useEffect, useRef, useCallback } from 'react';
import { Play, Pause, Square, VolumeX, Headphones, X } from 'lucide-react';
import { useLibraryStore } from '../../stores/libraryStore';
import {
  audioEngineInit,
  audioEnginePlay,
  audioEnginePause,
  audioEngineStop,
  audioEngineSetMute,
  audioEngineSetSolo,
  audioEngineSetBus,
  audioEngineGetMeters,
  type AudioMeterReadout,
  type PlayerMeterEntry,
} from '../../lib/tauri';

const MAX_PLAYERS = 8;
const ROLES = ['Foundation', 'Drums', 'Bass', 'Vocal', 'Harmony', 'Texture', 'FX'] as const;
type Role = typeof ROLES[number];

interface SlotState {
  trackId: number | null;
  role: Role;
  bus: 'a' | 'b' | 'master';
  muted: boolean;
  soloed: boolean;
  looping: boolean;
  playing: boolean;
}

const DEFAULT_SLOTS: SlotState[] = Array.from({ length: MAX_PLAYERS }, (_, i) => ({
  trackId: null,
  role: i === 0 ? 'Foundation' : i === 1 ? 'Drums' : 'Texture',
  bus: i % 2 === 0 ? 'a' : 'b',
  muted: false,
  soloed: false,
  looping: false,
  playing: false,
}));

export default function LayerLab() {
  const { tracks } = useLibraryStore();
  const [slots, setSlots] = useState<SlotState[]>(DEFAULT_SLOTS);
  const [selectedSlot, setSelectedSlot] = useState<number | null>(null);
  const [engineReady, setEngineReady] = useState(false);
  const [meters, setMeters] = useState<AudioMeterReadout | null>(null);
  const meterIntervalRef = useRef<ReturnType<typeof setInterval> | null>(null);

  // Initialize the audio engine on mount
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        await audioEngineInit();
        if (!cancelled) {
          setEngineReady(true);
          // Start meter polling at ~30 Hz
          meterIntervalRef.current = setInterval(async () => {
            try {
              const m = await audioEngineGetMeters();
              if (!cancelled) setMeters(m);
            } catch {
              // engine might not be ready yet
            }
          }, 33);
        }
      } catch (e) {
        // Engine may already be initialized
        if (!cancelled) setEngineReady(true);
      }
    })();
    return () => {
      cancelled = true;
      if (meterIntervalRef.current) clearInterval(meterIntervalRef.current);
    };
  }, []);

  const updateSlot = useCallback((index: number, partial: Partial<SlotState>) => {
    setSlots(prev => prev.map((s, i) => i === index ? { ...s, ...partial } : s));
  }, []);

  const handleStop = useCallback(async (slotIndex: number) => {
    await audioEngineStop(slotIndex).catch(() => {});
    updateSlot(slotIndex, { playing: false });
  }, [updateSlot]);

  const handlePlayPause = useCallback(async (slotIndex: number) => {
    const slot = slots[slotIndex];
    if (!slot.trackId) return;
    if (slot.playing) {
      await audioEnginePause(slotIndex).catch(() => {});
      updateSlot(slotIndex, { playing: false });
    } else {
      await audioEnginePlay(slotIndex).catch(() => {});
      updateSlot(slotIndex, { playing: true });
    }
  }, [slots, updateSlot]);

  const handleMute = useCallback(async (slotIndex: number) => {
    const newMuted = !slots[slotIndex].muted;
    await audioEngineSetMute(slotIndex, newMuted).catch(() => {});
    updateSlot(slotIndex, { muted: newMuted });
  }, [slots, updateSlot]);

  const handleSolo = useCallback(async (slotIndex: number) => {
    const newSoloed = !slots[slotIndex].soloed;
    await audioEngineSetSolo(slotIndex, newSoloed).catch(() => {});
    updateSlot(slotIndex, { soloed: newSoloed });
  }, [slots, updateSlot]);

  const handleBusChange = useCallback(async (slotIndex: number, bus: 'a' | 'b' | 'master') => {
    await audioEngineSetBus(slotIndex, bus).catch(() => {});
    updateSlot(slotIndex, { bus });
  }, [updateSlot]);

  const handleRemoveTrack = useCallback(async (slotIndex: number) => {
    await audioEngineStop(slotIndex).catch(() => {});
    updateSlot(slotIndex, { trackId: null, playing: false });
  }, [updateSlot]);

  const activeCount = slots.filter(s => s.trackId !== null).length;
  const playingCount = slots.filter(s => s.playing).length;

  return (
    <div className="walnut-frame p-3 flex flex-col gap-2">
      {/* Header */}
      <div className="flex items-center justify-between px-1">
        <div className="flex items-center gap-3">
          <span className="engraved-label">Layer Lab</span>
          <span className="text-[10px] text-cream-label/60">
            {activeCount} active · {playingCount} playing · {MAX_PLAYERS} slots
          </span>
        </div>
        <div className="flex items-center gap-2">
          <span className={`text-[10px] ${engineReady ? 'text-lamp-green' : 'text-lamp-amber'}`}>
            ● {engineReady ? 'Engine Ready' : 'Engine Starting...'}
          </span>
        </div>
      </div>

      {/* Slot grid: 4 columns x 2 rows */}
      <div className="grid grid-cols-4 gap-2">
        {slots.map((slot, i) => (
          <SlotCard
            key={i}
            index={i}
            slot={slot}
            track={slot.trackId ? tracks.get(slot.trackId) : null}
            meter={meters?.players[i] ?? null}
            crossfadePosition={meters?.crossfadePosition ?? 0.5}
            selected={selectedSlot === i}
            onSelect={() => setSelectedSlot(i)}
            onPlayPause={() => handlePlayPause(i)}
            onStop={() => handleStop(i)}
            onMute={() => handleMute(i)}
            onSolo={() => handleSolo(i)}
            onBusChange={(bus) => handleBusChange(i, bus)}
            onRemove={() => handleRemoveTrack(i)}
          />
        ))}
      </div>

      {/* Expanded slot detail (selected slot) */}
      {selectedSlot !== null && slots[selectedSlot].trackId && (
        <ExpandedSlotDetail
          index={selectedSlot}
          slot={slots[selectedSlot]}
          track={tracks.get(slots[selectedSlot].trackId!)}
          meter={meters?.players[selectedSlot] ?? null}
          onMute={() => handleMute(selectedSlot)}
          onSolo={() => handleSolo(selectedSlot)}
          onBusChange={(bus) => handleBusChange(selectedSlot, bus)}
        />
      )}

      {/* Drop zone hint when no track is loaded */}
      {activeCount === 0 && (
        <div className="text-center py-4 text-xs text-cream-label/40">
          Drag tracks from the Library rail into a slot, or click a slot to load.
        </div>
      )}
    </div>
  );
}

// ============================================================================
// SlotCard — compact slot display
// ============================================================================

interface SlotCardProps {
  index: number;
  slot: SlotState;
  track: any;
  meter: PlayerMeterEntry | null;
  crossfadePosition: number;
  selected: boolean;
  onSelect: () => void;
  onPlayPause: () => void;
  onStop: () => void;
  onMute: () => void;
  onSolo: () => void;
  onBusChange: (bus: 'a' | 'b' | 'master') => void;
  onRemove: () => void;
}

function SlotCard({
  index, slot, track, meter, selected, onSelect, onPlayPause, onStop, onMute, onSolo, onRemove,
}: SlotCardProps) {
  const isEmpty = !track;

  return (
    <div
      className={`slot-card p-2 cursor-pointer ${selected ? 'selected' : ''} ${isEmpty ? 'empty' : ''}`}
      onClick={onSelect}
    >
      {/* Slot header: number + state + bus */}
      <div className="flex items-center justify-between mb-1">
        <div className="flex items-center gap-1.5">
          <span className="text-[10px] font-mono text-cream-label/60">{index + 1}</span>
          {slot.playing && (
            <span className="w-1.5 h-1.5 rounded-full bg-lamp-green shadow-[0_0_4px_rgba(92,156,92,0.6)]" />
          )}
          {slot.trackId && !slot.playing && (
            <span className="w-1.5 h-1.5 rounded-full bg-lamp-amber/60" />
          )}
        </div>
        <div className="flex items-center gap-1">
          {track && (
            <button
              onClick={(e) => { e.stopPropagation(); onRemove(); }}
              className="text-text-secondary hover:text-lamp-red transition-colors"
              title="Unload"
            >
              <X className="w-3 h-3" />
            </button>
          )}
        </div>
      </div>

      {track ? (
        <>
          {/* Track name */}
          <div className="text-[11px] text-data-text font-medium truncate mb-0.5">
            {track.title || track.filename}
          </div>

          {/* Role + key + BPM */}
          <div className="flex items-center gap-1.5 text-[9px] text-data-text-dim mb-1">
            <span className="px-1 rounded bg-walnut-dark/50 text-cream-label/70">{slot.role}</span>
            {track.key_camelot && (
              <span className="font-mono text-brass-accent">{track.key_camelot}</span>
            )}
            {track.bpm && (
              <span className="font-mono">{track.bpm.toFixed(0)}</span>
            )}
          </div>

          {/* Mini waveform placeholder */}
          <div className="h-6 data-plane rounded mb-1 flex items-center justify-center overflow-hidden relative">
            <MiniWaveform playing={slot.playing} meter={meter} />
          </div>

          {/* Meter bar */}
          {meter && (
            <MeterBar rms={meter.rms} peak={meter.peak} clip={meter.clip} />
          )}

          {/* Controls */}
          <div className="flex items-center gap-1 mt-1">
            <button
              onClick={(e) => { e.stopPropagation(); onPlayPause(); }}
              className="launch-pad p-1 text-cream-label/80 hover:text-brass-bright"
              title={slot.playing ? 'Pause' : 'Play'}
            >
              {slot.playing ? <Pause className="w-3 h-3" /> : <Play className="w-3 h-3" />}
            </button>
            <button
              onClick={(e) => { e.stopPropagation(); onStop(); }}
              className="launch-pad p-1 text-cream-label/60 hover:text-lamp-red"
              title="Stop"
            >
              <Square className="w-3 h-3" />
            </button>
            <button
              onClick={(e) => { e.stopPropagation(); onMute(); }}
              className={`lamp-btn p-1 ${slot.muted ? 'lamp-on-red' : ''}`}
              title="Mute"
            >
              <VolumeX className="w-3 h-3 text-cream-label/70" />
            </button>
            <button
              onClick={(e) => { e.stopPropagation(); onSolo(); }}
              className={`lamp-btn p-1 ${slot.soloed ? 'lamp-on-amber' : ''}`}
              title="Solo"
            >
              <Headphones className="w-3 h-3 text-cream-label/70" />
            </button>
            {/* Bus indicator */}
            <span className={`ml-auto text-[9px] font-mono px-1 rounded ${
              slot.bus === 'a' ? 'bg-bronze-dark/50 text-cream-label/80' :
              slot.bus === 'b' ? 'bg-walnut-light/50 text-cream-label/80' :
              'bg-data-border text-data-text-dim'
            }`}>
              {slot.bus.toUpperCase()}
            </span>
          </div>
        </>
      ) : (
        <div className="flex flex-col items-center justify-center h-16 text-[10px] text-data-text-dim">
          <span>Empty</span>
          <span className="text-[8px] mt-0.5">Click to load</span>
        </div>
      )}
    </div>
  );
}

// ============================================================================
// MiniWaveform — simple animated placeholder
// ============================================================================

function MiniWaveform({ playing, meter }: { playing: boolean; meter: PlayerMeterEntry | null }) {
  const bars = 24;
  const [phase, setPhase] = useState(0);

  useEffect(() => {
    if (!playing) return;
    const id = setInterval(() => setPhase(p => (p + 1) % bars), 60);
    return () => clearInterval(id);
  }, [playing]);

  return (
    <div className="flex items-center gap-px h-full w-full px-1">
      {Array.from({ length: bars }).map((_, i) => {
        const baseHeight = 0.2 + Math.abs(Math.sin((i + phase) * 0.5)) * 0.3;
        const meterBoost = meter ? meter.rms * 0.4 : 0;
        const height = Math.min(baseHeight + meterBoost, 1.0);
        return (
          <div
            key={i}
            className="flex-1 rounded-sm"
            style={{
              height: `${height * 100}%`,
              backgroundColor: i === phase && playing ? 'var(--brass-accent)' : 'var(--bronze-face)',
              opacity: playing ? 0.8 : 0.3,
            }}
          />
        );
      })}
    </div>
  );
}

// ============================================================================
// MeterBar — compact RMS/peak meter
// ============================================================================

function MeterBar({ rms, peak, clip }: { rms: number; peak: number; clip: boolean }) {
  const rmsPct = Math.min(rms * 100, 100);
  const peakPct = Math.min(peak * 100, 100);

  return (
    <div className="h-1 bg-walnut-dark rounded-full overflow-hidden relative">
      <div
        className="h-full bg-lamp-green transition-all duration-75"
        style={{ width: `${rmsPct}%` }}
      />
      <div
        className={`absolute top-0 h-full w-px ${clip ? 'bg-lamp-red' : 'bg-brass-bright'}`}
        style={{ left: `${peakPct}%` }}
      />
    </div>
  );
}

// ============================================================================
// ExpandedSlotDetail — shown when a slot is selected
// ============================================================================

function ExpandedSlotDetail({
  index, slot, track, meter, onMute, onSolo, onBusChange,
}: {
  index: number;
  slot: SlotState;
  track: any;
  meter: PlayerMeterEntry | null;
  onMute: () => void;
  onSolo: () => void;
  onBusChange: (bus: 'a' | 'b' | 'master') => void;
}) {
  if (!track) return null;

  return (
    <div className="bronze-plate p-3 mt-1">
      <div className="flex items-center justify-between mb-2">
        <span className="engraved-label">Player {index + 1} — Detail</span>
        <span className="text-[10px] text-cream-label/60">{slot.role}</span>
      </div>

      <div className="grid grid-cols-3 gap-3 text-[11px]">
        {/* Track info */}
        <div className="data-plane p-2">
          <div className="text-data-text-dim text-[9px] uppercase mb-1">Track</div>
          <div className="text-data-text truncate">{track.title || track.filename}</div>
          <div className="text-data-text-dim text-[10px] truncate">{track.artist}</div>
          <div className="flex gap-2 mt-1">
            {track.key_camelot && (
              <span className="font-mono text-brass-accent">{track.key_camelot}</span>
            )}
            {track.bpm && (
              <span className="font-mono text-data-text">{track.bpm.toFixed(1)} BPM</span>
            )}
            {track.energy && (
              <span className="font-mono text-data-text-dim">E{track.energy}</span>
            )}
          </div>
        </div>

        {/* Routing */}
        <div className="data-plane p-2">
          <div className="text-data-text-dim text-[9px] uppercase mb-1">Bus Routing</div>
          <div className="flex gap-1">
            {(['a', 'b', 'master'] as const).map(bus => (
              <button
                key={bus}
                onClick={() => onBusChange(bus)}
                className={`lamp-btn px-2 py-1 text-[10px] font-mono ${
                  slot.bus === bus ? 'lamp-on-amber' : ''
                }`}
              >
                {bus.toUpperCase()}
              </button>
            ))}
          </div>
          <div className="flex gap-1 mt-2">
            <button
              onClick={onMute}
              className={`lamp-btn px-2 py-1 text-[10px] ${slot.muted ? 'lamp-on-red' : ''}`}
            >
              MUTE
            </button>
            <button
              onClick={onSolo}
              className={`lamp-btn px-2 py-1 text-[10px] ${slot.soloed ? 'lamp-on-amber' : ''}`}
            >
              SOLO
            </button>
          </div>
        </div>

        {/* Meter */}
        <div className="data-plane p-2">
          <div className="text-data-text-dim text-[9px] uppercase mb-1">Level</div>
          {meter && (
            <>
              <MeterBar rms={meter.rms} peak={meter.peak} clip={meter.clip} />
              <div className="text-[10px] font-mono text-data-text-dim mt-1">
                RMS: {(meter.rms * 100).toFixed(1)} · Peak: {(meter.peak * 100).toFixed(1)}
                {meter.clip && <span className="text-lamp-red ml-1">CLIP</span>}
              </div>
              <div className="text-[10px] font-mono text-data-text-dim">
                Pos: {meter.positionSec.toFixed(1)}s
              </div>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
