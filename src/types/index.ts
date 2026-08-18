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
  segment_count: number; // raw count out of 8 segments
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
