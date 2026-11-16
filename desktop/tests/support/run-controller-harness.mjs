// ./desktop/tests/support/run-controller-harness.mjs
// Shared fixtures and the fake-host harness for the run-controller tests (not a test file).

import { createRunController } from "../../src/lib/run-controller.ts";
import { RunEventBuffer, initialWorkspaceState } from "../../src/lib/workspace-state.ts";

export const entry = (path, extra = {}) => ({ path, name: path.split("/").at(-1), relative_path: path.split("/").at(-1), size_bytes: 100, ...extra });
export const source = (root, paths, extra = {}) => ({
  root, status: "ready", error: null, updatedAtMs: 1, changedFiles: [], missingFiles: [], excludedPaths: [], entryOrder: [],
  scanResult: { root, entries: paths.map((path) => entry(path)), candidate_count: paths.length, recursive: true, minimum_size_mib: 5, settings: { id: `settings-${root}` } },
  ...extra,
});
export const batchOf = (src) => ({ source: src, entries: src.scanResult.entries });
export const state = (status, extra = {}) => ({ status, mode: "full", message: null, outputPath: null, reportPath: null, reportStatus: null, updatedAtMs: 1, ...extra });
export const finished = (extra = {}) => ({ runId: "run-1", exitCode: 0, stopRequested: false, logPath: null, reportPath: null, error: null, ...extra });
export const report = (csv, rows) => ({ csv_path: csv, log_path: `${csv}.log`, created: 1, counts: {}, rows });

export function harness({ invokeImpl, hostOverrides = {}, workspaceSetup, persistResults = [] } = {}) {
  const calls = [];
  const workspace = initialWorkspaceState();
  workspace.queue.loaded = true;
  workspaceSetup?.(workspace);
  const host = {
    settings: { id: "active-settings", metadata: { comment: "" } }, mode: "check", pendingRunBatches: [], activeRunBatchIndex: -1, pendingRunFullConfirmed: false,
    recoveryReviewedPaths: new Set(["x"]), queueRunLogs: {}, abortConfirmOpen: true, queuedEntries: [], runBatches: [], reports: [], queueRunStatesByPath: new Map(),
    recursive: true, minimumSizeMib: "5", youtubeResolution: "1440p", youtubeQualityMode: "balanced", youtubeSourceMultiplier: 2, youtubeMode: "upscale",
    youtubeSdrCodec: "h264", youtubePreset: "slow", youtubeShrink: "auto", youtubeQualityCheck: "standard", youtubeOutputDir: "  ", stopOnError: false,
    integrityLevel: "quick", fileStreamOverrides: {}, fileSettingsOverrides: {}, youtubeHdr10plusLossPaths: new Set(), workflowBusy: false,
    appPreferences: { rebuildOutputDir: "/out", fileOutputDirs: {}, filenameIncludePatterns: ["inc"], filenameExcludePatterns: ["exc"] },
    setNotice: (message, kind = "info") => calls.push(["notice", kind, message]),
    openPage: (page) => calls.push(["openPage", page]),
    currentToolPaths: () => ({ ffmpeg: null }),
    loadReports: async () => { calls.push(["loadReports"]); },
    selectedPosterOverrides: (entries) => Object.fromEntries(entries.map((item) => [item.path, "poster"])),
    selectedMetadataOverrides: (entries) => Object.fromEntries(entries.map((item) => [item.path, { title: "T" }])),
    fileTuneOverridesForBatch: (entries) => Object.fromEntries(entries.map((item) => [item.path, "none"])),
    openLog: async (path) => { calls.push(["openLog", path]); },
    openMetadataForEntry: async (item) => { calls.push(["openMetadata", item.path]); },
    chooseReport: (path) => calls.push(["chooseReport", path]),
    ...hostOverrides,
  };
  const invokes = [];
  const invoke = async (command, args) => {
    invokes.push({ command, args });
    if (invokeImpl) return invokeImpl(command, args);
    if (command === "start_run") return "run-1";
    return null;
  };
  let persistIndex = 0;
  const persistence = {
    persistQueueRunStates: async (updates) => {
      workspace.queue.runStates = { ...workspace.queue.runStates, ...updates };
      calls.push(["persistStates", Object.fromEntries(Object.entries(updates).map(([path, item]) => [path, item.status]))]);
      return persistResults[persistIndex++] ?? true;
    },
    persistQueueSources: async () => { calls.push(["persistSources"]); return persistResults[persistIndex++] ?? true; },
  };
  const queue = { activateQueueSource: (root) => calls.push(["activate", root]) };
  const runEventBuffer = new RunEventBuffer();
  const controller = createRunController({ invoke, workspace, host, persistence, queue, runEventBuffer });
  return { controller, host, workspace, calls, invokes, runEventBuffer };
}

export const progressEvent = (extra = {}) => ({ runId: "run-1", fileName: "a.mkv", filePath: "/r/a.mkv", phase: "Validating output", batchNumber: 1, batchTotal: 1, filePercent: 50, queuePercent: 50, speed: null, etaSeconds: null, elapsedSeconds: null, inputBytes: null, outputBytes: null, estimatedOutputBytes: null, ...extra });

export const queuedHarness = (overrides = {}, setup) => {
  const src = source("/r", ["/r/a.mkv", "/r/b.mkv"]);
  return harness({ hostOverrides: { mode: "full", runBatches: [batchOf(src)], ...overrides }, workspaceSetup: (workspace) => { workspace.queue.sources = [src]; workspace.queue.activeRoot = "/r"; setup?.(workspace); } });
};

export const runningHarness = (overrides = {}, setup) => {
  const src = source("/r", ["/r/a.mkv", "/r/b.mkv", "/r/c.mkv"]);
  return harness({
    hostOverrides: { mode: "full", pendingRunBatches: [batchOf(src)], activeRunBatchIndex: 0, ...overrides },
    workspaceSetup: (workspace) => { workspace.run.id = "run-1"; workspace.run.mode = "full"; workspace.queue.sources = [src]; setup?.(workspace); },
  });
};

export const retryHarness = (path = "/r/a.mkv", overrides = {}, setup) => {
  const src = source("/r", [path]);
  return harness({ hostOverrides: { mode: "check", ...overrides }, workspaceSetup: (workspace) => { workspace.queue.sources = [src]; workspace.queue.selectedPaths = ["/keep"]; setup?.(workspace); } });
};
