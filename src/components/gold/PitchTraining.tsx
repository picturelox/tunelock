import { useState, useCallback, useRef } from 'react';
import { Play, Check, X, RotateCcw } from 'lucide-react';
import type { TrainingSession } from '../../types';

const PITCH_CLASSES = ['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B'];

type ExerciseType = 'pitch_id' | 'mode_id';

interface PitchTrainingProps {
  onSaveSession: (session: TrainingSession) => Promise<number>;
  onStatsChanged: () => void;
}

export default function PitchTraining({ onSaveSession, onStatsChanged }: PitchTrainingProps) {
  const [exerciseType, setExerciseType] = useState<ExerciseType>('pitch_id');
  const [currentTonic] = useState(() => PITCH_CLASSES[Math.floor(Math.random() * 12)]);
  const [currentMode] = useState(() => (Math.random() < 0.5 ? 'major' : 'minor'));
  const [userAnswer, setUserAnswer] = useState<string | null>(null);
  const [feedback, setFeedback] = useState<'correct' | 'wrong' | null>(null);
  const [startTime] = useState(() => Date.now());
  const audioCtxRef = useRef<AudioContext | null>(null);

  // Get or create an AudioContext for playing reference tones
  const getAudioCtx = useCallback(() => {
    if (!audioCtxRef.current) {
      audioCtxRef.current = new AudioContext();
    }
    return audioCtxRef.current;
  }, []);

  // Play a pitch class as a sine wave at the correct octave
  const playTone = useCallback((pitchClass: string, mode?: string) => {
    const ctx = getAudioCtx();
    const now = ctx.currentTime;

    // Map pitch class to frequency (C4 = 261.63 Hz)
    const pcIndex = PITCH_CLASSES.indexOf(pitchClass);
    if (pcIndex < 0) return;
    const baseFreq = 261.63 * Math.pow(2, pcIndex / 12);

    // For mode exercises, play a triad (root, third, fifth)
    if (mode) {
      const intervals = mode === 'major' ? [0, 4, 7] : [0, 3, 7];
      intervals.forEach((semitones, i) => {
        const freq = baseFreq * Math.pow(2, semitones / 12);
        const osc = ctx.createOscillator();
        const gain = ctx.createGain();
        osc.frequency.value = freq;
        osc.type = 'sine';
        gain.gain.setValueAtTime(0, now + i * 0.15);
        gain.gain.linearRampToValueAtTime(0.3, now + i * 0.15 + 0.05);
        gain.gain.linearRampToValueAtTime(0, now + i * 0.15 + 1.5);
        osc.connect(gain);
        gain.connect(ctx.destination);
        osc.start(now + i * 0.15);
        osc.stop(now + i * 0.15 + 1.5);
      });
    } else {
      // Single tone
      const osc = ctx.createOscillator();
      const gain = ctx.createGain();
      osc.frequency.value = baseFreq;
      osc.type = 'sine';
      gain.gain.setValueAtTime(0, now);
      gain.gain.linearRampToValueAtTime(0.3, now + 0.05);
      gain.gain.linearRampToValueAtTime(0, now + 1.5);
      osc.connect(gain);
      gain.connect(ctx.destination);
      osc.start(now);
      osc.stop(now + 1.5);
    }
  }, [getAudioCtx]);

  // Play the current exercise
  const playExercise = useCallback(() => {
    if (exerciseType === 'pitch_id') {
      playTone(currentTonic);
    } else if (exerciseType === 'mode_id') {
      playTone(currentTonic, currentMode);
    }
  }, [exerciseType, currentTonic, currentMode, playTone]);

  const checkAnswer = useCallback(async (answer: string) => {
    const correct = exerciseType === 'pitch_id'
      ? answer === currentTonic
      : answer === currentMode;

    setUserAnswer(answer);
    setFeedback(correct ? 'correct' : 'wrong');

    const responseTime = (Date.now() - startTime) / 1000;

    try {
      await onSaveSession({
        sessionType: exerciseType,
        presentedTonic: currentTonic,
        presentedMode: exerciseType === 'mode_id' ? currentMode : undefined,
        userAnswer: answer,
        correct,
        responseTimeS: responseTime,
      });
      onStatsChanged();
    } catch (e) {
      console.error('Failed to save training session:', e);
    }
  }, [exerciseType, currentTonic, currentMode, startTime, onSaveSession, onStatsChanged]);

  const nextExercise = useCallback(() => {
    // Reload to get a new random exercise
    window.location.reload();
  }, []);

  return (
    <div className="max-w-2xl space-y-6">
      <div>
        <h2 className="text-xl font-bold text-text-primary mb-2">Ear Training</h2>
        <p className="text-sm text-text-secondary">
          Train your ear to identify pitch classes and modes. These skills
          are essential for accurately annotating the gold set.
        </p>
      </div>

      {/* Exercise type selector */}
      <div className="flex gap-2">
        <button
          onClick={() => setExerciseType('pitch_id')}
          className={`
            px-4 py-2 rounded-lg text-sm font-medium transition-colors
            ${exerciseType === 'pitch_id'
              ? 'bg-accent-primary text-white'
              : 'bg-surface text-text-secondary hover:text-text-primary'
            }
          `}
        >
          Pitch Identification
        </button>
        <button
          onClick={() => setExerciseType('mode_id')}
          className={`
            px-4 py-2 rounded-lg text-sm font-medium transition-colors
            ${exerciseType === 'mode_id'
              ? 'bg-accent-primary text-white'
              : 'bg-surface text-text-secondary hover:text-text-primary'
            }
          `}
        >
          Major/Minor Identification
        </button>
      </div>

      {/* Exercise area */}
      <div className="bg-surface rounded-lg p-8 border border-white/5">
        <div className="text-center mb-6">
          <p className="text-sm text-text-secondary mb-4">
            {exerciseType === 'pitch_id'
              ? 'Listen to the tone and identify the pitch class'
              : 'Listen to the triad and identify major or minor'}
          </p>
          <button
            onClick={playExercise}
            className="inline-flex items-center gap-2 px-6 py-3 bg-accent-primary text-white rounded-lg font-medium hover:bg-accent-primary/90 transition-colors"
          >
            <Play className="w-5 h-5" />
            Play {exerciseType === 'pitch_id' ? 'Tone' : 'Triad'}
          </button>
        </div>

        {/* Answer buttons */}
        {feedback === null && (
          <div className="space-y-3">
            {exerciseType === 'pitch_id' ? (
              <div className="grid grid-cols-6 gap-2">
                {PITCH_CLASSES.map((pc) => (
                  <button
                    key={pc}
                    onClick={() => checkAnswer(pc)}
                    disabled={!userAnswer}
                    className={`
                      py-3 rounded-lg text-sm font-medium transition-colors
                      ${userAnswer === pc
                        ? 'bg-accent-primary text-white'
                        : 'bg-background text-text-primary hover:bg-white/5'
                      }
                    `}
                  >
                    {pc}
                  </button>
                ))}
              </div>
            ) : (
              <div className="grid grid-cols-2 gap-3">
                <button
                  onClick={() => checkAnswer('major')}
                  className="py-4 rounded-lg text-sm font-medium bg-background text-text-primary hover:bg-white/5 transition-colors"
                >
                  Major
                </button>
                <button
                  onClick={() => checkAnswer('minor')}
                  className="py-4 rounded-lg text-sm font-medium bg-background text-text-primary hover:bg-white/5 transition-colors"
                >
                  Minor
                </button>
              </div>
            )}
          </div>
        )}

        {/* Feedback */}
        {feedback && (
          <div className="text-center space-y-4">
            <div className={`
              inline-flex items-center gap-2 px-4 py-2 rounded-lg font-medium
              ${feedback === 'correct'
                ? 'bg-green-500/20 text-green-400'
                : 'bg-red-500/20 text-red-400'
              }
            `}>
              {feedback === 'correct' ? <Check className="w-5 h-5" /> : <X className="w-5 h-5" />}
              {feedback === 'correct' ? 'Correct!' : 'Incorrect'}
            </div>
            <div className="text-sm text-text-secondary">
              {exerciseType === 'pitch_id' ? (
                <>The tone was <span className="text-text-primary font-bold">{currentTonic}</span></>
              ) : (
                <>The triad was <span className="text-text-primary font-bold">{currentMode}</span> ({currentTonic} {currentMode})</>
              )}
            </div>
            <button
              onClick={nextExercise}
              className="inline-flex items-center gap-2 px-4 py-2 bg-accent-primary text-white rounded-lg text-sm font-medium hover:bg-accent-primary/90 transition-colors"
            >
              <RotateCcw className="w-4 h-4" />
              Next Exercise
            </button>
          </div>
        )}
      </div>

      {/* Tips */}
      <div className="bg-surface rounded-lg p-4 border border-white/5">
        <h3 className="text-sm font-semibold text-text-primary mb-2">Tips</h3>
        <ul className="space-y-1 text-sm text-text-secondary">
          <li>• Use a reference pitch (e.g., A=440 Hz) to calibrate your ear before starting.</li>
          <li>• For pitch ID: hum the tone and match it to a known note on a piano.</li>
          <li>• For mode ID: major sounds "happy", minor sounds "sad". Listen to the third.</li>
          <li>• The more you practice, the better your gold set annotations will be.</li>
        </ul>
      </div>
    </div>
  );
}
