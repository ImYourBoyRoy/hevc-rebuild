// ./desktop/src/lib/state/app-derived.svelte.ts
// Every route-level `$derived` value (visible/paged entries, queue and run batches, selection and
// disk budgets, library cards and crumbs, workflow flags, setup labels, progress), moved out of the
// route. The state instances and the few route helpers they need are injected, and each field is
// computed lazily on first read, exactly as the route's `let x = $derived(...)` declarations were.
import type { CredentialsState } from "./credentials-state.svelte.ts";
import type { SettingsState } from "./settings-state.svelte.ts";
import type { ToolSetupState } from "./tool-setup-state.svelte.ts";
import type { ArtworkState } from "./artwork-state.svelte.ts";
import type { AppState } from "./app-state.svelte.ts";
import type { ScanState } from "./scan-state.svelte.ts";
import type { ReviewState } from "./review-state.svelte.ts";
import type { LibraryUiState } from "./library-ui-state.svelte.ts";
import type { RunUiState } from "./run-ui-state.svelte.ts";
import type { YoutubeState } from "./youtube-state.svelte.ts";
import type { ReportsState } from "./reports-state.svelte.ts";
import type { createWorkspaceState } from "../workspace.svelte.js";
import type { MediaEntry, YouTubeEstimate, LibraryPosterCard, LibraryCrumb } from "../types.js";
import { queuePathKey } from "../queue-paths.js";
import { runProgressStatus } from "../queue-run-state.js";
import type { SourceRunBatch } from "../run-controller.js";
import { groupLibraryEntries } from "../library-browse.js";
import { latestFailureRows } from "../latest-outcomes.js";
import { finiteNumber } from "../telemetry-format.js";

export interface AppDerivedSources {
  workspace: ReturnType<typeof createWorkspaceState>;
  credentials: CredentialsState;
  settingsState: SettingsState;
  toolSetup: ToolSetupState;
  artwork: ArtworkState;
  appState: AppState;
  scanState: ScanState;
  reviewState: ReviewState;
  libraryUi: LibraryUiState;
  runUi: RunUiState;
  youtubeState: YoutubeState;
  reportsState: ReportsState;
}

export interface AppDerivedDeps {
  hasHdr10plusSignal: (entry: MediaEntry) => boolean;
  youtubeHdr10plusLossAccepted: (path: string) => boolean;
  youtubeEstimateFor: (entry: MediaEntry) => YouTubeEstimate | undefined;
  buildLibraryPosterCards: () => LibraryPosterCard[];
  buildLibraryCrumbs: () => LibraryCrumb[];
  lookupMessageText: () => string;
}

export class AppDerived {
  declare readonly workspace: AppDerivedSources["workspace"];
  declare readonly credentials: CredentialsState;
  declare readonly settingsState: SettingsState;
  declare readonly toolSetup: ToolSetupState;
  declare readonly artwork: ArtworkState;
  declare readonly appState: AppState;
  declare readonly scanState: ScanState;
  declare readonly reviewState: ReviewState;
  declare readonly libraryUi: LibraryUiState;
  declare readonly runUi: RunUiState;
  declare readonly youtubeState: YoutubeState;
  declare readonly reportsState: ReportsState;
  declare readonly d: AppDerivedDeps;

  constructor(sources: AppDerivedSources, deps: AppDerivedDeps) {
    this.workspace = sources.workspace;
    this.credentials = sources.credentials;
    this.settingsState = sources.settingsState;
    this.toolSetup = sources.toolSetup;
    this.artwork = sources.artwork;
    this.appState = sources.appState;
    this.scanState = sources.scanState;
    this.reviewState = sources.reviewState;
    this.libraryUi = sources.libraryUi;
    this.runUi = sources.runUi;
    this.youtubeState = sources.youtubeState;
    this.reportsState = sources.reportsState;
    this.d = deps;
  }

  activeMatchSuggestionCount = $derived.by(() => (this.scanState.scanResult?.entries.filter((entry) => {
    const suggestion = this.reviewState.queueMatchSuggestions[entry.path];
    return suggestion?.status === "ready" && Boolean(suggestion.title);
  }).length ?? 0));

  recoveryCurrentItem = $derived.by(() => (this.runUi.recoveryReviewItems[this.runUi.recoveryReviewIndex] ?? null));

  recoveryReportStatus = $derived.by(() => ((this.recoveryCurrentItem?.report?.status || this.recoveryCurrentItem?.state.reportStatus || "").trim().toUpperCase()));

