import { create } from 'zustand';
import type { AudioMeterReadout } from '../lib/tauri';
import {
  DECK_IDS,
  PLAYER_BY_DECK,
  asEngineGeneration,
  asCommandId,
  asLoadGeneration,
  makeSourceId,
  type DeckId,
  type DeckCommandKind,
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
  setDeckCueEnabled: (deckId: DeckId, enabled: boolean) => void;
  setHeadphoneLevel: (level: number) => void;
  setCueMasterBlend: (blend: number) => void;
  setDeckError: (deckId: DeckId, message: string | null) => void;
  setDeckCommandPending: (deckId: DeckId, kind: DeckCommandKind, commandId: number) => void;
  acknowledgeDeckCommand: (deckId: DeckId, kind: DeckCommandKind, commandId: number) => void;
  failDeckCommand: (deckId: DeckId, kind: DeckCommandKind, commandId: number, message: string) => void;
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
    cueEnabled: false,
    error: null,
    pendingCommands: {},
    lastAppliedCommandId: null,
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
    headphoneLevel: 1,
    cueMasterBlend: 0,
    error: null,
  },
  decks: initialDecks,
  meters: null,

  setEngineInitializing: () => set((state) => ({
    engine: { ...state.engine, status: 'initializing', error: null },
  })),

  setEngineReady: (sampleRate, generation) => set((state) => ({
    engine: { ...state.engine, status: 'ready', sampleRate, generation, error: null },
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
                pendingCommands: {},
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
            pendingCommands: {},
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

  setDeckCueEnabled: (deckId, enabled) => set((state) => ({
    decks: {
      ...state.decks,
      [deckId]: { ...state.decks[deckId], cueEnabled: enabled },
    },
  })),

  setHeadphoneLevel: (level) => set((state) => ({
    engine: { ...state.engine, headphoneLevel: level },
  })),

  setCueMasterBlend: (blend) => set((state) => ({
    engine: { ...state.engine, cueMasterBlend: blend },
  })),

  setDeckError: (deckId, error) => set((state) => ({
    decks: {
      ...state.decks,
      [deckId]: { ...state.decks[deckId], error },
    },
  })),

  setDeckCommandPending: (deckId, kind, commandId) => set((state) => ({
    decks: {
      ...state.decks,
      [deckId]: {
        ...state.decks[deckId],
        pendingCommands: {
          ...state.decks[deckId].pendingCommands,
          [kind]: {
            id: asCommandId(commandId),
            kind,
            requestedAtMs: Date.now(),
          },
        },
        error: null,
      },
    },
  })),

  acknowledgeDeckCommand: (deckId, kind, commandId) => set((state) => {
    const deck = state.decks[deckId];
    if (Number(deck.pendingCommands[kind]?.id) !== commandId) return state;
    const pendingCommands = { ...deck.pendingCommands };
    delete pendingCommands[kind];
    return {
      decks: {
        ...state.decks,
        [deckId]: {
          ...deck,
          pendingCommands,
          lastAppliedCommandId: asCommandId(commandId),
          loadStatus: kind === 'load' ? 'ready' : deck.loadStatus,
          acknowledgedTransport: kind === 'transport'
            ? deck.desiredTransport
            : kind === 'load'
              ? 'paused'
              : deck.acknowledgedTransport,
          error: null,
        },
      },
    };
  }),

  failDeckCommand: (deckId, kind, commandId, message) => set((state) => {
    const deck = state.decks[deckId];
    if (Number(deck.pendingCommands[kind]?.id) !== commandId) return state;
    const pendingCommands = { ...deck.pendingCommands };
    delete pendingCommands[kind];
    return {
      decks: {
        ...state.decks,
        [deckId]: {
          ...deck,
          pendingCommands,
          loadStatus: kind === 'load' ? 'error' : deck.loadStatus,
          desiredTransport: kind === 'transport'
            ? deck.acknowledgedTransport
            : deck.desiredTransport,
          error: message,
        },
      },
    };
  }),

  reconcileMeters: (meters) => set((state) => ({
    meters,
    engine: {
      ...state.engine,
      generation: asEngineGeneration(meters.engineGeneration),
    },
    decks: Object.fromEntries(DECK_IDS.map((id) => {
      const deck = state.decks[id];
      const telemetry = meters.players[deck.playerId];
      const transportPending = deck.pendingCommands.load
        || deck.pendingCommands.transport;
      const acknowledgedTransport: TransportState = transportPending
        ? deck.acknowledgedTransport
        : !deck.source
          ? 'empty'
          : telemetry?.playing
            ? 'playing'
            : deck.desiredTransport === 'stopped'
              ? 'stopped'
              : 'paused';
      return [id, {
        ...deck,
        acknowledgedTransport,
        tempoRatio: deck.pendingCommands.tempo
          ? deck.tempoRatio
          : telemetry?.tempoRatio || deck.tempoRatio,
        pitchSemitones: !deck.pendingCommands.pitch
          && Number.isFinite(telemetry?.pitchSemitones)
          ? telemetry.pitchSemitones
          : deck.pitchSemitones,
      }];
    })) as Record<DeckId, DeckSessionState>,
  })),
}));
