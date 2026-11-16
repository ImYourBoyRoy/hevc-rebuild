// ./desktop/src/lib/run-launch.ts
// Run launch controller extracted from the route (run slice B): the pre-start checks, the output
// preview review, and the recovery review of interrupted files. It decides whether a run may start
// and hands over to the run controller (beginQueuedRun) once the dialogs are satisfied. Route state is
// reached through one typed `RunLaunchHost`; `invoke`, the desktop check, persistence and the run
// controller are injected, so every branch runs in node (pnpm test:run-launch). No DOM.

import { queuePathKey } from './queue-paths.ts';
import type { WorkspaceState } from './workspace-state.ts';
import type { createQueuePersistence } from './queue-persistence.ts';
import type { RecoveryReviewItem } from './ui-contracts.js';
import type { AppPreferences } from './ui-contracts.js';
import type { MediaEntry, MediaQueueRunState, OutputPreview, RunMode, RunReport, ScanResult, Settings, ToolPaths } from './types.js';
import type { SourceRunBatch } from './run-controller.ts';

/** Everything the launch controller reads or writes on the route. Property names match the route's variables. */
export interface RunLaunchHost {
  readonly canStart: boolean;
  readonly mode: RunMode;
  readonly settings: Settings | null;
  readonly unresolvedSelectedPaths: number;
  readonly runBatches: SourceRunBatch[];
  readonly reports: RunReport[];
  readonly queuedEntries: MediaEntry[];
  readonly appPreferences: AppPreferences;
  fullAck: boolean;
  pendingRunFullConfirmed: boolean;
  recoveryReviewedPaths: Set<string>;
  recoveryReviewItems: RecoveryReviewItem[];
  recoveryReviewIndex: number;
  recoveryProbeEntry: MediaEntry | null;
  recoveryProbeError: string;
  recoveryProbeBusy: boolean;
  recoveryProbeAttempted: boolean;
  recoveryAcknowledged: boolean;
  pendingRecoveryFullConfirmed: boolean;
  recoveryReviewOpen: boolean;
  outputPreview: OutputPreview | null;
  outputPreviewOpen: boolean;
  outputPreviewBusy: boolean;
  readonly recoveryCurrentItem: RecoveryReviewItem | null;
  setNotice(message: string, kind?: "info" | "success" | "error"): void;
  loadReports(): Promise<void>;
  currentToolPaths(): ToolPaths;
  selectFilesToReview(): void;
}

export interface RunLaunchDeps {
  invoke: <T>(command: string, args?: Record<string, unknown>) => Promise<T>;
  isDesktop: () => boolean;
  workspace: WorkspaceState;
  host: RunLaunchHost;
  persistence: Pick<ReturnType<typeof createQueuePersistence>, 'persistQueueRunStates'>;
  runs: {
    queueRunStateForPath(path: string): MediaQueueRunState | undefined;
    beginQueuedRun(fullRebuildConfirmed: boolean): Promise<void>;
  };
}

