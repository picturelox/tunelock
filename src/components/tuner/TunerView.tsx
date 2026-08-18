import { useState, useCallback, useEffect, useRef, DragEvent } from 'react';
import {
  Upload,
  Mic,
  Radio,
  Copy,
  Check,
  Loader2,
  Play,
  Pause,
  Volume2,
} from 'lucide-react';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { convertFileSrc } from '@tauri-apps/api/core';
import { analyzeFile, onTunerProgress, getLibraryPage, onTrackAnalyzed } from '../../lib/tauri';
import type { TrackAnalysis, TunerProgress, KeyCandidate, Track } from '../../types';
import {
  formatCamelotBadge,
  getScaleNotes,
  pitchClassFrequencies,
  PITCH_NAMES_SHARP,
  camelotToStandardKey,
  getKeyAmbiguityRelationship,
  type ScaleNote,
} from '../../lib/camelot';
import { playNote } from '../../lib/audio';
import CamelotWheel from '../camelot/CamelotWheel';
import PianoRoll from '../piano/PianoRoll';
import Metronome from '../metronome/Metronome';
import HarmonicMosaic, { type FocalTrack } from '../mosaic/HarmonicMosaic';

type TunerInput = 'file' | 'mic' | 'line';

const STAGE_LABELS: Record<string, string> = {
  decode: 'Decoding audio',
  spectrogram: 'Computing spectrogram',
  hpss: 'Separating harmonic content (HPSS)',
  chromagram: 'Building chromagram',
  ensemble: 'Ensemble key voting',
  tempo: 'Detecting tempo',
  done: 'Done',
};