  recoveryCanMarkComplete = $derived.by(() => (Boolean(this.recoveryCurrentItem?.hasRecordedOutput) && ["OK", "METADATA_OK", "TEST_OK"].includes(this.recoveryReportStatus) && this.runUi.recoveryProbeEntry?.status === "Ready"));

  seriesReviewsByRoot = $derived.by(() => new Map(Object.entries(this.reviewState.queueSeriesReviews).map(([path, review]) => [queuePathKey(path), review])));

  queueRunStatesByPath = $derived.by(() => new Map(Object.entries(this.workspace.queue.runStates).map(([path, state]) => [queuePathKey(path), state])));

  visibleEntries = $derived.by(() => ((this.scanState.scanResult?.entries ?? []).filter((entry) => {
    const query = this.libraryUi.search.trim().toLowerCase();
    return !query || entry.relative_path.toLowerCase().includes(query) || entry.status.toLowerCase().includes(query) || (entry.resource_forecast?.level ?? "").includes(query);
  })));

  mediaPageCount = $derived.by(() => (Math.max(1, Math.ceil(this.visibleEntries.length / this.libraryUi.mediaPageSize))));

  currentPageEntries = $derived.by(() => (this.visibleEntries.slice((this.libraryUi.mediaPage - 1) * this.libraryUi.mediaPageSize, this.libraryUi.mediaPage * this.libraryUi.mediaPageSize)));

  mediaPageFirst = $derived.by(() => (this.visibleEntries.length ? (this.libraryUi.mediaPage - 1) * this.libraryUi.mediaPageSize + 1 : 0));

  mediaPageLast = $derived.by(() => (Math.min(this.libraryUi.mediaPage * this.libraryUi.mediaPageSize, this.visibleEntries.length)));

  queuedEntries = $derived.by(() => {
    const unique = new Map<string, MediaEntry>();
    for (const source of this.workspace.queue.sources) {
      for (const entry of source.scanResult?.entries ?? []) {
        if (!unique.has(entry.path)) unique.set(entry.path, entry);
      }
    }
    return [...unique.values()];
  });

  queuedEntryCount = $derived.by(() => (this.queuedEntries.length));

  queueStatusCounts = $derived.by(() => {
    const labels: Record<string, string> = {
      queued: "Queued", scanning: "Scanning", renaming: "Renaming", remuxing: "Copying streams",
      rebuilding: "Rebuilding", validating: "Validating", renamed: "Renamed", completed: "Complete",
      failed: "Failed", skipped: "Skipped", interrupted: "Interrupted", needs_review: "Needs review",
      running: "In progress", ready: "Ready",
    };
    const counts = new Map<string, number>();
    for (const entry of this.queuedEntries) {
      const saved = this.queueRunStatesByPath.get(queuePathKey(entry.path));
      const activeProgress = this.workspace.run.progress?.filePath && queuePathKey(this.workspace.run.progress.filePath) === queuePathKey(entry.path) ? this.workspace.run.progress : null;
      const source = this.workspace.queue.sources.find((candidate) => candidate.scanResult?.entries.some((item) => queuePathKey(item.path) === queuePathKey(entry.path)));
      const suggestion = this.reviewState.queueMatchSuggestions[entry.path];
      const key = activeProgress
        ? runProgressStatus(activeProgress.phase, saved?.mode ?? this.workspace.run.mode)
        : saved?.status
          ?? (source?.status === "scanning" || suggestion?.status === "searching" ? "scanning"
            : entry.queue_stale || entry.error || entry.status !== "Ready" || suggestion?.status === "error" ? "needs_review" : "ready");
      counts.set(key, (counts.get(key) ?? 0) + 1);
    }
    const order = ["needs_review", "scanning", "queued", "renaming", "remuxing", "rebuilding", "validating", "running", "ready", "renamed", "completed", "skipped", "failed", "interrupted"];
    return [...counts.entries()]
      .sort(([left], [right]) => {
        const leftOrder = order.indexOf(left);
        const rightOrder = order.indexOf(right);
        return (leftOrder < 0 ? order.length : leftOrder) - (rightOrder < 0 ? order.length : rightOrder);
      })
      .map(([key, count]) => ({ key, label: labels[key] ?? key, count }));
  });

  runReviewEntries = $derived.by(() => (this.queuedEntries.filter((entry) => {
    const status = this.queueRunStatesByPath.get(queuePathKey(entry.path))?.status;
    return status === "queued" || status === "failed" || status === "skipped" || status === "interrupted" || status === "needs_review";
  })));

  deferredQueueCount = $derived.by(() => (this.queuedEntries.filter((entry) => {
    const state = this.queueRunStatesByPath.get(queuePathKey(entry.path));
    return state?.mode === this.runUi.mode && ["completed", "failed", "skipped", "interrupted", "needs_review"].includes(state.status);
  }).length));

