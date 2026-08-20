import { useState, useRef } from 'react';
import { Play, Pause, Volume2 } from 'lucide-react';
import { convertFileSrc } from '@tauri-apps/api/core';

export default function AudioPlayer({ filePath }: { filePath: string }) {
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
