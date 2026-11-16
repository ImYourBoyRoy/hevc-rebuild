// ./desktop/src/lib/queue-document.ts
// Pure queue document construction and list edits, extracted from the route (queue slice A).
// Every function takes plain data and returns new data; the route still owns the reactive state
// and decides when to persist. No IPC, no DOM. Tested by pnpm test:queue-document.

import { moveQueueItem } from './workspace-state.ts';
import { pathIsWithin, queuePathKey } from './queue-paths.ts';
import type {
  MediaQueueDocument, MediaQueueRunState, MediaQueueSource, QueueArtworkCandidate, QueueMetadataSuggestion,
  QueueMovieReview, QueueSeriesReview, ScanResult,
} from './types.js';

export function uniquePosterChoices(candidates: QueueArtworkCandidate[]): QueueArtworkCandidate[] {
  const seen = new Set<string>();
  return candidates.filter((image) => {
    if (!image.id || image.id.length > 1024 || seen.has(image.id)) return false;
    seen.add(image.id);
    return true;
  });
}

export function scanResultWithQueueStatus(source: MediaQueueSource): ScanResult | null {
  if (!source.scanResult) return null;
  const missing = new Set(source.missingFiles);
  const changed = new Set(source.changedFiles);
  return {
    ...source.scanResult,
    entries: source.scanResult.entries.map((entry) => ({
      ...entry,
      queue_stale: missing.has(entry.path) ? "missing" : changed.has(entry.path) ? "changed" : undefined,
    })),
  };
}

export function normalizeQueueSource(source: MediaQueueSource): MediaQueueSource {
  return {
    ...source,
    changedFiles: source.changedFiles ?? [],
    missingFiles: source.missingFiles ?? [],
    excludedPaths: source.excludedPaths ?? [],
    entryOrder: source.entryOrder ?? [],
  };
}

export interface QueueDocumentParts {
  sources: MediaQueueSource[];
  selectedPaths: string[];
  fileTuneOverrides: Record<string, "animation" | "none">;
  fileStreamOverrides: Record<string, number[]>;
  movieReviews: Record<string, QueueMovieReview>;
  seriesReviews: Record<string, QueueSeriesReview>;
  matchSuggestions: Record<string, QueueMetadataSuggestion>;
  runStates: Record<string, MediaQueueRunState>;
  activeRoot: string | null;
}

export function buildQueueDocument(parts: QueueDocumentParts, nowMs: number = Date.now()): MediaQueueDocument {
  return {
    version: 1,
    sources: parts.sources,
    selectedMediaPaths: [...parts.selectedPaths],
    fileTuneOverrides: { ...parts.fileTuneOverrides },
    fileStreamOverrides: { ...parts.fileStreamOverrides },
    movieReviews: Object.fromEntries(Object.entries(parts.movieReviews).map(([path, review]) => [
      path,
      { ...review, artworkCandidates: uniquePosterChoices(review.artworkCandidates) },
    ])),
    seriesReviews: Object.fromEntries(Object.entries(parts.seriesReviews).map(([path, review]) => [
      path,
      { ...review, artworkCandidates: uniquePosterChoices(review.artworkCandidates) },
    ])),
    matchSuggestions: { ...parts.matchSuggestions },
    runStates: { ...parts.runStates },
    activeRoot: parts.activeRoot,
    updatedAtMs: nowMs,
  };
}

/** Returns the reordered sources, or null when the move is a no-op (unknown root or same position). */
export function reorderSources(sources: MediaQueueSource[], fromRoot: string, toRoot: string): MediaQueueSource[] | null {
  const from = sources.findIndex((source) => source.root === fromRoot);
  const to = sources.findIndex((source) => source.root === toRoot);
  if (from < 0 || to < 0 || from === to) return null;
  return moveQueueItem(sources, from, to);
}

export function reorderSourceEntries(sources: MediaQueueSource[], sourceRoot: string, fromPath: string, toPath: string): MediaQueueSource[] {
  return sources.map((source) => {
    if (source.root !== sourceRoot || !source.scanResult) return source;
    const from = source.scanResult.entries.findIndex((entry) => entry.path === fromPath);
    const to = source.scanResult.entries.findIndex((entry) => entry.path === toPath);
    const entries = moveQueueItem(source.scanResult.entries, from, to);
    return {
      ...source,
      scanResult: { ...source.scanResult, entries, candidate_count: entries.length },
      entryOrder: entries.map((entry) => entry.path),
    };
  });
}

