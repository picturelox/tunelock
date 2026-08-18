import type { RelationshipType } from '../lib/relationships';

export interface MixClip {
  id: string;
  trackId: number;
  position: number; // order index in the timeline
  notes?: string;
}

export interface MixTransition {
  id: string;
  fromClipId: string;
  toClipId: string;
  relationshipType: RelationshipType;
  score: number;
  label: string;
  explanation: string;
  risk: 'low' | 'medium' | 'high';
  bpmDeltaPercent: number;
}

export interface MixProject {
  id: string;
  name: string;
  clips: MixClip[];
  transitions: MixTransition[];
  createdAt: string;
  updatedAt: string;
  selectedClipId: string | null;
  selectedTransitionId: string | null;
}

export type MixViewPanel = 'library' | 'candidates' | 'inspector';
