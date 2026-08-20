// ConsoleView — the main console view, the landing page of TuneLock.
//
// Layout:
//   ┌──────────────────────────────────────────────────────────────┐
//   │ CHANNEL STRIPS (1-8)          │ MASTER SECTION               │
//   │ [strip][strip][strip]...      │ [VU][VU]                     │
//   │ [knob][knob][knob]...         │ [fader]                      │
//   │ [fader][fader][fader]...      │ [bus A/B meters]             │
//   │                               │ [crossfader]                 │
//   │                               │ [scene capture]              │
//   ├───────────────────────────────┴──────────────────────────────┤
//   │ TRANSPORT BAR                                                 │
//   └──────────────────────────────────────────────────────────────┘
//
// Click an empty channel to open the library drawer and load a track.
// Click a loaded channel to select it. The selected channel can be
// controlled via the transport and the library drawer.

import { useState, useEffect, useRef, useCallback } from 'react';
import { FolderOpen } from 'lucide-react';
import ChannelStrip, { type ChannelState, EMPTY_CHANNEL } from './ChannelStrip';
import MasterSection from './MasterSection';
import ConsoleTransport from './ConsoleTransport';
import LibraryDrawer from './LibraryDrawer';
import { useLibraryStore } from '../../stores/libraryStore';
import {
  audioEngineInit,
  audioEnginePlay,
  audioEnginePause,
  audioEngineStop,
  audioEngineLoadPlayer,
  audioEngineSetMute,
  audioEngineSetSolo,
  audioEngineSetBus,
  audioEngineSetPlayerGain,
  audioEngineSetCrossfade,
  audioEngineSetMasterGain,
  audioEngineGetMeters,
  type AudioMeterReadout,
} from '../../lib/tauri';

const MAX_CHANNELS = 8;

