import { useState, useEffect, useCallback } from 'react';
import { Sparkles, Settings, ListMusic, Wand2, AlertCircle, CheckCircle } from 'lucide-react';
import {
  assistStatus,
  assistSetEnabled,
  assistSetModel,
} from '../../lib/tauri';
import type { AssistStatus } from '../../types';
import SetlistAnalyzer from './SetlistAnalyzer';
import MetadataRepair from './MetadataRepair';

type Tab = 'setlist' | 'metadata' | 'settings';

export default function AssistView() {
  const [tab, setTab] = useState<Tab>('setlist');
  const [status, setStatus] = useState<AssistStatus | null>(null);

  const refreshStatus = useCallback(async () => {
    try {
      const s = await assistStatus();
      setStatus(s);
    } catch (e) {
      console.error('Failed to get assist status:', e);
    }
  }, []);

  useEffect(() => {
    refreshStatus();
  }, [refreshStatus]);

  const available = status?.available ?? false;
  const enabled = status?.enabled ?? false;

  // If Ollama is not available, show a setup screen
  if (!available && (tab as Tab) === 'setlist') {
    return (
      <div className="flex flex-col h-full bg-background">
        <div className="flex items-center gap-1 px-4 py-2 border-b border-white/5 bg-surface">
          <TabButton active={(tab as Tab) === 'setlist'} onClick={() => setTab('setlist')} icon={ListMusic} label="Setlist Analysis" />
          <TabButton active={(tab as Tab) === 'metadata'} onClick={() => setTab('metadata')} icon={Wand2} label="Metadata Repair" />
          <TabButton active={(tab as Tab) === 'settings'} onClick={() => setTab('settings')} icon={Settings} label="Settings" />
        </div>
        <div className="flex-1 flex items-center justify-center p-6">
          <div className="max-w-md text-center">
            <AlertCircle className="w-12 h-12 text-yellow-400 mx-auto mb-4" />
            <h2 className="text-lg font-bold text-text-primary mb-2">
              Ollama Not Detected
            </h2>
            <p className="text-sm text-text-secondary mb-4">
              The Assist layer uses Ollama to run LLMs locally on your machine.
              Ollama is free, open-source, and runs entirely offline — no API
              keys, no cloud calls.
            </p>
            <div className="bg-surface rounded-lg p-4 border border-white/5 text-left">
              <h3 className="text-sm font-semibold text-text-primary mb-2">
                To enable:
              </h3>
              <ol className="space-y-2 text-sm text-text-secondary">
                <li>
                  <span className="text-accent-primary font-bold">1.</span>{' '}
                  Install Ollama from{' '}
                  <span className="text-accent-primary">https://ollama.ai</span>
                </li>
                <li>
                  <span className="text-accent-primary font-bold">2.</span>{' '}
                  Run <code className="bg-background px-1 rounded">ollama pull llama3</code>{' '}
                  (or another model)
                </li>
                <li>
                  <span className="text-accent-primary font-bold">3.</span>{' '}
                  Restart TuneLock
                </li>
                <li>
                  <span className="text-accent-primary font-bold">4.</span>{' '}
                  Enable the Assist layer in Settings
                </li>
              </ol>
            </div>
            <button
              onClick={() => setTab('settings')}
              className="mt-4 px-4 py-2 bg-accent-primary text-white rounded-lg text-sm font-medium hover:bg-accent-primary/90 transition-colors"
            >
              Go to Settings
            </button>
          </div>
        </div>
      </div>
    );
  }

  // If not enabled, show enable prompt
  if (available && !enabled && (tab as Tab) === 'setlist') {
    return (
      <div className="flex flex-col h-full bg-background">
        <div className="flex items-center gap-1 px-4 py-2 border-b border-white/5 bg-surface">
          <TabButton active={(tab as Tab) === 'setlist'} onClick={() => setTab('setlist')} icon={ListMusic} label="Setlist Analysis" />
          <TabButton active={(tab as Tab) === 'metadata'} onClick={() => setTab('metadata')} icon={Wand2} label="Metadata Repair" />
          <TabButton active={(tab as Tab) === 'settings'} onClick={() => setTab('settings')} icon={Settings} label="Settings" />
        </div>
        <div className="flex-1 flex items-center justify-center p-6">
          <div className="max-w-md text-center">
            <Sparkles className="w-12 h-12 text-accent-primary mx-auto mb-4" />
            <h2 className="text-lg font-bold text-text-primary mb-2">
              Assist Layer Ready
            </h2>
            <p className="text-sm text-text-secondary mb-4">
              Ollama is running with {status?.models.length ?? 0} model(s) available.
              Enable the Assist layer to start using LLM-powered features.
            </p>
            <button
              onClick={async () => {
                await assistSetEnabled(true);
                refreshStatus();
              }}
              className="px-4 py-2 bg-accent-primary text-white rounded-lg text-sm font-medium hover:bg-accent-primary/90 transition-colors"
            >
              Enable Assist Layer
            </button>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="flex flex-col h-full bg-background">
      {/* Tab bar */}
      <div className="flex items-center gap-1 px-4 py-2 border-b border-white/5 bg-surface">
        <TabButton active={(tab as Tab) === 'setlist'} onClick={() => setTab('setlist')} icon={ListMusic} label="Setlist Analysis" />
        <TabButton active={(tab as Tab) === 'metadata'} onClick={() => setTab('metadata')} icon={Wand2} label="Metadata Repair" />
        <TabButton active={(tab as Tab) === 'settings'} onClick={() => setTab('settings')} icon={Settings} label="Settings" />
        <div className="flex-1" />
        {enabled && (
          <div className="flex items-center gap-1 text-xs text-green-400">
            <CheckCircle className="w-3 h-3" />
            Active
          </div>
        )}
      </div>

      {/* Content */}
      <div className="flex-1 overflow-hidden">
        {(tab as Tab) === 'setlist' && <SetlistAnalyzer />}
        {(tab as Tab) === 'metadata' && <MetadataRepair />}
        {(tab as Tab) === 'settings' && (
          <SettingsTab status={status} onRefresh={refreshStatus} />
        )}
      </div>
    </div>
  );
}

function TabButton({
  active,
  onClick,
  icon: Icon,
  label,
}: {
  active: boolean;
  onClick: () => void;
  icon: typeof Sparkles;
  label: string;
}) {
  return (
    <button
      onClick={onClick}
      className={`
        flex items-center gap-2 px-4 py-2 rounded-lg text-sm font-medium
        transition-colors duration-200
        ${active
          ? 'bg-accent-primary text-white'
          : 'text-text-secondary hover:text-text-primary hover:bg-white/5'
        }
      `}
    >
      <Icon className="w-4 h-4" />
      {label}
    </button>
  );
}

function SettingsTab({
  status,
  onRefresh,
}: {
  status: AssistStatus | null;
  onRefresh: () => void;
}) {
  const [selectedModel, setSelectedModel] = useState<string | null>(
    status?.selectedModel ?? null
  );

  useEffect(() => {
    setSelectedModel(status?.selectedModel ?? null);
  }, [status]);

  const handleToggle = async (enabled: boolean) => {
    await assistSetEnabled(enabled);
    onRefresh();
  };

  const handleModelChange = async (model: string) => {
    setSelectedModel(model);
    await assistSetModel(model);
  };

  if (!status) {
    return (
      <div className="flex items-center justify-center h-full">
        <Sparkles className="w-6 h-6 text-text-secondary animate-pulse" />
      </div>
    );
  }

  return (
    <div className="max-w-2xl p-6 space-y-6 overflow-y-auto">
      <div>
        <h2 className="text-xl font-bold text-text-primary mb-2">Assist Settings</h2>
        <p className="text-sm text-text-secondary">
          The Assist layer uses Ollama to run LLMs locally. No data leaves your
          machine. The LLM is never on the critical path to key/BPM results.
        </p>
      </div>

      {/* Connection status */}
      <div className="bg-surface rounded-lg p-4 border border-white/5">
        <div className="flex items-center justify-between mb-3">
          <h3 className="text-sm font-semibold text-text-primary">
            Ollama Connection
          </h3>
          <div className={`flex items-center gap-1 text-xs ${
            status.available ? 'text-green-400' : 'text-red-400'
          }`}>
            {status.available ? (
              <><CheckCircle className="w-3 h-3" /> Connected</>
            ) : (
              <><AlertCircle className="w-3 h-3" /> Not running</>
            )}
          </div>
        </div>
        <div className="text-xs text-text-secondary">
          URL: {status.ollamaUrl}
        </div>
        {status.available && (
          <div className="text-xs text-text-secondary mt-1">
            Models available: {status.models.length}
          </div>
        )}
      </div>

      {/* Enable/disable */}
      <div className="bg-surface rounded-lg p-4 border border-white/5">
        <div className="flex items-center justify-between">
          <div>
            <h3 className="text-sm font-semibold text-text-primary">
              Enable Assist Layer
            </h3>
            <p className="text-xs text-text-secondary mt-1">
              When enabled, LLM-powered features become available.
            </p>
          </div>
          <button
            onClick={() => handleToggle(!status.enabled)}
            disabled={!status.available}
            className={`
              relative w-12 h-6 rounded-full transition-colors
              ${status.enabled
                ? 'bg-accent-primary'
                : 'bg-background'
              }
              ${!status.available ? 'opacity-40 cursor-not-allowed' : ''}
            `}
          >
            <div className={`
              absolute top-0.5 w-5 h-5 rounded-full bg-white transition-transform
              ${status.enabled ? 'translate-x-6' : 'translate-x-0.5'}
            `} />
          </button>
        </div>
      </div>

      {/* Model selection */}
      {status.available && (
        <div className="bg-surface rounded-lg p-4 border border-white/5">
          <h3 className="text-sm font-semibold text-text-primary mb-3">
            Model Selection
          </h3>
          {status.models.length === 0 ? (
            <p className="text-sm text-text-secondary">
              No models installed. Run{' '}
              <code className="bg-background px-1 rounded">ollama pull llama3</code>{' '}
              to install a model.
            </p>
          ) : (
            <div className="space-y-2">
              {status.models.map((model) => (
                <button
                  key={model.name}
                  onClick={() => handleModelChange(model.name)}
                  className={`
                    w-full text-left px-3 py-2 rounded-lg text-sm transition-colors
                    ${selectedModel === model.name
                      ? 'bg-accent-primary text-white'
                      : 'bg-background text-text-primary hover:bg-white/5'
                    }
                  `}
                >
                  <div className="font-medium">{model.name}</div>
                  {model.size && (
                    <div className={`text-xs ${
                      selectedModel === model.name ? 'text-white/70' : 'text-text-secondary'
                    }`}>
                      {(model.size / 1e9).toFixed(1)} GB
                    </div>
                  )}
                </button>
              ))}
            </div>
          )}
        </div>
      )}

      {/* Privacy note */}
      <div className="bg-surface rounded-lg p-4 border border-white/5">
        <h3 className="text-sm font-semibold text-text-primary mb-2">Privacy</h3>
        <ul className="space-y-1 text-xs text-text-secondary">
          <li>• All LLM inference runs locally via Ollama — no data is sent to any server.</li>
          <li>• The LLM is never used for key/BPM/energy detection — those are always local DSP.</li>
          <li>• Assist features are user-initiated only — nothing happens automatically in the background.</li>
          <li>• You can disable the Assist layer at any time. All other features continue to work.</li>
        </ul>
      </div>
    </div>
  );
}
