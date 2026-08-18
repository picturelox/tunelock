import { useState } from 'react';
import { Download, FileText, Music, FolderUp, Copy } from 'lucide-react';
import { useLibraryStore } from '../../stores/libraryStore';
import { useMixStore } from '../../stores/mixStore';

export default function DeliveryView() {
  const { tracks } = useLibraryStore();
  const { project } = useMixStore();
  const [exporting, setExporting] = useState(false);
  const [lastResult, setLastResult] = useState<string | null>(null);

  const mixTracks = project.clips
    .map((c) => tracks.get(c.trackId))
    .filter((t): t is NonNullable<typeof t> => !!t);

  const handleExportCSV = () => {
    if (mixTracks.length === 0) return;
    setExporting(true);
    const rows = [
      ['#', 'Artist', 'Title', 'BPM', 'Key', 'Camelot', 'Confidence', 'Notes'],
      ...mixTracks.map((t, i) => [
        String(i + 1),
        t.artist ?? '',
        t.title ?? t.filename,
        String(t.bpm ?? ''),
        t.key_standard ?? '',
        t.key_camelot ?? '',
        t.key_confidence != null ? String(Math.round(t.key_confidence * 100)) : '',
        '',
      ]),
    ];
    const csv = rows.map((r) => r.map((c) => `"${c.replace(/"/g, '""')}"`).join(',')).join('\n');
    const blob = new Blob([csv], { type: 'text/csv;charset=utf-8;' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = `${project.name}.csv`;
    a.click();
    URL.revokeObjectURL(url);
    setLastResult('CSV exported');
    setExporting(false);
  };

  const handleExportM3U8 = () => {
    if (mixTracks.length === 0) return;
    setExporting(true);
    const lines = ['#EXTM3U'];
    for (const t of mixTracks) {
      const durationSec = t.duration_ms ? Math.floor(t.duration_ms / 1000) : -1;
      lines.push(`#EXTINF:${durationSec},${t.artist ?? ''} - ${t.title ?? t.filename}`);
      lines.push(t.file_path);
    }
    const m3u = lines.join('\n');
    const blob = new Blob([m3u], { type: 'audio/mpegurl' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = `${project.name}.m3u8`;
    a.click();
    URL.revokeObjectURL(url);
    setLastResult('M3U8 playlist exported');
    setExporting(false);
  };

  return (
    <div className="flex flex-col h-full p-8 gap-6 overflow-auto">
      <div className="flex items-center gap-3">
        <Download className="w-6 h-6 text-accent-primary" />
        <h2 className="text-2xl font-semibold">Delivery</h2>
      </div>

      <p className="text-text-secondary max-w-2xl">
        Non-destructive export of your mix. Original files are never modified.
        Export playlists, tracklists, or prepare a folder for DJ software.
      </p>

      {/* Mix summary */}
      <div className="bg-surface/40 rounded-xl p-4">
        <div className="text-sm font-semibold mb-2">Current Mix: {project.name}</div>
        <div className="text-xs text-text-secondary">
          {mixTracks.length} tracks · {project.transitions.length} transitions
        </div>
        {mixTracks.length === 0 && (
          <div className="text-xs text-text-secondary mt-2">
            Build a mix in the Mix Canvas first to enable exports.
          </div>
        )}
      </div>

      {/* Export actions */}
      <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-4">
        <ExportCard
          icon={<FileText className="w-5 h-5" />}
          label="Export CSV"
          description="Tracklist with artist, title, BPM, key, and Camelot."
          disabled={mixTracks.length === 0 || exporting}
          onClick={handleExportCSV}
        />
        <ExportCard
          icon={<Music className="w-5 h-5" />}
          label="Export M3U8"
          description="Playlist file for Rekordbox, Traktor, VLC, etc."
          disabled={mixTracks.length === 0 || exporting}
          onClick={handleExportM3U8}
        />
        <ExportCard
          icon={<Copy className="w-5 h-5" />}
          label="Copy to Folder"
          description="Non-destructive copy with rename pattern. (Backend command)"
          disabled={mixTracks.length === 0 || exporting}
          onClick={() => {
            setLastResult('Copy to folder requires Tauri backend export_tracks command.');
          }}
        />
        <ExportCard
          icon={<FolderUp className="w-5 h-5" />}
          label="Rename Pattern"
          description="Example: 01_8A_120BPM_Artist-Title.mp3"
          disabled={mixTracks.length === 0 || exporting}
          onClick={() => {
            setLastResult('Rename pattern is configured in the Copy to Folder step.');
          }}
        />
      </div>

      {lastResult && (
        <div className="text-sm text-accent-primary bg-accent-primary/10 rounded-lg px-4 py-2">
          {lastResult}
        </div>
      )}

      {/* Legend */}
      <div className="mt-auto text-xs text-text-secondary max-w-2xl">
        <div className="font-semibold mb-1">Export notes</div>
        <ul className="list-disc list-inside space-y-1">
          <li>Original audio files are never modified.</li>
          <li>CSV includes track order, BPM, key, and Camelot code.</li>
          <li>M3U8 uses absolute file paths for local playback.</li>
          <li>Copy/rename requires the Tauri backend export command (wired in lib.rs).</li>
        </ul>
      </div>
    </div>
  );
}

function ExportCard({
  icon,
  label,
  description,
  disabled,
  onClick,
}: {
  icon: React.ReactNode;
  label: string;
  description: string;
  disabled: boolean;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      className={`
        flex flex-col gap-2 p-4 rounded-xl border text-left transition-colors
        ${disabled
          ? 'border-white/5 bg-surface/20 opacity-40 cursor-not-allowed'
          : 'border-white/10 bg-surface/40 hover:border-accent-primary/50 hover:bg-surface/60'
        }
      `}
    >
      <div className="flex items-center gap-2 text-accent-primary">{icon}</div>
      <div className="text-sm font-medium text-text-primary">{label}</div>
      <div className="text-xs text-text-secondary">{description}</div>
    </button>
  );
}

