import { useState, useEffect, useCallback } from 'react';
import { Upload, FolderOpen, Play, Pause, Square, SkipBack, FastForward, X, Disc3 } from 'lucide-react';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { open } from '@tauri-apps/plugin-dialog';
import {
  analyzeFile,
  onTunerProgress,
  getLibraryPage,
  onTrackAnalyzed,
  getWaveformData,
  getLoudnessComparison,
  type LoudnessComparison,
  type WaveformData,
} from '../../lib/tauri';
import type { TrackAnalysis, TunerProgress, KeyCandidate, Track } from '../../types';
import { formatCamelotBadge } from '../../lib/harmony';
import ReadoutCard from '../tuner/ReadoutCard';
import AnalysisProgressDisplay from '../tuner/AnalysisProgressDisplay';
import HarmonicMosaic, { type FocalTrack } from '../mosaic/HarmonicMosaic';
import WaveformDisplay from '../waveform/WaveformDisplay';
import LibraryTable from '../library/LibraryTable';
import { sessionService } from '../../session/sessionService';
import { useSessionStore } from '../../session/sessionStore';

// ─── helpers ────────────────────────────────────────────────────────────

function linearToDbfs(value: number | null | undefined): number | null {
  if (typeof value !== 'number' || !Number.isFinite(value) || value < 0) return null;
  return value === 0 ? Number.NEGATIVE_INFINITY : 20 * Math.log10(value);
}

function fmtTime(sec: number) {
  const m = Math.floor(sec / 60);
  const s = sec % 60;
  return `${m}:${s.toFixed(3).padStart(6, '0')}`;
}

function MeterBar({ label, unit, value, over = false }: {
  label: string;
  unit: string;
  value: number | null;
  over?: boolean;
}) {
  const finite = typeof value === 'number' && Number.isFinite(value);
  const display = value === Number.NEGATIVE_INFINITY ? '−∞' : finite ? value.toFixed(1) : '—';
  const width = finite ? Math.max(0, Math.min(100, ((value + 60) / 66) * 100)) : 0;
  return (
    <div className="p-3 bg-plate-light rounded">
      <div className="text-xs text-label-dim mb-1">{label}</div>
      <div className={`font-mono text-xl tabular-nums ${over ? 'text-red-300' : 'text-label-cream'}`}>
        {display} <span className="text-xs text-label-dim">{unit}</span>
      </div>
      <div className="relative h-2 mt-2 bg-plate-darker rounded overflow-hidden" aria-hidden="true">
        <div className={`h-full ${over ? 'bg-red-500' : 'bg-cap-amber'}`} style={{ width: `${width}%` }} />
        <div className="absolute inset-y-0 w-px bg-label-cream" style={{ left: `${(60 / 66) * 100}%` }} />
      </div>
    </div>
  );
}

const TEMPO_PRESETS = [-10, -6, -2, 0, 2, 6, 10];
const PITCH_PRESETS = [-3, -1, 0, 1, 3];

// ─── component ──────────────────────────────────────────────────────────

