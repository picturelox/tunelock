// Core data models matching the Rust backend

export interface Track {
  id: number;
  file_path: string;
  filename: string;
  title: string | null;
  artist: string | null;
  album: string | null;
  duration_ms: number | null;
  // Analysis results
  key_standard: string | null;     // e.g., "A minor", "C major"
  key_camelot: string | null;        // e.g., "8A", "8B"
  key_confidence: number | null;     // 0.0 to 1.0
  bpm: number | null;                // e.g., 127.5
  energy_level: number | null;      // 1-10
  // Metadata
  file_format: string | null;
  file_size: number;
  sample_rate: number | null;
  bit_depth: number | null;
  analyzed_at: string | null;
  status: TrackStatus;
  /** Absolute filesystem path to the cached cover-art image (PNG/JPEG).
   *  Use Tauri's `convertFileSrc` before passing to an `<img>` tag. */
  artwork_path: string | null;
  created_at: string;
  updated_at: string;
}

export type TrackStatus = 'pending' | 'metadata_ready' | 'analyzing' | 'analyzed' | 'error';

export interface CuePoint {
  id: number;
  track_id: number;
  position_ms: number;
  name: string | null;
  color: string | null;
  hotcue_index: number | null; // 0-7
  created_at: string;
}

export interface Playlist {
  id: number;
  name: string;
  description: string | null;
  rules: PlaylistRules | null;
  created_at: string;
}

export interface PlaylistRules {
  sameKey: boolean;
  plusOne: boolean;
  minusOne: boolean;
  plusTwo: boolean;
  minusTwo: boolean;
  dominantToSubdominant: boolean; // A -> B (major to minor)
  subdominantToDominant: boolean; // B -> A (minor to major)
  energyCurve: 'build' | 'maintain' | 'wind_down' | 'peak_valley' | null;
}

export interface AnalysisProgress {
  total: number;
  completed: number;
  in_progress: number;
  speed_per_sec: number;
  eta_seconds: number;
}

export interface KeyCandidate {
  key_standard: string;
  key_camelot: string;
  confidence: number;
  agreement: number;     // fraction of segments voting for this candidate, 0..1
  avg_score: number;     // normalised profile-match score, 0..1
  segment_count: number; // raw winner count out of TrackAnalysis.section_count
}

export interface TunerTimings {
  decode_ms: number;
  spectrogram_ms: number;
  hpss_ms: number;
  chromagram_ms: number;
  ensemble_ms: number;
  tempo_ms: number;
  metadata_ms: number;
  total_ms: number;
}

export interface TunerProgress {
  stage: string;
  percent: number; // 0..1
}

/**
 * Tuner / batch analysis result.
 *
 * The Tuner path populates the diagnostic fields (`candidates`, `chroma`,
 * `timings`, `file_path`, `filename`). The legacy batch path leaves them
 * defaulted — those are optional on the wire.
 */
export interface TrackAnalysis {
  track_id: number;
  file_path?: string;
  filename?: string;
  key_standard: string;
  key_camelot: string;
  key_confidence: number;
  bpm: number;
  duration_ms: number;
  energy_level?: number;
  /** Tag-derived metadata, when present in the source file. */
  title?: string | null;
  artist?: string | null;
  album?: string | null;
  /** Absolute path to the cached cover-art image, if extracted. */
  artwork_path?: string | null;
  candidates?: KeyCandidate[];
  /** Valid temporal sections used for candidate section-vote evidence. */
  section_count?: number;
  /** 12-bin mean chroma vector. Order: C, C#, D, D#, E, F, F#, G, G#, A, A#, B. */
  chroma?: number[];
  timings?: TunerTimings;
}

export interface LibraryFilter {
  search?: string;
  artist?: string;
  key_camelot?: string;
  min_bpm?: number;
  max_bpm?: number;
  status?: TrackStatus;
  /** Smart filter preset: "unanalyzed", "low-confidence", "high-confidence".
   *  Applied server-side so it works across all 20k tracks. */
  smart_filter?: string;
}

export interface LibraryPage {
  tracks: Track[];
  total_count: number;
  page: number;
  page_size: number;
}

export interface CamelotPosition {
  number: number; // 1-12
  letter: 'A' | 'B';
}

export interface CamelotSegment {
  position: CamelotPosition;
  tracks: Track[];
  compatible_positions: CamelotPosition[];
}

export interface DeckState {
  track: Track | null;
  isPlaying: boolean;
  currentTime: number;
  duration: number;
  volume: number;
  cuePoints: CuePoint[];
  // EQ state
  eqLow: number;
  eqMid: number;
  eqHigh: number;
  eqLowKill: boolean;
  eqMidKill: boolean;
  eqHighKill: boolean;
}

export interface ExportOptions {
  write_tags: boolean;
  number_prefix: boolean;
  include_cues: boolean;
  dj_software_format: 'rekordbox' | 'serato' | 'traktor' | null;
}

