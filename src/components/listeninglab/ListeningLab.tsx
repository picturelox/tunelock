import { useState, useEffect, useRef, useCallback } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import {
  audioEngineInit,
  audioEngineLoadPlayerPaused,
  audioEnginePlay,
  audioEnginePause,
  audioEngineStop,
  audioEngineSeek,
  audioEngineSetTempo,
  audioEngineSetPitch,
  audioEngineSetLoop,
  audioEngineSetBus,
  audioEngineSetMasterGain,
  audioEngineSetProcessorType,
  audioEngineSetListeningCondition,
  audioEngineSyncLaunch,
  audioEngineSeekSourceSeconds,
  audioEngineBeatSync,
  audioEngineBarSync,
  audioEngineGetMeters,
  listeningLabGetProcessorInfo,
  listeningLabSaveResult,
  listeningLabGetResults,
  getGitRevision,
  type AudioMeterReadout,
  type ListeningLabProcessorInfo,
  type ListeningLabResult,
} from '../../lib/tauri';

type ProcessorMode = 'original' | 'signalsmith';
type TestMode = 'quality' | 'transport' | 'twodeck' | 'abx';

// Fallback version if git SHA is not available.
const APP_VERSION = '0.1.0-pb2-listening-lab';

const TEMPO_PRESETS = [-10, -6, -2, 0, 2, 6, 10];
const PITCH_PRESETS = [-3, -1, 0, 1, 3];
const MATERIAL_OPTIONS = [
  { value: 'drums', label: 'Drum-heavy' },
  { value: 'bass', label: 'Bass-heavy' },
  { value: 'vocals', label: 'Vocal-heavy' },
  { value: 'acoustic', label: 'Acoustic' },
  { value: 'dense', label: 'Dense master' },
  { value: 'sparse', label: 'Sparse' },
  { value: 'familiar', label: 'Well-known track' },
];

type InitState = 'idle' | 'initializing' | 'ready' | 'error';