export default function Workspace({ libraryOpen, setLibraryOpen }: {
  libraryOpen: boolean;
  setLibraryOpen: (open: boolean) => void;
}) {
  // Analysis state
  const [isDragging, setIsDragging] = useState(false);
  const [isAnalyzing, setIsAnalyzing] = useState(false);
  const [result, setResult] = useState<TrackAnalysis | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [filename, setFilename] = useState<string | null>(null);
  const [progress, setProgress] = useState<TunerProgress | null>(null);
  const [overrideKey, setOverrideKey] = useState<KeyCandidate | null>(null);
  const [, setHoveredCamelot] = useState<string | null>(null);
  const [libraryTracks, setLibraryTracks] = useState<Track[]>([]);
  const [waveform, setWaveform] = useState<WaveformData | null>(null);

  // The application-level session survives this component's lifetime.
  const engine = useSessionStore((state) => state.engine);
  const deckA = useSessionStore((state) => state.decks.A);
  const deckB = useSessionStore((state) => state.decks.B);
  const meters = useSessionStore((state) => state.meters);
  const engineState = engine.status;
  const engineError = engine.error ?? '';
  const filePathA = deckA.source?.filePath ?? '';
  const filePathB = deckB.source?.filePath ?? '';
  const trackNameB = deckB.source?.displayName ?? '';
  const isPlaying = deckA.acknowledgedTransport === 'playing';
  const tempo = (deckA.tempoRatio - 1) * 100;
  const pitch = deckA.pitchSemitones;
  const loopBeats = deckA.loopLengthBeats;
  const deckAPendingKinds = Object.keys(deckA.pendingCommands);
  const deckBPendingKinds = Object.keys(deckB.pendingCommands);
  const deckABusy = deckAPendingKinds.length > 0;
  const deckBBusy = deckBPendingKinds.length > 0;
  const positionSec = meters?.players?.[deckA.playerId]?.positionSec ?? 0;
  const [loudnessComp, setLoudnessComp] = useState<LoudnessComparison | null>(null);
  const [matchLevelOn, setMatchLevelOn] = useState(false);

  // Library drawer (controlled by parent)

  // ─── analysis ────────────────────────────────────────────────────────

  const handleAnalyzePath = useCallback(async (path: string, displayName: string) => {
    setError(null);
    setResult(null);
    setOverrideKey(null);
    setHoveredCamelot(null);
    setFilename(displayName);
    setProgress({ stage: 'decode', percent: 0 });
    setIsAnalyzing(true);
    try {
      const analysis = await analyzeFile(path);
      setResult(analysis);
      setProgress({ stage: 'done', percent: 1 });
    } catch (e) {
      setError(typeof e === 'string' ? e : 'Analysis failed.');
      setProgress(null);
    } finally {
      setIsAnalyzing(false);
    }
  }, []);

  const handleLoadAndAnalyze = useCallback(async (path: string, displayName: string) => {
    // Playback preparation and local analysis start independently. A decode or
    // device failure must not suppress the analysis result, and analysis must
    // not sit on the path to a playable deck.
    void sessionService.loadDeck('A', path, displayName).catch((loadError) => {
      console.warn('[workspace] Deck A load failed:', loadError);
    });
    await handleAnalyzePath(path, displayName);
  }, [handleAnalyzePath]);

  // Tauri drag-drop
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    (async () => {
      try {
        const webview = getCurrentWebview();
        unlisten = await webview.onDragDropEvent((event) => {
          if (cancelled) return;
          if (event.payload.type === 'enter' || event.payload.type === 'over') {
            setIsDragging(true);
          } else if (event.payload.type === 'leave') {
            setIsDragging(false);
          } else if (event.payload.type === 'drop') {
            setIsDragging(false);
            const paths = event.payload.paths;
            if (paths && paths.length > 0) {
              const fullPath = paths[0];
              const name = fullPath.split(/[\\/]/).pop() ?? fullPath;
              void handleLoadAndAnalyze(fullPath, name);
            }
          }
        });
        if (cancelled && unlisten) { unlisten(); unlisten = null; }
      } catch (err) {
        console.warn('Tauri drag-drop unavailable:', err);
      }
    })();
    return () => { cancelled = true; if (unlisten) unlisten(); };
  }, [handleLoadAndAnalyze]);

  // Tuner progress listener
  useEffect(() => {
    let unlisten: (() => void) | null = null;
    (async () => { unlisten = await onTunerProgress((p) => setProgress(p)); })();
    return () => { if (unlisten) unlisten(); };
  }, []);

  // Load library snapshot for the mosaic
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const page = await getLibraryPage(0, 500, 'key_camelot', 'asc');
        if (!cancelled) setLibraryTracks(page.tracks ?? []);
      } catch (e) {
        console.warn('[workspace] library load failed:', e);
      }
    })();
    return () => { cancelled = true; };
  }, []);

  // Live-update library when other analyses complete
  useEffect(() => {
    let unlisten: (() => void) | null = null;
    (async () => {
      unlisten = await onTrackAnalyzed((t) => {
        setLibraryTracks((prev) => {
          const idx = prev.findIndex((p) => p.id === t.id);
          if (idx === -1) return [...prev, t];
          const next = prev.slice();
          next[idx] = t;
          return next;
        });
      });
    })();
    return () => { if (unlisten) unlisten(); };
  }, []);

  // Waveform fetch
  useEffect(() => {
    setWaveform(null);
    if (!result?.track_id) return;
    let cancelled = false;
    (async () => {
      try {
        const data = await getWaveformData(result.track_id);
        if (!cancelled) setWaveform(data);
      } catch (err) {
        console.warn('[workspace] waveform fetch failed:', err);
      }
    })();
    return () => { cancelled = true; };
  }, [result?.track_id]);

  const handleOpenFile = useCallback(async () => {
    try {
      const selected = await open({
        directory: false,
        multiple: false,
        filters: [{
          name: 'Audio and video',
          extensions: ['mp3', 'wav', 'flac', 'aif', 'aiff', 'm4a', 'aac', 'ogg', 'mp4', 'mov', 'webm', 'mkv'],
        }],
      });
      if (typeof selected === 'string') {
        const name = selected.split(/[\\/]/).pop() ?? selected;
        await handleLoadAndAnalyze(selected, name);
      }
    } catch (err) {
      setError(typeof err === 'string' ? err : 'Could not open the file picker.');
    }
  }, [handleLoadAndAnalyze]);

  // ─── engine ──────────────────────────────────────────────────────────

  // Loudness comparison when both decks loaded
  useEffect(() => {
    if (!filePathA || !filePathB) { setLoudnessComp(null); return; }
    let active = true;
    getLoudnessComparison(filePathA, filePathB)
      .then(comp => { if (active) setLoudnessComp(comp); })
      .catch(() => { if (active) setLoudnessComp(null); });
    return () => { active = false; };
  }, [filePathA, filePathB]);

  const handleLoadDeckB = async () => {
    const selected = await open({
      filters: [{ name: 'Audio', extensions: ['wav', 'mp3', 'flac', 'aiff', 'm4a', 'ogg'] }],
      multiple: false,
    });
    if (!selected || typeof selected !== 'string') return;
    try {
      await sessionService.loadDeck(
        'B',
        selected,
        selected.split(/[\\/]/).pop() || selected,
      );
    } catch (e) {
      console.error('Load Deck B failed:', e);
    }
  };

  const handlePlay = async () => { try { await sessionService.setTransport('A', 'playing'); } catch (e) { console.error(e); } };
  const handlePause = async () => { try { await sessionService.setTransport('A', 'paused'); } catch (e) { console.error(e); } };
  const handleStop = async () => { try { await sessionService.setTransport('A', 'stopped'); } catch (e) { console.error(e); } };
  const handleSeek = async (beats: number) => { try { await sessionService.seek('A', beats); } catch (e) { console.error(e); } };

  const handleSetLoop = async (bars: number | null) => {
    if (bars === null) {
      try { await sessionService.setLoop('A', null); } catch (e) { console.error(e); }
      return;
    }
    const meterNum = meters?.players?.[deckA.playerId]?.meterNumerator || 4;
    const beats = bars * meterNum;
    try { await sessionService.setLoop('A', beats); } catch (e) { console.error(e); }
  };

  const applyTempo = async (pct: number) => {
    try { await sessionService.setTempo('A', 1 + pct / 100); } catch (e) { console.error(e); }
  };

  const applyPitch = async (st: number) => {
    try { await sessionService.setPitch('A', st); } catch (e) { console.error(e); }
  };

  const toggleMatchLevel = async () => {
    if (!loudnessComp?.matchGain) return;
    if (matchLevelOn) {
      await sessionService.setLoudnessMatchGain('B', 1.0);
      setMatchLevelOn(false);
    } else {
      await sessionService.setLoudnessMatchGain('B', loudnessComp.matchGain);
      setMatchLevelOn(true);
    }
  };

  // ─── derived ─────────────────────────────────────────────────────────

  const displayed: KeyCandidate | null = overrideKey
    ? overrideKey
    : result
      ? {
          key_standard: result.key_standard,
          key_camelot: result.key_camelot,
          confidence: result.key_confidence,
          agreement: result.candidates?.[0]?.agreement ?? 0,
          avg_score: result.candidates?.[0]?.avg_score ?? 0,
          segment_count: result.candidates?.[0]?.segment_count ?? 0,
        }
      : null;

  const badge = displayed?.key_camelot ? formatCamelotBadge(displayed.key_camelot) : null;

  const focal: FocalTrack | null = result && displayed ? {
    id: result.track_id,
    key_camelot: displayed.key_camelot,
    bpm: result.bpm,
    title: result.title ?? null,
    artist: result.artist ?? null,
    filename: result.filename ?? filename ?? null,
    artwork_path: result.artwork_path ?? null,
    chroma: result.chroma ?? null,
  } : null;

  const samplePeakDbfs = linearToDbfs(meters?.masterSamplePeak);
  const rmsDbfs = linearToDbfs(meters?.masterRms);
  const truePeakDbtp = meters?.masterTruePeakDbtp ?? null;
  const sampleOver = meters !== null && meters.masterSamplePeak >= 1;
  const truePeakOver = truePeakDbtp !== null && Number.isFinite(truePeakDbtp) && truePeakDbtp > 0;

  const meterNum = meters?.players?.[deckA.playerId]?.meterNumerator || 4;

  // ─── render ──────────────────────────────────────────────────────────

  return (
    <div className="flex flex-col h-full overflow-auto p-6 gap-6 max-w-6xl mx-auto">
      {/* Drop zone — only when no result */}
      {!result && !isAnalyzing && (
        <div
          onDragOver={(e) => { e.preventDefault(); setIsDragging(true); }}
          onDragLeave={() => setIsDragging(false)}
          onDrop={(e) => { e.preventDefault(); setIsDragging(false); }}
          className={`
            flex-1 min-h-[280px] flex flex-col items-center justify-center gap-4
            border-2 border-dashed rounded-2xl transition-colors
            ${isDragging ? 'border-accent-primary bg-accent-primary/5' : 'border-white/10 bg-surface/30'}
          `}
        >
          <Upload className="w-12 h-12 text-accent-primary" />
          <div className="text-xl font-medium text-text-primary">Drop a track anywhere</div>
          <div className="text-sm text-text-secondary">
            Get key, BPM, intensity, and harmonic relationships — then play it back with live meters.
          </div>
          <button
            onClick={handleOpenFile}
            className="mt-2 flex items-center gap-2 px-5 py-2.5 bg-accent-primary text-white rounded-md text-sm font-semibold hover:opacity-90"
          >
            <FolderOpen className="w-4 h-4" />
            Open audio file
          </button>
          <div className="text-xs text-text-secondary">
            MP3 · WAV · FLAC · AIFF · M4A · OGG · common video formats
          </div>
          {error && <div className="text-sm text-red-400 mt-4 max-w-md text-center">{error}</div>}
        </div>
      )}

      {/* Analysis progress */}
      {isAnalyzing && (
        <AnalysisProgressDisplay progress={progress} filename={filename} />
      )}

      {/* ─── Analysis results ─── */}
      {result && displayed && focal && (
        <>
          {/* Title + reset */}
          <div className="flex items-start justify-between gap-4">
            <div className="min-w-0">
              <h2 className="text-xl font-semibold text-text-primary truncate">
                {result.title ?? result.filename ?? filename ?? 'Analyzed track'}
              </h2>
              <div className="text-sm text-text-secondary truncate">{result.artist ?? 'Unknown artist'}</div>
            </div>
            <button
              onClick={() => {
                setResult(null);
                setOverrideKey(null);
                setFilename(null);
                setError(null);
                setProgress(null);
                setHoveredCamelot(null);
              }}
              className="px-3 py-1.5 text-sm rounded-md border border-white/10 bg-surface/40 text-text-secondary hover:text-text-primary hover:bg-surface/60"
            >
              New track
            </button>
          </div>

          {/* Key/BPM readout + waveform */}
          <div className="grid grid-cols-1 lg:grid-cols-[minmax(280px,0.8fr)_minmax(0,1.7fr)] gap-6">
            <ReadoutCard
              displayed={displayed}
              candidates={result.candidates ?? []}
              sectionCount={result.section_count ?? 0}
              badge={badge}
              bpm={result.bpm}
              overrideActive={overrideKey !== null}
              onClearOverride={() => setOverrideKey(null)}
              copied={false}
              onCopy={() => {
                if (!displayed || !result) return;
                navigator.clipboard.writeText(
                  `${displayed.key_camelot} · ${Math.round(result.bpm)} BPM · ${displayed.key_standard}`
                );
              }}
              onReset={() => {
                setResult(null); setOverrideKey(null); setFilename(null);
                setError(null); setProgress(null); setHoveredCamelot(null);
              }}
            />

            <div className="bg-surface/40 rounded-2xl p-4 flex flex-col gap-4">
              <div>
                <div className="flex items-center gap-2 text-xs text-text-secondary mb-2">
                  <Disc3 className="w-3.5 h-3.5" /> Musical map
                </div>
                <WaveformDisplay data={waveform} height={112} />
              </div>
            </div>
          </div>

          {/* Harmonic relationships */}
          <HarmonicMosaic
            focal={focal}
            library={libraryTracks}
            onHoverCandidate={setHoveredCamelot}
          />
        </>
      )}

      {/* ─── Performance section ─── */}
      <div className="border-t border-white/10 pt-6">
          <h3 className="text-lg font-bold text-text-primary mb-1">Performance</h3>
          <p className="text-sm text-text-secondary mb-4">
            Play back through the TuneLock engine with live metering. No safety limiter yet (PB-6.3 pending).
          </p>

          <div className="mb-4 text-xs text-text-secondary flex flex-wrap gap-3">
            <span>Deck A: {deckA.source?.displayName ?? 'No file loaded'}</span>
            <span>Load: {deckA.loadStatus}</span>
            <span>Command: {deckABusy ? `pending (${deckAPendingKinds.join(', ')})` : 'acknowledged'}</span>
            <span>Engine generation: {engine.generation}</span>
          </div>
          {deckA.error && (
            <div className="mb-4 p-3 bg-red-900/30 border border-red-700/50 rounded-lg text-sm text-red-300">
              Deck A: {deckA.error}
            </div>
          )}
          {meters && (meters.commandsDropped > 0 || meters.acknowledgementsDropped > 0) && (
            <div className="mb-4 p-3 bg-amber-900/30 border border-amber-700/50 rounded-lg text-sm text-amber-300">
              Audio control pressure detected: {meters.commandsDropped} command(s) rejected,
              {' '}{meters.acknowledgementsDropped} acknowledgement(s) lost. Current controls were reconciled from engine telemetry.
            </div>
          )}

          {/* Engine status */}
          {engineState === 'error' && (
            <div className="mb-4 p-3 bg-red-900/30 border border-red-700/50 rounded-lg text-sm text-red-300">
              Audio engine failed: {engineError}
              <button onClick={() => void sessionService.initialize()} className="ml-3 px-3 py-1 bg-red-800/50 rounded text-xs">Retry</button>
            </div>
          )}
          {engineState === 'initializing' && (
            <p className="text-sm text-text-secondary mb-4">Initializing audio engine…</p>
          )}

          {/* Live meters */}
          {engineState === 'ready' && (
            <>
              <div className="mb-4 p-4 bg-plate-dark rounded-lg border border-plate-darker">
                <div className="flex items-center justify-between mb-3">
                  <h4 className="text-sm font-bold text-label-cream">Live Master Meters</h4>
                  <div className="text-xs text-label-dim flex gap-4">
                    <span>SR: {engine.sampleRate || '?'} Hz</span>
                    <span>Pos: {meters && Number.isFinite(positionSec) ? fmtTime(positionSec) : '—'}</span>
                    {isPlaying && <span className="text-cap-amber">● PLAYING</span>}
                  </div>
                </div>
                <div className="grid grid-cols-1 sm:grid-cols-3 gap-3">
                  <MeterBar label="Sample Peak" unit="dBFS" value={samplePeakDbfs} over={sampleOver} />
                  <MeterBar label="True Peak" unit="dBTP" value={truePeakDbtp} over={truePeakOver} />
                  <MeterBar label="RMS" unit="dBFS" value={rmsDbfs} />
                </div>
                <div className="flex flex-wrap gap-3 mt-3 text-xs">
                  <span className={`px-2 py-1 rounded ${sampleOver ? 'bg-red-900/40 text-red-300' : 'bg-plate-light text-label-dim'}`}>
                    Sample clip: {!meters ? '—' : sampleOver ? 'AT / OVER 0 dBFS' : 'none'}
                  </span>
                  <span className={`px-2 py-1 rounded ${truePeakOver ? 'bg-red-900/40 text-red-300' : 'bg-plate-light text-label-dim'}`}>
                    TP over: {truePeakDbtp === null ? '—' : truePeakOver ? 'OVER 0 dBTP' : 'none'}
                  </span>
                  {meters?.masterClip && <span className="px-2 py-1 text-amber-300">Engine clip flag: detected</span>}
                </div>
              </div>

              {/* Transport */}
              <div className="flex flex-wrap gap-2 mb-4">
                {!isPlaying ? (
                  <button disabled={deckA.loadStatus !== 'ready' || deckABusy} onClick={handlePlay} className="flex items-center gap-1.5 px-4 py-2 bg-cap-amber text-black rounded text-sm font-medium disabled:opacity-40 disabled:cursor-not-allowed">
                    <Play className="w-4 h-4" /> Play
                  </button>
                ) : (
                  <button disabled={deckA.loadStatus !== 'ready' || deckABusy} onClick={handlePause} className="flex items-center gap-1.5 px-4 py-2 bg-plate-lighter rounded text-sm disabled:opacity-40 disabled:cursor-not-allowed">
                    <Pause className="w-4 h-4" /> Pause
                  </button>
                )}
                <button disabled={deckA.loadStatus !== 'ready' || deckABusy} onClick={handleStop} className="flex items-center gap-1.5 px-4 py-2 bg-plate-light rounded text-sm disabled:opacity-40 disabled:cursor-not-allowed">
                  <Square className="w-4 h-4" /> Stop
                </button>
                <button disabled={deckA.loadStatus !== 'ready' || deckABusy} onClick={() => handleSeek(0)} className="flex items-center gap-1.5 px-4 py-2 bg-plate-light rounded text-sm disabled:opacity-40 disabled:cursor-not-allowed">
                  <SkipBack className="w-4 h-4" /> Start
                </button>
                <button disabled={deckA.loadStatus !== 'ready' || deckABusy} onClick={() => handleSeek(32)} className="flex items-center gap-1.5 px-4 py-2 bg-plate-light rounded text-sm disabled:opacity-40 disabled:cursor-not-allowed">
                  <FastForward className="w-4 h-4" /> +32 beats
                </button>
                <button disabled={deckA.loadStatus !== 'ready' || deckABusy} onClick={() => handleSeek(64)} className="flex items-center gap-1.5 px-4 py-2 bg-plate-light rounded text-sm disabled:opacity-40 disabled:cursor-not-allowed">
                  <FastForward className="w-4 h-4" /> +64 beats
                </button>
              </div>

              {/* Loop controls */}
              <div className="mb-4">
                <label className="text-xs text-text-secondary block mb-1">
                  LOOP ({meterNum}/4 time — bar = {meterNum} beats)
                </label>
                <div className="flex gap-2">
                  <button disabled={deckA.loadStatus !== 'ready' || deckABusy} onClick={() => handleSetLoop(null)} className={`px-3 py-1 text-sm rounded disabled:opacity-40 disabled:cursor-not-allowed ${loopBeats === null ? 'bg-cap-amber text-black' : 'bg-plate-light text-label-dim'}`}>Off</button>
                  <button disabled={deckA.loadStatus !== 'ready' || deckABusy} onClick={() => handleSetLoop(1)} className={`px-3 py-1 text-sm rounded disabled:opacity-40 disabled:cursor-not-allowed ${loopBeats === meterNum * 1 ? 'bg-cap-amber text-black' : 'bg-plate-light text-label-dim'}`}>1 bar</button>
                  <button disabled={deckA.loadStatus !== 'ready' || deckABusy} onClick={() => handleSetLoop(2)} className={`px-3 py-1 text-sm rounded disabled:opacity-40 disabled:cursor-not-allowed ${loopBeats === meterNum * 2 ? 'bg-cap-amber text-black' : 'bg-plate-light text-label-dim'}`}>2 bars</button>
                  <button disabled={deckA.loadStatus !== 'ready' || deckABusy} onClick={() => handleSetLoop(4)} className={`px-3 py-1 text-sm rounded disabled:opacity-40 disabled:cursor-not-allowed ${loopBeats === meterNum * 4 ? 'bg-cap-amber text-black' : 'bg-plate-light text-label-dim'}`}>4 bars</button>
                  <button disabled={deckA.loadStatus !== 'ready' || deckABusy} onClick={() => handleSetLoop(8)} className={`px-3 py-1 text-sm rounded disabled:opacity-40 disabled:cursor-not-allowed ${loopBeats === meterNum * 8 ? 'bg-cap-amber text-black' : 'bg-plate-light text-label-dim'}`}>8 bars</button>
                </div>
              </div>

              {/* Tempo / pitch */}
              <div className="grid grid-cols-1 sm:grid-cols-2 gap-4 mb-4">
                <div>
                  <label className="text-xs text-text-secondary block mb-1">TEMPO</label>
                  <div className="flex gap-2">
                    {TEMPO_PRESETS.map(p => (
                      <button key={p} disabled={deckA.loadStatus !== 'ready' || deckABusy} onClick={() => applyTempo(p)}
                        className={`px-3 py-1 text-sm rounded min-w-[3.5rem] disabled:opacity-40 disabled:cursor-not-allowed ${tempo === p ? 'bg-cap-amber text-black' : 'bg-plate-light text-label-dim'}`}>
                        {p > 0 ? `+${p}%` : `${p}%`}
                      </button>
                    ))}
                  </div>
                </div>
                <div>
                  <label className="text-xs text-text-secondary block mb-1">PITCH (semitones)</label>
                  <div className="flex gap-2">
                    {PITCH_PRESETS.map(p => (
                      <button key={p} disabled={deckA.loadStatus !== 'ready' || deckABusy} onClick={() => applyPitch(p)}
                        className={`px-3 py-1 text-sm rounded min-w-[3rem] disabled:opacity-40 disabled:cursor-not-allowed ${pitch === p ? 'bg-cap-amber text-black' : 'bg-plate-light text-label-dim'}`}>
                        {p > 0 ? `+${p}` : `${p}`}
                      </button>
                    ))}
                  </div>
                </div>
              </div>

              {/* Musical telemetry */}
              {meters?.players?.[deckA.playerId] && (
                <div className="mb-4 text-xs text-text-secondary flex flex-wrap gap-4">
                  <span>Source BPM: <span className="text-text-primary">{(meters.players[deckA.playerId].sourceBpm ?? 0) > 0 ? meters.players[deckA.playerId].sourceBpm.toFixed(2) : '—'}</span></span>
                  <span>Effective BPM: <span className="text-text-primary">{(meters.players[deckA.playerId].effectiveBpm ?? 0) > 0 ? meters.players[deckA.playerId].effectiveBpm.toFixed(2) : '—'}</span></span>
                  <span>Tempo: <span className="text-text-primary">{(((meters.players[deckA.playerId].tempoRatio ?? 1) - 1) * 100).toFixed(2)}%</span></span>
                  <span>Pitch: <span className="text-text-primary">{(meters.players[deckA.playerId].pitchSemitones ?? 0) > 0 ? '+' : ''}{(meters.players[deckA.playerId].pitchSemitones ?? 0).toFixed(1)} st</span></span>
                  <span>Beat: <span className="text-text-primary">{(meters.players[deckA.playerId].beatPosition ?? 0).toFixed(1)}</span></span>
                  <span>Bar: <span className="text-text-primary">{(meters.players[deckA.playerId].barPosition ?? 0).toFixed(1)}</span></span>
                </div>
              )}

              {/* ─── Two-deck ─── */}
              <div className="mt-6 p-4 bg-plate-dark rounded-lg border border-plate-darker">
                <h4 className="text-sm font-bold mb-3">Deck B — Load a second track for matching</h4>
                <div className="flex items-center gap-3 mb-3">
                  <button disabled={deckB.loadStatus === 'loading' || deckBBusy} onClick={handleLoadDeckB} className="px-3 py-1.5 bg-plate-light rounded text-sm disabled:opacity-40 disabled:cursor-not-allowed">
                    Load Deck B
                  </button>
                  <span className="text-sm text-label-dim truncate">{trackNameB || 'No file loaded'}</span>
                </div>

                {deckB.error && (
                  <div className="mb-3 p-2 bg-red-900/30 border border-red-700/50 rounded text-xs text-red-300">
                    Deck B: {deckB.error}
                  </div>
                )}
                {deckBBusy && (
                  <div className="mb-3 text-xs text-cap-amber">
                    Deck B command pending: {deckBPendingKinds.join(', ')}
                  </div>
                )}

                {trackNameB && (
                  <>
                    <div className="flex gap-2 flex-wrap mb-3">
                      <button disabled={deckB.loadStatus !== 'ready' || deckBBusy} onClick={() => void sessionService.setTransport('B', 'playing').catch(console.error)} className="flex items-center gap-1 px-3 py-1.5 bg-plate-light rounded text-sm disabled:opacity-40 disabled:cursor-not-allowed">
                        <Play className="w-3.5 h-3.5" /> Play B
                      </button>
                      <button disabled={deckB.loadStatus !== 'ready' || deckBBusy} onClick={() => void sessionService.setTransport('B', 'paused').catch(console.error)} className="flex items-center gap-1 px-3 py-1.5 bg-plate-light rounded text-sm disabled:opacity-40 disabled:cursor-not-allowed">
                        <Pause className="w-3.5 h-3.5" /> Pause B
                      </button>
                      <button disabled={deckA.loadStatus !== 'ready' || deckB.loadStatus !== 'ready'} onClick={() => void sessionService.syncLaunch('A', 'B').catch(console.error)} className="px-3 py-1.5 bg-plate-lighter rounded text-sm disabled:opacity-40 disabled:cursor-not-allowed" title="Start both at the same engine frame">
                        Same-Frame Start
                      </button>
                      <button
                        onClick={() => void sessionService.beatSync('A', 'B').catch(console.error)}
                        disabled={(meters?.players?.[deckA.playerId]?.sourceBpm ?? 0) <= 0 || (meters?.players?.[deckB.playerId]?.sourceBpm ?? 0) <= 0}
                        className="px-4 py-1.5 bg-cap-amber text-black rounded text-sm font-medium disabled:opacity-40 disabled:cursor-not-allowed"
                        title="Tempo-match B to A and align nearest beats"
                      >
                        Beat Sync
                      </button>
                      <button
                        onClick={() => void sessionService.barSync('A', 'B').catch(console.error)}
                        disabled={(meters?.players?.[deckA.playerId]?.sourceBpm ?? 0) <= 0 || (meters?.players?.[deckB.playerId]?.sourceBpm ?? 0) <= 0}
                        className="px-4 py-1.5 bg-cap-amber text-black rounded text-sm font-medium disabled:opacity-40 disabled:cursor-not-allowed"
                        title="Tempo-match B→A + align downbeat/bar boundaries"
                      >
                        Bar Sync
                      </button>
                    </div>

                    {/* Match Level */}
                    {loudnessComp && (
                      <div className="border-t border-plate-darker pt-3">
                        <div className="text-xs font-bold text-label-cream mb-2">Match Level (B → A)</div>
                        <div className="grid grid-cols-3 gap-4 mb-3 text-sm">
                          <div>
                            <div className="text-label-dim text-xs mb-1">Deck A</div>
                            <div>LUFS: {loudnessComp.lufsA !== null ? loudnessComp.lufsA.toFixed(1) : '—'}</div>
                            <div className="text-xs text-label-dim">TP: {loudnessComp.truePeakA !== null ? `${loudnessComp.truePeakA.toFixed(2)} dBTP` : '—'}</div>
                          </div>
                          <div>
                            <div className="text-label-dim text-xs mb-1">Deck B</div>
                            <div>LUFS: {loudnessComp.lufsB !== null ? loudnessComp.lufsB.toFixed(1) : '—'}</div>
                            <div className="text-xs text-label-dim">TP: {loudnessComp.truePeakB !== null ? `${loudnessComp.truePeakB.toFixed(2)} dBTP` : '—'}</div>
                          </div>
                          <div>
                            <div className="text-label-dim text-xs mb-1">Match</div>
                            <div>Δ = {loudnessComp.deltaLu !== null ? `${loudnessComp.deltaLu.toFixed(1)} LU` : '—'}</div>
                            <div className="text-xs text-label-dim">
                              {loudnessComp.matchGainDb !== null ? `${loudnessComp.matchGainDb >= 0 ? '+' : ''}${loudnessComp.matchGainDb.toFixed(1)} dB` : '—'}
                            </div>
                          </div>
                        </div>
                        {loudnessComp.headroomStatus === 'warning' && (
                          <div className="mb-3 p-2 bg-amber-900/40 border border-amber-600/50 rounded text-xs text-amber-300">
                            Insufficient headroom — predicted true peak of B after match exceeds 0 dBTP.
                          </div>
                        )}
                        {loudnessComp.headroomStatus === 'excessive' && (
                          <div className="mb-3 p-2 bg-red-900/40 border border-red-600/50 rounded text-xs text-red-300">
                            Excessive match gain ({loudnessComp.matchGainDb?.toFixed(1)} dB). Check LUFS values.
                          </div>
                        )}
                        <button
                          onClick={toggleMatchLevel}
                          disabled={deckBBusy || loudnessComp.matchGain === null || loudnessComp.headroomStatus === 'excessive'}
                          className={`px-4 py-2 rounded text-sm font-medium disabled:opacity-40 disabled:cursor-not-allowed ${
                            matchLevelOn ? 'bg-cap-amber text-black' : 'bg-plate-lighter text-label-cream'
                          }`}
                        >
                          {matchLevelOn ? 'Match Level: ON' : 'Match B → A'}
                        </button>
                      </div>
                    )}
                  </>
                )}
              </div>
            </>
          )}
        </div>

      {/* ─── Library drawer ─── */}
      {libraryOpen && (
        <div className="fixed inset-0 z-50 flex">
          <div className="flex-1 bg-black/50" onClick={() => setLibraryOpen(false)} />
          <div className="w-[600px] max-w-[80vw] bg-surface border-l border-white/10 flex flex-col">
            <div className="flex items-center justify-between p-4 border-b border-white/10">
              <h3 className="text-sm font-bold text-text-primary">Library</h3>
              <button onClick={() => setLibraryOpen(false)} className="p-1 text-text-secondary hover:text-text-primary">
                <X className="w-5 h-5" />
              </button>
            </div>
            <div className="flex-1 overflow-hidden">
              <LibraryTable />
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
