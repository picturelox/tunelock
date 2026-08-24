import { useState, useEffect } from 'react';
import { ChevronDown, Gauge, Music2 } from 'lucide-react';
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
  const [detailsOpen, setDetailsOpen] = useState(false);
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

  useEffect(() => {
    setDetailsOpen(false);
  }, [result.track_id]);

  const displayTitle = result.title ?? result.filename ?? filename ?? 'Analyzed track';
  const displayArtist = result.artist ?? 'Unknown artist';

  return (
    <div className="flex flex-col gap-6">
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <h2 className="text-xl font-semibold text-text-primary truncate">{displayTitle}</h2>
          <div className="text-sm text-text-secondary truncate">{displayArtist}</div>
        </div>
        <div className="flex items-center gap-3 shrink-0">
          <div className="rounded-lg bg-surface/60 border border-white/10 px-3 py-2 text-right">
            <div className="flex items-center gap-1.5 text-[10px] uppercase tracking-wide text-text-secondary">
              <Gauge className="w-3 h-3" /> Estimated intensity
            </div>
            <div className="font-mono text-lg text-text-primary">
              {result.energy_level != null ? `${result.energy_level}/10` : 'Unavailable'}
            </div>
          </div>
        </div>
      </div>

      {/* First useful viewport: musical answer, playback, and time map. */}
      <div className="grid grid-cols-1 lg:grid-cols-[minmax(280px,0.8fr)_minmax(0,1.7fr)] gap-6">
        <ReadoutCard
          displayed={displayed}
          candidates={result.candidates ?? []}
          sectionCount={result.section_count ?? 0}
          badge={badge}
          bpm={result.bpm}
          overrideActive={overrideActive}
          onClearOverride={() => onOverride(null)}
          copied={copied}
          onCopy={onCopy}
          onReset={onReset}
        />

        <div className="bg-surface/40 rounded-2xl p-4 flex flex-col gap-4">
          {result.file_path && <AudioPlayer filePath={result.file_path} />}
          <div>
            <div className="flex items-center gap-2 text-xs text-text-secondary mb-2">
              <Music2 className="w-3.5 h-3.5" /> Musical map
            </div>
            <WaveformDisplay data={waveform} height={112} />
            <div className="text-[10px] text-text-secondary mt-2">
              Key regions, beats, and intensity will layer onto this map as their validated analyzers become ready.
            </div>
          </div>
        </div>
      </div>

      {/* Relationship intelligence remains prominent. */}
      <HarmonicMosaic
        focal={focal}
        library={libraryTracks}
        onHoverCandidate={onHoverCamelot}
      />

      <button
        onClick={() => setDetailsOpen((open) => !open)}
        className="flex items-center justify-between rounded-xl border border-white/10 bg-surface/30 px-4 py-3 text-left hover:bg-surface/50 transition-colors"
        aria-expanded={detailsOpen}
      >
        <div>
          <div className="text-sm font-semibold text-text-primary">Why this result?</div>
          <div className="text-xs text-text-secondary">Alternatives, Camelot wheel, scale exploration, chroma, and timings</div>
        </div>
        <ChevronDown className={`w-4 h-4 text-text-secondary transition-transform ${detailsOpen ? 'rotate-180' : ''}`} />
      </button>

      {detailsOpen && (
        <div className="flex flex-col gap-6">
          <div className="bg-surface/40 rounded-2xl p-2">
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

          <ExplorePanel
            activeCamelot={activeCamelot}
            activeStandard={activeStandard}
            activeScale={activeScale}
            isHovered={hoveredCamelot !== null && hoveredCamelot !== displayed.key_camelot}
            bpm={result.bpm}
          />

          <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
            {result.candidates && result.candidates.length > 0 && (
              <CandidatesPanel
                candidates={result.candidates}
                sectionCount={result.section_count ?? 0}
                selected={displayed}
                winnerStandard={displayed.key_standard}
                onSelect={onOverride}
              />
            )}
            {result.chroma && <ChromaPanel chroma={result.chroma} winnerCamelot={displayed.key_camelot} />}
          </div>

          {result.timings && <TimingsPanel timings={result.timings} />}
        </div>
      )}
    </div>
  );
}
