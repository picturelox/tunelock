import { useState, useRef } from 'react';
import { Play, Pause, SkipBack, SkipForward } from 'lucide-react';
import { convertFileSrc } from '@tauri-apps/api/core';
import { useMixStore } from '../../stores/mixStore';
import { useLibraryStore } from '../../stores/libraryStore';

export default function DualAuditionPanel() {
  const { project } = useMixStore();
  const { tracks } = useLibraryStore();

  const [deckA, setDeckA] = useState<number | null>(null);
  const [deckB, setDeckB] = useState<number | null>(null);
  const [playingA, setPlayingA] = useState(false);
  const [playingB, setPlayingB] = useState(false);

  const audioARef = useRef<HTMLAudioElement>(null);
  const audioBRef = useRef<HTMLAudioElement>(null);

  // Auto-populate decks from selected transition
  const selectedTrans = project.transitions.find((t) => t.id === project.selectedTransitionId);
  if (selectedTrans) {
    const fromClip = project.clips.find((c) => c.id === selectedTrans.fromClipId);
    const toClip = project.clips.find((c) => c.id === selectedTrans.toClipId);
    const fromId = fromClip?.trackId ?? null;
    const toId = toClip?.trackId ?? null;
    if (deckA !== fromId) setDeckA(fromId);
    if (deckB !== toId) setDeckB(toId);
  }

  const trackA = deckA ? tracks.get(deckA) : null;
  const trackB = deckB ? tracks.get(deckB) : null;

  const toggleA = () => {
    const el = audioARef.current;
    if (!el) return;
    if (el.paused) {
      el.play().catch(() => {});
      setPlayingA(true);
    } else {
      el.pause();
      setPlayingA(false);
    }
  };

  const toggleB = () => {
    const el = audioBRef.current;
    if (!el) return;
    if (el.paused) {
      el.play().catch(() => {});
      setPlayingB(true);
    } else {
      el.pause();
      setPlayingB(false);
    }
  };

  return (
    <div className="flex items-center h-full gap-4 px-4">
      {/* Deck A */}
      <Deck
        label="A"
        track={trackA}
        isPlaying={playingA}
        onToggle={toggleA}
        audioRef={audioARef}
        onPrev={() => {
          /* cycle to previous clip in mix */
          const idx = project.clips.findIndex((c) => c.trackId === deckA);
          if (idx > 0) setDeckA(project.clips[idx - 1].trackId);
        }}
        onNext={() => {
          const idx = project.clips.findIndex((c) => c.trackId === deckA);
          if (idx >= 0 && idx < project.clips.length - 1) setDeckA(project.clips[idx + 1].trackId);
        }}
      />

      {/* VS / comparison info */}
      <div className="flex flex-col items-center gap-1 px-2">
        <div className="text-[10px] uppercase tracking-wide text-text-secondary">vs</div>
        {trackA && trackB && (
          <div className="text-[10px] text-text-secondary text-center">
            <div>{trackA.key_camelot} → {trackB.key_camelot}</div>
            {trackA.bpm && trackB.bpm && (
              <div>
                {trackA.bpm.toFixed(1)} → {trackB.bpm.toFixed(1)} BPM
              </div>
            )}
          </div>
        )}
      </div>

      {/* Deck B */}
      <Deck
        label="B"
        track={trackB}
        isPlaying={playingB}
        onToggle={toggleB}
        audioRef={audioBRef}
        onPrev={() => {
          const idx = project.clips.findIndex((c) => c.trackId === deckB);
          if (idx > 0) setDeckB(project.clips[idx - 1].trackId);
        }}
        onNext={() => {
          const idx = project.clips.findIndex((c) => c.trackId === deckB);
          if (idx >= 0 && idx < project.clips.length - 1) setDeckB(project.clips[idx + 1].trackId);
        }}
      />
    </div>
  );
}

function Deck({
  label,
  track,
  isPlaying,
  onToggle,
  audioRef,
  onPrev,
  onNext,
}: {
  label: string;
  track: any;
  isPlaying: boolean;
  onToggle: () => void;
  audioRef: React.RefObject<HTMLAudioElement>;
  onPrev: () => void;
  onNext: () => void;
}) {
  const src = track?.file_path ? convertFileSrc(track.file_path) : '';

  return (
    <div className="flex-1 flex items-center gap-3 bg-surface/40 rounded-xl px-3 py-2 min-w-0">
      <div
        className={`
          w-8 h-8 rounded-full flex items-center justify-center text-xs font-bold
          ${label === 'A' ? 'bg-accent-primary text-white' : 'bg-surface-light text-text-primary'}
        `}
      >
        {label}
      </div>

      <div className="flex flex-col min-w-0 flex-1">
        <div className="text-xs font-medium text-text-primary truncate">
          {track ? track.title || track.filename : 'No track loaded'}
        </div>
        {track && (
          <div className="text-[10px] text-text-secondary truncate">
            {track.artist} · {track.key_camelot} · {track.bpm?.toFixed(1)} BPM
          </div>
        )}
      </div>

      {track && (
        <>
          <button
            onClick={onPrev}
            className="p-1.5 rounded hover:bg-white/10 text-text-secondary"
            title="Previous in mix"
          >
            <SkipBack className="w-4 h-4" />
          </button>
          <button
            onClick={onToggle}
            className="p-1.5 rounded hover:bg-white/10 text-text-primary"
            title={isPlaying ? 'Pause' : 'Play'}
          >
            {isPlaying ? <Pause className="w-4 h-4" /> : <Play className="w-4 h-4" />}
          </button>
          <button
            onClick={onNext}
            className="p-1.5 rounded hover:bg-white/10 text-text-secondary"
            title="Next in mix"
          >
            <SkipForward className="w-4 h-4" />
          </button>
          <audio ref={audioRef} src={src} onEnded={() => {}} className="hidden" />
        </>
      )}
    </div>
  );
}
