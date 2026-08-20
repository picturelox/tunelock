import { useState, useEffect } from 'react';
import type { TrackAnalysis, KeyCandidate, Track } from '../../types';
import type { ScaleNote } from '../../lib/harmony';
import CamelotWheel from '../camelot/CamelotWheel';
import HarmonicMosaic, { type FocalTrack } from '../mosaic/HarmonicMosaic';
import ReadoutCard from './ReadoutCard';
import ExplorePanel from './ExplorePanel';
import AudioPlayer from './AudioPlayer';
import CandidatesPanel from './CandidatesPanel';
import ChromaPanel from './ChromaPanel';
import TimingsPanel from './TimingsPanel';
import WaveformDisplay from '../waveform/WaveformDisplay';
import { getWaveformData, type WaveformData } from '../../lib/tauri';

export interface ResultPanelProps {
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

export default function ResultPanel({
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

  // Fetch waveform data for this track (async, non-blocking — the readout
  // renders first, the waveform appears when ready).
  const [waveform, setWaveform] = useState<WaveformData | null>(null);
  useEffect(() => {
    setWaveform(null);
    let cancelled = false;
    (async () => {
      try {
        const data = await getWaveformData(result.track_id);
        if (!cancelled) setWaveform(data);
      } catch (err) {
        console.warn('[tuner] waveform fetch failed:', err);
      }
    })();
    return () => { cancelled = true; };
  }, [result.track_id]);

  return (
    <div className="flex flex-col gap-6">
      {filename && <div className="text-sm text-text-secondary truncate">{filename}</div>}

      {/* Player */}
      {result.file_path && <AudioPlayer filePath={result.file_path} />}

      {/* Three-band waveform */}
      <div className="bg-surface/40 rounded-xl p-3">
        <div className="text-xs text-text-secondary mb-2">Waveform</div>
        <WaveformDisplay data={waveform} height={80} />
      </div>

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

      {/* Harmonic Mosaic */}
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