  queuedInputBytes = $derived.by(() => (this.queuedEntries.reduce((sum, entry) => sum + entry.size_bytes, 0)));

  queuedFreeBytes = $derived.by(() => (Math.min(...this.workspace.queue.sources.map((source) => source.scanResult?.disk_free_bytes ?? Infinity))));

  queueHasScans = $derived.by(() => (this.workspace.queue.sources.some((source) => source.scanResult)));

  queuedNeedsReview = $derived.by(() => (this.queuedEntries.filter((entry) => entry.error || entry.status === "Tools required" || entry.status === "Backed up").length + this.workspace.queue.sources.filter((source) => source.changedFiles.length || source.missingFiles.length || source.status === "interrupted" || source.status === "failed").length));

  queuedSourceCount = $derived.by(() => (this.workspace.queue.sources.length));

  queueSelectionCount = $derived.by(() => (this.workspace.queue.selectedPaths.length));

  runBatches = $derived.by(() => {
    const selected = new Set(this.workspace.queue.selectedPaths);
    const seenPaths = new Set<string>();
    return this.workspace.queue.sources.flatMap((source): SourceRunBatch[] => {
      if (source.status !== "ready" || !source.scanResult) return [];
      const missing = new Set(source.missingFiles);
      const changed = new Set(source.changedFiles);
      const entries = source.scanResult.entries
        .filter((entry) => {
          const key = queuePathKey(entry.path);
          if (seenPaths.has(key)) return false;
          seenPaths.add(key);
          if (selected.size) return selected.has(entry.path);
          const state = this.queueRunStatesByPath.get(key);
          if (state?.mode === this.runUi.mode && ["completed", "failed", "skipped", "interrupted", "needs_review"].includes(state.status)) return false;
          return true;
        })
        .map((entry) => ({
          ...entry,
          queue_stale: missing.has(entry.path) ? "missing" as const : changed.has(entry.path) ? "changed" as const : undefined,
        }));
      return entries.length ? [{ source, entries }] : [];
    });
  });

  selectedRunEntries = $derived.by(() => (this.runBatches.flatMap((batch) => batch.entries)));

  selectedMediaPathsForRun = $derived.by(() => (this.workspace.queue.selectedPaths.filter((path) => this.selectedRunEntries.some((entry) => entry.path === path))));

  selectedRunInputBytes = $derived.by(() => (this.selectedRunEntries.reduce((total, entry) => total + entry.size_bytes, 0)));

  selectionInOtherSources = $derived.by(() => (this.runBatches.length > 1));

  selectionHasNoRunnableFiles = $derived.by(() => (this.workspace.queue.selectedPaths.length > 0 && this.selectedMediaPathsForRun.length === 0));

  unresolvedSelectedPaths = $derived.by(() => (this.workspace.queue.selectedPaths.length - this.selectedMediaPathsForRun.length));

  selectedRunFreeBytes = $derived.by(() => (Math.min(...this.runBatches.map((batch) => batch.source.scanResult?.disk_free_bytes ?? Infinity))));

  selectedRunDiskBudget = $derived.by(() => {
    const oneGiB = 1024 ** 3;
    const ratio = this.settingsState.settings?.resources.preflight_output_ratio ?? 1.3;
    const reserve = Math.max(0, this.settingsState.settings?.resources.min_free_disk_gib ?? 0) * oneGiB;
    const largest = this.selectedRunEntries.reduce((max, entry) => Math.max(max, entry.size_bytes || 0), 0);
    const fromSource = Math.trunc(largest * ratio);
    return {
      largest,
      working: largest > 0 ? reserve + Math.max(2 * oneGiB, fromSource) : 0,
      floored: largest > 0 && fromSource < 2 * oneGiB,
    };
  });

  hasQueueCandidates = $derived.by(() => (this.runBatches.some((batch) => (batch.source.scanResult?.candidate_count ?? 0) > 0)));

  staleRunSources = $derived.by(() => (this.runBatches.filter((batch) => batch.entries.some((entry) => entry.queue_stale))));

  runPlanPreviewByRoot = $derived.by(() => {
    let remaining = 250;
    return new Map(this.runBatches.map((batch) => {
      const preview = batch.entries.slice(0, remaining);
      remaining -= preview.length;
      return [batch.source.root, preview] as const;
    }));
  });

  librarySeriesGroups = $derived.by(() => (this.scanState.scanResult ? groupLibraryEntries(this.scanState.scanResult.root, this.visibleEntries) : []));

  libraryPosterCards = $derived.by(() => this.d.buildLibraryPosterCards());