export interface ValidationReport {
  total_tracks: number;
  matched_tracks: number;
  accuracy_percentage: number;
  per_method_accuracy: Record<string, number>;
  disagreements: ValidationDisagreement[];
}

export interface ValidationDisagreement {
  track_id: number;
  track_name: string;
  mik_key: string;
  our_key: string;
  our_confidence: number;
}

// ============================================================================
// Gold set annotation types (Step 6)
// ============================================================================

export interface GoldAnnotation {
  id?: number;
  trackId: number;
  keyTonic: string;        // 'C', 'C#', 'D', ... 'B'
  keyMode: string;         // 'major', 'minor', 'ambiguous', 'atonal'
  modulates: boolean;
  modulationNote?: string;
  annotatorConfidence: number;  // 1-5
  evidence?: string;
  annotatorId: string;     // 'self' or named annotator
  blind: boolean;
  createdAt?: string;
}

export interface GoldAnnotationSummary {
  totalTracks: number;
  annotatedTracks: number;
  totalAnnotations: number;
  selfAgreementPct: number | null;
  modeDistribution: Record<string, number>;
}

export interface TrainingSession {
  id?: number;
  sessionType: string;     // 'pitch_id', 'tonic_id', 'mode_id', 'full_key'
  trackId?: number;
  presentedTonic?: string;
  presentedMode?: string;
  userAnswer: string;
  correct: boolean;
  responseTimeS?: number;
  createdAt?: string;
}

export interface TrainingStats {
  totalSessions: number;
  correctCount: number;
  accuracyPct: number;
  byType: Record<string, [number, number]>;  // [total, correct]
}

// ============================================================================
// Assist layer types (Phase 11)
// ============================================================================

export interface OllamaModel {
  name: string;
  size: number | null;
}

export interface AssistStatus {
  available: boolean;
  ollamaUrl: string;
  models: OllamaModel[];
  selectedModel: string | null;
  enabled: boolean;
}

export interface ParsedTrack {
  position: number;
  artist: string;
  title: string;
  timestamp: string | null;
  keyHint: string | null;
}

export interface ParsedSetlist {
  setName: string | null;
  djName: string | null;
  tracks: ParsedTrack[];
}

export interface LocalMatch {
  trackId: number;
  filename: string;
  title: string | null;
  artist: string | null;
  keyCamelot: string | null;
  bpm: number | null;
  energyLevel: number | null;
  matchScore: number;
}

export interface MatchedTrack {
  parsed: ParsedTrack;
  localMatch: LocalMatch | null;
  harmonicFlow: string | null;
}

export interface SetlistSummary {
  totalTracks: number;
  matchedLocally: number;
  unmatched: number;
  keyFlow: string[];
  bpmRange: [number, number] | null;
  energyArc: (number | null)[];
  transitions: string[];
}

export interface SetlistAnalysis {
  parsed: ParsedSetlist;
  matchedTracks: MatchedTrack[];
  summary: SetlistSummary;
}

export interface MetadataProposal {
  trackId: number;
  filename: string;
  currentArtist: string | null;
  currentTitle: string | null;
  currentAlbum: string | null;
  currentGenre: string | null;
  proposedArtist: string | null;
  proposedTitle: string | null;
  proposedAlbum: string | null;
  proposedGenre: string | null;
  confidence: number;
  source: string;
}

export interface MetadataRepairBatch {
  proposals: MetadataProposal[];
  totalScanned: number;
  totalProposed: number;
}

export interface GenreInference {
  trackId: number;
  inferredGenre: string;
  confidence: number;
  reasoning: string | null;
}

export interface TransitionExplanation {
  fromKey: string;
  toKey: string;
  fromBpm: number | null;
  toBpm: number | null;
  explanation: string;
  source: string;
}

export interface SetPlan {
  description: string;
  trackIds: number[];
  reasoning: string;
}

// ============================================================================
// Transition Workbench types (Phase 7 / Slice A)
// ============================================================================

export interface BeatGrid {
  trackId: number;
  source: string; // "engine" | "manual" | "imported"
  bpm: number;
  firstBeatMs: number;
  meterNumerator: number;
  downbeatOffsetBeats: number;
  confidence: number | null;
  isOverride: boolean;
}

export interface TransitionPlan {
  playlistId: number;
  transitionId: string;
  schemaVersion: number;
  planJson: string;
}

export interface StemManifest {
  trackId: number;
  sourceFingerprint: string;
  provider: string;
  model: string;
  modelVersion: string | null;
  vocalsPath: string | null;
  drumsPath: string | null;
  bassPath: string | null;
  otherPath: string | null;
  durationMs: number | null;
  alignmentOffsetMs: number;
  status: string;
  storageBytes: number | null;
}
