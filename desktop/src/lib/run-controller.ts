// ./desktop/src/lib/run-controller.ts
// Run controller extracted from the route (run slice A): queueing a run, starting source batches,
// finishing them, safe and immediate stop, progress-to-queue-state mapping, and manual
// retry/skip/diagnostics for queued files. Route state is reached through one typed `RunHost`
// of accessors; the native bridge, workspace, persistence, queue controller and the early-event
// buffer are injected, so every branch runs in node with fakes (pnpm test:run-controller).
// The output-preview and recovery-review dialogs stay in the route. No DOM.

import { reportOutcome } from './workspace-state.ts';
import type { RunEventBuffer, WorkspaceState } from './workspace-state.ts';
import { fileName, queuePathKey } from './queue-paths.ts';
import { makeQueueRunState, runProgressStatus } from './queue-run-state.ts';
import type { createQueuePersistence } from './queue-persistence.ts';
import type { AppPreferences } from './ui-contracts.js';
import type {
  FileRunSettingsOverride, MediaEntry, MediaQueueRunState, MediaQueueSource, RunFinishedEvent, RunMode, RunProgressEvent, RunReport,
  Settings, ToolPaths, YoutubeMode, YoutubePreset, YoutubeQualityCheck, YoutubeSdrCodec, YoutubeShrink,
} from './types.js';

export type SourceRunBatch = { source: MediaQueueSource; entries: MediaEntry[] };

/** Everything the run controller reads or writes on the route. Property names match the route's variables. */
export interface RunHost {
  readonly settings: Settings | null;
  mode: RunMode;
  pendingRunBatches: SourceRunBatch[];
  activeRunBatchIndex: number;
  pendingRunFullConfirmed: boolean;
  recoveryReviewedPaths: Set<string>;
  queueRunLogs: Record<string, { loading: boolean; text: string; error: string }>;
  abortConfirmOpen: boolean;
  readonly queuedEntries: MediaEntry[];
  readonly runBatches: SourceRunBatch[];
  readonly reports: RunReport[];
  readonly queueRunStatesByPath: Map<string, MediaQueueRunState>;
  readonly recursive: boolean;
  readonly minimumSizeMib: string;
  readonly youtubeResolution: "source" | "1440p" | "4k";
  readonly youtubeQualityMode: "balanced" | "recommended";
  readonly youtubeSourceMultiplier: number;
  readonly youtubeMode: YoutubeMode;
  readonly youtubeSdrCodec: YoutubeSdrCodec;
  readonly youtubePreset: YoutubePreset;
  readonly youtubeShrink: YoutubeShrink;
  readonly youtubeQualityCheck: YoutubeQualityCheck;
  readonly youtubeOutputDir: string;
  readonly stopOnError: boolean;
  readonly integrityLevel: "quick" | "full";
  readonly fileStreamOverrides: Record<string, number[]>;
  readonly fileSettingsOverrides: Record<string, FileRunSettingsOverride>;
  readonly youtubeHdr10plusLossPaths: Set<string>;
  readonly workflowBusy: boolean;
  readonly appPreferences: AppPreferences;
  setNotice(message: string, kind?: "info" | "success" | "error"): void;
  openPage(page: "run" | "youtube" | "library" | "reports"): void;
  currentToolPaths(): ToolPaths;
  loadReports(): Promise<void>;
  selectedPosterOverrides(entries: MediaEntry[]): Record<string, string>;
  selectedMetadataOverrides(entries: MediaEntry[]): Record<string, Record<string, string | string[]>>;
  fileTuneOverridesForBatch(entries: MediaEntry[], settings: Settings): Record<string, "animation" | "none">;
  openLog(path: string | null | undefined): Promise<void>;
  openMetadataForEntry(entry: MediaEntry): Promise<void>;
  chooseReport(path: string): void;
}

