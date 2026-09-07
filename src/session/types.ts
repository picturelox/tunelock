import type { AudioMeterReadout } from '../lib/tauri';

export type DeckId = 'A' | 'B' | 'C' | 'D';
export type PlayerId = 0 | 1 | 2 | 3;

declare const sourceIdBrand: unique symbol;
declare const loadGenerationBrand: unique symbol;
declare const engineGenerationBrand: unique symbol;
declare const analysisRevisionBrand: unique symbol;
declare const gridRevisionBrand: unique symbol;
declare const commandIdBrand: unique symbol;

export type SourceId = string & { readonly [sourceIdBrand]: true };
export type LoadGeneration = number & { readonly [loadGenerationBrand]: true };
export type EngineGeneration = number & { readonly [engineGenerationBrand]: true };
export type AnalysisRevision = number & { readonly [analysisRevisionBrand]: true };
export type GridRevision = number & { readonly [gridRevisionBrand]: true };
export type CommandId = number & { readonly [commandIdBrand]: true };

export type DeckCommandKind =
  | 'load'
  | 'transport'
  | 'seek'
  | 'loop'
  | 'tempo'
  | 'pitch'
  | 'gain'
  | 'cue'
  | 'sync';

export interface PendingDeckCommand {
  id: CommandId;
  kind: DeckCommandKind;
  requestedAtMs: number;
}

export const DECK_IDS: readonly DeckId[] = ['A', 'B', 'C', 'D'];

export const PLAYER_BY_DECK: Readonly<Record<DeckId, PlayerId>> = {
  A: 0,
  B: 1,
  C: 2,
  D: 3,
};

export type EngineStatus = 'idle' | 'initializing' | 'ready' | 'error';
export type DeckLoadStatus = 'empty' | 'loading' | 'ready' | 'error';
export type TransportState = 'empty' | 'paused' | 'playing' | 'stopped';

export interface SessionSource {
  id: SourceId;
  filePath: string;
  displayName: string;
  analysisRevision: AnalysisRevision | null;
  gridRevision: GridRevision | null;
}

export interface DeckSessionState {
  id: DeckId;
  playerId: PlayerId;
  source: SessionSource | null;
  loadGeneration: LoadGeneration;
  loadStatus: DeckLoadStatus;
  desiredTransport: TransportState;
  acknowledgedTransport: TransportState;
  tempoRatio: number;
  pitchSemitones: number;
  loopLengthBeats: number | null;
  cueEnabled: boolean;
  error: string | null;
  pendingCommands: Partial<Record<DeckCommandKind, PendingDeckCommand>>;
  lastAppliedCommandId: CommandId | null;
}

export interface EngineSessionState {
  status: EngineStatus;
  generation: EngineGeneration;
  sampleRate: number | null;
  headphoneLevel: number;
  cueMasterBlend: number;
  error: string | null;
}

export interface SessionSnapshot {
  engine: EngineSessionState;
  decks: Record<DeckId, DeckSessionState>;
  meters: AudioMeterReadout | null;
}

export function asEngineGeneration(value: number): EngineGeneration {
  return value as EngineGeneration;
}

export function asLoadGeneration(value: number): LoadGeneration {
  return value as LoadGeneration;
}

export function asCommandId(value: number): CommandId {
  return value as CommandId;
}

export function makeSourceId(
  deckId: DeckId,
  generation: LoadGeneration,
  filePath: string,
): SourceId {
  return `${deckId}:${generation}:${filePath}` as SourceId;
}