  selectedVisibleCount = $derived.by(() => (this.libraryPosterCards.filter((card) => card.selection === "all").length));

  libraryCrumbs = $derived.by((): LibraryCrumb[] => this.d.buildLibraryCrumbs());

  libraryLookupMessage = $derived.by(() => (this.d.lookupMessageText()));

  selectedFilteredCount = $derived.by(() => (this.visibleEntries.filter((entry) => this.workspace.queue.selectedPaths.includes(entry.path)).length));

  staleSelectedEntries = $derived.by(() => (this.selectedRunEntries.filter((entry) => entry.queue_stale)));

  youtubeHdr10plusHelperReady = $derived.by(() => (
    (this.toolSetup.environment?.checks ?? []).some((check) => check.key === "hdr10plus_tool" && check.available)));

  youtubeEstimateSummary = $derived.by(() => {
    if (!this.runBatches.length || this.youtubeState.youtubeMode === "optimize") return null;
    const entries = this.selectedRunEntries.filter((entry) =>
      entry.profile &&
      !entry.profile.hdr.dolby_vision &&
      (!this.d.hasHdr10plusSignal(entry) || this.youtubeHdr10plusHelperReady || this.d.youtubeHdr10plusLossAccepted(entry.path)),
    );
    const estimates = entries.map(this.d.youtubeEstimateFor).filter((value) => value !== undefined);
    return {
      files: estimates.length,
      sourceBytes: entries.reduce((sum, entry) => sum + entry.size_bytes, 0),
      outputBytes: estimates.reduce((sum, estimate) => sum + estimate.estimated_output_bytes, 0),
    };
  });

  analysisErrors = $derived.by(() => ((this.scanState.scanResult?.entries ?? []).filter((entry) => entry.error)));

  toolsRequiredEntries = $derived.by(() => ((this.scanState.scanResult?.entries ?? []).filter((entry) => entry.status === "Tools required")));

  highMemoryEntries = $derived.by(() => ((this.scanState.scanResult?.entries ?? []).filter((entry) => entry.resource_forecast?.level === "high" || entry.resource_forecast?.level === "blocked")));

  constrainedMemoryEntries = $derived.by(() => ((this.scanState.scanResult?.entries ?? []).filter((entry) => entry.resource_forecast?.level === "constrained")));

  // Only a file's most recent decisive outcome counts, so a failure that a later run fixed (or repeated attempts at
  // one file) does not stay on the Reports screen and the sidebar badge forever. See lib/latest-outcomes.ts.
  failedRows = $derived.by(() => latestFailureRows(this.reportsState.reports));

  activeReport = $derived.by(() => (this.reportsState.reports.find((report) => report.csv_path === this.reportsState.selectedReportPath) ?? this.reportsState.reports[0] ?? null));

  settingsChanged = $derived.by(() => (Boolean(this.settingsState.settings && this.settingsState.settingsBaseline && JSON.stringify(this.settingsState.settings) !== this.settingsState.settingsBaseline)));

  forecastOutdated = $derived.by(() => (Boolean(this.scanState.scanResult && this.settingsState.settings && (
    JSON.stringify(this.scanState.scanResult.settings.resources) !== JSON.stringify(this.settingsState.settings.resources) ||
    this.scanState.scanResult.settings.video.preset !== this.settingsState.settings.video.preset ||
    this.scanState.scanResult.settings.video.promote_8bit_to_10bit !== this.settingsState.settings.video.promote_8bit_to_10bit
  ))));

  queueRenameBusy = $derived.by(() => (Object.values(this.workspace.queue.runStates).some((state) => state.status === "renaming")));

  workflowBusy = $derived.by(() => (Boolean(this.scanState.scanning || this.workspace.run.starting || this.workspace.run.id || this.toolSetup.repairId || this.toolSetup.hdrInstalling || this.runUi.outputPreviewBusy || this.queueRenameBusy)));

  modeToolsReady = $derived.by(() => {
    if (this.runUi.mode === "youtube") return this.toolSetup.environment?.youtube_ready;
    if (this.runUi.mode === "check" || this.runUi.mode === "integrity") return this.toolSetup.environment?.can_analyze && this.toolSetup.environment.checks.some((check) => check.key === "ffmpeg" && check.available);
    if (this.runUi.mode === "metadata") return this.toolSetup.environment?.metadata_ready;
    return this.toolSetup.environment?.ready;
  });

  canStart = $derived.by(() => (Boolean(this.modeToolsReady && this.hasQueueCandidates && this.selectedRunEntries.length > 0 && this.unresolvedSelectedPaths === 0 && !this.staleSelectedEntries.length && !this.scanState.scanning && !this.workspace.run.starting && !this.workspace.run.id && !this.toolSetup.repairId && !this.toolSetup.hdrInstalling && !this.runUi.outputPreviewBusy)));

