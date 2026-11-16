// ./desktop/src/lib/queue-persistence.ts
// Durable queue writes, extracted from the route (queue slice A): one serialized write chain,
// the full-document save and the targeted saves. The native `invoke` and every piece of route
// state are injected, so the controller is unit-tested with a fake `invoke`
// (pnpm test:queue-persistence). Nothing is written in browser preview or before the queue loaded.

import { buildQueueDocument } from './queue-document.ts';
import type { QueueState } from './workspace-state.ts';
import type {
  MediaQueueDocument, MediaQueueRunState, QueueMetadataSuggestion, QueueMovieReview, QueueSeriesReview,
} from './types.js';

export interface QueuePersistenceDeps {
  invoke: (command: string, args?: Record<string, unknown>) => Promise<unknown>;
  isDesktop: () => boolean;
  queue: QueueState;
  getFileTuneOverrides: () => Record<string, "animation" | "none">;
  getFileStreamOverrides: () => Record<string, number[]>;
  getMovieReviews: () => Record<string, QueueMovieReview>;
  getSeriesReviews: () => Record<string, QueueSeriesReview>;
  getMatchSuggestions: () => Record<string, QueueMetadataSuggestion>;
}

export function createQueuePersistence(deps: QueuePersistenceDeps) {
  const { invoke, queue } = deps;
  let queueWriteChain: Promise<unknown> = Promise.resolve();
  let metadataSuggestionSaveTimer: ReturnType<typeof setTimeout> | null = null;

  function queueDocument(): MediaQueueDocument {
    return buildQueueDocument({
      sources: queue.sources,
      selectedPaths: queue.selectedPaths,
      fileTuneOverrides: deps.getFileTuneOverrides(),
      fileStreamOverrides: deps.getFileStreamOverrides(),
      movieReviews: deps.getMovieReviews(),
      seriesReviews: deps.getSeriesReviews(),
      matchSuggestions: deps.getMatchSuggestions(),
      runStates: queue.runStates,
      activeRoot: queue.activeRoot,
    });
  }

  function enqueueQueueWrite(write: () => Promise<void>) {
    const operation = queueWriteChain.catch(() => {}).then(write);
    queueWriteChain = operation.then(() => {}, () => {});
    return operation;
  }

  function persistQueueSources(): Promise<boolean> {
    if (!deps.isDesktop() || !queue.loaded) return Promise.resolve(false);
    const operation = enqueueQueueWrite(async () => {
      await invoke("save_library_queue", { queue: queueDocument() });
    });
    return operation.then(() => {
      queue.persistenceError = "";
      return true;
    }, (error) => {
      queue.persistenceError = String(error);
      return false;
    });
  }

  function persistQueueRunStates(updates: Record<string, MediaQueueRunState>): Promise<boolean> {
    queue.runStates = { ...queue.runStates, ...updates };
    return persistQueueSources();
  }

  function persistQueueSelection(
    selected = [...queue.selectedPaths],
    active = queue.activeRoot,
    tunes: Record<string, "animation" | "none"> = JSON.parse(JSON.stringify(deps.getFileTuneOverrides())) as Record<string, "animation" | "none">,
    streams: Record<string, number[]> = JSON.parse(JSON.stringify(deps.getFileStreamOverrides())) as Record<string, number[]>,
  ): Promise<void> {
    if (!deps.isDesktop() || !queue.loaded) return Promise.resolve();
    return enqueueQueueWrite(async () => {
      await invoke("save_library_queue_selection", {
        selectedMediaPaths: selected,
        activeRoot: active,
        fileTuneOverrides: tunes,
        fileStreamOverrides: streams,
      });
    }).then(() => {
      queue.persistenceError = "";
    }).catch((error) => {
      queue.persistenceError = String(error);
    });
  }

  function persistQueueMovieReviews(reviews: Record<string, QueueMovieReview>): Promise<void> {
    if (!deps.isDesktop() || !queue.loaded) return Promise.resolve();
    return enqueueQueueWrite(async () => {
      await invoke("save_library_queue_movie_reviews", { movieReviews: reviews });
    }).then(() => {
      queue.persistenceError = "";
    }).catch((error) => {
      queue.persistenceError = String(error);
    });
  }

  function persistQueueSeriesReviews(reviews: Record<string, QueueSeriesReview>): Promise<void> {
    if (!deps.isDesktop() || !queue.loaded) return Promise.resolve();
    return enqueueQueueWrite(async () => {
      await invoke("save_library_queue_series_reviews", { seriesReviews: reviews });
    }).then(() => {
      queue.persistenceError = "";
    }).catch((error) => {
      queue.persistenceError = String(error);
    });
  }

  function persistQueueMetadataSuggestions(suggestions: Record<string, QueueMetadataSuggestion> = deps.getMatchSuggestions()): Promise<void> {
    if (!deps.isDesktop() || !queue.loaded) return Promise.resolve();
    if (metadataSuggestionSaveTimer) clearTimeout(metadataSuggestionSaveTimer);
    metadataSuggestionSaveTimer = null;
    const snapshot = JSON.parse(JSON.stringify(suggestions)) as Record<string, QueueMetadataSuggestion>;
    return enqueueQueueWrite(async () => {
      await invoke("save_library_queue_metadata_suggestions", { matchSuggestions: snapshot });
    }).then(() => {
      queue.persistenceError = "";
    }).catch((error) => {
      queue.persistenceError = String(error);
    });
  }

  function scheduleQueueMetadataSuggestionSave() {
    if (!deps.isDesktop() || !queue.loaded) return;
    if (metadataSuggestionSaveTimer) clearTimeout(metadataSuggestionSaveTimer);
    metadataSuggestionSaveTimer = setTimeout(() => {
      metadataSuggestionSaveTimer = null;
      void persistQueueMetadataSuggestions();
    }, 3000);
  }

  return {
    queueDocument, persistQueueSources, persistQueueRunStates, persistQueueSelection,
    persistQueueMovieReviews, persistQueueSeriesReviews, persistQueueMetadataSuggestions, scheduleQueueMetadataSuggestionSave,
  };
}
