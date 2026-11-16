// ./desktop/src/lib/queue-controller.ts
// Queue controller extracted from the route (queue slice B): scanning sources, activating and
// removing them, reordering, and restoring the saved queue. The route still owns the reactive
// state; it is reached through one typed `QueueHost` object of accessors, and the native bridge,
// the desktop check and the persistence layer are injected. That makes every branch testable with
// a plain fake host and a fake `invoke` (pnpm test:queue-controller). No DOM.

import { applyQueueFilePolicy } from './workspace-state.ts';
import type { WorkspaceState } from './workspace-state.ts';
import { pathIsWithin, queuePathKey } from './queue-paths.ts';
import {
  normalizeQueueSource, pruneAfterSourceRemoval, removeEntryFromSources, reorderSourceEntries, reorderSources,
  restoreExcludedFiles, scanResultWithQueueStatus, withoutQueuePath,
} from './queue-document.ts';
import { makeQueueRunState } from './queue-run-state.ts';
import type { createQueuePersistence } from './queue-persistence.ts';
import type { AppPreferences } from './ui-contracts.js';
import type {
  Environment, MediaQueueLoadResult, MediaQueueSource, MetadataLookupStatus, Page, QueueMetadataSuggestion, QueueMovieReview,
  QueueSeriesReview, RunMode, ScanProgress, ScanResult, Settings, ToolPaths, WorkDirectoryPreview,
} from './types.js';

/** Everything the queue controller reads or writes on the route. Property names match the route's variables. */
export interface QueueHost {
  root: string;
  scanResult: ScanResult | null;
  settings: Settings | null;
  settingsRoot: string;
  settingsBaseline: string;
  librarySettingsBaseline: string;
  settingsSources: string[];
  workDirectory: WorkDirectoryPreview | null;
  environment: Environment | null;
  recursive: boolean;
  minimumSizeMib: string;
  readonly stopOnError: boolean;
  mode: RunMode;
  readonly page: Page;
  scanning: boolean;
  scanProgress: ScanProgress | null;
  metadataLookupClaimed: boolean;
  metadataLookupPending: boolean;
  metadataLookupStatus: MetadataLookupStatus | null;
  search: string;
  mediaPage: number;
  libraryBrowse: { seriesRoot: string; season: number | null } | null;
  fileTuneOverrides: Record<string, "animation" | "none">;
  fileStreamOverrides: Record<string, number[]>;
  queueMatchSuggestions: Record<string, QueueMetadataSuggestion>;
  queueMovieReviews: Record<string, QueueMovieReview>;
  queueSeriesReviews: Record<string, QueueSeriesReview>;
  readonly appPreferences: AppPreferences;
  readonly settingsChanged: boolean;
  readonly workflowBusy: boolean;
  readonly repairId: string;
  readonly hdrInstalling: boolean;
  setNotice(message: string, kind?: "info" | "success" | "error"): void;
  persistPreferences(patch: Partial<AppPreferences>): Promise<boolean>;
  openPage(page: Page): void;
  currentToolPaths(): ToolPaths;
  loadSettings(folder: string): Promise<void>;
  sourceNeedsMetadataLookup(source: MediaQueueSource): boolean;
  recoverInterruptedSession(): Promise<void>;
}

export interface QueueControllerDeps {
  invoke: <T>(command: string, args?: Record<string, unknown>) => Promise<T>;
  isDesktop: () => boolean;
  workspace: WorkspaceState;
  host: QueueHost;
  persistence: Pick<ReturnType<typeof createQueuePersistence>, 'persistQueueSources'>;
}

