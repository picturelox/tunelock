import { useState, useCallback, useEffect, DragEvent } from 'react';
import { Upload, FolderOpen } from 'lucide-react';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { open } from '@tauri-apps/plugin-dialog';
import { analyzeFile, onTunerProgress, getLibraryPage, onTrackAnalyzed } from '../../lib/tauri';
import type { TrackAnalysis, TunerProgress, KeyCandidate, Track } from '../../types';
import {
  formatCamelotBadge,
  getScaleNotes,
  camelotToStandardKey,
  type ScaleNote,
} from '../../lib/harmony';
import ResultPanel from './ResultPanel';
import AnalysisProgressDisplay from './AnalysisProgressDisplay';

export default function TunerView() {
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
  // Guard against React StrictMode double-mounting: the async unlisten
  // may not have resolved when cleanup runs, so we track a cancelled flag
  // and dispose the listener once it resolves if the effect was already
  // cleaned up. This prevents the duplicate `[tuner] DONE` logs.
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
        // If cleanup ran while we were awaiting, tear down immediately.
        if (cancelled && unlisten) {
          unlisten();
          unlisten = null;
        }
      } catch (err) {
        console.warn('Tauri drag-drop unavailable (running outside Tauri?):', err);
      }
    })();
    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  }, [handleAnalyzePath]);

  const handleHtmlDrop = useCallback((e: DragEvent<HTMLDivElement>) => {
    e.preventDefault();
    setIsDragging(false);
  }, []);

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
      {/* One front door: analyze a file. */}
      {!result && (
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
              <Upload className="w-12 h-12 text-accent-primary" />
              <div className="text-xl font-medium">Drop a track anywhere</div>
              <div className="text-sm text-text-secondary">
                Get key, BPM, intensity, and harmonic relationships in one pass.
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

    </div>
  );
}
