import { create } from 'zustand';
import type { AudioMeterReadout } from '../lib/tauri';
import {
  DECK_IDS,
  PLAYER_BY_DECK,
  asEngineGeneration,
  asLoadGeneration,
  makeSourceId,
  type DeckId,
  type DeckSessionState,
  type EngineGeneration,
  type SessionSnapshot,
  type TransportState,
} from './types';

interface SessionActions {
  setEngineInitializing: () => void;
  setEngineReady: (sampleRate: number, generation: EngineGeneration) => void;
  setEngineError: (message: string) => void;
  beginDeckLoad: (deckId: DeckId, filePath: string, displayName: string) => number;
  completeDeckLoad: (deckId: DeckId, generation: number) => void;
  failDeckLoad: (deckId: DeckId, generation: number, message: string) => void;
  setDesiredTransport: (deckId: DeckId, transport: TransportState) => void;
  setDeckControl: (
    deckId: DeckId,
    control: 'tempoRatio' | 'pitchSemitones' | 'loopLengthBeats',
    value: number | null,
  ) => void;
  setDeckError: (deckId: DeckId, message: string | null) => void;
  reconcileMeters: (meters: AudioMeterReadout) => void;
}

export type SessionStore = SessionSnapshot & SessionActions;

function emptyDeck(id: DeckId): DeckSessionState {
  return {
    id,
    playerId: PLAYER_BY_DECK[id],
    source: null,
    loadGeneration: asLoadGeneration(0),
    loadStatus: 'empty',
    desiredTransport: 'empty',
    acknowledgedTransport: 'empty',
    tempoRatio: 1,
    pitchSemitones: 0,
    loopLengthBeats: null,
    error: null,
  };
}

const initialDecks = Object.fromEntries(
  DECK_IDS.map((id) => [id, emptyDeck(id)]),
) as Record<DeckId, DeckSessionState>;

export const useSessionStore = create<SessionStore>((set) => ({
  engine: {
    status: 'idle',
    generation: asEngineGeneration(0),
    sampleRate: null,
    error: null,
  },
  decks: initialDecks,
  meters: null,

  setEngineInitializing: () => set((state) => ({
    engine: { ...state.engine, status: 'initializing', error: null },
  })),

  setEngineReady: (sampleRate, generation) => set((state) => ({
    engine: { status: 'ready', sampleRate, generation, error: null },
    // A replaced engine owns no sources from the prior registry. Preserve the
    // user's selected sources but state that playback must be loaded again.
    decks: generation === state.engine.generation
      ? state.decks
      : Object.fromEntries(DECK_IDS.map((id) => {
          const deck = state.decks[id];
          return [id, deck.source
            ? {
                ...deck,
                loadStatus: 'empty',
                desiredTransport: 'stopped',
                acknowledgedTransport: 'stopped',
                error: 'Audio device changed; reload this deck.',
              }
            : deck];
        })) as Record<DeckId, DeckSessionState>,
  })),

  setEngineError: (message) => set((state) => ({
    engine: { ...state.engine, status: 'error', error: message },
  })),

  beginDeckLoad: (deckId, filePath, displayName) => {
    let nextGeneration = 0;
    set((state) => {
      const deck = state.decks[deckId];
      nextGeneration = Number(deck.loadGeneration) + 1;
      const loadGeneration = asLoadGeneration(nextGeneration);
      return {
        decks: {
          ...state.decks,
          [deckId]: {
            ...deck,
            source: {
              id: makeSourceId(deckId, loadGeneration, filePath),
              filePath,
              displayName,
              analysisRevision: null,
              gridRevision: null,
            },
            loadGeneration,
            loadStatus: 'loading',
            desiredTransport: 'paused',
            acknowledgedTransport: 'empty',
            error: null,
          },
        },
      };
    });
    return nextGeneration;
  },

  completeDeckLoad: (deckId, generation) => set((state) => {
    const deck = state.decks[deckId];
    if (Number(deck.loadGeneration) !== generation) return state;
    return {
      decks: {
        ...state.decks,
        [deckId]: {
          ...deck,
          loadStatus: 'ready',
          desiredTransport: 'paused',
          error: null,
        },
      },
    };
  }),

  failDeckLoad: (deckId, generation, message) => set((state) => {
    const deck = state.decks[deckId];
    if (Number(deck.loadGeneration) !== generation) return state;
    return {
      decks: {
        ...state.decks,
        [deckId]: { ...deck, loadStatus: 'error', error: message },
      },
    };
  }),

  setDesiredTransport: (deckId, desiredTransport) => set((state) => ({
    decks: {
      ...state.decks,
      [deckId]: { ...state.decks[deckId], desiredTransport, error: null },
    },
  })),

  setDeckControl: (deckId, control, value) => set((state) => ({
    decks: {
      ...state.decks,
      [deckId]: { ...state.decks[deckId], [control]: value },
    },
  })),

  setDeckError: (deckId, error) => set((state) => ({
    decks: {
      ...state.decks,
      [deckId]: { ...state.decks[deckId], error },
    },
  })),

  reconcileMeters: (meters) => set((state) => ({
    meters,
    engine: {
      ...state.engine,
      generation: asEngineGeneration(meters.engineGeneration),
    },
    decks: Object.fromEntries(DECK_IDS.map((id) => {
      const deck = state.decks[id];
      const telemetry = meters.players[deck.playerId];
      const acknowledgedTransport: TransportState = !deck.source
        ? 'empty'
        : telemetry?.playing
          ? 'playing'
          : deck.desiredTransport === 'stopped'
            ? 'stopped'
            : 'paused';
      return [id, {
        ...deck,
        acknowledgedTransport,
        tempoRatio: telemetry?.tempoRatio || deck.tempoRatio,
        pitchSemitones: Number.isFinite(telemetry?.pitchSemitones)
          ? telemetry.pitchSemitones
          : deck.pitchSemitones,
      }];
    })) as Record<DeckId, DeckSessionState>,
  })),
}));