export interface RunControllerDeps {
  invoke: <T>(command: string, args?: Record<string, unknown>) => Promise<T>;
  workspace: WorkspaceState;
  host: RunHost;
  persistence: Pick<ReturnType<typeof createQueuePersistence>, 'persistQueueRunStates' | 'persistQueueSources'>;
  queue: { activateQueueSource(sourceRoot: string): void };
  runEventBuffer: RunEventBuffer;
}

export function createRunController(deps: RunControllerDeps) {
  const { invoke, workspace, host: h, persistence, queue, runEventBuffer } = deps;

  function queueRunStateForPath(path: string): MediaQueueRunState | undefined {
    return h.queueRunStatesByPath.get(queuePathKey(path));
  }

  function recordQueueRunProgress(event: RunProgressEvent) {
    if (!event.filePath) return;
    const current = queueRunStateForPath(event.filePath);
    if (!current || !["running", "scanning", "renaming", "remuxing", "rebuilding", "validating"].includes(current.status)) return;
    const status = runProgressStatus(event.phase, current.mode);
    if (current.status === status) return;
    void persistence.persistQueueRunStates({
      [event.filePath]: {
        ...current,
        status,
        message: [...event.phase].slice(0, 1024).join(""),
        updatedAtMs: Date.now(),
      },
    });
  }

  function noteRunProgress(event: RunProgressEvent) {
    workspace.run.progress = event;
    workspace.run.progressSeenAt = Date.now();
    recordQueueRunProgress(event);
  }

  function handleQueueRenameState(paths: string[], status: "renaming" | "renamed" | "failed", message = "") {
    if (!workspace.queue.loaded || !paths.length) return;
    const allowed = new Map(h.queuedEntries.map((entry) => [queuePathKey(entry.path), entry.path]));
    const updates: Record<string, MediaQueueRunState> = {};
    for (const path of paths) {
      const savedPath = allowed.get(queuePathKey(path));
      if (!savedPath) continue;
      updates[savedPath] = makeQueueRunState(status, "organize", { message });
    }
    if (Object.keys(updates).length) void persistence.persistQueueRunStates(updates);
  }

  async function beginQueuedRun(fullRebuildConfirmed: boolean) {
    if (!h.settings) return;
    workspace.run.mode = h.mode;
    h.pendingRunBatches = h.runBatches.map((batch) => ({ source: batch.source, entries: [...batch.entries] }));
    h.activeRunBatchIndex = -1;
    h.pendingRunFullConfirmed = fullRebuildConfirmed;
    workspace.run.stopAfterCurrent = false;
    workspace.run.aborting = false;
    const queuedStates = Object.fromEntries(h.pendingRunBatches.flatMap((batch) => batch.entries.map((entry) => [
      entry.path,
      makeQueueRunState("queued", workspace.run.mode),
    ])));
    h.recoveryReviewedPaths = new Set();
    if (!await persistence.persistQueueRunStates(queuedStates)) {
      h.pendingRunBatches = [];
      h.setNotice(`The queue could not save its run plan, so the workflow did not start: ${workspace.queue.persistenceError}`, "error");
      return;
    }
    await startNextRunBatch();
  }

  async function startNextRunBatch() {
    const runMode = workspace.run.mode;
    const nextIndex = h.activeRunBatchIndex + 1;
    const batch = h.pendingRunBatches[nextIndex];
    if (!batch) return;
    const batchSettings = batch.source.root === workspace.queue.activeRoot
      ? h.settings
      : batch.source.scanResult?.settings ?? h.settings;
    if (!batchSettings) {
      h.setNotice(`Settings are unavailable for ${batch.source.root}. Rescan that source before running it.`, "error");
      h.pendingRunBatches = [];
      h.activeRunBatchIndex = -1;
      return;
    }
    const activeBatchStates = Object.fromEntries(batch.entries.map((entry) => [
      entry.path,
      makeQueueRunState("running", runMode, { message: "This file is included in the active source batch." }),
    ]));
    if (!await persistence.persistQueueRunStates(activeBatchStates)) {
      h.pendingRunBatches = [];
      h.activeRunBatchIndex = -1;
      h.setNotice(`The queue could not record the active batch, so it was not started: ${workspace.queue.persistenceError}`, "error");
      return;
    }
    h.activeRunBatchIndex = nextIndex;
    runEventBuffer.reset();
    workspace.run.lines = [];
    workspace.run.progress = null;
    workspace.run.startedAt = Date.now();
    workspace.run.progressSeenAt = null;
    workspace.run.resources = null;
    workspace.run.starting = true;
    workspace.run.message = `Starting source ${nextIndex + 1} of ${h.pendingRunBatches.length}…`;
    try {
      const runId = await invoke<string>("start_run", {
        request: {
          root: batch.source.root,
          recursive: batch.source.scanResult?.recursive ?? h.recursive,
          minimumSizeMib: batch.source.scanResult?.minimum_size_mib ?? Math.max(0, Math.min(4096, Number(h.minimumSizeMib) || 0)),
          toolPaths: h.currentToolPaths(),
          mode: runMode,
          youtubeResolution: h.youtubeResolution,
          youtubeQualityMode: h.youtubeQualityMode,
          youtubeSourceMultiplier: h.youtubeSourceMultiplier,
          youtubeMode: h.youtubeMode,
          youtubeSdrCodec: h.youtubeSdrCodec,
          youtubePreset: h.youtubePreset,
          youtubeShrink: h.youtubeShrink,
          youtubeQualityCheck: h.youtubeQualityCheck,
          youtubeOutputDir: h.youtubeOutputDir.trim() || null,
          rebuildOutputDir: runMode === "metadata" || runMode === "full" || runMode === "test" ? h.appPreferences.rebuildOutputDir : null,
          fileOutputDirs: runMode === "metadata" || runMode === "full" || runMode === "test" ? Object.fromEntries(Object.entries(h.appPreferences.fileOutputDirs).filter(([path]) => batch.entries.some((entry) => entry.path === path))) : {},
          filenameIncludePatterns: h.appPreferences.filenameIncludePatterns,
          filenameExcludePatterns: h.appPreferences.filenameExcludePatterns,
          selectedFiles: batch.entries.map((entry) => entry.path),
          settings: batchSettings,
          stopOnError: h.stopOnError,
          fullRebuildConfirmed: h.pendingRunFullConfirmed,
          integrityLevel: runMode === "integrity" ? h.integrityLevel : null,
          fileTuneOverrides: runMode === "youtube" || runMode === "metadata" ? {} : h.fileTuneOverridesForBatch(batch.entries, batchSettings),
          fileStreamOverrides: runMode === "youtube" || runMode === "metadata" ? {} : Object.fromEntries(Object.entries(h.fileStreamOverrides).filter(([path]) => batch.entries.some((entry) => entry.path === path))),
          fileSettingsOverrides: runMode === "youtube" || runMode === "metadata" ? {} : Object.fromEntries(Object.entries(h.fileSettingsOverrides).filter(([path]) => batch.entries.some((entry) => entry.path === path))),
          filePosterOverrides: runMode === "metadata" || runMode === "full" || runMode === "test" ? h.selectedPosterOverrides(batch.entries) : {},
          fileMetadataOverrides: runMode === "metadata" || runMode === "full" || runMode === "test" ? h.selectedMetadataOverrides(batch.entries) : {},
          allowYoutubeHdr10plusLossPaths: runMode === "youtube" ? [...h.youtubeHdr10plusLossPaths].filter((path) => batch.entries.some((entry) => entry.path === path)) : [],
        },
      });
      const earlyEvents = runEventBuffer.claim(runId);
      workspace.run.id = runId;
      workspace.run.starting = false;
      workspace.run.lines = earlyEvents.lines;
      const startedProgress = earlyEvents.progress;
      if (startedProgress) noteRunProgress(startedProgress);
      else workspace.run.progress = null;
      workspace.run.resources = earlyEvents.resources;
      const batchLabel = `${nextIndex + 1}/${h.pendingRunBatches.length} · ${fileName(batch.source.root)} · ${batch.entries.length} file${batch.entries.length === 1 ? "" : "s"}`;
      workspace.run.message = runMode === "full" ? `Full rebuild ${batchLabel}. Originals stay protected until each rebuilt file validates.` : runMode === "test" ? `Short tests ${batchLabel}. Originals remain untouched.` : runMode === "metadata" ? `Metadata-only copies ${batchLabel}. All tracks are preserved; no video is encoded and sources stay in place.` : runMode === "youtube" ? `Separate YouTube copies ${batchLabel}. Originals stay in place.` : runMode === "integrity" ? `Integrity check (${h.integrityLevel}) ${batchLabel}. Nothing is written.` : `Check-only pass ${batchLabel}. No media will be encoded.`;
      h.openPage(runMode === "youtube" ? "youtube" : "run");
      h.setNotice("Run started. Live progress and diagnostics are shown below.", "success");
      const earlyFinish = earlyEvents.finished;
      if (earlyFinish?.runId === runId) await finishRun(earlyFinish);
    } catch (error) {
      workspace.run.id = "";
      workspace.run.starting = false;
      workspace.run.message = "Could not start the selected workflow.";
      await persistence.persistQueueRunStates(Object.fromEntries(batch.entries.map((entry) => [
        entry.path,
        makeQueueRunState("failed", runMode, { message: String(error) }),
      ])));
      h.pendingRunBatches = [];
      h.activeRunBatchIndex = -1;
      h.setNotice(String(error), "error");
    }
  }

  async function finishRun(payload: RunFinishedEvent) {
    workspace.run.id = "";
    if (payload.error) workspace.run.message = payload.error;
    else if (payload.stopRequested) workspace.run.message = "Queue stopped safely after the active file.";
    else if (payload.exitCode === 0) workspace.run.message = "Workflow finished. Review the report and any skipped files.";
    else workspace.run.message = `Workflow finished with exit code ${payload.exitCode ?? "unknown"}. Review the diagnostics.`;
    await h.loadReports();
    const finishedBatch = h.pendingRunBatches[h.activeRunBatchIndex];
    const matchingReport = payload.reportPath
      ? h.reports.find((report) => queuePathKey(report.csv_path) === queuePathKey(payload.reportPath!))
      : undefined;
    const reportRows = new Map((matchingReport?.rows ?? []).map((row) => [queuePathKey(row.input), row]));
    const finishedAt = Date.now();
    const completedStates: Record<string, MediaQueueRunState> = {};
    for (const entry of finishedBatch?.entries ?? []) {
      const row = reportRows.get(queuePathKey(entry.path));
      const reportStatus = row?.status?.trim().toUpperCase() ?? null;
      const status = reportOutcome(reportStatus, Boolean(row), payload);
      completedStates[entry.path] = {
        status,
        mode: workspace.run.mode,
        message: (() => {
          const message = row?.message || (status === "queued"
          ? "Not started before the safe stop."
          : status === "needs_review"
            ? "No matching per-file result was found in the run report. Review the report and output before retrying."
            : payload.error ?? null);
          return message ? [...message].slice(0, 1024).join("") : null;
        })(),
        outputPath: row?.output || null,
        reportPath: payload.reportPath,
        reportStatus,
        updatedAtMs: finishedAt,
      };
    }
    if (Object.keys(completedStates).length && !await persistence.persistQueueRunStates(completedStates)) {
      h.pendingRunBatches = [];
      h.activeRunBatchIndex = -1;
      workspace.run.stopAfterCurrent = false;
      workspace.run.aborting = false;
      workspace.run.message = "The report was written, but queue state could not be saved. Later source batches were not started.";
      h.setNotice(`${workspace.run.message} ${workspace.queue.persistenceError}`, "error");
      return;
    }
    const hasFailure = payload.exitCode !== 0 || Boolean(payload.error);
    const isLastBatch = h.activeRunBatchIndex >= h.pendingRunBatches.length - 1;
    const shouldContinue = !isLastBatch && !payload.stopRequested && !workspace.run.stopAfterCurrent && !workspace.run.aborting && !(h.stopOnError && hasFailure);
    if (shouldContinue) {
      workspace.run.message = `Finished source ${h.activeRunBatchIndex + 1} of ${h.pendingRunBatches.length}. Starting the next source…`;
      await startNextRunBatch();
      return;
    }
    if (payload.logPath) void h.openLog(payload.logPath);
    const remainingBatches = Math.max(0, h.pendingRunBatches.length - h.activeRunBatchIndex - 1);
    if (remainingBatches > 0 && (payload.stopRequested || workspace.run.stopAfterCurrent || workspace.run.aborting || (h.stopOnError && hasFailure))) {
      const remaining = remainingBatches === 1 ? "1 source batch remains queued" : `${remainingBatches} source batches remain queued`;
      workspace.run.message = workspace.run.aborting
        ? `Stopped now. ${remaining}. Source files were not replaced.`
        : payload.stopRequested || workspace.run.stopAfterCurrent
        ? `Stopped safely. ${remaining}.`
        : `Stopped after an error. ${remaining}.`;
    }
    h.pendingRunBatches = [];
    h.activeRunBatchIndex = -1;
    workspace.run.stopAfterCurrent = false;
    workspace.run.aborting = false;
    h.setNotice(workspace.run.message, payload.exitCode === 0 && !payload.error ? "success" : "error");
  }

  async function stopAfterCurrent() {
    if (!workspace.run.id || workspace.run.aborting) return;
    try {
      workspace.run.stopAfterCurrent = true;
      await invoke("request_stop", { runId: workspace.run.id });
      workspace.run.message = "Safe stop requested. The current file will finish before the queue stops.";
      h.setNotice("Safe stop requested; the active file will finish first.", "success");
    } catch (error) {
      workspace.run.stopAfterCurrent = false;
      h.setNotice(`Could not request a safe stop: ${String(error)}`, "error");
    }
  }

  async function confirmStopNow() {
    if (!workspace.run.id || workspace.run.stopAfterCurrent) return;
    try {
      workspace.run.aborting = true;
      h.abortConfirmOpen = false;
      await invoke("request_abort", { runId: workspace.run.id });
      workspace.run.message = "Stop now requested. The source stays in place; this encode will not resume.";
      h.setNotice("Stop now requested. The original file is unchanged.", "success");
    } catch (error) {
      workspace.run.aborting = false;
      h.setNotice(`Could not stop the run: ${String(error)}`, "error");
    }
  }

  async function prepareQueueEntryForRetry(entry: MediaEntry, state: MediaQueueRunState) {
    if (h.workflowBusy || entry.queue_stale) return;
    const source = workspace.queue.sources.find((item) => item.scanResult?.entries.some((candidate) => queuePathKey(candidate.path) === queuePathKey(entry.path)));
    if (!source) {
      h.setNotice("This file is no longer attached to a completed queue scan. Refresh its source before retrying.", "error");
      return;
    }
    if (source.status !== "ready" || !source.scanResult) {
      h.setNotice("Refresh this source and wait for its scan to finish before retrying the file.", "error");
      return;
    }
    if (state.mode === "organize") {
      queue.activateQueueSource(source.root);
      h.openPage("library");
      await h.openMetadataForEntry(entry);
      h.setNotice("Rename review reopened for this file. Check the title and preview the proposed filename before confirming another change.", "info");
      return;
    }
    const previousSelection = workspace.queue.selectedPaths;
    const previousMode = h.mode;
    queue.activateQueueSource(source.root);
    const retryMode: RunMode = state.mode ?? h.mode;
    h.mode = retryMode;
    workspace.queue.selectedPaths = [entry.path];
    if (state.status !== "interrupted" && state.status !== "needs_review") {
      const queued = makeQueueRunState("queued", retryMode, {
        message: "Selected for a manual retry. Existing outputs remain protected by the destination check.",
        outputPath: state.outputPath,
        reportPath: state.reportPath,
        reportStatus: state.reportStatus,
      });
      if (!await persistence.persistQueueRunStates({ [entry.path]: queued })) {
        workspace.queue.runStates = { ...workspace.queue.runStates, [entry.path]: state };
        workspace.queue.selectedPaths = previousSelection;
        h.mode = previousMode;
        h.setNotice(`The retry could not be saved to the queue: ${workspace.queue.persistenceError}`, "error");
        return;
      }
    } else {
      if (!await persistence.persistQueueSources()) {
        workspace.queue.selectedPaths = previousSelection;
        h.mode = previousMode;
        h.setNotice(`The retry selection could not be saved to the queue: ${workspace.queue.persistenceError}`, "error");
        return;
      }
    }
    h.openPage(retryMode === "youtube" ? "youtube" : "run");
    h.setNotice(state.status === "interrupted" || state.status === "needs_review"
      ? "This file is selected. Start its saved task to review the last report and output before retrying."
      : "This file is selected for a retry. Review the task and output destination before starting.", "info");
  }

  async function skipQueueEntry(entry: MediaEntry, state: MediaQueueRunState) {
    if (h.workflowBusy || entry.queue_stale) return;
    const skipped = makeQueueRunState("skipped", state.mode ?? h.mode, {
      message: "Skipped by the user. Previous report and output details are retained.",
      outputPath: state.outputPath,
      reportPath: state.reportPath,
      reportStatus: state.reportStatus,
    });
    const previousSelection = workspace.queue.selectedPaths;
    workspace.queue.selectedPaths = workspace.queue.selectedPaths.filter((path) => queuePathKey(path) !== queuePathKey(entry.path));
    if (!await persistence.persistQueueRunStates({ [entry.path]: skipped })) {
      workspace.queue.runStates = { ...workspace.queue.runStates, [entry.path]: state };
      workspace.queue.selectedPaths = previousSelection;
      h.setNotice(`The skipped state could not be saved: ${workspace.queue.persistenceError}`, "error");
      return;
    }
    h.setNotice(`${entry.name} is skipped for ${skipped.mode?.toUpperCase() ?? "this task"}. Select it again to review or run it.`, "success");
  }

  async function openQueueRunReport(path: string) {
    await h.loadReports();
    const report = h.reports.find((candidate) => queuePathKey(candidate.csv_path) === queuePathKey(path));
    if (!report) {
      h.setNotice("The saved report is no longer available in recent history.", "error");
      return;
    }
    h.openPage("reports");
    h.chooseReport(report.csv_path);
  }

  async function loadQueueRunDiagnostics(entry: MediaEntry, state: MediaQueueRunState) {
    const key = queuePathKey(entry.path);
    if (!state.reportPath || h.queueRunLogs[key]?.loading) return;
    h.queueRunLogs = { ...h.queueRunLogs, [key]: { loading: true, text: "", error: "" } };
    try {
      await h.loadReports();
      const report = h.reports.find((candidate) => queuePathKey(candidate.csv_path) === queuePathKey(state.reportPath!));
      if (!report) throw new Error("The report is no longer available in recent history.");
      const text = await invoke<string>("read_log", { path: report.log_path });
      h.queueRunLogs = { ...h.queueRunLogs, [key]: { loading: false, text, error: "" } };
    } catch (error) {
      h.queueRunLogs = { ...h.queueRunLogs, [key]: { loading: false, text: "", error: String(error) } };
    }
  }

  return {
    queueRunStateForPath, noteRunProgress, handleQueueRenameState, beginQueuedRun, startNextRunBatch, finishRun,
    stopAfterCurrent, confirmStopNow, prepareQueueEntryForRetry, skipQueueEntry, openQueueRunReport, loadQueueRunDiagnostics,
  };
}