export function createRunLaunchController(deps: RunLaunchDeps) {
  const { invoke, workspace, host: h, persistence, runs } = deps;

  async function beginRun() {
    if (!h.canStart) {
      h.setNotice(`Analyze the selected media and resolve the tools required for ${h.mode === "youtube" ? "YouTube export" : h.mode === "integrity" ? "this integrity check" : "this rebuild task"} before starting.`, "error");
      return;
    }
    h.fullAck = false;
    await launchRun(false);
  }

  function outputPreviewRequest() {
    const selected = new Set(h.runBatches.flatMap((batch) => batch.entries.map((entry) => entry.path)));
    return {
      mode: h.mode as "metadata" | "test" | "full",
      sources: h.runBatches.map((batch) => ({
        root: batch.source.root,
        files: batch.entries.map((entry) => entry.path),
      })),
      outputDir: h.appPreferences.rebuildOutputDir,
      fileOutputDirs: Object.fromEntries(
        Object.entries(h.appPreferences.fileOutputDirs).filter(([path]) => selected.has(path)),
      ),
    };
  }

  async function launchRun(fullRebuildConfirmed: boolean) {
    if (!h.settings) return;
    if (h.unresolvedSelectedPaths > 0) {
      h.setNotice("Some selected files no longer belong to a completed scan. Refresh or remove those source folders before starting.", "error");
      return;
    }
    if (!h.runBatches.length) {
      h.setNotice("Add and scan at least one source before starting a workflow.", "error");
      return;
    }
    if (["metadata", "test", "full"].includes(h.mode)) {
      const pendingRecovery = h.runBatches.flatMap((batch) => batch.entries).flatMap((entry) => {
        const state = runs.queueRunStateForPath(entry.path);
        if (!state || !["interrupted", "needs_review"].includes(state.status) || h.recoveryReviewedPaths.has(queuePathKey(entry.path))) return [];
        const report = state.reportPath
          ? h.reports.find((candidate) => queuePathKey(candidate.csv_path) === queuePathKey(state.reportPath!))?.rows.find((row) => queuePathKey(row.input) === queuePathKey(entry.path)) ?? null
          : null;
        const recordedOutput = state.outputPath || report?.output || "";
        return [{ entry, state, report, targetPath: recordedOutput || entry.path, hasRecordedOutput: Boolean(recordedOutput) }];
      });
      if (pendingRecovery.length) {
        await h.loadReports();
        const refreshed = pendingRecovery.map((item) => {
          const report = item.state.reportPath
            ? h.reports.find((candidate) => queuePathKey(candidate.csv_path) === queuePathKey(item.state.reportPath!))?.rows.find((row) => queuePathKey(row.input) === queuePathKey(item.entry.path)) ?? null
            : null;
          const recordedOutput = item.state.outputPath || report?.output || "";
          return { ...item, report, targetPath: recordedOutput || item.entry.path, hasRecordedOutput: Boolean(recordedOutput) };
        });
        h.recoveryReviewItems = refreshed;
        h.recoveryReviewIndex = 0;
        h.recoveryProbeEntry = null;
        h.recoveryProbeError = "";
        h.recoveryProbeAttempted = false;
        h.recoveryAcknowledged = false;
        h.pendingRecoveryFullConfirmed = fullRebuildConfirmed;
        h.recoveryReviewOpen = true;
        return;
      }
    }
    if (h.mode === "metadata" || h.mode === "test" || h.mode === "full") {
      h.outputPreview = null;
      h.outputPreviewOpen = false;
      h.outputPreviewBusy = true;
      try {
        h.outputPreview = await invoke<OutputPreview>("preview_queue_output_targets", {
          request: outputPreviewRequest(),
        });
        h.outputPreviewOpen = true;
        if (!h.outputPreview.eligible) {
          h.setNotice(`${h.outputPreview.collisionCount} output collision${h.outputPreview.collisionCount === 1 ? "" : "s"} must be resolved before this queue can start.`, "error");
        }
      } catch (error) {
        h.setNotice(`Could not check the queue's output paths: ${String(error)}`, "error");
      } finally {
        h.outputPreviewBusy = false;
      }
      if (!h.outputPreview) return;
      h.pendingRunFullConfirmed = fullRebuildConfirmed;
      return;
    }
    await runs.beginQueuedRun(fullRebuildConfirmed);
  }

  async function confirmOutputPreview() {
    if (!h.outputPreview || h.outputPreviewBusy || !h.outputPreview.eligible || !h.canStart) return;
    h.outputPreviewBusy = true;
    try {
      const reviewedMode = h.outputPreview.mode;
      const refreshed = await invoke<OutputPreview>("preview_queue_output_targets", {
        request: outputPreviewRequest(),
      });
      h.outputPreview = refreshed;
      if (h.mode !== reviewedMode || refreshed.mode !== reviewedMode) {
        h.setNotice("The selected workflow changed during output review. Review its destinations again before starting.", "error");
        return;
      }
      if (!refreshed.eligible) {
        h.setNotice(`${refreshed.collisionCount} output collision${refreshed.collisionCount === 1 ? "" : "s"} appeared during review. Resolve them before starting.`, "error");
        return;
      }
      h.outputPreviewOpen = false;
      await runs.beginQueuedRun(refreshed.mode === "full" ? h.fullAck : h.pendingRunFullConfirmed);
    } catch (error) {
      h.setNotice(`Could not recheck the queue's output paths: ${String(error)}`, "error");
    } finally {
      h.outputPreviewBusy = false;
    }
  }

  async function inspectRecoveryTarget() {
    if (!deps.isDesktop() || h.recoveryProbeBusy || !h.recoveryCurrentItem) return;
    h.recoveryProbeBusy = true;
    h.recoveryProbeAttempted = true;
    h.recoveryProbeEntry = null;
    h.recoveryProbeError = "";
    try {
      const result = await invoke<ScanResult>("scan_folder", {
        options: {
          root: h.recoveryCurrentItem.targetPath,
          recursive: false,
          minimumSizeMib: 0,
          filenameIncludePatterns: [],
          filenameExcludePatterns: [],
          toolPaths: h.currentToolPaths(),
        },
      });
      h.recoveryProbeEntry = result.entries.find((entry) => queuePathKey(entry.path) === queuePathKey(h.recoveryCurrentItem!.targetPath))
        ?? result.entries[0]
        ?? null;
      if (!h.recoveryProbeEntry) h.recoveryProbeError = "No readable media file was found at this path.";
    } catch (error) {
      h.recoveryProbeError = String(error);
    } finally {
      h.recoveryProbeBusy = false;
    }
  }

  async function finishRecoveryReview(markCompleted = false) {
    const item = h.recoveryCurrentItem;
    if (!item || h.recoveryProbeBusy || !h.recoveryProbeAttempted || !h.recoveryAcknowledged) return;
    const reportStatus = (item.report?.status || item.state.reportStatus || "").trim().toUpperCase();
    const reportIsSuccessful = ["OK", "METADATA_OK", "TEST_OK"].includes(reportStatus);
    if (markCompleted) {
      if (!item.hasRecordedOutput || !reportIsSuccessful || h.recoveryProbeEntry?.status !== "Ready") return;
      const completed: MediaQueueRunState = {
        status: "completed",
        mode: item.state.mode,
        message: "Recorded as complete after reviewing the successful report and readable output.",
        outputPath: item.state.outputPath || item.report?.output || item.entry.path,
        reportPath: item.state.reportPath,
        reportStatus,
        updatedAtMs: Date.now(),
      };
      if (!await persistence.persistQueueRunStates({ [item.entry.path]: completed })) {
        h.setNotice(`Could not save the reviewed run result. ${workspace.queue.persistenceError}`, "error");
        return;
      }
    } else {
      h.recoveryReviewedPaths = new Set([...h.recoveryReviewedPaths, queuePathKey(item.entry.path)]);
    }
    if (h.recoveryReviewIndex + 1 < h.recoveryReviewItems.length) {
      h.recoveryReviewIndex += 1;
      h.recoveryProbeEntry = null;
      h.recoveryProbeError = "";
      h.recoveryProbeAttempted = false;
      h.recoveryAcknowledged = false;
      return;
    }
    h.recoveryReviewOpen = false;
    h.recoveryReviewItems = [];
    h.recoveryProbeEntry = null;
    h.recoveryProbeError = "";
    h.recoveryAcknowledged = false;
    h.setNotice(markCompleted ? "The reviewed successful output was recorded as complete." : "Recovery review is recorded for this session. Output paths will be checked again before the run starts.", "info");
    await launchRun(h.pendingRecoveryFullConfirmed);
  }

  function recoveryItemsForEntries(entries: MediaEntry[]): RecoveryReviewItem[] {
    return entries.flatMap((entry) => {
      const state = runs.queueRunStateForPath(entry.path);
      if (!state || !["interrupted", "needs_review"].includes(state.status) || h.recoveryReviewedPaths.has(queuePathKey(entry.path))) return [];
      const report = state.reportPath
        ? h.reports.find((candidate) => queuePathKey(candidate.csv_path) === queuePathKey(state.reportPath!))?.rows.find((row) => queuePathKey(row.input) === queuePathKey(entry.path)) ?? null
        : null;
      const recordedOutput = state.outputPath || report?.output || "";
      return [{ entry, state, report, targetPath: recordedOutput || entry.path, hasRecordedOutput: Boolean(recordedOutput) }];
    });
  }

  async function openInterruptedRecovery() {
    await h.loadReports();
    const items = recoveryItemsForEntries(h.queuedEntries.filter((entry) => {
      const status = runs.queueRunStateForPath(entry.path)?.status;
      return status === "interrupted" || status === "needs_review";
    }));
    if (!items.length) {
      h.selectFilesToReview();
      return;
    }
    h.recoveryReviewItems = items;
    h.recoveryReviewIndex = 0;
    h.recoveryProbeEntry = null;
    h.recoveryProbeError = "";
    h.recoveryProbeAttempted = false;
    h.recoveryAcknowledged = false;
    h.pendingRecoveryFullConfirmed = false;
    h.recoveryReviewOpen = true;
  }

  return { beginRun, launchRun, confirmOutputPreview, inspectRecoveryTarget, finishRecoveryReview, openInterruptedRecovery };
}
