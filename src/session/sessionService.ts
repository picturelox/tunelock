import {
  audioEngineBarSync,
  audioEngineBeatSync,
  audioEngineGetMeters,
  audioEngineInit,
  audioEngineLoadPlayerPaused,
  audioEnginePause,
  audioEnginePlay,
  audioEngineSeek,
  audioEngineSetBus,
  audioEngineSetCueEnabled,
  audioEngineSetCueGain,
  audioEngineSetCueMasterBlend,
  audioEngineSetLoop,
  audioEngineSetLoudnessMatchGain,
  audioEngineSetMasterGain,
  audioEngineSetPitch,
  audioEngineSetTempo,
  audioEngineStop,
  audioEngineSyncLaunch,
  type AudioCommandSubmission,
} from '../lib/tauri';
import { useSessionStore } from './sessionStore';
import {
  DECK_IDS,
  PLAYER_BY_DECK,
  asEngineGeneration,
  type DeckId,
  type DeckCommandKind,
  type TransportState,
} from './types';

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

class SessionService {
  private initialization: Promise<void> | null = null;
  private configuredGeneration = 0;
  private pendingAcknowledgements = new Map<string, {
    deckId: DeckId;
    kind: DeckCommandKind;
    commandId: number;
    resolve: () => void;
    reject: (error: Error) => void;
    timer: ReturnType<typeof setTimeout>;
  }>();
  private observedAcknowledgements = new Set<string>();
  private acknowledgementsDropped = 0;

  private acknowledgementKey(engineGeneration: number, commandId: number): string {
    return `${engineGeneration}:${commandId}`;
  }