export default function TunerView() {
  const [activeInput, setActiveInput] = useState<TunerInput>('file');
  const [isDragging, setIsDragging] = useState(false);
  const [isAnalyzing, setIsAnalyzing] = useState(false);
  const [result, setResult] = useState<TrackAnalysis | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [filename, setFilename] = useState<string | null>(null);
  const [progress, setProgress] = useState<TunerProgress | null>(null);
  const [overrideKey, setOverrideKey] = useState<KeyCandidate | null>(null);
  /**
   * The wheel-hovered key (after the dwell delay). Drives the piano roll and
   * the "Notes in this key" panel. Falls back to the displayed key when null.
   */
  const [hoveredCamelot, setHoveredCamelot] = useState<string | null>(null);
  /**
   * Cached snapshot of the user's analyzed library, used as the candidate
   * pool for the Harmonic Mosaic. We keep this in TunerView so it survives
   * across analyses and updates live when other analyses finish.
   */
  const [libraryTracks, setLibraryTracks] = useState<Track[]>([]);

  useEffect(() => {
    let unlisten: (() => void) | null = null;
    (async () => {
      unlisten = await onTunerProgress((p) => setProgress(p));
    })();
    return () => {
      if (unlisten) unlisten();
    };
  }, []);

  // Load the user's library (analyzed tracks) so the mosaic has neighbors
  // to show. We pull a generous page; sub-second on SQLite for thousands.
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const page = await getLibraryPage(0, 500, 'key_camelot', 'asc');
        if (!cancelled) setLibraryTracks(page.tracks ?? []);
      } catch (e) {
        console.warn('[tuner] failed to load library for mosaic:', e);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  // Live-update the library snapshot when other analyses complete elsewhere
  // in the app, so the mosaic immediately picks up new neighbors.
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
    return () => {
      if (unlisten) unlisten();
    };
  }, []);

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

  // Tauri 2 webview drag-drop event.
  useEffect(() => {
    let unlisten: (() => void) | null = null;
    (async () => {
      try {
        const webview = getCurrentWebview();
        unlisten = await webview.onDragDropEvent((event) => {
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
      } catch (err) {
        console.warn('Tauri drag-drop unavailable (running outside Tauri?):', err);
      }
    })();
    return () => {
      if (unlisten) unlisten();
    };
  }, [handleAnalyzePath]);

  const handleHtmlDrop = useCallback((e: DragEvent<HTMLDivElement>) => {
    e.preventDefault();
    setIsDragging(false);
  }, []);

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

  // The "active" key in the explore panel: hovered (if any) else the displayed key.
  const activeCamelot = hoveredCamelot ?? displayed?.key_camelot ?? null;
  const activeScale: ScaleNote[] = activeCamelot ? getScaleNotes(activeCamelot) : [];
  const activeStandard = activeCamelot ? camelotToStandardKey(activeCamelot) : null;

  const handleCopy = () => {
    if (!displayed || !result) return;
    const text = `${displayed.key_camelot} \u00b7 ${Math.round(result.bpm)} BPM \u00b7 ${displayed.key_standard}`;
    navigator.clipboard.writeText(text);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  };

  const badge = displayed?.key_camelot ? formatCamelotBadge(displayed.key_camelot) : null;

  return (
    <div className="flex flex-col h-full p-8 gap-6 overflow-auto">
      {/* Input mode selector */}
      <div className="flex gap-2 flex-wrap">
        <InputTab
          icon={<Upload className="w-4 h-4" />}
          label="File"
          active={activeInput === 'file'}
          onClick={() => setActiveInput('file')}
        />
        <InputTab
          icon={<Mic className="w-4 h-4" />}
          label="Microphone"
          active={activeInput === 'mic'}
          onClick={() => setActiveInput('mic')}
          disabled
        />
        <InputTab
          icon={<Radio className="w-4 h-4" />}
          label="Line-in"
          active={activeInput === 'line'}
          onClick={() => setActiveInput('line')}
          disabled
        />
        <div className="flex items-center text-xs text-text-secondary ml-auto">
          Mic / line-in coming next pass
        </div>
      </div>

      {/* Drop zone — visible until a result arrives */}
      {activeInput === 'file' && !result && (
        <div
          onDragOver={(e) => {
            e.preventDefault();
            setIsDragging(true);
          }}
          onDragLeave={() => setIsDragging(false)}
          onDrop={handleHtmlDrop}
          className={`
            flex-1 min-h-[280px] flex flex-col items-center justify-center gap-4
            border-2 border-dashed rounded-2xl transition-colors
            ${isDragging ? 'border-accent-primary bg-accent-primary/5' : 'border-white/10 bg-surface/30'}
          `}
        >
          {isAnalyzing ? (
            <AnalysisProgressDisplay progress={progress} filename={filename} />
          ) : (
            <>
              <Upload className="w-12 h-12 text-text-secondary" />
              <div className="text-lg">Drop an audio file</div>
              <div className="text-sm text-text-secondary">
                .mp3 · .wav · .flac · .aiff · .m4a · .ogg
              </div>
              {error && (
                <div className="text-sm text-red-400 mt-4 max-w-md text-center">{error}</div>
              )}
            </>
          )}
        </div>
      )}

      {/* Result */}
      {result && displayed && (
        <ResultPanel
          result={result}
          displayed={displayed}
          badge={badge}
          filename={filename}
          copied={copied}
          onCopy={handleCopy}
          onReset={() => {
            setResult(null);
            setOverrideKey(null);
            setFilename(null);
            setError(null);
            setProgress(null);
            setHoveredCamelot(null);
          }}
          onOverride={setOverrideKey}
          overrideActive={overrideKey !== null}
          hoveredCamelot={hoveredCamelot}
          onHoverCamelot={setHoveredCamelot}
          activeCamelot={activeCamelot}
          activeStandard={activeStandard}
          activeScale={activeScale}
          libraryTracks={libraryTracks}
        />
      )}

      {(activeInput === 'mic' || activeInput === 'line') && (
        <div className="flex-1 flex items-center justify-center text-text-secondary text-center max-w-md mx-auto">
          Live audio input is wired in the next pass. For now, drop a file to get
          an instant key + BPM readout using the same engine.
        </div>
      )}
    </div>
  );
}

// ============================================================================
// Result panel
// ============================================================================

interface ResultPanelProps {
  result: TrackAnalysis;
  displayed: KeyCandidate;
  badge: { text: string; color: string } | null;
  filename: string | null;
  copied: boolean;
  onCopy: () => void;
  onReset: () => void;
  onOverride: (c: KeyCandidate | null) => void;
  overrideActive: boolean;
  hoveredCamelot: string | null;
  onHoverCamelot: (camelot: string | null) => void;
  activeCamelot: string | null;
  activeStandard: string | null;
  activeScale: ScaleNote[];
  libraryTracks: Track[];
}

function ResultPanel({
  result,
  displayed,
  badge,
  filename,
  copied,
  onCopy,
  onReset,
  onOverride,
  overrideActive,
  hoveredCamelot,
  onHoverCamelot,
  activeCamelot,
  activeStandard,
  activeScale,
  libraryTracks,
}: ResultPanelProps) {
  // Build a `FocalTrack` for the mosaic from the current Tuner result.
  // We use the user-selected (possibly overridden) Camelot, so the mosaic
  // follows the same key the user is currently exploring.
  const focal: FocalTrack = {
    id: result.track_id,
    key_camelot: displayed.key_camelot,
    bpm: result.bpm,
    title: result.title ?? null,
    artist: result.artist ?? null,
    filename: result.filename ?? filename ?? null,
    artwork_path: result.artwork_path ?? null,
    chroma: result.chroma ?? null,
  };
  return (
    <div className="flex flex-col gap-6">
      {filename && <div className="text-sm text-text-secondary truncate">{filename}</div>}

      {/* Player */}
      {result.file_path && <AudioPlayer filePath={result.file_path} />}

      {/* Top row: readout + wheel */}
      <div className="grid grid-cols-1 lg:grid-cols-3 gap-6">
        <ReadoutCard
          displayed={displayed}
          candidates={result.candidates ?? []}
          badge={badge}
          bpm={result.bpm}
          overrideActive={overrideActive}
          onClearOverride={() => onOverride(null)}
          copied={copied}
          onCopy={onCopy}
          onReset={onReset}
        />

        <div className="lg:col-span-2 bg-surface/40 rounded-2xl p-2">
          <CamelotWheel
            selectedCamelot={displayed.key_camelot}
            onHover={onHoverCamelot}
            onSelect={(camelot) => {
              if (!camelot) return;
              const cand = result.candidates?.find((c) => c.key_camelot === camelot);
              if (cand) onOverride(cand);
            }}
            showLegend={false}
            showTrackCounts={false}
            showCenterStats={false}
            title=""
          />
        </div>
      </div>

      {/* Harmonic Mosaic: album-art relationship view. Bridges the wheel
          to actual library tracks the user can mix into. */}
      <HarmonicMosaic
        focal={focal}
        library={libraryTracks}
        onHoverCandidate={onHoverCamelot}
      />

      {/* Explore: scale notes + piano + metronome */}
      <ExplorePanel
        activeCamelot={activeCamelot}
        activeStandard={activeStandard}
        activeScale={activeScale}
        isHovered={hoveredCamelot !== null && hoveredCamelot !== displayed.key_camelot}
        bpm={result.bpm}
      />

      {/* Candidates + chroma side-by-side */}
      <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
        {result.candidates && result.candidates.length > 0 && (
          <CandidatesPanel
            candidates={result.candidates}
            selected={displayed}
            winnerStandard={displayed.key_standard}
            onSelect={onOverride}
          />
        )}
        {result.chroma && <ChromaPanel chroma={result.chroma} winnerCamelot={displayed.key_camelot} />}
      </div>

      {/* Timings full-width */}
      {result.timings && <TimingsPanel timings={result.timings} />}
    </div>
  );
}

// ============================================================================
// Sub-components
// ============================================================================

function ReadoutCard({
  displayed,
  candidates,
  badge,
  bpm,
  overrideActive,
  onClearOverride,
  copied,
  onCopy,
  onReset,
}: {
  displayed: KeyCandidate;
  candidates: KeyCandidate[];
  badge: { text: string; color: string } | null;
  bpm: number;
  overrideActive: boolean;
  onClearOverride: () => void;
  copied: boolean;
  onCopy: () => void;
  onReset: () => void;
}) {
  // Show up to 2 runner-ups that are "close" (conf >= 0.35) and not the winner.
  const runnerUps = candidates
    .filter(
      (c) =>
        c.key_camelot !== displayed.key_camelot &&
        c.confidence >= 0.35
    )
    .slice(0, 2);

  return (
    <div className="flex flex-col items-center gap-4 bg-surface/40 rounded-2xl p-6">
      {badge && (
        <div
          className="text-6xl font-bold px-10 py-5 rounded-3xl text-white shadow-2xl"
          style={{ backgroundColor: badge.color }}
        >
          {badge.text}
        </div>
      )}
      <div className="text-2xl font-light text-text-primary">{displayed.key_standard}</div>
      <div className="text-xl font-mono text-text-secondary">{bpm.toFixed(1)} BPM</div>

      <div className="w-full flex flex-col gap-1">
        <div className="flex justify-between text-xs text-text-secondary">
          <span>Confidence</span>
          <span className="font-mono">{Math.round(displayed.confidence * 100)}%</span>
        </div>
        <div className="h-2 bg-surface-light rounded-full overflow-hidden">
          <div
            className="h-full bg-accent-primary transition-all duration-500"
            style={{ width: `${displayed.confidence * 100}%` }}
          />
        </div>
        <div className="text-[10px] text-text-secondary mt-1">
          {displayed.segment_count}/8 segments agreed · profile-match{' '}
          {Math.round(displayed.avg_score * 100)}%
        </div>
      </div>

      {/* Secondary / ambiguous runner-up hint */}
      {runnerUps.length > 0 && (
        <div className="w-full flex flex-col gap-2 mt-1">
          <div className="text-[10px] uppercase tracking-wide text-text-secondary">
            Could also be
          </div>
          <div className="flex flex-wrap gap-2">
            {runnerUps.map((c) => {
              const rel = getKeyAmbiguityRelationship(displayed.key_standard, c.key_standard);
              const b = formatCamelotBadge(c.key_camelot);
              return (
                <div
                  key={c.key_camelot}
                  className="group relative flex items-center gap-2 px-3 py-1.5 rounded-lg bg-surface-light hover:bg-white/10 transition-colors"
                  title={rel.description}
                >
                  <span
                    className="px-1.5 py-0.5 rounded text-[10px] font-bold text-white"
                    style={{ backgroundColor: b.color }}
                  >
                    {b.text}
                  </span>
                  <span className="text-xs text-text-primary">{c.key_standard}</span>
                  {rel.label && (
                    <span className="text-[10px] px-1.5 py-0.5 rounded bg-white/10 text-text-secondary">
                      {rel.label}
                    </span>
                  )}
                </div>
              );
            })}
          </div>
        </div>
      )}

      {overrideActive && (
        <button
          onClick={onClearOverride}
          className="text-xs text-accent-primary hover:underline"
        >
          Revert to engine pick
        </button>
      )}

      <div className="flex gap-2 mt-2">
        <button
          onClick={onCopy}
          className="flex items-center gap-2 px-4 py-2 bg-accent-primary text-white rounded-md text-sm font-medium hover:opacity-90"
        >
          {copied ? <Check className="w-4 h-4" /> : <Copy className="w-4 h-4" />}
          {copied ? 'Copied' : 'Copy'}
        </button>
        <button
          onClick={onReset}
          className="px-4 py-2 bg-surface-light rounded-md text-sm hover:bg-white/10"
        >
          Analyze another
        </button>
      </div>
    </div>
  );
}

// === Explore: scale-notes panel + piano roll ================================

function ExplorePanel({
  activeCamelot,
  activeStandard,
  activeScale,
  isHovered,
  bpm,
}: {
  activeCamelot: string | null;
  activeStandard: string | null;
  activeScale: ScaleNote[];
  isHovered: boolean;
  bpm: number;
}) {
  if (!activeCamelot || activeScale.length === 0) return null;
  const badge = formatCamelotBadge(activeCamelot);

  return (
    <div className="grid grid-cols-1 lg:grid-cols-3 gap-6">
      {/* Notes in this key */}
      <div className="bg-surface/40 rounded-2xl p-4">
        <div className="flex items-center justify-between mb-2">
          <h3 className="text-sm font-semibold">
            Notes in <span style={{ color: badge.color }}>{activeCamelot}</span>
          </h3>
          {isHovered && (
            <span className="text-[10px] uppercase tracking-wide text-accent-primary">
              hovered
            </span>
          )}
        </div>
        <div className="text-xs text-text-secondary mb-3">{activeStandard}</div>
        <div className="flex flex-wrap gap-2">
          {activeScale.map((n) => (
            <button
              key={n.midi}
              onClick={() => playNote({ frequency: n.frequency })}
              className="flex flex-col items-center px-3 py-2 rounded-lg bg-surface-light hover:bg-white/10 transition-colors min-w-[3.5rem]"
              title={`Play ${n.name}4 (${n.frequency.toFixed(2)} Hz)`}
            >
              <span className="text-[10px] text-text-secondary">{n.degree}</span>
              <span className="text-lg font-bold">{n.name}</span>
              {n.altName && (
                <span className="text-[9px] text-text-secondary">/ {n.altName}</span>
              )}
              <span className="text-[9px] font-mono text-text-secondary mt-0.5">
                {n.frequency.toFixed(1)} Hz
              </span>
            </button>
          ))}
        </div>
        <div className="text-[10px] text-text-secondary mt-3 leading-snug">
          Click any note to hear it. Hover a different key on the wheel to swap
          the scale.
        </div>
      </div>

      {/* Piano roll spans 2/3 */}
      <div className="lg:col-span-2 flex flex-col gap-6">
        <PianoRoll highlightedScale={activeScale} />
        <Metronome initialBpm={bpm} />
      </div>
    </div>
  );
}

// === Audio player using Tauri's asset protocol ==============================

function AudioPlayer({ filePath }: { filePath: string }) {
  const audioRef = useRef<HTMLAudioElement>(null);
  const [isPlaying, setIsPlaying] = useState(false);
  const [audioError, setAudioError] = useState<string | null>(null);
  const src = convertFileSrc(filePath);

  const toggle = () => {
    const el = audioRef.current;
    if (!el) return;
    if (el.paused) {
      el.play().catch((e) => setAudioError(String(e)));
    } else {
      el.pause();
    }
  };

  return (
    <div className="flex items-center gap-3 bg-surface/40 rounded-xl px-4 py-3">
      <button
        onClick={toggle}
        className="w-10 h-10 rounded-full bg-accent-primary text-white flex items-center justify-center hover:opacity-90"
        title={isPlaying ? 'Pause' : 'Play'}
      >
        {isPlaying ? <Pause className="w-5 h-5" /> : <Play className="w-5 h-5 ml-0.5" />}
      </button>
      <Volume2 className="w-4 h-4 text-text-secondary" />
      <audio
        ref={audioRef}
        src={src}
        controls
        onPlay={() => setIsPlaying(true)}
        onPause={() => setIsPlaying(false)}
        onError={() => setAudioError('Could not load audio. Check file path / format.')}
        className="flex-1 h-10"
      />
      {audioError && <span className="text-xs text-red-400">{audioError}</span>}
    </div>
  );
}

// === Candidates list ========================================================

function CandidatesPanel({
  candidates,
  selected,
  winnerStandard,
  onSelect,
}: {
  candidates: KeyCandidate[];
  selected: KeyCandidate;
  winnerStandard: string;
  onSelect: (c: KeyCandidate) => void;
}) {
  return (
    <div className="bg-surface/40 rounded-xl p-4">
      <div className="flex items-center justify-between mb-3">
        <h3 className="text-sm font-semibold">Key candidates</h3>
        <div className="text-[10px] text-text-secondary">Click any row to override</div>
      </div>
      <div className="flex flex-col divide-y divide-white/5">
        {candidates.map((c, i) => {
          const isSelected =
            c.key_camelot === selected.key_camelot && c.key_standard === selected.key_standard;
          const badge = formatCamelotBadge(c.key_camelot);
          const rel = getKeyAmbiguityRelationship(winnerStandard, c.key_standard);
          return (
            <button
              key={`${c.key_camelot}-${i}`}
              onClick={() => onSelect(c)}
              className={`
                flex items-center gap-3 py-2 px-1 text-left text-sm transition-colors
                ${isSelected ? 'bg-accent-primary/10' : 'hover:bg-white/5'}
              `}
            >
              <span className="text-text-secondary w-4 text-xs">{i + 1}</span>
              <span
                className="px-2 py-0.5 rounded text-xs font-bold text-white min-w-[2.5rem] text-center"
                style={{ backgroundColor: badge.color }}
              >
                {badge.text}
              </span>
              <span className="flex-1 truncate">{c.key_standard}</span>
              {rel.label && (
                <span
                  className="text-[10px] px-1.5 py-0.5 rounded bg-white/10 text-text-secondary hidden sm:inline"
                  title={rel.description}
                >
                  {rel.label}
                </span>
              )}
              <span className="font-mono text-xs text-text-secondary">
                conf {Math.round(c.confidence * 100)}%
              </span>
              <span className="font-mono text-xs text-text-secondary">
                {c.segment_count}/8
              </span>
            </button>
          );
        })}
      </div>
    </div>
  );
}

// === Chroma bar chart with frequency labels =================================

function ChromaPanel({ chroma, winnerCamelot }: { chroma: number[]; winnerCamelot: string }) {
  const max = Math.max(...chroma, 1e-9);
  const refs = pitchClassFrequencies();
  const scaleNotes = getScaleNotes(winnerCamelot);
  const scalePitchClasses = new Set(scaleNotes.map((n) => n.pitchClass));
  const tonicPc = scaleNotes[0]?.pitchClass ?? -1;

  return (
    <div className="bg-surface/40 rounded-xl p-4">
      <h3 className="text-sm font-semibold mb-2">Chroma vector</h3>
      <div className="text-[10px] text-text-secondary mb-3 leading-snug">
        Average pitch-class energy across the track. Bars in the detected key
        are highlighted; the tonic bar is solid. The tonic should be tall — if
        it isn't, the key pick is suspect.
      </div>
      <div className="flex items-end gap-1 h-28">
        {chroma.map((v, i) => {
          const h = Math.max(2, (v / max) * 100);
          const inScale = scalePitchClasses.has(i);
          const isTonic = i === tonicPc;
          const color = isTonic
            ? '#a78bfa'
            : inScale
              ? '#a78bfa88'
              : '#7c7c8a55';
          return (
            <button
              key={i}
              onClick={() => playNote({ frequency: refs[i].frequency })}
              className="flex-1 flex flex-col items-center gap-1 hover:opacity-80"
              title={`Click to hear ${PITCH_NAMES_SHARP[i]} (${refs[i].frequency.toFixed(2)} Hz)`}
            >
              <div
                className="w-full rounded-t transition-colors"
                style={{ height: `${h}%`, backgroundColor: color }}
              />
              <div className="text-[10px] font-mono text-text-secondary">
                {PITCH_NAMES_SHARP[i]}
              </div>
              <div className="text-[8px] font-mono text-text-secondary/70 leading-none">
                {refs[i].frequency.toFixed(0)}
              </div>
            </button>
          );
        })}
      </div>
      <div className="text-[10px] text-text-secondary mt-3 leading-snug">
        Frequencies shown at octave 4. Click any bar to hear that pitch class.
      </div>
    </div>
  );
}

// === Timings ================================================================

function TimingsPanel({ timings }: { timings: NonNullable<TrackAnalysis['timings']> }) {
  const rows: { label: string; ms: number }[] = [
    { label: 'Decode',       ms: timings.decode_ms },
    { label: 'Spectrogram',  ms: timings.spectrogram_ms },
    { label: 'HPSS',         ms: timings.hpss_ms },
    { label: 'Chromagram',   ms: timings.chromagram_ms },
    { label: 'Key ensemble', ms: timings.ensemble_ms },
    { label: 'Tempo',        ms: timings.tempo_ms },
    { label: 'Metadata',     ms: timings.metadata_ms },
  ];
  const maxMs = Math.max(...rows.map((r) => r.ms), 1);
  return (
    <div className="bg-surface/40 rounded-xl p-4">
      <div className="flex items-center justify-between mb-3">
        <h3 className="text-sm font-semibold">Stage timings</h3>
        <span className="font-mono text-xs text-text-secondary">
          total {timings.total_ms} ms
        </span>
      </div>
      <div className="flex flex-col gap-1.5">
        {rows.map((r) => (
          <div key={r.label} className="flex items-center gap-3 text-xs">
            <span className="w-28 text-text-secondary">{r.label}</span>
            <div className="flex-1 h-2 bg-surface-light rounded-full overflow-hidden">
              <div
                className="h-full bg-accent-primary/60"
                style={{ width: `${(r.ms / maxMs) * 100}%` }}
              />
            </div>
            <span className="w-14 text-right font-mono text-text-secondary">{r.ms} ms</span>
          </div>
        ))}
      </div>
    </div>
  );
}

// === Progress display =======================================================

function AnalysisProgressDisplay({
  progress,
  filename,
}: {
  progress: TunerProgress | null;
  filename: string | null;
}) {
  const percent = Math.round((progress?.percent ?? 0) * 100);
  const stageLabel = progress ? (STAGE_LABELS[progress.stage] ?? progress.stage) : 'Starting';
  return (
    <div className="w-full max-w-md flex flex-col items-center gap-4 px-8">
      <Loader2 className="w-10 h-10 text-accent-primary animate-spin" />
      {filename && <div className="text-sm text-text-secondary truncate max-w-full">{filename}</div>}
      <div className="w-full">
        <div className="flex justify-between text-xs text-text-secondary mb-1">
          <span>{stageLabel}</span>
          <span className="font-mono">{percent}%</span>
        </div>
        <div className="h-2 bg-surface-light rounded-full overflow-hidden">
          <div
            className="h-full bg-accent-primary transition-all duration-200"
            style={{ width: `${percent}%` }}
          />
        </div>
      </div>
    </div>
  );
}

// === Input tab ==============================================================

interface InputTabProps {
  icon: React.ReactNode;
  label: string;
  active: boolean;
  disabled?: boolean;
  onClick: () => void;
}

function InputTab({ icon, label, active, disabled, onClick }: InputTabProps) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      className={`
        flex items-center gap-2 px-4 py-2 rounded-md text-sm font-medium
        transition-colors
        ${active
          ? 'bg-accent-primary text-white'
          : 'bg-surface text-text-secondary hover:text-text-primary hover:bg-white/5'
        }
        ${disabled ? 'opacity-40 cursor-not-allowed' : 'cursor-pointer'}
      `}
    >
      {icon}
      {label}
    </button>
  );
}
