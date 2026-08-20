// Tauri IPC wrapper with typed commands.
//
// Every invoke() here MUST resolve to a registered Rust command in
// src-tauri/src/lib.rs generate_handler!([...]). No phantom wrappers.
//
// Commands that are not yet implemented (playlists, cue points, waveforms,
// validation, tag writing, batch metadata, visible-range hints, priority
// queues, track deletion) are intentionally absent. They will be added in
// later phases alongside their Rust implementations.
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type {
  Track,
  TrackAnalysis,
  LibraryFilter,
  LibraryPage,
  ExportOptions,
  AnalysisProgress,
  TunerProgress,
  Playlist,
} from '../types';

// === Analysis Commands ===
export async function analyzeFile(path: string): Promise<TrackAnalysis> {
  return invoke('analyze_file', { path });
}

export async function startAnalysis(): Promise<void> {
  return invoke('start_analysis');
}

export async function pauseAnalysis(): Promise<void> {
  return invoke('pause_analysis');
}

export async function resumeAnalysis(): Promise<void> {
  return invoke('resume_analysis');
}

export async function cancelAnalysis(): Promise<void> {
  return invoke('cancel_analysis');
}

export async function getAnalysisStatus(): Promise<AnalysisProgress> {
  return invoke('get_analysis_status');
}

// === Library Commands ===
export async function getLibraryPage(
  page: number,
  pageSize: number,
  sortBy: string,
  sortDir: 'asc' | 'desc',
  filter?: LibraryFilter
): Promise<LibraryPage> {
  return invoke('get_library_page', { page, pageSize, sortBy, sortDir, filter });
}

export async function scanFolder(path: string): Promise<{ totalFiles: number; newFiles: number; skipped: number }> {
  return invoke('scan_folder', { path });
}

// === Metadata Commands ===
export async function readFileMetadata(path: string): Promise<{
  title?: string;
  artist?: string;
  album?: string;
  durationMs?: number;
}> {
  return invoke('read_file_metadata', { path });
}

// === Playlist Commands ===
export async function generatePlaylist(
  startTrackId: number,
  rules: unknown,
  maxLength: number
): Promise<Track[]> {
  return invoke('generate_playlist', { startTrackId, rules, maxLength });
}

export async function getCompatibleTracks(
  trackId: number,
  rules: unknown
): Promise<Track[]> {
  return invoke('get_compatible_tracks', { trackId, rules });
}

export async function savePlaylist(
  name: string,
  trackIds: number[],
  description?: string
): Promise<Playlist> {
  return invoke('save_playlist', { name, trackIds, description });
}

export async function getPlaylists(): Promise<Playlist[]> {
  return invoke('get_playlists');
}

export async function deletePlaylist(id: number): Promise<void> {
  return invoke('delete_playlist', { id });
}

export async function getPlaylistTracks(playlistId: number): Promise<Track[]> {
  return invoke('get_playlist_tracks', { playlistId });
}

// === MIK CSV Import ===
export interface MikImportResult {
  totalRows: number;
  matched: number;
  unmatched: number;
  errors: string[];
}

export async function importMikCsv(csvPath: string): Promise<MikImportResult> {
  return invoke('import_mik_csv', { csvPath });
}

// === Consensus Commands ===
export interface TrackOpinion {
  id: number;
  trackId: number;
  source: 'tunelock' | 'mik' | 'traktor' | 'acoustid';
  keyCamelot: string | null;
  keyStandard: string | null;
  bpm: number | null;
  energy: number | null;
  confidence: number;
  provenance: string;
  createdAt: string;
}

export interface ConsensusResult {
  sourceCount: number;
  keyAgreement: number;
  bpmAgreement: number;
  consensusKey: string | null;
  consensusBpm: number | null;
  status: 'agreed' | 'contested' | 'single' | 'unknown';
  opinions: TrackOpinion[];
}

export async function getConsensus(trackId: number): Promise<ConsensusResult> {
  return invoke('get_consensus', { trackId });
}

export async function getConsensusBatch(
  trackIds: number[]
): Promise<Record<number, ConsensusResult>> {
  return invoke('get_consensus_batch', { trackIds });
}

export async function getContestedTracks(limit?: number): Promise<number[]> {
  return invoke('get_contested_tracks', { limit });
}

export async function setTrackOpinion(
  trackId: number,
  source: string,
  keyCamelot?: string | null,
  keyStandard?: string | null,
  bpm?: number | null,
  energy?: number | null,
  confidence?: number,
  provenance?: string
): Promise<void> {
  return invoke('set_track_opinion', {
    trackId,
    source,
    keyCamelot: keyCamelot ?? null,
    keyStandard: keyStandard ?? null,
    bpm: bpm ?? null,
    energy: energy ?? null,
    confidence: confidence ?? 1.0,
    provenance: provenance ?? 'manual',
  });
}

export interface NmlImportResult {
  totalEntries: number;
  matched: number;
  unmatched: number;
  errors: string[];
}

export async function importTraktorNml(nmlPath: string): Promise<NmlImportResult> {
  return invoke('import_traktor_nml', { nmlPath });
}

// === Waveform Commands ===
export interface WaveformColumn {
  low: number;
  mid: number;
  high: number;
}

export interface WaveformData {
  columns: WaveformColumn[];
  sampleRate: number;
  durationMs: number;
}

export async function getWaveformData(trackId: number): Promise<WaveformData> {
  return invoke('get_waveform_data', { trackId });
}

// === Export Commands ===
export async function exportTracks(
  trackIds: number[],
  targetDir: string,
  options: ExportOptions
): Promise<{ copied: number; failed: number; playlistPath: string | null }> {
  return invoke('export_tracks', { trackIds, targetDir, options });
}

// === Event Listeners ===
export function onTrackAnalyzed(callback: (track: Track) => void): Promise<UnlistenFn> {
  return listen('track-analyzed', (event) => callback(event.payload as Track));
}

export function onMetadataBatchComplete(callback: (tracks: Track[]) => void): Promise<UnlistenFn> {
  return listen('metadata-batch-complete', (event) => callback(event.payload as Track[]));
}

export function onAnalysisProgress(callback: (progress: AnalysisProgress) => void): Promise<UnlistenFn> {
  return listen('analysis-progress', (event) => callback(event.payload as AnalysisProgress));
}

export function onExportProgress(callback: (progress: { current: number; total: number; path: string }) => void): Promise<UnlistenFn> {
  return listen('export-progress', (event) => callback(event.payload as { current: number; total: number; path: string }));
}

/**
 * Subscribe to per-stage progress emitted by `analyze_file` (Tuner Mode).
 * Payload: `{ stage, percent }` where percent is 0..1.
 */
export function onTunerProgress(
  callback: (progress: TunerProgress) => void
): Promise<UnlistenFn> {
  return listen('tuner-progress', (event) =>
    callback(event.payload as TunerProgress)
  );
}
