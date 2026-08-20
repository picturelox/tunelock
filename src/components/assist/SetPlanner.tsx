import { useState } from 'react';
import { Calendar, Loader, Sparkles, ArrowRight } from 'lucide-react';
import { assistPlanSet } from '../../lib/tauri';
import { useMixStore } from '../../stores/mixStore';
import { useLibraryStore } from '../../stores/libraryStore';
import type { SetPlan } from '../../types';

export default function SetPlanner() {
  const [instruction, setInstruction] = useState('');
  const [planning, setPlanning] = useState(false);
  const [plan, setPlan] = useState<SetPlan | null>(null);
  const [error, setError] = useState<string | null>(null);

  const addTrack = useMixStore((s) => s.addTrack);
  const { tracks } = useLibraryStore();

  const examples = [
    '90 minutes, start mellow, peak around 60, end on a singalong',
    '60-minute warmup set for a house night, deep and groovy',
    '30-minute closing set, uplifting and emotional, end on a big anthem',
    '2-hour techno journey, dark and driving, build to a peak at 90',
  ];

  const handlePlan = async () => {
    if (!instruction.trim()) return;
    setPlanning(true);
    setError(null);
    try {
      const result = await assistPlanSet(instruction);
      setPlan(result);
    } catch (e) {
      setError(String(e));
    } finally {
      setPlanning(false);
    }
  };

  const handleLoadToMix = () => {
    if (!plan) return;
    plan.trackIds.forEach((id) => addTrack(id));
  };

  return (
    <div className="flex flex-col h-full overflow-hidden">
      <div className="p-4 border-b border-white/5 bg-surface/30">
        <h2 className="text-lg font-bold text-text-primary mb-1">
          Natural-Language Set Planning
        </h2>
        <p className="text-sm text-text-secondary mb-3">
          Describe the set you want in plain English. The LLM sequences tracks
          from your library using harmonic compatibility, BPM, and energy.
          Suggestion, never authority — you can edit the result in Mix Canvas.
        </p>
        <textarea
          value={instruction}
          onChange={(e) => setInstruction(e.target.value)}
          placeholder="e.g., 90 minutes, start mellow, peak around 60, end on a singalong"
          rows={2}
          className="w-full px-3 py-2 bg-background rounded-lg text-sm text-text-primary border border-white/5 focus:border-accent-primary focus:outline-none resize-none"
        />
        <div className="flex items-center gap-3 mt-2">
          <button
            onClick={handlePlan}
            disabled={planning || !instruction.trim()}
            className={`
              flex items-center gap-2 px-4 py-2 rounded-lg text-sm font-medium transition-colors
              ${planning || !instruction.trim()
                ? 'bg-surface text-text-secondary cursor-not-allowed'
                : 'bg-accent-primary text-white hover:bg-accent-primary/90'
              }
            `}
          >
            {planning ? (
              <><Loader className="w-4 h-4 animate-spin" /> Planning...</>
            ) : (
              <><Sparkles className="w-4 h-4" /> Plan Set</>
            )}
          </button>
          <div className="flex gap-1 flex-wrap">
            {examples.map((ex, i) => (
              <button
                key={i}
                onClick={() => setInstruction(ex)}
                className="px-2 py-1 text-xs text-text-secondary hover:text-text-primary bg-background rounded transition-colors"
              >
                Example {i + 1}
              </button>
            ))}
          </div>
        </div>
        {error && (
          <div className="mt-2 text-sm text-red-400 bg-red-500/10 rounded p-2">
            {error}
          </div>
        )}
      </div>

      {plan && (
        <div className="flex-1 overflow-y-auto p-4 space-y-4">
          {/* Plan description */}
          <div className="bg-surface rounded-lg p-4 border border-white/5">
            <h3 className="text-sm font-semibold text-text-primary mb-2">
              {plan.description}
            </h3>
            <p className="text-sm text-text-secondary">{plan.reasoning}</p>
            <div className="mt-3 flex items-center gap-3">
              <span className="text-xs text-text-secondary">
                {plan.trackIds.length} tracks selected
              </span>
              <button
                onClick={handleLoadToMix}
                className="flex items-center gap-1 px-3 py-1.5 bg-accent-primary text-white rounded-lg text-xs font-medium hover:bg-accent-primary/90 transition-colors"
              >
                <Calendar className="w-3 h-3" />
                Load to Mix Canvas
              </button>
            </div>
          </div>

          {/* Track list */}
          <div className="bg-surface rounded-lg p-4 border border-white/5">
            <h3 className="text-sm font-semibold text-text-primary mb-3">
              Proposed Sequence
            </h3>
            <div className="space-y-1">
              {plan.trackIds.map((id, i) => {
                const track = tracks.get(id);
                return (
                  <div key={i} className="flex items-center gap-2 p-2 bg-background rounded text-sm">
                    <span className="text-text-secondary w-6 text-right">{i + 1}</span>
                    <ArrowRight className="w-3 h-3 text-text-secondary" />
                    <div className="flex-1 min-w-0">
                      <span className="text-text-primary truncate">
                        {track?.title || track?.filename || `Track #${id}`}
                      </span>
                      {track?.artist && (
                        <span className="text-text-secondary text-xs ml-2">
                          {track.artist}
                        </span>
                      )}
                    </div>
                    {track?.key_camelot && (
                      <span className="px-1.5 py-0.5 bg-accent-primary/20 text-accent-primary rounded font-mono text-xs">
                        {track.key_camelot}
                      </span>
                    )}
                    {track?.bpm && (
                      <span className="text-text-secondary text-xs">
                        {Math.round(track.bpm)} BPM
                      </span>
                    )}
                  </div>
                );
              })}
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