  private waitForApplication(
    deckId: DeckId,
    kind: DeckCommandKind,
    submission: AudioCommandSubmission,
  ): Promise<void> {
    const key = this.acknowledgementKey(
      submission.engineGeneration,
      submission.commandId,
    );
    useSessionStore.getState().setDeckCommandPending(
      deckId,
      kind,
      submission.commandId,
    );
    if (this.observedAcknowledgements.delete(key)) {
      useSessionStore.getState().acknowledgeDeckCommand(
        deckId,
        kind,
        submission.commandId,
      );
      return Promise.resolve();
    }

    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pendingAcknowledgements.delete(key);
        const error = new Error(
          `Audio command ${submission.commandId} was not acknowledged within 2 seconds.`,
        );
        useSessionStore.getState().failDeckCommand(
          deckId,
          kind,
          submission.commandId,
          error.message,
        );
        reject(error);
      }, 2_000);
      this.pendingAcknowledgements.set(key, {
        deckId,
        kind,
        commandId: submission.commandId,
        resolve,
        reject,
        timer,
      });
    });
  }

  private rejectPendingAcknowledgements(message: string): void {
    for (const pending of this.pendingAcknowledgements.values()) {
      clearTimeout(pending.timer);
      useSessionStore.getState().failDeckCommand(
        pending.deckId,
        pending.kind,
        pending.commandId,
        message,
      );
      pending.reject(new Error(message));
    }
    this.pendingAcknowledgements.clear();
    this.observedAcknowledgements.clear();
  }

  async initialize(): Promise<void> {
    const current = useSessionStore.getState().engine;
    if (
      current.status === 'ready'
      && this.configuredGeneration === Number(current.generation)
    ) return;
    if (this.initialization) return this.initialization;

    useSessionStore.getState().setEngineInitializing();
    this.initialization = (async () => {
      try {
        const result = await audioEngineInit();
        if (this.configuredGeneration !== result.engineGeneration) {
          await audioEngineSetMasterGain(1);
          await Promise.all([
            audioEngineSetBus(PLAYER_BY_DECK.A, 'master'),
            audioEngineSetBus(PLAYER_BY_DECK.B, 'master'),
            audioEngineSetBus(PLAYER_BY_DECK.C, 'master'),
            audioEngineSetBus(PLAYER_BY_DECK.D, 'master'),
          ]);
          const { headphoneLevel, cueMasterBlend } = useSessionStore.getState().engine;
          await audioEngineSetCueGain(headphoneLevel);
          await audioEngineSetCueMasterBlend(cueMasterBlend);
          for (const deckId of DECK_IDS) {
            if (useSessionStore.getState().decks[deckId].cueEnabled) {
              await audioEngineSetCueEnabled(PLAYER_BY_DECK[deckId], true);
            }
          }
          this.configuredGeneration = result.engineGeneration;
        }
        useSessionStore.getState().setEngineReady(
          result.sampleRate,
          asEngineGeneration(result.engineGeneration),
        );
      } catch (error) {
        useSessionStore.getState().setEngineError(errorMessage(error));
        throw error;
      } finally {
        this.initialization = null;
      }
    })();

    return this.initialization;
  }

  async loadDeck(deckId: DeckId, filePath: string, displayName: string): Promise<void> {
    const generation = useSessionStore.getState()
      .beginDeckLoad(deckId, filePath, displayName);
    try {
      await this.initialize();
      const result = await audioEngineLoadPlayerPaused(
        PLAYER_BY_DECK[deckId],
        filePath,
        Number(generation),
      );
      if (!result.installed) {
        const currentGeneration = useSessionStore.getState().decks[deckId].loadGeneration;
        if (currentGeneration === generation) {
          throw new Error('The audio engine changed while the track was loading; load it again.');
        }
        return;
      }
      if (result.commandId === null) {
        throw new Error('The audio engine accepted a load without a command identity.');
      }
      await this.waitForApplication(deckId, 'load', {
        commandId: result.commandId,
        engineGeneration: result.engineGeneration,
        queuedFrame: 0,
      });
      useSessionStore.getState().completeDeckLoad(deckId, generation);
    } catch (error) {
      useSessionStore.getState().failDeckLoad(deckId, generation, errorMessage(error));
      throw error;
    }
  }

  async setTransport(deckId: DeckId, transport: TransportState): Promise<void> {
    const store = useSessionStore.getState();
    const previousTransport = store.decks[deckId].acknowledgedTransport;
    store.setDesiredTransport(deckId, transport);
    try {
      await this.initialize();
      const player = PLAYER_BY_DECK[deckId];
      let submission: AudioCommandSubmission;
      if (transport === 'playing') submission = await audioEnginePlay(player);
      else if (transport === 'paused') submission = await audioEnginePause(player);
      else if (transport === 'stopped') submission = await audioEngineStop(player);
      else return;
      await this.waitForApplication(deckId, 'transport', submission);
    } catch (error) {
      if (!useSessionStore.getState().decks[deckId].pendingCommands.transport) {
        useSessionStore.getState().setDesiredTransport(deckId, previousTransport);
      }
      useSessionStore.getState().setDeckError(deckId, errorMessage(error));
      throw error;
    }
  }

  async seek(deckId: DeckId, sourceBeat: number): Promise<void> {
    await this.initialize();
    const submission = await audioEngineSeek(PLAYER_BY_DECK[deckId], sourceBeat);
    await this.waitForApplication(deckId, 'seek', submission);
  }

  async setTempo(deckId: DeckId, ratio: number): Promise<void> {
    const previous = useSessionStore.getState().decks[deckId].tempoRatio;
    useSessionStore.getState().setDeckControl(deckId, 'tempoRatio', ratio);
    try {
      await this.initialize();
      const submission = await audioEngineSetTempo(PLAYER_BY_DECK[deckId], ratio);
      await this.waitForApplication(deckId, 'tempo', submission);
    } catch (error) {
      useSessionStore.getState().setDeckControl(deckId, 'tempoRatio', previous);
      useSessionStore.getState().setDeckError(deckId, errorMessage(error));
      throw error;
    }
  }

  async setPitch(deckId: DeckId, semitones: number): Promise<void> {
    const previous = useSessionStore.getState().decks[deckId].pitchSemitones;
    useSessionStore.getState().setDeckControl(deckId, 'pitchSemitones', semitones);
    try {
      await this.initialize();
      const submission = await audioEngineSetPitch(PLAYER_BY_DECK[deckId], semitones);
      await this.waitForApplication(deckId, 'pitch', submission);
    } catch (error) {
      useSessionStore.getState().setDeckControl(deckId, 'pitchSemitones', previous);
      useSessionStore.getState().setDeckError(deckId, errorMessage(error));
      throw error;
    }
  }

  async setLoop(deckId: DeckId, lengthBeats: number | null): Promise<void> {
    const previous = useSessionStore.getState().decks[deckId].loopLengthBeats;
    useSessionStore.getState().setDeckControl(deckId, 'loopLengthBeats', lengthBeats);
    try {
      await this.initialize();
      const submission = await audioEngineSetLoop(
        PLAYER_BY_DECK[deckId],
        lengthBeats === null ? null : 0,
        lengthBeats,
      );
      await this.waitForApplication(deckId, 'loop', submission);
    } catch (error) {
      useSessionStore.getState().setDeckControl(deckId, 'loopLengthBeats', previous);
      useSessionStore.getState().setDeckError(deckId, errorMessage(error));
      throw error;
    }
  }

  async setLoudnessMatchGain(deckId: DeckId, gain: number): Promise<void> {
    await this.initialize();
    const submission = await audioEngineSetLoudnessMatchGain(
      PLAYER_BY_DECK[deckId],
      gain,
    );
    await this.waitForApplication(deckId, 'gain', submission);
  }

  async setCueEnabled(deckId: DeckId, enabled: boolean): Promise<void> {
    const previous = useSessionStore.getState().decks[deckId].cueEnabled;
    useSessionStore.getState().setDeckCueEnabled(deckId, enabled);
    try {
      await this.initialize();
      const submission = await audioEngineSetCueEnabled(
        PLAYER_BY_DECK[deckId],
        enabled,
      );
      await this.waitForApplication(deckId, 'cue', submission);
    } catch (error) {
      useSessionStore.getState().setDeckCueEnabled(deckId, previous);
      useSessionStore.getState().setDeckError(deckId, errorMessage(error));
      throw error;
    }
  }

  async setHeadphoneLevel(level: number): Promise<void> {
    const previous = useSessionStore.getState().engine.headphoneLevel;
    useSessionStore.getState().setHeadphoneLevel(level);
    try {
      await this.initialize();
      await audioEngineSetCueGain(level);
    } catch (error) {
      useSessionStore.getState().setHeadphoneLevel(previous);
      throw error;
    }
  }

  async setCueMasterBlend(blend: number): Promise<void> {
    const previous = useSessionStore.getState().engine.cueMasterBlend;
    useSessionStore.getState().setCueMasterBlend(blend);
    try {
      await this.initialize();
      await audioEngineSetCueMasterBlend(blend);
    } catch (error) {
      useSessionStore.getState().setCueMasterBlend(previous);
      throw error;
    }
  }

  async beatSync(leader: DeckId, follower: DeckId): Promise<void> {
    await this.initialize();
    await audioEngineBeatSync(PLAYER_BY_DECK[leader], PLAYER_BY_DECK[follower]);
  }

  async syncLaunch(first: DeckId, second: DeckId): Promise<void> {
    await this.initialize();
    useSessionStore.getState().setDesiredTransport(first, 'playing');
    useSessionStore.getState().setDesiredTransport(second, 'playing');
    await audioEngineSyncLaunch(PLAYER_BY_DECK[first], PLAYER_BY_DECK[second]);
  }

  async barSync(leader: DeckId, follower: DeckId): Promise<void> {
    await this.initialize();
    await audioEngineBarSync(PLAYER_BY_DECK[leader], PLAYER_BY_DECK[follower]);
  }

  async pollMeters(): Promise<void> {
    const meters = await audioEngineGetMeters();
    const state = useSessionStore.getState();
    if (state.engine.generation !== meters.engineGeneration) {
      this.rejectPendingAcknowledgements(
        'Audio engine changed before the command was applied.',
      );
      state.setEngineReady(
        state.engine.sampleRate ?? 0,
        asEngineGeneration(meters.engineGeneration),
      );
      // Reapply generation-scoped defaults before accepting more deck work.
      // Device replacement creates a fresh registry and mixer state.
      await this.initialize();
    }
    useSessionStore.getState().reconcileMeters(meters);

    if (meters.acknowledgementsDropped > this.acknowledgementsDropped) {
      this.rejectPendingAcknowledgements(
        'Audio acknowledgement capacity was exceeded; state was reconciled from telemetry.',
      );
    }
    this.acknowledgementsDropped = meters.acknowledgementsDropped;

    for (const acknowledgement of meters.acknowledgements) {
      const key = this.acknowledgementKey(
        meters.engineGeneration,
        acknowledgement.commandId,
      );
      const pending = this.pendingAcknowledgements.get(key);
      if (!pending) {
        this.observedAcknowledgements.add(key);
        if (this.observedAcknowledgements.size > 512) {
          const oldest = this.observedAcknowledgements.values().next().value;
          if (oldest !== undefined) this.observedAcknowledgements.delete(oldest);
        }
        continue;
      }
      clearTimeout(pending.timer);
      this.pendingAcknowledgements.delete(key);
      useSessionStore.getState().acknowledgeDeckCommand(
        pending.deckId,
        pending.kind,
        pending.commandId,
      );
      pending.resolve();
    }
  }
}

export const sessionService = new SessionService();