export default function ConsoleView() {
  const { tracks } = useLibraryStore();
  const [channels, setChannels] = useState<ChannelState[]>(
    Array.from({ length: MAX_CHANNELS }, () => ({ ...EMPTY_CHANNEL }))
  );
  const [selectedChannel, setSelectedChannel] = useState<number | null>(null);
  const [libraryOpen, setLibraryOpen] = useState(false);
  const [crossfade, setCrossfade] = useState(0.5);
  const [masterGain, setMasterGain] = useState(0.8);
  const [looping, setLooping] = useState(false);
  const [positionSec, setPositionSec] = useState(0);
  const [meters, setMeters] = useState<AudioMeterReadout | null>(null);
  const [engineReady, setEngineReady] = useState(false);
  const meterRef = useRef<ReturnType<typeof setInterval> | null>(null);

  // Initialize engine on mount — with safe error handling
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        await audioEngineInit();
        if (!cancelled) setEngineReady(true);
      } catch (e) {
        console.warn('Audio engine init failed (may already be running):', e);
        if (!cancelled) setEngineReady(true); // assume it's already running
      }
    })();

    // Start meter polling
    meterRef.current = setInterval(async () => {
      try {
        const m = await audioEngineGetMeters();
        if (!cancelled) {
          setMeters(m);
          // Update channel meters from engine
          setChannels(prev => prev.map((ch, i) => {
            if (i < m.players.length && ch.trackId !== null) {
              return {
                ...ch,
                rms: m.players[i].rms,
                peak: m.players[i].peak,
                clip: m.players[i].clip,
              };
            }
            return ch;
          }));
          setPositionSec(m.players.find(p => p.playing)?.positionSec ?? 0);
        }
      } catch {
        // Engine not ready yet — silent
      }
    }, 50); // 20 Hz

    return () => {
      cancelled = true;
      if (meterRef.current) clearInterval(meterRef.current);
    };
  }, []);

  // Update a single channel
  const updateChannel = useCallback((index: number, partial: Partial<ChannelState>) => {
    setChannels(prev => prev.map((ch, i) => i === index ? { ...ch, ...partial } : ch));
  }, []);

  // Load a track into a channel
  const handleLoadTrack = useCallback(async (channelIndex: number, trackId: number) => {
    const track = tracks.get(trackId);
    if (!track) return;

    updateChannel(channelIndex, {
      trackId,
      trackTitle: track.title || track.filename || 'Unknown',
      trackArtist: track.artist || '',
      keyCamelot: track.key_camelot || null,
      bpm: track.bpm ?? null,
      playing: false,
    });

    // Load into engine
    if (track.file_path) {
      try {
        await audioEngineLoadPlayer(channelIndex, track.file_path);
      } catch (e) {
        console.warn('Failed to load player:', e);
      }
    }
  }, [tracks, updateChannel]);

  // Play/pause a channel
  const handlePlayPause = useCallback(async (index: number) => {
    const ch = channels[index];
    if (!ch.trackId) return;
    if (ch.playing) {
      try { await audioEnginePause(index); } catch {}
      updateChannel(index, { playing: false });
    } else {
      try { await audioEnginePlay(index); } catch {}
      updateChannel(index, { playing: true });
    }
  }, [channels, updateChannel]);

  // Stop a channel
  const handleStop = useCallback(async (index: number) => {
    try { await audioEngineStop(index); } catch {}
    updateChannel(index, { playing: false });
  }, [updateChannel]);

  // Remove a track from a channel
  const handleRemove = useCallback(async (index: number) => {
    try { await audioEngineStop(index); } catch {}
    updateChannel(index, { ...EMPTY_CHANNEL });
  }, [updateChannel]);

  // Handle channel state changes that need to talk to the engine
  const handleChannelChange = useCallback((index: number, partial: Partial<ChannelState>) => {
    updateChannel(index, partial);

    // Sync to engine
    if ('muted' in partial) {
      audioEngineSetMute(index, partial.muted!).catch(() => {});
    }
    if ('soloed' in partial) {
      audioEngineSetSolo(index, partial.soloed!).catch(() => {});
    }
    if ('bus' in partial) {
      audioEngineSetBus(index, partial.bus!).catch(() => {});
    }
    if ('gain' in partial) {
      audioEngineSetPlayerGain(index, partial.gain!).catch(() => {});
    }
  }, [updateChannel]);

  // Master controls
  const handleMasterGain = useCallback((v: number) => {
    setMasterGain(v);
    audioEngineSetMasterGain(v).catch(() => {});
  }, []);

  const handleCrossfade = useCallback((v: number) => {
    setCrossfade(v);
    audioEngineSetCrossfade(v).catch(() => {});
  }, []);

  // Transport
  const handleTransportPlayPause = useCallback(() => {
    // Play/pause the first playing channel, or channel 0
    const firstPlaying = channels.findIndex(ch => ch.playing);
    const target = firstPlaying >= 0 ? firstPlaying : channels.findIndex(ch => ch.trackId !== null);
    if (target >= 0) handlePlayPause(target);
  }, [channels, handlePlayPause]);

  const handleTransportStop = useCallback(() => {
    channels.forEach((ch, i) => {
      if (ch.playing) handleStop(i);
    });
  }, [channels, handleStop]);

  const handleEmptyChannelClick = useCallback((index: number) => {
    setSelectedChannel(index);
    setLibraryOpen(true);
  }, []);

  const loadedCount = channels.filter(ch => ch.trackId !== null).length;
  const playingCount = channels.filter(ch => ch.playing).length;

  return (
    <div className="flex flex-col h-full bg-background">
      {/* Top bar */}
      <div className="faceplate-flat flex items-center justify-between px-4 py-2">
        <div className="flex items-center gap-3">
          <span className="engraved">TuneLock Console</span>
          <span className="text-[10px] text-label-dim">
            {loadedCount} loaded · {playingCount} playing · {engineReady ? 'Engine Ready' : 'Starting...'}
          </span>
        </div>
        <div className="flex items-center gap-2">
          <button
            onClick={() => setLibraryOpen(true)}
            className="cap cap-amber flex items-center gap-1.5 px-3 py-1.5"
          >
            <FolderOpen className="w-3.5 h-3.5" />
            <span className="text-[10px] font-bold">LIBRARY</span>
          </button>
        </div>
      </div>

      {/* Console body */}
      <div className="flex flex-1 overflow-hidden">
        {/* Channel strips */}
        <div className="flex-1 flex overflow-x-auto">
          <div className="flex" style={{ minWidth: 'fit-content' }}>
            {channels.map((channel, i) => (
              <ChannelStrip
                key={i}
                index={i}
                channel={channel}
                selected={selectedChannel === i}
                onSelect={() => {
                  if (channel.trackId === null) {
                    handleEmptyChannelClick(i);
                  } else {
                    setSelectedChannel(i);
                  }
                }}
                onPlayPause={() => handlePlayPause(i)}
                onStop={() => handleStop(i)}
                onRemove={() => handleRemove(i)}
                onChange={(partial) => handleChannelChange(i, partial)}
              />
            ))}
          </div>
        </div>

        {/* Master section */}
        <MasterSection
          masterGain={masterGain}
          onMasterGain={handleMasterGain}
          crossfade={crossfade}
          onCrossfade={handleCrossfade}
          masterRms={meters?.masterRms ?? 0}
          masterPeak={meters?.masterPeak ?? 0}
          masterClip={meters?.masterClip ?? false}
          busARms={meters?.busARms ?? 0}
          busAPeak={meters?.busAPeak ?? 0}
          busBRms={meters?.busBRms ?? 0}
          busBPeak={meters?.busBPeak ?? 0}
          onCaptureScene={() => {
            // TODO: implement scene capture
            console.log('Scene capture not yet implemented');
          }}
        />
      </div>

      {/* Transport bar */}
      <ConsoleTransport
        playing={playingCount > 0}
        onPlayPause={handleTransportPlayPause}
        onStop={handleTransportStop}
        onLoop={() => setLooping(!looping)}
        looping={looping}
        positionSec={positionSec}
        totalSec={0}
      />

      {/* Library drawer */}
      <LibraryDrawer
        open={libraryOpen}
        onClose={() => setLibraryOpen(false)}
        selectedChannel={selectedChannel}
        onLoadTrack={handleLoadTrack}
      />
    </div>
  );
}
