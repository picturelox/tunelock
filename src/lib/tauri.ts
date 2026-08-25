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
  GoldAnnotation,
  GoldAnnotationSummary,
  TrainingSession,
  TrainingStats,
  AssistStatus,
  SetlistAnalysis,
  MetadataProposal,
  MetadataRepairBatch,
  GenreInference,
  TransitionExplanation,
  SetPlan,
  BeatGrid,
  TransitionPlan,
  StemManifest,
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

// === Mix Persistence (Loose End — Mix Canvas survives restarts) ===

export interface SavedMix {
  id: number;
  name: string;
  description: string | null;
  trackIds: number[];
  clipNotes: (string | null)[];
  createdAt: string;
}

export async function saveMix(
  id: number | null,
  name: string,
  description: string | null,
  trackIds: number[],
  clipNotes: [number, string][],
): Promise<number> {
  return invoke('save_mix', { id, name, description, trackIds, clipNotes });
}

export async function loadMix(playlistId: number): Promise<SavedMix> {
  return invoke('load_mix', { playlistId });
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

// === Key Timeline Commands ===
export interface KeySegment {
  startSec: number;
  endSec: number;
  keyStandard: string;
  keyCamelot: string;
  confidence: number;
}

export interface KeyTimeline {
  segments: KeySegment[];
  globalKeyStandard: string;
  globalKeyCamelot: string;
  globalConfidence: number;
  abstained: boolean;
  modulates: boolean;
  modulationSummary: string;
}

export async function getKeyTimeline(trackId: number): Promise<KeyTimeline> {
  return invoke('get_key_timeline', { trackId });
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

// === Gold Set Annotation Commands (Step 6) ===

export async function saveGoldAnnotation(annotation: GoldAnnotation): Promise<number> {
  return invoke('save_gold_annotation', { annotation });
}

export async function getGoldAnnotations(trackId: number): Promise<GoldAnnotation[]> {
  return invoke('get_gold_annotations', { trackId });
}

export async function getGoldAnnotationSummary(): Promise<GoldAnnotationSummary> {
  return invoke('get_gold_annotation_summary');
}

export async function saveTrainingSession(session: TrainingSession): Promise<number> {
  return invoke('save_training_session', { session });
}

export async function getTrainingStats(): Promise<TrainingStats> {
  return invoke('get_training_stats');
}

// === Assist Layer Commands (Phase 11) ===

export async function assistStatus(): Promise<AssistStatus> {
  return invoke('assist_status');
}

export async function assistSetEnabled(enabled: boolean): Promise<void> {
  return invoke('assist_set_enabled', { enabled });
}

export async function assistSetModel(model: string): Promise<void> {
  return invoke('assist_set_model', { model });
}

export async function assistAnalyzeSetlist(rawText: string): Promise<SetlistAnalysis> {
  return invoke('assist_analyze_setlist', { rawText });
}

export async function assistRepairMetadata(): Promise<MetadataRepairBatch> {
  return invoke('assist_repair_metadata');
}

export async function assistApplyMetadataRepair(proposal: MetadataProposal): Promise<void> {
  return invoke('assist_apply_metadata_repair', { proposal });
}

export async function assistInferGenres(): Promise<GenreInference[]> {
  return invoke('assist_infer_genres');
}

export async function assistExplainTransition(
  fromKey: string,
  toKey: string,
  fromBpm: number | null,
  toBpm: number | null,
): Promise<TransitionExplanation> {
  return invoke('assist_explain_transition', { fromKey, toKey, fromBpm, toBpm });
}

export async function assistPlanSet(instruction: string): Promise<SetPlan> {
  return invoke('assist_plan_set', { instruction });
}

// === Transition Workbench Commands (Phase 7 / Slice A) ===

export async function getBeatGrid(trackId: number): Promise<BeatGrid | null> {
  return invoke('get_beat_grid', { trackId });
}

export async function saveBeatGridOverride(
  trackId: number,
  bpm: number,
  firstBeatMs: number,
  meterNumerator: number,
  downbeatOffsetBeats: number,
): Promise<void> {
  return invoke('save_beat_grid_override', {
    trackId, bpm, firstBeatMs, meterNumerator, downbeatOffsetBeats,
  });
}

export async function resetBeatGridOverride(trackId: number): Promise<void> {
  return invoke('reset_beat_grid_override', { trackId });
}

export async function getTransitionPlan(
  playlistId: number,
  transitionId: string,
): Promise<TransitionPlan | null> {
  return invoke('get_transition_plan', { playlistId, transitionId });
}

export async function saveTransitionPlan(plan: TransitionPlan): Promise<void> {
  return invoke('save_transition_plan', { plan });
}

export async function getStemManifest(trackId: number): Promise<StemManifest | null> {
  return invoke('get_stem_manifest', { trackId });
}

// === Audio Engine Commands (Transition Workbench — real-time playback) ===
//
// Generalized player/bus vocabulary:
//   - PlayerId: u8 (0..7), eight player slots (MAX_PLAYERS=8)
//   - BusId: 'a' | 'b' | 'master'
//   - Players 0 and 1 correspond to the Transition Workbench's A and B decks
//   - The Layer Lab exposes all eight slots

export interface PlayerMeterEntry {
  playing: boolean;
  positionSec: number;
  rms: number;
  peak: number;
  clip: boolean;
}

export interface AudioMeterReadout {
  playing: boolean;
  currentFrame: number;
  players: PlayerMeterEntry[]; // length 8
  busARms: number;
  busAPeak: number;
  busBRms: number;
  busBPeak: number;
  masterRms: number;
  masterPeak: number;
  masterTruePeak: number;
  masterClip: boolean;
  crossfadePosition: number;
  underruns: number;
  commandsDropped: number;
}

export async function audioEngineInit(): Promise<number> {
  return invoke('audio_engine_init');
}

export async function audioEnginePlay(player: number): Promise<void> {
  return invoke('audio_engine_play', { player });
}

export async function audioEnginePause(player: number): Promise<void> {
  return invoke('audio_engine_pause', { player });
}

export async function audioEngineStop(player: number): Promise<void> {
  return invoke('audio_engine_stop', { player });
}

export async function audioEngineSeek(player: number, sourceBeat: number): Promise<void> {
  return invoke('audio_engine_seek', { player, sourceBeat });
}

export async function audioEngineSetCrossfade(position: number): Promise<void> {
  return invoke('audio_engine_set_crossfade', { position });
}

export async function audioEngineSetTempo(player: number, rate: number): Promise<void> {
  return invoke('audio_engine_set_tempo', { player, rate });
}

export async function audioEngineSetPitch(player: number, semitones: number): Promise<void> {
  return invoke('audio_engine_set_pitch', { player, semitones });
}

export async function audioEngineSetPlayerGain(player: number, gain: number): Promise<void> {
  return invoke('audio_engine_set_player_gain', { player, gain });
}

export async function audioEngineSetPan(player: number, pan: number): Promise<void> {
  return invoke('audio_engine_set_pan', { player, pan });
}

export async function audioEngineSetMute(player: number, muted: boolean): Promise<void> {
  return invoke('audio_engine_set_mute', { player, muted });
}

export async function audioEngineSetSolo(player: number, soloed: boolean): Promise<void> {
  return invoke('audio_engine_set_solo', { player, soloed });
}

export async function audioEngineSetBus(player: number, bus: 'a' | 'b' | 'master'): Promise<void> {
  return invoke('audio_engine_set_bus', { player, bus });
}

export async function audioEngineSetEq(player: number, band: 'low' | 'mid' | 'high', gainDb: number): Promise<void> {
  return invoke('audio_engine_set_eq', { player, band, gainDb });
}

export async function audioEngineSetEqKill(player: number, band: 'low' | 'mid' | 'high', killed: boolean): Promise<void> {
  return invoke('audio_engine_set_eq_kill', { player, band, killed });
}

export async function audioEngineSetLoop(player: number, startBeat: number | null, lengthBeats: number | null): Promise<void> {
  return invoke('audio_engine_set_loop', { player, startBeat, lengthBeats });
}

export async function audioEngineLoadPlayer(player: number, filePath: string): Promise<void> {
  return invoke('audio_engine_load_player', { player, filePath });
}

export async function audioEngineSetMasterGain(gain: number): Promise<void> {
  return invoke('audio_engine_set_master_gain', { gain });
}

export async function audioEngineSetBusGain(bus: 'a' | 'b' | 'master', gain: number): Promise<void> {
  return invoke('audio_engine_set_bus_gain', { bus, gain });
}

export async function audioEngineGetMeters(): Promise<AudioMeterReadout> {
  return invoke('audio_engine_get_meters');
}

// === Beat-Grid DSP Commands ===

export interface BeatMarker {
  sourceFrame: number;
  beatNumber: number;
  isDownbeat: boolean;
  confidence: number;
}

export interface TempoSegment {
  startSourceFrame: number;
  startBeat: number;
  bpm: number;
}

export interface BeatGridDetectionResult {
  bpm: number;
  firstBeatMs: number;
  beatTimesMs: number[];
  beatMarkers: BeatMarker[];
  downbeatOffset: number;
  downbeatConfidence: number;
  meterNumerator: number;
  confidence: number;
  tempoSegments: TempoSegment[];
}

export async function detectBeatGrid(trackId: number): Promise<BeatGridDetectionResult> {
  return invoke('detect_beat_grid', { trackId });
}

// === PB-2 Listening Lab ===

export interface ListeningLabProcessorInfo {
  processorType: string;
  latencyFrames: number;
  sampleRate: number;
}

export interface ListeningLabResult {
  id?: number;
  timestamp: string;
  processor: string;
  tempoPercent: number;
  pitchSemitones: number;
  material: string;
  trackName?: string;
  transients: number;
  bass: number;
  vocals: number;
  stereo: number;
  artifacts: number;
  overall: number;
  abxCorrect?: number;
  abxTrials?: number;
  notes?: string;
}

export async function listeningLabGetProcessorInfo(): Promise<ListeningLabProcessorInfo> {
  return invoke('listening_lab_get_processor_info');
}

export async function listeningLabSaveResult(result: ListeningLabResult): Promise<number> {
  return invoke('listening_lab_save_result', { result });
}

export async function listeningLabGetResults(): Promise<ListeningLabResult[]> {
  return invoke('listening_lab_get_results');
}
