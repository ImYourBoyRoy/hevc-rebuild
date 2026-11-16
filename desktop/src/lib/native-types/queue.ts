// The persisted library queue: sources, run states, interrupted-run recovery and output previews.

import type { ScanResult } from './media.js';
import type { RunMode } from './modes.js';
import type { QueueMetadataSuggestion, QueueMovieReview, QueueSeriesReview } from './reviews.js';

export interface MediaQueueSource {
  root: string;
  status: 'scanning' | 'ready' | 'failed' | 'interrupted';
  scanResult: ScanResult | null;
  error: string | null;
  updatedAtMs: number;
  changedFiles: string[];
  missingFiles: string[];
  excludedPaths: string[];
  entryOrder: string[];
}

export interface MediaQueueDocument {
  version: number;
  sources: MediaQueueSource[];
  selectedMediaPaths: string[];
  fileTuneOverrides: Record<string, 'animation' | 'none'>;
  fileStreamOverrides: Record<string, number[]>;
  movieReviews: Record<string, QueueMovieReview>;
  seriesReviews: Record<string, QueueSeriesReview>;
  matchSuggestions: Record<string, QueueMetadataSuggestion>;
  runStates: Record<string, MediaQueueRunState>;
  activeRoot: string | null;
  updatedAtMs: number;
}

export type MediaQueueRunStatus = 'queued' | 'running' | 'scanning' | 'renaming' | 'remuxing' | 'rebuilding' | 'validating' | 'renamed' | 'completed' | 'failed' | 'skipped' | 'interrupted' | 'needs_review';

export type MediaQueueOperationMode = RunMode | 'organize';

export interface InterruptedRecovery {
  skipped: boolean;
  completed: number;
  cleared: number;
  held: number;
  removedTemporaryFiles: number;
  runStates: Record<string, MediaQueueRunState>;
  summary: string;
}

export interface MediaQueueRunState {
  status: MediaQueueRunStatus;
  mode: MediaQueueOperationMode | null;
  message: string | null;
  outputPath: string | null;
  reportPath: string | null;
  reportStatus: string | null;
  updatedAtMs: number;
}

export interface PlannedOutputTarget {
  source: string;
  target: string;
  destination: 'source_folder' | 'default_folder' | 'per_file_folder';
  replacesSource: boolean;
  note: string | null;
}

export interface OutputCollision {
  kind: 'existing_output' | 'duplicate_target' | string;
  source: string;
  target: string;
  conflictsWith: string | null;
}

export interface OutputPreview {
  mode: 'metadata' | 'test' | 'full';
  sourceCount: number;
  fileCount: number;
  eligible: boolean;
  targets: PlannedOutputTarget[];
  omittedTargetCount: number;
  collisions: OutputCollision[];
  collisionCount: number;
  omittedCollisionCount: number;
  testOutputsMayReceiveUniqueSuffixes: boolean;
}

export interface MediaQueueLoadResult {
  queue: MediaQueueDocument;
  queueFile: string;
  warning: string | null;
}
