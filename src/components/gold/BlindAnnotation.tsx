import { useState, useCallback } from 'react';
import { Play, Save, Eye, EyeOff, Music2 } from 'lucide-react';
import type { Track, GoldAnnotation } from '../../types';

const PITCH_CLASSES = ['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B'];
const MODES = ['major', 'minor', 'ambiguous', 'atonal'] as const;

interface BlindAnnotationProps {
  tracks: Track[];
  onSaveAnnotation: (annotation: GoldAnnotation) => Promise<number>;
  onSaved: () => void;
}

export default function BlindAnnotation({
  tracks,
  onSaveAnnotation,
  onSaved,
}: BlindAnnotationProps) {
  const [selectedTrack, setSelectedTrack] = useState<Track | null>(null);
  const [showPrediction, setShowPrediction] = useState(false);
  const [tonic, setTonic] = useState<string>('');
  const [mode, setMode] = useState<string>('');
  const [confidence, setConfidence] = useState(3);
  const [evidence, setEvidence] = useState('');
  const [modulates, setModulates] = useState(false);
  const [modulationNote, setModulationNote] = useState('');
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);

  const selectTrack = useCallback((track: Track) => {
    setSelectedTrack(track);
    setShowPrediction(false);
    setTonic('');
    setMode('');
    setConfidence(3);
    setEvidence('');
    setModulates(false);
    setModulationNote('');
    setSaved(false);
  }, []);

  const handleSave = useCallback(async () => {
    if (!selectedTrack || !tonic || !mode) return;
    setSaving(true);
    try {
      await onSaveAnnotation({
        trackId: selectedTrack.id,
        keyTonic: tonic,
        keyMode: mode,
        modulates,
        modulationNote: modulates ? modulationNote : undefined,
        annotatorConfidence: confidence,
        evidence: evidence || undefined,
        annotatorId: 'self',
        blind: !showPrediction,
      });
      setSaved(true);
      onSaved();
    } catch (e) {
      console.error('Failed to save annotation:', e);
    } finally {
      setSaving(false);
    }
  }, [selectedTrack, tonic, mode, confidence, evidence, modulates, modulationNote, showPrediction, onSaveAnnotation, onSaved]);

  return (
    <div className="flex gap-4 h-full">
      {/* Track list */}
      <div className="w-80 flex flex-col bg-surface rounded-lg border border-white/5 overflow-hidden">
        <div className="px-4 py-3 border-b border-white/5">
          <h3 className="text-sm font-semibold text-text-primary">Tracks</h3>
          <p className="text-xs text-text-secondary mt-1">
            {tracks.length} tracks available
          </p>
        </div>
        <div className="flex-1 overflow-y-auto">
          {tracks.map((track) => (
            <button
              key={track.id}
              onClick={() => selectTrack(track)}
              className={`
                w-full text-left px-4 py-2 border-b border-white/5
                transition-colors
                ${selectedTrack?.id === track.id
                  ? 'bg-accent-primary/20'
                  : 'hover:bg-white/5'
                }
              `}
            >
              <div className="text-sm text-text-primary truncate">
                {track.title || track.filename}
              </div>
              <div className="text-xs text-text-secondary truncate">
                {track.artist || 'Unknown artist'}
              </div>
            </button>
          ))}
        </div>
      </div>

      {/* Annotation panel */}
      <div className="flex-1 overflow-y-auto">
        {!selectedTrack ? (
          <div className="flex items-center justify-center h-full">
            <div className="text-center">
              <Music2 className="w-12 h-12 text-text-secondary mx-auto mb-3" />
              <p className="text-text-secondary">
                Select a track from the list to begin annotating
              </p>
            </div>
          </div>
        ) : (
          <div className="max-w-2xl space-y-6">
            {/* Track info */}
            <div className="bg-surface rounded-lg p-4 border border-white/5">
              <h3 className="text-lg font-bold text-text-primary">
                {selectedTrack.title || selectedTrack.filename}
              </h3>
              <p className="text-sm text-text-secondary">
                {selectedTrack.artist || 'Unknown artist'}
              </p>
              <div className="mt-3 flex items-center gap-3">
                <button
                  onClick={() => {
                    // Use Tauri's asset protocol to play the file
                    const audio = new Audio(
                      `asset://localhost/${encodeURIComponent(selectedTrack.file_path)}`
                    );
                    audio.play().catch(console.error);
                  }}
                  className="inline-flex items-center gap-2 px-3 py-1.5 bg-accent-primary text-white rounded-lg text-sm font-medium hover:bg-accent-primary/90 transition-colors"
                >
                  <Play className="w-4 h-4" />
                  Play Track
                </button>
                <button
                  onClick={() => setShowPrediction(!showPrediction)}
                  className={`
                    inline-flex items-center gap-2 px-3 py-1.5 rounded-lg text-sm font-medium transition-colors
                    ${showPrediction
                      ? 'bg-yellow-500/20 text-yellow-400'
                      : 'bg-surface text-text-secondary hover:text-text-primary'
                    }
                  `}
                >
                  {showPrediction ? <Eye className="w-4 h-4" /> : <EyeOff className="w-4 h-4" />}
                  {showPrediction ? 'Prediction Visible' : 'Blind Mode'}
                </button>
              </div>
              {showPrediction && selectedTrack.key_standard && (
                <div className="mt-3 p-2 bg-yellow-500/10 rounded text-sm text-yellow-400">
                  Engine prediction: {selectedTrack.key_standard} ({selectedTrack.key_camelot})
                  — Confidence: {((selectedTrack.key_confidence ?? 0) * 100).toFixed(1)}%
                </div>
              )}
            </div>

            {/* Annotation form */}
            <div className="bg-surface rounded-lg p-4 border border-white/5 space-y-4">
              <h3 className="text-sm font-semibold text-text-primary">
                Your Annotation
              </h3>

              {/* Tonic */}
              <div>
                <label className="text-xs text-text-secondary uppercase tracking-wide block mb-2">
                  Tonic (Root Note)
                </label>
                <div className="grid grid-cols-6 gap-2">
                  {PITCH_CLASSES.map((pc) => (
                    <button
                      key={pc}
                      onClick={() => setTonic(pc)}
                      className={`
                        py-2 rounded-lg text-sm font-medium transition-colors
                        ${tonic === pc
                          ? 'bg-accent-primary text-white'
                          : 'bg-background text-text-primary hover:bg-white/5'
                        }
                      `}
                    >
                      {pc}
                    </button>
                  ))}
                </div>
              </div>

              {/* Mode */}
              <div>
                <label className="text-xs text-text-secondary uppercase tracking-wide block mb-2">
                  Mode
                </label>
                <div className="grid grid-cols-4 gap-2">
                  {MODES.map((m) => (
                    <button
                      key={m}
                      onClick={() => setMode(m)}
                      className={`
                        py-2 rounded-lg text-sm font-medium transition-colors capitalize
                        ${mode === m
                          ? 'bg-accent-primary text-white'
                          : 'bg-background text-text-primary hover:bg-white/5'
                        }
                      `}
                    >
                      {m}
                    </button>
                  ))}
                </div>
              </div>

              {/* Modulation */}
              <div>
                <label className="flex items-center gap-2 text-sm text-text-primary cursor-pointer">
                  <input
                    type="checkbox"
                    checked={modulates}
                    onChange={(e) => setModulates(e.target.checked)}
                    className="w-4 h-4 accent-accent-primary"
                  />
                  Track modulates (changes key)
                </label>
                {modulates && (
                  <input
                    type="text"
                    value={modulationNote}
                    onChange={(e) => setModulationNote(e.target.value)}
                    placeholder="e.g., starts in C minor, modulates to Eb major at 2:30"
                    className="mt-2 w-full px-3 py-2 bg-background rounded-lg text-sm text-text-primary border border-white/5 focus:border-accent-primary focus:outline-none"
                  />
                )}
              </div>

              {/* Confidence */}
              <div>
                <label className="text-xs text-text-secondary uppercase tracking-wide block mb-2">
                  Your Confidence: {confidence}/5
                </label>
                <div className="flex gap-2">
                  {[1, 2, 3, 4, 5].map((n) => (
                    <button
                      key={n}
                      onClick={() => setConfidence(n)}
                      className={`
                        w-10 h-10 rounded-lg text-sm font-medium transition-colors
                        ${confidence >= n
                          ? 'bg-accent-primary text-white'
                          : 'bg-background text-text-secondary hover:bg-white/5'
                        }
                      `}
                    >
                      {n}
                    </button>
                  ))}
                </div>
                <div className="text-xs text-text-secondary mt-1">
                  1=total guess, 3=somewhat sure, 5=certain
                </div>
              </div>

              {/* Evidence */}
              <div>
                <label className="text-xs text-text-secondary uppercase tracking-wide block mb-2">
                  Evidence (optional)
                </label>
                <textarea
                  value={evidence}
                  onChange={(e) => setEvidence(e.target.value)}
                  placeholder="e.g., Bass line sits on C throughout. Melody emphasizes E minor. The G is clearly minor (Eb, not E natural)."
                  rows={3}
                  className="w-full px-3 py-2 bg-background rounded-lg text-sm text-text-primary border border-white/5 focus:border-accent-primary focus:outline-none resize-none"
                />
              </div>

              {/* Save */}
              <button
                onClick={handleSave}
                disabled={!tonic || !mode || saving || saved}
                className={`
                  w-full py-3 rounded-lg text-sm font-medium transition-colors
                  flex items-center justify-center gap-2
                  ${(!tonic || !mode || saving || saved)
                    ? 'bg-surface text-text-secondary cursor-not-allowed'
                    : 'bg-accent-primary text-white hover:bg-accent-primary/90'
                  }
                `}
              >
                <Save className="w-4 h-4" />
                {saved ? 'Saved!' : saving ? 'Saving...' : 'Save Annotation'}
              </button>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
