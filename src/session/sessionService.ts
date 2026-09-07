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
  audioEngineSetLoop,
  audioEngineSetLoudnessMatchGain,
  audioEngineSetMasterGain,
  audioEngineSetPitch,
  audioEngineSetTempo,
  audioEngineStop,
  audioEngineSyncLaunch,
} from '../lib/tauri';
import { useSessionStore } from './sessionStore';
import {
  PLAYER_BY_DECK,
  asEngineGeneration,
  type DeckId,
  type TransportState,
} from './types';

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

class SessionService {
  private initialization: Promise<void> | null = null;
  private configuredGeneration = 0;

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
      useSessionStore.getState().completeDeckLoad(deckId, generation);
    } catch (error) {
      useSessionStore.getState().failDeckLoad(deckId, generation, errorMessage(error));
      throw error;
    }
  }

  async setTransport(deckId: DeckId, transport: TransportState): Promise<void> {
    const store = useSessionStore.getState();
    store.setDesiredTransport(deckId, transport);
    try {
      await this.initialize();
      const player = PLAYER_BY_DECK[deckId];
      if (transport === 'playing') await audioEnginePlay(player);
      else if (transport === 'paused') await audioEnginePause(player);
      else if (transport === 'stopped') await audioEngineStop(player);
    } catch (error) {
      useSessionStore.getState().setDeckError(deckId, errorMessage(error));
      throw error;
    }
  }

  async seek(deckId: DeckId, sourceBeat: number): Promise<void> {
    await this.initialize();
    await audioEngineSeek(PLAYER_BY_DECK[deckId], sourceBeat);
  }

  async setTempo(deckId: DeckId, ratio: number): Promise<void> {
    useSessionStore.getState().setDeckControl(deckId, 'tempoRatio', ratio);
    await this.initialize();
    await audioEngineSetTempo(PLAYER_BY_DECK[deckId], ratio);
  }

  async setPitch(deckId: DeckId, semitones: number): Promise<void> {
    useSessionStore.getState().setDeckControl(deckId, 'pitchSemitones', semitones);
    await this.initialize();
    await audioEngineSetPitch(PLAYER_BY_DECK[deckId], semitones);
  }

  async setLoop(deckId: DeckId, lengthBeats: number | null): Promise<void> {
    useSessionStore.getState().setDeckControl(deckId, 'loopLengthBeats', lengthBeats);
    await this.initialize();
    await audioEngineSetLoop(
      PLAYER_BY_DECK[deckId],
      lengthBeats === null ? null : 0,
      lengthBeats,
    );
  }

  async setLoudnessMatchGain(deckId: DeckId, gain: number): Promise<void> {
    await this.initialize();
    await audioEngineSetLoudnessMatchGain(PLAYER_BY_DECK[deckId], gain);
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
      state.setEngineReady(
        state.engine.sampleRate ?? 0,
        asEngineGeneration(meters.engineGeneration),
      );
      // Reapply generation-scoped defaults before accepting more deck work.
      // Device replacement creates a fresh registry and mixer state.
      await this.initialize();
    }
    useSessionStore.getState().reconcileMeters(meters);
  }
}

export const sessionService = new SessionService();