export function createQueueController(deps: QueueControllerDeps) {
  const { invoke, workspace, host: h, persistence } = deps;

  function syncActiveScanResult() {
    const active = workspace.queue.sources.find((source) => source.root === workspace.queue.activeRoot);
    h.scanResult = active ? scanResultWithQueueStatus(active) : (workspace.queue.activeRoot ? null : h.scanResult);
  }

  function pruneQueuePath(path: string) {
    const next = withoutQueuePath(path, {
      selectedPaths: workspace.queue.selectedPaths,
      fileTuneOverrides: h.fileTuneOverrides,
      fileStreamOverrides: h.fileStreamOverrides,
      runStates: workspace.queue.runStates,
      matchSuggestions: h.queueMatchSuggestions,
      movieReviews: h.queueMovieReviews,
    });
    workspace.queue.selectedPaths = next.selectedPaths;
    h.fileTuneOverrides = next.fileTuneOverrides;
    h.fileStreamOverrides = next.fileStreamOverrides;
    workspace.queue.runStates = next.runStates;
    h.queueMatchSuggestions = next.matchSuggestions;
    h.queueMovieReviews = next.movieReviews;
  }

  function activateQueueSource(sourceRoot: string) {
    const source = workspace.queue.sources.find((item) => item.root === sourceRoot);
    if (!source) return;
    if (workspace.queue.activeRoot !== source.root) h.metadataLookupClaimed = false;
    workspace.queue.activeRoot = source.root;
    h.root = source.root;
    h.search = "";
    h.mediaPage = 1;
    h.libraryBrowse = null;
    const result = scanResultWithQueueStatus(source);
    h.scanResult = result;
    if (result) {
      h.recursive = result.recursive;
      h.minimumSizeMib = String(result.minimum_size_mib);
      h.environment = result.environment;
      h.settings = result.settings;
      h.settingsSources = result.settings_sources;
      h.workDirectory = result.work_directory;
      h.settingsRoot = result.root;
      h.settingsBaseline = JSON.stringify(result.settings);
      h.librarySettingsBaseline = h.settingsBaseline;
      h.metadataLookupPending = h.sourceNeedsMetadataLookup(source);
      h.metadataLookupStatus = null;
    } else {
      h.metadataLookupPending = false;
      h.metadataLookupStatus = null;
      void h.loadSettings(source.root);
    }
    void h.persistPreferences({ root: source.root });
  }

  function removeQueueSource(sourceRoot: string) {
    if (h.workflowBusy) return;
    const removed = workspace.queue.sources.find((source) => source.root === sourceRoot);
    if (!removed) return;
    workspace.queue.sources = workspace.queue.sources.filter((source) => source.root !== sourceRoot);
    const next = pruneAfterSourceRemoval(sourceRoot, workspace.queue.sources, {
      selectedPaths: workspace.queue.selectedPaths,
      fileTuneOverrides: h.fileTuneOverrides,
      fileStreamOverrides: h.fileStreamOverrides,
      runStates: workspace.queue.runStates,
      matchSuggestions: h.queueMatchSuggestions,
      movieReviews: h.queueMovieReviews,
      seriesReviews: h.queueSeriesReviews,
    });
    workspace.queue.selectedPaths = next.selectedPaths;
    h.fileTuneOverrides = next.fileTuneOverrides;
    h.fileStreamOverrides = next.fileStreamOverrides;
    workspace.queue.runStates = next.runStates;
    h.queueMatchSuggestions = next.matchSuggestions;
    h.queueMovieReviews = next.movieReviews;
    h.queueSeriesReviews = next.seriesReviews;
    if (workspace.queue.activeRoot === sourceRoot) {
      const next = workspace.queue.sources.find((source) => source.scanResult) ?? workspace.queue.sources[0];
      if (next) activateQueueSource(next.root);
      else {
        workspace.queue.activeRoot = null;
        h.scanResult = null;
        h.root = "";
      }
    }
    void persistence.persistQueueSources();
  }

  async function scanQueueSource(path: string, activate: boolean, refresh: boolean) {
    if (!deps.isDesktop()) {
      h.setNotice("Open HEVC Rebuild Studio from the desktop app to inspect this computer and connect to the media tools.", "error");
      return;
    }
    if (workspace.run.id || workspace.run.starting || h.repairId || h.hdrInstalling || h.scanning) return;
    const requestedRoot = path.trim();
    if (!requestedRoot) {
      h.setNotice("Choose a media file or folder first.", "error");
      return;
    }
    const existing = workspace.queue.sources.find((source) => source.root === requestedRoot);
    if (existing && !refresh && existing.scanResult) {
      if (activate) activateQueueSource(existing.root);
      h.setNotice("This source is already in the queue. Use Refresh on its card to scan it again.", "info");
      return;
    }

    const preserveRunSettings = h.settingsChanged && h.settingsRoot === requestedRoot ? h.settings : null;
    if (refresh || !existing?.scanResult) {
      h.metadataLookupClaimed = false;
      h.queueMatchSuggestions = Object.fromEntries(Object.entries(h.queueMatchSuggestions)
        .filter(([mediaPath]) => !pathIsWithin(requestedRoot, mediaPath)));
    }
    h.metadataLookupStatus = null;
    const scanStartedAt = Date.now();
    const scanningSource: MediaQueueSource = existing
      ? { ...normalizeQueueSource(existing), status: "scanning", error: null, updatedAtMs: scanStartedAt }
      : { root: requestedRoot, status: "scanning", scanResult: null, error: null, updatedAtMs: scanStartedAt, changedFiles: [], missingFiles: [], excludedPaths: [], entryOrder: [] };
    workspace.queue.sources = existing
      ? workspace.queue.sources.map((source) => source.root === requestedRoot ? scanningSource : source)
      : [...workspace.queue.sources, scanningSource];
    h.root = requestedRoot;
    h.scanning = true;
    h.scanProgress = null;
    h.setNotice("Scanning this source. Other queued folders stay available.", "info");
    await persistence.persistQueueSources();
    await h.persistPreferences({ root: requestedRoot, recursive: h.recursive, minimumSizeMib: h.minimumSizeMib, stopOnError: h.stopOnError });
    try {
      const result = await invoke<ScanResult>("scan_folder", {
        options: {
          root: requestedRoot,
          recursive: h.recursive,
          minimumSizeMib: Math.max(0, Math.min(4096, Number(h.minimumSizeMib) || 0)),
          filenameIncludePatterns: h.appPreferences.filenameIncludePatterns,
          filenameExcludePatterns: h.appPreferences.filenameExcludePatterns,
          toolPaths: h.currentToolPaths(),
        },
      });
      const excludedPaths = existing?.excludedPaths ?? [];
      const keptEntries = applyQueueFilePolicy(result.entries, excludedPaths, existing?.entryOrder ?? [], queuePathKey);
      if (result.interrupted && keptEntries.length === 0 && existing?.scanResult) {
        workspace.queue.sources = workspace.queue.sources.map((source) => source.root === requestedRoot
          ? { ...normalizeQueueSource(existing), status: "ready", error: null, updatedAtMs: Date.now() }
          : source);
        if (activate) activateQueueSource(existing.root);
        await persistence.persistQueueSources();
        h.setNotice("Scan stopped. The previous analysis is unchanged.", "info");
        return;
      }
      const shaped: ScanResult = { ...result, entries: keptEntries, candidate_count: keptEntries.length };
      const refreshedPaths = new Set(shaped.entries.map((entry) => entry.path));
      const updatedSource: MediaQueueSource = {
        root: result.root,
        status: result.interrupted && shaped.entries.length === 0 ? "interrupted" : "ready",
        scanResult: shaped,
        error: null,
        updatedAtMs: Date.now(),
        changedFiles: [],
        missingFiles: [],
        excludedPaths,
        entryOrder: shaped.entries.map((entry) => entry.path),
      };
      let replaced = false;
      const nextSources: MediaQueueSource[] = [];
      for (const source of workspace.queue.sources) {
        if (source.root === requestedRoot || source.root === result.root) {
          if (!replaced) nextSources.push(updatedSource);
          replaced = true;
        } else nextSources.push(source);
      }
      if (!replaced) nextSources.push(updatedSource);
      workspace.queue.sources = nextSources;
      const availablePaths = new Set(workspace.queue.sources.flatMap((source) => source.scanResult?.entries.map((entry) => entry.path) ?? []));
      workspace.queue.selectedPaths = workspace.queue.selectedPaths.filter((mediaPath) => availablePaths.has(mediaPath));
      h.fileTuneOverrides = Object.fromEntries(Object.entries(h.fileTuneOverrides).filter(([mediaPath]) => availablePaths.has(mediaPath)));
      h.queueMatchSuggestions = Object.fromEntries(Object.entries(h.queueMatchSuggestions).filter(([mediaPath]) => availablePaths.has(mediaPath)));
      const oldEntries = new Map((existing?.scanResult?.entries ?? []).map((entry) => [queuePathKey(entry.path), entry]));
      const refreshedEntries = new Map(shaped.entries.map((entry) => [queuePathKey(entry.path), entry]));
      workspace.queue.runStates = Object.fromEntries(Object.entries(workspace.queue.runStates)
        .filter(([path]) => availablePaths.has(path))
        .map(([path, state]) => {
          if (!refreshedPaths.has(path)) return [path, state];
          const before = oldEntries.get(queuePathKey(path));
          const after = refreshedEntries.get(queuePathKey(path));
          const sameSource = Boolean(before && after && before.modified_unix_ms != null && after.modified_unix_ms != null && before.size_bytes === after.size_bytes && before.modified_unix_ms === after.modified_unix_ms);
          return [path, sameSource ? state : makeQueueRunState("needs_review", state.mode ?? h.mode, {
            message: "This source changed since the recorded run. Review the existing output before retrying.",
            reportPath: state.reportPath,
            reportStatus: state.reportStatus,
          })];
        }));
      // A fresh probe may change track indices for this source. Preserve overrides for every other queue source.
      h.fileStreamOverrides = Object.fromEntries(Object.entries(h.fileStreamOverrides).filter(([mediaPath]) => availablePaths.has(mediaPath) && !refreshedPaths.has(mediaPath)));
      const retainedOutputDirs = Object.fromEntries(Object.entries(h.appPreferences.fileOutputDirs).filter(([mediaPath]) => availablePaths.has(mediaPath)));
      if (Object.keys(retainedOutputDirs).length !== Object.keys(h.appPreferences.fileOutputDirs).length) {
        void h.persistPreferences({ fileOutputDirs: retainedOutputDirs });
      }
      if (activate) {
        workspace.queue.activeRoot = result.root;
        h.root = result.root;
        h.scanResult = scanResultWithQueueStatus(updatedSource);
        h.recursive = result.recursive;
        h.minimumSizeMib = String(result.minimum_size_mib);
        h.settingsRoot = result.root;
        h.settingsBaseline = JSON.stringify(result.settings);
        h.librarySettingsBaseline = h.settingsBaseline;
        h.environment = result.environment;
        h.settings = preserveRunSettings ?? result.settings;
        h.settingsSources = result.settings_sources;
        h.workDirectory = result.work_directory;
        h.metadataLookupPending = h.sourceNeedsMetadataLookup(updatedSource);
        if (h.metadataLookupPending) h.metadataLookupClaimed = false;
        if (h.page !== "home" && h.page !== "youtube") h.openPage("library");
        void h.persistPreferences({ root: h.root });
      }
      const persisted = await persistence.persistQueueSources();
      const scannedCount = shaped.candidate_count;
      h.setNotice(
        result.interrupted
          ? `Scan stopped. ${scannedCount} analyzed file${scannedCount === 1 ? "" : "s"} remain in the queue.`
          : persisted
            ? `${scannedCount} media file${scannedCount === 1 ? "" : "s"} scanned and saved in the queue.`
            : `${scannedCount} media file${scannedCount === 1 ? "" : "s"} scanned, but the queue could not be saved.`,
        result.interrupted ? "info" : persisted ? "success" : "error",
      );
      if (h.page !== "youtube" && h.mode === "youtube") h.mode = "check";
    } catch (error) {
      const failed = workspace.queue.sources.find((source) => source.root === requestedRoot);
      if (failed) {
        workspace.queue.sources = workspace.queue.sources.map((source) => source.root === requestedRoot
          ? { ...source, status: source.scanResult ? "ready" : "failed", error: String(error), updatedAtMs: Date.now() }
          : source);
      }
      await persistence.persistQueueSources();
      h.setNotice(`Media scan failed: ${String(error)}`, "error");
    } finally {
      h.scanning = false;
      h.scanProgress = null;
    }
  }

  async function addQueueSources(paths: string[]) {
    for (const path of [...new Set(paths.map((item) => item.trim()).filter(Boolean))]) {
      const existing = workspace.queue.sources.find((source) => source.root === path);
      if (existing?.scanResult) {
        activateQueueSource(existing.root);
        continue;
      }
      await scanQueueSource(path, true, false);
    }
  }

  function reorderQueueSources(fromRoot: string, toRoot: string) {
    const reordered = reorderSources(workspace.queue.sources, fromRoot, toRoot);
    if (!reordered) return;
    workspace.queue.sources = reordered;
    void persistence.persistQueueSources();
  }

  function moveQueueSource(root: string, delta: number) {
    const from = workspace.queue.sources.findIndex((source) => source.root === root);
    reorderQueueSources(root, workspace.queue.sources[from + delta]?.root ?? root);
  }

  function reorderQueueFiles(sourceRoot: string, fromPath: string, toPath: string) {
    workspace.queue.sources = reorderSourceEntries(workspace.queue.sources, sourceRoot, fromPath, toPath);
    syncActiveScanResult();
    void persistence.persistQueueSources();
  }

  function moveQueueFile(sourceRoot: string, path: string, delta: number) {
    const source = workspace.queue.sources.find((item) => item.root === sourceRoot);
    const entries = source?.scanResult?.entries ?? [];
    const from = entries.findIndex((entry) => entry.path === path);
    const target = entries[from + delta];
    if (!target) return;
    reorderQueueFiles(sourceRoot, path, target.path);
  }

  function removeQueueEntry(path: string) {
    if (h.workflowBusy) return;
    const { sources, removed } = removeEntryFromSources(workspace.queue.sources, path);
    workspace.queue.sources = sources;
    if (!removed) return;
    pruneQueuePath(path);
    syncActiveScanResult();
    void persistence.persistQueueSources();
  }

  function restoreExcludedQueueFiles(sourceRoot: string) {
    if (h.workflowBusy) return;
    workspace.queue.sources = restoreExcludedFiles(workspace.queue.sources, sourceRoot);
    void persistence.persistQueueSources();
    void scanQueueSource(sourceRoot, true, true);
  }

  async function loadMediaQueue() {
    try {
      const payload = await invoke<MediaQueueLoadResult>("load_library_queue");
      workspace.queue.sources = (payload.queue.sources ?? []).map(normalizeQueueSource);
      workspace.queue.selectedPaths = payload.queue.selectedMediaPaths ?? [];
      h.fileTuneOverrides = payload.queue.fileTuneOverrides ?? {};
      h.fileStreamOverrides = payload.queue.fileStreamOverrides ?? {};
      h.queueMovieReviews = payload.queue.movieReviews ?? {};
      h.queueSeriesReviews = Object.fromEntries(Object.entries(payload.queue.seriesReviews ?? {}).map(([root, review]) => [
        root,
        { ...review, seasonRecords: review.seasonRecords ?? {}, localSeasonPosterPaths: review.localSeasonPosterPaths ?? {}, episodeAssignments: review.episodeAssignments ?? {} },
      ]));
      h.queueMatchSuggestions = payload.queue.matchSuggestions ?? {};
      workspace.queue.runStates = payload.queue.runStates ?? {};
      workspace.queue.file = payload.queueFile;
      workspace.queue.persistenceWarning = payload.warning ?? "";
      const active = workspace.queue.sources.find((source) => source.root === payload.queue.activeRoot && source.scanResult)
        ?? workspace.queue.sources.find((source) => source.scanResult);
      workspace.queue.activeRoot = active?.root ?? null;
      if (active) {
        h.root = active.root;
        h.scanResult = scanResultWithQueueStatus(active);
        if (h.scanResult) {
          h.environment = h.scanResult.environment;
          h.settings = h.scanResult.settings;
          h.settingsSources = h.scanResult.settings_sources;
          h.workDirectory = h.scanResult.work_directory;
          h.settingsBaseline = JSON.stringify(h.scanResult.settings);
          h.librarySettingsBaseline = h.settingsBaseline;
          h.settingsRoot = h.scanResult.root;
          h.recursive = h.scanResult.recursive;
          h.minimumSizeMib = String(h.scanResult.minimum_size_mib);
        }
      }
      workspace.queue.loaded = true;
      h.metadataLookupPending = active ? h.sourceNeedsMetadataLookup(active) : false;
      if (h.metadataLookupPending) h.metadataLookupClaimed = false;
      if (Object.values(workspace.queue.runStates).some((state) => state.status === "interrupted")) {
        await h.recoverInterruptedSession();
      }
      if (workspace.queue.persistenceWarning) h.setNotice(workspace.queue.persistenceWarning, "error");
    } catch (error) {
      workspace.queue.loaded = true;
      workspace.queue.persistenceError = String(error);
      h.setNotice(`Could not restore the media queue: ${String(error)}`, "error");
    }
  }

  return {
    syncActiveScanResult, activateQueueSource, removeQueueSource, scanQueueSource, addQueueSources,
    reorderQueueSources, moveQueueSource, reorderQueueFiles, moveQueueFile, removeQueueEntry,
    restoreExcludedQueueFiles, loadMediaQueue,
  };
}
