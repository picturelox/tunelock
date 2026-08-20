import { useState } from 'react';
import { Search, Loader, Music, ArrowRight, TrendingUp } from 'lucide-react';
import { assistAnalyzeSetlist } from '../../lib/tauri';
import type { SetlistAnalysis } from '../../types';

export default function SetlistAnalyzer() {
  const [rawText, setRawText] = useState('');
  const [analyzing, setAnalyzing] = useState(false);
  const [analysis, setAnalysis] = useState<SetlistAnalysis | null>(null);
  const [error, setError] = useState<string | null>(null);

  const handleAnalyze = async () => {
    if (!rawText.trim()) return;
    setAnalyzing(true);
    setError(null);
    try {
      const result = await assistAnalyzeSetlist(rawText);
      setAnalysis(result);
    } catch (e) {
      setError(String(e));
    } finally {
      setAnalyzing(false);
    }
  };

  const examples = [
    `1. Daft Punk - One More Time (00:00)
2. Stardust - Music Sounds Better With You (05:35)
3. Modjo - Lady (Hear Me Tonight) (10:12)
4. Cassius - Feeling For You (14:30)
5. Bob Sinclar - I Feel For You (18:45)`,
    `00:00 - Bicep - Glue
03:42 - Four Tet - Two Thousand and Seventeen
07:15 - Bonobo - Kerala
11:30 - Jon Hopkins - Open Eye Signal
15:20 - Moderat - A New Error`,
  ];

  return (
    <div className="flex flex-col h-full overflow-hidden">
      {/* Input area */}
      <div className="p-4 border-b border-white/5 bg-surface/30">
        <h2 className="text-lg font-bold text-text-primary mb-1">
          DJ Setlist Analysis
        </h2>
        <p className="text-sm text-text-secondary mb-3">
          Paste a DJ tracklist to analyze its harmonic flow. The LLM parses
          the text into structured tracks, matches them against your local
          library, and shows the key/BPM/energy journey of the set.
        </p>
        <textarea
          value={rawText}
          onChange={(e) => setRawText(e.target.value)}
          placeholder="Paste a tracklist here..."
          rows={6}
          className="w-full px-3 py-2 bg-background rounded-lg text-sm text-text-primary border border-white/5 focus:border-accent-primary focus:outline-none resize-none font-mono"
        />
        <div className="flex items-center gap-3 mt-2">
          <button
            onClick={handleAnalyze}
            disabled={analyzing || !rawText.trim()}
            className={`
              flex items-center gap-2 px-4 py-2 rounded-lg text-sm font-medium transition-colors
              ${analyzing || !rawText.trim()
                ? 'bg-surface text-text-secondary cursor-not-allowed'
                : 'bg-accent-primary text-white hover:bg-accent-primary/90'
              }
            `}
          >
            {analyzing ? (
              <><Loader className="w-4 h-4 animate-spin" /> Analyzing...</>
            ) : (
              <><Search className="w-4 h-4" /> Analyze Setlist</>
            )}
          </button>
          <div className="flex gap-1">
            {examples.map((ex, i) => (
              <button
                key={i}
                onClick={() => setRawText(ex)}
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

      {/* Results */}
      {analysis && (
        <div className="flex-1 overflow-y-auto p-4 space-y-4">
          {/* Summary */}
          <div className="bg-surface rounded-lg p-4 border border-white/5">
            <h3 className="text-sm font-semibold text-text-primary mb-3">
              Set Summary
            </h3>
            {analysis.parsed.setName && (
              <div className="text-lg font-bold text-text-primary mb-1">
                {analysis.parsed.setName}
              </div>
            )}
            {analysis.parsed.djName && (
              <div className="text-sm text-text-secondary mb-3">
                DJ: {analysis.parsed.djName}
              </div>
            )}
            <div className="grid grid-cols-4 gap-3">
              <SummaryStat label="Total Tracks" value={analysis.summary.totalTracks} />
              <SummaryStat label="Matched" value={analysis.summary.matchedLocally} />
              <SummaryStat label="Unmatched" value={analysis.summary.unmatched} />
              <SummaryStat
                label="BPM Range"
                value={analysis.summary.bpmRange
                  ? `${Math.round(analysis.summary.bpmRange[0])}-${Math.round(analysis.summary.bpmRange[1])}`
                  : 'N/A'}
              />
            </div>
          </div>

          {/* Key flow visualization */}
          {analysis.summary.keyFlow.length > 0 && (
            <div className="bg-surface rounded-lg p-4 border border-white/5">
              <h3 className="text-sm font-semibold text-text-primary mb-3">
                Harmonic Flow
              </h3>
              <div className="flex flex-wrap items-center gap-1">
                {analysis.summary.keyFlow.map((key, i) => (
                  <div key={i} className="flex items-center gap-1">
                    <span className="px-2 py-1 bg-background rounded text-xs font-mono text-text-primary">
                      {key}
                    </span>
                    {i < analysis.summary.keyFlow.length - 1 && (
                      <ArrowRight className="w-3 h-3 text-text-secondary" />
                    )}
                  </div>
                ))}
              </div>
            </div>
          )}

          {/* Energy arc */}
          {analysis.summary.energyArc.some((e) => e !== null) && (
            <div className="bg-surface rounded-lg p-4 border border-white/5">
              <h3 className="text-sm font-semibold text-text-primary mb-3 flex items-center gap-2">
                <TrendingUp className="w-4 h-4" />
                Energy Arc
              </h3>
              <div className="flex items-end gap-1 h-20">
                {analysis.summary.energyArc.map((energy, i) => (
                  <div
                    key={i}
                    className="flex-1 bg-accent-primary rounded-t transition-all"
                    style={{
                      height: energy ? `${(energy / 10) * 100}%` : '4px',
                      opacity: energy ? 1 : 0.3,
                    }}
                    title={energy ? `Energy: ${energy}/10` : 'Unmatched'}
                  />
                ))}
              </div>
              <div className="flex justify-between text-xs text-text-secondary mt-1">
                <span>Start</span>
                <span>End</span>
              </div>
            </div>
          )}

          {/* Track list */}
          <div className="bg-surface rounded-lg p-4 border border-white/5">
            <h3 className="text-sm font-semibold text-text-primary mb-3">
              Track Breakdown
            </h3>
            <div className="space-y-2">
              {analysis.matchedTracks.map((mt, i) => (
                <div
                  key={i}
                  className="flex items-start gap-3 p-2 bg-background rounded-lg"
                >
                  <div className="text-xs text-text-secondary w-6 text-right pt-0.5">
                    {mt.parsed.position}
                  </div>
                  <div className="flex-1 min-w-0">
                    <div className="flex items-center gap-2">
                      <Music className="w-3 h-3 text-text-secondary flex-shrink-0" />
                      <span className="text-sm text-text-primary truncate">
                        {mt.parsed.artist} — {mt.parsed.title}
                      </span>
                    </div>
                    {mt.parsed.timestamp && (
                      <div className="text-xs text-text-secondary mt-0.5">
                        {mt.parsed.timestamp}
                      </div>
                    )}
                    {mt.localMatch ? (
                      <div className="flex items-center gap-3 mt-1 text-xs">
                        <span className="text-green-400">
                          Matched: {Math.round(mt.localMatch.matchScore * 100)}%
                        </span>
                        {mt.localMatch.keyCamelot && (
                          <span className="px-1.5 py-0.5 bg-accent-primary/20 text-accent-primary rounded font-mono">
                            {mt.localMatch.keyCamelot}
                          </span>
                        )}
                        {mt.localMatch.bpm && (
                          <span className="text-text-secondary">
                            {Math.round(mt.localMatch.bpm)} BPM
                          </span>
                        )}
                        {mt.localMatch.energyLevel && (
                          <span className="text-text-secondary">
                            E: {mt.localMatch.energyLevel}/10
                          </span>
                        )}
                      </div>
                    ) : (
                      <div className="text-xs text-yellow-400 mt-1">
                        Not in library — reference only
                      </div>
                    )}
                    {mt.harmonicFlow && (
                      <div className="text-xs text-text-secondary mt-1 font-mono">
                        {mt.harmonicFlow}
                      </div>
                    )}
                  </div>
                </div>
              ))}
            </div>
          </div>

          {/* Transitions */}
          {analysis.summary.transitions.length > 0 && (
            <div className="bg-surface rounded-lg p-4 border border-white/5">
              <h3 className="text-sm font-semibold text-text-primary mb-3">
                Transitions
              </h3>
              <div className="space-y-1">
                {analysis.summary.transitions.map((trans, i) => (
                  <div key={i} className="text-xs text-text-secondary font-mono">
                    {trans}
                  </div>
                ))}
              </div>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

function SummaryStat({ label, value }: { label: string; value: number | string }) {
  return (
    <div>
      <div className="text-xs text-text-secondary uppercase tracking-wide">
        {label}
      </div>
      <div className="text-lg font-bold text-text-primary">
        {value}
      </div>
    </div>
  );
}
