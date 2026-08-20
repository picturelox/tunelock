import { useState, useEffect, useCallback } from 'react';
import { GraduationCap, Music, Ear, BarChart3 } from 'lucide-react';
import {
  getGoldAnnotationSummary,
  getTrainingStats,
  saveGoldAnnotation,
  saveTrainingSession,
} from '../../lib/tauri';
import type { GoldAnnotationSummary, TrainingStats, Track } from '../../types';
import { getLibraryPage } from '../../lib/tauri';
import PitchTraining from './PitchTraining';
import BlindAnnotation from './BlindAnnotation';

type Tab = 'overview' | 'training' | 'annotate' | 'stats';

export default function GoldView() {
  const [tab, setTab] = useState<Tab>('overview');
  const [summary, setSummary] = useState<GoldAnnotationSummary | null>(null);
  const [trainingStats, setTrainingStats] = useState<TrainingStats | null>(null);
  const [tracks, setTracks] = useState<Track[]>([]);

  const refresh = useCallback(async () => {
    try {
      const [s, t] = await Promise.all([
        getGoldAnnotationSummary(),
        getTrainingStats(),
      ]);
      setSummary(s);
      setTrainingStats(t);
    } catch (e) {
      console.error('Failed to load gold set data:', e);
    }
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  // Load tracks for annotation
  useEffect(() => {
    if (tab === 'annotate') {
      getLibraryPage(0, 50, 'filename', 'asc', undefined).then((page) => {
        setTracks(page.tracks);
      }).catch(console.error);
    }
  }, [tab]);

  const tabs: { id: Tab; label: string; icon: typeof GraduationCap }[] = [
    { id: 'overview', label: 'Overview', icon: BarChart3 },
    { id: 'training', label: 'Ear Training', icon: Ear },
    { id: 'annotate', label: 'Annotate', icon: Music },
    { id: 'stats', label: 'Statistics', icon: GraduationCap },
  ];

  return (
    <div className="flex flex-col h-full bg-background">
      {/* Tab bar */}
      <div className="flex items-center gap-1 px-4 py-2 border-b border-white/5 bg-surface">
        {tabs.map(({ id, label, icon: Icon }) => (
          <button
            key={id}
            onClick={() => setTab(id)}
            className={`
              flex items-center gap-2 px-4 py-2 rounded-lg text-sm font-medium
              transition-colors duration-200
              ${tab === id
                ? 'bg-accent-primary text-white'
                : 'text-text-secondary hover:text-text-primary hover:bg-white/5'
              }
            `}
          >
            <Icon className="w-4 h-4" />
            {label}
          </button>
        ))}
      </div>

      {/* Content */}
      <div className="flex-1 overflow-y-auto p-6">
        {tab === 'overview' && (
          <Overview summary={summary} trainingStats={trainingStats} />
        )}
        {tab === 'training' && (
          <PitchTraining
            onSaveSession={saveTrainingSession}
            onStatsChanged={refresh}
          />
        )}
        {tab === 'annotate' && (
          <BlindAnnotation
            tracks={tracks}
            onSaveAnnotation={saveGoldAnnotation}
            onSaved={refresh}
          />
        )}
        {tab === 'stats' && (
          <StatsView summary={summary} trainingStats={trainingStats} />
        )}
      </div>
    </div>
  );
}

function Overview({
  summary,
  trainingStats,
}: {
  summary: GoldAnnotationSummary | null;
  trainingStats: TrainingStats | null;
}) {
  return (
    <div className="max-w-2xl space-y-6">
      <div>
        <h2 className="text-xl font-bold text-text-primary mb-2">Gold Set Overview</h2>
        <p className="text-sm text-text-secondary">
          The gold set is a collection of tracks you have personally annotated
          with key labels, independent of any automated tool. It serves as
          ground truth for measuring engine accuracy.
        </p>
      </div>

      <div className="grid grid-cols-2 gap-4">
        <StatCard
          label="Annotated Tracks"
          value={summary?.annotatedTracks ?? 0}
          total={summary?.totalTracks}
        />
        <StatCard
          label="Total Annotations"
          value={summary?.totalAnnotations ?? 0}
        />
        <StatCard
          label="Self-Agreement"
          value={summary?.selfAgreementPct != null
            ? `${(summary.selfAgreementPct * 100).toFixed(1)}%`
            : 'N/A'}
          hint="Tracks annotated 2+ times with same key"
        />
        <StatCard
          label="Training Accuracy"
          value={trainingStats ? `${trainingStats.accuracyPct.toFixed(1)}%` : 'N/A'}
          hint={`${trainingStats?.totalSessions ?? 0} sessions`}
        />
      </div>

      <div className="bg-surface rounded-lg p-4 border border-white/5">
        <h3 className="text-sm font-semibold text-text-primary mb-3">How it works</h3>
        <ol className="space-y-2 text-sm text-text-secondary">
          <li>
            <span className="text-accent-primary font-bold">1. Train your ear</span>
            {' — '}Use the Ear Training tab to practice identifying pitch
            classes, tonics, and modes. This builds the skill you need to
            annotate accurately.
          </li>
          <li>
            <span className="text-accent-primary font-bold">2. Annotate blindly</span>
            {' — '}In the Annotate tab, listen to a track and identify its key
            without seeing the engine's prediction. This prevents bias.
          </li>
          <li>
            <span className="text-accent-primary font-bold">3. Re-annotate later</span>
            {' — '}Come back after a week and annotate the same tracks again.
            Your self-agreement rate measures annotation reliability.
          </li>
          <li>
            <span className="text-accent-primary font-bold">4. Measure accuracy</span>
            {' — '}Once you have 300+ annotated tracks, the engine's accuracy
            can be measured against your gold set instead of MIK agreement.
          </li>
        </ol>
      </div>

      {summary && Object.keys(summary.modeDistribution).length > 0 && (
        <div className="bg-surface rounded-lg p-4 border border-white/5">
          <h3 className="text-sm font-semibold text-text-primary mb-3">
            Mode Distribution
          </h3>
          <div className="space-y-2">
            {Object.entries(summary.modeDistribution).map(([mode, count]) => (
              <div key={mode} className="flex items-center gap-3">
                <span className="text-sm text-text-secondary w-24">{mode}</span>
                <div className="flex-1 bg-background rounded-full h-4 overflow-hidden">
                  <div
                    className="bg-accent-primary h-full rounded-full"
                    style={{
                      width: `${(count / summary.totalAnnotations) * 100}%`,
                    }}
                  />
                </div>
                <span className="text-sm text-text-primary w-8 text-right">
                  {count}
                </span>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

function StatCard({
  label,
  value,
  total,
  hint,
}: {
  label: string;
  value: number | string;
  total?: number;
  hint?: string;
}) {
  return (
    <div className="bg-surface rounded-lg p-4 border border-white/5">
      <div className="text-xs text-text-secondary uppercase tracking-wide">
        {label}
      </div>
      <div className="text-2xl font-bold text-text-primary mt-1">
        {value}
        {total != null && (
          <span className="text-sm text-text-secondary ml-1">/ {total}</span>
        )}
      </div>
      {hint && (
        <div className="text-xs text-text-secondary mt-1">{hint}</div>
      )}
    </div>
  );
}

function StatsView({
  summary,
  trainingStats,
}: {
  summary: GoldAnnotationSummary | null;
  trainingStats: TrainingStats | null;
}) {
  return (
    <div className="max-w-2xl space-y-6">
      <h2 className="text-xl font-bold text-text-primary">Statistics</h2>

      {trainingStats && (
        <div className="bg-surface rounded-lg p-4 border border-white/5">
          <h3 className="text-sm font-semibold text-text-primary mb-3">
            Training Performance
          </h3>
          <div className="grid grid-cols-2 gap-4 mb-4">
            <div>
              <div className="text-xs text-text-secondary">Total Sessions</div>
              <div className="text-xl font-bold text-text-primary">
                {trainingStats.totalSessions}
              </div>
            </div>
            <div>
              <div className="text-xs text-text-secondary">Accuracy</div>
              <div className="text-xl font-bold text-text-primary">
                {trainingStats.accuracyPct.toFixed(1)}%
              </div>
            </div>
          </div>
          {Object.entries(trainingStats.byType).length > 0 && (
            <div className="space-y-2">
              <div className="text-xs text-text-secondary uppercase">
                By Exercise Type
              </div>
              {Object.entries(trainingStats.byType).map(([type, [total, correct]]) => (
                <div key={type} className="flex items-center gap-3">
                  <span className="text-sm text-text-secondary w-24">{type}</span>
                  <div className="flex-1 bg-background rounded-full h-4 overflow-hidden">
                    <div
                      className="bg-accent-primary h-full rounded-full"
                      style={{ width: `${(correct / total) * 100}%` }}
                    />
                  </div>
                  <span className="text-sm text-text-primary w-16 text-right">
                    {correct}/{total}
                  </span>
                </div>
              ))}
            </div>
          )}
        </div>
      )}

      {summary && (
        <div className="bg-surface rounded-lg p-4 border border-white/5">
          <h3 className="text-sm font-semibold text-text-primary mb-3">
            Gold Set Progress
          </h3>
          <div className="grid grid-cols-3 gap-4">
            <div>
              <div className="text-xs text-text-secondary">Annotated</div>
              <div className="text-xl font-bold text-text-primary">
                {summary.annotatedTracks}
              </div>
            </div>
            <div>
              <div className="text-xs text-text-secondary">Annotations</div>
              <div className="text-xl font-bold text-text-primary">
                {summary.totalAnnotations}
              </div>
            </div>
            <div>
              <div className="text-xs text-text-secondary">Self-Agreement</div>
              <div className="text-xl font-bold text-text-primary">
                {summary.selfAgreementPct != null
                  ? `${(summary.selfAgreementPct * 100).toFixed(1)}%`
                  : 'N/A'}
              </div>
            </div>
          </div>
          {summary.annotatedTracks < 300 && (
            <div className="mt-4 text-sm text-text-secondary">
              Target: 300 annotated tracks for a reliable gold set.
              {' '}Current: {summary.annotatedTracks}.{' '}
              {300 - summary.annotatedTracks > 0
                ? `${300 - summary.annotatedTracks} to go.`
                : 'Goal reached!'}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