export default function ListeningLab() {
  const [initState, setInitState] = useState<InitState>('idle');
  const [initError, setInitError] = useState<string>('');
  const [processorInfo, setProcessorInfo] = useState<ListeningLabProcessorInfo | null>(null);
  const [meters, setMeters] = useState<AudioMeterReadout | null>(null);
  const [testMode, setTestMode] = useState<TestMode>('quality');
  const [tempo, setTempo] = useState(0);
  const [pitch, setPitch] = useState(0);
  const [processor, setProcessor] = useState<ProcessorMode>('signalsmith');
  const [material, setMaterial] = useState('drums');
  const [trackName, setTrackName] = useState<string>('');
  const [filePath, setFilePath] = useState<string>('');
  const [isPlaying, setIsPlaying] = useState(false);
  const [loopBeats, setLoopBeats] = useState<number | null>(null);
  const [positionSec, setPositionSec] = useState(0);

  // ABX state
  const [abxHidden, setAbxHidden] = useState(false);
  const [abxIsB, setAbxIsB] = useState(false);
  const [abxCorrect, setAbxCorrect] = useState(0);
  const [abxTrials, setAbxTrials] = useState(0);
  const [abxLastAnswer, setAbxLastAnswer] = useState<string>('');
  const [gitRevision, setGitRevision] = useState<string>('');
  // ABX cue position (in seconds). Each trial restarts from this position.
  const [abxCueSec, setAbxCueSec] = useState(0);

  // Ratings
  const [transients, setTransients] = useState(0);
  const [bass, setBass] = useState(0);
  const [vocals, setVocals] = useState(0);
  const [stereo, setStereo] = useState(0);
  const [artifacts, setArtifacts] = useState(0);
  const [overall, setOverall] = useState(0);
  const [notes, setNotes] = useState('');
  const [savedResults, setSavedResults] = useState<ListeningLabResult[]>([]);
  const [saveStatus, setSaveStatus] = useState('');

  // Two-deck state
  const [, setFilePathB] = useState<string>('');
  const [trackNameB, setTrackNameB] = useState<string>('');

  const meterRef = useRef<number | null>(null);

  const initEngine = useCallback(async () => {
    setInitState('initializing');
    setInitError('');
    try {
      await audioEngineInit();
      const info = await listeningLabGetProcessorInfo();
      setProcessorInfo(info);
      await audioEngineSetMasterGain(1.0);
      await audioEngineSetBus(0, 'master');
      await audioEngineSetBus(1, 'master');
      setInitState('ready');
    } catch (e) {
      console.error('Listening Lab init failed:', e);
      setInitError(String(e));
      setInitState('error');
    }
  }, []);

  useEffect(() => {
    initEngine();
    listeningLabGetResults().then(setSavedResults).catch(() => {});
    // Fetch the actual git SHA for saving with results
    getGitRevision().then(setGitRevision).catch(() => setGitRevision(APP_VERSION));
    return () => {
      if (meterRef.current) cancelAnimationFrame(meterRef.current);
    };
  }, [initEngine]);

  // Poll meters for position display
  useEffect(() => {
    if (initState !== 'ready') return;
    let active = true;
    const poll = async () => {
      if (!active) return;
      try {
        const m = await audioEngineGetMeters();
        if (!active) return;
        setMeters(m);
        if (m.players[0]) setPositionSec(m.players[0].positionSec);
      } catch { /* ignore */ }
      meterRef.current = requestAnimationFrame(poll);
    };
    meterRef.current = requestAnimationFrame(poll);
    return () => {
      active = false;
      if (meterRef.current) cancelAnimationFrame(meterRef.current);
    };
  }, [initState]);

  const handleLoadFile = async (deck: 0 | 1) => {
    const selected = await open({
      filters: [{ name: 'Audio', extensions: ['wav', 'mp3', 'flac', 'aiff', 'm4a', 'ogg'] }],
      multiple: false,
    });
    if (!selected || typeof selected !== 'string') return;
    try {
      await audioEngineLoadPlayerPaused(deck, selected);
      const name = selected.split(/[\\/]/).pop() || selected;
      if (deck === 0) {
        setFilePath(selected);
        setTrackName(name);
      } else {
        setFilePathB(selected);
        setTrackNameB(name);
      }
    } catch (e) {
      console.error('Load failed:', e);
    }
  };

  const handlePlay = async () => {
    if (!filePath) return;
    try {
      await audioEnginePlay(0);
      setIsPlaying(true);
    } catch (e) { console.error(e); }
  };

  const handlePause = async () => {
    try {
      await audioEnginePause(0);
      setIsPlaying(false);
    } catch (e) { console.error(e); }
  };

  const handleStop = async () => {
    try {
      await audioEngineStop(0);
      setIsPlaying(false);
    } catch (e) { console.error(e); }
  };

  const handleSeek = async (beats: number) => {
    try { await audioEngineSeek(0, beats); } catch (e) { console.error(e); }
  };

  // Meter-aware loop: bars → beats using the track's meter numerator.
  // In 4/4: 1 bar = 4 beats, 2 bars = 8 beats, etc.
  // In 3/4: 1 bar = 3 beats, 2 bars = 6 beats, etc.
  const handleSetLoop = async (bars: number | null) => {
    if (bars === null) {
      setLoopBeats(null);
      try { await audioEngineSetLoop(0, null, null); } catch (e) { console.error(e); }
      return;
    }
    const meterNum = meters?.players[0]?.meterNumerator || 4;
    const beats = bars * meterNum;
    setLoopBeats(beats);
    try {
      await audioEngineSetLoop(0, 0, beats);
    } catch (e) { console.error(e); }
  };

  const applyTempo = async (pct: number) => {
    setTempo(pct);
    const rate = 1 + pct / 100;
    try { await audioEngineSetTempo(0, rate); } catch (e) { console.error(e); }
  };

  const applyPitch = async (st: number) => {
    setPitch(st);
    try { await audioEngineSetPitch(0, st); } catch (e) { console.error(e); }
  };

  // ABX: randomly switch between original (tempo=0, pitch=0) and processed
  // ABX: each trial restarts from the cue position with the selected
  // processor applied atomically (processor + tempo + pitch in one
  // command). No hot-swapping mid-playback — the switch itself would
  // reveal which processor was selected.
  const abxStartTrial = (isB: boolean) => {
    // Pause first
    audioEnginePause(0);
    // Seek to cue position in seconds (not beats) — no BPM assumption
    audioEngineSeekSourceSeconds(0, abxCueSec);
    // Apply condition atomically: processor + tempo + pitch in one command
    if (isB) {
      // B = Signalsmith with current tempo/pitch
      audioEngineSetListeningCondition(0, 'signalsmith', 1 + tempo / 100, pitch);
    } else {
      // A = bypass (true original)
      audioEngineSetListeningCondition(0, 'bypass', 1.0, 0);
    }
    // Start playback from the cue
    audioEnginePlay(0);
  };

  const abxStart = () => {
    setAbxHidden(true);
    const initialIsB = Math.random() < 0.5;
    setAbxIsB(initialIsB);
    setAbxCorrect(0);
    setAbxTrials(0);
    setAbxLastAnswer('');
    abxStartTrial(initialIsB);
  };

  const abxReveal = (guess: 'a' | 'b') => {
    const actualIsB = abxIsB;
    const correct = (guess === 'b') === actualIsB;
    const newCorrect = abxCorrect + (correct ? 1 : 0);
    const newTrials = abxTrials + 1;
    setAbxCorrect(newCorrect);
    setAbxTrials(newTrials);
    setAbxLastAnswer(correct ? 'Correct!' : 'Wrong.');
    // Next trial: randomly select, restart from cue
    const nextIsB = Math.random() < 0.5;
    setAbxIsB(nextIsB);
    abxStartTrial(nextIsB);
  };

  const abxStop = () => {
    setAbxHidden(false);
    // Restore signalsmith processor at unity
    audioEngineSetListeningCondition(0, 'signalsmith', 1 + tempo / 100, pitch);
  };

  const handleSave = async () => {
    const result: ListeningLabResult = {
      timestamp: new Date().toISOString(),
      processor: processor === 'original' ? 'bypass' : 'signalsmith',
      tempoPercent: tempo,
      pitchSemitones: pitch,
      material,
      trackName: trackName || undefined,
      transients,
      bass,
      vocals,
      stereo,
      artifacts,
      overall,
      abxCorrect: abxTrials > 0 ? abxCorrect : undefined,
      abxTrials: abxTrials > 0 ? abxTrials : undefined,
      notes: notes || undefined,
      gitRevision: gitRevision || undefined,
    };
    try {
      await listeningLabSaveResult(result);
      setSaveStatus('Saved!');
      setTimeout(() => setSaveStatus(''), 2000);
      const updated = await listeningLabGetResults();
      setSavedResults(updated);
      // Reset ratings
      setTransients(0); setBass(0); setVocals(0); setStereo(0); setArtifacts(0); setOverall(0);
      setNotes('');
    } catch (e) {
      setSaveStatus('Save failed: ' + String(e));
    }
  };

  const fmtTime = (sec: number) => {
    const m = Math.floor(sec / 60);
    const s = sec % 60;
    return `${m}:${s.toFixed(3).padStart(6, '0')}`;
  };

  if (initState !== 'ready') {
    return (
      <div className="p-8 text-label-dim">
        <h2 className="text-xl font-bold text-label-cream mb-4">PB-2 Listening Lab</h2>
        {initState === 'initializing' && <p>Initializing audio engine...</p>}
        {initState === 'idle' && <p>Click to initialize the audio engine.</p>}
        {initState === 'idle' && (
          <button onClick={() => initEngine()} className="mt-3 px-4 py-2 bg-cap-amber text-black rounded text-sm font-medium">
            Initialize
          </button>
        )}
        {initState === 'error' && (
          <div className="mt-3">
            <p className="text-red-400 mb-2">Audio engine failed to start:</p>
            <pre className="text-xs text-label-dim bg-plate-dark p-3 rounded mb-3 whitespace-pre-wrap">{initError}</pre>
            <button onClick={() => initEngine()} className="px-4 py-2 bg-cap-amber text-black rounded text-sm font-medium">
              Retry
            </button>
          </div>
        )}
      </div>
    );
  }

  return (
    <div className="p-6 max-w-6xl mx-auto text-label-cream">
      <h2 className="text-xl font-bold mb-1">PB-2 Listening Lab</h2>
      <p className="text-sm text-label-dim mb-6">
        Developer tool for human validation of time/pitch processing.
        Uses the production Performance Engine and SignalsmithProcessor.
        No filters, limiter, or mastering — clean signal path only.
      </p>

      {/* Processor info + musical telemetry */}
      <div className="mb-4 text-xs text-label-dim flex flex-wrap gap-6">
        <span>Processor: <span className="text-label-cream">{processorInfo?.processorType || 'signalsmith'}</span></span>
        <span>Sample rate: <span className="text-label-cream">{processorInfo?.sampleRate || '?'} Hz</span></span>
        <span>Latency: <span className="text-label-cream">{processorInfo?.latencyFrames ?? '?'} frames</span></span>
        <span>Position: <span className="text-label-cream">{fmtTime(positionSec)}</span></span>
        {meters?.players?.[0] && (
          <>
            <span>Source BPM: <span className="text-label-cream">{(meters.players[0].sourceBpm ?? 0) > 0 ? meters.players[0].sourceBpm.toFixed(2) : '—'}</span></span>
            <span>Effective BPM: <span className="text-label-cream">{(meters.players[0].effectiveBpm ?? 0) > 0 ? meters.players[0].effectiveBpm.toFixed(2) : '—'}</span></span>
            <span>Tempo: <span className="text-label-cream">{(((meters.players[0].tempoRatio ?? 1) - 1) * 100).toFixed(2)}%</span></span>
            <span>Pitch: <span className="text-label-cream">{(meters.players[0].pitchSemitones ?? 0) > 0 ? '+' : ''}{(meters.players[0].pitchSemitones ?? 0).toFixed(1)} st</span></span>
            <span>Beat: <span className="text-label-cream">{(meters.players[0].beatPosition ?? 0).toFixed(1)}</span></span>
            <span>Bar: <span className="text-label-cream">{(meters.players[0].barPosition ?? 0).toFixed(1)}</span></span>
            <span>Meter: <span className="text-label-cream">{meters.players[0].meterNumerator ?? 4}/4</span></span>
          </>
        )}
        {isPlaying && <span className="text-cap-amber">● PLAYING</span>}
      </div>

      {/* Test mode selector */}
      <div className="flex gap-2 mb-6">
        {(['quality', 'transport', 'twodeck', 'abx'] as TestMode[]).map((m) => (
          <button
            key={m}
            onClick={() => setTestMode(m)}
            className={`px-3 py-1.5 text-sm rounded ${
              testMode === m ? 'bg-cap-amber text-black' : 'bg-plate-light text-label-dim hover:text-label-cream'
            }`}
          >
            {m === 'quality' ? 'Quality' : m === 'transport' ? 'Transport' : m === 'twodeck' ? 'Two-Deck' : 'A/B/X'}
          </button>
        ))}
      </div>

      {/* File loading */}
      <div className="mb-6 p-4 bg-plate-dark rounded-lg border border-plate-darker">
        <div className="flex items-center gap-4">
          <button
            onClick={() => handleLoadFile(0)}
            className="px-4 py-2 bg-plate-light rounded text-sm hover:bg-plate-lighter"
          >
            Load Track A
          </button>
          <span className="text-sm text-label-dim">{trackName || 'No file loaded'}</span>
        </div>
        {testMode === 'twodeck' && (
          <div className="flex items-center gap-4 mt-3">
            <button
              onClick={() => handleLoadFile(1)}
              className="px-4 py-2 bg-plate-light rounded text-sm hover:bg-plate-lighter"
            >
              Load Track B
            </button>
            <span className="text-sm text-label-dim">{trackNameB || 'No file loaded'}</span>
          </div>
        )}
      </div>

      {/* Controls */}
      {!abxHidden && (
        <>
          {/* Processor selection */}
          {testMode === 'quality' && (
            <div className="mb-4">
              <label className="text-xs text-label-dim block mb-1">PROCESSOR</label>
              <div className="flex gap-2">
                <button
                  onClick={() => {
                    setProcessor('original');
                    audioEngineSetProcessorType(0, 'bypass');
                    applyTempo(0);
                    applyPitch(0);
                  }}
                  className={`px-3 py-1 text-sm rounded ${processor === 'original' ? 'bg-cap-amber text-black' : 'bg-plate-light text-label-dim'}`}
                >
                  Original (bypass)
                </button>
                <button
                  onClick={() => {
                    setProcessor('signalsmith');
                    audioEngineSetProcessorType(0, 'signalsmith');
                  }}
                  className={`px-3 py-1 text-sm rounded ${processor === 'signalsmith' ? 'bg-cap-amber text-black' : 'bg-plate-light text-label-dim'}`}
                >
                  Signalsmith
                </button>
              </div>
              <p className="text-xs text-label-dim mt-1">
                Original = true bypass (unprocessed source). Signalsmith = pitch-preserving time stretch.
              </p>
            </div>
          )}

          {/* Tempo presets */}
          <div className="mb-4">
            <label className="text-xs text-label-dim block mb-1">TEMPO</label>
            <div className="flex gap-2">
              {TEMPO_PRESETS.map((p) => (
                <button
                  key={p}
                  onClick={() => applyTempo(p)}
                  className={`px-3 py-1 text-sm rounded min-w-[3.5rem] ${
                    tempo === p ? 'bg-cap-amber text-black' : 'bg-plate-light text-label-dim hover:text-label-cream'
                  }`}
                >
                  {p > 0 ? `+${p}%` : `${p}%`}
                </button>
              ))}
            </div>
          </div>

          {/* Pitch presets */}
          <div className="mb-6">
            <label className="text-xs text-label-dim block mb-1">PITCH (semitones)</label>
            <div className="flex gap-2">
              {PITCH_PRESETS.map((p) => (
                <button
                  key={p}
                  onClick={() => applyPitch(p)}
                  className={`px-3 py-1 text-sm rounded min-w-[3rem] ${
                    pitch === p ? 'bg-cap-amber text-black' : 'bg-plate-light text-label-dim hover:text-label-cream'
                  }`}
                >
                  {p > 0 ? `+${p}` : `${p}`}
                </button>
              ))}
            </div>
          </div>
        </>
      )}

      {/* Transport controls */}
      <div className="flex gap-2 mb-6">
        {!isPlaying ? (
          <button onClick={handlePlay} className="px-4 py-2 bg-cap-amber text-black rounded text-sm font-medium">
            ▶ Play
          </button>
        ) : (
          <button onClick={handlePause} className="px-4 py-2 bg-plate-lighter rounded text-sm">
            ❚❚ Pause
          </button>
        )}
        <button onClick={handleStop} className="px-4 py-2 bg-plate-light rounded text-sm">■ Stop</button>
        <button onClick={() => handleSeek(0)} className="px-4 py-2 bg-plate-light rounded text-sm">⏮ Start</button>
        <button onClick={() => handleSeek(32)} className="px-4 py-2 bg-plate-light rounded text-sm">+32 beats</button>
        <button onClick={() => handleSeek(64)} className="px-4 py-2 bg-plate-light rounded text-sm">+64 beats</button>
      </div>

      {/* Loop controls (transport mode) — meter-aware */}
      {testMode === 'transport' && (
        <div className="mb-6">
          <label className="text-xs text-label-dim block mb-1">
            LOOP ({meters?.players[0]?.meterNumerator || 4}/4 time — bar = {meters?.players[0]?.meterNumerator || 4} beats)
          </label>
          <div className="flex gap-2">
            <button onClick={() => handleSetLoop(null)} className={`px-3 py-1 text-sm rounded ${loopBeats === null ? 'bg-cap-amber text-black' : 'bg-plate-light text-label-dim'}`}>Off</button>
            <button onClick={() => handleSetLoop(1)} className={`px-3 py-1 text-sm rounded ${loopBeats === (meters?.players[0]?.meterNumerator || 4) * 1 ? 'bg-cap-amber text-black' : 'bg-plate-light text-label-dim'}`}>1 bar</button>
            <button onClick={() => handleSetLoop(2)} className={`px-3 py-1 text-sm rounded ${loopBeats === (meters?.players[0]?.meterNumerator || 4) * 2 ? 'bg-cap-amber text-black' : 'bg-plate-light text-label-dim'}`}>2 bars</button>
            <button onClick={() => handleSetLoop(4)} className={`px-3 py-1 text-sm rounded ${loopBeats === (meters?.players[0]?.meterNumerator || 4) * 4 ? 'bg-cap-amber text-black' : 'bg-plate-light text-label-dim'}`}>4 bars</button>
            <button onClick={() => handleSetLoop(8)} className={`px-3 py-1 text-sm rounded ${loopBeats === (meters?.players[0]?.meterNumerator || 4) * 8 ? 'bg-cap-amber text-black' : 'bg-plate-light text-label-dim'}`}>8 bars</button>
          </div>
        </div>
      )}

      {/* Two-deck mode */}
      {testMode === 'twodeck' && (
        <div className="mb-6 p-4 bg-plate-dark rounded-lg border border-plate-darker">
          <div className="flex gap-4 mb-3">
            <div className="flex-1">
              <h3 className="text-sm font-bold mb-2">Deck A</h3>
              <button onClick={() => audioEnginePause(0)} className="px-3 py-1 bg-plate-light rounded text-sm">❚❚ Pause A</button>
              <div className="text-xs text-label-dim mt-1">
                {(meters?.players?.[0]?.sourceBpm ?? 0) > 0 ? (
                  <span>BPM: <span className="text-label-cream">{meters!.players![0].sourceBpm.toFixed(2)}</span></span>
                ) : (
                  <span className="text-yellow-500">BPM: analyzing…</span>
                )}
              </div>
            </div>
            <div className="flex-1">
              <h3 className="text-sm font-bold mb-2">Deck B</h3>
              <button onClick={() => audioEnginePause(1)} className="px-3 py-1 bg-plate-light rounded text-sm">❚❚ Pause B</button>
              <div className="text-xs text-label-dim mt-1">
                {(meters?.players?.[1]?.sourceBpm ?? 0) > 0 ? (
                  <span>BPM: <span className="text-label-cream">{meters!.players![1].sourceBpm.toFixed(2)}</span></span>
                ) : (
                  <span className="text-yellow-500">BPM: analyzing…</span>
                )}
              </div>
            </div>
          </div>
          <div className="flex gap-2 flex-wrap mb-3">
            <button
              onClick={() => audioEngineSyncLaunch(0, 1)}
              className="px-3 py-2 bg-plate-lighter rounded text-sm"
              title="Start both at the same engine frame (engineering diagnostic)"
            >
              Same-Frame Start
            </button>
            <button
              onClick={() => audioEngineBeatSync(0, 1)}
              disabled={(meters?.players?.[0]?.sourceBpm ?? 0) <= 0 || (meters?.players?.[1]?.sourceBpm ?? 0) <= 0}
              className="px-4 py-2 bg-cap-amber text-black rounded text-sm font-medium disabled:opacity-40 disabled:cursor-not-allowed"
              title="Tempo-match B to A and align nearest beats (requires BPM on both decks)"
            >
              Beat Sync
            </button>
            <button
              onClick={() => audioEngineBarSync(0, 1)}
              disabled={(meters?.players?.[0]?.sourceBpm ?? 0) <= 0 || (meters?.players?.[1]?.sourceBpm ?? 0) <= 0}
              className="px-4 py-2 bg-cap-amber text-black rounded text-sm font-medium disabled:opacity-40 disabled:cursor-not-allowed"
              title="Tempo-match B to A and align downbeat/bar boundaries (requires BPM on both decks)"
            >
              Bar Sync
            </button>
          </div>
          <p className="text-xs text-label-dim mt-3">
            <strong>Same-Frame Start</strong> = engineering diagnostic (no tempo match).
            <strong> Beat Sync</strong> = tempo-match B→A + align nearest beats.
            <strong> Bar Sync</strong> = tempo-match + align downbeat/bar boundaries.
            Listen for drift or phasing over 30s, 1min, 2min+.
          </p>
        </div>
      )}

      {/* ABX mode */}
      {testMode === 'abx' && (
        <div className="mb-6 p-4 bg-plate-dark rounded-lg border border-plate-darker">
          {!abxHidden ? (
            <>
              <p className="text-sm text-label-dim mb-3">
                Blind A/B/X test. Each trial restarts from the cue position with
                the processor applied atomically. No hot-swapping — the switch
                itself won't reveal the answer.
              </p>
              <div className="mb-3">
                <label className="text-xs text-label-dim block mb-1">
                  CUE POSITION (seconds): {abxCueSec.toFixed(1)}
                </label>
                <input
                  type="range"
                  min={0}
                  max={300}
                  step={0.5}
                  value={abxCueSec}
                  onChange={(e) => setAbxCueSec(parseFloat(e.target.value))}
                  className="w-full"
                />
              </div>
              <button onClick={abxStart} className="px-4 py-2 bg-cap-amber text-black rounded text-sm font-medium">
                Start ABX Test
              </button>
            </>
          ) : (
            <>
              <p className="text-sm text-label-cream mb-3">
                Trial {abxTrials + 1}: Is X the original (A) or processed (B)?
              </p>
              <div className="flex gap-3 mb-3">
                <button onClick={() => abxReveal('a')} className="px-4 py-2 bg-plate-lighter rounded text-sm">
                  X is A (Original)
                </button>
                <button onClick={() => abxReveal('b')} className="px-4 py-2 bg-plate-lighter rounded text-sm">
                  X is B (Signalsmith)
                </button>
              </div>
              {abxTrials > 0 && (
                <p className="text-sm text-label-dim">
                  Score: {abxCorrect}/{abxTrials} correct ({((abxCorrect / abxTrials) * 100).toFixed(0)}%)
                  {abxLastAnswer && <span className="ml-3 text-cap-amber">{abxLastAnswer}</span>}
                </p>
              )}
              <button onClick={abxStop} className="mt-3 px-3 py-1 bg-plate-light rounded text-xs text-label-dim">
                End ABX Test
              </button>
            </>
          )}
        </div>
      )}

      {/* Current settings display */}
      <div className="mb-6 p-3 bg-plate-dark/50 rounded text-xs text-label-dim flex gap-6">
        <span>Tempo: <span className="text-label-cream">{tempo > 0 ? '+' : ''}{tempo}%</span></span>
        <span>Pitch: <span className="text-label-cream">{pitch > 0 ? '+' : ''}{pitch} st</span></span>
        <span>Processor: <span className="text-label-cream">{processor}</span></span>
      </div>

      {/* Quality ratings */}
      {(testMode === 'quality' || testMode === 'transport' || testMode === 'abx') && (
        <div className="mb-6 p-4 bg-plate-dark rounded-lg border border-plate-darker">
          <h3 className="text-sm font-bold mb-3">Quality Ratings (1–5)</h3>

          {/* Material selector */}
          <div className="mb-4">
            <label className="text-xs text-label-dim block mb-1">MATERIAL</label>
            <select
              value={material}
              onChange={(e) => setMaterial(e.target.value)}
              className="bg-plate-light text-label-cream text-sm rounded px-2 py-1 border border-plate-darker"
            >
              {MATERIAL_OPTIONS.map((m) => (
                <option key={m.value} value={m.value}>{m.label}</option>
              ))}
            </select>
          </div>

          <div className="grid grid-cols-3 gap-4 mb-4">
            {[
              ['Transients', transients, setTransients],
              ['Bass stability', bass, setBass],
              ['Vocals', vocals, setVocals],
              ['Stereo image', stereo, setStereo],
              ['Artifacts', artifacts, setArtifacts],
              ['Overall', overall, setOverall],
            ].map(([label, val, setter]: any) => (
              <div key={label}>
                <label className="text-xs text-label-dim block mb-1">{label}</label>
                <div className="flex gap-1">
                  {[1, 2, 3, 4, 5].map((n) => (
                    <button
                      key={n}
                      onClick={() => setter(n)}
                      className={`w-7 h-7 rounded text-xs ${val === n ? 'bg-cap-amber text-black' : 'bg-plate-light text-label-dim hover:text-label-cream'}`}
                    >
                      {n}
                    </button>
                  ))}
                </div>
              </div>
            ))}
          </div>

          <textarea
            value={notes}
            onChange={(e) => setNotes(e.target.value)}
            placeholder="Notes: e.g. 'Slight vocal texture on sustained vowels at +6%'"
            className="w-full bg-plate-light text-label-cream text-sm rounded px-3 py-2 border border-plate-darker mb-3"
            rows={2}
          />

          <div className="flex items-center gap-4">
            <button
              onClick={handleSave}
              className="px-4 py-2 bg-cap-amber text-black rounded text-sm font-medium"
            >
              Save Result
            </button>
            {saveStatus && <span className="text-sm text-label-dim">{saveStatus}</span>}
          </div>
        </div>
      )}

      {/* Saved results summary */}
      {savedResults.length > 0 && (
        <div className="p-4 bg-plate-dark rounded-lg border border-plate-darker">
          <h3 className="text-sm font-bold mb-3">
            Previous Results ({savedResults.length} total)
          </h3>
          <div className="max-h-48 overflow-y-auto text-xs">
            <table className="w-full text-left">
              <thead className="text-label-dim border-b border-plate-darker">
                <tr>
                  <th className="py-1 pr-3">Date</th>
                  <th className="py-1 pr-3">Proc</th>
                  <th className="py-1 pr-3">Tempo</th>
                  <th className="py-1 pr-3">Pitch</th>
                  <th className="py-1 pr-3">Mat</th>
                  <th className="py-1 pr-3">Overall</th>
                  <th className="py-1">ABX</th>
                </tr>
              </thead>
              <tbody>
                {savedResults.slice(0, 50).map((r) => (
                  <tr key={r.id} className="border-b border-plate-darker/50">
                    <td className="py-1 pr-3 text-label-dim">{r.timestamp.slice(0, 16)}</td>
                    <td className="py-1 pr-3">{r.processor.slice(0, 4)}</td>
                    <td className="py-1 pr-3">{r.tempoPercent > 0 ? '+' : ''}{r.tempoPercent}%</td>
                    <td className="py-1 pr-3">{r.pitchSemitones > 0 ? '+' : ''}{r.pitchSemitones}</td>
                    <td className="py-1 pr-3">{r.material.slice(0, 6)}</td>
                    <td className="py-1 pr-3">{r.overall}/5</td>
                    <td className="py-1">{r.abxTrials ? `${r.abxCorrect}/${r.abxTrials}` : '—'}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </div>
      )}
    </div>
  );
}
