// Tauri IPC wrapper with typed commands
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { 
  Track, 
  TrackAnalysis, 
  LibraryFilter, 
  LibraryPage,
  Playlist,
  PlaylistRules,
  CuePoint,
  ExportOptions,
  ValidationReport,
  AnalysisProgress
} from '../types';

// === Analysis Commands ===
export async function analyzeFile(path: string): Promise<TrackAnalysis> {
  return invoke('analyze_file', { path });
}

export async function analyzeBatch(paths: string[]): Promise<void> {
  return invoke('analyze_batch', { paths });
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

export async function importFolder(path: string): Promise<{ totalFiles: number; newFiles: number }> {
  return invoke('import_folder', { path });
}

export async function deleteTracks(ids: number[]): Promise<void> {
  return invoke('delete_tracks', { ids });
}

export async function scanFolder(path: string): Promise<{ totalFiles: number; newFiles: number; skipped: number }> {
  return invoke('scan_folder', { path });
}

export async function readMetadataBatch(trackIds: number[]): Promise<void> {
  return invoke('read_metadata_batch', { trackIds });
}

export async function setVisibleRange(startIdx: number, endIdx: number): Promise<void> {
  return invoke('set_visible_range', { startIdx, endIdx });
}

export async function prioritizeTracks(trackIds: number[]): Promise<void> {
  return invoke('prioritize_tracks', { trackIds });
}

// === Tag Commands ===
export async function writeTags(trackId: number): Promise<void> {
  return invoke('write_tags', { trackId });
}

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
  rules: PlaylistRules,
  maxLength: number
): Promise<Track[]> {
  return invoke('generate_playlist', { startTrackId, rules, maxLength });
}

export async function getCompatibleTracks(
  trackId: number,
  rules: PlaylistRules
): Promise<Track[]> {
  return invoke('get_compatible_tracks', { trackId, rules });
}

export async function savePlaylist(name: string, trackIds: number[], description?: string): Promise<Playlist> {
  return invoke('save_playlist', { name, trackIds, description });
}

export async function getPlaylists(): Promise<Playlist[]> {
  return invoke('get_playlists');
}

export async function deletePlaylist(id: number): Promise<void> {
  return invoke('delete_playlist', { id });
}

// === Cue Point Commands ===
export async function setCuePoint(
  trackId: number,
  positionMs: number,
  name?: string,
  color?: string,
  hotcueIndex?: number
): Promise<CuePoint> {
  return invoke('set_cue_point', { trackId, positionMs, name, color, hotcueIndex });
}

export async function deleteCuePoint(cueId: number): Promise<void> {
  return invoke('delete_cue_point', { cueId });
}

export async function getCuePoints(trackId: number): Promise<CuePoint[]> {
  return invoke('get_cue_points', { trackId });
}

// === Waveform Commands ===
export async function getWaveformData(path: string, numPoints: number): Promise<number[]> {
  return invoke('get_waveform_data', { path, numPoints });
}

// === Export Commands ===
export async function exportPlaylistFiles(
  playlistId: number,
  destination: string,
  options: ExportOptions
): Promise<{ exportedCount: number; errors: string[] }> {
  return invoke('export_playlist_files', { playlistId, destination, options });
}

/**
 * Non-destructive export (Phase 7).
 * Copies tracks to `targetDir`, optionally writing key/BPM/Camelot tags
 * into the copies and emitting an M3U8 playlist file.
 */
export async function exportTracks(
  trackIds: number[],
  targetDir: string,
  options: ExportOptions
): Promise<{ copied: number; failed: number; playlistPath: string | null }> {
  return invoke('export_tracks', { trackIds, targetDir, options });
}

// === Validation Commands ===
export async function runMikValidation(trackIds: number[]): Promise<ValidationReport> {
  return invoke('run_mik_validation', { trackIds });
}

export async function getValidationReport(): Promise<ValidationReport> {
  return invoke('get_validation_report');
}

export async function recalibrateEnsemble(): Promise<Record<string, number>> {
  return invoke('recalibrate_ensemble');
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
  callback: (progress: import('../types').TunerProgress) => void
): Promise<UnlistenFn> {
  return listen('tuner-progress', (event) =>
    callback(event.payload as import('../types').TunerProgress)
  );
}