  overallRunPercent = $derived.by(() => {
    if (!this.workspace.run.progress) return null;
    const sourcePercent = this.workspace.run.progress.queuePercent ?? this.workspace.run.progress.filePercent;
    if (!finiteNumber(sourcePercent)) return null;
    const totalFiles = this.runUi.pendingRunBatches.reduce((sum, batch) => sum + batch.entries.length, 0);
    if (!totalFiles || this.runUi.activeRunBatchIndex < 0) return Math.max(0, Math.min(100, sourcePercent));
    const completedFiles = this.runUi.pendingRunBatches.slice(0, this.runUi.activeRunBatchIndex).reduce((sum, batch) => sum + batch.entries.length, 0);
    const currentFiles = this.runUi.pendingRunBatches[this.runUi.activeRunBatchIndex]?.entries.length ?? this.workspace.run.progress.batchTotal;
    return Math.max(0, Math.min(100, ((completedFiles + currentFiles * sourcePercent / 100) / totalFiles) * 100));
  });

  encodeMeasured = $derived.by(() => (Boolean(
    this.workspace.run.progress && (
      (finiteNumber(this.workspace.run.progress.filePercent) && this.workspace.run.progress.filePercent > 0) ||
      finiteNumber(this.workspace.run.progress.speed) ||
      (finiteNumber(this.workspace.run.progress.outputBytes) && this.workspace.run.progress.outputBytes > 0)
    ),
  )));

  mutatingRun = $derived.by(() => (this.runUi.mode === "metadata" || this.runUi.mode === "test" || this.runUi.mode === "full" || this.runUi.mode === "youtube"));

  progressElapsedLabel = $derived.by(() => {
    const base = this.workspace.run.progress?.elapsedSeconds ?? 0;
    if (!finiteNumber(base)) return "—";
    const since = this.workspace.run.progressSeenAt ?? this.workspace.run.startedAt;
    const extra = since ? Math.max(0, (this.runUi.progressClock - since) / 1000) : 0;
    const whole = Math.max(0, Math.round(base + extra));
    const minutes = Math.floor(whole / 60);
    const seconds = whole % 60;
    return minutes ? `${minutes}m ${seconds}s` : `${seconds}s`;
  });

  instrumentTheme = $derived.by(() => (this.appState.theme === "phosphor" || this.appState.theme === "ion" || this.appState.theme === "breach"));

  missingRequiredTools = $derived.by(() => ((this.toolSetup.environment?.checks ?? []).filter((check) => check.required && !check.available)));

  aptHost = $derived.by(() => (this.toolSetup.environment?.os.supported === true && this.toolSetup.environment?.os.package_manager === "apt" && this.toolSetup.environment?.os.package_manager_available !== false));

  hostSetupShell = $derived.by(() => (this.toolSetup.environment?.os.package_manager === "winget" ? "PowerShell" : "Terminal"));

  hostSetupCommand = $derived.by(() => (
    this.toolSetup.environment?.os.package_manager === "winget"
      ? ["Gyan.FFmpeg", "MediaArea.MediaInfo", "MoritzBunkus.MKVToolNix"].map((id) => `winget install --exact --id ${id} --accept-source-agreements --accept-package-agreements`).join("\n")
      : this.toolSetup.environment?.os.package_manager === "brew"
        ? "brew install ffmpeg mediainfo mkvtoolnix fontconfig"
        : ""));

  packageManagerLabel = $derived.by(() => (this.toolSetup.environment?.os.package_manager === "winget" ? "winget" : this.toolSetup.environment?.os.package_manager === "brew" ? "Homebrew" : "APT"));

  setupGapLabel = $derived.by(() => {
    if (!this.toolSetup.environment) return "SETUP NEEDED";
    if (this.missingRequiredTools.length === 1) return `${this.missingRequiredTools[0].label} missing`;
    if (this.missingRequiredTools.length > 1) return `${this.missingRequiredTools.length} tools missing`;
    if (!this.toolSetup.environment.os.supported) return "Unsupported system";
    if (this.toolSetup.environment.missing_apt_packages.length) return `${this.toolSetup.environment.missing_apt_packages.length} packages missing`;
    return "SETUP NEEDED";
  });

  setupGapDetail = $derived.by(() => (
    this.missingRequiredTools.map((check) => check.label).join(", ")
      || this.toolSetup.environment?.missing_apt_packages.join(", ")
      || "Open environment setup"));
}
