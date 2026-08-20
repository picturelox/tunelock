import { useState } from 'react';
import { useLibraryStore } from '../../stores/libraryStore';
import { Save, Download, X, Loader2 } from 'lucide-react';
import type { PlaylistRules, Track } from '../../types';
import { parseCamelot, getRelationshipInfo } from '../../lib/harmony';
import { savePlaylist } from '../../lib/tauri';

export default function PlaylistBuilder() {
  const { tracks } = useLibraryStore();
  const [playlist, setPlaylist] = useState<Track[]>([]);
  const [seedTrack, setSeedTrack] = useState<Track | null>(null);
  const [isSaving, setIsSaving] = useState(false);
  const [saveStatus, setSaveStatus] = useState<string | null>(null);
  const [rules, setRules] = useState<PlaylistRules>({
    sameKey: true,
    plusOne: true,
    minusOne: true,
    plusTwo: false,
    minusTwo: false,
    dominantToSubdominant: true,
    subdominantToDominant: true,
    energyCurve: null,
  });

  const trackList = Array.from(tracks.values()).filter((t) => t.status === 'analyzed');

  const generatePlaylist = () => {
    if (!seedTrack || !seedTrack.key_camelot) return;

    const seedPos = seedTrack.key_camelot;
    const compatible: Track[] = [];

    // Get all tracks compatible with the seed
    for (const track of trackList) {
      if (track.id === seedTrack.id) continue;
      if (!track.key_camelot) continue;

      // Simple compatibility check
      if (track.key_camelot === seedPos) {
        if (rules.sameKey) compatible.push(track);
      } else {
        // Would implement full Camelot wheel rules here
        compatible.push(track);
      }
    }

    // Sort by BPM similarity to seed
    const sorted = compatible.sort((a, b) => {
      const seedBpm = seedTrack.bpm ?? 128;
      const aDiff = Math.abs((a.bpm ?? 128) - seedBpm);
      const bDiff = Math.abs((b.bpm ?? 128) - seedBpm);
      return aDiff - bDiff;
    });

    setPlaylist([seedTrack, ...sorted.slice(0, 19)]);
  };

  const removeFromPlaylist = (id: number) => {
    setPlaylist(playlist.filter((t) => t.id !== id));
  };

  const handleSave = async () => {
    if (playlist.length === 0) return;
    setIsSaving(true);
    setSaveStatus(null);
    try {
      const name = `Set ${new Date().toLocaleDateString()} ${new Date().toLocaleTimeString()}`;
      const trackIds = playlist.map((t) => t.id);
      const saved = await savePlaylist(name, trackIds);
      setSaveStatus(`Saved as "${saved.name}"`);
    } catch (err) {
      setSaveStatus(`Save failed: ${err}`);
    } finally {
      setIsSaving(false);
    }
  };

  return (
    <div className="flex flex-col h-full p-4">
      <div className="flex items-center justify-between mb-4">
        <h2 className="text-xl font-semibold">Playlist Builder</h2>
        <div className="flex items-center gap-2">
          {saveStatus && (
            <span className="text-xs text-text-secondary">{saveStatus}</span>
          )}
          <div className="flex gap-2">
          <button
            onClick={handleSave}
            disabled={playlist.length === 0 || isSaving}
            className="flex items-center gap-2 px-3 py-1.5 bg-surface-light rounded-md text-sm disabled:opacity-50"
          >
            {isSaving ? <Loader2 className="w-4 h-4 animate-spin" /> : <Save className="w-4 h-4" />}
            Save
          </button>
          <button
            disabled={playlist.length === 0}
            className="flex items-center gap-2 px-3 py-1.5 bg-accent-primary text-white rounded-md text-sm disabled:opacity-50"
            title="Send to Delivery Mode"
          >
            <Download className="w-4 h-4" />
            Deliver
          </button>
          </div>
        </div>
      </div>

      <div className="grid grid-cols-3 gap-4 mb-4">
        {/* Seed Track Selection */}
        <div className="bg-surface rounded-lg p-3">
          <h3 className="text-sm font-medium mb-2">Starting Track</h3>
          <select
            className="w-full bg-background text-sm rounded px-2 py-1 border border-white/5"
            onChange={(e) => {
              const track = trackList.find((t) => t.id.toString() === e.target.value);
              setSeedTrack(track ?? null);
            }}
          >
            <option value="">Select a track...</option>
            {trackList.slice(0, 100).map((track) => (
              <option key={track.id} value={track.id}>
                {track.artist ?? 'Unknown'} - {track.title ?? track.filename}
              </option>
            ))}
          </select>
        </div>

        {/* Rules */}
        <div className="bg-surface rounded-lg p-3">
          <h3 className="text-sm font-medium mb-2">Harmonic Rules</h3>
          <div className="space-y-1 text-sm">
            {[
              { key: 'sameKey', label: 'Same key' },
              { key: 'plusOne', label: '+1 (clockwise)' },
              { key: 'minusOne', label: '-1 (counter-clockwise)' },
              { key: 'plusTwo', label: '+2 (energy boost)' },
              { key: 'minusTwo', label: '-2' },
              { key: 'dominantToSubdominant', label: 'Major → Minor' },
              { key: 'subdominantToDominant', label: 'Minor → Major' },
            ].map(({ key, label }) => (
              <label key={key} className="flex items-center gap-2 cursor-pointer">
                <input
                  type="checkbox"
                  checked={rules[key as keyof PlaylistRules] as boolean}
                  onChange={(e) =>
                    setRules({ ...rules, [key]: e.target.checked })
                  }
                  className="rounded bg-background border-white/20"
                />
                <span className="text-text-secondary">{label}</span>
              </label>
            ))}
          </div>
        </div>

        {/* Energy Curve */}
        <div className="bg-surface rounded-lg p-3">
          <h3 className="text-sm font-medium mb-2">Energy Curve</h3>
          <div className="space-y-1 text-sm">
            {[
              { value: null, label: 'None (harmonic only)' },
              { value: 'build', label: 'Build up' },
              { value: 'maintain', label: 'Maintain' },
              { value: 'wind_down', label: 'Wind down' },
              { value: 'peak_valley', label: 'Peak & Valley' },
            ].map(({ value, label }) => (
              <label key={label} className="flex items-center gap-2 cursor-pointer">
                <input
                  type="radio"
                  name="energy"
                  checked={rules.energyCurve === value}
                  onChange={() => setRules({ ...rules, energyCurve: value as PlaylistRules['energyCurve'] })}
                  className="bg-background border-white/20"
                />
                <span className="text-text-secondary">{label}</span>
              </label>
            ))}
          </div>
          <button
            onClick={generatePlaylist}
            disabled={!seedTrack}
            className="w-full mt-3 px-3 py-1.5 bg-accent-primary text-white rounded-md text-sm disabled:opacity-50"
          >
            Generate Playlist
          </button>
        </div>
      </div>

      {/* Generated Playlist */}
      {playlist.length > 0 && (
        <div className="flex-1 bg-surface rounded-lg overflow-hidden">
          <div className="flex items-center px-4 py-2 bg-surface-light/50 text-xs font-medium text-text-secondary border-b border-white/5">
            <div className="w-8">#</div>
            <div className="flex-1">Track</div>
            <div className="w-16 text-center">Key</div>
            <div className="w-16 text-right">BPM</div>
            <div className="w-10"></div>
          </div>
          <div className="overflow-auto max-h-[400px]">
            {playlist.map((track, i) => {
              const prev = i > 0 ? playlist[i - 1] : null;
              const prevPos = prev?.key_camelot ? parseCamelot(prev.key_camelot) : null;
              const currPos = track.key_camelot ? parseCamelot(track.key_camelot) : null;
              const rel = prevPos && currPos ? getRelationshipInfo(prevPos, currPos) : null;
              const bpmDelta = prev?.bpm && track.bpm ? track.bpm - prev.bpm : null;

              return (
                <div key={track.id}>
                  {/* Transition hint between this track and the previous one */}
                  {rel && (
                    <div className="flex items-center gap-3 px-4 py-1 text-[11px] text-text-secondary bg-background/40">
                      <span
                        className="px-1.5 py-0.5 rounded text-white font-semibold"
                        style={{ backgroundColor: rel.color }}
                        title={rel.description}
                      >
                        {rel.label}
                      </span>
                      {bpmDelta !== null && (
                        <span className="font-mono">
                          {bpmDelta > 0 ? '+' : ''}
                          {bpmDelta.toFixed(1)} BPM
                        </span>
                      )}
                    </div>
                  )}

                  <div className="flex items-center px-4 py-2 text-sm border-b border-white/5 hover:bg-white/5">
                    <div className="w-8 text-text-secondary">{i + 1}</div>
                    <div className="flex-1 min-w-0 truncate">
                      <div className="truncate">{track.title ?? track.filename}</div>
                      <div className="text-text-secondary text-xs">{track.artist}</div>
                    </div>
                    <div className="w-16 text-center">
                      {track.key_camelot && (
                        <span
                          className="px-1.5 py-0.5 rounded text-xs font-bold text-white"
                          style={{
                            backgroundColor: `hsl(${(parseInt(track.key_camelot) - 1) * 30}, 50%, 50%)`,
                          }}
                        >
                          {track.key_camelot}
                        </span>
                      )}
                    </div>
                    <div className="w-16 text-right font-mono">
                      {track.bpm?.toFixed(1) ?? '—'}
                    </div>
                    <div className="w-10 flex justify-center">
                      <button
                        onClick={() => removeFromPlaylist(track.id)}
                        className="p-1 hover:text-red-400"
                        title="Remove from playlist"
                      >
                        <X className="w-4 h-4" />
                      </button>
                    </div>
                  </div>
                </div>
              );
            })}
          </div>
        </div>
      )}
    </div>
  );
}
