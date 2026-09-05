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
  audioEngineInit,
  audioEngineLoadPlayerPaused,
  audioEnginePlay,
  audioEnginePause,
  audioEngineStop,
  audioEngineSeek,
  audioEngineSetTempo,
  audioEngineSetPitch,
  audioEngineSetLoop,
  audioEngineSetBus,
  audioEngineSetMasterGain,
  audioEngineSyncLaunch,
  audioEngineBeatSync,
  audioEngineBarSync,
  audioEngineGetMeters,
  listeningLabGetProcessorInfo,
  getLoudnessComparison,
  audioEngineSetLoudnessMatchGain,
  type AudioMeterReadout,
  type ListeningLabProcessorInfo,
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

type EngineState = 'idle' | 'initializing' | 'ready' | 'error';

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

  // Engine state
  const [engineState, setEngineState] = useState<EngineState>('idle');
  const [engineError, setEngineError] = useState('');
  const [processorInfo, setProcessorInfo] = useState<ListeningLabProcessorInfo | null>(null);
  const [meters, setMeters] = useState<AudioMeterReadout | null>(null);
  const [isPlaying, setIsPlaying] = useState(false);
  const [tempo, setTempo] = useState(0);
  const [pitch, setPitch] = useState(0);
  const [loopBeats, setLoopBeats] = useState<number | null>(null);
  const [positionSec, setPositionSec] = useState(0);

  // Two-deck state
  const [filePathA, setFilePathA] = useState<string>('');
  const [, setTrackNameA] = useState<string>('');
  const [filePathB, setFilePathB] = useState<string>('');
  const [trackNameB, setTrackNameB] = useState<string>('');
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
      // Auto-load into Deck A of the performance engine
      if (engineState === 'ready' && analysis.file_path) {
        try {
          await audioEngineLoadPlayerPaused(0, analysis.file_path);
          setFilePathA(analysis.file_path);
          setTrackNameA(displayName);
        } catch (e) {
          console.warn('[workspace] auto-load into engine failed:', e);
        }
      }
    } catch (e) {
      setError(typeof e === 'string' ? e : 'Analysis failed.');
      setProgress(null);
    } finally {
      setIsAnalyzing(false);
    }
  }, [engineState]);

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
              handleAnalyzePath(fullPath, name);
            }
          }
        });
        if (cancelled && unlisten) { unlisten(); unlisten = null; }
      } catch (err) {
        console.warn('Tauri drag-drop unavailable:', err);
      }
    })();
    return () => { cancelled = true; if (unlisten) unlisten(); };
  }, [handleAnalyzePath]);

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
        await handleAnalyzePath(selected, name);
      }
    } catch (err) {
      setError(typeof err === 'string' ? err : 'Could not open the file picker.');
    }
  }, [handleAnalyzePath]);

  // ─── engine ──────────────────────────────────────────────────────────

  const initEngine = useCallback(async () => {
    setEngineState('initializing');
    setEngineError('');
    try {
      await audioEngineInit();
      const info = await listeningLabGetProcessorInfo();
      setProcessorInfo(info);
      await audioEngineSetMasterGain(1.0);
      await audioEngineSetBus(0, 'master');
      await audioEngineSetBus(1, 'master');
      setEngineState('ready');
    } catch (e) {
      console.error('Engine init failed:', e);
      setEngineError(String(e));
      setEngineState('error');
    }
  }, []);

  // Auto-init engine on mount
  useEffect(() => {
    initEngine();
  }, [initEngine]);

  // Poll meters at 20 Hz
  useEffect(() => {
    if (engineState !== 'ready') return;
    let active = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const poll = async () => {
      if (!active) return;
      try {
        const m = await audioEngineGetMeters();
        if (!active) return;
        setMeters(m);
        if (Number.isFinite(m?.players?.[0]?.positionSec)) {
          setPositionSec(m.players[0].positionSec);
        }
      } catch (e) {
        if (!active) return;
        setMeters(null);
      }
      if (active) timer = setTimeout(poll, 50);
    };
    timer = setTimeout(poll, 50);
    return () => { active = false; clearTimeout(timer); };
  }, [engineState]);

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
      await audioEngineLoadPlayerPaused(1, selected);
      setFilePathB(selected);
      setTrackNameB(selected.split(/[\\/]/).pop() || selected);
    } catch (e) {
      console.error('Load Deck B failed:', e);
    }
  };

  const handlePlay = async () => { try { await audioEnginePlay(0); setIsPlaying(true); } catch (e) { console.error(e); } };
  const handlePause = async () => { try { await audioEnginePause(0); setIsPlaying(false); } catch (e) { console.error(e); } };
  const handleStop = async () => { try { await audioEngineStop(0); setIsPlaying(false); } catch (e) { console.error(e); } };
  const handleSeek = async (beats: number) => { try { await audioEngineSeek(0, beats); } catch (e) { console.error(e); } };

  const handleSetLoop = async (bars: number | null) => {
    if (bars === null) {
      setLoopBeats(null);
      try { await audioEngineSetLoop(0, null, null); } catch (e) { console.error(e); }
      return;
    }
    const meterNum = meters?.players?.[0]?.meterNumerator || 4;
    const beats = bars * meterNum;
    setLoopBeats(beats);
    try { await audioEngineSetLoop(0, 0, beats); } catch (e) { console.error(e); }
  };

  const applyTempo = async (pct: number) => {
    setTempo(pct);
    try { await audioEngineSetTempo(0, 1 + pct / 100); } catch (e) { console.error(e); }
  };

  const applyPitch = async (st: number) => {
    setPitch(st);
    try { await audioEngineSetPitch(0, st); } catch (e) { console.error(e); }
  };

  const toggleMatchLevel = async () => {
    if (!loudnessComp?.matchGain) return;
    if (matchLevelOn) {
      await audioEngineSetLoudnessMatchGain(1, 1.0);
      setMatchLevelOn(false);
    } else {
      await audioEngineSetLoudnessMatchGain(1, loudnessComp.matchGain);
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

  const meterNum = meters?.players?.[0]?.meterNumerator || 4;

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
      {result && (
        <div className="border-t border-white/10 pt-6">
          <h3 className="text-lg font-bold text-text-primary mb-1">Performance</h3>
          <p className="text-sm text-text-secondary mb-4">
            Play back through the TuneLock engine with live metering. No safety limiter yet (PB-6.3 pending).
          </p>

          {/* Engine status */}
          {engineState === 'error' && (
            <div className="mb-4 p-3 bg-red-900/30 border border-red-700/50 rounded-lg text-sm text-red-300">
              Audio engine failed: {engineError}
              <button onClick={initEngine} className="ml-3 px-3 py-1 bg-red-800/50 rounded text-xs">Retry</button>
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
                    <span>SR: {processorInfo?.sampleRate || '?'} Hz</span>
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
                  <button onClick={handlePlay} className="flex items-center gap-1.5 px-4 py-2 bg-cap-amber text-black rounded text-sm font-medium">
                    <Play className="w-4 h-4" /> Play
                  </button>
                ) : (
                  <button onClick={handlePause} className="flex items-center gap-1.5 px-4 py-2 bg-plate-lighter rounded text-sm">
                    <Pause className="w-4 h-4" /> Pause
                  </button>
                )}
                <button onClick={handleStop} className="flex items-center gap-1.5 px-4 py-2 bg-plate-light rounded text-sm">
                  <Square className="w-4 h-4" /> Stop
                </button>
                <button onClick={() => handleSeek(0)} className="flex items-center gap-1.5 px-4 py-2 bg-plate-light rounded text-sm">
                  <SkipBack className="w-4 h-4" /> Start
                </button>
                <button onClick={() => handleSeek(32)} className="flex items-center gap-1.5 px-4 py-2 bg-plate-light rounded text-sm">
                  <FastForward className="w-4 h-4" /> +32 beats
                </button>
                <button onClick={() => handleSeek(64)} className="flex items-center gap-1.5 px-4 py-2 bg-plate-light rounded text-sm">
                  <FastForward className="w-4 h-4" /> +64 beats
                </button>
              </div>

              {/* Loop controls */}
              <div className="mb-4">
                <label className="text-xs text-text-secondary block mb-1">
                  LOOP ({meterNum}/4 time — bar = {meterNum} beats)
                </label>
                <div className="flex gap-2">
                  <button onClick={() => handleSetLoop(null)} className={`px-3 py-1 text-sm rounded ${loopBeats === null ? 'bg-cap-amber text-black' : 'bg-plate-light text-label-dim'}`}>Off</button>
                  <button onClick={() => handleSetLoop(1)} className={`px-3 py-1 text-sm rounded ${loopBeats === meterNum * 1 ? 'bg-cap-amber text-black' : 'bg-plate-light text-label-dim'}`}>1 bar</button>
                  <button onClick={() => handleSetLoop(2)} className={`px-3 py-1 text-sm rounded ${loopBeats === meterNum * 2 ? 'bg-cap-amber text-black' : 'bg-plate-light text-label-dim'}`}>2 bars</button>
                  <button onClick={() => handleSetLoop(4)} className={`px-3 py-1 text-sm rounded ${loopBeats === meterNum * 4 ? 'bg-cap-amber text-black' : 'bg-plate-light text-label-dim'}`}>4 bars</button>
                  <button onClick={() => handleSetLoop(8)} className={`px-3 py-1 text-sm rounded ${loopBeats === meterNum * 8 ? 'bg-cap-amber text-black' : 'bg-plate-light text-label-dim'}`}>8 bars</button>
                </div>
              </div>

              {/* Tempo / pitch */}
              <div className="grid grid-cols-1 sm:grid-cols-2 gap-4 mb-4">
                <div>
                  <label className="text-xs text-text-secondary block mb-1">TEMPO</label>
                  <div className="flex gap-2">
                    {TEMPO_PRESETS.map(p => (
                      <button key={p} onClick={() => applyTempo(p)}
                        className={`px-3 py-1 text-sm rounded min-w-[3.5rem] ${tempo === p ? 'bg-cap-amber text-black' : 'bg-plate-light text-label-dim'}`}>
                        {p > 0 ? `+${p}%` : `${p}%`}
                      </button>
                    ))}
                  </div>
                </div>
                <div>
                  <label className="text-xs text-text-secondary block mb-1">PITCH (semitones)</label>
                  <div className="flex gap-2">
                    {PITCH_PRESETS.map(p => (
                      <button key={p} onClick={() => applyPitch(p)}
                        className={`px-3 py-1 text-sm rounded min-w-[3rem] ${pitch === p ? 'bg-cap-amber text-black' : 'bg-plate-light text-label-dim'}`}>
                        {p > 0 ? `+${p}` : `${p}`}
                      </button>
                    ))}
                  </div>
                </div>
              </div>

              {/* Musical telemetry */}
              {meters?.players?.[0] && (
                <div className="mb-4 text-xs text-text-secondary flex flex-wrap gap-4">
                  <span>Source BPM: <span className="text-text-primary">{(meters.players[0].sourceBpm ?? 0) > 0 ? meters.players[0].sourceBpm.toFixed(2) : '—'}</span></span>
                  <span>Effective BPM: <span className="text-text-primary">{(meters.players[0].effectiveBpm ?? 0) > 0 ? meters.players[0].effectiveBpm.toFixed(2) : '—'}</span></span>
                  <span>Tempo: <span className="text-text-primary">{(((meters.players[0].tempoRatio ?? 1) - 1) * 100).toFixed(2)}%</span></span>
                  <span>Pitch: <span className="text-text-primary">{(meters.players[0].pitchSemitones ?? 0) > 0 ? '+' : ''}{(meters.players[0].pitchSemitones ?? 0).toFixed(1)} st</span></span>
                  <span>Beat: <span className="text-text-primary">{(meters.players[0].beatPosition ?? 0).toFixed(1)}</span></span>
                  <span>Bar: <span className="text-text-primary">{(meters.players[0].barPosition ?? 0).toFixed(1)}</span></span>
                </div>
              )}

              {/* ─── Two-deck ─── */}
              <div className="mt-6 p-4 bg-plate-dark rounded-lg border border-plate-darker">
                <h4 className="text-sm font-bold mb-3">Deck B — Load a second track for matching</h4>
                <div className="flex items-center gap-3 mb-3">
                  <button onClick={handleLoadDeckB} className="px-3 py-1.5 bg-plate-light rounded text-sm">
                    Load Deck B
                  </button>
                  <span className="text-sm text-label-dim truncate">{trackNameB || 'No file loaded'}</span>
                </div>

                {trackNameB && (
                  <>
                    <div className="flex gap-2 flex-wrap mb-3">
                      <button onClick={() => audioEnginePlay(1)} className="flex items-center gap-1 px-3 py-1.5 bg-plate-light rounded text-sm">
                        <Play className="w-3.5 h-3.5" /> Play B
                      </button>
                      <button onClick={() => audioEnginePause(1)} className="flex items-center gap-1 px-3 py-1.5 bg-plate-light rounded text-sm">
                        <Pause className="w-3.5 h-3.5" /> Pause B
                      </button>
                      <button onClick={() => audioEngineSyncLaunch(0, 1)} className="px-3 py-1.5 bg-plate-lighter rounded text-sm" title="Start both at the same engine frame">
                        Same-Frame Start
                      </button>
                      <button
                        onClick={() => audioEngineBeatSync(0, 1)}
                        disabled={(meters?.players?.[0]?.sourceBpm ?? 0) <= 0 || (meters?.players?.[1]?.sourceBpm ?? 0) <= 0}
                        className="px-4 py-1.5 bg-cap-amber text-black rounded text-sm font-medium disabled:opacity-40 disabled:cursor-not-allowed"
                        title="Tempo-match B to A and align nearest beats"
                      >
                        Beat Sync
                      </button>
                      <button
                        onClick={() => audioEngineBarSync(0, 1)}
                        disabled={(meters?.players?.[0]?.sourceBpm ?? 0) <= 0 || (meters?.players?.[1]?.sourceBpm ?? 0) <= 0}
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
                          disabled={loudnessComp.matchGain === null || loudnessComp.headroomStatus === 'excessive'}
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
      )}

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
