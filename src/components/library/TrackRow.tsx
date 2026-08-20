import type { Track } from '../../types';
import { Music2 } from 'lucide-react';
import { convertFileSrc } from '@tauri-apps/api/core';
import { formatCamelotBadge } from '../../lib/harmony';
import ConsensusDots from './ConsensusDots';
import type { ConsensusResult } from '../../lib/tauri';

interface TrackRowProps {
  track: Track;
  index: number;
  consensus?: ConsensusResult | null;
}

function formatDuration(ms: number | null): string {
  if (!ms) return '--:--';
  const totalSeconds = Math.floor(ms / 1000);
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes}:${seconds.toString().padStart(2, '0')}`;
}

export default function TrackRow({ track, index, consensus }: TrackRowProps) {
  const badge = track.key_camelot ? formatCamelotBadge(track.key_camelot) : null;

  return (
    <div
      className={`
        flex items-center px-4 py-2 text-sm border-b border-white/5
        hover:bg-white/5 transition-colors cursor-pointer group
      `}
    >
      <div className="w-8 text-text-secondary text-xs">{index}</div>
      
      <div className="flex-1 min-w-0 truncate flex items-center gap-2">
        {track.artwork_path ? (
          <img
            src={convertFileSrc(track.artwork_path)}
            alt=""
            className="w-6 h-6 rounded object-cover shrink-0 ring-1 ring-white/10"
            draggable={false}
          />
        ) : (
          <Music2 className="w-4 h-4 text-text-secondary shrink-0" />
        )}
        <span className="truncate">{track.title ?? track.filename}</span>
      </div>
      
      <div className="w-32 truncate text-text-secondary">{track.artist ?? '—'}</div>
      <div className="w-32 truncate">{track.title ?? '—'}</div>
      
      <div className="w-16 flex justify-center">
        {badge ? (
          <span
            className="px-2 py-0.5 rounded text-xs font-bold text-white"
            style={{ backgroundColor: badge.color }}
          >
            {badge.text}
          </span>
        ) : (
          <span className="text-text-secondary">—</span>
        )}
      </div>
      
      <div className="w-16 text-right font-mono">
        {track.bpm ? track.bpm.toFixed(1) : '—'}
      </div>
      
      <div className="w-20 text-right text-text-secondary">
        {formatDuration(track.duration_ms)}
      </div>
      
      <div className="w-10 flex justify-center">
        {track.status === 'analyzed' && (
          <div className="w-2 h-2 rounded-full bg-green-500" title="Analyzed" />
        )}
        {track.status === 'analyzing' && (
          <div className="w-2 h-2 rounded-full bg-yellow-500 animate-pulse" title="Analyzing..." />
        )}
        {track.status === 'pending' && (
          <div className="w-2 h-2 rounded-full bg-text-secondary/30" title="Pending" />
        )}
        {track.status === 'error' && (
          <div className="w-2 h-2 rounded-full bg-red-500" title="Error" />
        )}
      </div>

      <ConsensusDots consensus={consensus ?? null} />

    </div>
  );
}
