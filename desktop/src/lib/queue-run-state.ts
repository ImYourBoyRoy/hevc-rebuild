// ./desktop/src/lib/queue-run-state.ts
// Queue run-state helpers (status labels, state construction, progress-phase classification),
// extracted from the route. Pure: no IPC, no DOM, no persistence. Tested by pnpm test:queue-run-state.

import type { MediaQueueRunState, MediaQueueRunStatus } from './types.js';

export function queueRunStatusLabel(status: MediaQueueRunStatus): string {
  return ({
    queued: "Queued",
    running: "In active batch",
    scanning: "Scanning",
    renaming: "Renaming",
    remuxing: "Copying streams",
    rebuilding: "Rebuilding video",
    validating: "Validating output",
    renamed: "Renamed",
    completed: "Completed",
    failed: "Failed",
    skipped: "Skipped",
    interrupted: "Interrupted",
    needs_review: "Review before retry",
  })[status];
}

export function makeQueueRunState(
  status: MediaQueueRunStatus,
  runMode: MediaQueueRunState["mode"],
  details: Partial<Omit<MediaQueueRunState, "status" | "mode" | "updatedAtMs">> = {},
): MediaQueueRunState {
  return {
    status,
    mode: runMode,
    message: details.message ? [...details.message].slice(0, 1024).join("") : null,
    outputPath: details.outputPath ?? null,
    reportPath: details.reportPath ?? null,
    reportStatus: details.reportStatus ?? null,
    updatedAtMs: Date.now(),
  };
}

export function runProgressStatus(phase: string, runMode: MediaQueueRunState["mode"]): MediaQueueRunStatus {
  const value = phase.toLocaleLowerCase();
  if (/validat|verified output|output installed|transaction commit|picture quality/.test(value)) return "validating";
  if (/remux|mux|metadata embedding|embedding metadata|copying streams/.test(value)) return "remuxing";
  if (/encod|calibrat|hdr10\+ metadata extraction|finalizing/.test(value)) return "rebuilding";
  if (runMode === "check" || /analy[sz]|reading media tracks|storage preflight/.test(value)) return "scanning";
  return "running";
}