export function removeEntryFromSources(sources: MediaQueueSource[], path: string): { sources: MediaQueueSource[]; removed: boolean } {
  const key = queuePathKey(path);
  let removed = false;
  const next = sources.map((source) => {
    if (!source.scanResult?.entries.some((entry) => queuePathKey(entry.path) === key)) return source;
    removed = true;
    const entries = source.scanResult.entries.filter((entry) => queuePathKey(entry.path) !== key);
    return {
      ...source,
      scanResult: { ...source.scanResult, entries, candidate_count: entries.length },
      excludedPaths: [...new Set([...(source.excludedPaths ?? []), path])],
      entryOrder: entries.map((entry) => entry.path),
    };
  });
  return { sources: next, removed };
}

export function restoreExcludedFiles(sources: MediaQueueSource[], sourceRoot: string): MediaQueueSource[] {
  return sources.map((source) => source.root === sourceRoot ? { ...source, excludedPaths: [] } : source);
}

export interface QueuePerFileRecords {
  selectedPaths: string[];
  fileTuneOverrides: Record<string, "animation" | "none">;
  fileStreamOverrides: Record<string, number[]>;
  runStates: Record<string, MediaQueueRunState>;
  matchSuggestions: Record<string, QueueMetadataSuggestion>;
  movieReviews: Record<string, QueueMovieReview>;
}

function filterRecord<T>(record: Record<string, T>, keep: (path: string) => boolean): Record<string, T> {
  return Object.fromEntries(Object.entries(record).filter(([path]) => keep(path)));
}

/** Drops one file from every per-file record. Series reviews are keyed by folder and are left alone. */
export function withoutQueuePath(path: string, records: QueuePerFileRecords): QueuePerFileRecords {
  const key = queuePathKey(path);
  const keep = (candidate: string) => queuePathKey(candidate) !== key;
  return {
    selectedPaths: records.selectedPaths.filter(keep),
    fileTuneOverrides: filterRecord(records.fileTuneOverrides, keep),
    fileStreamOverrides: filterRecord(records.fileStreamOverrides, keep),
    runStates: filterRecord(records.runStates, keep),
    matchSuggestions: filterRecord(records.matchSuggestions, keep),
    movieReviews: filterRecord(records.movieReviews, keep),
  };
}

/** After a source is removed, keeps only records whose files are still queued. `remainingSources` excludes the removed one. */
export function pruneAfterSourceRemoval(
  removedRoot: string,
  remainingSources: MediaQueueSource[],
  records: QueuePerFileRecords & { seriesReviews: Record<string, QueueSeriesReview> },
): QueuePerFileRecords & { seriesReviews: Record<string, QueueSeriesReview> } {
  const remainingPaths = new Set(remainingSources.flatMap((source) => source.scanResult?.entries.map((entry) => entry.path) ?? []));
  const keep = (path: string) => remainingPaths.has(path);
  return {
    selectedPaths: records.selectedPaths.filter(keep),
    fileTuneOverrides: filterRecord(records.fileTuneOverrides, keep),
    fileStreamOverrides: filterRecord(records.fileStreamOverrides, keep),
    runStates: filterRecord(records.runStates, keep),
    matchSuggestions: filterRecord(records.matchSuggestions, keep),
    movieReviews: filterRecord(records.movieReviews, keep),
    seriesReviews: Object.fromEntries(Object.entries(records.seriesReviews).filter(([seriesRoot, review]) => {
      const hasRemainingEpisode = Object.keys(review.episodeAssignments).some((path) => remainingPaths.has(path));
      const hasRemainingSeriesMedia = [...remainingPaths].some((path) => pathIsWithin(seriesRoot, path));
      const belongedToRemovedSource = pathIsWithin(removedRoot, seriesRoot)
        || pathIsWithin(seriesRoot, removedRoot)
        || Object.keys(review.episodeAssignments).some((path) => pathIsWithin(removedRoot, path));
      return hasRemainingEpisode || hasRemainingSeriesMedia || !belongedToRemovedSource;
    })),
  };
}
