<script lang="ts">
  import { onMount, tick } from "svelte";
  import { invoke, isTauri } from "@tauri-apps/api/core";
  import { listen, TauriEvent, type UnlistenFn } from "@tauri-apps/api/event";
  import { Menu, type MenuOptions } from "@tauri-apps/api/menu";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { currentMonitor, getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
  import type { Theme, MetadataSources, TmdbCredentialStatus, TvdbCredentialStatus, AppPreferences, PreferencesPayload, RecoveryReviewItem, RepairToolOutcome, ToolSetupLogSummary } from "#lib/ui-contracts.js";
  import SettingsPreferences from "#lib/SettingsPreferences.svelte";
  import ReportBrowser from "#lib/ReportBrowser.svelte";
  import ToolRepairActivity from "#lib/ToolRepairActivity.svelte";
  import ToolRepairConfirmation from "#lib/ToolRepairConfirmation.svelte";
  import RecoveryReview from "#lib/RecoveryReview.svelte";
  import HelpCenter from "#lib/HelpCenter.svelte";
  import HomeDashboard from "#lib/HomeDashboard.svelte";
  import { insertRenameTemplateToken } from "#lib/rename-template.js";
  import LibraryRenameDialog from "#lib/LibraryRenameDialog.svelte";
  import LibraryQueueStatus from "#lib/LibraryQueueStatus.svelte";
  import LibraryPosterGrid from "#lib/LibraryPosterGrid.svelte";
  import LibraryTvReview from "#lib/LibraryTvReview.svelte";
  import { episodeIdentity, episodeOutputText, fillEpisodeFromCatalog, groupLibraryEntries, preferredLocalPoster, seasonPosterFromArtwork, type LibrarySeriesGroup } from "#lib/library-browse.js";
  import { createWorkspaceState } from "#lib/workspace.svelte.js";
  import { RunEventBuffer, reportOutcome, selectQueuePaths } from "#lib/workspace-state.js";
  import Modal from "#lib/Modal.svelte";
  import MetadataWorkbench from "#lib/MetadataWorkbench.svelte";
  import YouTubePreparation from "#lib/YouTubePreparation.svelte";
  import RunConsole from "#lib/RunConsole.svelte";
  import RunProgress from "#lib/RunProgress.svelte";
  import { finiteNumber } from "#lib/telemetry-format.js";
  import {
    Activity, AlertTriangle, ArrowRight, BookOpen, Check, CheckCircle2, ChevronRight,
    Clapperboard, Cpu, Database, FileText, Film, FolderOpen, Gauge, HardDrive,
    History, House, Info, Layers, LoaderCircle, MemoryStick, Monitor, Moon, Play, RefreshCw,
    ScanSearch, Search, Settings2, ShieldCheck, Square, Sun, Terminal, TerminalSquare, Timer,
    Trash2, Video, Wrench, X, Zap,
  } from "@lucide/svelte";
          import type {
    OrganizerTemplates, LibraryCrumb, LibraryPosterCard, AptUpdateStatus, ArtworkCacheInfo, ArtworkCachePurgeResult, CommandCheck, Environment, MediaEntry, Page, ReportRow, RunFinishedEvent, RunLineEvent,
    InterruptedRecovery, MediaQueueDocument, MediaQueueLoadResult, MediaQueueRunState, MediaQueueRunStatus, MediaQueueSource, MetadataLookupStatus, OutputPreview, QueueArtworkCandidate, QueueMetadataSuggestion, QueueMovieReview, QueueSeasonReview, QueueSeriesReview, QueueTvEpisode, RunProgressEvent, RunResourceEvent, RuntimeResourceSnapshot,
    RunMode, RunReport, ScanProgress, ScanResult, Settings, ToolPaths,
  } from "#lib/types.js";

  type SettingsLoad = { settings: Settings; sources: string[]; user_config: string };
  type ReportsPayload = { reports: RunReport[] };
type RepairFinished = { jobId: string; success: boolean; error: string | null; logWarning?: string | null };

  type SourceRunBatch = { source: MediaQueueSource; entries: MediaEntry[] };
  type LocalAssetRecord = {
    path: string;
    artwork: { name: string; path: string; role: string }[];
    metadata: {
      found: boolean;
      title: string | null;
      showTitle: string | null;
      year: string | null;
      certification: string | null;
      plot: string | null;
      genres: string[];
      rating: number | null;
      season: number | null;
      episode: number | null;
      airdate: string | null;
      seasonNames: Record<string, string>;
    };
  };
  type LocalLibraryRecord = {
    title: string;
    showTitle: string;
    year: string;
    certification: string;
    plot: string;
    genresText: string;
    ratingText: string;
    season: number | null;
    episode: number | null;
    airdate: string | null;
    seasonNames: Record<string, string>;
    artwork: { name: string; path: string; role: string }[];
  };
  type LibraryTvReviewTarget = { level: "series" | "season" | "episode"; seriesRoot: string; season: number | null; episodePath: string | null };

  type TvdbCredentials = { apiKey: string; subscriberPin: string | null };



  const navigation: { id: Page; label: string; icon: typeof Activity }[] = [
    { id: "home", label: "Home", icon: House },
    { id: "library", label: "Media library", icon: Film },
    { id: "metadata", label: "Identify & organize", icon: Database },
    { id: "youtube", label: "YouTube prep", icon: Video },
    { id: "run", label: "Run setup", icon: Settings2 },
    { id: "settings", label: "Settings", icon: Wrench },
    { id: "reports", label: "Reports & errors", icon: History },
    { id: "help", label: "Help & commands", icon: BookOpen },
  ];
  const toolPathOptions = [
    { key: "ffmpeg", label: "FFmpeg", note: "Encoder and muxer" },
    { key: "ffprobe", label: "FFprobe", note: "Stream analysis" },
    { key: "mediainfo", label: "MediaInfo", note: "Track metadata" },
    { key: "mkvpropedit", label: "MKVToolNix editor", note: "Matroska tags and attachments" },
    { key: "mkvmerge", label: "MKVToolNix merger", note: "Matroska remuxing" },
    { key: "mkvextract", label: "MKVToolNix extractor", note: "Metadata and track extraction" },
  ] as const;
  type ToolPathKey = typeof toolPathOptions[number]["key"];
  const browserPreferencesKey = "hevc-rebuild-studio.preview-preferences.v1";

  const workspace = createWorkspaceState();
  const runEventBuffer = new RunEventBuffer();
  let page = $state<Page>("home");
  let theme = $state<Theme>("light");
  const defaultOrganizerTemplates: OrganizerTemplates = {
    movie: "{title} [[({year} {certification})]]{ext}",
    series: "{series_title}",
    season: "Season {season:02}",
    episode: "S{season:02}E{episode:02}.{episode_title}{ext}",
  };
  const defaultFilenameExcludePatterns = [
    "old", ".*", ".renamethethings", "@eadir", "extras", "extra", "trailers", "trailer",
    "samples", "sample", "behind the scenes", "behindthescenes", "deleted scenes",
    "deleted", "interviews", "interview", "scenes", "clips", "clip", "featurettes",
    "featurette", "shorts", "short", "other", "theme-music", "thememusic", "backdrops",
    "extrafanart",
  ];
  let appPreferences = $state<AppPreferences>({ theme: "light", windowState: { width: 1600, height: 1000, maximized: false }, lastPage: "home", root: null, toolPaths: { ffmpeg: null, ffprobe: null, mediainfo: null, mkvpropedit: null, mkvmerge: null, mkvextract: null }, recursive: true, minimumSizeMib: "5", stopOnError: false, youtubeResolution: "1440p", youtubeQualityMode: "balanced", youtubeSourceMultiplier: 2, youtubeOutputDir: null, rebuildOutputDir: null, fileOutputDirs: {}, filenameIncludePatterns: [], filenameExcludePatterns: [...defaultFilenameExcludePatterns], filenameExcludeDefaultsVersion: 2, metadataAutoLookup: true, metadataLanguage: "en-US", metadataSources: { tmdb: true, wikidata: true, tvmaze: true, tvdb: false }, organizerTemplates: { ...defaultOrganizerTemplates }, savePosterBesideMedia: true, settings: null });
  let preferenceStorage = $state<PreferencesPayload | null>(null);
  let preferenceSaveQueue: Promise<void> = Promise.resolve();
  let windowStateSaveTimer: ReturnType<typeof setTimeout> | null = null;
  let windowPreferencesReady = false;
  let restoringWindowState = false;
  let booting = $state(true);
  let desktopShell = $state(false);
  let environment = $state<Environment | null>(null);
  let environmentLoading = $state(false);
  let environmentError = $state("");
  let tmdbCredentialStatus = $state<TmdbCredentialStatus>({ configured: false, secureStoreAvailable: false, message: null });
  let tmdbTokenDraft = $state("");
  let tmdbTokenVisible = $state(true);
  let tmdbCredentialBusy = $state(false);
  let tvdbCredentialStatus = $state<TvdbCredentialStatus>({ configured: false, subscriberPinConfigured: false, secureStoreAvailable: false, message: null });
  let tvdbApiKeyDraft = $state("");
  let tvdbSubscriberPinDraft = $state("");
  let tvdbCredentialsVisible = $state(true);
  let tvdbAccessConfirmed = $state(false);
  let tvdbCredentialBusy = $state(false);
  let metadataLanguageDraft = $state("en-US");
  let root = $state("");
  let recursive = $state(true);
  let minimumSizeMib = $state("5");
  let ffmpegPath = $state("");
  let ffprobePath = $state("");
  let mediainfoPath = $state("");
  let mkvpropeditPath = $state("");
  let mkvmergePath = $state("");
  let mkvextractPath = $state("");
  let settings = $state<Settings | null>(null);
  let settingsSources = $state<string[]>([]);
  let settingsPath = $state("");
  let settingsBaseline = $state("");
  let librarySettingsBaseline = $state("");
  let settingsRoot = $state("");
  let newMetadataBlacklistPattern = $state("");
  let metadataBlacklistError = $state("");
  let artworkCacheInfo = $state<ArtworkCacheInfo | null>(null);
  let artworkCacheLoading = $state(false);
  let artworkCachePurging = $state(false);
  let artworkCacheConfirm = $state(false);
  let artworkCacheError = $state("");
  let artworkCacheSettingsActive = false;
  let newFilenameIncludePattern = $state("");
  let newFilenameExcludePattern = $state("");
  let filenameFilterError = $state("");
  let scanning = $state(false);
  let scanProgress = $state<ScanProgress | null>(null);
  let scanResult = $state<ScanResult | null>(null);






  let queueWriteChain: Promise<unknown> = Promise.resolve();
  let metadataSuggestionSaveTimer: ReturnType<typeof setTimeout> | null = null;
  let metadataLookupPending = $state(false);
  let metadataLookupClaimed = false;
  let metadataFocusPath = $state<string | null>(null);
  let metadataDialogOpen = $state(false);
  let metadataDialogElement = $state<HTMLDialogElement | null>(null);
  let metadataLookupStatus = $state<MetadataLookupStatus | null>(null);
  let queueMatchSuggestions = $state<Record<string, QueueMetadataSuggestion>>({});
  let activeMatchSuggestionCount = $derived(scanResult?.entries.filter((entry) => {
    const suggestion = queueMatchSuggestions[entry.path];
    return suggestion?.status === "ready" && Boolean(suggestion.title);
  }).length ?? 0);

  let pendingRunBatches = $state<SourceRunBatch[]>([]);
  let activeRunBatchIndex = $state(-1);
  let pendingRunFullConfirmed = $state(false);
  let outputPreview = $state<OutputPreview | null>(null);
  let outputPreviewOpen = $state(false);
  let outputPreviewBusy = $state(false);
  let recoveryReviewOpen = $state(false);
  let recoveryReviewItems = $state<RecoveryReviewItem[]>([]);
  let recoveryReviewIndex = $state(0);
  let recoveryProbeEntry = $state<MediaEntry | null>(null);
  let recoveryProbeError = $state("");
  let recoveryProbeBusy = $state(false);
  let recoveryProbeAttempted = $state(false);
  let recoveryAcknowledged = $state(false);
  let pendingRecoveryFullConfirmed = $state(false);
  let recoveryReviewedPaths = $state<Set<string>>(new Set());
  let recoveryCurrentItem = $derived(recoveryReviewItems[recoveryReviewIndex] ?? null);
  let recoveryReportStatus = $derived((recoveryCurrentItem?.report?.status || recoveryCurrentItem?.state.reportStatus || "").trim().toUpperCase());
  let recoveryCanMarkComplete = $derived(Boolean(recoveryCurrentItem?.hasRecordedOutput) && ["OK", "METADATA_OK", "TEST_OK"].includes(recoveryReportStatus) && recoveryProbeEntry?.status === "Ready");

  let visibleSelectAllCheckbox = $state<HTMLInputElement | null>(null);
  let fileTuneOverrides = $state<Record<string, "animation" | "none">>({});
  let fileStreamOverrides = $state<Record<string, number[]>>({});
  let queueMovieReviews = $state<Record<string, QueueMovieReview>>({});
  let queueSeriesReviews = $state<Record<string, QueueSeriesReview>>({});

  let queueArtworkPreviews = $state<Record<string, string>>({});
  let libraryBrowse = $state<{ seriesRoot: string; season: number | null } | null>(null);
  let libraryTvReview = $state<LibraryTvReviewTarget | null>(null);
  let libraryLocalPosters = $state<Record<string, string>>({});
  let libraryLocalRecords = $state<Record<string, LocalLibraryRecord>>({});
  let libraryRenameDialog = $state<HTMLDialogElement | null>(null);
  let libraryRenameOpen = $state(false);
  let renameDraft = $state<OrganizerTemplates>({ ...defaultOrganizerTemplates });
  let moviePatternField = $state<HTMLInputElement | null>(null);
  let episodePatternField = $state<HTMLInputElement | null>(null);
  let libraryRenameBusy = $state(false);
  let libraryRenameError = $state("");
  let queueArtworkPreviewErrors = $state<Record<string, string>>({});
  let queueArtworkPreviewBusy = $state<Record<string, boolean>>({});
  let queueRunLogs = $state<Record<string, { loading: boolean; text: string; error: string }>>({});
  let seriesReviewsByRoot = $derived.by(() => new Map(Object.entries(queueSeriesReviews).map(([path, review]) => [queuePathKey(path), review])));
  let queueRunStatesByPath = $derived.by(() => new Map(Object.entries(workspace.queue.runStates).map(([path, state]) => [queuePathKey(path), state])));
  let youtubeHdr10plusLossPaths = $state<Set<string>>(new Set());
  let search = $state("");
  let mediaPage = $state(1);
  let mediaPageSize = $state(50);
  let mode = $state<RunMode>("check");

  let youtubeResolution = $state<"source" | "1440p" | "4k">("1440p");
  let youtubeQualityMode = $state<"balanced" | "recommended">("balanced");
  let youtubeSourceMultiplier = $state(2);
  let youtubeOutputDir = $state("");
  let stopOnError = $state(false);






  let progressClock = $state(Date.now());


  let fullConfirmOpen = $state(false);
  let runLaunchButton = $state<HTMLButtonElement | null>(null);
  let fullAck = $state(false);
  let repairConfirmOpen = $state(false);
  let aptUpdates = $state<AptUpdateStatus | null>(null);
  let aptChecking = $state(false);
  let continueToolchainSetupAfterRefresh = $state(false);
  let installHdrHelperAfterPackages = $state(false);
  let reports = $state<RunReport[]>([]);
  let selectedReportPath = $state("");
  let selectedLog = $state("");
  let reportDeleteConfirm = $state(false);
  let reportDeleteBusy = $state(false);
  let logLoading = $state(false);
  let repairId = $state("");
  let activeToolSetupLogId = $state("");
  let packageJobKind = $state<"repair" | "catalog-refresh" | "upgrade" | "toolchain" | null>(null);
  let repairLines = $state<string[]>([]);
  let repairSummary = $state("");
  let repairSummaryIsError = $state(false);
  let repairLogWarning = $state("");
  let toolSetupLogs = $state<ToolSetupLogSummary[]>([]);
  let selectedToolSetupLog = $state("");
  let viewedToolSetupLog = $state("");
  let toolSetupLogLoading = $state(false);
  let toolSetupLogError = $state("");
  let toolSetupLogExporting = $state(false);
  let repairToolChecksBefore = $state<CommandCheck[] | null>(null);
  let repairAffectedToolKeys = $state<string[]>([]);
  let repairToolOutcomes = $state<RepairToolOutcome[]>([]);
  let hdrInstalling = $state(false);


  let unlisteners: UnlistenFn[] = [];
  let contextMenu: Menu | null = null;

  let visibleEntries = $derived((scanResult?.entries ?? []).filter((entry) => {
    const query = search.trim().toLowerCase();
    return !query || entry.relative_path.toLowerCase().includes(query) || entry.status.toLowerCase().includes(query) || (entry.resource_forecast?.level ?? "").includes(query);
  }));
  let mediaPageCount = $derived(Math.max(1, Math.ceil(visibleEntries.length / mediaPageSize)));
  let currentPageEntries = $derived(visibleEntries.slice((mediaPage - 1) * mediaPageSize, mediaPage * mediaPageSize));
  let mediaPageFirst = $derived(visibleEntries.length ? (mediaPage - 1) * mediaPageSize + 1 : 0);
  let mediaPageLast = $derived(Math.min(mediaPage * mediaPageSize, visibleEntries.length));
  let queuedEntries = $derived.by(() => {
    const unique = new Map<string, MediaEntry>();
    for (const source of workspace.queue.sources) {
      for (const entry of source.scanResult?.entries ?? []) {
        if (!unique.has(entry.path)) unique.set(entry.path, entry);
      }
    }
    return [...unique.values()];
  });
  let queuedEntryCount = $derived(queuedEntries.length);
  let queueStatusCounts = $derived.by(() => {
    const labels: Record<string, string> = {
      queued: "Queued", scanning: "Scanning", renaming: "Renaming", remuxing: "Copying streams",
      rebuilding: "Rebuilding", validating: "Validating", renamed: "Renamed", completed: "Complete",
      failed: "Failed", skipped: "Skipped", interrupted: "Interrupted", needs_review: "Needs review",
      running: "In progress", ready: "Ready",
    };
    const counts = new Map<string, number>();
    for (const entry of queuedEntries) {
      const saved = queueRunStatesByPath.get(queuePathKey(entry.path));
      const activeProgress = workspace.run.progress?.filePath && queuePathKey(workspace.run.progress.filePath) === queuePathKey(entry.path) ? workspace.run.progress : null;
      const source = workspace.queue.sources.find((candidate) => candidate.scanResult?.entries.some((item) => queuePathKey(item.path) === queuePathKey(entry.path)));
      const suggestion = queueMatchSuggestions[entry.path];
      const key = activeProgress
        ? runProgressStatus(activeProgress.phase, saved?.mode ?? workspace.run.mode)
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
  let runReviewEntries = $derived(queuedEntries.filter((entry) => {
    const status = queueRunStatesByPath.get(queuePathKey(entry.path))?.status;
    return status === "queued" || status === "failed" || status === "skipped" || status === "interrupted" || status === "needs_review";
  }));
  let deferredQueueCount = $derived(queuedEntries.filter((entry) => {
    const state = queueRunStatesByPath.get(queuePathKey(entry.path));
    return state?.mode === mode && ["completed", "failed", "skipped", "interrupted", "needs_review"].includes(state.status);
  }).length);
  let queuedInputBytes = $derived(queuedEntries.reduce((sum, entry) => sum + entry.size_bytes, 0));
  let queuedFreeBytes = $derived(Math.min(...workspace.queue.sources.map((source) => source.scanResult?.disk_free_bytes ?? Infinity)));
  let queueHasScans = $derived(workspace.queue.sources.some((source) => source.scanResult));
  let queuedNeedsReview = $derived(queuedEntries.filter((entry) => entry.error || entry.status === "Tools required" || entry.status === "Backed up").length + workspace.queue.sources.filter((source) => source.changedFiles.length || source.missingFiles.length || source.status === "interrupted" || source.status === "failed").length);
  let queuedSourceCount = $derived(workspace.queue.sources.length);
  let queueSelectionCount = $derived(workspace.queue.selectedPaths.length);
  let runBatches = $derived.by(() => {
    const selected = new Set(workspace.queue.selectedPaths);
    const seenPaths = new Set<string>();
    return workspace.queue.sources.flatMap((source): SourceRunBatch[] => {
      if (source.status !== "ready" || !source.scanResult) return [];
      const missing = new Set(source.missingFiles);
      const changed = new Set(source.changedFiles);
      const entries = source.scanResult.entries
        .filter((entry) => {
          const key = queuePathKey(entry.path);
          if (seenPaths.has(key)) return false;
          seenPaths.add(key);
          if (selected.size) return selected.has(entry.path);
          const state = queueRunStatesByPath.get(key);
          if (state?.mode === mode && ["completed", "failed", "skipped", "interrupted", "needs_review"].includes(state.status)) return false;
          return true;
        })
        .map((entry) => ({
          ...entry,
          queue_stale: missing.has(entry.path) ? "missing" as const : changed.has(entry.path) ? "changed" as const : undefined,
        }));
      return entries.length ? [{ source, entries }] : [];
    });
  });
  let selectedRunEntries = $derived(runBatches.flatMap((batch) => batch.entries));
  let selectedMediaPathsForRun = $derived(workspace.queue.selectedPaths.filter((path) => selectedRunEntries.some((entry) => entry.path === path)));
  let selectedRunInputBytes = $derived(selectedRunEntries.reduce((total, entry) => total + entry.size_bytes, 0));
  let selectionInOtherSources = $derived(runBatches.length > 1);
  let selectionHasNoRunnableFiles = $derived(workspace.queue.selectedPaths.length > 0 && selectedMediaPathsForRun.length === 0);
  let unresolvedSelectedPaths = $derived(workspace.queue.selectedPaths.length - selectedMediaPathsForRun.length);
  let selectedRunFreeBytes = $derived(Math.min(...runBatches.map((batch) => batch.source.scanResult?.disk_free_bytes ?? Infinity)));
  let selectedRunDiskBudget = $derived.by(() => {
    const oneGiB = 1024 ** 3;
    const ratio = settings?.resources.preflight_output_ratio ?? 1.3;
    const reserve = Math.max(0, settings?.resources.min_free_disk_gib ?? 0) * oneGiB;
    const largest = selectedRunEntries.reduce((max, entry) => Math.max(max, entry.size_bytes || 0), 0);
    const fromSource = Math.trunc(largest * ratio);
    return {
      largest,
      working: largest > 0 ? reserve + Math.max(2 * oneGiB, fromSource) : 0,
      floored: largest > 0 && fromSource < 2 * oneGiB,
    };
  });
  let hasQueueCandidates = $derived(runBatches.some((batch) => (batch.source.scanResult?.candidate_count ?? 0) > 0));
  let staleRunSources = $derived(runBatches.filter((batch) => batch.entries.some((entry) => entry.queue_stale)));
  let runPlanPreviewByRoot = $derived.by(() => {
    let remaining = 250;
    return new Map(runBatches.map((batch) => {
      const preview = batch.entries.slice(0, remaining);
      remaining -= preview.length;
      return [batch.source.root, preview] as const;
    }));
  });
  let librarySeriesGroups = $derived(scanResult ? groupLibraryEntries(scanResult.root, visibleEntries) : []);
  let libraryPosterCards = $derived.by(() => {
    const groupedPaths = new Set(librarySeriesGroups.flatMap((group) => group.episodePaths));
    if (!libraryBrowse) {
      const seriesCards = librarySeriesGroups.map((group) => groupPosterCard(group));
      const movies = currentPageEntries.filter((entry) => !groupedPaths.has(entry.path)).map((entry) => libraryPosterCard(entry));
      return [...seriesCards, ...movies];
    }
    const group = librarySeriesGroups.find((item) => queuePathKey(item.root) === queuePathKey(libraryBrowse?.seriesRoot ?? ""));
    if (!group) return [];
    if (libraryBrowse.season === null) return group.seasons.map((season) => seasonPosterCard(group, season));
    const season = group.seasons.find((item) => item.season === libraryBrowse?.season);
    if (!season) return [];
    const seasonReview = reviewedSeriesForPath(season.episodePaths[0] ?? group.root);
    const seasonPoster = seasonArtworkPath(seasonReview, season.season, season.directory);
    return visibleEntries
      .filter((entry) => season.episodePaths.includes(entry.path))
      .map((entry) => episodePosterCard(entry, seasonPoster));
  });
  let selectedVisibleCount = $derived(libraryPosterCards.filter((card) => card.selection === "all").length);
  let libraryCrumbs = $derived.by((): LibraryCrumb[] => {
    if (!libraryBrowse) return [];
    const group = librarySeriesGroups.find((item) => queuePathKey(item.root) === queuePathKey(libraryBrowse?.seriesRoot ?? ""));
    const seriesLabel = group ? (reviewedSeriesForPath(group.episodePaths[0] ?? group.root)?.title.trim() || group.folderName) : "Series";
    const crumbs: LibraryCrumb[] = [
      { label: "All media", onSelect: () => { libraryBrowse = null; } },
      {
        label: seriesLabel,
        onSelect: libraryBrowse.season === null ? null : () => { libraryBrowse = { seriesRoot: libraryBrowse?.seriesRoot ?? "", season: null }; },
      },
    ];
      if (libraryBrowse.season !== null) {
      const season = group?.seasons.find((item) => item.season === libraryBrowse?.season);
      const savedTitle = group ? queueSeriesReviews[seriesReviewStorageKey(group.root)]?.seasonRecords?.[String(libraryBrowse.season)]?.title : "";
      crumbs.push({ label: savedTitle || season?.label || `Season ${libraryBrowse.season}`, onSelect: null });
    }
    return crumbs;
  });
  $effect(() => {
    const scan = scanResult;
    const groups = librarySeriesGroups;
    if (!scan || !groups.length) return;
    const directoryPaths = [...new Set(groups.flatMap((group) => [group.root, ...group.seasons.map((season) => season.directory).filter(Boolean)]))];
    const mediaPaths = groups.flatMap((group) => group.episodePaths);
    let cancelled = false;
    void invoke<{ directories: LocalAssetRecord[]; files: LocalAssetRecord[] }>("scan_local_library_assets", {
      request: { root: scan.root, mediaPaths, directoryPaths },
    }).then((found) => {
      if (cancelled) return;
      const next: Record<string, string> = {};
      const records: typeof libraryLocalRecords = {};
      const seasons = groups.flatMap((group) => group.seasons);
      for (const directory of found.directories) {
        const season = seasons.find((item) => item.directory && queuePathKey(item.directory) === queuePathKey(directory.path));
        const poster = preferredLocalPoster(directory.artwork, season ? season.season : null);
        if (poster) next[queuePathKey(directory.path)] = poster;
        records[queuePathKey(directory.path)] = localRecordFromAsset(directory);
      }
      for (const file of found.files) records[queuePathKey(file.path)] = localRecordFromAsset(file);
      for (const group of groups) {
        const seriesArt = records[queuePathKey(group.root)]?.artwork ?? [];
        for (const season of group.seasons) {
          if (!season.directory) continue;
          const key = queuePathKey(season.directory);
          if (next[key]) continue;
          const seasonal = seasonPosterFromArtwork(seriesArt, season.season);
          if (seasonal) next[key] = seasonal;
        }
      }
      libraryLocalPosters = next;
      libraryLocalRecords = records;
      for (const group of groups) {
        const storage = seriesReviewStorageKey(group.root);
        const review = queueSeriesReviews[storage];
        if (!review) continue;
        let seasonPaths = review.localSeasonPosterPaths;
        let postersChanged = false;
        for (const season of group.seasons) {
          if (!season.directory || season.season <= 0) continue;
          const onDisk = next[queuePathKey(season.directory)] ?? "";
          const current = seasonPaths[String(season.season)] ?? "";
          if (!onDisk || (current && !current.startsWith("https://"))) continue;
          seasonPaths = { ...seasonPaths, [String(season.season)]: onDisk };
          postersChanged = true;
        }
        if (postersChanged) queueSeriesReviews = { ...queueSeriesReviews, [storage]: { ...review, localSeasonPosterPaths: seasonPaths } };
      }
      for (const group of groups) {
        const key = seriesReviewStorageKey(group.root);
        const review = queueSeriesReviews[key];
        const names = records[queuePathKey(group.root)]?.seasonNames ?? {};
        if (!review || !Object.keys(names).length) continue;
        let seasonRecords = review.seasonRecords;
        let changed = false;
        for (const season of group.seasons) {
          const named = names[String(season.season)]?.trim();
          const current = seasonRecords[String(season.season)];
          if (!named || !current) continue;
          const generated = season.label || `Season ${String(season.season).padStart(2, "0")}`;
          if (current.title === generated || current.title === `Season ${String(season.season).padStart(2, "0")}`) {
            seasonRecords = { ...seasonRecords, [String(season.season)]: { ...current, title: named } };
            changed = true;
          }
        }
        if (changed) queueSeriesReviews = { ...queueSeriesReviews, [key]: { ...queueSeriesReviews[key], seasonRecords } };
      }
      for (const group of groups) {
        const sample = group.episodePaths[0];
        if (!sample) continue;
        for (const artworkPath of [next[queuePathKey(group.root)], ...group.seasons.map((season) => season.directory ? next[queuePathKey(season.directory)] : "")].filter(Boolean)) {
          void ensureQueueArtworkPreview(sample, artworkPath);
        }
      }
    }).catch(() => {
      if (!cancelled) {
        libraryLocalPosters = {};
        libraryLocalRecords = {};
      }
    });
    return () => {
      cancelled = true;
    };
  });
  const automaticPosterDownloads = new Set<string>();

  function posterCandidateForUrl(base: QueueArtworkCandidate, imageUrl: string, title: string, idSuffix: string): QueueArtworkCandidate | null {
    if (!imageUrl.startsWith("https://") || !base.imageUrl.startsWith("https://") || !base.sourceUrl.startsWith("https://")) return null;
    try {
      if (new URL(imageUrl).host !== new URL(base.imageUrl).host) return null;
    } catch {
      return null;
    }
    return {
      ...base,
      id: `${base.id}-${idSuffix}`.slice(0, 512),
      title: title.slice(0, 1024) || base.title,
      imageUrl,
      thumbnailUrl: imageUrl,
    };
  }

  $effect(() => {
    if (!desktopShell || !scanResult || !appPreferences.savePosterBesideMedia || !librarySeriesGroups.length) return;
    const root = scanResult.root;
    for (const group of librarySeriesGroups) {
      const review = reviewedSeriesForPath(group.episodePaths[0] ?? group.root);
      const candidate = review?.artworkCandidates.find((image) => remoteArtworkUrl(image)) ?? seriesSuggestionArtwork(group);
      if (!candidate) continue;
      if (review && !providerPosterUrl(review)) {
        const storage = seriesReviewStorageKey(group.root);
        const current = queueSeriesReviews[storage];
        if (current && !providerPosterUrl(current)) {
          queueSeriesReviews = {
            ...queueSeriesReviews,
            [storage]: {
              ...current,
              artworkCandidates: uniquePosterChoices([candidate, ...current.artworkCandidates]),
              posterId: current.posterId || candidate.id,
            },
          };
        }
      }
      const seriesKey = queuePathKey(group.root);
      const savedSeriesPoster = review?.localPosterPath ?? "";
      if (!libraryLocalPosters[seriesKey] && !(savedSeriesPoster && !savedSeriesPoster.startsWith("https://"))) {
        const token = `series:${seriesKey}:${candidate.id}`;
        if (!automaticPosterDownloads.has(token)) {
          automaticPosterDownloads.add(token);
          const sample = group.episodePaths[0] ?? "";
          void invoke<{ files: { path: string }[] }>("save_library_artwork", {
            request: { root, candidate, targets: [{ directory: group.root, basename: "poster" }] },
          }).then((result) => {
            const saved = result.files[0]?.path ?? "";
            if (!saved) return;
            libraryLocalPosters = { ...libraryLocalPosters, [seriesKey]: saved };
            const storage = seriesReviewStorageKey(group.root);
            const current = queueSeriesReviews[storage];
            if (current) {
              const currentPoster = current.localPosterPath ?? "";
              queueSeriesReviews = {
                ...queueSeriesReviews,
                [storage]: {
                  ...current,
                  localPosterPath: currentPoster && !currentPoster.startsWith("https://") ? currentPoster : saved,
                  artworkCandidates: current.artworkCandidates.some((image) => image.id === candidate.id) ? current.artworkCandidates : [candidate, ...current.artworkCandidates],
                  posterId: current.posterId || candidate.id,
                },
              };
            }
            if (sample) void ensureQueueArtworkPreview(sample, saved);
          }).catch(() => undefined);
        }
      }
      if (!review) continue;
      for (const season of group.seasons) {
        if (!season.directory || season.season <= 0) continue;
        const seasonUrl = review.localSeasonPosterPaths?.[String(season.season)] ?? "";
        if (!seasonUrl.startsWith("https://") || libraryLocalPosters[queuePathKey(season.directory)]) continue;
        const seasonCandidate = posterCandidateForUrl(candidate, seasonUrl, `${review.title || group.folderName} season ${season.season} poster`, `season-${season.season}`);
        if (!seasonCandidate) continue;
        const seasonKey = queuePathKey(season.directory);
        const token = `season:${seasonKey}:${seasonCandidate.id}`;
        if (automaticPosterDownloads.has(token)) continue;
        automaticPosterDownloads.add(token);
        const sample = season.episodePaths[0] ?? group.episodePaths[0] ?? "";
        void invoke<{ files: { path: string }[] }>("save_library_artwork", {
          request: { root, candidate: seasonCandidate, targets: [{ directory: season.directory, basename: "poster" }] },
        }).then((result) => {
          const saved = result.files[0]?.path ?? "";
          if (!saved) return;
          libraryLocalPosters = { ...libraryLocalPosters, [seasonKey]: saved };
          const storage = seriesReviewStorageKey(group.root);
          const current = queueSeriesReviews[storage];
          const currentSeason = current?.localSeasonPosterPaths?.[String(season.season)] ?? "";
          if (current && (currentSeason.startsWith("https://") || !currentSeason)) {
            queueSeriesReviews = {
              ...queueSeriesReviews,
              [storage]: {
                ...current,
                localSeasonPosterPaths: { ...current.localSeasonPosterPaths, [String(season.season)]: saved },
              },
            };
          }
          if (sample) void ensureQueueArtworkPreview(sample, saved);
        }).catch(() => undefined);
      }
    }
  });

  let libraryLookupMessage = $derived(
    metadataLookupStatus?.status === "searching"
      ? `Matching titles and posters · ${metadataLookupStatus.done} of ${metadataLookupStatus.total}`
      : metadataLookupStatus?.status === "error"
        ? metadataLookupStatus.message
        : "",
  );
  let selectedFilteredCount = $derived(visibleEntries.filter((entry) => workspace.queue.selectedPaths.includes(entry.path)).length);
  let staleSelectedEntries = $derived(selectedRunEntries.filter((entry) => entry.queue_stale));
  $effect(() => {
    if (mediaPage > mediaPageCount) mediaPage = mediaPageCount;
  });
  $effect(() => {
    if (visibleSelectAllCheckbox) {
      visibleSelectAllCheckbox.indeterminate = selectedVisibleCount > 0 && selectedVisibleCount < currentPageEntries.length;
    }
  });
  $effect(() => {
    const selectedSnapshot = [...workspace.queue.selectedPaths];
    const activeSnapshot = workspace.queue.activeRoot;
    const tuneSnapshot = JSON.parse(JSON.stringify(fileTuneOverrides)) as Record<string, "animation" | "none">;
    const streamSnapshot = JSON.parse(JSON.stringify(fileStreamOverrides)) as Record<string, number[]>;
    if (!desktopShell || !workspace.queue.loaded) return;
    const timer = window.setTimeout(() => {
      void persistQueueSelection(selectedSnapshot, activeSnapshot, tuneSnapshot, streamSnapshot);
    }, 200);
    return () => window.clearTimeout(timer);
  });
  $effect(() => {
    const snapshot = JSON.parse(JSON.stringify(queueMovieReviews)) as Record<string, QueueMovieReview>;
    if (!desktopShell || !workspace.queue.loaded) return;
    const timer = window.setTimeout(() => {
      void persistQueueMovieReviews(snapshot);
    }, 450);
    return () => window.clearTimeout(timer);
  });
  $effect(() => {
    const snapshot = JSON.parse(JSON.stringify(queueSeriesReviews)) as Record<string, QueueSeriesReview>;
    if (!desktopShell || !workspace.queue.loaded) return;
    const timer = window.setTimeout(() => {
      void persistQueueSeriesReviews(snapshot);
    }, 450);
    return () => window.clearTimeout(timer);
  });
  $effect(() => {
    const active = desktopShell && page === "settings";
    if (active && !artworkCacheSettingsActive) void refreshArtworkCache();
    artworkCacheSettingsActive = active;
  });
  let youtubeEstimateSummary = $derived.by(() => {
    if (!runBatches.length) return null;
    const entries = selectedRunEntries.filter((entry) =>
      entry.profile &&
      !entry.profile.hdr.dolby_vision &&
      (!hasHdr10plusSignal(entry) || youtubeHdr10plusLossAccepted(entry.path)),
    );
    const estimates = entries.map(youtubeEstimateFor).filter((value) => value !== undefined);
    return {
      files: estimates.length,
      sourceBytes: entries.reduce((sum, entry) => sum + entry.size_bytes, 0),
      outputBytes: estimates.reduce((sum, estimate) => sum + estimate.estimated_output_bytes, 0),
    };
  });
  let analysisErrors = $derived((scanResult?.entries ?? []).filter((entry) => entry.error));
  let toolsRequiredEntries = $derived((scanResult?.entries ?? []).filter((entry) => entry.status === "Tools required"));
  let highMemoryEntries = $derived((scanResult?.entries ?? []).filter((entry) => entry.resource_forecast?.level === "high" || entry.resource_forecast?.level === "blocked"));
  let constrainedMemoryEntries = $derived((scanResult?.entries ?? []).filter((entry) => entry.resource_forecast?.level === "constrained"));
  let failedRows = $derived((reports.flatMap((report) => report.rows)).filter((row) => isFailure(row.status)));
  let activeReport = $derived(reports.find((report) => report.csv_path === selectedReportPath) ?? reports[0] ?? null);
  let settingsChanged = $derived(Boolean(settings && settingsBaseline && JSON.stringify(settings) !== settingsBaseline));
  let forecastOutdated = $derived(Boolean(scanResult && settings && (
    JSON.stringify(scanResult.settings.resources) !== JSON.stringify(settings.resources) ||
    scanResult.settings.video.preset !== settings.video.preset ||
    scanResult.settings.video.promote_8bit_to_10bit !== settings.video.promote_8bit_to_10bit
  )));
  let queueRenameBusy = $derived(Object.values(workspace.queue.runStates).some((state) => state.status === "renaming"));
  let workflowBusy = $derived(Boolean(scanning || workspace.run.starting || workspace.run.id || repairId || hdrInstalling || outputPreviewBusy || queueRenameBusy));
  let modeToolsReady = $derived.by(() => {
    if (mode === "youtube") return environment?.youtube_ready;
    if (mode === "check") return environment?.can_analyze;
    if (mode === "metadata") return environment?.metadata_ready;
    return environment?.ready;
  });
  let canStart = $derived(Boolean(modeToolsReady && hasQueueCandidates && selectedRunEntries.length > 0 && unresolvedSelectedPaths === 0 && !staleSelectedEntries.length && !scanning && !workspace.run.starting && !workspace.run.id && !repairId && !hdrInstalling && !outputPreviewBusy));
  let overallRunPercent = $derived.by(() => {
    if (!workspace.run.progress) return null;
    const sourcePercent = workspace.run.progress.queuePercent ?? workspace.run.progress.filePercent;
    if (!finiteNumber(sourcePercent)) return null;
    const totalFiles = pendingRunBatches.reduce((sum, batch) => sum + batch.entries.length, 0);
    if (!totalFiles || activeRunBatchIndex < 0) return Math.max(0, Math.min(100, sourcePercent));
    const completedFiles = pendingRunBatches.slice(0, activeRunBatchIndex).reduce((sum, batch) => sum + batch.entries.length, 0);
    const currentFiles = pendingRunBatches[activeRunBatchIndex]?.entries.length ?? workspace.run.progress.batchTotal;
    return Math.max(0, Math.min(100, ((completedFiles + currentFiles * sourcePercent / 100) / totalFiles) * 100));
  });
  let encodeMeasured = $derived(Boolean(
    workspace.run.progress && (
      (finiteNumber(workspace.run.progress.filePercent) && workspace.run.progress.filePercent > 0) ||
      finiteNumber(workspace.run.progress.speed) ||
      (finiteNumber(workspace.run.progress.outputBytes) && workspace.run.progress.outputBytes > 0)
    ),
  ));
  let homeActivityLabel = $derived.by(() => {
    if (workspace.run.id || workspace.run.starting) {
      const name = workspace.run.progress?.fileName || workspace.run.message || "Preparing the selected files";
      const percent = encodeMeasured && overallRunPercent != null ? ` · ${Math.round(overallRunPercent)}%` : "";
      return `${name}${percent}`;
    }
    const scanningSources = workspace.queue.sources.filter((source) => source.status === "scanning").length;
    if (scanningSources === 1) return "Scanning 1 source";
    if (scanningSources > 1) return `Scanning ${scanningSources} sources`;
    return "";
  });
  let progressElapsedLabel = $derived.by(() => {
    const base = workspace.run.progress?.elapsedSeconds ?? 0;
    if (!finiteNumber(base)) return "—";
    const since = workspace.run.progressSeenAt ?? workspace.run.startedAt;
    const extra = since ? Math.max(0, (progressClock - since) / 1000) : 0;
    const whole = Math.max(0, Math.round(base + extra));
    const minutes = Math.floor(whole / 60);
    const seconds = whole % 60;
    return minutes ? `${minutes}m ${seconds}s` : `${seconds}s`;
  });

  $effect(() => {
    if (!workspace.run.id) return;
    const timer = setInterval(() => {
      progressClock = Date.now();
    }, 1000);
    return () => clearInterval(timer);
  });
  let instrumentTheme = $derived(theme === "phosphor" || theme === "ion" || theme === "breach");
  let missingRequiredTools = $derived((environment?.checks ?? []).filter((check) => check.required && !check.available));
  let setupGapLabel = $derived.by(() => {
    if (!environment) return "SETUP NEEDED";
    if (missingRequiredTools.length === 1) return `${missingRequiredTools[0].label} missing`;
    if (missingRequiredTools.length > 1) return `${missingRequiredTools.length} tools missing`;
    if (!environment.os.supported) return "Unsupported system";
    if (environment.missing_apt_packages.length) return `${environment.missing_apt_packages.length} packages missing`;
    return "SETUP NEEDED";
  });
  let setupGapDetail = $derived(
    missingRequiredTools.map((check) => check.label).join(", ")
      || environment?.missing_apt_packages.join(", ")
      || "Open environment setup",
  );

  function normalizeTheme(value: unknown): Theme {
    if (value === "hacker") return "phosphor";
    if (value === "light" || value === "dark" || value === "phosphor" || value === "ion" || value === "breach") return value;
    return "light";
  }

  function normalizeToolPaths(value: unknown, legacyFfmpeg?: unknown): ToolPaths {
    const candidate = value && typeof value === "object" ? value as Partial<Record<ToolPathKey, unknown>> : {};
    const normalize = (key: ToolPathKey, legacy?: unknown): string | null => {
      const raw = Object.hasOwn(candidate, key) ? candidate[key] : legacy;
      return typeof raw === "string" && raw.trim() ? raw.trim() : null;
    };
    return {
      ffmpeg: normalize("ffmpeg", legacyFfmpeg),
      ffprobe: normalize("ffprobe"),
      mediainfo: normalize("mediainfo"),
      mkvpropedit: normalize("mkvpropedit"),
      mkvmerge: normalize("mkvmerge"),
      mkvextract: normalize("mkvextract"),
    };
  }

  function currentToolPaths(): ToolPaths {
    return normalizeToolPaths({
      ffmpeg: ffmpegPath,
      ffprobe: ffprobePath,
      mediainfo: mediainfoPath,
      mkvpropedit: mkvpropeditPath,
      mkvmerge: mkvmergePath,
      mkvextract: mkvextractPath,
    });
  }

  function setToolPathValue(key: ToolPathKey, value: string) {
    switch (key) {
      case "ffmpeg": ffmpegPath = value; break;
      case "ffprobe": ffprobePath = value; break;
      case "mediainfo": mediainfoPath = value; break;
      case "mkvpropedit": mkvpropeditPath = value; break;
      case "mkvmerge": mkvmergePath = value; break;
      case "mkvextract": mkvextractPath = value; break;
    }
  }

  function toolPathValue(key: ToolPathKey): string {
    switch (key) {
      case "ffmpeg": return ffmpegPath;
      case "ffprobe": return ffprobePath;
      case "mediainfo": return mediainfoPath;
      case "mkvpropedit": return mkvpropeditPath;
      case "mkvmerge": return mkvmergePath;
      case "mkvextract": return mkvextractPath;
    }
  }

  function applyToolPaths(paths: ToolPaths) {
    ffmpegPath = paths.ffmpeg ?? "";
    ffprobePath = paths.ffprobe ?? "";
    mediainfoPath = paths.mediainfo ?? "";
    mkvpropeditPath = paths.mkvpropedit ?? "";
    mkvmergePath = paths.mkvmerge ?? "";
    mkvextractPath = paths.mkvextract ?? "";
  }

  function publishTheme(value: Theme) {
    theme = value;
    document.documentElement.dataset.theme = value;
    if (value === "phosphor" || value === "ion" || value === "breach") document.documentElement.dataset.instrument = value;
    else delete document.documentElement.dataset.instrument;
  }

  function setNotice(message: string, kind: "info" | "success" | "error" = "info") {
    workspace.notice.message = message;
    workspace.notice.kind = kind;
  }

  function setVisibleSelection(select: boolean) {
    const visible = new Set(libraryPosterCards.flatMap((card) => card.mediaPaths));
    workspace.queue.selectedPaths = selectQueuePaths(workspace.queue.selectedPaths, visible, select);
  }

  function setFilteredSelection(select: boolean) {
    const filtered = new Set(visibleEntries.map((entry) => entry.path));
    workspace.queue.selectedPaths = selectQueuePaths(workspace.queue.selectedPaths, filtered, select);
  }

  async function openMetadataForEntry(entry: MediaEntry) {
    workspace.queue.selectedPaths = [...new Set([...workspace.queue.selectedPaths, entry.path])];
    metadataFocusPath = entry.path;
    libraryTvReview = tvReviewForEpisode(entry);
    const opened = libraryTvReview;
    if (opened?.level === "episode" && opened.season !== null) {
      const group = seriesGroupForRoot(opened.seriesRoot);
      if (group) {
        for (const path of posterPathsForReview(group, opened.season)) void ensureQueueArtworkPreview(entry.path, path);
      }
    }
    metadataDialogOpen = true;
    await tick();
    if (metadataDialogOpen && metadataDialogElement && !metadataDialogElement.open) {
      metadataDialogElement.showModal();
    }
  }

  function closeMetadataDialog() {
    if (metadataDialogElement?.open) metadataDialogElement.close();
    else {
      metadataDialogOpen = false;
      libraryTvReview = null;
    }
  }

  function openFullMetadataOrganizer() {
    closeMetadataDialog();
    openPage("metadata");
  }

  function libraryRenameEntries(): MediaEntry[] {
    const selected = new Set(workspace.queue.selectedPaths);
    return queuedEntries.filter((entry) => selected.has(entry.path));
  }

  function libraryRenameShape(): "movie" | "episode" | "both" {
    let movie = false;
    let episode = false;
    for (const entry of libraryRenameEntries()) {
      if (queueMovieReviews[entry.path]?.title.trim()) movie = true;
      else episode = true;
    }
    if (movie && episode) return "both";
    return movie ? "movie" : "episode";
  }

  function insertRenameToken(kind: "movie" | "episode", token: string, field: HTMLInputElement | null) {
    const current = renameDraft[kind];
    const insertion = insertRenameTemplateToken(current, token, field?.selectionStart ?? null, field?.selectionEnd ?? null);
    renameDraft = { ...renameDraft, [kind]: insertion.value };
    libraryRenameError = "";
    queueMicrotask(() => {
      field?.focus();
      field?.setSelectionRange(insertion.caret, insertion.caret);
    });
  }

  function resetRenamePattern(kind: "movie" | "episode") {
    renameDraft = { ...renameDraft, [kind]: defaultOrganizerTemplates[kind] };
    libraryRenameError = "";
  }

  function libraryRenameNote(entry: MediaEntry, templates: OrganizerTemplates = renameDraft): { next: string | null; note: string } {
    const movie = queueMovieReviews[entry.path];
    const series = reviewedSeriesForPath(entry.path);
    const proposal = proposedFilenameForEntry(entry, movie, series, queueMatchSuggestions[entry.path], templates);
    const next = proposal?.name && proposal.name !== entry.name ? proposal.name : null;
    if (next && proposal?.needsTitleReview) return { next, note: "Suggested name" };
    if (next) return { next, note: "New name" };
    return { next: null, note: "Name stays the same" };
  }

  async function applyLibraryRename() {
    if (libraryRenameBusy) return;
    const entries = libraryRenameEntries();
    libraryRenameError = "";
    if (!entries.length) {
      libraryRenameError = "Select the files to rename.";
      return;
    }
    const roots = [...new Set(entries.flatMap((entry) => {
      const source = workspace.queue.sources.find((item) => item.scanResult?.entries.some((candidate) => candidate.path === entry.path));
      return source ? [source.root] : [];
    }))];
    if (roots.length !== 1) {
      libraryRenameError = "Rename one folder at a time.";
      return;
    }
    const templates = renameDraft;
    const items: { sourcePath: string; kind: "movie" | "episode"; template: string; values: Record<string, string> }[] = [];
    for (const entry of entries) {
      const movie = queueMovieReviews[entry.path];
      if (movie?.title.trim()) {
        items.push({ sourcePath: entry.path, kind: "movie", template: templates.movie, values: { title: movie.title, year: movie.year, certification: movie.certification } });
        continue;
      }
      const series = reviewedSeriesForPath(entry.path);
      const assignment = series?.episodeAssignments[entry.path]
        ?? Object.entries(series?.episodeAssignments ?? {}).find(([path]) => queuePathKey(path) === queuePathKey(entry.path))?.[1];
      if (!series?.title.trim() || !assignment?.title.trim()) continue;
      items.push({ sourcePath: entry.path, kind: "episode", template: templates.episode, values: {
        series_title: series.title,
        season: String(assignment.season),
        episode: String(assignment.episode),
        episode_title: assignment.title,
      } });
    }
    if (!items.length) {
      libraryRenameError = "Confirm a title on each selected file before renaming. A file that already has the right name stays as it is.";
      return;
    }
    libraryRenameBusy = true;
    try {
      if (JSON.stringify(renameDraft) !== JSON.stringify(appPreferences.organizerTemplates)) {
        await persistPreferences({ organizerTemplates: renameDraft });
      }
      const request = { root: roots[0], items, textBlacklistPatterns: settings?.metadata.text_blacklist ?? [] };
      const preview = await invoke<{ previewId: string; canApply: boolean; conflicts: { reason: string }[]; items: { sourcePath: string; targetPath: string; unchanged: boolean }[] }>("preview_library_rename", request);
      if (!preview.canApply) {
        libraryRenameError = preview.conflicts[0]?.reason ?? "These files already use the proposed names.";
        return;
      }
      await invoke("apply_library_rename", { ...request, previewId: preview.previewId, confirmed: true });
      handleQueuePathsRenamed(Object.fromEntries(preview.items.map((item) => [item.sourcePath, item.targetPath])));
      const changed = preview.items.filter((item) => !item.unchanged).length;
      libraryRenameOpen = false;
      setNotice(`Renamed ${changed} file${changed === 1 ? "" : "s"}. The video itself was not changed.`, "success");
    } catch (cause) {
      libraryRenameError = cause instanceof Error ? cause.message : String(cause);
    } finally {
      libraryRenameBusy = false;
    }
  }

  function chooseLibraryTask(task: "organize" | "metadata" | "rebuild") {
    if (workflowBusy) return;
    if (task === "organize") {
      libraryRenameError = "";
      renameDraft = { ...appPreferences.organizerTemplates };
      libraryRenameOpen = true;
      void tick().then(() => {
        if (libraryRenameOpen && libraryRenameDialog && !libraryRenameDialog.open) libraryRenameDialog.showModal();
      });
      return;
    }
    mode = task === "metadata" ? "metadata" : "full";
    openPage("run");
  }

  function startMetadataOnlyFromOrganizer(mediaPaths: string[]) {
    if (workflowBusy) return;
    const queuedPaths = new Map<string, string>();
    for (const source of workspace.queue.sources) {
      for (const entry of source.scanResult?.entries ?? []) {
        const key = queuePathKey(entry.path);
        if (!queuedPaths.has(key)) queuedPaths.set(key, entry.path);
      }
    }
    const requestedKeys = [...new Set(mediaPaths.map(queuePathKey))];
    const uniquePaths = requestedKeys.flatMap((key) => {
      const path = queuedPaths.get(key);
      return path ? [path] : [];
    });
    if (uniquePaths.length !== requestedKeys.length) {
      setNotice("Add these files to the Media Library queue and finish their scan before creating MKV copies.", "error");
      return;
    }
    workspace.queue.selectedPaths = uniquePaths;
    mode = "metadata";
    openPage("run");
    setNotice(`${uniquePaths.length} file${uniquePaths.length === 1 ? "" : "s"} selected for metadata-only MKV output. Review the destination and collision check before starting.`, "success");
  }

  function uniquePosterChoices(candidates: QueueArtworkCandidate[]): QueueArtworkCandidate[] {
    const seen = new Set<string>();
    return candidates.filter((image) => {
      if (!image.id || image.id.length > 1024 || seen.has(image.id)) return false;
      seen.add(image.id);
      return true;
    });
  }

  function queueDocument(): MediaQueueDocument {
    return {
      version: 1,
      sources: workspace.queue.sources,
      selectedMediaPaths: [...workspace.queue.selectedPaths],
      fileTuneOverrides: { ...fileTuneOverrides },
      fileStreamOverrides: { ...fileStreamOverrides },
      movieReviews: Object.fromEntries(Object.entries(queueMovieReviews).map(([path, review]) => [
        path,
        { ...review, artworkCandidates: uniquePosterChoices(review.artworkCandidates) },
      ])),
      seriesReviews: Object.fromEntries(Object.entries(queueSeriesReviews).map(([path, review]) => [
        path,
        { ...review, artworkCandidates: uniquePosterChoices(review.artworkCandidates) },
      ])),
      matchSuggestions: { ...queueMatchSuggestions },
      runStates: { ...workspace.queue.runStates },
      activeRoot: workspace.queue.activeRoot,
      updatedAtMs: Date.now(),
    };
  }

  function enqueueQueueWrite(write: () => Promise<void>) {
    const operation = queueWriteChain.catch(() => {}).then(write);
    queueWriteChain = operation.then(() => {}, () => {});
    return operation;
  }

  function persistQueueSources(): Promise<boolean> {
    if (!desktopShell || !workspace.queue.loaded) return Promise.resolve(false);
    const operation = enqueueQueueWrite(async () => {
      await invoke("save_library_queue", { queue: queueDocument() });
    });
    return operation.then(() => {
      workspace.queue.persistenceError = "";
      return true;
    }, (error) => {
      workspace.queue.persistenceError = String(error);
      return false;
    });
  }

  function persistQueueRunStates(updates: Record<string, MediaQueueRunState>): Promise<boolean> {
    workspace.queue.runStates = { ...workspace.queue.runStates, ...updates };
    return persistQueueSources();
  }

  function queueRunStateForPath(path: string): MediaQueueRunState | undefined {
    return queueRunStatesByPath.get(queuePathKey(path));
  }

  function queueRunStatusLabel(status: MediaQueueRunStatus): string {
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

  function makeQueueRunState(
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

  function runProgressStatus(phase: string, runMode: MediaQueueRunState["mode"]): MediaQueueRunStatus {
    const value = phase.toLocaleLowerCase();
    if (/validat|verified output|output installed|transaction commit/.test(value)) return "validating";
    if (/remux|mux|metadata embedding|embedding metadata|copying streams/.test(value)) return "remuxing";
    if (/encode|calibration|hdr10\+ metadata extraction|finalizing/.test(value)) return "rebuilding";
    if (runMode === "check" || /analysis|reading media tracks|storage preflight/.test(value)) return "scanning";
    return "running";
  }

  function recordQueueRunProgress(event: RunProgressEvent) {
    if (!event.filePath) return;
    const current = queueRunStateForPath(event.filePath);
    if (!current || !["running", "scanning", "renaming", "remuxing", "rebuilding", "validating"].includes(current.status)) return;
    const status = runProgressStatus(event.phase, current.mode);
    if (current.status === status) return;
    void persistQueueRunStates({
      [event.filePath]: {
        ...current,
        status,
        message: [...event.phase].slice(0, 1024).join(""),
        updatedAtMs: Date.now(),
      },
    });
  }

  function handleQueueRenameState(paths: string[], status: "renaming" | "renamed" | "failed", message = "") {
    if (!workspace.queue.loaded || !paths.length) return;
    const allowed = new Map(queuedEntries.map((entry) => [queuePathKey(entry.path), entry.path]));
    const updates: Record<string, MediaQueueRunState> = {};
    for (const path of paths) {
      const savedPath = allowed.get(queuePathKey(path));
      if (!savedPath) continue;
      updates[savedPath] = makeQueueRunState(status, "organize", { message });
    }
    if (Object.keys(updates).length) void persistQueueRunStates(updates);
  }

  function selectFilesToReview() {
    workspace.queue.selectedPaths = runReviewEntries.map((entry) => entry.path);
    if (runReviewEntries.some((entry) => queueRunStateForPath(entry.path)?.status === "interrupted")) {
      setNotice("Interrupted files are selected. Check their last report and output before retrying; existing outputs are never overwritten.", "info");
    } else {
      setNotice(`${workspace.queue.selectedPaths.length} unfinished or skipped file${workspace.queue.selectedPaths.length === 1 ? "" : "s"} selected for review.`, "info");
    }
  }

  async function prepareQueueEntryForRetry(entry: MediaEntry, state: MediaQueueRunState) {
    if (workflowBusy || entry.queue_stale) return;
    const source = workspace.queue.sources.find((item) => item.scanResult?.entries.some((candidate) => queuePathKey(candidate.path) === queuePathKey(entry.path)));
    if (!source) {
      setNotice("This file is no longer attached to a completed queue scan. Refresh its source before retrying.", "error");
      return;
    }
    if (source.status !== "ready" || !source.scanResult) {
      setNotice("Refresh this source and wait for its scan to finish before retrying the file.", "error");
      return;
    }
    if (state.mode === "organize") {
      activateQueueSource(source.root);
      openPage("library");
      await openMetadataForEntry(entry);
      setNotice("Rename review reopened for this file. Check the title and preview the proposed filename before confirming another change.", "info");
      return;
    }
    const previousSelection = workspace.queue.selectedPaths;
    const previousMode = mode;
    activateQueueSource(source.root);
    const retryMode: RunMode = state.mode ?? mode;
    mode = retryMode;
    workspace.queue.selectedPaths = [entry.path];
    if (state.status !== "interrupted" && state.status !== "needs_review") {
      const queued = makeQueueRunState("queued", retryMode, {
        message: "Selected for a manual retry. Existing outputs remain protected by the destination check.",
        outputPath: state.outputPath,
        reportPath: state.reportPath,
        reportStatus: state.reportStatus,
      });
      if (!await persistQueueRunStates({ [entry.path]: queued })) {
        workspace.queue.runStates = { ...workspace.queue.runStates, [entry.path]: state };
        workspace.queue.selectedPaths = previousSelection;
        mode = previousMode;
        setNotice(`The retry could not be saved to the queue: ${workspace.queue.persistenceError}`, "error");
        return;
      }
    } else {
      if (!await persistQueueSources()) {
        workspace.queue.selectedPaths = previousSelection;
        mode = previousMode;
        setNotice(`The retry selection could not be saved to the queue: ${workspace.queue.persistenceError}`, "error");
        return;
      }
    }
    openPage(retryMode === "youtube" ? "youtube" : "run");
    setNotice(state.status === "interrupted" || state.status === "needs_review"
      ? "This file is selected. Start its saved task to review the last report and output before retrying."
      : "This file is selected for a retry. Review the task and output destination before starting.", "info");
  }

  async function skipQueueEntry(entry: MediaEntry, state: MediaQueueRunState) {
    if (workflowBusy || entry.queue_stale) return;
    const skipped = makeQueueRunState("skipped", state.mode ?? mode, {
      message: "Skipped by the user. Previous report and output details are retained.",
      outputPath: state.outputPath,
      reportPath: state.reportPath,
      reportStatus: state.reportStatus,
    });
    const previousSelection = workspace.queue.selectedPaths;
    workspace.queue.selectedPaths = workspace.queue.selectedPaths.filter((path) => queuePathKey(path) !== queuePathKey(entry.path));
    if (!await persistQueueRunStates({ [entry.path]: skipped })) {
      workspace.queue.runStates = { ...workspace.queue.runStates, [entry.path]: state };
      workspace.queue.selectedPaths = previousSelection;
      setNotice(`The skipped state could not be saved: ${workspace.queue.persistenceError}`, "error");
      return;
    }
    setNotice(`${entry.name} is skipped for ${skipped.mode?.toUpperCase() ?? "this task"}. Select it again to review or run it.`, "success");
  }

  async function openQueueRunReport(path: string) {
    await loadReports();
    const report = reports.find((candidate) => queuePathKey(candidate.csv_path) === queuePathKey(path));
    if (!report) {
      setNotice("The saved report is no longer available in recent history.", "error");
      return;
    }
    openPage("reports");
    chooseReport(report.csv_path);
  }

  async function loadQueueRunDiagnostics(entry: MediaEntry, state: MediaQueueRunState) {
    const key = queuePathKey(entry.path);
    if (!state.reportPath || queueRunLogs[key]?.loading) return;
    queueRunLogs = { ...queueRunLogs, [key]: { loading: true, text: "", error: "" } };
    try {
      await loadReports();
      const report = reports.find((candidate) => queuePathKey(candidate.csv_path) === queuePathKey(state.reportPath!));
      if (!report) throw new Error("The report is no longer available in recent history.");
      const text = await invoke<string>("read_log", { path: report.log_path });
      queueRunLogs = { ...queueRunLogs, [key]: { loading: false, text, error: "" } };
    } catch (error) {
      queueRunLogs = { ...queueRunLogs, [key]: { loading: false, text: "", error: String(error) } };
    }
  }

  function persistQueueSelection(
    selected = [...workspace.queue.selectedPaths],
    active = workspace.queue.activeRoot,
    tunes: Record<string, "animation" | "none"> = JSON.parse(JSON.stringify(fileTuneOverrides)) as Record<string, "animation" | "none">,
    streams: Record<string, number[]> = JSON.parse(JSON.stringify(fileStreamOverrides)) as Record<string, number[]>,
  ): Promise<void> {
    if (!desktopShell || !workspace.queue.loaded) return Promise.resolve();
    return enqueueQueueWrite(async () => {
      await invoke("save_library_queue_selection", {
        selectedMediaPaths: selected,
        activeRoot: active,
        fileTuneOverrides: tunes,
        fileStreamOverrides: streams,
      });
    }).then(() => {
      workspace.queue.persistenceError = "";
    }).catch((error) => {
      workspace.queue.persistenceError = String(error);
    });
  }

  function persistQueueMovieReviews(reviews: Record<string, QueueMovieReview>): Promise<void> {
    if (!desktopShell || !workspace.queue.loaded) return Promise.resolve();
    return enqueueQueueWrite(async () => {
      await invoke("save_library_queue_movie_reviews", { movieReviews: reviews });
    }).then(() => {
      workspace.queue.persistenceError = "";
    }).catch((error) => {
      workspace.queue.persistenceError = String(error);
    });
  }

  function persistQueueSeriesReviews(reviews: Record<string, QueueSeriesReview>): Promise<void> {
    if (!desktopShell || !workspace.queue.loaded) return Promise.resolve();
    return enqueueQueueWrite(async () => {
      await invoke("save_library_queue_series_reviews", { seriesReviews: reviews });
    }).then(() => {
      workspace.queue.persistenceError = "";
    }).catch((error) => {
      workspace.queue.persistenceError = String(error);
    });
  }

  function persistQueueMetadataSuggestions(suggestions: Record<string, QueueMetadataSuggestion> = queueMatchSuggestions): Promise<void> {
    if (!desktopShell || !workspace.queue.loaded) return Promise.resolve();
    if (metadataSuggestionSaveTimer) clearTimeout(metadataSuggestionSaveTimer);
    metadataSuggestionSaveTimer = null;
    const snapshot = JSON.parse(JSON.stringify(suggestions)) as Record<string, QueueMetadataSuggestion>;
    return enqueueQueueWrite(async () => {
      await invoke("save_library_queue_metadata_suggestions", { matchSuggestions: snapshot });
    }).then(() => {
      workspace.queue.persistenceError = "";
    }).catch((error) => {
      workspace.queue.persistenceError = String(error);
    });
  }

  function scheduleQueueMetadataSuggestionSave() {
    if (!desktopShell || !workspace.queue.loaded) return;
    if (metadataSuggestionSaveTimer) clearTimeout(metadataSuggestionSaveTimer);
    metadataSuggestionSaveTimer = setTimeout(() => {
      metadataSuggestionSaveTimer = null;
      void persistQueueMetadataSuggestions();
    }, 3000);
  }

  function scanResultWithQueueStatus(source: MediaQueueSource): ScanResult | null {
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

  function activateQueueSource(sourceRoot: string) {
    const source = workspace.queue.sources.find((item) => item.root === sourceRoot);
    if (!source) return;
    if (workspace.queue.activeRoot !== source.root) metadataLookupClaimed = false;
    workspace.queue.activeRoot = source.root;
    root = source.root;
    search = "";
    mediaPage = 1;
    libraryBrowse = null;
    const result = scanResultWithQueueStatus(source);
    scanResult = result;
    if (result) {
      recursive = result.recursive;
      minimumSizeMib = String(result.minimum_size_mib);
      environment = result.environment;
      settings = result.settings;
      settingsSources = result.settings_sources;
      settingsRoot = result.root;
      settingsBaseline = JSON.stringify(result.settings);
      librarySettingsBaseline = settingsBaseline;
      metadataLookupPending = sourceNeedsMetadataLookup(source);
      metadataLookupStatus = null;
    } else {
      metadataLookupPending = false;
      metadataLookupStatus = null;
      void loadSettings(source.root);
    }
    void persistPreferences({ root: source.root });
  }

  function claimMetadataLookup(): boolean {
    if (metadataLookupClaimed) return false;
    metadataLookupClaimed = true;
    return true;
  }

  function sourceNeedsMetadataLookup(source: MediaQueueSource): boolean {
    if (!appPreferences.metadataAutoLookup || !source.scanResult) return false;
    return source.scanResult.entries.some((entry) => {
      if (queueMovieReviews[entry.path]?.recordId) return false;
      const series = reviewedSeriesForPath(entry.path);
      const seriesRecord = series?.recordId ?? "";
      if (seriesRecord && !seriesRecord.startsWith("local")) {
        const hasProviderArt = (series?.artworkCandidates ?? []).some((image) => image.imageUrl.startsWith("https://") || image.thumbnailUrl.startsWith("https://"));
        const group = librarySeriesGroups.find((item) => item.episodePaths.some((path) => queuePathKey(path) === queuePathKey(entry.path)));
        const seasonNumbers = group?.seasons.map((item) => item.season).filter((number) => number > 0) ?? [];
        const hasSeasonArt = seasonNumbers.length
          ? seasonNumbers.every((number) => (series?.localSeasonPosterPaths?.[String(number)] ?? "").startsWith("https://"))
          : Object.values(series?.localSeasonPosterPaths ?? {}).some((path) => path.startsWith("https://"));
        const hasCertificate = Boolean(series?.certification.trim());
        return !hasProviderArt || !hasSeasonArt || !hasCertificate;
      }
      const status = queueMatchSuggestions[entry.path]?.status;
      return status === undefined || status === "searching";
    });
  }

  function removeQueueSource(sourceRoot: string) {
    if (workflowBusy) return;
    const removed = workspace.queue.sources.find((source) => source.root === sourceRoot);
    if (!removed) return;
    workspace.queue.sources = workspace.queue.sources.filter((source) => source.root !== sourceRoot);
    const remainingPaths = new Set(workspace.queue.sources.flatMap((source) => source.scanResult?.entries.map((entry) => entry.path) ?? []));
    workspace.queue.selectedPaths = workspace.queue.selectedPaths.filter((path) => remainingPaths.has(path));
    fileTuneOverrides = Object.fromEntries(Object.entries(fileTuneOverrides).filter(([path]) => remainingPaths.has(path)));
    fileStreamOverrides = Object.fromEntries(Object.entries(fileStreamOverrides).filter(([path]) => remainingPaths.has(path)));
    workspace.queue.runStates = Object.fromEntries(Object.entries(workspace.queue.runStates).filter(([path]) => remainingPaths.has(path)));
    queueMatchSuggestions = Object.fromEntries(Object.entries(queueMatchSuggestions).filter(([path]) => remainingPaths.has(path)));
    queueMovieReviews = Object.fromEntries(Object.entries(queueMovieReviews).filter(([path]) => remainingPaths.has(path)));
    queueSeriesReviews = Object.fromEntries(Object.entries(queueSeriesReviews).filter(([seriesRoot, review]) => {
      const hasRemainingEpisode = Object.keys(review.episodeAssignments).some((path) => remainingPaths.has(path));
      const hasRemainingSeriesMedia = [...remainingPaths].some((path) => pathIsWithin(seriesRoot, path));
      const belongedToRemovedSource = pathIsWithin(sourceRoot, seriesRoot)
        || pathIsWithin(seriesRoot, sourceRoot)
        || Object.keys(review.episodeAssignments).some((path) => pathIsWithin(sourceRoot, path));
      return hasRemainingEpisode || hasRemainingSeriesMedia || !belongedToRemovedSource;
    }));
    if (workspace.queue.activeRoot === sourceRoot) {
      const next = workspace.queue.sources.find((source) => source.scanResult) ?? workspace.queue.sources[0];
      if (next) activateQueueSource(next.root);
      else {
        workspace.queue.activeRoot = null;
        scanResult = null;
        root = "";
      }
    }
    void persistQueueSources();
  }

  async function scanQueueSource(path: string, activate: boolean, refresh: boolean) {
    if (!desktopShell) {
      setNotice("Open HEVC Rebuild Studio from the desktop app to inspect this computer and connect to the media tools.", "error");
      return;
    }
    if (workspace.run.id || workspace.run.starting || repairId || hdrInstalling || scanning) return;
    const requestedRoot = path.trim();
    if (!requestedRoot) {
      setNotice("Choose a media file or folder first.", "error");
      return;
    }
    const existing = workspace.queue.sources.find((source) => source.root === requestedRoot);
    if (existing && !refresh && existing.scanResult) {
      if (activate) activateQueueSource(existing.root);
      setNotice("This source is already in the queue. Use Refresh on its card to scan it again.", "info");
      return;
    }

    const preserveRunSettings = settingsChanged && settingsRoot === requestedRoot ? settings : null;
    if (refresh || !existing?.scanResult) {
      metadataLookupClaimed = false;
      queueMatchSuggestions = Object.fromEntries(Object.entries(queueMatchSuggestions)
        .filter(([mediaPath]) => !pathIsWithin(requestedRoot, mediaPath)));
    }
    metadataLookupStatus = null;
    const scanStartedAt = Date.now();
    const scanningSource: MediaQueueSource = existing
      ? { ...existing, status: "scanning", error: null, updatedAtMs: scanStartedAt }
      : { root: requestedRoot, status: "scanning", scanResult: null, error: null, updatedAtMs: scanStartedAt, changedFiles: [], missingFiles: [] };
    workspace.queue.sources = existing
      ? workspace.queue.sources.map((source) => source.root === requestedRoot ? scanningSource : source)
      : [...workspace.queue.sources, scanningSource];
    root = requestedRoot;
    scanning = true;
    scanProgress = null;
    setNotice("Scanning this source. Other queued folders stay available.", "info");
    await persistQueueSources();
    await persistPreferences({ root: requestedRoot, recursive, minimumSizeMib, stopOnError });
    try {
      const result = await invoke<ScanResult>("scan_folder", {
        root: requestedRoot,
        recursive,
        minimumSizeMib: Math.max(0, Math.min(4096, Number(minimumSizeMib) || 0)),
        filenameIncludePatterns: appPreferences.filenameIncludePatterns,
        filenameExcludePatterns: appPreferences.filenameExcludePatterns,
        toolPaths: currentToolPaths(),
      });
      const refreshedPaths = new Set(result.entries.map((entry) => entry.path));
      const updatedSource: MediaQueueSource = {
        root: result.root,
        status: "ready",
        scanResult: result,
        error: null,
        updatedAtMs: Date.now(),
        changedFiles: [],
        missingFiles: [],
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
      fileTuneOverrides = Object.fromEntries(Object.entries(fileTuneOverrides).filter(([mediaPath]) => availablePaths.has(mediaPath)));
      queueMatchSuggestions = Object.fromEntries(Object.entries(queueMatchSuggestions).filter(([mediaPath]) => availablePaths.has(mediaPath)));
      const oldEntries = new Map((existing?.scanResult?.entries ?? []).map((entry) => [queuePathKey(entry.path), entry]));
      const refreshedEntries = new Map(result.entries.map((entry) => [queuePathKey(entry.path), entry]));
      workspace.queue.runStates = Object.fromEntries(Object.entries(workspace.queue.runStates)
        .filter(([path]) => availablePaths.has(path))
        .map(([path, state]) => {
          if (!refreshedPaths.has(path)) return [path, state];
          const before = oldEntries.get(queuePathKey(path));
          const after = refreshedEntries.get(queuePathKey(path));
          const sameSource = Boolean(before && after && before.modified_unix_ms != null && after.modified_unix_ms != null && before.size_bytes === after.size_bytes && before.modified_unix_ms === after.modified_unix_ms);
          return [path, sameSource ? state : makeQueueRunState("needs_review", state.mode ?? mode, {
            message: "This source changed since the recorded run. Review the existing output before retrying.",
            reportPath: state.reportPath,
            reportStatus: state.reportStatus,
          })];
        }));
      // A fresh probe may change track indices for this source. Preserve overrides for every other queue source.
      fileStreamOverrides = Object.fromEntries(Object.entries(fileStreamOverrides).filter(([mediaPath]) => availablePaths.has(mediaPath) && !refreshedPaths.has(mediaPath)));
      const retainedOutputDirs = Object.fromEntries(Object.entries(appPreferences.fileOutputDirs).filter(([mediaPath]) => availablePaths.has(mediaPath)));
      if (Object.keys(retainedOutputDirs).length !== Object.keys(appPreferences.fileOutputDirs).length) {
        void persistPreferences({ fileOutputDirs: retainedOutputDirs });
      }
      if (activate) {
        workspace.queue.activeRoot = result.root;
        root = result.root;
        scanResult = result;
        recursive = result.recursive;
        minimumSizeMib = String(result.minimum_size_mib);
        settingsRoot = result.root;
        settingsBaseline = JSON.stringify(result.settings);
        librarySettingsBaseline = settingsBaseline;
        environment = result.environment;
        settings = preserveRunSettings ?? result.settings;
        settingsSources = result.settings_sources;
        metadataLookupPending = sourceNeedsMetadataLookup(updatedSource);
        if (metadataLookupPending) metadataLookupClaimed = false;
        if (page !== "home" && page !== "youtube") openPage("library");
        void persistPreferences({ root });
      }
      const persisted = await persistQueueSources();
      setNotice(
        persisted
          ? `${result.candidate_count} media file${result.candidate_count === 1 ? "" : "s"} scanned and saved in the queue.`
          : `${result.candidate_count} media file${result.candidate_count === 1 ? "" : "s"} scanned, but the queue could not be saved.`,
        persisted ? "success" : "error",
      );
      if (page !== "youtube" && mode === "youtube") mode = "check";
    } catch (error) {
      const failed = workspace.queue.sources.find((source) => source.root === requestedRoot);
      if (failed) {
        workspace.queue.sources = workspace.queue.sources.map((source) => source.root === requestedRoot
          ? { ...source, status: source.scanResult ? "ready" : "failed", error: String(error), updatedAtMs: Date.now() }
          : source);
      }
      await persistQueueSources();
      setNotice(`Media scan failed: ${String(error)}`, "error");
    } finally {
      scanning = false;
      scanProgress = null;
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

  function applyTheme(value: string) {
    publishTheme(normalizeTheme(value));
    void persistPreferences({ theme });
  }

  function persistPreferences(patch: Partial<AppPreferences>, successMessage?: string) {
    appPreferences = { ...appPreferences, ...patch };
    if (patch.metadataLanguage) metadataLanguageDraft = patch.metadataLanguage;
    const snapshot = JSON.parse(JSON.stringify(appPreferences)) as AppPreferences;
    const operation = preferenceSaveQueue.catch(() => {}).then(async () => {
      if (desktopShell) {
        preferenceStorage = await invoke<PreferencesPayload>("save_app_preferences", { preferences: snapshot });
      } else {
        localStorage.setItem(browserPreferencesKey, JSON.stringify(snapshot));
      }
      if (successMessage) setNotice(successMessage, "success");
    });
    preferenceSaveQueue = operation.then(() => {}, () => {});
    return operation.then(() => true, (error) => {
      setNotice(`Could not save application settings: ${String(error)}`, "error");
      return false;
    });
  }

  const workspacePages: Page[] = ["home", "library", "metadata", "youtube", "run", "settings", "reports", "help"];

  function normalizePage(value: unknown): Page {
    return typeof value === "string" && workspacePages.includes(value as Page) ? value as Page : "home";
  }

  function openPage(next: Page) {
    page = next;
    if (next === "youtube") mode = "youtube";
    else if (mode === "youtube") mode = "check";
    if (appPreferences.lastPage !== next) void persistPreferences({ lastPage: next });
  }

  async function restoreWindowState(saved: AppPreferences["windowState"]) {
    if (!desktopShell) return;
    const currentWindow = getCurrentWindow();
    restoringWindowState = true;
    try {
      const monitor = await currentMonitor();
      const scaleFactor = monitor?.scaleFactor ?? await currentWindow.scaleFactor();
      const workArea = monitor?.workArea.size.toLogical(scaleFactor);
      const maxWidth = workArea ? Math.max(1120, Math.floor(workArea.width - 40)) : 4096;
      const maxHeight = workArea ? Math.max(720, Math.floor(workArea.height - 40)) : 4096;
      const width = Math.min(Math.max(saved.width, 1120), maxWidth);
      const height = Math.min(Math.max(saved.height, 720), maxHeight);
      await currentWindow.setSize(new LogicalSize(width, height));
      if (saved.maximized) await currentWindow.maximize();
    } catch (error) {
      setNotice(`Could not restore the previous window size: ${String(error)}`, "error");
    } finally {
      window.setTimeout(() => {
        restoringWindowState = false;
        windowPreferencesReady = true;
      }, 350);
    }
  }

  function scheduleWindowStateSave() {
    if (!desktopShell || !windowPreferencesReady || restoringWindowState) return;
    if (windowStateSaveTimer !== null) window.clearTimeout(windowStateSaveTimer);
    windowStateSaveTimer = window.setTimeout(() => {
      windowStateSaveTimer = null;
      void saveCurrentWindowState();
    }, 500);
  }

  async function saveCurrentWindowState() {
    if (!desktopShell || !windowPreferencesReady || restoringWindowState) return;
    try {
      const currentWindow = getCurrentWindow();
      const maximized = await currentWindow.isMaximized();
      const next = { ...appPreferences.windowState, maximized };
      if (!maximized) {
        const [size, scaleFactor] = await Promise.all([currentWindow.innerSize(), currentWindow.scaleFactor()]);
        next.width = Math.round(size.width / scaleFactor);
        next.height = Math.round(size.height / scaleFactor);
      }
      if (JSON.stringify(next) === JSON.stringify(appPreferences.windowState)) return;
      appPreferences = { ...appPreferences, windowState: next };
      await persistPreferences({ windowState: next });
    } catch (error) {
      setNotice(`Could not save the window size: ${String(error)}`, "error");
    }
  }

  function loadBrowserPreferences() {
    let saved: Partial<AppPreferences> = {};
    let legacyTheme = false;
    try {
      const raw = localStorage.getItem(browserPreferencesKey);
      if (raw) {
        const value: unknown = JSON.parse(raw);
        if (value && typeof value === "object") {
          const candidate = value as Omit<Partial<AppPreferences>, "theme"> & { theme?: unknown; ffmpegPath?: unknown };
          if (candidate.theme === "hacker") legacyTheme = true;
          if (typeof candidate.theme === "string") saved.theme = normalizeTheme(candidate.theme);
          if (typeof candidate.lastPage === "string") saved.lastPage = normalizePage(candidate.lastPage);
          if (typeof candidate.root === "string" || candidate.root === null) saved.root = candidate.root;
          if (candidate.toolPaths && typeof candidate.toolPaths === "object") {
            saved.toolPaths = normalizeToolPaths(candidate.toolPaths, candidate.ffmpegPath);
          } else if (typeof candidate.ffmpegPath === "string" || candidate.ffmpegPath === null) {
            saved.toolPaths = normalizeToolPaths({}, candidate.ffmpegPath);
          }
          if (typeof candidate.recursive === "boolean") saved.recursive = candidate.recursive;
          if (typeof candidate.minimumSizeMib === "string") saved.minimumSizeMib = candidate.minimumSizeMib;
          if (typeof candidate.stopOnError === "boolean") saved.stopOnError = candidate.stopOnError;
          if (candidate.youtubeResolution === "source" || candidate.youtubeResolution === "1440p" || candidate.youtubeResolution === "4k") {
            saved.youtubeResolution = candidate.youtubeResolution === "source" && candidate.youtubeQualityMode === undefined
              ? "1440p"
              : candidate.youtubeResolution;
          }
          if (candidate.youtubeQualityMode === "balanced" || candidate.youtubeQualityMode === "recommended") saved.youtubeQualityMode = candidate.youtubeQualityMode;
          if (typeof candidate.youtubeSourceMultiplier === "number" && Number.isFinite(candidate.youtubeSourceMultiplier) && candidate.youtubeSourceMultiplier >= 0.5 && candidate.youtubeSourceMultiplier <= 4) saved.youtubeSourceMultiplier = candidate.youtubeSourceMultiplier;
          if (typeof candidate.youtubeOutputDir === "string" || candidate.youtubeOutputDir === null) saved.youtubeOutputDir = candidate.youtubeOutputDir;
          if (typeof candidate.rebuildOutputDir === "string" || candidate.rebuildOutputDir === null) saved.rebuildOutputDir = candidate.rebuildOutputDir;
          if (candidate.fileOutputDirs && typeof candidate.fileOutputDirs === "object" && !Array.isArray(candidate.fileOutputDirs)) {
            const entries = Object.entries(candidate.fileOutputDirs as Record<string, unknown>).filter((item): item is [string, string] => typeof item[1] === "string");
            if (entries.length <= 512) saved.fileOutputDirs = Object.fromEntries(entries);
          }
          if (Array.isArray(candidate.filenameIncludePatterns) && candidate.filenameIncludePatterns.every((item) => typeof item === "string")) saved.filenameIncludePatterns = candidate.filenameIncludePatterns as string[];
          if (Array.isArray(candidate.filenameExcludePatterns) && candidate.filenameExcludePatterns.every((item) => typeof item === "string")) {
            const excludes = candidate.filenameExcludePatterns as string[];
            const defaultsApplied = candidate.filenameExcludeDefaultsVersion === 2;
            saved.filenameExcludePatterns = defaultsApplied
              ? excludes
              : [...defaultFilenameExcludePatterns, ...excludes.filter((pattern) => !defaultFilenameExcludePatterns.some((fallback) => fallback.toLocaleLowerCase() === pattern.toLocaleLowerCase()))];
            saved.filenameExcludeDefaultsVersion = 2;
          }
          if (typeof candidate.metadataAutoLookup === "boolean") saved.metadataAutoLookup = candidate.metadataAutoLookup;
          if (typeof candidate.metadataLanguage === "string" && /^[a-zA-Z]{2}(?:-[a-zA-Z]{2})?$/.test(candidate.metadataLanguage)) saved.metadataLanguage = candidate.metadataLanguage;
          if (candidate.metadataSources && typeof candidate.metadataSources === "object" && !Array.isArray(candidate.metadataSources)) {
            const sources = candidate.metadataSources as Partial<MetadataSources>;
            saved.metadataSources = {
              tmdb: typeof sources.tmdb === "boolean" ? sources.tmdb : appPreferences.metadataSources.tmdb,
              wikidata: typeof sources.wikidata === "boolean" ? sources.wikidata : appPreferences.metadataSources.wikidata,
              tvmaze: typeof sources.tvmaze === "boolean" ? sources.tvmaze : appPreferences.metadataSources.tvmaze,
              tvdb: typeof sources.tvdb === "boolean" ? sources.tvdb : appPreferences.metadataSources.tvdb,
            };
          }
          if (candidate.organizerTemplates && typeof candidate.organizerTemplates === "object") {
            const templates = candidate.organizerTemplates as Partial<OrganizerTemplates>;
            saved.organizerTemplates = {
              movie: typeof templates.movie === "string" ? templates.movie : defaultOrganizerTemplates.movie,
              series: templates.series === "{series_title} [[({year_range})]]"
                ? defaultOrganizerTemplates.series
                : typeof templates.series === "string" ? templates.series : defaultOrganizerTemplates.series,
              season: typeof templates.season === "string" ? templates.season : defaultOrganizerTemplates.season,
              episode: typeof templates.episode === "string" ? templates.episode : defaultOrganizerTemplates.episode,
            };
          }
          if (candidate.settings === null || (candidate.settings && typeof candidate.settings === "object")) saved.settings = candidate.settings;
        }
      } else {
        const oldTheme = localStorage.getItem("hevc-rebuild-theme");
        const oldRoot = localStorage.getItem("hevc-rebuild.root");
        const oldFfmpeg = localStorage.getItem("hevc-rebuild.ffmpeg");
        const oldRecursive = localStorage.getItem("hevc-rebuild.recursive");
        const oldMinimum = localStorage.getItem("hevc-rebuild.minimumSizeMib");
        if (oldTheme === "hacker") legacyTheme = true;
        if (oldTheme) saved.theme = normalizeTheme(oldTheme);
        if (oldRoot) saved.root = oldRoot;
        if (oldFfmpeg) saved.toolPaths = normalizeToolPaths(saved.toolPaths, oldFfmpeg);
        if (oldRecursive !== null) saved.recursive = oldRecursive !== "false";
        if (oldMinimum !== null) saved.minimumSizeMib = oldMinimum;
      }
    } catch {
      // The browser preview is optional; unavailable or invalid browser storage does not block the UI.
    }

    appPreferences = { ...appPreferences, ...saved };
    metadataLanguageDraft = appPreferences.metadataLanguage;
    publishTheme(normalizeTheme(appPreferences.theme));
    appPreferences.theme = theme;
    root = appPreferences.root ?? "";
    applyToolPaths(appPreferences.toolPaths);
    recursive = appPreferences.recursive;
    minimumSizeMib = appPreferences.minimumSizeMib;
    stopOnError = appPreferences.stopOnError;
    youtubeResolution = appPreferences.youtubeResolution;
    youtubeQualityMode = appPreferences.youtubeQualityMode;
    youtubeSourceMultiplier = appPreferences.youtubeSourceMultiplier;
    youtubeOutputDir = appPreferences.youtubeOutputDir ?? "";
    if (legacyTheme) localStorage.setItem(browserPreferencesKey, JSON.stringify(appPreferences));
  }

  async function copyUtilityText(value: string, description: string) {
    try {
      await navigator.clipboard.writeText(value);
      setNotice(`${description} copied to the clipboard.`, "success");
    } catch (error) {
      setNotice(`Could not copy ${description.toLowerCase()}: ${String(error)}`, "error");
    }
  }

  function handleUtilityContextMenu(event: MouseEvent) {
    const target = event.target instanceof Element ? event.target : null;
    if (!target?.closest(".app-shell")) return;
    const editingField = Boolean(target.closest("input:not([type]), input[type='text'], input[type='search'], input[type='number'], input[type='email'], input[type='url'], input[type='tel'], input[type='password'], textarea, [contenteditable]:not([contenteditable='false']), [role='textbox']"));

    if (!desktopShell && editingField) return;
    event.preventDefault();
    if (!desktopShell) return;
    if (editingField) {
      void showNativeEditingMenu();
      return;
    }
    void showUtilityContextMenu(target);
  }

  async function showNativeContextMenu(items: NonNullable<MenuOptions["items"]>) {
    try {
      if (contextMenu) {
        await contextMenu.close();
        contextMenu = null;
      }
      const menu = await Menu.new({ items });
      contextMenu = menu;
      await menu.popup(undefined, getCurrentWindow());
    } catch (error) {
      if (contextMenu) {
        void contextMenu.close().catch(() => {});
        contextMenu = null;
      }
      setNotice(`Could not open the native context menu: ${String(error)}`, "error");
    }
  }

  async function showNativeEditingMenu() {
    const items: NonNullable<MenuOptions["items"]> = [
      { item: "Cut" },
      { item: "Copy" },
      { item: "Paste" },
      { item: "Separator" },
      { item: "SelectAll" },
    ];
    await showNativeContextMenu(items);
  }

  async function showUtilityContextMenu(target: Element) {
    const mediaPath = target.closest<HTMLElement>("[data-media-path]")?.dataset.mediaPath;
    const logViewer = Boolean(target.closest(".diagnostic-card pre"));
    const hasTextSelection = Boolean(window.getSelection()?.toString().trim());
    const items: NonNullable<MenuOptions["items"]> = [
      { id: "choose-folder", text: "Choose media folder", enabled: !workflowBusy, action: () => { void browseFolder(); } },
      { id: "analyze-library", text: "Add current path to queue", enabled: Boolean(root.trim()) && !workflowBusy, action: () => { void analyzeFolder(); } },
      { item: "Separator" },
      { id: "home", text: "Home", enabled: page !== "home", action: () => { openPage("home"); } },
      { id: "library", text: "Media library", enabled: page !== "library", action: () => { openPage("library"); } },
      { id: "metadata", text: "Identify & organize", enabled: page !== "metadata", action: () => { openPage("metadata"); } },
      { id: "youtube", text: "YouTube prep", enabled: page !== "youtube", action: () => { openPage("youtube"); } },
      { id: "run", text: "Run setup", enabled: page !== "run", action: () => { openPage("run"); } },
      { id: "settings", text: "Settings", enabled: page !== "settings", action: () => { openPage("settings"); } },
      { id: "reports", text: "Reports & errors", enabled: page !== "reports", action: () => { openPage("reports"); } },
      { item: "Separator" },
      { id: "refresh-environment", text: "Refresh environment", enabled: !environmentLoading, action: () => { void loadEnvironment(); } },
    ];
    if (mediaPath) {
      items.push({ item: "Separator" });
      items.push({ id: "copy-media-path", text: "Copy media file path", action: () => { void copyUtilityText(mediaPath, "Media file path"); } });
    } else if (root.trim()) {
      items.push({ item: "Separator" });
      items.push({ id: "copy-library-path", text: "Copy library path", action: () => { void copyUtilityText(root, "Library path"); } });
    }
    if (logViewer && selectedLog) {
      items.push({ item: "Separator" });
      items.push({ id: "copy-diagnostic-log", text: "Copy diagnostic log", action: () => { void copyLog(); } });
    }
    if (hasTextSelection) items.push({ item: "Copy" });

    await showNativeContextMenu(items);
  }

  function isFailure(status: string | undefined) {
    const normalized = (status ?? "").toLowerCase();
    return normalized.includes("fail") || normalized.includes("error") || normalized.includes("skip");
  }

  function bytes(value: number | null | undefined) {
    if (value == null || !Number.isFinite(value)) return "—";
    const units = ["B", "KB", "MB", "GB", "TB"];
    let size = value;
    let unit = 0;
    while (size >= 1024 && unit < units.length - 1) {
      size /= 1024;
      unit += 1;
    }
    return `${size.toFixed(unit > 1 ? 1 : 0)} ${units[unit]}`;
  }

  function largestFileDiskFloor() {
    if (!scanResult?.entries.length) return "Analyze media for an estimate";
    const largestBytes = Math.max(...scanResult.entries.map((entry) => entry.size_bytes));
    const oneGiB = 1024 ** 3;
    const workingFloor = Math.max(oneGiB, Math.min(8 * oneGiB, largestBytes * 0.05));
    const extraReserve = Math.max(0, settings?.resources.min_free_disk_gib ?? 0) * oneGiB;
    return `${bytes(workingFloor + extraReserve)} for the largest file`;
  }

  function duration(seconds: number | undefined) {
    if (seconds == null || !Number.isFinite(seconds) || seconds < 0) return "—";
    const whole = Math.max(0, Math.round(seconds));
    const hours = Math.floor(whole / 3600);
    const minutes = Math.floor((whole % 3600) / 60);
    const remaining = whole % 60;
    if (hours) return minutes ? `${hours}h ${minutes}m` : `${hours}h`;
    if (minutes) return `${minutes}m ${remaining}s`;
    return `${remaining}s`;
  }

  function noteRunProgress(event: RunProgressEvent) {
    workspace.run.progress = event;
    workspace.run.progressSeenAt = Date.now();
    recordQueueRunProgress(event);
  }

  function openProgressDetails() {
    openPage(workspace.run.mode === "youtube" ? "youtube" : "run");
    requestAnimationFrame(() => requestAnimationFrame(() => {
      document.getElementById("run-activity")?.scrollIntoView({ behavior: "smooth", block: "start" });
    }));
  }

  function fileName(path: string) {
    return path.split(/[\\/]/).filter(Boolean).at(-1) ?? path;
  }

  function archivedOriginalPath(source: string): string {
    const parts = source.split(/[\\/]/);
    const name = parts.pop() || source;
    const slash = source.includes("\\") && !source.includes("/") ? "\\" : "/";
    return [...parts, "old", name].join(slash);
  }

  function renderFilenameChunk(
    chunk: string,
    allowed: readonly string[],
    values: Record<string, string>,
  ): { text: string; hasValue: boolean } | null {
    let text = "";
    let hasValue = false;
    let cursor = 0;
    while (cursor < chunk.length) {
      const open = chunk.indexOf("{", cursor);
      const close = chunk.indexOf("}", cursor);
      if (open < 0 && close < 0) {
        text += chunk.slice(cursor);
        break;
      }
      if (open < 0 || (close >= 0 && close < open)) return null;
      text += chunk.slice(cursor, open);
      const end = chunk.indexOf("}", open + 1);
      if (end < 0) return null;
      const token = chunk.slice(open + 1, end);
      if (!token || token.includes("{")) return null;
      const match = token.match(/^([a-z_]+)(?::(02|03|04))?$/);
      if (!match || !allowed.includes(match[1])) return null;
      const value = values[match[1]] ?? "";
      if (match[2] && value && !/^\d+$/.test(value)) return null;
      text += match[2] && value ? value.padStart(Number(match[2]), "0") : value;
      hasValue ||= Boolean(value);
      cursor = end + 1;
    }
    return { text, hasValue };
  }

  function renderFilenameTemplate(
    template: string,
    allowed: readonly string[],
    values: Record<string, string>,
  ): string | null {
    if (!template.trim() || new TextEncoder().encode(template).length > 1024) return null;
    let output = "";
    let rest = template;
    while (rest.length) {
      const open = rest.indexOf("[[");
      const actualClose = rest.indexOf("]]");
      if (open < 0 && actualClose < 0) {
        const tail = renderFilenameChunk(rest, allowed, values);
        if (!tail) return null;
        output += tail.text;
        break;
      }
      if (open < 0 || (actualClose >= 0 && actualClose < open)) return null;
      const prefix = renderFilenameChunk(rest.slice(0, open), allowed, values);
      const bodyStart = open + 2;
      const bodyEnd = rest.indexOf("]]", bodyStart);
      if (!prefix || bodyEnd < 0 || rest.slice(bodyStart, bodyEnd).includes("[[")) return null;
      const optional = renderFilenameChunk(rest.slice(bodyStart, bodyEnd), allowed, values);
      if (!optional) return null;
      output += prefix.text + (optional.hasValue ? optional.text : "");
      rest = rest.slice(bodyEnd + 2);
    }
    let cleaned = output.split(/\s+/u).filter(Boolean).join(" ")
      .replaceAll("( ", "(").replaceAll(" )", ")")
      .replaceAll("[ ", "[").replaceAll(" ]", "]")
      .trim();
    for (const [left, right] of [["(", ")"], ["[", "]"]] as const) {
      cleaned = cleaned.replaceAll(`${left}${right}`, "");
    }
    return cleaned.trim() || null;
  }

  function proposedFilenameForEntry(
    entry: MediaEntry,
    movie: QueueMovieReview | undefined,
    series: QueueSeriesReview | undefined,
    suggestion: QueueMetadataSuggestion | undefined,
    templates: OrganizerTemplates,
  ): { name: string; needsTitleReview: boolean } | null {
    const extension = entry.name.match(/\.[^.]+$/)?.[0] ?? "";
    const movieTitle = movie?.title.trim() || (suggestion?.kind === "movie" && suggestion.status === "ready" ? suggestion.title?.trim() : "") || "";
    if (movieTitle) {
      const isSuggestion = !movie;
      const name = renderFilenameTemplate(templates.movie, ["title", "year", "certification", "ext"], {
        title: movie?.title ?? movieTitle,
        year: movie?.year ?? suggestion?.year ?? "",
        certification: movie?.certification ?? "",
        ext: extension,
      });
      return name ? { name, needsTitleReview: isSuggestion } : null;
    }

    if (!series) return null;
    const assignment = series.episodeAssignments[entry.path] ??
      Object.entries(series.episodeAssignments).find(([path]) => queuePathKey(path) === queuePathKey(entry.path))?.[1];
    if (!assignment?.title.trim()) return null;
    const name = renderFilenameTemplate(templates.episode, ["series_title", "season", "episode", "episode_title", "ext"], {
      series_title: series.title,
      season: String(assignment.season),
      episode: String(assignment.episode),
      episode_title: assignment.title,
      ext: extension,
    });
    return name ? { name, needsTitleReview: false } : null;
  }

  function reviewDestinationPath(entry: MediaEntry, proposedName: string | null): { path: string; note: string } {
    const name = proposedName || entry.name;
    const perFile = appPreferences.fileOutputDirs[entry.path];
    const folder = (perFile || appPreferences.rebuildOutputDir || "").replace(/[\\/]+$/, "");
    if (folder) {
      const stem = name.replace(/\.[^.]+$/, "");
      return {
        path: `${folder}/${stem}.mkv`,
        note: perFile ? "Per-file output folder" : "Default output folder",
      };
    }
    const slash = Math.max(entry.path.lastIndexOf("/"), entry.path.lastIndexOf("\\"));
    const directory = slash >= 0 ? entry.path.slice(0, slash + 1) : "";
    return { path: `${directory}${name}`, note: "Proposed name in the source folder" };
  }

  function libraryPosterCard(entry: MediaEntry): LibraryPosterCard {
    const reviewedMovie = queueMovieReviews[entry.path];
    const reviewedSeries = reviewedSeriesForPath(entry.path);
    const reviewed = reviewedMovie ?? reviewedSeries;
    const suggestion = queueMatchSuggestions[entry.path];
    const proposal = proposedFilenameForEntry(entry, reviewedMovie, reviewedSeries, suggestion, appPreferences.organizerTemplates);
    const reviewedPoster = reviewed?.artworkCandidates.find((candidate) => candidate.id === reviewed.posterId);
    const localArtworkPath = reviewed?.localPosterPath ?? "";
    const localPreview = localArtworkPath ? queueArtworkPreviews[queuePathKey(localArtworkPath)] : "";
    const suggestionArt = suggestion?.artwork?.thumbnailUrl || suggestion?.artwork?.imageUrl || "";
    const reviewedArt = reviewedPoster?.thumbnailUrl || reviewedPoster?.imageUrl || "";
    const posterUrl = localPreview || reviewedArt || suggestionArt || null;
    const title = reviewed?.title.trim()
      || (suggestion?.status === "ready" ? suggestion.title?.trim() ?? "" : "")
      || entry.name.replace(/\.[^.]+$/, "");
    const year = reviewed?.year || (suggestion?.status === "ready" ? suggestion.year ?? "" : "");
    const localRecord = Boolean(reviewed && (reviewed.recordId === "local-nfo" || reviewed.recordId.startsWith("local-nfo:") || reviewed.recordId.startsWith("local:")));
    let matchState: LibraryPosterCard["matchState"] = "none";
    let matchLabel = posterUrl ? "Poster ready" : "No artwork yet";
    if (suggestion?.status === "searching" && !reviewed) {
      matchState = "matching";
      matchLabel = "Matching";
    } else if (suggestion?.status === "error" && !reviewed) {
      matchState = "error";
      matchLabel = "Lookup failed";
    } else if (localRecord) {
      matchState = "local";
      matchLabel = "Local metadata";
    } else if (reviewed || suggestion?.autoAssociated) {
      matchState = "matched";
      matchLabel = "Matched";
    } else if (suggestion?.status === "ready" && suggestion.title) {
      matchState = "suggested";
      matchLabel = "Suggested";
    } else if (metadataLookupStatus?.status === "searching" || suggestion?.status === "searching") {
      matchState = "matching";
      matchLabel = "Matching";
    }
    const provider = localRecord
      ? "From a local NFO"
      : reviewed
        ? reviewed.recordId.split(":")[0].toUpperCase()
        : suggestion?.provider?.toUpperCase() ?? "";
    const tune = effectiveContentTuneForEntry(entry, reviewed, settingsForEntry(entry));
    const forecast = resourceForecastFor(entry);
    const queueRun = queueRunStateForPath(entry.path);
    const rowProgress = workspace.run.progress?.filePath && queuePathKey(workspace.run.progress.filePath) === queuePathKey(entry.path) ? workspace.run.progress : null;
    const profile = entry.profile;
    return {
      path: entry.path,
      filename: entry.name,
      relativePath: entry.relative_path,
      selected: workspace.queue.selectedPaths.includes(entry.path),
      title,
      year,
      proposedName: proposal?.name && proposal.name !== entry.name ? proposal.name : null,
      proposedNeedsReview: proposal?.needsTitleReview ?? false,
      posterUrl,
      localArtworkPath,
      matchState,
      matchLabel,
      provider,
      animated: tune === "animation",
      statusLabel: statusLabel(entry.status),
      sourceBytes: bytes(entry.size_bytes),
      profileLine: profileSummary(entry),
      streamLine: profile
        ? `${profile.video.secondary_count + 1} video · ${profile.audio_count} audio · ${profile.subtitle_count} subtitles · ${profile.attachment_count} attachments`
        : "Streams not available yet",
      memoryLine: forecast
        ? `${forecast.level} · ${forecast.planned_workers ?? "—"} workers · ${forecast.planned_frame_threads ?? "—"} frame threads`
        : "",
      outputLine: appPreferences.fileOutputDirs[entry.path]
        ? `Output: ${appPreferences.fileOutputDirs[entry.path]}`
        : appPreferences.rebuildOutputDir
          ? `Default output: ${appPreferences.rebuildOutputDir}`
          : "Output: source folder",
      outputIsCustom: Boolean(appPreferences.fileOutputDirs[entry.path]),
      error: entry.error ?? "",
      warning: entry.warning ?? "",
      stale: entry.queue_stale ?? "",
      runLabel: queueRun ? `${queueRunStatusLabel(queueRun.status)}${queueRun.mode ? ` · ${queueRun.mode === "organize" ? "Rename" : queueRun.mode}` : ""}` : "",
      progressLabel: rowProgress?.phase ?? "",
      progressPercent: rowProgress?.filePercent ?? null,
      kind: "media",
      seasonNumber: null,
      mediaPaths: [entry.path],
      selection: workspace.queue.selectedPaths.includes(entry.path) ? "all" : "none",
      animation: animationSelection([entry.path]),
      summary: "",
      reviewLabel: "",
    };
  }

  function localRecordFromAsset(item: LocalAssetRecord) {
    const rating = item.metadata.rating && item.metadata.rating > 0 ? String(item.metadata.rating) : "";
    return {
      title: item.metadata.title || item.metadata.showTitle || "",
      showTitle: item.metadata.showTitle || item.metadata.title || "",
      year: item.metadata.year || "",
      certification: item.metadata.certification || "",
      plot: item.metadata.plot || "",
      genresText: item.metadata.genres.join(", "),
      ratingText: rating,
      season: item.metadata.season,
      episode: item.metadata.episode,
      airdate: item.metadata.airdate,
      seasonNames: item.metadata.seasonNames ?? {},
      artwork: item.artwork,
    };
  }

  function animationSelection(paths: string[]): LibraryPosterCard["animation"] {
    if (!paths.length) return "none";
    const marked = paths.filter((path) => {
      const entry = queuedEntries.find((item) => item.path === path);
      if (!entry) return fileTuneOverrides[path] === "animation";
      return effectiveContentTuneForEntry(entry, queueMovieReviews[path] ?? reviewedSeriesForPath(path), settingsForEntry(entry)) === "animation";
    }).length;
    if (!marked) return "none";
    return marked === paths.length ? "all" : "some";
  }

  function cardSelection(paths: string[]): LibraryPosterCard["selection"] {
    const selected = paths.filter((path) => workspace.queue.selectedPaths.includes(path)).length;
    if (!selected) return "none";
    return selected === paths.length ? "all" : "some";
  }

  function localPosterUrl(path: string): string | null {
    return path ? queueArtworkPreviews[queuePathKey(path)] || null : null;
  }

  function displayPosterUrl(path: string): string | null {
    if (/^https:\/\//u.test(path)) return path;
    return localPosterUrl(path);
  }

  function knownLocalPoster(path: string): string {
    if (!path || path.startsWith("https://")) return "";
    const key = queuePathKey(path);
    return Object.values(libraryLocalPosters).some((item) => queuePathKey(item) === key) ? path : "";
  }

  function chosenPoster(path: string): string {
    if (!path) return "";
    if (path.startsWith("https://")) return path;
    if (knownLocalPoster(path)) return path;
    return queueArtworkPreviews[queuePathKey(path)] ? path : "";
  }

  function providerPosterUrl(review: QueueSeriesReview | undefined): string {
    return review?.artworkCandidates.find((image) => image.imageUrl.startsWith("https://"))?.imageUrl
      || review?.artworkCandidates.find((image) => image.thumbnailUrl.startsWith("https://"))?.thumbnailUrl
      || "";
  }

  function remoteArtworkUrl(candidate: QueueArtworkCandidate | null | undefined): string {
    if (!candidate) return "";
    return candidate.imageUrl.startsWith("https://") ? candidate.imageUrl : candidate.thumbnailUrl.startsWith("https://") ? candidate.thumbnailUrl : "";
  }

  function seriesSuggestionArtwork(group: LibrarySeriesGroup): QueueArtworkCandidate | null {
    for (const path of group.episodePaths) {
      const artwork = queueMatchSuggestions[path]?.artwork;
      if (remoteArtworkUrl(artwork)) return artwork;
    }
    return null;
  }

  function seriesProviderLabel(group: LibrarySeriesGroup, review: QueueSeriesReview | undefined): string {
    const localRecord = libraryLocalRecords[queuePathKey(group.root)];
    if (localRecord?.title) return "From a local NFO";
    const record = review?.recordId ?? "";
    if (record && !record.startsWith("local")) return record.split(":")[0].toUpperCase();
    const provider = seriesSuggestionArtwork(group)?.provider ?? "";
    return provider ? provider.toUpperCase() : "";
  }

  function visiblePosterUrl(localPath: string, remotePath: string): string | null {
    const localPreview = localPath && !localPath.startsWith("https://") ? displayPosterUrl(localPath) : null;
    return localPreview || displayPosterUrl(remotePath) || displayPosterUrl(localPath);
  }

  function seasonArtworkPath(review: QueueSeriesReview | undefined, season: number, directory: string): string {
    const onDisk = directory ? libraryLocalPosters[queuePathKey(directory)] ?? "" : "";
    const saved = chosenPoster(review?.localSeasonPosterPaths?.[String(season)] ?? "");
    return onDisk || saved || providerPosterUrl(review);
  }

  function seasonPosterSources(review: QueueSeriesReview | undefined, season: number, directory: string): { localPath: string; remotePath: string } {
    const onDisk = directory ? libraryLocalPosters[queuePathKey(directory)] ?? "" : "";
    const saved = review?.localSeasonPosterPaths?.[String(season)] ?? "";
    const remotePath = saved.startsWith("https://") ? saved : "";
    const localPath = onDisk || (saved.startsWith("https://") ? "" : chosenPoster(saved));
    return { localPath, remotePath };
  }

  function groupPosterCard(group: LibrarySeriesGroup): LibraryPosterCard {
    const review = reviewedSeriesForPath(group.episodePaths[0] ?? group.root);
    const localPath = libraryLocalPosters[queuePathKey(group.root)] || chosenPoster((review?.localPosterPath ?? "").startsWith("https://") ? "" : review?.localPosterPath ?? "");
    const remotePath = providerPosterUrl(review) || remoteArtworkUrl(seriesSuggestionArtwork(group));
    const artworkPath = localPath || remotePath;
    const seasonCount = group.seasons.length;
    const episodeCount = group.episodePaths.length;
    return {
      ...libraryPosterCard({
        path: group.root,
        relative_path: group.folderName,
        name: group.folderName,
        size_bytes: 0,
        status: "Ready",
        already_processed: false,
        profile: null,
        error: null,
      }),
      path: group.root,
      filename: group.folderName,
      title: review?.title.trim() || group.folderName,
      year: review?.year ?? "",
      posterUrl: visiblePosterUrl(localPath, remotePath),
      localArtworkPath: artworkPath,
      provider: seriesProviderLabel(group, review),
      matchState: artworkPath ? "local" : "none",
      matchLabel: "",
      kind: "series",
      seasonNumber: null,
      mediaPaths: group.episodePaths,
      selection: cardSelection(group.episodePaths),
      animation: animationSelection(group.episodePaths),
      summary: `${seasonCount} season${seasonCount === 1 ? "" : "s"} · ${episodeCount} episode${episodeCount === 1 ? "" : "s"}`,
      reviewLabel: "Review series",
      proposedName: null,
    };
  }

  function seasonPosterCard(group: LibrarySeriesGroup, season: LibrarySeriesGroup["seasons"][number]): LibraryPosterCard {
    const review = reviewedSeriesForPath(season.episodePaths[0] ?? group.root);
    const sources = seasonPosterSources(review, season.season, season.directory);
    const artworkPath = sources.localPath || sources.remotePath;
    const waitingForSeasonPoster = !artworkPath && Boolean(seriesSuggestionArtwork(group) || review?.recordId);
    const episodeCount = season.episodePaths.length;
    return {
      ...groupPosterCard(group),
      path: season.directory || `${group.root}::season:${season.season}`,
      filename: season.label,
      title: season.label,
      year: "",
      posterUrl: visiblePosterUrl(sources.localPath, sources.remotePath),
      localArtworkPath: artworkPath,
      matchLabel: "",
      matchState: artworkPath ? "local" : waitingForSeasonPoster ? "matched" : "none",
      kind: "season",
      seasonNumber: season.season,
      mediaPaths: season.episodePaths,
      selection: cardSelection(season.episodePaths),
      animation: animationSelection(season.episodePaths),
      summary: `${episodeCount} episode${episodeCount === 1 ? "" : "s"}`,
      reviewLabel: "Review season",
    };
  }

  function episodePosterCard(entry: MediaEntry, artworkPath: string): LibraryPosterCard {
    const card = libraryPosterCard(entry);
    const identity = episodeIdentity(entry.name);
    const seasonPoster = artworkPath ? displayPosterUrl(artworkPath) : "";
    return {
      ...card,
      title: identity?.title || card.title,
      year: "",
      posterUrl: seasonPoster || card.posterUrl,
      localArtworkPath: artworkPath || card.localArtworkPath,
      matchState: artworkPath || card.posterUrl ? (card.matchState === "none" ? "local" : card.matchState) : card.matchState,
      matchLabel: card.matchState === "matched" || card.matchState === "suggested" || card.matchState === "matching" || card.matchState === "error" ? card.matchLabel : "",
    };
  }

  function seasonPremiereYear(review: QueueSeriesReview | undefined, season: number | null): string {
    if (!review || season === null) return "";
    const dates = [...review.episodes, ...Object.values(review.episodeAssignments)]
      .filter((episode) => episode.season === season)
      .map((episode) => episode.airdate?.trim() ?? "")
      .filter((airdate) => /^\d{4}-\d{2}-\d{2}$/.test(airdate))
      .sort();
    return dates[0]?.slice(0, 4) ?? "";
  }

  function blankSeasonReview(title: string, series: QueueSeriesReview | undefined): QueueSeasonReview {
    return {
      title,
      year: series?.year ?? "",
      certification: series?.certification ?? "",
      plot: "",
      genresText: series?.genresText ?? "",
      ratingText: "",
    };
  }

  function blankSeriesReview(group: LibrarySeriesGroup): QueueSeriesReview {
    const local = libraryLocalRecords[queuePathKey(group.root)];
    return {
      title: local?.title || group.folderName,
      year: local?.year ?? "",
      certification: local?.certification ?? "",
      recordId: "",
      shortDescription: "",
      plot: local?.plot ?? "",
      genresText: local?.genresText ?? "",
      ratingText: local?.ratingText ?? "",
      sourceUrl: "",
      artworkCandidates: [],
      posterId: "",
      localPosterPath: libraryLocalPosters[queuePathKey(group.root)] ?? "",
      localSeasonPosterPaths: {},
      seasonRecords: {},
      nfoPaths: [],
      nfoError: "",
      episodes: [],
      episodeAssignments: {},
    };
  }

  function seriesReviewStorageKey(root: string): string {
    return Object.keys(queueSeriesReviews).find((key) => queuePathKey(key) === queuePathKey(root)) ?? root;
  }

  function ensureSeriesReview(group: LibrarySeriesGroup): string {
    const key = seriesReviewStorageKey(group.root);
    const local = libraryLocalRecords[queuePathKey(group.root)];
    const current = queueSeriesReviews[key];
    const seeded = current ?? blankSeriesReview(group);
    const next: QueueSeriesReview = {
      ...seeded,
      seasonRecords: seeded.seasonRecords ?? {},
      localSeasonPosterPaths: seeded.localSeasonPosterPaths ?? {},
      episodeAssignments: seeded.episodeAssignments ?? {},
      title: seeded.title.trim() || local?.title || group.folderName,
      year: seeded.year || local?.year || "",
      certification: seeded.certification || local?.certification || "",
      plot: seeded.plot || local?.plot || "",
      genresText: seeded.genresText || local?.genresText || "",
      ratingText: seeded.ratingText || local?.ratingText || "",
      localPosterPath: seeded.localPosterPath || libraryLocalPosters[queuePathKey(group.root)] || "",
    };
    const changed = !current
      || next.title !== current.title
      || next.year !== current.year
      || next.certification !== current.certification
      || next.plot !== current.plot
      || next.genresText !== current.genresText
      || next.ratingText !== current.ratingText
      || next.localPosterPath !== current.localPosterPath
      || !current.seasonRecords
      || !current.episodeAssignments;
    if (changed) queueSeriesReviews = { ...queueSeriesReviews, [key]: next };
    return key;
  }

  function ensureSeasonReview(group: LibrarySeriesGroup, season: number): { key: string; seasonKey: string } {
    const key = ensureSeriesReview(group);
    const seasonKey = String(season);
    const review = queueSeriesReviews[key];
    const named = libraryLocalRecords[queuePathKey(group.root)]?.seasonNames?.[String(season)] ?? "";
    const generated = group.seasons.find((item) => item.season === season)?.label ?? `Season ${season}`;
    const current = review.seasonRecords[seasonKey];
    const premiereYear = seasonPremiereYear(review, season);
    if (!current) {
      const created = blankSeasonReview(named || generated, review);
      if (premiereYear && premiereYear !== review.year) created.year = premiereYear;
      queueSeriesReviews = {
        ...queueSeriesReviews,
        [key]: { ...review, seasonRecords: { ...review.seasonRecords, [seasonKey]: created } },
      };
    } else if ((premiereYear && current.year === review.year && premiereYear !== review.year) || (named && (current.title === generated || current.title === `Season ${String(season).padStart(2, "0")}`))) {
      queueSeriesReviews = {
        ...queueSeriesReviews,
        [key]: { ...review, seasonRecords: { ...review.seasonRecords, [seasonKey]: {
          ...current,
          title: named && (current.title === generated || current.title === `Season ${String(season).padStart(2, "0")}`) ? named : current.title,
          year: premiereYear && current.year === review.year && premiereYear !== review.year ? premiereYear : current.year,
        } } },
      };
    }
    return { key, seasonKey };
  }

  function containerTags(entry: MediaEntry | undefined): Record<string, string> {
    const format = entry?.profile?.source_metadata?.format;
    if (!format || typeof format !== "object") return {};
    const tags = (format as { tags?: unknown }).tags;
    if (!tags || typeof tags !== "object") return {};
    const values: Record<string, string> = {};
    for (const [key, value] of Object.entries(tags)) {
      if (typeof value === "string" && value.trim()) values[key.toLowerCase()] = value.trim();
    }
    return values;
  }

  function episodeFileFacts(path: string): { title: string; plot: string; airdate: string | null } {
    const entry = queuedEntries.find((item) => item.path === path);
    const tags = containerTags(entry);
    const title = tags.title && tags.title !== tags.showtitle ? tags.title : "";
    const plot = tags.synopsis || tags.description || "";
    const dated = tags.date_released || "";
    const seriesDir = path.split(/[\\/]/u).slice(0, -2).join("/");
    const dates = queuedEntries
      .filter((item) => seriesDir && item.path.startsWith(`${seriesDir}/`))
      .map((item) => containerTags(item).date_released)
      .filter((value): value is string => Boolean(value));
    const airdate = dated && new Set(dates).size > 1 ? dated : null;
    return { title, plot, airdate };
  }

  function episodeDraft(path: string, seriesTitle = ""): QueueTvEpisode {
    const entry = queuedEntries.find((item) => item.path === path);
    const identity = episodeIdentity(entry?.name ?? "");
    const local = libraryLocalRecords[queuePathKey(path)];
    const file = episodeFileFacts(path);
    const localTitle = local?.title.trim() ?? "";
    const showTitle = (local?.showTitle || seriesTitle).trim();
    const ownFile = Boolean(localTitle && localTitle !== showTitle);
    return {
      season: identity?.season ?? 0,
      episode: identity?.episode && identity.episode > 0 ? identity.episode : 1,
      title: (ownFile ? localTitle : "") || file.title || identity?.title || entry?.name || "Episode",
      airdate: (ownFile ? local?.airdate : null) || file.airdate,
      summary: (ownFile ? local?.plot : "") || file.plot,
      sourceUrl: "",
    };
  }

  function ensureEpisodeReview(group: LibrarySeriesGroup, path: string): { key: string; episode: QueueTvEpisode } {
    const key = ensureSeriesReview(group);
    const review = queueSeriesReviews[key];
    const draft = episodeDraft(path, review.title);
    const existing = review.episodeAssignments[path];
    const identityTitle = episodeIdentity(queuedEntries.find((item) => item.path === path)?.name ?? "")?.title ?? "";
    const match = review.episodes.find((item) => item.season === (existing?.season ?? draft.season) && item.episode === (existing?.episode ?? draft.episode));
    const starting = existing
      ? { ...existing, summary: review.plot && existing.summary === review.plot ? draft.summary : existing.summary }
      : draft;
    const episode = fillEpisodeFromCatalog(starting, match, review.title, review.plot, identityTitle);
    if (existing && episode.title === existing.title && episode.summary === existing.summary && episode.airdate === existing.airdate && episode.sourceUrl === existing.sourceUrl) {
      return { key, episode: existing };
    }
    queueSeriesReviews = {
      ...queueSeriesReviews,
      [key]: { ...review, episodeAssignments: { ...review.episodeAssignments, [path]: episode } },
    };
    return { key, episode };
  }

  function tvReviewForEpisode(entry: MediaEntry): LibraryTvReviewTarget | null {
    const identity = episodeIdentity(entry.name);
    const group = librarySeriesGroups.find((item) => item.episodePaths.some((path) => queuePathKey(path) === queuePathKey(entry.path)))
      ?? (scanResult ? groupLibraryEntries(scanResult.root, queuedEntries) : []).find((item) => item.episodePaths.some((path) => queuePathKey(path) === queuePathKey(entry.path)));
    if (!group || !identity) return null;
    ensureEpisodeReview(group, entry.path);
    return { level: "episode", seriesRoot: group.root, season: identity.season, episodePath: entry.path };
  }

  function seriesGroupForRoot(root: string): LibrarySeriesGroup | undefined {
    return librarySeriesGroups.find((item) => queuePathKey(item.root) === queuePathKey(root))
      ?? (scanResult ? groupLibraryEntries(scanResult.root, queuedEntries) : []).find((item) => queuePathKey(item.root) === queuePathKey(root));
  }

  async function showLibraryReview(target: LibraryTvReviewTarget) {
    const sample = target.episodePath ?? seriesGroupForRoot(target.seriesRoot)?.episodePaths[0] ?? "";
    metadataFocusPath = sample || metadataFocusPath;
    libraryTvReview = target;
    metadataDialogOpen = true;
    await tick();
    if (metadataDialogOpen && metadataDialogElement && !metadataDialogElement.open) metadataDialogElement.showModal();
  }

  function openSeriesReview(root: string) {
    const group = seriesGroupForRoot(root);
    if (!group) return;
    const key = ensureSeriesReview(group);
    const posters = posterPathsForReview(group, null);
    const sample = group.episodePaths[0] ?? "";
    for (const path of posters) if (sample) void ensureQueueArtworkPreview(sample, path);
    void showLibraryReview({ level: "series", seriesRoot: key, season: null, episodePath: sample || null });
  }

  function openSeasonReview(root: string, season: number) {
    const group = seriesGroupForRoot(root);
    if (!group) return;
    const { key } = ensureSeasonReview(group, season);
    const posters = posterPathsForReview(group, season);
    const sample = group.seasons.find((item) => item.season === season)?.episodePaths[0] ?? group.episodePaths[0] ?? "";
    for (const path of posters) if (sample) void ensureQueueArtworkPreview(sample, path);
    void showLibraryReview({ level: "season", seriesRoot: key, season, episodePath: sample || null });
  }

  function activeReviewPosterPath(target: LibraryTvReviewTarget, review: QueueSeriesReview | undefined): string {
    const group = seriesGroupForRoot(target.seriesRoot);
    if (target.level === "series") {
      return chosenPoster(review?.localPosterPath ?? "")
        || (group ? libraryLocalPosters[queuePathKey(group.root)] ?? "" : "")
        || providerPosterUrl(review);
    }
    const directory = group?.seasons.find((item) => item.season === target.season)?.directory ?? "";
    return seasonArtworkPath(review, target.season ?? 0, directory);
  }

  function posterPathsForReview(group: LibrarySeriesGroup, season: number | null): string[] {
    const seriesArtwork = libraryLocalRecords[queuePathKey(group.root)]?.artwork ?? [];
    const seasonDirectory = season === null ? "" : group.seasons.find((item) => item.season === season)?.directory ?? "";
    const seasonArtwork = seasonDirectory ? libraryLocalRecords[queuePathKey(seasonDirectory)]?.artwork ?? [] : [];
    const images = season === null ? seriesArtwork : [...seasonArtwork, ...seriesArtwork];
    const filtered = images.filter((image) => !/fanart|banner|landscape|clearlogo|logo/i.test(`${image.role} ${image.name}`));
    const chosen = (filtered.length ? filtered : images).map((image) => image.path);
    return [...new Set(chosen)];
  }

  function openLibraryReview(path: string) {
    const card = libraryPosterCards.find((item) => item.path === path);
    if (card?.kind === "series") {
      openSeriesReview(path);
      return;
    }
    if (card?.kind === "season" && card.seasonNumber !== null && libraryBrowse) {
      openSeasonReview(libraryBrowse.seriesRoot, card.seasonNumber);
    }
  }

  function reviewCurrentBrowse() {
    if (!libraryBrowse) return;
    if (libraryBrowse.season === null) openSeriesReview(libraryBrowse.seriesRoot);
    else openSeasonReview(libraryBrowse.seriesRoot, libraryBrowse.season);
  }

  function tvReviewHeading(target: LibraryTvReviewTarget): string {
    const review = queueSeriesReviews[seriesReviewStorageKey(target.seriesRoot)];
    if (!review) return target.episodePath ? episodeDraft(target.episodePath).title : "";
    if (target.level === "series") return review.title;
    if (target.level === "season" && target.season !== null) return review.seasonRecords?.[String(target.season)]?.title || `Season ${target.season}`;
    if (target.episodePath) return review.episodeAssignments?.[target.episodePath]?.title?.trim() || episodeDraft(target.episodePath).title;
    return "";
  }

  function tvReviewPaths(target: LibraryTvReviewTarget): string[] {
    const group = seriesGroupForRoot(target.seriesRoot);
    if (target.level === "episode" && target.episodePath) return [target.episodePath];
    if (target.level === "season" && target.season !== null) return group?.seasons.find((item) => item.season === target.season)?.episodePaths ?? [];
    return group?.episodePaths ?? [];
  }

  function patchTvField(target: LibraryTvReviewTarget, field: string, value: string) {
    patchSeriesReview(target.seriesRoot, (review) => {
      if (target.level === "episode" && target.episodePath) {
        const current = review.episodeAssignments[target.episodePath] ?? episodeDraft(target.episodePath);
        const next = { ...current };
        if (field === "title") next.title = value;
        if (field === "plot") next.summary = value;
        if (field === "airdate") next.airdate = value || null;
        if (field === "comment") next.comment = value;
        if (field === "certification") {
          const inherited = sharedCertificateFor(target, review);
          next.certification = inherited && value.trim() === inherited ? "" : value;
        }
        if (field === "season") {
          const number = Number(value);
          if (!Number.isInteger(number) || number < 0) return review;
          next.season = number;
        }
        if (field === "episode") {
          const number = Number(value);
          if (!Number.isInteger(number) || number < 1) return review;
          next.episode = number;
        }
        return { ...review, episodeAssignments: { ...review.episodeAssignments, [target.episodePath]: next } };
      }
      if (target.level === "season" && target.season !== null) {
        const seasonKey = String(target.season);
        const current = review.seasonRecords[seasonKey] ?? blankSeasonReview(`Season ${target.season}`, review);
        const inherited = field === "plot" ? review.plot.trim() : field === "ratingText" ? review.ratingText.trim() : field === "certification" ? review.certification.trim() : "";
        const nextValue = inherited && value.trim() === inherited ? "" : value;
        return { ...review, seasonRecords: { ...review.seasonRecords, [seasonKey]: { ...current, [field]: nextValue } } };
      }
      return { ...review, [field]: value };
    });
  }

  function reviewPosterChoices(group: LibrarySeriesGroup, season: number | null, review: QueueSeriesReview | undefined): { path: string; url: string }[] {
    const local = posterPathsForReview(group, season).map((path) => ({ path, url: displayPosterUrl(path) ?? "" }));
    const remote = (review?.artworkCandidates ?? [])
      .map((image) => image.imageUrl || image.thumbnailUrl)
      .filter((url) => url.startsWith("https://"))
      .map((url) => ({ path: url, url }));
    const seen = new Set(local.map((item) => item.path));
    const choices = [...local, ...remote.filter((item) => !seen.has(item.path))];
    const selected = season === null ? review?.localPosterPath ?? "" : review?.localSeasonPosterPaths?.[String(season ?? "")] ?? "";
    if (selected.startsWith("https://") && !choices.some((item) => item.path === selected)) {
      choices.unshift({ path: selected, url: selected });
    }
    return choices;
  }

  function sharedCertificateFor(target: LibraryTvReviewTarget, review: QueueSeriesReview): string {
    const seriesCertificate = review.certification.trim();
    if (target.level !== "episode") return seriesCertificate;
    const seasonCertificate = target.season === null ? "" : review.seasonRecords?.[String(target.season)]?.certification.trim() ?? "";
    return seasonCertificate || seriesCertificate;
  }

  function ownCertificateFor(target: LibraryTvReviewTarget, review: QueueSeriesReview): string {
    if (target.level === "episode" && target.episodePath) return review.episodeAssignments[target.episodePath]?.certification?.trim() ?? "";
    if (target.level === "season" && target.season !== null) return review.seasonRecords?.[String(target.season)]?.certification.trim() ?? "";
    return "";
  }

  function certificateNoteFor(target: LibraryTvReviewTarget, review: QueueSeriesReview): string {
    if (target.level === "series") return "";
    if (ownCertificateFor(target, review)) return target.level === "episode" ? "Saved for this episode." : "Saved for this season.";
    if (!sharedCertificateFor(target, review)) return "No certificate is published.";
    if (target.level === "episode" && target.season !== null && review.seasonRecords?.[String(target.season)]?.certification.trim()) return "Using this season's certificate.";
    return "Using the series certificate.";
  }

  function seasonPlotNote(review: QueueSeriesReview, season: QueueSeasonReview | undefined): string {
    if (season?.plot.trim()) return "Saved for this season.";
    if (review.plot.trim()) return "This season has no plot of its own. Showing the series description.";
    return "No season description is published.";
  }

  function seasonRatingNote(review: QueueSeriesReview, season: QueueSeasonReview | undefined): string {
    if (season?.ratingText.trim()) return "Saved for this season.";
    if (review.ratingText.trim()) return "This season has no score of its own. Showing the series score.";
    return "No season score is published.";
  }

  function sharedCommentFor(target: LibraryTvReviewTarget, review: QueueSeriesReview): string {
    const globalComment = settings?.metadata.comment?.trim() ?? "";
    const seriesComment = review.comment?.trim() ?? "";
    if (target.level === "series") return globalComment;
    if (target.level === "season") return seriesComment || globalComment;
    const seasonComment = target.season === null ? "" : review.seasonRecords?.[String(target.season)]?.comment?.trim() ?? "";
    return seasonComment || seriesComment || globalComment;
  }

  function ownCommentFor(target: LibraryTvReviewTarget, review: QueueSeriesReview): string {
    if (target.level === "episode" && target.episodePath) return review.episodeAssignments[target.episodePath]?.comment?.trim() ?? "";
    if (target.level === "season" && target.season !== null) return review.seasonRecords?.[String(target.season)]?.comment?.trim() ?? "";
    return review.comment?.trim() ?? "";
  }

  function commentNoteFor(target: LibraryTvReviewTarget, review: QueueSeriesReview): string {
    if (ownCommentFor(target, review)) {
      if (target.level === "episode") return "Saved for this episode.";
      if (target.level === "season") return "Saved for this season. Episodes without their own comment use it.";
      return "Saved for this series. Seasons and episodes without their own comment use it.";
    }
    if (target.level === "episode") {
      const seasonComment = target.season === null ? "" : review.seasonRecords?.[String(target.season)]?.comment?.trim() ?? "";
      if (seasonComment) return "Using this season's comment. Edit it to save one for this episode.";
      if (review.comment?.trim()) return "Using the series comment. Edit it to save one for this episode.";
    }
    if (target.level === "season" && review.comment?.trim()) return "Using the series comment. Edit it to save one for this season.";
    return "Using the comment from Settings. Edit it to save one here.";
  }

  function setMovieComment(path: string, value: string) {
    const review = queueMovieReviews[path];
    if (!review) return;
    const globalComment = settings?.metadata.comment?.trim() ?? "";
    const next = value.trim() === globalComment ? "" : value;
    queueMovieReviews = { ...queueMovieReviews, [path]: { ...review, comment: next } };
  }

  function setReviewComment(target: LibraryTvReviewTarget, value: string) {
    const review = queueSeriesReviews[seriesReviewStorageKey(target.seriesRoot)];
    if (!review) return;
    const next = value.trim() === sharedCommentFor(target, review) ? "" : value;
    patchTvField(target, "comment", next);
  }

  function resolvedFileComment(series: QueueSeriesReview, assignment: QueueTvEpisode | undefined): string {
    const episodeComment = assignment?.comment?.trim() ?? "";
    if (episodeComment) return episodeComment;
    const seasonComment = assignment ? series.seasonRecords?.[String(assignment.season)]?.comment?.trim() ?? "" : "";
    if (seasonComment) return seasonComment;
    return series.comment?.trim() ?? "";
  }

  function selectReviewPoster(target: LibraryTvReviewTarget, path: string) {
    const sample = target.episodePath ?? seriesGroupForRoot(target.seriesRoot)?.episodePaths[0] ?? "";
    if (sample && !path.startsWith("https://")) void ensureQueueArtworkPreview(sample, path);
    if (target.level !== "series" && target.season !== null) {
      patchSeriesReview(target.seriesRoot, (review) => ({
        ...review,
        localSeasonPosterPaths: { ...review.localSeasonPosterPaths, [String(target.season)]: path },
      }));
      return;
    }
    patchSeriesReview(target.seriesRoot, (review) => ({ ...review, localPosterPath: path, posterId: "" }));
  }

  function setPathsAnimated(paths: string[], animated: boolean) {
    const next = { ...fileTuneOverrides };
    for (const path of paths) next[path] = animated ? "animation" : "none";
    fileTuneOverrides = next;
  }

  function patchSeriesReview(root: string, update: (review: QueueSeriesReview) => QueueSeriesReview) {
    const key = seriesReviewStorageKey(root);
    const group = seriesGroupForRoot(root);
    const current = queueSeriesReviews[key] ?? (group ? blankSeriesReview(group) : null);
    if (!current) return;
    queueSeriesReviews = { ...queueSeriesReviews, [key]: update(current) };
  }

  async function chooseReviewImage() {
    const target = libraryTvReview;
    if (!target) return;
    const selected = await openDialog({
      directory: false,
      multiple: false,
      title: "Choose a poster image",
      defaultPath: target.seriesRoot,
      filters: [{ name: "Images", extensions: ["jpg", "jpeg", "png", "webp"] }],
    });
    const path = Array.isArray(selected) ? selected[0] : selected;
    if (!path) return;
    const sample = target.episodePath ?? seriesGroupForRoot(target.seriesRoot)?.episodePaths[0] ?? "";
    if (sample) void ensureQueueArtworkPreview(sample, path);
    if (target.level === "season" && target.season !== null) {
      patchSeriesReview(target.seriesRoot, (review) => ({
        ...review,
        localSeasonPosterPaths: { ...review.localSeasonPosterPaths, [String(target.season)]: path },
      }));
      return;
    }
    if (target.level === "episode" && target.season !== null) {
      patchSeriesReview(target.seriesRoot, (review) => ({
        ...review,
        localSeasonPosterPaths: { ...review.localSeasonPosterPaths, [String(target.season)]: path },
      }));
      return;
    }
    patchSeriesReview(target.seriesRoot, (review) => ({ ...review, localPosterPath: path, posterId: "" }));
  }

  function openLibraryPoster(path: string) {
    const card = libraryPosterCards.find((item) => item.path === path);
    if (card?.kind === "series") {
      libraryBrowse = { seriesRoot: path, season: null };
      return;
    }
    if (card?.kind === "season" && card.seasonNumber !== null && libraryBrowse) {
      libraryBrowse = { seriesRoot: libraryBrowse.seriesRoot, season: card.seasonNumber };
      return;
    }
    const entry = visibleEntries.find((candidate) => candidate.path === path)
      ?? currentPageEntries.find((candidate) => candidate.path === path)
      ?? queuedEntries.find((candidate) => candidate.path === path);
    if (entry) void openMetadataForEntry(entry);
  }

  function pathIsWithin(directory: string, candidate: string): boolean {
    const separator = directory.includes("\\") ? "\\" : "/";
    const base = directory.replace(/[\\/]+$/, "");
    const prefix = base ? `${base}${separator}` : separator;
    const windowsPath = separator === "\\";
    const left = windowsPath ? candidate.toLowerCase() : candidate;
    const normalizedBase = windowsPath ? base.toLowerCase() : base;
    const normalizedPrefix = windowsPath ? prefix.toLowerCase() : prefix;
    return left === normalizedBase || left.startsWith(normalizedPrefix);
  }

  function queuePathKey(path: string): string {
    const separators = path.replaceAll("\\", "/");
    const networkPrefix = separators.startsWith("//") ? "//" : "";
    let normalized = networkPrefix + separators.slice(networkPrefix.length).replace(/\/{2,}/g, "/");
    if (normalized.length > 1 && !/^[a-z]:\/$/i.test(normalized)) normalized = normalized.replace(/\/$/, "");
    return /^(?:[a-z]:\/|\/\/)/i.test(normalized) ? normalized.toLowerCase() : normalized;
  }

  function reviewedSeriesForPath(path: string): QueueSeriesReview | undefined {
    let current = queuePathKey(path);
    while (current) {
      const review = seriesReviewsByRoot.get(current);
      if (review) return review;
      const separator = current.lastIndexOf("/");
      if (separator < 0) break;
      if (separator === 0) current = "/";
      else if (separator === 2 && /^[a-z]:\//i.test(current)) current = current.slice(0, 3);
      else current = current.slice(0, separator);
      if (current === "/" || /^[a-z]:\/$/i.test(current)) {
        const rootReview = seriesReviewsByRoot.get(current);
        if (rootReview) return rootReview;
        break;
      }
    }
    return undefined;
  }

  function queueArtworkRoot(mediaPath: string, artworkPath: string): string | null {
    const sources = workspace.queue.sources.filter((source) => source.scanResult?.entries.some((entry) => queuePathKey(entry.path) === queuePathKey(mediaPath)));
    const candidates = sources.length ? sources : workspace.queue.sources;
    for (const source of candidates) {
      const scan = source.scanResult;
      if (!scan) continue;
      const separator = Math.max(scan.root.lastIndexOf("/"), scan.root.lastIndexOf("\\"));
      const root = scan.input_kind === "file" ? scan.root.slice(0, separator + 1) : scan.root;
      if (root && (pathIsWithin(root, artworkPath) || pathIsWithin(root, mediaPath))) return root;
    }
    return null;
  }

  async function ensureQueueArtworkPreview(mediaPath: string, artworkPath: string) {
    const key = queuePathKey(artworkPath);
    if (!key || queueArtworkPreviews[key] || queueArtworkPreviewBusy[key]) return;
    const root = queueArtworkRoot(mediaPath, artworkPath);
    if (!root) {
      queueArtworkPreviewErrors = { ...queueArtworkPreviewErrors, [key]: "The selected poster is outside the analyzed library." };
      return;
    }
    queueArtworkPreviewBusy = { ...queueArtworkPreviewBusy, [key]: true };
    queueArtworkPreviewErrors = { ...queueArtworkPreviewErrors, [key]: "" };
    try {
      const preview = await invoke<{ dataUrl: string }>("read_local_library_artwork_preview", {
        request: { root, path: artworkPath },
      });
      const next = { ...queueArtworkPreviews, [key]: preview.dataUrl };
      const previewKeys = Object.keys(next);
      while (previewKeys.length > 24) {
        const oldest = previewKeys.shift();
        if (oldest) delete next[oldest];
      }
      queueArtworkPreviews = next;
    } catch (cause) {
      queueArtworkPreviewErrors = { ...queueArtworkPreviewErrors, [key]: String(cause) };
    } finally {
      queueArtworkPreviewBusy = { ...queueArtworkPreviewBusy, [key]: false };
    }
  }

  function observeQueueArtwork(node: HTMLElement, params: { mediaPath: string; artworkPath: string }) {
    let current = params;
    let observer: IntersectionObserver | null = null;
    const observe = () => {
      observer?.disconnect();
      if (!current.artworkPath) return;
      if (typeof IntersectionObserver === "undefined") {
        void ensureQueueArtworkPreview(current.mediaPath, current.artworkPath);
        return;
      }
      observer = new IntersectionObserver((entries) => {
        if (entries.some((entry) => entry.isIntersecting)) {
          observer?.disconnect();
          void ensureQueueArtworkPreview(current.mediaPath, current.artworkPath);
        }
      }, { rootMargin: "160px" });
      observer.observe(node);
    };
    observe();
    return {
      update(next: typeof params) {
        if (next.mediaPath === current.mediaPath && next.artworkPath === current.artworkPath) return;
        current = next;
        observe();
      },
      destroy() {
        observer?.disconnect();
      },
    };
  }

  function remapQueuePath(path: string, movedPaths: Map<string, string>): string {
    const direct = movedPaths.get(path);
    if (direct) return direct;
    const separator = path.includes("\\") ? "\\" : "/";
    const normalizedPath = separator === "\\" ? path.toLowerCase() : path;
    const ancestor = [...movedPaths.entries()]
      .filter(([source]) => {
        const prefix = source.endsWith(separator) ? source : `${source}${separator}`;
        return normalizedPath.startsWith(separator === "\\" ? prefix.toLowerCase() : prefix);
      })
      .sort(([left], [right]) => right.length - left.length)[0];
    return ancestor ? `${ancestor[1]}${path.slice(ancestor[0].length)}` : path;
  }

  function relativeQueuePath(rootPath: string, mediaPath: string, inputKind: ScanResult["input_kind"], previous: string): string {
    if (inputKind === "file") return fileName(mediaPath);
    const separator = rootPath.includes("\\") ? "\\" : "/";
    const prefix = rootPath.endsWith(separator) ? rootPath : `${rootPath}${separator}`;
    const normalizedPath = separator === "\\" ? mediaPath.toLowerCase() : mediaPath;
    const normalizedPrefix = separator === "\\" ? prefix.toLowerCase() : prefix;
    return normalizedPath.startsWith(normalizedPrefix) ? mediaPath.slice(prefix.length) : previous;
  }

  function handleQueuePathsRenamed(paths: Record<string, string>) {
    const movedPaths = new Map(Object.entries(paths));
    if (!movedPaths.size) return;
    const remapRecord = <T,>(record: Record<string, T>): Record<string, T> => {
      const entries = Object.entries(record);
      const updated: Record<string, T> = {};
      for (const [path, value] of entries) {
        if (remapQueuePath(path, movedPaths) === path) updated[path] = value;
      }
      for (const [path, value] of entries) {
        const target = remapQueuePath(path, movedPaths);
        if (target !== path) updated[target] = value;
      }
      return updated;
    };
    workspace.queue.sources = workspace.queue.sources.map((source) => {
      const sourceRoot = remapQueuePath(source.root, movedPaths);
      if (!source.scanResult) return { ...source, root: sourceRoot };
      const scanRoot = remapQueuePath(source.scanResult.root, movedPaths);
      const scanResult = {
        ...source.scanResult,
        root: scanRoot,
        entries: source.scanResult.entries.map((entry) => {
          const path = remapQueuePath(entry.path, movedPaths);
          return path === entry.path ? entry : {
            ...entry,
            path,
            name: fileName(path),
            relative_path: relativeQueuePath(scanRoot, path, source.scanResult!.input_kind, entry.relative_path),
            queue_stale: undefined,
          };
        }),
      };
      return {
        ...source,
        root: sourceRoot,
        scanResult,
        changedFiles: source.changedFiles.filter((path) => remapQueuePath(path, movedPaths) === path),
        missingFiles: source.missingFiles.filter((path) => remapQueuePath(path, movedPaths) === path),
      };
    });
    workspace.queue.selectedPaths = [...new Set(workspace.queue.selectedPaths.map((path) => remapQueuePath(path, movedPaths)))];
    fileTuneOverrides = remapRecord(fileTuneOverrides);
    fileStreamOverrides = remapRecord(fileStreamOverrides);
    queueMovieReviews = Object.fromEntries(Object.entries(remapRecord(queueMovieReviews)).map(([path, review]) => [
      path,
      { ...review, localPosterPath: review.localPosterPath ? remapQueuePath(review.localPosterPath, movedPaths) : "" },
    ]));
    queueSeriesReviews = Object.fromEntries(Object.entries(queueSeriesReviews).map(([seriesRoot, review]) => [
      remapQueuePath(seriesRoot, movedPaths),
      {
        ...review,
        localPosterPath: review.localPosterPath ? remapQueuePath(review.localPosterPath, movedPaths) : "",
        localSeasonPosterPaths: Object.fromEntries(Object.entries(review.localSeasonPosterPaths).map(([season, path]) => [
          season,
          remapQueuePath(path, movedPaths),
        ])),
        episodeAssignments: remapRecord(review.episodeAssignments),
      },
    ]));
    workspace.queue.runStates = Object.fromEntries(Object.entries(workspace.queue.runStates).map(([path, state]) => {
      const target = remapQueuePath(path, movedPaths);
      if (target === path) return [path, state];
      return [target, makeQueueRunState(state.status === "queued" ? "queued" : "needs_review", state.mode ?? mode, {
        message: state.status === "queued" ? state.message : "The media path changed after this run. Review the report and output before retrying.",
        reportPath: state.reportPath,
        reportStatus: state.reportStatus,
      })];
    }));
    queueArtworkPreviews = {};
    queueArtworkPreviewErrors = {};
    queueArtworkPreviewBusy = {};
    workspace.queue.activeRoot = workspace.queue.activeRoot ? remapQueuePath(workspace.queue.activeRoot, movedPaths) : workspace.queue.activeRoot;
    root = root ? remapQueuePath(root, movedPaths) : root;
      metadataFocusPath = metadataFocusPath ? remapQueuePath(metadataFocusPath, movedPaths) : metadataFocusPath;
    queueMatchSuggestions = remapRecord(queueMatchSuggestions);
    if (scanResult) {
      const active = workspace.queue.sources.find((source) => source.root === workspace.queue.activeRoot);
      if (active?.scanResult) scanResult = scanResultWithQueueStatus(active);
    }
    const fileOutputDirs = remapRecord(appPreferences.fileOutputDirs);
    void persistPreferences({ root, fileOutputDirs });
    void persistQueueSources();
  }

  function addMetadataBlacklistPattern() {
    if (!settings) return;
    const pattern = newMetadataBlacklistPattern.trim();
    if (!pattern) {
      metadataBlacklistError = "Enter a pattern first.";
      return;
    }
    if (new TextEncoder().encode(pattern).length > 128) {
      metadataBlacklistError = "Patterns can be up to 128 UTF-8 bytes.";
      return;
    }
    if (/^\*+$/.test(pattern)) {
      metadataBlacklistError = "Add at least one letter or number; a wildcard-only pattern would remove all text.";
      return;
    }
    if (/[\u0000-\u001f\u007f]/.test(pattern)) {
      metadataBlacklistError = "Control characters are not allowed in patterns.";
      return;
    }
    const existing = settings.metadata.text_blacklist ?? [];
    if (existing.some((item) => item.toLocaleLowerCase() === pattern.toLocaleLowerCase())) {
      metadataBlacklistError = "That pattern is already in the list.";
      return;
    }
    if (existing.length >= 100) {
      metadataBlacklistError = "The list is limited to 100 patterns.";
      return;
    }
    settings.metadata.text_blacklist = [...existing, pattern];
    newMetadataBlacklistPattern = "";
    metadataBlacklistError = "";
  }

  function removeMetadataBlacklistPattern(index: number) {
    if (!settings) return;
    settings.metadata.text_blacklist = settings.metadata.text_blacklist.filter((_, itemIndex) => itemIndex !== index);
    metadataBlacklistError = "";
  }

  function clearMetadataBlacklist() {
    if (!settings) return;
    settings.metadata.text_blacklist = [];
    metadataBlacklistError = "";
  }

  function addFilenamePattern(kind: "include" | "exclude") {
    const draft = kind === "include" ? newFilenameIncludePattern : newFilenameExcludePattern;
    const pattern = draft.trim();
    const key = kind === "include" ? "filenameIncludePatterns" : "filenameExcludePatterns";
    const current = appPreferences[key];
    if (!pattern) { filenameFilterError = "Enter a filename pattern first."; return; }
    if (new TextEncoder().encode(pattern).length > 128) { filenameFilterError = "Filename patterns can be up to 128 UTF-8 bytes."; return; }
    if (/[\\/\u0000-\u001f\u007f]/.test(pattern)) { filenameFilterError = "Use a filename pattern without folder separators or control characters."; return; }
    if ([...pattern].every((character) => character === "*" || character === "?")) { filenameFilterError = "Add at least one filename character; wildcard-only filters can hide the entire library."; return; }
    if (current.some((value) => value.toLocaleLowerCase() === pattern.toLocaleLowerCase())) { filenameFilterError = "That pattern is already in this list."; return; }
    if (current.length >= 100) { filenameFilterError = "Each list is limited to 100 patterns."; return; }
    const updated = [...current, pattern];
    void persistPreferences({ [key]: updated } as Partial<AppPreferences>);
    if (kind === "include") newFilenameIncludePattern = "";
    else newFilenameExcludePattern = "";
    filenameFilterError = "";
  }

  function removeFilenamePattern(kind: "include" | "exclude", index: number) {
    const key = kind === "include" ? "filenameIncludePatterns" : "filenameExcludePatterns";
    const updated = appPreferences[key].filter((_, itemIndex) => itemIndex !== index);
    void persistPreferences({ [key]: updated } as Partial<AppPreferences>);
    filenameFilterError = "";
  }

  function clearFilenamePatterns(kind: "include" | "exclude") {
    const key = kind === "include" ? "filenameIncludePatterns" : "filenameExcludePatterns";
    void persistPreferences({ [key]: [] } as Partial<AppPreferences>);
    filenameFilterError = "";
  }

  function profileSummary(entry: MediaEntry) {
    if (!entry.profile) return "Profile unavailable";
    const p = entry.profile;
    const hdr = p.hdr.dolby_vision ? "Dolby Vision" : p.hdr.hdr10plus ? "HDR10+" : p.hdr.hdr10plus_hint ? "HDR10+ candidate" : p.hdr.hdr10 ? "HDR10" : p.hdr.hlg ? "HLG" : "SDR";
    return `${p.video.width} × ${p.video.height} · ${p.video.source_bit_depth}-bit · ${hdr}`;
  }

  function hasHdr10plusSignal(entry: MediaEntry): boolean {
    return Boolean(entry.profile?.hdr.hdr10plus || entry.profile?.hdr.hdr10plus_hint);
  }

  function youtubeHdr10plusConsentMessage(entry: MediaEntry): string {
    if (entry.profile?.hdr.hdr10plus) return "HDR10+ dynamic metadata was detected. The export may lose it; this file stays excluded until you approve.";
    return "HDR10+ is indicated but not confirmed from frame metadata. Review the source details before approving; otherwise this file stays excluded.";
  }

  function clockDuration(seconds: number): string {
    const total = Math.max(0, Math.round(seconds));
    const hours = Math.floor(total / 3600);
    const minutes = Math.floor((total % 3600) / 60);
    const remain = total % 60;
    if (hours) return `${hours}:${String(minutes).padStart(2, "0")}:${String(remain).padStart(2, "0")}`;
    return `${minutes}:${String(remain).padStart(2, "0")}`;
  }

  function mediaInfoLine(entry: MediaEntry): string {
    const profile = entry.profile;
    if (!profile) return bytes(entry.size_bytes);
    const resolution = profile.video.width && profile.video.height ? `${profile.video.width}×${profile.video.height}` : "";
    const hdr = profile.hdr.dolby_vision ? "Dolby Vision" : profile.hdr.hdr10plus ? "HDR10+" : profile.hdr.hdr10 ? "HDR10" : profile.hdr.hlg ? "HLG" : "";
    return [
      profile.video.codec || "Video",
      resolution,
      profile.video.source_bit_depth ? `${profile.video.source_bit_depth}-bit` : "",
      profile.duration_seconds > 0 ? clockDuration(profile.duration_seconds) : "",
      `${profile.audio_count} audio`,
      `${profile.subtitle_count} subtitles`,
      hdr,
      bytes(entry.size_bytes),
    ].filter(Boolean).join(" · ");
  }

  function reviewMediaEntries(target: LibraryTvReviewTarget): MediaEntry[] {
    const group = seriesGroupForRoot(target.seriesRoot);
    const paths = target.level === "episode" && target.episodePath
      ? [target.episodePath]
      : target.level === "season"
        ? group?.seasons.find((item) => item.season === target.season)?.episodePaths ?? []
        : group?.episodePaths ?? [];
    return paths.flatMap((path) => {
      const entry = queuedEntries.find((item) => queuePathKey(item.path) === queuePathKey(path));
      return entry ? [entry] : [];
    });
  }

  function reviewLocationPath(target: LibraryTvReviewTarget | null, fallback: string): string {
    if (!target) return fallback;
    if (target.level === "episode" && target.episodePath) return target.episodePath;
    if (target.level === "season") {
      const directory = seriesGroupForRoot(target.seriesRoot)?.seasons.find((item) => item.season === target.season)?.directory ?? "";
      return directory || target.seriesRoot;
    }
    return target.seriesRoot;
  }

  async function openFileLocation(path: string) {
    if (!path || path.includes("::season:")) {
      setNotice("That folder is not available.", "error");
      return;
    }
    try {
      await invoke("open_file_location", { path });
    } catch (cause) {
      setNotice(String(cause), "error");
    }
  }

  function formatMetadataValue(value: unknown): string {
    if (value === null || value === undefined) return "—";
    if (typeof value === "string") return value;
    if (typeof value === "number" || typeof value === "boolean") return String(value);
    try {
      return JSON.stringify(value, null, 2) ?? String(value);
    } catch {
      return String(value);
    }
  }

  function youtubeEstimateFor(entry: MediaEntry) {
    const estimate = entry.youtube_estimates?.[youtubeResolution]?.[youtubeQualityMode];
    const profile = entry.profile;
    if (!estimate || !profile || youtubeQualityMode !== "balanced" || !Number.isFinite(profile.duration_seconds) || profile.duration_seconds <= 0) return estimate;
    // Keep the interactive forecast aligned with youtube._video_rate_kbps and youtube_output_estimate.
    const sourceAverageKbps = entry.size_bytes * 8 / profile.duration_seconds / 1000;
    const outputAudioKbps = profile.audio_count * 384;
    const videoBitrateKbps = Math.min(
      estimate.recommended_video_bitrate_kbps,
      Math.max(250, Math.round(sourceAverageKbps * youtubeSourceMultiplier - outputAudioKbps)),
    );
    return {
      ...estimate,
      video_bitrate_kbps: videoBitrateKbps,
      estimated_output_bytes: Math.max(0, Math.round(profile.duration_seconds * (videoBitrateKbps + outputAudioKbps) * 1000 / 8 * 1.12)),
    };
  }

  function resourceForecastFor(entry: MediaEntry) {
    return mode === "youtube"
      ? entry.youtube_resource_forecasts?.[youtubeResolution] ?? entry.resource_forecast
      : entry.resource_forecast;
  }

  function setFileTuneOverride(path: string, value: string) {
    if (value === "animation" || value === "none") {
      fileTuneOverrides = { ...fileTuneOverrides, [path]: value };
      return;
    }
    const remaining = { ...fileTuneOverrides };
    delete remaining[path];
    fileTuneOverrides = remaining;
  }

  function metadataSuggestsAnimation(review: QueueMovieReview | QueueSeriesReview | undefined): boolean {
    return /\b(?:animation|anime|animated|cartoon|cartoons)\b/i.test(review?.genresText ?? "");
  }

  function settingsForEntry(entry: MediaEntry): Settings | null | undefined {
    const source = workspace.queue.sources.find((candidate) =>
      candidate.scanResult?.entries.some((item) => queuePathKey(item.path) === queuePathKey(entry.path)),
    );
    if (!source) return settings;
    return source.root === workspace.queue.activeRoot
      ? settings ?? source.scanResult?.settings
      : source.scanResult?.settings ?? settings;
  }

  function effectiveContentTuneForEntry(
    entry: MediaEntry,
    review: QueueMovieReview | QueueSeriesReview | undefined,
    effectiveSettings: Settings | null | undefined,
  ): "animation" | "grain" | "none" {
    if (fileTuneOverrides[entry.path] === "animation" || fileTuneOverrides[entry.path] === "none") {
      return fileTuneOverrides[entry.path];
    }
    if (metadataSuggestsAnimation(review) || entry.profile?.content?.animation_detected) return "animation";
    const globalTune = effectiveSettings?.content.tune ?? "auto";
    if (globalTune === "animation" || globalTune === "grain" || globalTune === "none") return globalTune;
    return "none";
  }

  function fileTuneOverridesForBatch(entries: MediaEntry[], _effectiveSettings: Settings): Record<string, "animation" | "none"> {
    const paths = new Set(entries.map((entry) => entry.path));
    const overrides = Object.fromEntries(
      Object.entries(fileTuneOverrides).filter(([path]) => paths.has(path)),
    ) as Record<string, "animation" | "none">;
    for (const entry of entries) {
      if (overrides[entry.path]) continue;
      const review = queueMovieReviews[entry.path] ?? reviewedSeriesForPath(entry.path);
      if (metadataSuggestsAnimation(review) || entry.profile?.content?.animation_detected) overrides[entry.path] = "animation";
    }
    return overrides;
  }

  function defaultStreamIndices(entry: MediaEntry): number[] {
    const tracks = entry.profile?.tracks ?? [];
    const videos = tracks
      .filter((track) => track.type === "video" && !track.attached_picture && (track.width ?? 0) > 0 && (track.height ?? 0) > 0)
      .sort((a, b) => {
        const pixelDifference = (b.width ?? 0) * (b.height ?? 0) - (a.width ?? 0) * (a.height ?? 0);
        return pixelDifference || (b.width ?? 0) - (a.width ?? 0) || (b.height ?? 0) - (a.height ?? 0) || Number(b.default) - Number(a.default) || Number(b.bit_depth ?? 0) - Number(a.bit_depth ?? 0) || a.index - b.index;
      });
    const selected = settings?.streams.secondary_video === "drop" ? videos.slice(0, 1) : [...videos];
    selected.push(...tracks.filter((track) => ["audio", "subtitle", "attachment"].includes(track.type)));
    if ((settings?.streams.data_streams ?? "preserve") === "preserve") {
      selected.push(...tracks.filter((track) => track.type === "data"));
    }
    return [...new Set(selected.map((track) => track.index))].sort((a, b) => a - b);
  }

  function selectedStreamIndices(entry: MediaEntry): number[] {
    return fileStreamOverrides[entry.path] ?? defaultStreamIndices(entry);
  }

  function streamIsSelectable(type: string): boolean {
    return ["video", "audio", "subtitle", "attachment", "data"].includes(type);
  }

  function streamIsRealVideo(entry: MediaEntry, index: number): boolean {
    const stream = entry.profile?.tracks.find((track) => track.index === index);
    return Boolean(stream && stream.type === "video" && !stream.attached_picture && (stream.width ?? 0) > 0 && (stream.height ?? 0) > 0);
  }

  function setFileStreamSelected(entry: MediaEntry, index: number, included: boolean) {
    const current = selectedStreamIndices(entry);
    if (!included && streamIsRealVideo(entry, index) && current.filter((selected) => streamIsRealVideo(entry, selected)).length <= 1) {
      setNotice("Keep at least one real video track selected for a rebuild.", "error");
      return;
    }
    const next = included
      ? [...new Set([...current, index])].sort((a, b) => a - b)
      : current.filter((selected) => selected !== index);
    const defaults = defaultStreamIndices(entry);
    const overrides = { ...fileStreamOverrides };
    if (next.length === defaults.length && next.every((value, position) => value === defaults[position])) {
      delete overrides[entry.path];
    } else {
      overrides[entry.path] = next;
    }
    fileStreamOverrides = overrides;
  }

  function selectedTrackCount(entry: MediaEntry): number {
    return selectedStreamIndices(entry).length;
  }

  function youtubeHdr10plusLossAccepted(path: string): boolean {
    return youtubeHdr10plusLossPaths.has(path);
  }

  function setYoutubeHdr10plusLossAccepted(path: string, accepted: boolean) {
    const next = new Set(youtubeHdr10plusLossPaths);
    if (accepted && !next.has(path) && next.size >= 512) {
      setNotice("Per-run HDR10+ loss approval is limited to 512 files. Split this library into smaller export batches.", "error");
      return;
    }
    if (accepted) next.add(path);
    else next.delete(path);
    youtubeHdr10plusLossPaths = next;
  }

  function statusLabel(status: string) {
    if (status === "Ready") return "Ready";
    if (status === "Backed up") return "Has backup";
    if (status === "Tools required") return "Tools needed";
    return status;
  }

  async function loadEnvironment(): Promise<boolean> {
    environmentLoading = true;
    environmentError = "";
    try {
      environment = await invoke<Environment>("inspect_environment", { toolPaths: currentToolPaths() });
      return true;
    } catch (error) {
      environmentError = String(error);
      environment = null;
      setNotice(`Could not check the selected executables: ${environmentError}`, "error");
      return false;
    } finally {
      environmentLoading = false;
    }
  }

  function aptPackageForTool(checkKey: string): string | null {
    if (checkKey === "python3") return "python3";
    if (checkKey === "ffmpeg" || checkKey === "ffprobe") return "ffmpeg";
    if (["mkvpropedit", "mkvmerge", "mkvextract"].includes(checkKey)) return "mkvtoolnix";
    if (["mediainfo", "fontconfig", "ca-certificates"].includes(checkKey)) return checkKey;
    return null;
  }

  function packageUpdateForTool(checkKey: string) {
    const packageName = aptPackageForTool(checkKey);
    return packageName ? aptUpdates?.packages.find((item) => item.name === packageName) : undefined;
  }

  function toolCheckReadout(check: CommandCheck | undefined): string {
    if (!check) return "Not checked";
    if (!check.available) return `Missing · ${check.path ?? check.detail}`;
    return `${check.version ?? "Available"}${check.path ? ` · ${check.path}` : ""}`;
  }

  function recordRepairToolOutcomes(before: CommandCheck[] | null, after: CommandCheck[]) {
    if (!before || !repairAffectedToolKeys.length) return;
    const beforeByKey = new Map(before.map((check) => [check.key, check]));
    const afterByKey = new Map(after.map((check) => [check.key, check]));
    const latest = new Map(repairToolOutcomes.map((outcome) => [outcome.key, outcome]));
    for (const key of repairAffectedToolKeys) {
      const oldCheck = beforeByKey.get(key);
      const newCheck = afterByKey.get(key);
      if (!newCheck) continue;
      const status: RepairToolOutcome["status"] = !newCheck.available
        ? "Still missing"
        : !oldCheck?.available
          ? "Installed"
          : oldCheck.version !== newCheck.version || oldCheck.path !== newCheck.path
            ? "Updated or re-resolved"
            : "Ready · unchanged";
      latest.set(key, {
        key,
        label: newCheck.label,
        before: toolCheckReadout(oldCheck),
        after: toolCheckReadout(newCheck),
        status,
      });
    }
    repairToolOutcomes = [...latest.values()];
  }

  function toolKeysForPackages(packages: string[]): string[] {
    const selectedPackages = new Set(packages);
    return [...new Set((environment?.checks ?? [])
      .filter((check) => {
        const packageName = aptPackageForTool(check.key);
        return packageName !== null && selectedPackages.has(packageName);
      })
      .map((check) => check.key))];
  }

  async function refreshTmdbCredentialStatus() {
    if (!desktopShell) {
      tmdbCredentialStatus = { configured: false, secureStoreAvailable: false, message: "Saving API tokens requires the installed desktop app and its operating-system keyring. This browser preview does not store credentials." };
      return;
    }
    try {
      tmdbCredentialStatus = await invoke<TmdbCredentialStatus>("tmdb_credential_status");
    } catch (error) {
      tmdbCredentialStatus = { configured: false, secureStoreAvailable: false, message: String(error) };
    }
  }

  async function refreshTvdbCredentialStatus() {
    if (!desktopShell) {
      tvdbCredentialStatus = {
        configured: false,
        subscriberPinConfigured: false,
        secureStoreAvailable: false,
        message: "Saving provider credentials requires the installed desktop app and its operating-system keyring. This browser preview does not store them.",
      };
      return;
    }
    try {
      tvdbCredentialStatus = await invoke<TvdbCredentialStatus>("tvdb_credential_status");
    } catch (error) {
      tvdbCredentialStatus = { configured: false, subscriberPinConfigured: false, secureStoreAvailable: false, message: String(error) };
    }
  }

  async function refreshArtworkCache() {
    if (!desktopShell || artworkCacheLoading || artworkCachePurging) return;
    artworkCacheLoading = true;
    artworkCacheError = "";
    artworkCacheConfirm = false;
    try {
      artworkCacheInfo = await invoke<ArtworkCacheInfo>("inspect_app_artwork_cache");
    } catch (error) {
      artworkCacheError = String(error);
    } finally {
      artworkCacheLoading = false;
    }
  }

  async function purgeUnusedArtworkCache() {
    if (!desktopShell || !artworkCacheInfo?.purgeableImageCount || !artworkCacheConfirm || artworkCachePurging) return;
    artworkCachePurging = true;
    artworkCacheError = "";
    try {
      const result = await invoke<ArtworkCachePurgeResult>("purge_unused_app_artwork_cache", {
        previewId: artworkCacheInfo.previewId,
        confirmed: true,
      });
      artworkCacheInfo = result.remaining;
      artworkCacheConfirm = false;
      setNotice(result.message, result.failedFileCount ? "error" : "success");
    } catch (error) {
      artworkCacheError = String(error);
      artworkCacheConfirm = false;
    } finally {
      artworkCachePurging = false;
    }
  }

  async function openMetadataProviderSettings(provider?: "tmdb" | "tvdb") {
    openPage("settings");
    await tick();
    const target = provider ? document.getElementById(`${provider}-credential-settings`) : null;
    target?.scrollIntoView({ behavior: "smooth", block: "center" });
    const inputId = provider === "tmdb" ? "tmdb-api-token" : provider === "tvdb" ? "tvdb-project-key" : null;
    if (inputId) document.getElementById(inputId)?.focus({ preventScroll: true });
  }

  function saveMetadataLanguageDraft() {
    const parts = metadataLanguageDraft.trim().match(/^([a-zA-Z]{2})(?:-([a-zA-Z]{2}))?$/);
    if (!parts) {
      metadataLanguageDraft = appPreferences.metadataLanguage;
      setNotice("Use a two-letter language code with an optional two-letter region, such as en-US, fr-FR, or ja-JP.", "error");
      return;
    }
    const normalized = parts[2] ? `${parts[1].toLowerCase()}-${parts[2].toUpperCase()}` : parts[1].toLowerCase();
    metadataLanguageDraft = normalized;
    if (normalized !== appPreferences.metadataLanguage) void persistPreferences({ metadataLanguage: normalized });
  }

  async function saveTmdbCredential() {
    if (!desktopShell || tmdbCredentialBusy) return;
    if (!tmdbTokenDraft.trim()) {
      setNotice("Paste the TMDB API Read Access Token before saving.", "error");
      return;
    }
    tmdbCredentialBusy = true;
    try {
      await invoke("save_tmdb_read_token", { token: tmdbTokenDraft.trim() });
      tmdbTokenVisible = true;
      await refreshTmdbCredentialStatus();
      setNotice("TMDB token saved in the operating-system keyring.", "success");
    } catch (error) {
      setNotice(`Could not save the TMDB token: ${String(error)}`, "error");
    } finally {
      tmdbCredentialBusy = false;
    }
  }

  async function revealTmdbCredential() {
    if (!desktopShell || !tmdbCredentialStatus.configured || tmdbCredentialBusy) return;
    tmdbCredentialBusy = true;
    try {
      tmdbTokenDraft = await invoke<string>("reveal_tmdb_read_token");
      tmdbTokenVisible = true;
    } catch (error) {
      setNotice(`Could not read the saved TMDB token: ${String(error)}`, "error");
    } finally {
      tmdbCredentialBusy = false;
    }
  }

  function toggleTmdbCredentialVisibility() {
    if (!tmdbTokenDraft) {
      void revealTmdbCredential();
      return;
    }
    tmdbTokenVisible = !tmdbTokenVisible;
  }

  async function removeTmdbCredential() {
    if (!desktopShell || tmdbCredentialBusy) return;
    tmdbCredentialBusy = true;
    try {
      await invoke("remove_tmdb_read_token");
      tmdbTokenDraft = "";
      tmdbTokenVisible = true;
      await refreshTmdbCredentialStatus();
      setNotice("Saved TMDB token removed from the operating-system keyring.", "success");
    } catch (error) {
      setNotice(`Could not remove the TMDB token: ${String(error)}`, "error");
    } finally {
      tmdbCredentialBusy = false;
    }
  }

  async function saveTvdbCredential() {
    if (!desktopShell || tvdbCredentialBusy) return;
    if (!tvdbApiKeyDraft.trim()) {
      setNotice("Enter a TheTVDB project API key before saving.", "error");
      return;
    }
    if (!tvdbAccessConfirmed) {
      setNotice("Confirm that this project has valid TheTVDB API access before saving.", "error");
      return;
    }
    tvdbCredentialBusy = true;
    try {
      await invoke("save_tvdb_credentials", {
        apiKey: tvdbApiKeyDraft.trim(),
        subscriberPin: tvdbSubscriberPinDraft.trim() || null,
        accessConfirmed: true,
      });
      tvdbCredentialsVisible = true;
      tvdbAccessConfirmed = false;
      await refreshTvdbCredentialStatus();
      setNotice("TheTVDB credentials were saved in the operating-system keyring. Their validity will be checked on the next search.", "success");
    } catch (error) {
      setNotice(`Could not save TheTVDB credentials: ${String(error)}`, "error");
    } finally {
      tvdbCredentialBusy = false;
    }
  }

  async function revealTvdbCredentials() {
    if (!desktopShell || !tvdbCredentialStatus.configured || tvdbCredentialBusy) return;
    tvdbCredentialBusy = true;
    try {
      const credentials = await invoke<TvdbCredentials>("reveal_tvdb_credentials");
      tvdbApiKeyDraft = credentials.apiKey;
      tvdbSubscriberPinDraft = credentials.subscriberPin ?? "";
      tvdbCredentialsVisible = true;
    } catch (error) {
      setNotice(`Could not read the saved TheTVDB credentials: ${String(error)}`, "error");
    } finally {
      tvdbCredentialBusy = false;
    }
  }

  function toggleTvdbCredentialVisibility() {
    if (!tvdbApiKeyDraft && !tvdbSubscriberPinDraft && tvdbCredentialStatus.configured) {
      void revealTvdbCredentials();
      return;
    }
    tvdbCredentialsVisible = !tvdbCredentialsVisible;
  }

  async function removeTvdbCredential() {
    if (!desktopShell || tvdbCredentialBusy) return;
    tvdbCredentialBusy = true;
    try {
      await invoke("remove_tvdb_credentials");
      tvdbApiKeyDraft = "";
      tvdbSubscriberPinDraft = "";
      tvdbCredentialsVisible = true;
      tvdbAccessConfirmed = false;
      await refreshTvdbCredentialStatus();
      setNotice("TheTVDB credentials were removed from the operating-system keyring.", "success");
    } catch (error) {
      setNotice(`Could not remove TheTVDB credentials: ${String(error)}`, "error");
    } finally {
      tvdbCredentialBusy = false;
    }
  }

  function updateMetadataSources(patch: Partial<MetadataSources>) {
    void persistPreferences({ metadataSources: { ...appPreferences.metadataSources, ...patch } });
  }

  async function loadSettings(folder = root) {
    if (!folder.trim()) return;
    try {
      const payload = await invoke<SettingsLoad>("load_settings", { root: folder.trim() });
      librarySettingsBaseline = JSON.stringify(payload.settings);
      settings = payload.settings;
      settingsBaseline = JSON.stringify(settings);
      settingsRoot = folder.trim();
      settingsSources = payload.sources;
      settingsPath = payload.user_config;
    } catch (error) {
      setNotice(`Could not load the effective settings: ${String(error)}`, "error");
    }
  }

  async function resetSettings() {
    if (!settingsBaseline || workflowBusy) return;
    try {
      if (appPreferences.settings) {
        const folder = root.trim() || settingsRoot;
        if (await persistPreferences({ settings: null }, "Removed the saved application profile.") && folder) await loadSettings(folder);
      } else {
        settings = JSON.parse(settingsBaseline) as Settings;
        setNotice("Restored the effective settings for this library.", "success");
      }
    } catch (error) {
      setNotice(`Could not restore the effective settings: ${String(error)}`, "error");
    }
  }

  async function saveSettings() {
    if (!settings || !settingsChanged || workflowBusy) return;
    const snapshot = JSON.parse(JSON.stringify(settings)) as Settings;
    if (await persistPreferences({ settings: snapshot }, "Saved these rebuild settings as this app's defaults.")) {
      settingsBaseline = JSON.stringify(snapshot);
      librarySettingsBaseline = librarySettingsBaseline || settingsBaseline;
      if (!settingsSources.includes("Saved application profile")) settingsSources = [...settingsSources, "Saved application profile"];
    }
  }

  async function deleteReportHistory() {
    if (!reportDeleteConfirm) {
      reportDeleteConfirm = true;
      return;
    }
    reportDeleteBusy = true;
    try {
      const removed = await invoke<number>("delete_reports");
      reports = [];
      selectedReportPath = "";
      selectedLog = "";
      reportDeleteConfirm = false;
      await loadReports();
      setNotice(
        removed
          ? `Permanently deleted ${removed} report file${removed === 1 ? "" : "s"}.`
          : "No saved reports were found to delete.",
        "success",
      );
    } catch (error) {
      setNotice(`Could not delete the report history: ${String(error)}`, "error");
    } finally {
      reportDeleteBusy = false;
    }
  }

  async function loadReports() {
    try {
      const payload = await invoke<ReportsPayload>("list_reports", { limit: 20 });
      reports = payload.reports;
      if (!reports.some((report) => report.csv_path === selectedReportPath)) {
        selectedReportPath = reports[0]?.csv_path ?? "";
      }
    } catch (error) {
      setNotice(`Could not read report history: ${String(error)}`, "error");
    }
  }

  async function recoverInterruptedSession() {
    try {
      const recovery = await invoke<InterruptedRecovery>("reconcile_interrupted_runs");
      if (recovery.skipped) {
        workspace.queue.runStates = recovery.runStates;
        return;
      }
      workspace.queue.runStates = recovery.runStates;
      if (recovery.summary) setNotice(recovery.summary, recovery.held > 0 ? "info" : "success");
    } catch (error) {
      setNotice(`Could not recover the interrupted session: ${String(error)}`, "error");
    }
  }

  async function loadMediaQueue() {
    try {
      const payload = await invoke<MediaQueueLoadResult>("load_library_queue");
      workspace.queue.sources = payload.queue.sources ?? [];
      workspace.queue.selectedPaths = payload.queue.selectedMediaPaths ?? [];
      fileTuneOverrides = payload.queue.fileTuneOverrides ?? {};
      fileStreamOverrides = payload.queue.fileStreamOverrides ?? {};
      queueMovieReviews = payload.queue.movieReviews ?? {};
      queueSeriesReviews = Object.fromEntries(Object.entries(payload.queue.seriesReviews ?? {}).map(([root, review]) => [
        root,
        { ...review, seasonRecords: review.seasonRecords ?? {}, localSeasonPosterPaths: review.localSeasonPosterPaths ?? {}, episodeAssignments: review.episodeAssignments ?? {} },
      ]));
      queueMatchSuggestions = payload.queue.matchSuggestions ?? {};
      workspace.queue.runStates = payload.queue.runStates ?? {};
      workspace.queue.file = payload.queueFile;
      workspace.queue.persistenceWarning = payload.warning ?? "";
      const active = workspace.queue.sources.find((source) => source.root === payload.queue.activeRoot && source.scanResult)
        ?? workspace.queue.sources.find((source) => source.scanResult);
      workspace.queue.activeRoot = active?.root ?? null;
      if (active) {
        root = active.root;
        scanResult = scanResultWithQueueStatus(active);
        if (scanResult) {
          environment = scanResult.environment;
          settings = scanResult.settings;
          settingsSources = scanResult.settings_sources;
          settingsBaseline = JSON.stringify(scanResult.settings);
          librarySettingsBaseline = settingsBaseline;
          settingsRoot = scanResult.root;
          recursive = scanResult.recursive;
          minimumSizeMib = String(scanResult.minimum_size_mib);
        }
      }
      workspace.queue.loaded = true;
      metadataLookupPending = active ? sourceNeedsMetadataLookup(active) : false;
      if (metadataLookupPending) metadataLookupClaimed = false;
      if (Object.values(workspace.queue.runStates).some((state) => state.status === "interrupted")) {
        await recoverInterruptedSession();
      }
      if (workspace.queue.persistenceWarning) setNotice(workspace.queue.persistenceWarning, "error");
    } catch (error) {
      workspace.queue.loaded = true;
      workspace.queue.persistenceError = String(error);
      setNotice(`Could not restore the media queue: ${String(error)}`, "error");
    }
  }

  async function initialize() {
    try {
      const stored = await invoke<PreferencesPayload>("load_app_preferences");
      preferenceStorage = stored;
      appPreferences = stored.preferences;
      await restoreWindowState(appPreferences.windowState);
      metadataLanguageDraft = appPreferences.metadataLanguage;
      const migration: Partial<AppPreferences> = {};
      if (appPreferences.organizerTemplates.series === "{series_title} [[({year_range})]]") {
        migration.organizerTemplates = { ...appPreferences.organizerTemplates, series: defaultOrganizerTemplates.series };
      }
      if (!stored.hasSavedSettings) {
        try {
          const oldTheme = localStorage.getItem("hevc-rebuild-theme");
          const oldRoot = localStorage.getItem("hevc-rebuild.root");
          const oldFfmpeg = localStorage.getItem("hevc-rebuild.ffmpeg");
          const oldRecursive = localStorage.getItem("hevc-rebuild.recursive");
          const oldMinimum = localStorage.getItem("hevc-rebuild.minimumSizeMib");
          if (oldTheme) migration.theme = normalizeTheme(oldTheme);
          if (oldRoot) migration.root = oldRoot;
          if (oldFfmpeg) migration.toolPaths = normalizeToolPaths(appPreferences.toolPaths, oldFfmpeg);
          if (oldRecursive !== null) migration.recursive = oldRecursive !== "false";
          if (oldMinimum !== null) migration.minimumSizeMib = oldMinimum;
        } catch { /* Native preferences remain available if legacy browser storage is disabled. */ }
      }
      appPreferences = { ...appPreferences, ...migration };
      publishTheme(normalizeTheme(appPreferences.theme));
      appPreferences.theme = theme;
      applyToolPaths(appPreferences.toolPaths);
      minimumSizeMib = appPreferences.minimumSizeMib;
      recursive = appPreferences.recursive;
      stopOnError = appPreferences.stopOnError;
      youtubeResolution = appPreferences.youtubeResolution;
      youtubeQualityMode = appPreferences.youtubeQualityMode;
      youtubeSourceMultiplier = appPreferences.youtubeSourceMultiplier;
      youtubeOutputDir = appPreferences.youtubeOutputDir ?? "";
      root = appPreferences.root ?? await invoke<string>("default_media_root");
      await loadMediaQueue();
      const queueHasMedia = workspace.queue.sources.some((source) => (source.scanResult?.entries.length ?? 0) > 0);
      page = queueHasMedia ? normalizePage(appPreferences.lastPage) : "home";
      if (workspace.queue.activeRoot) migration.root = workspace.queue.activeRoot;
      if (!stored.hasSavedSettings || migration.root === undefined) {
        const saved = await persistPreferences({ ...migration, root });
        if (saved && !stored.hasSavedSettings) {
          try {
            ["hevc-rebuild-theme", "hevc-rebuild.root", "hevc-rebuild.ffmpeg", "hevc-rebuild.recursive", "hevc-rebuild.minimumSizeMib"].forEach((key) => localStorage.removeItem(key));
          } catch { /* The native preference file is already authoritative. */ }
        }
      } else if (Object.keys(migration).length) {
        await persistPreferences(migration);
      }
      if (stored.storageWarning) setNotice(stored.storageWarning, "error");
      await Promise.all([loadEnvironment(), loadSettings(root), loadReports(), refreshToolSetupLogs(), refreshTmdbCredentialStatus(), refreshTvdbCredentialStatus()]);
      if (environment?.os.supported) await checkUbuntuPackageUpdates(true);
    } catch (error) {
      environmentError = String(error);
      setNotice(`The desktop backend did not start: ${String(error)}`, "error");
    } finally {
      booting = false;
    }
  }

  async function browseFolder() {
    if (!desktopShell || workspace.run.id || workspace.run.starting || repairId || hdrInstalling || scanning) return;
    let defaultPath = root.trim() || workspace.queue.activeRoot || appPreferences.root || undefined;
    if (defaultPath && /\.(mkv|mp4|m4v|mov|avi|ts|m2ts|mts|webm)$/i.test(defaultPath)) {
      const separator = Math.max(defaultPath.lastIndexOf("/"), defaultPath.lastIndexOf("\\"));
      defaultPath = separator > 0 ? defaultPath.slice(0, separator) : undefined;
    }
    if (!defaultPath) {
      try { defaultPath = await invoke<string>("default_media_root"); } catch { /* The native dialog can use its platform default. */ }
    }
    const picked = await openDialog({ directory: true, multiple: true, title: "Add media folder(s) to the queue", defaultPath });
    const paths = typeof picked === "string" ? [picked] : Array.isArray(picked) ? picked : [];
    if (paths.length) await addQueueSources(paths);
  }

  async function browseFile() {
    if (!desktopShell || workspace.run.id || workspace.run.starting || repairId || hdrInstalling || scanning) return;
    const picked = await openDialog({
      directory: false,
      multiple: true,
      title: "Add media file(s) to the queue",
      filters: [{ name: "Video files", extensions: ["mkv", "mp4", "m4v", "mov", "avi", "ts", "m2ts", "mts", "webm"] }],
    });
    const paths = typeof picked === "string" ? [picked] : Array.isArray(picked) ? picked : [];
    if (paths.length) await addQueueSources(paths);
  }

  async function chooseYoutubeOutput() {
    if (!desktopShell || workspace.run.id || workspace.run.starting || repairId || hdrInstalling || scanning) return;
    const picked = await openDialog({ directory: true, multiple: false, title: "Choose the YouTube export folder" });
    if (typeof picked !== "string") return;
    youtubeOutputDir = picked;
    await persistPreferences({ youtubeOutputDir });
    setNotice("YouTube exports will be saved to the selected folder.", "success");
  }

  async function chooseRebuildOutput() {
    if (!desktopShell || workflowBusy) return;
    const picked = await openDialog({ directory: true, multiple: false, title: "Choose the default rebuild output folder" });
    if (typeof picked !== "string") return;
    await persistPreferences({ rebuildOutputDir: picked }, "Default rebuild outputs will be saved to this folder. Original files will stay where they are.");
  }

  async function chooseFileOutput(path: string) {
    if (!desktopShell || workflowBusy) return;
    const picked = await openDialog({ directory: true, multiple: false, title: `Choose output folder for ${fileName(path)}` });
    if (typeof picked !== "string") return;
    const fileOutputDirs = { ...appPreferences.fileOutputDirs, [path]: picked };
    if (Object.keys(fileOutputDirs).length > 512) {
      setNotice("Per-file output folders are limited to 512 media files.", "error");
      return;
    }
    await persistPreferences({ fileOutputDirs });
  }

  function clearFileOutput(path: string) {
    const fileOutputDirs = { ...appPreferences.fileOutputDirs };
    delete fileOutputDirs[path];
    void persistPreferences({ fileOutputDirs });
  }

  async function saveToolPaths(recheck = true) {
    if (workflowBusy) return;
    const paths = currentToolPaths();
    if (!(await persistPreferences({ toolPaths: paths }))) return;
    if (recheck && !(await loadEnvironment())) return;
    setNotice("Saved executable paths and checked the selected tools.", "success");
  }

  async function chooseToolPath(key: ToolPathKey, label: string) {
    if (!desktopShell || workspace.run.id || workspace.run.starting || repairId || hdrInstalling) return;
    const picked = await openDialog({ directory: false, multiple: false, title: `Choose ${label} executable` });
    if (typeof picked !== "string") return;
    setToolPathValue(key, picked);
    await saveToolPaths();
  }

  async function clearToolPath(key: ToolPathKey) {
    if (workflowBusy) return;
    setToolPathValue(key, "");
    await saveToolPaths();
  }

  async function chooseFfmpeg() {
    await chooseToolPath("ffmpeg", "FFmpeg");
  }

  async function clearFfmpegOverride() {
    await clearToolPath("ffmpeg");
  }

  async function analyzeFolder() {
    if (!root.trim()) {
      setNotice("Choose a media file or folder to add to the queue.", "error");
      return;
    }
    await scanQueueSource(root.trim(), true, true);
  }

  async function openToolchainSettings() {
    openPage("settings");
    await tick();
    document.getElementById("environment-capabilities")?.scrollIntoView({ behavior: "smooth", block: "start" });
  }

  function refreshQueueSource(sourceRoot: string) {
    void scanQueueSource(sourceRoot, true, true);
  }

  async function beginRun() {
    if (!canStart) {
      setNotice(`Analyze the selected media and resolve the tools required for ${mode === "youtube" ? "YouTube export" : "this rebuild task"} before starting.`, "error");
      return;
    }
    if (mode === "full") {
      fullAck = false;
      fullConfirmOpen = true;
      return;
    }
    await launchRun(false);
  }

  function selectedPosterOverrides(entries: MediaEntry[]): Record<string, string> {
    const overrides: Record<string, string> = {};
    for (const entry of entries) {
      const movieReview = queueMovieReviews[entry.path];
      if (movieReview?.localPosterPath && !movieReview.localPosterPath.startsWith("https://")) {
        overrides[entry.path] = movieReview.localPosterPath;
        continue;
      }
      const seriesReview = reviewedSeriesForPath(entry.path);
      if (!seriesReview) continue;
      const episodeAssignment = seriesReview.episodeAssignments[entry.path] ??
        Object.entries(seriesReview.episodeAssignments).find(([path]) => queuePathKey(path) === queuePathKey(entry.path))?.[1];
      const parentName = entry.path.split(/[\\/]/).at(-2) ?? "";
      const folderSeason = parentName.match(/^season[\s._-]*(\d+)$/i)?.[1];
      const season = episodeAssignment?.season ?? (folderSeason ? Number(folderSeason) : null);
      const seasonPoster = season === null ? "" : seriesReview.localSeasonPosterPaths[String(season)] ?? "";
      const seasonDirectory = entry.path.replace(/[/\\][^/\\]+$/, "");
      const seasonOnDisk = libraryLocalPosters[queuePathKey(seasonDirectory)] ?? "";
      const seasonFile = [seasonPoster, seasonOnDisk].find((path) => path && !path.startsWith("https://")) ?? "";
      if (seasonFile) {
        overrides[entry.path] = seasonFile;
        continue;
      }
      // A provider URL means this season already has its own art. Do not
      // substitute the series poster; the engine uses Season NN/poster.* on disk.
      if (seasonPoster.startsWith("https://")) continue;
      const seriesPoster = seriesReview.localPosterPath ?? "";
      if (seriesPoster && !seriesPoster.startsWith("https://")) overrides[entry.path] = seriesPoster;
    }
    return overrides;
  }

  function selectedMetadataOverrides(entries: MediaEntry[]): Record<string, Record<string, string | string[]>> {
    const overrides: Record<string, Record<string, string | string[]>> = {};
    const addText = (target: Record<string, string | string[]>, key: string, value: string | null | undefined) => {
      const normalized = value?.trim();
      if (normalized) target[key] = normalized;
    };
    const addGenres = (target: Record<string, string | string[]>, value: string) => {
      const genres = value.split(/[;,]/).map((genre) => genre.trim()).filter(Boolean).slice(0, 50);
      if (genres.length) target.genres = genres;
    };

    for (const entry of entries) {
      const movie = queueMovieReviews[entry.path];
      if (movie) {
        const fields: Record<string, string | string[]> = {};
        addText(fields, "title", movie.title);
        addText(fields, "year", movie.year);
        addText(fields, "certification", movie.certification);
        addText(fields, "short_description", movie.shortDescription);
        addText(fields, "plot", movie.plot);
        addGenres(fields, movie.genresText);
        addText(fields, "rating", movie.ratingText);
        addText(fields, "source_url", movie.sourceUrl);
        const movieComment = movie.comment?.trim() ?? "";
        if (movieComment && movieComment !== settings?.metadata.comment?.trim()) addText(fields, "comment", movieComment);
        if (Object.keys(fields).length) overrides[entry.path] = fields;
        continue;
      }

      const series = reviewedSeriesForPath(entry.path);
      if (!series) continue;
      const assignment = series.episodeAssignments[entry.path] ??
        Object.entries(series.episodeAssignments).find(([path]) => queuePathKey(path) === queuePathKey(entry.path))?.[1];
      const fields: Record<string, string | string[]> = {};
      addText(fields, "title", assignment?.title ?? entry.name.replace(/\.[^.]+$/, ""));
      addText(fields, "show_title", series.title);
      addText(fields, "year", series.year);
      const seasonCertificate = assignment ? series.seasonRecords?.[String(assignment.season)]?.certification.trim() ?? "" : "";
      addText(fields, "certification", assignment?.certification?.trim() || seasonCertificate || series.certification);
      const episodeText = episodeOutputText(assignment?.summary, assignment?.sourceUrl);
      addText(fields, "short_description", episodeText.summary);
      addText(fields, "plot", episodeText.synopsis);
      addGenres(fields, series.genresText);
      addText(fields, "source_url", episodeText.sourceUrl);
      if (assignment) {
        fields.season = String(assignment.season);
        fields.episode = String(assignment.episode);
        addText(fields, "airdate", assignment.airdate);
      }
      const individualComment = resolvedFileComment(series, assignment);
      if (individualComment && individualComment !== settings?.metadata.comment?.trim()) addText(fields, "comment", individualComment);
      if (Object.keys(fields).length) overrides[entry.path] = fields;
    }
    return overrides;
  }

  function outputPreviewRequest() {
    const selected = new Set(runBatches.flatMap((batch) => batch.entries.map((entry) => entry.path)));
    return {
      mode: mode as "metadata" | "test" | "full",
      sources: runBatches.map((batch) => ({
        root: batch.source.root,
        files: batch.entries.map((entry) => entry.path),
      })),
      outputDir: appPreferences.rebuildOutputDir,
      fileOutputDirs: Object.fromEntries(
        Object.entries(appPreferences.fileOutputDirs).filter(([path]) => selected.has(path)),
      ),
    };
  }

  async function launchRun(fullRebuildConfirmed: boolean) {
    if (!settings) return;
    if (fullRebuildConfirmed) fullConfirmOpen = false;
    if (unresolvedSelectedPaths > 0) {
      setNotice("Some selected files no longer belong to a completed scan. Refresh or remove those source folders before starting.", "error");
      return;
    }
    if (!runBatches.length) {
      setNotice("Add and scan at least one source before starting a workflow.", "error");
      return;
    }
    if (["metadata", "test", "full"].includes(mode)) {
      const pendingRecovery = runBatches.flatMap((batch) => batch.entries).flatMap((entry) => {
        const state = queueRunStateForPath(entry.path);
        if (!state || !["interrupted", "needs_review"].includes(state.status) || recoveryReviewedPaths.has(queuePathKey(entry.path))) return [];
        const report = state.reportPath
          ? reports.find((candidate) => queuePathKey(candidate.csv_path) === queuePathKey(state.reportPath!))?.rows.find((row) => queuePathKey(row.input) === queuePathKey(entry.path)) ?? null
          : null;
        const recordedOutput = state.outputPath || report?.output || "";
        return [{ entry, state, report, targetPath: recordedOutput || entry.path, hasRecordedOutput: Boolean(recordedOutput) }];
      });
      if (pendingRecovery.length) {
        await loadReports();
        const refreshed = pendingRecovery.map((item) => {
          const report = item.state.reportPath
            ? reports.find((candidate) => queuePathKey(candidate.csv_path) === queuePathKey(item.state.reportPath!))?.rows.find((row) => queuePathKey(row.input) === queuePathKey(item.entry.path)) ?? null
            : null;
          const recordedOutput = item.state.outputPath || report?.output || "";
          return { ...item, report, targetPath: recordedOutput || item.entry.path, hasRecordedOutput: Boolean(recordedOutput) };
        });
        recoveryReviewItems = refreshed;
        recoveryReviewIndex = 0;
        recoveryProbeEntry = null;
        recoveryProbeError = "";
        recoveryProbeAttempted = false;
        recoveryAcknowledged = false;
        pendingRecoveryFullConfirmed = fullRebuildConfirmed;
        recoveryReviewOpen = true;
        return;
      }
    }
    if (mode === "metadata" || mode === "test" || mode === "full") {
      outputPreview = null;
      outputPreviewOpen = false;
      outputPreviewBusy = true;
      try {
        outputPreview = await invoke<OutputPreview>("preview_queue_output_targets", {
          request: outputPreviewRequest(),
        });
        outputPreviewOpen = true;
        if (!outputPreview.eligible) {
          setNotice(`${outputPreview.collisionCount} output collision${outputPreview.collisionCount === 1 ? "" : "s"} must be resolved before this queue can start.`, "error");
        }
      } catch (error) {
        setNotice(`Could not check the queue's output paths: ${String(error)}`, "error");
      } finally {
        outputPreviewBusy = false;
      }
      if (!outputPreview) return;
      pendingRunFullConfirmed = fullRebuildConfirmed;
      return;
    }
    await beginQueuedRun(fullRebuildConfirmed);
  }

  async function confirmOutputPreview() {
    if (!outputPreview || outputPreviewBusy || !outputPreview.eligible || !canStart) return;
    outputPreviewBusy = true;
    try {
      const reviewedMode = outputPreview.mode;
      const refreshed = await invoke<OutputPreview>("preview_queue_output_targets", {
        request: outputPreviewRequest(),
      });
      outputPreview = refreshed;
      if (mode !== reviewedMode || refreshed.mode !== reviewedMode) {
        setNotice("The selected workflow changed during output review. Review its destinations again before starting.", "error");
        return;
      }
      if (!refreshed.eligible) {
        setNotice(`${refreshed.collisionCount} output collision${refreshed.collisionCount === 1 ? "" : "s"} appeared during review. Resolve them before starting.`, "error");
        return;
      }
      outputPreviewOpen = false;
      await beginQueuedRun(pendingRunFullConfirmed);
    } catch (error) {
      setNotice(`Could not recheck the queue's output paths: ${String(error)}`, "error");
    } finally {
      outputPreviewBusy = false;
    }
  }

  async function beginQueuedRun(fullRebuildConfirmed: boolean) {
    if (!settings) return;
    workspace.run.mode = mode;
    fullConfirmOpen = false;
    pendingRunBatches = runBatches.map((batch) => ({ source: batch.source, entries: [...batch.entries] }));
    activeRunBatchIndex = -1;
    pendingRunFullConfirmed = fullRebuildConfirmed;
    workspace.run.stopAfterCurrent = false;
    const queuedStates = Object.fromEntries(pendingRunBatches.flatMap((batch) => batch.entries.map((entry) => [
      entry.path,
      makeQueueRunState("queued", workspace.run.mode),
    ])));
    recoveryReviewedPaths = new Set();
    if (!await persistQueueRunStates(queuedStates)) {
      pendingRunBatches = [];
      setNotice(`The queue could not save its run plan, so the workflow did not start: ${workspace.queue.persistenceError}`, "error");
      return;
    }
    await startNextRunBatch();
  }

  async function inspectRecoveryTarget() {
    if (!desktopShell || recoveryProbeBusy || !recoveryCurrentItem) return;
    recoveryProbeBusy = true;
    recoveryProbeAttempted = true;
    recoveryProbeEntry = null;
    recoveryProbeError = "";
    try {
      const result = await invoke<ScanResult>("scan_folder", {
        root: recoveryCurrentItem.targetPath,
        recursive: false,
        minimumSizeMib: 0,
        filenameIncludePatterns: [],
        filenameExcludePatterns: [],
        toolPaths: currentToolPaths(),
      });
      recoveryProbeEntry = result.entries.find((entry) => queuePathKey(entry.path) === queuePathKey(recoveryCurrentItem!.targetPath))
        ?? result.entries[0]
        ?? null;
      if (!recoveryProbeEntry) recoveryProbeError = "No readable media file was found at this path.";
    } catch (error) {
      recoveryProbeError = String(error);
    } finally {
      recoveryProbeBusy = false;
    }
  }

  async function finishRecoveryReview(markCompleted = false) {
    const item = recoveryCurrentItem;
    if (!item || recoveryProbeBusy || !recoveryProbeAttempted || !recoveryAcknowledged) return;
    const reportStatus = (item.report?.status || item.state.reportStatus || "").trim().toUpperCase();
    const reportIsSuccessful = ["OK", "METADATA_OK", "TEST_OK"].includes(reportStatus);
    if (markCompleted) {
      if (!item.hasRecordedOutput || !reportIsSuccessful || recoveryProbeEntry?.status !== "Ready") return;
      const completed: MediaQueueRunState = {
        status: "completed",
        mode: item.state.mode,
        message: "Recorded as complete after reviewing the successful report and readable output.",
        outputPath: item.state.outputPath || item.report?.output || item.entry.path,
        reportPath: item.state.reportPath,
        reportStatus,
        updatedAtMs: Date.now(),
      };
      if (!await persistQueueRunStates({ [item.entry.path]: completed })) {
        setNotice(`Could not save the reviewed run result. ${workspace.queue.persistenceError}`, "error");
        return;
      }
    } else {
      recoveryReviewedPaths = new Set([...recoveryReviewedPaths, queuePathKey(item.entry.path)]);
    }
    if (recoveryReviewIndex + 1 < recoveryReviewItems.length) {
      recoveryReviewIndex += 1;
      recoveryProbeEntry = null;
      recoveryProbeError = "";
      recoveryProbeAttempted = false;
      recoveryAcknowledged = false;
      return;
    }
    recoveryReviewOpen = false;
    recoveryReviewItems = [];
    recoveryProbeEntry = null;
    recoveryProbeError = "";
    recoveryAcknowledged = false;
    setNotice(markCompleted ? "The reviewed successful output was recorded as complete." : "Recovery review is recorded for this session. Output paths will be checked again before the run starts.", "info");
    await launchRun(pendingRecoveryFullConfirmed);
  }

  async function startNextRunBatch() {
    const runMode = workspace.run.mode;
    const nextIndex = activeRunBatchIndex + 1;
    const batch = pendingRunBatches[nextIndex];
    if (!batch) return;
    const batchSettings = batch.source.root === workspace.queue.activeRoot
      ? settings
      : batch.source.scanResult?.settings ?? settings;
    if (!batchSettings) {
      setNotice(`Settings are unavailable for ${batch.source.root}. Rescan that source before running it.`, "error");
      pendingRunBatches = [];
      activeRunBatchIndex = -1;
      return;
    }
    const activeBatchStates = Object.fromEntries(batch.entries.map((entry) => [
      entry.path,
      makeQueueRunState("running", runMode, { message: "This file is included in the active source batch." }),
    ]));
    if (!await persistQueueRunStates(activeBatchStates)) {
      pendingRunBatches = [];
      activeRunBatchIndex = -1;
      setNotice(`The queue could not record the active batch, so it was not started: ${workspace.queue.persistenceError}`, "error");
      return;
    }
    activeRunBatchIndex = nextIndex;
    runEventBuffer.reset();
    workspace.run.lines = [];
    workspace.run.progress = null;
    workspace.run.startedAt = Date.now();
    workspace.run.progressSeenAt = null;
    workspace.run.resources = null;
    workspace.run.starting = true;
    workspace.run.message = `Starting source ${nextIndex + 1} of ${pendingRunBatches.length}…`;
    try {
      const runId = await invoke<string>("start_run", {
        request: {
          root: batch.source.root,
          recursive: batch.source.scanResult?.recursive ?? recursive,
          minimumSizeMib: batch.source.scanResult?.minimum_size_mib ?? Math.max(0, Math.min(4096, Number(minimumSizeMib) || 0)),
          toolPaths: currentToolPaths(),
          mode: runMode,
          youtubeResolution,
          youtubeQualityMode,
          youtubeSourceMultiplier,
          youtubeOutputDir: youtubeOutputDir.trim() || null,
          rebuildOutputDir: runMode === "metadata" || runMode === "full" || runMode === "test" ? appPreferences.rebuildOutputDir : null,
          fileOutputDirs: runMode === "metadata" || runMode === "full" || runMode === "test" ? Object.fromEntries(Object.entries(appPreferences.fileOutputDirs).filter(([path]) => batch.entries.some((entry) => entry.path === path))) : {},
          filenameIncludePatterns: appPreferences.filenameIncludePatterns,
          filenameExcludePatterns: appPreferences.filenameExcludePatterns,
          selectedFiles: batch.entries.map((entry) => entry.path),
          settings: batchSettings,
          stopOnError,
          fullRebuildConfirmed: pendingRunFullConfirmed,
          fileTuneOverrides: runMode === "youtube" || runMode === "metadata" ? {} : fileTuneOverridesForBatch(batch.entries, batchSettings),
          fileStreamOverrides: runMode === "youtube" || runMode === "metadata" ? {} : Object.fromEntries(Object.entries(fileStreamOverrides).filter(([path]) => batch.entries.some((entry) => entry.path === path))),
          filePosterOverrides: runMode === "metadata" || runMode === "full" || runMode === "test" ? selectedPosterOverrides(batch.entries) : {},
          fileMetadataOverrides: runMode === "metadata" || runMode === "full" || runMode === "test" ? selectedMetadataOverrides(batch.entries) : {},
          allowYoutubeHdr10plusLossPaths: runMode === "youtube" ? [...youtubeHdr10plusLossPaths].filter((path) => batch.entries.some((entry) => entry.path === path)) : [],
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
      const batchLabel = `${nextIndex + 1}/${pendingRunBatches.length} · ${fileName(batch.source.root)} · ${batch.entries.length} file${batch.entries.length === 1 ? "" : "s"}`;
      workspace.run.message = runMode === "full" ? `Full rebuild ${batchLabel}. Originals stay protected until each rebuilt file validates.` : runMode === "test" ? `Short tests ${batchLabel}. Originals remain untouched.` : runMode === "metadata" ? `Metadata-only copies ${batchLabel}. All tracks are preserved; no video is encoded and sources stay in place.` : runMode === "youtube" ? `Separate YouTube copies ${batchLabel}. Originals stay in place.` : `Check-only pass ${batchLabel}. No media will be encoded.`;
      openPage(runMode === "youtube" ? "youtube" : "run");
      setNotice("Run started. Live progress and diagnostics are shown below.", "success");
      const earlyFinish = earlyEvents.finished;
      if (earlyFinish?.runId === runId) await finishRun(earlyFinish);
    } catch (error) {
      workspace.run.id = "";
      workspace.run.starting = false;
      workspace.run.message = "Could not start the selected workflow.";
      await persistQueueRunStates(Object.fromEntries(batch.entries.map((entry) => [
        entry.path,
        makeQueueRunState("failed", runMode, { message: String(error) }),
      ])));
      pendingRunBatches = [];
      activeRunBatchIndex = -1;
      setNotice(String(error), "error");
    }
  }

  async function finishRun(payload: RunFinishedEvent) {
    workspace.run.id = "";
    if (payload.error) workspace.run.message = payload.error;
    else if (payload.stopRequested) workspace.run.message = "Queue stopped safely after the active file.";
    else if (payload.exitCode === 0) workspace.run.message = "Workflow finished. Review the report and any skipped files.";
    else workspace.run.message = `Workflow finished with exit code ${payload.exitCode ?? "unknown"}. Review the diagnostics.`;
    await loadReports();
    const finishedBatch = pendingRunBatches[activeRunBatchIndex];
    const matchingReport = payload.reportPath
      ? reports.find((report) => queuePathKey(report.csv_path) === queuePathKey(payload.reportPath!))
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
    if (Object.keys(completedStates).length && !await persistQueueRunStates(completedStates)) {
      pendingRunBatches = [];
      activeRunBatchIndex = -1;
      workspace.run.stopAfterCurrent = false;
      workspace.run.message = "The report was written, but queue state could not be saved. Later source batches were not started.";
      setNotice(`${workspace.run.message} ${workspace.queue.persistenceError}`, "error");
      return;
    }
    const hasFailure = payload.exitCode !== 0 || Boolean(payload.error);
    const isLastBatch = activeRunBatchIndex >= pendingRunBatches.length - 1;
    const shouldContinue = !isLastBatch && !payload.stopRequested && !workspace.run.stopAfterCurrent && !(stopOnError && hasFailure);
    if (shouldContinue) {
      workspace.run.message = `Finished source ${activeRunBatchIndex + 1} of ${pendingRunBatches.length}. Starting the next source…`;
      await startNextRunBatch();
      return;
    }
    if (payload.logPath) void openLog(payload.logPath);
    const remainingBatches = Math.max(0, pendingRunBatches.length - activeRunBatchIndex - 1);
    if (remainingBatches > 0 && (payload.stopRequested || workspace.run.stopAfterCurrent || (stopOnError && hasFailure))) {
      workspace.run.message = payload.stopRequested || workspace.run.stopAfterCurrent
        ? `Stopped safely. ${remainingBatches} source batch${remainingBatches === 1 ? "" : "es"} remain queued.`
        : `Stopped after an error. ${remainingBatches} source batch${remainingBatches === 1 ? "" : "es"} remain queued.`;
    }
    pendingRunBatches = [];
    activeRunBatchIndex = -1;
    workspace.run.stopAfterCurrent = false;
    setNotice(workspace.run.message, payload.exitCode === 0 ? "success" : "error");
  }

  async function stopAfterCurrent() {
    if (!workspace.run.id) return;
    try {
      workspace.run.stopAfterCurrent = true;
      await invoke("request_stop", { runId: workspace.run.id });
      workspace.run.message = "Safe stop requested. The current file will finish before the queue stops.";
      setNotice("Safe stop requested; the active file will finish first.", "success");
    } catch (error) {
      workspace.run.stopAfterCurrent = false;
      setNotice(`Could not request a safe stop: ${String(error)}`, "error");
    }
  }

  async function repairApt() {
    if (workspace.run.id || workspace.run.starting || repairId || hdrInstalling || scanning) return;
    if (!environment?.os.supported) {
      setNotice("Automatic APT repair is available only on supported Ubuntu and Linux Mint releases.", "error");
      return;
    }
    if (!aptUpdates) {
      const status = await checkUbuntuPackageUpdates(true);
      if (!status) return;
    }
    if (aptUpdates?.catalog_stale) {
      continueToolchainSetupAfterRefresh = true;
      setNotice("The Ubuntu package catalog is stale. Refreshing it first; the package list will be shown for review before anything is installed.", "info");
      await refreshUbuntuPackageCatalog(true);
      return;
    }
    const missing = environment.missing_apt_packages;
    const helperMissing = environment.checks.some((check) => check.key === "hdr10plus_tool" && !check.available);
    if (!missing.length && !aptUpdates?.packages.length) {
      if (helperMissing) {
        await installHdrHelper();
        return;
      }
      repairSummary = "The checked Ubuntu packages are installed and current in the enabled repositories.";
      repairSummaryIsError = false;
      setNotice("The required system tools are installed and current in the enabled Ubuntu package sources.", "success");
      return;
    }
    if (!environment.checks.some((check) => check.key === "pkexec" && check.available)) {
      repairSummary = "Desktop administrator authentication (pkexec) is missing. No system packages were changed.";
      repairSummaryIsError = true;
      setNotice("Desktop administrator authentication (pkexec) is missing. Install the listed system packages through Ubuntu Software or a terminal.", "error");
      return;
    }
    repairConfirmOpen = true;
  }

  async function checkUbuntuPackageUpdates(silent = false): Promise<AptUpdateStatus | null> {
    if (!desktopShell || workflowBusy || aptChecking) return aptUpdates;
    aptChecking = true;
    try {
      aptUpdates = await invoke<AptUpdateStatus>("check_ubuntu_package_updates");
      const count = aptUpdates.packages.length;
      if (!silent) {
        setNotice(
          aptUpdates.catalog_stale
            ? `Found ${count} update${count === 1 ? "" : "s"} in a stale local APT catalog. Refresh the catalog before installing.`
            : count
              ? `${count} allowlisted Ubuntu package update${count === 1 ? "" : "s"} found.`
              : "No updates are available for the checked Ubuntu packages.",
          aptUpdates.catalog_stale ? "info" : "success",
        );
      }
      return aptUpdates;
    } catch (error) {
      if (!silent) setNotice(`Could not check Ubuntu package updates: ${String(error)}`, "error");
      return null;
    } finally {
      aptChecking = false;
    }
  }

  async function refreshUbuntuPackageCatalog(continueSetup = false) {
    if (!desktopShell || workflowBusy || aptChecking || !environment?.os.supported) return;
    repairLines = [];
    repairLogWarning = "";
    repairSummary = "Refreshing Ubuntu package information…";
    repairSummaryIsError = false;
    try {
      repairId = await invoke<string>("refresh_ubuntu_package_catalog");
      packageJobKind = "catalog-refresh";
      continueToolchainSetupAfterRefresh = continueSetup;
      setNotice("Refreshing the configured Ubuntu package catalog. No software is being installed.", "info");
    } catch (error) {
      continueToolchainSetupAfterRefresh = false;
      repairSummary = `Package information could not be refreshed: ${String(error)}`;
      repairSummaryIsError = true;
      setNotice(`Could not refresh the Ubuntu package catalog: ${String(error)}`, "error");
    }
  }

  async function refreshToolSetupLogs(preferredJobId = "") {
    if (!desktopShell) return;
    try {
      toolSetupLogs = await invoke<ToolSetupLogSummary[]>("list_tool_setup_logs");
      if (preferredJobId && toolSetupLogs.some((log) => log.jobId === preferredJobId)) {
        selectedToolSetupLog = preferredJobId;
        await openToolSetupLog(preferredJobId);
      } else if (!toolSetupLogs.some((log) => log.jobId === selectedToolSetupLog)) {
        selectedToolSetupLog = toolSetupLogs[0]?.jobId ?? "";
        viewedToolSetupLog = "";
      }
      toolSetupLogError = "";
    } catch (error) {
      toolSetupLogError = `Could not load saved setup logs: ${String(error)}`;
    }
  }

  async function openToolSetupLog(jobId = selectedToolSetupLog) {
    if (!desktopShell || !jobId || toolSetupLogLoading) return;
    selectedToolSetupLog = jobId;
    toolSetupLogLoading = true;
    toolSetupLogError = "";
    try {
      viewedToolSetupLog = await invoke<string>("read_tool_setup_log", { jobId });
    } catch (error) {
      viewedToolSetupLog = "";
      toolSetupLogError = `Could not read this setup log: ${String(error)}`;
    } finally {
      toolSetupLogLoading = false;
    }
  }

  async function exportToolSetupLog() {
    if (!desktopShell || !selectedToolSetupLog || toolSetupLogExporting) return;
    toolSetupLogExporting = true;
    try {
      const destination = await saveDialog({
        title: "Export tool setup log",
        defaultPath: `hevc-tool-setup-${selectedToolSetupLog}.log`,
        filters: [{ name: "Setup log", extensions: ["log", "txt"] }],
      });
      if (!destination) return;
      const savedPath = await invoke<string>("export_tool_setup_log", {
        jobId: selectedToolSetupLog,
        destination,
      });
      setNotice(`Saved the complete setup log to ${savedPath}.`, "success");
    } catch (error) {
      setNotice(`Could not export the setup log: ${String(error)}`, "error");
    } finally {
      toolSetupLogExporting = false;
    }
  }

  async function confirmAptRepair() {
    if (workspace.run.id || workspace.run.starting || repairId || hdrInstalling || scanning) return;
    if (!environment || !aptUpdates || aptUpdates.catalog_stale) return;
    const missingPackages = [...environment.missing_apt_packages];
    const updatePackages = aptUpdates.packages.map((item) => item.name);
    if (!missingPackages.length && !updatePackages.length) return;
    repairConfirmOpen = false;
    repairLines = [];
    repairLogWarning = "";
    const actions = [
      ...(missingPackages.length ? [`installing ${missingPackages.length} missing package${missingPackages.length === 1 ? "" : "s"}`] : []),
      ...(updatePackages.length ? [`updating ${updatePackages.length} package${updatePackages.length === 1 ? "" : "s"}`] : []),
    ];
    repairSummary = `${actions.join(" and ").replace(/^./, (first) => first.toUpperCase())}…`;
    repairSummaryIsError = false;
    installHdrHelperAfterPackages = environment.checks.some((check) => check.key === "hdr10plus_tool" && !check.available);
    repairToolChecksBefore = environment.checks.map((check) => ({ ...check }));
    repairAffectedToolKeys = toolKeysForPackages([...missingPackages, ...updatePackages]);
    if (installHdrHelperAfterPackages) repairAffectedToolKeys = [...new Set([...repairAffectedToolKeys, "hdr10plus_tool"])];
    repairToolOutcomes = [];
    try {
      repairId = await invoke<string>("install_ubuntu_toolchain", {
        toolPaths: currentToolPaths(),
        confirmedMissingPackages: missingPackages,
        confirmedUpdatePackages: updatePackages,
      });
      packageJobKind = "toolchain";
      setNotice("System tool setup started. APT will install the reviewed missing packages, then apply only the listed updates.", "info");
    } catch (error) {
      repairId = "";
      installHdrHelperAfterPackages = false;
      repairSummary = `Tool setup could not start: ${String(error)}`;
      repairSummaryIsError = true;
      setNotice(`Could not start system tool setup: ${String(error)}`, "error");
    }
  }

  async function installHdrHelper(preservePreviousResults = false) {
    if (workspace.run.id || workspace.run.starting || repairId || hdrInstalling || scanning) return;
    hdrInstalling = true;
    repairToolChecksBefore = environment?.checks.map((check) => ({ ...check })) ?? null;
    repairAffectedToolKeys = ["hdr10plus_tool"];
    if (!preservePreviousResults) repairToolOutcomes = [];
    repairLogWarning = "";
    const setupLogId = `${Date.now()}-${Math.floor(Math.random() * 1_000_000_000)}`;
    activeToolSetupLogId = setupLogId;
    repairSummary = "Downloading and verifying the latest HDR10+ helper…";
    repairSummaryIsError = false;
    setNotice("Installing the HDR10+ helper from its verified upstream release…", "info");
    try {
      const path = await invoke<string>("install_hdr10plus", { jobId: setupLogId });
      repairSummary = `HDR10+ helper installed at ${path}; checking its version…`;
      setNotice(`HDR10+ helper installed at ${path}. Rechecking the environment.`, "success");
      if (await loadEnvironment()) recordRepairToolOutcomes(repairToolChecksBefore, environment?.checks ?? []);
      repairSummary = environment?.checks.find((check) => check.key === "hdr10plus_tool")?.available
        ? "HDR10+ helper installed and detected."
        : "The helper installer finished, but the environment check still cannot detect HDR10+ helper.";
      repairSummaryIsError = !environment?.checks.find((check) => check.key === "hdr10plus_tool")?.available;
      await refreshToolSetupLogs(setupLogId);
    } catch (error) {
      repairSummary = `HDR10+ helper setup failed: ${String(error)}`;
      repairSummaryIsError = true;
      if (await loadEnvironment()) recordRepairToolOutcomes(repairToolChecksBefore, environment?.checks ?? []);
      await refreshToolSetupLogs(setupLogId);
      setNotice(`HDR10+ helper setup failed: ${String(error)}`, "error");
    } finally {
      hdrInstalling = false;
      activeToolSetupLogId = "";
      repairToolChecksBefore = null;
      repairAffectedToolKeys = [];
    }
  }

  async function openLog(path: string | null | undefined) {
    if (!path) {
      selectedLog = "This report has no companion diagnostic log.";
      return;
    }
    logLoading = true;
    try {
      selectedLog = await invoke<string>("read_log", { path });
    } catch (error) {
      selectedLog = `Could not read diagnostic log: ${String(error)}`;
    } finally {
      logLoading = false;
    }
  }

  function chooseReport(path: string) {
    selectedReportPath = path;
    const report = reports.find((item) => item.csv_path === path);
    if (report) void openLog(report.log_path);
  }

  async function copyLog() {
    try {
      await navigator.clipboard.writeText(selectedLog);
      setNotice("Diagnostic excerpt copied to the clipboard.", "success");
    } catch (error) {
      setNotice(`Could not copy diagnostic text: ${String(error)}`, "error");
    }
  }

  onMount(() => {
    let disposed = false;
    document.addEventListener("contextmenu", handleUtilityContextMenu);
    publishTheme(theme);
    desktopShell = isTauri();
    if (!desktopShell) {
      loadBrowserPreferences();
      environmentError = "Open HEVC Rebuild Studio from the desktop app to inspect this computer and connect to the media tools.";
      void Promise.all([refreshTmdbCredentialStatus(), refreshTvdbCredentialStatus()]);
      booting = false;
      return () => {
        disposed = true;
        document.removeEventListener("contextmenu", handleUtilityContextMenu);
      };
    }
    const register = async () => {
      const listeners = await Promise.all([
        listen<ScanProgress>("scan-progress", (event) => { scanProgress = event.payload; }),
        listen<RunLineEvent>("run-line", (event) => {
          if (event.payload.runId === workspace.run.id) workspace.run.lines = [...workspace.run.lines, event.payload.line].slice(-240);
          else if (workspace.run.starting) runEventBuffer.line(event.payload);
        }),
        listen<RunProgressEvent>("run-progress", (event) => {
          if (event.payload.runId === workspace.run.id) {
            noteRunProgress(event.payload);
          }
          else if (workspace.run.starting) runEventBuffer.update(event.payload);
        }),
        listen<RunResourceEvent>("run-resources", (event) => {
          if (event.payload.runId === workspace.run.id) workspace.run.resources = event.payload.snapshot;
          else if (workspace.run.starting) runEventBuffer.sample(event.payload);
        }),
        listen<RunFinishedEvent>("run-finished", async (event) => {
          if (event.payload.runId !== workspace.run.id) {
            if (workspace.run.starting) runEventBuffer.finish(event.payload);
            return;
          }
          await finishRun(event.payload);
        }),
        listen<RunLineEvent>("repair-line", (event) => {
          if (event.payload.runId === repairId) repairLines = [...repairLines, event.payload.line].slice(-120);
        }),
        listen<RunLineEvent>("repair-log-warning", (event) => {
          if (event.payload.runId === activeToolSetupLogId) repairLogWarning = event.payload.line;
        }),
        listen<RepairFinished>("repair-finished", async (event) => {
          if (event.payload.jobId !== repairId) return;
          const completedKind = packageJobKind;
          const checksBefore = repairToolChecksBefore;
          const affectedToolKeys = repairAffectedToolKeys;
          let helperStarted = false;
          repairLogWarning = event.payload.logWarning ?? "";
          repairId = "";
          packageJobKind = null;
          repairSummary = event.payload.success
            ? completedKind === "catalog-refresh" ? "Package information refreshed; checking installed versions…"
              : completedKind === "toolchain" ? "Reviewed package setup finished; checking installed versions…"
                : completedKind === "upgrade" ? "Package updates finished; checking installed versions…"
                  : "Package repair finished; checking installed versions…"
            : event.payload.error ?? "The package operation did not finish successfully.";
          repairSummaryIsError = !event.payload.success;
          if (event.payload.success) setNotice(completedKind === "catalog-refresh" ? "Ubuntu package catalog refreshed." : completedKind === "toolchain" ? "Reviewed Ubuntu package setup completed. Rechecking installed tools." : completedKind === "upgrade" ? "Ubuntu package upgrades completed." : "Package repair completed. Rechecking the installed tools.", "success");
          else setNotice(event.payload.error ?? "The package operation did not finish successfully.", "error");
          await refreshToolSetupLogs(event.payload.jobId);
          const environmentLoaded = await loadEnvironment();
          if (environmentLoaded && affectedToolKeys.length) recordRepairToolOutcomes(checksBefore, environment?.checks ?? []);
          repairToolChecksBefore = null;
          repairAffectedToolKeys = [];
          if (aptUpdates || completedKind === "catalog-refresh" || completedKind === "upgrade" || completedKind === "toolchain") await checkUbuntuPackageUpdates(true);
          if (completedKind === "catalog-refresh" && continueToolchainSetupAfterRefresh) {
            continueToolchainSetupAfterRefresh = false;
            if (event.payload.success) await repairApt();
          }
          if (completedKind === "toolchain" && event.payload.success && installHdrHelperAfterPackages) {
            installHdrHelperAfterPackages = false;
            if (environment?.checks.some((check) => check.key === "hdr10plus_tool" && !check.available)) {
              helperStarted = true;
              await installHdrHelper(true);
            }
          }
          if (completedKind === "toolchain" && !event.payload.success) installHdrHelperAfterPackages = false;
          if (completedKind === "catalog-refresh" && !event.payload.success) continueToolchainSetupAfterRefresh = false;
          if (event.payload.success && !helperStarted) {
            repairSummary = environment?.ready
              ? "Setup complete. All required media tools are ready."
              : "Package work completed. One or more required tools still need attention; check their status in Settings.";
          }
        }),
        listen<{ paths?: unknown }>(TauriEvent.DRAG_DROP, (event) => {
          const paths = Array.isArray(event.payload.paths)
            ? [...new Set(event.payload.paths.filter((path): path is string => typeof path === "string" && path.trim().length > 0))]
            : [];
          if (!paths.length) return;
          if (workflowBusy) {
            setNotice("Finish the current scan or run before adding dropped files.", "error");
            return;
          }
          openPage("library");
          setNotice(`Adding ${paths.length} dropped path${paths.length === 1 ? "" : "s"} to the queue.`, "info");
          void addQueueSources(paths);
        }),
      ]);
      const windowResizeListener = await getCurrentWindow().onResized(() => scheduleWindowStateSave());
      listeners.push(windowResizeListener);
      if (disposed) listeners.forEach((unlisten) => unlisten());
      else unlisteners = listeners;
    };
    void register().then(() => initialize()).catch((error) => {
      environmentError = String(error);
      booting = false;
      setNotice(`The desktop backend did not start: ${String(error)}`, "error");
    });
    return () => {
      disposed = true;
      document.removeEventListener("contextmenu", handleUtilityContextMenu);
      unlisteners.forEach((unlisten) => unlisten());
      if (contextMenu) void contextMenu.close().catch(() => {});
    };
  });
</script>

<svelte:head>
  <title>HEVC Rebuild Studio</title>
  <meta name="description" content="A hardware-aware, safety-first desktop studio for HEVC media library rebuilds." />
</svelte:head>

{#snippet MetadataWorkbenchSurface(compactItemMode: boolean, backgroundWorker = false)}
  <MetadataWorkbench {scanResult} focusMediaPath={metadataFocusPath} workbenchActive={backgroundWorker || (compactItemMode ? metadataDialogOpen : page === "metadata")} {compactItemMode} autoLookupPending={metadataLookupPending} metadataAutoLookup={appPreferences.metadataAutoLookup} metadataLanguage={appPreferences.metadataLanguage} textBlacklistPatterns={settings?.metadata.text_blacklist ?? []} savePosterBesideMedia={appPreferences.savePosterBesideMedia} {tmdbCredentialStatus} {tvdbCredentialStatus} metadataSources={appPreferences.metadataSources} queueSuggestions={queueMatchSuggestions} onOpenSettings={openMetadataProviderSettings} onStartMetadataOnly={startMetadataOnlyFromOrganizer} onAutoLookupConsumed={() => metadataLookupPending = false} onClaimAutoLookup={claimMetadataLookup} onLookupStatus={(status: MetadataLookupStatus) => {
    if (!status.sourceRoot || !workspace.queue.activeRoot || queuePathKey(status.sourceRoot) === queuePathKey(workspace.queue.activeRoot)) {
      metadataLookupStatus = status;
      if (status.status !== "searching") void persistQueueMetadataSuggestions();
    }
  }} onMatchSuggestion={(mediaPath: string, suggestion: QueueMetadataSuggestion) => {
    queueMatchSuggestions = { ...queueMatchSuggestions, [mediaPath]: suggestion };
    scheduleQueueMetadataSuggestionSave();
  }} onPathsRenamed={handleQueuePathsRenamed} onRenameState={handleQueueRenameState} bind:selectedMediaPaths={workspace.queue.selectedPaths} bind:movieAssignments={queueMovieReviews} bind:seriesAssignments={queueSeriesReviews} bind:organizerTemplates={appPreferences.organizerTemplates} onOrganizerTemplatesChange={(templates: OrganizerTemplates) => void persistPreferences({ organizerTemplates: templates })} animationForPath={(path: string) => { const entry = queuedEntries.find((item) => item.path === path); if (!entry) return fileTuneOverrides[path] === "animation"; return effectiveContentTuneForEntry(entry, queueMovieReviews[path] ?? reviewedSeriesForPath(path), settingsForEntry(entry)) === "animation"; }} onAnimationChange={(path: string, animated: boolean) => setFileTuneOverride(path, animated ? "animation" : "none")} />
{/snippet}



<div class="app-shell" data-theme={theme} data-instrument={instrumentTheme ? theme : undefined}>
  <aside class="sidebar">
    <div class="brand-lockup">
      <div class="brand-mark"><Clapperboard size={20} strokeWidth={2.2} /></div>
      <div><div class="brand-name">HEVC <span>REBUILD</span></div><div class="brand-edition">STUDIO · UBUNTU</div></div>
    </div>

    <div class="workspace-label">WORKSPACE</div>
    <nav aria-label="Main navigation">
      {#each navigation as item (item.id)}
        {@const Icon = item.icon}
        <button class="nav-item" class:active={page === item.id} onclick={() => openPage(item.id)} aria-label={item.label} title={item.label} aria-current={page === item.id ? "page" : undefined}>
          <Icon size={18} strokeWidth={1.8} /><span>{item.label}</span>
          {#if item.id === "home" && missingRequiredTools.length}<span class="nav-count">{missingRequiredTools.length}</span>{/if}
          {#if item.id === "reports" && failedRows.length}<span class="nav-count">{failedRows.length}</span>{/if}
          {#if page === item.id}<ChevronRight class="nav-arrow" size={15} />{/if}
        </button>
      {/each}
    </nav>

    <div class="sidebar-grow"></div>
    <div class="sidebar-library">
      <button class="sidebar-library-home" type="button" aria-label="Open media library" title="Open media library" onclick={() => openPage("library")}>
        <span class="sidebar-library-icon"><FolderOpen size={17} /></span>
        <span class="sidebar-library-copy"><span>ACTIVE LIBRARY</span><strong title={root}>{root ? fileName(root) : "No folder selected"}</strong></span>
      </button>
      <button class="icon-button subdued" aria-label="Choose media folder" title="Choose a media folder" onclick={browseFolder}><ChevronRight size={16} /></button>
    </div>
    <div class="sidebar-bottom">
      <span class="connection-dot" class:green={environment?.ready}></span>
      <span>{environment?.ready ? "Engine ready" : environmentLoading || booting ? "Checking engine…" : "Setup required"}</span>
      <span class="version-tag">v1.3.0</span>
    </div>
  </aside>

  <main class="main-panel">
    <header class="topbar">
      <div class="breadcrumbs"><span>HEVC REBUILD</span><ChevronRight size={14} /><strong>{navigation.find((item) => item.id === page)?.label}</strong></div>
      <div class="top-actions">
        <label class="theme-control" title="Choose the application appearance">
          {#if theme === "light"}<Sun size={15} aria-hidden="true" />{:else if theme === "dark"}<Moon size={15} aria-hidden="true" />{:else}<Terminal size={15} aria-hidden="true" />{/if}
          <span class="appearance-label">Appearance</span>
          <select aria-label="Appearance" value={theme} onchange={(event) => applyTheme(event.currentTarget.value)}>
            <option value="light">Light</option>
            <option value="dark">Dark</option>
            <option value="phosphor">Phosphor</option>
            <option value="ion">Ion</option>
            <option value="breach">Breach</option>
          </select>
        </label>
        {#if environment?.ready}
          <span class="status-pill good"><span class="status-dot"></span> SYSTEM READY</span>
        {:else if environmentLoading || booting}
          <span class="status-pill neutral"><LoaderCircle size={14} class="spin" /> CHECKING</span>
        {:else}
          <button class="status-pill warning" aria-label={setupGapDetail} title={setupGapDetail} onclick={openToolchainSettings}><AlertTriangle size={14} /><span>{setupGapLabel}</span></button>
        {/if}
        <button class="icon-button" aria-label="Refresh environment" title="Refresh environment" disabled={environmentLoading} onclick={loadEnvironment}><RefreshCw size={17} class={environmentLoading ? "spin" : ""} /></button>
      </div>
    </header>

    {#if workspace.notice.message}
      <div class="notice" class:error={workspace.notice.kind === "error"} class:success={workspace.notice.kind === "success"} role="status">
        {#if workspace.notice.kind === "error"}<AlertTriangle size={16} />{:else if workspace.notice.kind === "success"}<CheckCircle2 size={16} />{:else}<Info size={16} />{/if}
        <span>{workspace.notice.message}</span><button class="notice-close" aria-label="Dismiss notification" onclick={() => (workspace.notice.message = "")}><X size={15} /></button>
      </div>
    {/if}

    <RunProgress run={workspace.run} {encodeMeasured} {overallRunPercent} {progressElapsedLabel} batchCount={pendingRunBatches.length} batchIndex={activeRunBatchIndex} {bytes} {duration} {openProgressDetails} {stopAfterCurrent} />

    <div class="page-wrap" class:page-wrap-library={page === "library"} class:page-wrap-home={page === "home"}>
      {#if page === "home"}
        <HomeDashboard
          skin={theme === "phosphor" || theme === "ion" || theme === "breach" ? theme : null}
          environment={environment}
          environmentLoading={environmentLoading || booting}
          sourceCount={queuedSourceCount}
          fileCount={queuedEntryCount}
          activeFolder={workspace.queue.activeRoot ? fileName(workspace.queue.activeRoot) : ""}
          statusCounts={queueStatusCounts}
          activityLabel={homeActivityLabel}
          reports={reports}
          setupLogs={toolSetupLogs}
          onOpen={openPage}
          onOpenReport={(csvPath: string) => { openPage("reports"); chooseReport(csvPath); }}
          onAddFolder={() => { void browseFolder(); }}
          onAddFiles={() => { void browseFile(); }}
          onRefreshChecks={() => { void loadEnvironment(); }}
        />
      {:else if page === "help"}
        <HelpCenter />
      {:else if page === "settings"}
        <SettingsPreferences {preferenceStorage} bind:settings {settingsChanged} {workflowBusy} {appPreferences} bind:metadataLanguageDraft {tmdbCredentialStatus} {tvdbCredentialStatus} {desktopShell} bind:tmdbTokenDraft bind:tmdbTokenVisible {tmdbCredentialBusy} bind:tvdbApiKeyDraft bind:tvdbCredentialsVisible {tvdbCredentialBusy} bind:tvdbSubscriberPinDraft bind:tvdbAccessConfirmed {artworkCacheLoading} {artworkCachePurging} {artworkCacheInfo} {artworkCacheError} bind:artworkCacheConfirm bind:newFilenameIncludePattern bind:newFilenameExcludePattern {filenameFilterError} bind:newMetadataBlacklistPattern {metadataBlacklistError} {saveSettings} {persistPreferences} {saveMetadataLanguageDraft} {updateMetadataSources} {openMetadataProviderSettings} {toggleTmdbCredentialVisibility} {saveTmdbCredential} {removeTmdbCredential} {toggleTvdbCredentialVisibility} {saveTvdbCredential} {removeTvdbCredential} {refreshArtworkCache} {bytes} {purgeUnusedArtworkCache} {addFilenamePattern} {clearFilenamePatterns} {removeFilenamePattern} {addMetadataBlacklistPattern} {clearMetadataBlacklist} {removeMetadataBlacklistPattern}>
          {#snippet environmentPanel()}{@render EnvironmentPanel(environment)}{/snippet}
        </SettingsPreferences>

      {:else if page === "metadata"}
        <!-- The record workbench is mounted below so scans can look up matches from the queue. -->
      {:else if page === "library"}
        <section class="library-toolbar card-surface">
          <div class="folder-field"><FolderOpen size={18} /><input aria-label="Path to add to queue" bind:value={root} placeholder="Paste a media file or folder path to add" disabled={workflowBusy} onblur={() => void persistPreferences({ root })} /><button class="button secondary" type="button" disabled={!desktopShell || workflowBusy || !root} onclick={analyzeFolder}>{scanning ? "Scanning" : "Add path"}</button></div>
          <div class="toolbar-options"><label class="check-label"><input type="checkbox" bind:checked={recursive} disabled={workflowBusy} onchange={() => void persistPreferences({ recursive })} /> Include subfolders on new scans</label><label class="small-field">Ignore smaller than <input type="number" min="0" max="4096" step="1" bind:value={minimumSizeMib} disabled={workflowBusy} onblur={() => void persistPreferences({ minimumSizeMib })} /> MiB</label><button class="button secondary" disabled={!desktopShell || workflowBusy} onclick={browseFolder}><FolderOpen size={17} /> Add folder</button><button class="button secondary" disabled={!desktopShell || workflowBusy} onclick={browseFile}><Video size={17} /> Add files</button></div>
        </section>

        <LibraryQueueStatus queue={workspace.queue} busy={workflowBusy} {scanning} {scanProgress} {fileName} onActivate={activateQueueSource} onRefresh={(path: string) => scanQueueSource(path, true, true)} onRemove={removeQueueSource} />

        {#if scanResult}
          {#if runReviewEntries.some((entry) => queueRunStateForPath(entry.path)?.status === "interrupted")}<div class="inline-warning"><AlertTriangle size={17} /><div><strong>Some files were still active when the last session ended.</strong><span>Automatic recovery did not finish. Originals and existing outputs were left untouched, and the encode was not resumed.</span></div></div>{/if}
          {#if unresolvedSelectedPaths > 0}<div class="inline-warning"><AlertTriangle size={17} /><div><strong>{selectionHasNoRunnableFiles ? "These selected files are not ready to run." : `${unresolvedSelectedPaths} selected file${unresolvedSelectedPaths === 1 ? " is" : "s are"} not in a completed scan.`}</strong><span>Rescan or remove the interrupted source before starting this selection.</span></div><button class="text-button" onclick={() => workspace.queue.selectedPaths = workspace.queue.selectedPaths.filter((path) => selectedRunEntries.some((entry) => entry.path === path))}>Keep ready files</button></div>{/if}
          {#if staleSelectedEntries.length}<div class="inline-warning"><AlertTriangle size={17} /><div><strong>{staleSelectedEntries.length} queued file{staleSelectedEntries.length === 1 ? " has" : "s have"} changed or gone missing.</strong><span>Refresh each affected source and review the updated scan before running.</span><details><summary>Sources to refresh · {staleRunSources.length}</summary>{#each staleRunSources as batch (batch.source.root)}<button class="text-button" disabled={workflowBusy} onclick={() => refreshQueueSource(batch.source.root)}>Refresh {fileName(batch.source.root)}</button>{/each}</details></div></div>{/if}
          {#if analysisErrors.length}
            <div class="inline-warning"><AlertTriangle size={17} /><div><strong>{analysisErrors.length} file{analysisErrors.length === 1 ? "" : "s"} could not be analyzed</strong><span>Open the card details for the error. These files stay out of a run until the scan succeeds.</span></div></div>
          {/if}
          {#if !modeToolsReady && mode !== "check"}
            <div class="inline-warning"><Wrench size={17} /><div><strong>{mode === "metadata" ? "Metadata-only output needs FFmpeg, FFprobe, MediaInfo, and MKVToolNix." : "Rebuild needs the full toolchain."}</strong><span>Review detected versions and select compatible executable paths in Settings.</span></div><button class="text-button" onclick={() => openPage("settings")}>Open Settings</button></div>
          {/if}
          <div class="table-header-tools library-poster-search"><label class="search-box"><Search size={16} /><input aria-label="Search files" value={search} oninput={(event) => { search = event.currentTarget.value; mediaPage = 1; }} placeholder="Search files or status" /></label></div>
          <LibraryPosterGrid
            cards={libraryPosterCards}
            busy={workflowBusy}
            selectedOnPage={selectedVisibleCount}
            selectedTotal={queueSelectionCount}
            readyCount={selectedRunEntries.length}
            lookupMessage={workspace.run.progress?.fileName ? `${workspace.run.progress.fileName} · ${workspace.run.progress.phase}` : libraryLookupMessage}
            lookupFailed={metadataLookupStatus?.status === "error"}
            crumbs={libraryCrumbs}
            reviewCurrentLabel={libraryBrowse ? (libraryBrowse.season === null ? "Review series" : "Review season") : ""}
            onReviewCurrent={libraryBrowse ? reviewCurrentBrowse : null}
            canRename={!workflowBusy && queueSelectionCount > 0}
            canProcess={!workflowBusy && selectedRunEntries.length > 0}
            observeArtwork={observeQueueArtwork}
            onToggleSelect={(paths: string[], selected: boolean) => { workspace.queue.selectedPaths = selectQueuePaths(workspace.queue.selectedPaths, paths, selected); }}
            onToggleAnimated={setPathsAnimated}
            onOpenReview={openLibraryPoster}
            onOpenLocation={(path: string) => void openFileLocation(path)}
            onReview={openLibraryReview}
            onRename={() => chooseLibraryTask("organize")}
            onMetadata={() => chooseLibraryTask("metadata")}
            onRebuild={() => chooseLibraryTask("rebuild")}
            onSetOutput={chooseFileOutput}
            onClearOutput={clearFileOutput}
            onSelectPage={setVisibleSelection}
          />
          <LibraryRenameDialog {libraryRenameOpen} bind:libraryRenameDialog bind:renameDraft bind:moviePatternField bind:episodePatternField bind:libraryRenameError {libraryRenameBusy} {libraryRenameEntries} {libraryRenameShape} {libraryRenameNote} {insertRenameToken} {resetRenamePattern} {applyLibraryRename} onClose={() => libraryRenameOpen = false} />
          {#if visibleEntries.length > mediaPageSize}
            <nav class="media-pagination" aria-label="Media file pages">
              <span>Showing {mediaPageFirst}–{mediaPageLast} of {visibleEntries.length} files · page {mediaPage} of {mediaPageCount}</span>
              <label>Cards per page<select aria-label="Cards per page" value={mediaPageSize} onchange={(event) => { mediaPageSize = Number(event.currentTarget.value); mediaPage = 1; }}><option value={50}>50</option><option value={100}>100</option><option value={200}>200</option></select></label>
              <div><button type="button" class="button secondary" disabled={workflowBusy || mediaPage <= 1} onclick={() => mediaPage = Math.max(1, mediaPage - 1)}>Previous</button><button type="button" class="button secondary" disabled={workflowBusy || mediaPage >= mediaPageCount} onclick={() => mediaPage = Math.min(mediaPageCount, mediaPage + 1)}>Next</button></div>
            </nav>
          {/if}
        {:else if !scanning}
          <div class="empty-state large"><div class="empty-icon"><Layers size={26} /></div><div><div class="eyebrow">LIBRARY QUEUE</div><h2>{workspace.queue.sources.length ? "Choose a folder to review." : "Add your first media."}</h2><p>{workspace.queue.sources.length ? "Select a folder above, or add another folder or a group of files." : "Add a video or folder. Posters and titles will appear here as each file is matched."}</p></div><button class="button secondary" disabled={!desktopShell || workflowBusy} onclick={browseFolder}>Add folder</button></div>
        {/if}

      {:else if page === "youtube"}
        <YouTubePreparation queue={workspace.queue} run={workspace.run} bind:root bind:recursive bind:minimumSizeMib bind:youtubeResolution bind:youtubeQualityMode bind:youtubeSourceMultiplier bind:youtubeOutputDir bind:search bind:mediaPage {scanResult} {scanProgress} {scanning} {workflowBusy} {desktopShell} {youtubeEstimateSummary} {analysisErrors} {toolsRequiredEntries} {selectedRunEntries} {visibleEntries} {selectedVisibleCount} {selectedFilteredCount} {mediaPageSize} {selectedRunInputBytes} {selectedMediaPathsForRun} {modeToolsReady} {canStart} {progressElapsedLabel} {bytes} {fileName} {persistPreferences} {browseFolder} {browseFile} {analyzeFolder} {chooseYoutubeOutput} {openPage} {setFilteredSelection} {stopAfterCurrent} {beginRun}>
          {#snippet mediaTable()}{@render MediaTable(currentPageEntries, () => {})}{/snippet}
        </YouTubePreparation>

      {:else if page === "run"}
        <section class="page-heading">
          <div><div class="eyebrow">CONTROLLED PROCESSING</div><h1>Run <em>setup.</em></h1><p>Choose how to prepare the selected files and review the output location before starting.</p></div>
          {#if scanResult}<div class="library-chip"><FolderOpen size={15} /><span title={root}>{fileName(root)}</span><span class="chip-divider"></span>{scanResult.candidate_count} files</div>{/if}
        </section>

        {#if !scanResult}<div class="inline-warning"><Info size={17} /><div><strong>Analyze a file or folder before choosing a task.</strong><span>The scan lists media details, resource forecasts, and file-specific warnings.</span></div><button class="button secondary" onclick={() => openPage("library")}>Open media library</button></div>{/if}

        <section class="mode-section"><div class="section-title-row"><div><div class="eyebrow">PROCESSING TASK</div><h2>Choose what to do</h2></div><span class="mode-guidance"><ShieldCheck size={15} />{modeToolsReady ? "Required tools ready" : "Resolve the required tools"}</span></div>
          <div class="mode-grid run-mode-grid">
            <button class="mode-card" class:selected={mode === "check"} disabled={workflowBusy} onclick={() => (mode = "check")}><span class="mode-number">01</span><span class="mode-icon mint"><ScanSearch size={19} /></span><span class="mode-title">Check only</span><span class="mode-description">Review planned actions and preflight each file. No output is written.</span><span class="mode-tag">READ-ONLY</span></button>
            <button class="mode-card" class:selected={mode === "metadata"} disabled={workflowBusy} onclick={() => (mode = "metadata")}><span class="mode-number">02</span><span class="mode-icon lavender"><FileText size={18} /></span><span class="mode-title">Metadata-only MKV</span><span class="mode-description">Copy every track into a new MKV, update reviewed tags and poster, then validate. No video encode.</span><span class="mode-tag">SOURCE STAYS IN PLACE</span></button>
            <button class="mode-card" class:selected={mode === "test"} disabled={workflowBusy} onclick={() => (mode = "test")}><span class="mode-number">03</span><span class="mode-icon lavender"><Play size={18} /></span><span class="mode-title">Short test</span><span class="mode-description">Create short test samples. Originals stay exactly where they are.</span><span class="mode-tag">5 SECONDS · TEST CRF 30</span></button>
            <button class="mode-card" class:selected={mode === "full"} disabled={workflowBusy} onclick={() => (mode = "full")}><span class="mode-number">04</span><span class="mode-icon amber"><Zap size={18} /></span><span class="mode-title">Full rebuild</span><span class="mode-description">Process the real files. In-place replacements happen only after validation.</span><span class="mode-tag warning-tag">CONFIRM TO START</span></button>
          </div>
        </section>

        <section class="settings-card output-folder-settings" aria-labelledby="rebuild-output-title">
          <div class="settings-card-heading"><div class="settings-icon blue"><FolderOpen size={18} /></div><div><h3 id="rebuild-output-title">Output folders</h3><p>Choose where new MKVs and short tests are written.</p></div></div>
          <div class="upload-destination"><span>Default folder</span><strong>{appPreferences.rebuildOutputDir ?? (mode === "metadata" ? "Beside each source as <filename>.metadata.mkv" : "In place: validated full rebuild replaces the source, with the original saved under old/")}</strong><div><button class="text-button" disabled={!desktopShell || workflowBusy} onclick={chooseRebuildOutput}>Choose folder</button><button class="text-button" disabled={!appPreferences.rebuildOutputDir || workflowBusy} onclick={() => void persistPreferences({ rebuildOutputDir: null })}>Use default location</button></div></div>
          <p class="filename-filter-help">Metadata-only output always creates a separate file. A chosen default folder mirrors the source folder structure; a per-file folder chosen in Media Library takes precedence. Existing outputs are never overwritten.</p>
        </section>

        {#if settings}
          <section class="settings-intro" id="rebuild-parameters"><div class="section-title-row"><div><div class="eyebrow">EFFECTIVE CONFIGURATION</div><h2>Rebuild parameters</h2></div><div class="settings-intro-actions"><span class="edit-badge"><Settings2 size={14} /> {settingsChanged ? "Unsaved changes" : appPreferences.settings ? "Saved app profile" : "Library values"}</span><button class="text-button" disabled={(!settingsChanged && !appPreferences.settings) || workflowBusy} onclick={resetSettings}>Reset to library values</button><button class="text-button" disabled={!settingsChanged || workflowBusy} onclick={saveSettings}>Save as app defaults</button></div></div><p>Application, user, and library values load first. Save your edits as app defaults to keep them across launches; unsaved edits apply to the next run only.</p>{#if forecastOutdated}<div class="inline-warning"><Info size={17} /><div><strong>Resource settings changed after the folder scan.</strong><span>The per-file forecast on the library page is from that scan. Actual worker/frame counts are recomputed immediately before encoding each file.</span></div></div>{/if}</section>
          <fieldset class="settings-grid" aria-label="Rebuild settings" disabled={workflowBusy}>
            <article class="settings-card"><div class="settings-card-heading"><div class="settings-icon purple"><Film size={18} /></div><div><h3>Picture encoding</h3><p>CPU libx265 · video stream only</p></div></div>
              <label class="form-row range-row"><span><strong>Quality</strong><small>CRF {settings.video.crf} · lower means larger, higher-quality files</small></span><input aria-label="CRF quality" type="range" min="0" max="51" step="1" bind:value={settings.video.crf} /><b>{settings.video.crf}</b></label>
              <label class="form-row"><span><strong>Preset</strong><small>Compression effort and time</small></span><select bind:value={settings.video.preset}><option value="ultrafast">ultrafast</option><option value="superfast">superfast</option><option value="veryfast">veryfast</option><option value="faster">faster</option><option value="fast">fast</option><option value="medium">medium</option><option value="slow">slow · recommended</option><option value="slower">slower</option><option value="veryslow">veryslow</option></select></label>
              <label class="toggle-row"><span><strong>Promote 8-bit sources to 10-bit</strong><small>Off by default; does not add missing color detail</small></span><input type="checkbox" bind:checked={settings.video.promote_8bit_to_10bit} /></label>
              <label class="form-row"><span><strong>Content tune</strong><small>Auto uses reliable animation metadata</small></span><select bind:value={settings.content.tune}><option value="auto">Automatic</option><option value="animation">Animation</option><option value="grain">Film grain</option><option value="none">Normal</option></select></label>
              <label class="toggle-row"><span><strong>Detect animation</strong><small>Only when content tune is automatic</small></span><input type="checkbox" bind:checked={settings.content.detect_animation} /></label>
            </article>

            <article class="settings-card"><div class="settings-card-heading"><div class="settings-icon blue"><ShieldCheck size={18} /></div><div><h3>HDR & stream safety</h3><p>Preserve by default. Never transcode audio or subtitles.</p></div></div>
              <label class="toggle-row"><span><strong>Preserve HDR10+ dynamic metadata</strong><small>Verify dynamic metadata after encoding</small></span><input type="checkbox" bind:checked={settings.hdr.preserve_hdr10plus} /></label>
              <label class="form-row"><span><strong>Dolby Vision policy</strong><small>Safe skips unsupported profiles instead of damaging them</small></span><select bind:value={settings.hdr.dolby_vision}><option value="safe">Safe preservation</option><option value="skip">Skip all Dolby Vision</option><option value="hdr10-fallback">Verified HDR10 fallback</option></select></label>
              <label class="form-row"><span><strong>Secondary video tracks</strong><small>Choose how additional selected video tracks are handled</small></span><select bind:value={settings.streams.secondary_video}><option value="encode">Encode all selected tracks</option><option value="copy">Copy additional tracks</option><option value="drop">Keep primary track only</option></select></label>
              <label class="form-row"><span><strong>Data streams</strong><small>Preserve Matroska-compatible data</small></span><select bind:value={settings.streams.data_streams}><option value="preserve">Preserve</option><option value="drop">Drop</option></select></label>
              <label class="toggle-row"><span><strong>SHA-256 verify copied streams</strong><small>Audio and subtitle packets are copied, never encoded</small></span><input type="checkbox" bind:checked={settings.safety.verify_copy_hashes} /></label>
              <label class="text-setting"><span><strong>Matroska COMMENT</strong><small>Default comment for every new file. A series, season, episode, or film can keep its own.</small></span><input maxlength="500" bind:value={settings.metadata.comment} /></label>
            </article>

            <article class="settings-card wide-settings"><div class="settings-card-heading"><div class="settings-icon green-bg"><Gauge size={18} /></div><div><h3>Resource guardrails</h3><p>One file at a time · automatic memory and disk protection</p></div></div>
              <div class="resource-groups">
                <section>
                  <h4>CPU</h4>
                  <div class="resource-grid">
                    <label><span>CPU policy</span><select bind:value={settings.resources.threading}><option value="auto">Automatic</option><option value="unlimited">Use all CPU threads · memory-limited</option></select></label>
                    <label><span>Leave CPU threads free</span><input type="number" min="0" max="64" bind:value={settings.resources.reserved_cpu_threads} /></label>
                    <label><span>Worker thread cap · 0 = auto</span><input type="number" min="0" max="64" bind:value={settings.resources.max_worker_threads} /></label>
                    <label><span>Frame thread cap · 0 = auto</span><input type="number" min="0" max="16" bind:value={settings.resources.frame_threads} /></label>
                  </div>
                </section>
                <section>
                  <h4>Memory</h4>
                  <div class="resource-grid">
                    <label><span>Minimum free memory · GiB</span><input type="number" min="0" step="0.25" bind:value={settings.resources.min_free_memory_gib} /></label>
                    <label><span>Memory reserve · percent</span><input type="number" min="0" max="50" step="0.5" bind:value={settings.resources.memory_reserve_percent} /></label>
                  </div>
                </section>
                <section>
                  <h4>Disk</h4>
                  <div class="resource-grid">
                    <label><span>Extra free-disk reserve · GiB</span><input type="number" min="0" step="0.5" bind:value={settings.resources.min_free_disk_gib} /></label>
                    <label><span>Temporary output estimate · ratio</span><input type="number" min="0.25" max="4" step="0.05" bind:value={settings.resources.preflight_output_ratio} /></label>
                    <label><span>Live size projection margin · percent</span><input type="number" min="0" max="100" step="1" bind:value={settings.resources.disk_projection_margin_percent} /></label>
                  </div>
                </section>
                <section>
                  <h4>Calibration</h4>
                  <div class="resource-grid">
                    <label><span>Hardware calibration</span><select bind:value={settings.calibration.mode}><option value="auto">Automatic · 4K/large files</option><option value="ask">Ask in interactive CLI when uncached</option><option value="always">Every file · benchmark each time</option><option value="off">Disabled</option></select></label>
                    <label><span>Calibration cache · hours</span><input type="number" min="0" max="720" step="1" bind:value={settings.calibration.cache_hours} /></label>
                    <label><span>Large-file threshold · GiB</span><input type="number" min="0" max="1000" step="1" bind:value={settings.calibration.large_file_gib} /></label>
                    <label><span>Sample duration · seconds</span><input type="number" min="1" max="15" step="1" bind:value={settings.calibration.sample_seconds} /></label>
                    <label><span>Maximum layouts to compare</span><input type="number" min="1" max="4" step="1" bind:value={settings.calibration.max_candidates} /></label>
                    <label><span>Throughput tie margin · percent</span><input type="number" min="0" max="20" step="0.5" bind:value={settings.calibration.throughput_tolerance_percent} /></label>
                  </div>
                </section>
                <div class="hardware-note"><Cpu size={17} /><span>Detected: <strong>{environment?.hardware.cpu_model ?? "CPU details unavailable"}</strong>. Automatic calibration samples 4K and large files, then caches the worker and frame counts. RAM, swap, and memory-stall checks stay on during the encode. Swap already on disk stays out of the free-memory count.</span></div>
              </div>
            </article>
          </fieldset>
          <div class="config-source"><Info size={14} /><span title={settingsSources.join(" · ")}>Configuration: {settingsSources.length ? settingsSources.map((source) => fileName(source)).join(" · ") : fileName(settingsPath) || "application defaults"}{preferenceStorage ? ` · ${fileName(preferenceStorage.settingsFile)}` : !desktopShell ? " · browser preview" : ""}</span><button class="text-button" disabled={workflowBusy} onclick={() => loadSettings(root)}>Reload effective values</button></div>
        {/if}

        {#if selectionInOtherSources}<div class="inline-warning"><Info size={17} /><div><strong>This workflow will process {runBatches.length} queued sources.</strong><span>{workspace.queue.selectedPaths.length ? `${selectedRunEntries.length} selected files` : `${selectedRunEntries.length} queued files`} will run sequentially, with each source’s saved scan settings.</span></div></div>{/if}
        {#if runBatches.length}<details class="queue-run-plan"><summary>Review {selectedRunEntries.length} files in {runBatches.length} source batch{runBatches.length === 1 ? "" : "es"}</summary>{#each runBatches as batch (batch.source.root)}{@const previewEntries = runPlanPreviewByRoot.get(batch.source.root) ?? []}<details class="queue-run-source"><summary><strong>{fileName(batch.source.root)}</strong><span>{batch.entries.length} file{batch.entries.length === 1 ? "" : "s"}</span><code title={batch.source.root}>{batch.source.root}</code></summary><ul>{#each previewEntries as entry (entry.path)}<li><strong>{entry.name}</strong><code title={entry.path}>{entry.path}</code></li>{/each}</ul></details>{/each}{#if selectedRunEntries.length > 250}<small>Showing the first 250 files across the queue plan. All {selectedRunEntries.length} files remain included.</small>{/if}</details>{/if}
        {#if unresolvedSelectedPaths > 0}<div class="inline-warning"><AlertTriangle size={17} /><div><strong>{unresolvedSelectedPaths} selected file{unresolvedSelectedPaths === 1 ? " is" : "s are"} not available from a completed scan.</strong><span>Refresh or remove that source before starting. Ready files can be kept in the selection.</span></div><button class="text-button" onclick={() => workspace.queue.selectedPaths = workspace.queue.selectedPaths.filter((path) => selectedRunEntries.some((entry) => entry.path === path))}>Keep ready files</button></div>{/if}
        {#if staleSelectedEntries.length}<div class="inline-warning"><AlertTriangle size={17} /><div><strong>Refresh {staleRunSources.length} affected source{staleRunSources.length === 1 ? "" : "s"} before running.</strong><span>{staleSelectedEntries.length} file{staleSelectedEntries.length === 1 ? " has" : "s have"} changed or disappeared since scanning. Refresh only the source groups shown here.</span><details><summary>Sources to refresh</summary>{#each staleRunSources as batch (batch.source.root)}<button class="text-button" disabled={workflowBusy} onclick={() => refreshQueueSource(batch.source.root)}>Refresh {fileName(batch.source.root)}</button>{/each}</details></div></div>{/if}
        <section class="run-launch card-surface"><div class="launch-left"><div class="launch-icon"><TerminalSquare size={19} /></div><div><strong>{workspace.run.id ? "A workflow is running" : mode === "full" ? "Full rebuild requires confirmation" : mode === "test" ? "Short test run" : mode === "metadata" ? "Create validated MKV copies without re-encoding" : mode === "youtube" ? "Create a YouTube upload copy" : "Ready for a check-only pass"}</strong><span>{scanResult ? `${selectedRunEntries.length} file${selectedRunEntries.length === 1 ? "" : "s"}${selectedMediaPathsForRun.length ? " selected" : ""} · ${bytes(selectedRunInputBytes)} total` : "Add a file or folder to the queue"}{mode !== "metadata" && mode !== "check" && environment?.hardware.gpu_devices.length ? ` · ${environment.hardware.gpu_devices.length} GPU detected, CPU encoder selected` : ""}</span></div></div>
          <label class="check-label stop-error"><input type="checkbox" bind:checked={stopOnError} onchange={() => void persistPreferences({ stopOnError })} /> Stop queue on first error</label>
          {#if workspace.run.id}<button class="button danger-outline" onclick={stopAfterCurrent}><Square size={15} /> Stop after current file</button>{:else}<button bind:this={runLaunchButton} class="button primary" disabled={!canStart} onclick={beginRun}>{#if mode === "full"}<ShieldCheck size={17} />{:else if mode === "test"}<Play size={17} />{:else if mode === "metadata"}<FileText size={17} />{:else if mode === "youtube"}<Video size={17} />{:else}<ScanSearch size={17} />{/if}{mode === "full" ? "Review & start full rebuild" : mode === "test" ? "Start short tests" : mode === "metadata" ? "Create metadata-only MKVs" : mode === "youtube" ? "Create upload copies" : "Start check-only pass"}</button>{/if}
        </section>

        {#if workspace.run.message || workspace.run.lines.length}
          <RunConsole run={workspace.run} title="Run activity" elapsedLabel={progressElapsedLabel} onStop={stopAfterCurrent} />
        {/if}

      {:else if page === "reports"}
        <ReportBrowser {repairId} {hdrInstalling} {reportDeleteBusy} bind:reportDeleteConfirm {failedRows} {loadReports} {deleteReportHistory} {reports} {activeReport} {logLoading} {selectedLog} {fileName} {bytes} {isFailure} {chooseReport} {openLog} {copyLog} {openPage} hasScan={Boolean(scanResult)} />

      {/if}

      {#if page === "metadata"}
        <div class="metadata-workbench-slot active">{@render MetadataWorkbenchSurface(false)}</div>
      {/if}

      {#if scanResult && appPreferences.metadataAutoLookup && page !== "metadata"}
        <div class="metadata-discovery-worker" aria-hidden="true" hidden>
          {@render MetadataWorkbenchSurface(false, true)}
        </div>
      {/if}

      {#if metadataDialogOpen && scanResult?.entries.find((entry) => entry.path === metadataFocusPath)}
        {@const focusedEntry = scanResult.entries.find((entry) => entry.path === metadataFocusPath)!}
        {@const focusedMovie = queueMovieReviews[focusedEntry.path]}
        {@const focusedSeries = reviewedSeriesForPath(focusedEntry.path)}
        {@const focusedSuggestion = queueMatchSuggestions[focusedEntry.path]}
        {@const focusedFilename = proposedFilenameForEntry(focusedEntry, focusedMovie, focusedSeries, focusedSuggestion, appPreferences.organizerTemplates)}
        {@const focusedProposedName = focusedFilename?.name ?? focusedEntry.name}
        {@const focusedDestination = reviewDestinationPath(focusedEntry, focusedProposedName)}
        {@const focusedFolder = focusedEntry.path.slice(0, Math.max(focusedEntry.path.lastIndexOf("/"), focusedEntry.path.lastIndexOf("\\")))}
        <Modal open={metadataDialogOpen} class="metadata-review-dialog" bind:element={metadataDialogElement} labelId="metadata-review-dialog-title" closeOnBackdrop={true} onClose={() => { metadataDialogOpen = false; libraryTvReview = null; }} initialFocus=".metadata-review-dialog-header button">
          <header class="metadata-review-dialog-header">
            <div class="metadata-review-dialog-title">
              <span class="eyebrow">{libraryTvReview?.level === "series" ? "SERIES REVIEW" : libraryTvReview?.level === "season" ? "SEASON REVIEW" : libraryTvReview ? "EPISODE REVIEW" : "MEDIA FILE REVIEW"}</span>
              <h2 id="metadata-review-dialog-title">{libraryTvReview ? (tvReviewHeading(libraryTvReview) || focusedEntry.name) : (focusedMovie?.title || focusedSeries?.title || focusedSuggestion?.title || focusedEntry.name)}</h2>
              {#if libraryTvReview?.level === "series" || libraryTvReview?.level === "season"}
                <p class="metadata-review-stay">{libraryTvReview.level === "series" ? libraryTvReview.seriesRoot.split(/[\\/]/u).filter(Boolean).at(-1) : `${queueSeriesReviews[seriesReviewStorageKey(libraryTvReview.seriesRoot)]?.title || "Series"} · season ${libraryTvReview.season}`}</p>
              {:else}
              <div class="metadata-review-filenames">
                {#if focusedProposedName === focusedEntry.name}
                  <div><span>Name</span><strong>{focusedEntry.name}</strong></div>
                {:else}
                  <div><span>Original name</span><strong>{focusedEntry.name}</strong></div>
                  <ArrowRight class="review-name-arrow" size={16} />
                  <div><span>New name</span><strong>{focusedProposedName}</strong></div>
                {/if}
                {#if focusedDestination.path === focusedEntry.path}
                  <p class="metadata-review-stay">Stays in {focusedFolder}</p>
                {:else}
                  <p class="metadata-review-move">{focusedDestination.note} <code>{focusedDestination.path}</code></p>
                {/if}
              </div>
              {/if}
            </div>
            <button class="icon-button" type="button" aria-label="Close media file review" title="Close" onclick={closeMetadataDialog}><X size={18} /></button>
          </header>
          <div class="metadata-review-dialog-content">
            {#if libraryTvReview}
              {@const target = libraryTvReview}
              {@const review = queueSeriesReviews[seriesReviewStorageKey(target.seriesRoot)]}
              {@const seasonKey = target.season === null ? "" : String(target.season)}
              {@const seasonRecord = seasonKey ? review?.seasonRecords?.[seasonKey] : undefined}
              {@const episodeRecord = target.episodePath ? (review?.episodeAssignments?.[target.episodePath] ?? episodeDraft(target.episodePath)) : undefined}
              {@const group = seriesGroupForRoot(target.seriesRoot)}
              {@const posterPath = activeReviewPosterPath(target, review)}
              {@const markedPaths = tvReviewPaths(target)}
              {@const marked = animationSelection(markedPaths)}
              {@const premiereYear = seasonPremiereYear(review, target.season)}
              {@const seasonYear = seasonRecord?.year && seasonRecord.year !== review.year ? seasonRecord.year : (premiereYear || seasonRecord?.year || "")}
              {#if review && (target.level !== "season" || seasonRecord) && (target.level !== "episode" || target.episodePath)}
                <LibraryTvReview
                  level={target.level}
                  kicker={target.level === "series" ? "" : (review.title || "")}
                  title={target.level === "season" ? (seasonRecord?.title ?? "") : target.level === "episode" ? (episodeRecord?.title ?? "") : review.title}
                  year={target.level === "season" ? seasonYear : review.year}
                  certification={target.level === "series" ? review.certification : (ownCertificateFor(target, review) || sharedCertificateFor(target, review))}
                  certificationNote={certificateNoteFor(target, review)}
                  certificationCustom={Boolean(ownCertificateFor(target, review))}
                  plot={target.level === "season" ? (seasonRecord?.plot.trim() || review.plot) : target.level === "episode" ? (episodeRecord?.summary ?? "") : review.plot}
                  plotNote={target.level === "season" ? seasonPlotNote(review, seasonRecord) : ""}
                  plotCustom={target.level === "season" && Boolean(seasonRecord?.plot.trim())}
                  genresText={target.level === "season" ? (seasonRecord?.genresText ?? "") : review.genresText}
                  ratingText={target.level === "season" ? (seasonRecord?.ratingText.trim() || review.ratingText) : review.ratingText}
                  ratingNote={target.level === "season" ? seasonRatingNote(review, seasonRecord) : ""}
                  ratingCustom={target.level === "season" && Boolean(seasonRecord?.ratingText.trim())}
                  seasonNumber={target.level === "episode" ? String(episodeRecord?.season ?? "") : ""}
                  episodeNumber={target.level === "episode" ? String(episodeRecord?.episode ?? "") : ""}
                  airdate={episodeRecord?.airdate ?? ""}
                  comment={review ? (ownCommentFor(target, review) || sharedCommentFor(target, review)) : (settings?.metadata.comment ?? "")}
                  commentNote={review ? commentNoteFor(target, review) : "Using the comment from Settings."}
                  commentCustom={review ? Boolean(ownCommentFor(target, review)) : false}
                  posterUrl={displayPosterUrl(posterPath) ?? ""}
                  posters={group ? reviewPosterChoices(group, target.level === "series" ? null : target.season, review) : []}
                  selectedPosterPath={posterPath}
                  animated={marked === "all"}
                  animatedMixed={marked === "some"}
                  onTitle={(value: string) => patchTvField(target, "title", value)}
                  onYear={(value: string) => patchTvField(target, "year", value)}
                  onCertification={(value: string) => patchTvField(target, "certification", value)}
                  onUseSharedCertification={() => patchTvField(target, "certification", "")}
                  onPlot={(value: string) => patchTvField(target, "plot", value)}
                  onUseSharedPlot={() => patchTvField(target, "plot", "")}
                  onGenres={(value: string) => patchTvField(target, "genresText", value)}
                  onRating={(value: string) => patchTvField(target, "ratingText", value)}
                  onUseSharedRating={() => patchTvField(target, "ratingText", "")}
                  onSeasonNumber={(value: string) => patchTvField(target, "season", value)}
                  onEpisodeNumber={(value: string) => patchTvField(target, "episode", value)}
                  onAirdate={(value: string) => patchTvField(target, "airdate", value)}
                  onComment={(value: string) => setReviewComment(target, value)}
                  onUseSharedComment={() => setReviewComment(target, "")}
                  onSelectPoster={(path: string) => selectReviewPoster(target, path)}
                  onChooseImage={() => void chooseReviewImage()}
                  onAnimated={(animated: boolean) => setPathsAnimated(markedPaths, animated)}
                />
              {:else}
                <p class="review-status">Preparing this review…</p>
              {/if}
            {:else}
              {@render MetadataWorkbenchSurface(true)}
              {#if focusedEntry && queueMovieReviews[focusedEntry.path]}
                {@const movieComment = queueMovieReviews[focusedEntry.path].comment?.trim() ?? ""}
                <div class="review-fields review-movie-comment">
                  <label>Comment<input maxlength="500" value={movieComment || settings?.metadata.comment || ""} oninput={(event) => setMovieComment(focusedEntry.path, event.currentTarget.value)} /></label>
                  {#if movieComment}<p class="review-note"><button class="text-button" type="button" onclick={() => setMovieComment(focusedEntry.path, "")}>Use the shared comment</button></p>{/if}
                </div>
              {/if}
            {/if}
            {#if focusedEntry}
            {@const mediaEntries = libraryTvReview ? reviewMediaEntries(libraryTvReview) : [focusedEntry]}
            {#if mediaEntries.length}
              <details class="review-more review-media-info">
                <summary>Media info · {mediaEntries.length} file{mediaEntries.length === 1 ? "" : "s"}</summary>
                {#each mediaEntries as entry (entry.path)}
                  <details class="review-media-file" open={mediaEntries.length === 1}>
                    <summary><strong>{entry.name}</strong><small>{mediaInfoLine(entry)}</small></summary>
                    <div class="review-media-facts">
                      <button class="text-button" type="button" onclick={() => void openFileLocation(entry.path)}>Open file location</button>
                      {#if entry.profile}
                        {#each entry.profile.tracks as track (track.index)}
                          <p>{track.type} · {track.codec || "Unknown codec"}{track.language ? ` · ${track.language}` : ""}{track.title ? ` · ${track.title}` : ""}{track.width && track.height ? ` · ${track.width}×${track.height}` : ""}{track.channels ? ` · ${track.channels} ch` : ""}</p>
                        {/each}
                        {#if Object.keys(entry.profile.mediainfo_general ?? {}).length}
                          <dl class="track-metadata">{#each Object.entries(entry.profile.mediainfo_general) as [key, value] (key)}<div><dt>{key.replaceAll("_", " ")}</dt><dd>{formatMetadataValue(value)}</dd></div>{/each}</dl>
                        {/if}
                      {:else}
                        <p>This file has no scanned media info yet. Refresh the source and open the review again.</p>
                      {/if}
                    </div>
                  </details>
                {/each}
              </details>
            {/if}
            {/if}
          </div>
          <footer class="metadata-review-dialog-footer">
            <span>{libraryTvReview ? "Edits are saved as you type. Done closes this review." : "Choose a poster and confirm the title. Batch rename stays in the organizer."}</span>
            <div><button class="button secondary" type="button" onclick={() => void openFileLocation(reviewLocationPath(libraryTvReview, focusedEntry.path))}>Open file location</button><button class="button secondary" type="button" onclick={openFullMetadataOrganizer}>Open full organizer <ArrowRight size={15} /></button><button class="button primary" type="button" onclick={closeMetadataDialog}>Done</button></div>
          </footer>
        </Modal>
      {/if}

      <footer class="page-footer"><span><ShieldCheck size={14} /> Local processing · Originals protected</span><span>HEVC Rebuild Studio <i>•</i> Ubuntu-first preview</span></footer>
    </div>
  </main>

{#if fullConfirmOpen}
  <Modal returnFocus={runLaunchButton} open={fullConfirmOpen} class="confirm-modal" labelId="confirm-title" onClose={() => (fullConfirmOpen = false)}>
      <div class="confirm-symbol"><AlertTriangle size={22} /></div><button class="icon-button modal-close" aria-label="Close confirmation" onclick={() => (fullConfirmOpen = false)}><X size={18} /></button>
      <div class="eyebrow">FINAL SAFETY CHECK</div><h2 id="confirm-title">Start the full rebuild?</h2><p>This will encode {selectedMediaPathsForRun.length ? `${selectedMediaPathsForRun.length} selected file${selectedMediaPathsForRun.length === 1 ? "" : "s"}` : "every eligible queued file"} across {runBatches.length} source{runBatches.length === 1 ? "" : "s"}, one source at a time.{#if appPreferences.rebuildOutputDir || Object.keys(appPreferences.fileOutputDirs).some((path) => selectedRunEntries.some((entry) => entry.path === path))} Rebuilt files will be saved to the configured output folders. Originals stay in place. Per-file choices override the default folder, and existing output files will not be replaced.{:else} Each original stays untouched until its rebuilt output passes validation, then moves into that folder’s <code>old/</code> directory.{/if}</p>
      {#if appPreferences.rebuildOutputDir || Object.keys(appPreferences.fileOutputDirs).some((path) => selectedRunEntries.some((entry) => entry.path === path))}
        <div class="confirm-output-review"><strong>Output destinations</strong>
          {#if appPreferences.rebuildOutputDir}<p>Default: <code>{appPreferences.rebuildOutputDir}</code> · input subfolders mirrored</p>{:else}<p>No default folder. Files without a per-file choice use in-place replacement.</p>{/if}
          {#if selectedRunEntries.some((entry) => appPreferences.fileOutputDirs[entry.path])}<div class="confirm-output-list">{#each selectedRunEntries.filter((entry) => appPreferences.fileOutputDirs[entry.path]) as entry (entry.path)}<div><span>{entry.name}</span><code>{appPreferences.fileOutputDirs[entry.path]}</code></div>{/each}</div>{/if}
        </div>
      {/if}
      <div class="confirm-facts"><span><Film size={16} /> {selectedRunEntries.length} {selectedMediaPathsForRun.length ? "selected" : "queued"} file{selectedRunEntries.length === 1 ? "" : "s"}</span><span class="confirm-disk"><HardDrive size={16} /> About {bytes(selectedRunDiskBudget.working)} of temporary space for {selectedRunEntries.length === 1 ? "this file" : "the largest file"}{#if selectedRunDiskBudget.floored}. The finished size is not known yet, so the budget stays at 2 GB even for a small file{/if}. {Number.isFinite(selectedRunFreeBytes) ? `${bytes(selectedRunFreeBytes)} is already free on ${runBatches.length === 1 ? "this volume" : "the tightest volume"}` : "Free space is checked again before the file starts"}.</span><span><Cpu size={16} /> CPU libx265 · one file at a time</span></div>
      <label class="confirm-check"><input type="checkbox" bind:checked={fullAck} /> I reviewed the analysis and understand this starts real encodes.</label>
      <div class="modal-actions"><button class="button secondary" onclick={() => (fullConfirmOpen = false)}>Go back</button><button class="button primary" disabled={!fullAck || !canStart} onclick={() => launchRun(true)}><ShieldCheck size={16} /> Start full rebuild</button></div>
  </Modal>
{/if}

{#if outputPreviewOpen && outputPreview}
  <Modal returnFocus={runLaunchButton} open={outputPreviewOpen} class="confirm-modal queue-output-modal" labelId="output-preview-title" onClose={() => (outputPreviewOpen = false)}>
      <button class="icon-button modal-close" aria-label="Close output review" onclick={() => (outputPreviewOpen = false)}><X size={18} /></button>
      <div class="eyebrow">OUTPUT REVIEW · {outputPreview.mode.toUpperCase()}</div>
      <h2 id="output-preview-title">Review where these files will go</h2>
      <p>{outputPreview.fileCount} file{outputPreview.fileCount === 1 ? "" : "s"} across {outputPreview.sourceCount} source{outputPreview.sourceCount === 1 ? "" : "s"}. This checks the whole queue before any file starts; the engine checks each source again before writing.</p>
      {#if outputPreview.eligible}
        <div class="queue-output-status clear"><CheckCircle2 size={17} /><span>No existing-output or duplicate-path collisions were found.</span></div>
      {:else}
        <div class="queue-output-status blocked"><AlertTriangle size={17} /><span>{outputPreview.collisionCount} collision{outputPreview.collisionCount === 1 ? "" : "s"} need{outputPreview.collisionCount === 1 ? "s" : ""} a different output folder before this queue can start.</span></div>
      {/if}
      {#if outputPreview.mode === "full"}
        <div class="queue-output-note">The new file is encoded beside the original. After it passes validation, the original moves into <code>old/</code> and the new file takes the current name. If validation fails, the original stays where it is.</div>
      {:else if outputPreview.mode === "metadata"}
        <div class="queue-output-note">Metadata-only mode creates a new MKV copy and keeps the original file in place.</div>
      {:else if outputPreview.testOutputsMayReceiveUniqueSuffixes}
        <div class="queue-output-note">Short-test outputs use a <code>Test_</code> prefix. If that name already exists, the engine adds a unique timestamp.</div>
      {/if}

      {#if outputPreview.collisions.length}
        <div class="queue-output-list-heading"><strong>Paths that block the run</strong>{#if outputPreview.omittedCollisionCount}<span>Showing {outputPreview.collisions.length} of {outputPreview.collisionCount}</span>{/if}</div>
        <div class="queue-output-list queue-output-conflicts">
          {#each outputPreview.collisions as collision, index (`${collision.source}:${collision.target}:${index}`)}
            <article><span>{collision.kind === "existing_output" ? "Already exists" : "Same destination"}</span><code title={collision.target}>{#each collision.target.split("/") as part, index (index)}{#if index > 0}/{/if}{part}<wbr>{/each}</code><small>{fileName(collision.source)}{#if collision.conflictsWith} · also planned for {fileName(collision.conflictsWith)}{/if}</small></article>
          {/each}
        </div>
        {#if outputPreview.omittedCollisionCount}<small class="queue-output-truncated">{outputPreview.omittedCollisionCount} additional collision records are omitted from this view.</small>{/if}
      {/if}

      <div class="queue-output-list-heading"><strong>{outputPreview.targets.every((target) => target.replacesSource) ? "After a successful rebuild" : "Planned destinations"}</strong><span>{Math.min(outputPreview.targets.length, 250)}{#if outputPreview.omittedTargetCount} shown · {outputPreview.omittedTargetCount} more{/if}</span></div>
      <div class="queue-output-list">
        {#each outputPreview.targets as target (target.source)}
          <article>
            <span title={target.source}>{fileName(target.source)}</span>
            {#if target.replacesSource}
              <small>Original moves here</small>
              <code title={archivedOriginalPath(target.source)}>{#each archivedOriginalPath(target.source).split("/") as part, index (index)}{#if index > 0}/{/if}{part}<wbr>{/each}</code>
              <small>Rebuilt file keeps the current name</small>
              <code title={target.target}>{#each target.target.split("/") as part, index (index)}{#if index > 0}/{/if}{part}<wbr>{/each}</code>
            {:else}
              <code title={target.target}>{#each target.target.split("/") as part, index (index)}{#if index > 0}/{/if}{part}<wbr>{/each}</code>
              {#if target.note}<small>{target.note}</small>{/if}
            {/if}
          </article>
        {/each}
      </div>
      {#if outputPreview.omittedTargetCount}<small class="queue-output-truncated">The preview is capped at 250 rows. All {outputPreview.fileCount} files were checked for collisions.</small>{/if}

      <div class="modal-actions">
        <button class="button secondary" onclick={() => (outputPreviewOpen = false)}>Back</button>
        {#if !outputPreview.eligible}<button class="button secondary" onclick={() => { outputPreviewOpen = false; openPage("run"); }}>Adjust output folders</button>{/if}
        <button class="button primary" disabled={!outputPreview.eligible || outputPreviewBusy || !canStart} onclick={confirmOutputPreview}>{outputPreviewBusy ? "Checking paths…" : outputPreview.eligible ? (outputPreview.targets.every((target) => target.replacesSource) ? "Confirm and start" : "Confirm destinations and start") : "Resolve collisions first"}</button>
      </div>
  </Modal>
{/if}

<RecoveryReview returnFocus={runLaunchButton} {recoveryReviewOpen} {recoveryCurrentItem} {recoveryReviewIndex} {recoveryReportStatus} {recoveryProbeEntry} {recoveryProbeError} {recoveryProbeAttempted} {desktopShell} {recoveryProbeBusy} {workflowBusy} bind:recoveryAcknowledged {recoveryCanMarkComplete} {queueRunStatusLabel} {inspectRecoveryTarget} {finishRecoveryReview} itemCount={recoveryReviewItems.length} onClose={() => recoveryReviewOpen = false} />

<ToolRepairConfirmation {repairConfirmOpen} {environment} {aptUpdates} {workflowBusy} {confirmAptRepair} onClose={() => repairConfirmOpen = false} />
</div>

{#snippet MediaTable(entries: MediaEntry[], onOpenLibrary: () => void)}
  {#if entries.length}
    <div class="table-scroll"><table><thead><tr><th class="selection-header">{#if page === "library" || page === "youtube"}<input bind:this={visibleSelectAllCheckbox} type="checkbox" aria-label={`Select all ${currentPageEntries.length} media files on this page`} checked={currentPageEntries.length > 0 && selectedVisibleCount === currentPageEntries.length} disabled={workflowBusy || !currentPageEntries.length} onchange={(event) => setVisibleSelection(event.currentTarget.checked)} />{:else}<span class="sr-only">Select file</span>{/if}</th><th>MEDIA FILE</th><th>VIDEO PROFILE</th><th>STREAMS</th><th>{page === "youtube" ? "YOUTUBE MEMORY PLAN" : "REBUILD MEMORY PLAN"}</th><th>STATUS</th></tr></thead><tbody>
      {#each entries as entry (entry.path)}
        {@const youtubeEstimate = youtubeEstimateFor(entry)}
        {@const memoryForecast = resourceForecastFor(entry)}
        {@const queueRun = page === "library" ? queueRunStateForPath(entry.path) : undefined}
        {@const queueRunLog = queueRun ? queueRunLogs[queuePathKey(entry.path)] : undefined}
        {@const rowProgress = workspace.run.progress?.filePath && queuePathKey(workspace.run.progress.filePath) === queuePathKey(entry.path) ? workspace.run.progress : null}
        {@const entrySettings = page === "library" ? settingsForEntry(entry) : settings}
        {@const reviewedMovie = queueMovieReviews[entry.path]}
        {@const reviewedSeries = page === "library" ? reviewedSeriesForPath(entry.path) : undefined}
        {@const reviewedMetadata = reviewedMovie ?? reviewedSeries}
        {@const matchSuggestion = page === "library" ? queueMatchSuggestions[entry.path] : undefined}
        {@const filenameProposal = page === "library" ? proposedFilenameForEntry(entry, reviewedMovie, reviewedSeries, matchSuggestion, appPreferences.organizerTemplates) : null}
        {@const reviewedPoster = reviewedMetadata?.artworkCandidates.find((candidate) => candidate.id === reviewedMetadata.posterId)}
        {@const localPosterPath = reviewedMetadata?.localPosterPath ?? ""}
        {@const localPosterKey = localPosterPath ? queuePathKey(localPosterPath) : ""}
        <tr data-media-path={entry.path}>
          <td><input type="checkbox" aria-label={`Select ${entry.name}`} checked={workspace.queue.selectedPaths.includes(entry.path)} disabled={workflowBusy || (entry.queue_stale === "missing" && !workspace.queue.selectedPaths.includes(entry.path))} onchange={(event) => { workspace.queue.selectedPaths = event.currentTarget.checked ? [...new Set([...workspace.queue.selectedPaths, entry.path])] : workspace.queue.selectedPaths.filter((path) => path !== entry.path); }} /></td>
          <td>
            <div class="file-cell">
              {#if page === "library" && reviewedMetadata}
                <button class="queue-poster-button" use:observeQueueArtwork={{ mediaPath: entry.path, artworkPath: localPosterPath }} type="button" aria-label={`Edit title and poster for ${entry.name}`} title={localPosterPath ? `${reviewedMetadata.title} · local poster` : reviewedPoster ? `${reviewedMetadata.title} poster · ${reviewedPoster.provider}` : `Edit ${reviewedMetadata.title || "reviewed title"}`} onclick={() => openMetadataForEntry(entry)}>
                  {#if localPosterPath && queueArtworkPreviews[localPosterKey]}<img src={queueArtworkPreviews[localPosterKey]} alt="" loading="lazy" />{:else if reviewedPoster && (!localPosterPath || queueArtworkPreviewErrors[localPosterKey])}<img src={reviewedPoster.thumbnailUrl || reviewedPoster.imageUrl} alt="" loading="lazy" referrerpolicy="no-referrer" />{:else}<span class="queue-poster-placeholder"><Film size={17} /></span>{/if}
                  <span class="queue-poster-edit"><Database size={12} /></span>
                </button>
              {:else if page === "library" && matchSuggestion}
                <button class="queue-poster-button queue-poster-suggested" type="button" aria-label={`Review suggested title and poster for ${entry.name}`} title={matchSuggestion.title ? `Suggested: ${matchSuggestion.title}${matchSuggestion.year ? ` (${matchSuggestion.year})` : ""} · review before assigning` : "Review title and poster"} onclick={() => openMetadataForEntry(entry)}>
                  {#if matchSuggestion.artwork}<img src={matchSuggestion.artwork.thumbnailUrl || matchSuggestion.artwork.imageUrl} alt="" loading="lazy" referrerpolicy="no-referrer" />{:else}<span class="queue-poster-placeholder">{#if matchSuggestion.status === "searching"}<LoaderCircle size={17} class="spin" />{:else}<Film size={17} />{/if}</span>{/if}
                  <span class="queue-poster-edit"><Database size={12} /></span>
                </button>
              {:else}
                <div class="file-icon"><Video size={17} /></div>
              {/if}
              <div class="file-cell-copy">
                <strong class="file-cell-title" title={entry.path}>{entry.name}</strong>
                <small class="file-cell-sub" title={entry.path}>{entry.relative_path}</small>
                {#if page === "library" && reviewedMetadata}<small class="queue-reviewed-metadata" title={`${reviewedMetadata.sourceUrl}${reviewedPoster ? ` · Poster: ${reviewedPoster.title}` : ""}`}><strong>{reviewedMetadata.title || "Untitled review"}</strong><span>{matchSuggestion?.autoAssociated ? "Auto-associated · exact title and year" : reviewedMetadata.recordId === "local-nfo" || reviewedMetadata.recordId.startsWith("local-nfo:") || reviewedMetadata.recordId.startsWith("local:") ? "Existing local metadata" : reviewedMetadata.recordId.split(":")[0].toUpperCase()}{reviewedMetadata.year ? ` · ${reviewedMetadata.year}` : ""}{localPosterPath ? " · poster saved for MKV output" : reviewedPoster ? " · poster preview only; save to include" : ""}{reviewedSeries && !reviewedMovie ? " · series" : ""}</span></small>{/if}
                {#if page === "library" && !reviewedMetadata && matchSuggestion?.status === "ready"}
                  {#if matchSuggestion.title}<small class="queue-match-suggestion"><strong>{matchSuggestion.autoAssociated ? "Auto-associated: " : "Suggested: "}{matchSuggestion.title}{matchSuggestion.year ? ` (${matchSuggestion.year})` : ""}</strong><span>{matchSuggestion.autoAssociated ? "Exact title and year; no competing result" : `${matchSuggestion.provider?.toUpperCase() ?? "Public source"} · review before assigning`}</span></small>
                  {:else}<small class="queue-match-suggestion"><strong>No likely title match found</strong><span>Review the filename and search other sources.</span></small>{/if}
                {:else if page === "library" && matchSuggestion?.status === "searching"}<small class="queue-match-suggestion">Checking the title and poster…</small>
                {:else if page === "library" && matchSuggestion?.status === "error"}<small class="row-error" title={matchSuggestion.error}>Title lookup failed: {matchSuggestion.error.slice(0, 150)}</small>{/if}
                {#if filenameProposal}<small class="queue-filename-proposal" class:needs-review={filenameProposal.needsTitleReview} title={`Proposed filename: ${filenameProposal.name}`}><span>{filenameProposal.needsTitleReview ? "Candidate filename · review title first" : "Proposed filename"}</span><code>{filenameProposal.name}</code></small>{/if}
                {#if page === "library"}
                  <label class="per-file-tune"><span>Content tune</span><select aria-label={`Content tune for ${entry.name}`} value={fileTuneOverrides[entry.path] ?? ""} disabled={workflowBusy} onchange={(event) => setFileTuneOverride(entry.path, event.currentTarget.value)}><option value="">Automatic · {effectiveContentTuneForEntry(entry, reviewedMetadata, entrySettings) === "animation" ? "Animated" : effectiveContentTuneForEntry(entry, reviewedMetadata, entrySettings) === "grain" ? "Film grain" : "Normal"}</option><option value="animation">Animated</option><option value="none">Normal</option></select></label>
                  {#if metadataSuggestsAnimation(reviewedMetadata)}<small class="file-cell-sub tune-suggestion">Animation is in the genres, so this file uses the animation tune unless you choose Normal.</small>{:else if entry.profile?.content?.animation_detected}<small class="file-cell-sub tune-suggestion">Animation was detected in source metadata and is reflected in the Automatic setting above.</small>{/if}
                {/if}
                {#if page === "library" && localPosterPath && queueArtworkPreviewErrors[localPosterKey]}<small class="queue-poster-error" title={queueArtworkPreviewErrors[localPosterKey]}>Local poster could not be previewed</small>{/if}
                {#if page === "library" && localPosterPath && queueArtworkPreviewBusy[localPosterKey]}<small class="queue-poster-loading">Loading local poster…</small>{/if}
                {#if entry.error}<small class="row-error">{entry.error}</small>{/if}
                {#if entry.warning}<small class="row-warning">{entry.warning}</small>{/if}
                {#if entry.queue_stale === "missing"}<small class="row-error">This file is missing since the last scan. Refresh the source after restoring or removing it.</small>{:else if entry.queue_stale === "changed"}<small class="row-warning">This file changed after its last scan. Refresh this source before running it.</small>{/if}
                {#if page === "library"}<button class="text-button file-metadata-action" type="button" aria-label={`${reviewedMetadata ? "Edit" : "Review"} title and poster for ${entry.name}`} onclick={() => openMetadataForEntry(entry)}><Database size={14} /> {reviewedMetadata ? `Edit ${reviewedSeries && !reviewedMovie ? "series" : "title"} and poster` : "Review title and poster"}</button>{/if}
                {#if entry.profile?.tracks.length}
                  <details class="stream-details">
                    <summary>Tracks & stream metadata · {entry.profile.tracks.length}{#if page === "library"} · {selectedTrackCount(entry)}/{entry.profile.tracks.length} included{/if}</summary>
                    {#if entry.profile.mediainfo_available === false}<small class="file-cell-sub">MediaInfo is unavailable; FFprobe still provides the visible stream and container data below.</small>{/if}
                    {#if entry.profile.frame_metadata_available === false}<small class="file-cell-sub">Per-frame side data could not be read. HDR10+ indicators are candidates and have not been confirmed from frame metadata.</small>{/if}
                    {#if page === "library"}
                      <p class="track-selection-note">Default selection follows the saved secondary-video and data-stream policies. After you customize this file, included real-video tracks are encoded to HEVC, while included audio, subtitles, attachments, and data tracks are copied. Keep both 4K and 1080p selected to retain both in one MKV.</p>
                    {/if}
                    {#if Object.keys(entry.profile.mediainfo_general ?? {}).length}
                      <details class="track-item">
                        <summary><strong>Container / general</strong></summary>
                        <dl class="track-metadata">{#each Object.entries(entry.profile.mediainfo_general) as [key, value] (key)}<div><dt>{key.replaceAll("_", " ")}</dt><dd>{formatMetadataValue(value)}</dd></div>{/each}</dl>
                      </details>
                    {/if}
                    {#if entry.profile.source_metadata}
                      <details class="track-item source-metadata-item">
                        <summary><strong>Source container & chapters</strong><span>{entry.profile.chapters_count} chapters</span></summary>
                        {#if entry.profile.source_metadata.format}<details class="raw-metadata-details"><summary>FFprobe format fields</summary><pre class="raw-metadata">{formatMetadataValue(entry.profile.source_metadata.format)}</pre></details>{/if}
                        {#if entry.profile.source_metadata.chapters?.length}<details class="raw-metadata-details"><summary>Chapter data</summary><pre class="raw-metadata">{formatMetadataValue(entry.profile.source_metadata.chapters)}</pre></details>{/if}
                      </details>
                    {/if}
                    <div class="track-list">
                      {#each entry.profile.tracks as track (track.index)}
                        <div class="track-selection-row">
                          {#if page === "library" && streamIsSelectable(track.type)}
                            <label class="track-selection-toggle"><input type="checkbox" checked={selectedStreamIndices(entry).includes(track.index)} disabled={workflowBusy} onchange={(event) => setFileStreamSelected(entry, track.index, event.currentTarget.checked)} /><span>{selectedStreamIndices(entry).includes(track.index) ? "Included" : "Excluded"}<small>stream #{track.index}</small></span></label>
                          {:else if page === "library"}
                            <span class="track-selection-toggle unavailable">View only<small>unsupported type</small></span>
                          {/if}
                          <details class="track-item">
                            <summary>
                              <strong>{track.attached_picture ? "Cover image" : `${track.type} ${track.ordinal + 1}`}</strong><span>{track.codec ?? "unknown codec"}</span>
                              {#if track.width && track.height}<span>{track.width} × {track.height}</span>{/if}
                              {#if track.bit_depth}<span>{track.bit_depth}-bit</span>{/if}
                              {#if track.language}<span>{track.language}</span>{/if}
                              {#if track.title}<span>{track.title}</span>{/if}
                              {#if track.default}<span class="track-flag">Default</span>{/if}
                              {#if track.forced}<span class="track-flag">Forced</span>{/if}
                            </summary>
                            <dl class="track-metadata">
                              {#if track.pixel_format}<div><dt>Pixel format</dt><dd>{track.pixel_format}</dd></div>{/if}
                              {#if track.color_primaries || track.color_transfer || track.color_matrix}<div><dt>Color</dt><dd>{[track.color_primaries, track.color_transfer, track.color_matrix].filter(Boolean).join(" · ")}</dd></div>{/if}
                              {#if track.channels || track.channel_layout || track.sample_rate}<div><dt>Audio</dt><dd>{[track.channels ? `${track.channels} channels` : "", track.channel_layout, track.sample_rate ? `${track.sample_rate} Hz` : ""].filter(Boolean).join(" · ")}</dd></div>{/if}
                              {#if track.filename || track.mime_type}<div><dt>Attachment</dt><dd>{[track.filename, track.mime_type].filter(Boolean).join(" · ")}</dd></div>{/if}
                              {#each Object.entries(track.mediainfo ?? {}) as [key, value] (key)}<div><dt>{key.replaceAll("_", " ")}</dt><dd>{formatMetadataValue(value)}</dd></div>{/each}
                            </dl>
                            {#if track.ffprobe}<details class="raw-metadata-details"><summary>FFprobe stream fields</summary><pre class="raw-metadata">{formatMetadataValue(track.ffprobe)}</pre></details>{/if}
                          </details>
                        </div>
                      {/each}
                    </div>
                  </details>
                {/if}
                {#if page === "library" && entry.profile}
                  {@const globalTune = entrySettings?.content.tune ?? "auto"}
                  {@const resolvedTune = effectiveContentTuneForEntry(entry, reviewedMetadata, entrySettings)}
                  <small class="file-cell-sub">Applied tune: {resolvedTune === "animation" ? "Animated" : resolvedTune === "grain" ? "Film grain" : "Normal"}{fileTuneOverrides[entry.path] ? " · file choice" : ` · ${globalTune === "auto" ? "Automatic" : "global setting"}`}</small>
                  {#if entry.profile.content?.animation_detected}<small class="file-cell-sub">Animation genre found in {entry.profile.content.sources.join(" + ") || "source metadata"}. {#if globalTune === "auto" && settings?.content.detect_animation}Automatic mode selects Animated.{:else}Choose Animated above to apply it to this file.{/if}</small>{/if}
                  <div class="file-output-control"><small>{appPreferences.fileOutputDirs[entry.path] ? `Output: ${appPreferences.fileOutputDirs[entry.path]}` : appPreferences.rebuildOutputDir ? `Default: ${appPreferences.rebuildOutputDir}` : "Output: source folder"}</small><div><button class="text-button" disabled={!desktopShell || workflowBusy} onclick={() => chooseFileOutput(entry.path)}>Set output folder</button>{#if appPreferences.fileOutputDirs[entry.path]}<button class="text-button" disabled={workflowBusy} onclick={() => clearFileOutput(entry.path)}>Use default</button>{/if}</div></div>
                {/if}
              </div>
            </div>
          </td>
          <td><span class="profile-primary">{profileSummary(entry)}</span><small class="file-cell-sub">{entry.profile?.video.codec ?? "—"}{entry.profile?.interlaced ? " · Interlaced" : ""}</small></td>
          <td><span class="stream-count"><Layers size={14} /> {entry.profile?.video.secondary_count !== undefined ? entry.profile.video.secondary_count + 1 : "—"} video <i>·</i> {entry.profile?.audio_count ?? "—"} audio <i>·</i> {entry.profile?.subtitle_count ?? "—"} subs <i>·</i> {entry.profile?.attachment_count ?? "—"} attachments</span>{#if entry.profile?.hdr.dolby_vision}<small class="file-cell-sub">Profile {entry.profile.hdr.dolby_vision_profile ?? "?"} · Dolby Vision</small>{/if}</td>
          <td>
            {#if memoryForecast}
              <div class="resource-plan-summary"><span class="status-tag" class:status-ready={memoryForecast.level === "guarded"} class:status-warning={memoryForecast.level === "constrained" || memoryForecast.level === "unknown"} class:status-error={memoryForecast.level === "high" || memoryForecast.level === "blocked"}>{memoryForecast.level.toUpperCase()}</span><small>{memoryForecast.planned_workers ?? "—"} workers · {memoryForecast.planned_frame_threads ?? "—"} frame threads</small></div>
              <details class="resource-plan-details">
                <summary>Memory and retry details</summary>
                <small>{memoryForecast.planning_budget_gib !== null && memoryForecast.planning_budget_gib !== undefined ? `${memoryForecast.planning_budget_gib.toFixed(1)} GiB planning budget` : "Memory budget unavailable"}{memoryForecast.swap_used_percent !== null && memoryForecast.swap_used_percent !== undefined ? ` · swap ${memoryForecast.swap_used_percent.toFixed(0)}% used` : ""}</small>
                {#if memoryForecast.messages.length}<p>{memoryForecast.messages.join(" ")}</p>{/if}
                {#if memoryForecast.retry_plan && memoryForecast.level !== "guarded"}
                  {#if memoryForecast.retry_plan.reduces_parallelism}<p>First pressure retry: {memoryForecast.retry_plan.planned_workers} workers · {memoryForecast.retry_plan.planned_frame_threads} frame thread{memoryForecast.retry_plan.planned_frame_threads === 1 ? "" : "s"} · {memoryForecast.retry_plan.level.toUpperCase()}.</p>
                  {:else}<p>Already at minimum parallelism. If pressure persists, the encode stops instead of repeating the same plan.</p>{/if}
                {/if}
              </details>
            {:else}<small class="file-cell-sub">No forecast</small>{/if}
          </td>
          <td>
            <span class="status-tag" class:status-ready={entry.status === "Ready"} class:status-warning={entry.status === "Backed up"} class:status-error={entry.status === "Analysis failed" || entry.status === "Tools required"}>{statusLabel(entry.status)}</span>
            <small class="file-cell-sub">Source {bytes(entry.size_bytes)}</small>
            {#if rowProgress}<div class="queue-live-progress" aria-label={`${rowProgress.phase} for ${entry.name}`}><div><span>{rowProgress.phase}</span>{#if rowProgress.filePercent != null && rowProgress.filePercent > 0}<strong>{rowProgress.filePercent.toFixed(0)}%</strong>{/if}</div><div class="queue-live-progress-track" role="progressbar" aria-label={`${rowProgress.phase} progress`} aria-valuemin="0" aria-valuemax="100" aria-valuenow={rowProgress.filePercent != null && rowProgress.filePercent > 0 ? rowProgress.filePercent : undefined} aria-busy={rowProgress.filePercent == null || rowProgress.filePercent <= 0}><span class:indeterminate={rowProgress.filePercent == null || rowProgress.filePercent <= 0} style={rowProgress.filePercent != null && rowProgress.filePercent > 0 ? `width:${rowProgress.filePercent}%` : ""}></span></div>{#if rowProgress.outputBytes != null || rowProgress.estimatedOutputBytes != null}<small>{rowProgress.outputBytes != null ? `Output ${bytes(rowProgress.outputBytes)}` : "Output is being created"}{rowProgress.estimatedOutputBytes != null ? ` · estimated ${bytes(rowProgress.estimatedOutputBytes)}` : ""}</small>{/if}</div>{/if}
            {#if queueRun}
              <small class="queue-run-state" class:queue-run-completed={queueRun.status === "completed" || queueRun.status === "renamed"} class:queue-run-warning={["queued", "running", "scanning", "renaming", "remuxing", "rebuilding", "validating", "skipped"].includes(queueRun.status)} class:queue-run-error={queueRun.status === "failed" || queueRun.status === "interrupted" || queueRun.status === "needs_review"}>{queueRunStatusLabel(queueRun.status)}{queueRun.mode ? ` · ${queueRun.mode === "organize" ? "RENAME" : queueRun.mode.toUpperCase()}` : ""}</small>
              {#if queueRun.message || queueRun.outputPath || queueRun.reportPath}
                <details class="queue-run-details">
                  <summary>Run details</summary>
                  {#if queueRun.message}<p>{queueRun.message}</p>{/if}
                  {#if queueRun.reportStatus}<small>Report result · {queueRun.reportStatus}</small>{/if}
                  {#if queueRun.outputPath}<small title={queueRun.outputPath}>Output · {queueRun.outputPath}</small>{/if}
                  {#if queueRun.reportPath}
                    <div class="queue-run-detail-actions">
                      <button class="text-button" type="button" disabled={workflowBusy} onclick={() => openQueueRunReport(queueRun.reportPath!)}>Open report <ArrowRight size={13} /></button>
                      <button class="text-button" type="button" disabled={queueRunLog?.loading} onclick={() => loadQueueRunDiagnostics(entry, queueRun)}>{queueRunLog?.loading ? "Loading diagnostics…" : queueRunLog?.text ? "Reload diagnostics" : "Load bounded diagnostics"}</button>
                    </div>
                    {#if queueRunLog?.error}<small class="queue-run-log-error">{queueRunLog.error}</small>{/if}
                    {#if queueRunLog?.text}<pre class="queue-run-log">{queueRunLog.text}</pre>{/if}
                  {/if}
                </details>
              {/if}
              {#if ["queued", "failed", "skipped", "interrupted", "needs_review"].includes(queueRun.status)}
                <div class="queue-row-actions">
                  <button class="text-button" type="button" disabled={!desktopShell || workflowBusy || Boolean(entry.queue_stale)} onclick={() => prepareQueueEntryForRetry(entry, queueRun)}>{queueRun.mode === "organize" ? "Review rename" : queueRun.status === "queued" ? "Run this file" : queueRun.status === "interrupted" || queueRun.status === "needs_review" ? "Select to review" : "Select to retry"}</button>
                  {#if queueRun.status !== "skipped"}<button class="text-button queue-skip-action" type="button" disabled={!desktopShell || workflowBusy || Boolean(entry.queue_stale)} onclick={() => skipQueueEntry(entry, queueRun)}>Skip {queueRun.mode?.toUpperCase() ?? "task"}</button>{/if}
                </div>
              {/if}
            {/if}
            {#if page === "youtube" && youtubeEstimate && (!hasHdr10plusSignal(entry) || youtubeHdr10plusLossAccepted(entry.path))}<small class="file-cell-sub">Upload estimate {bytes(youtubeEstimate.estimated_output_bytes)} · {youtubeEstimate.video_bitrate_kbps} kb/s video · {youtubeEstimate.output_width}×{youtubeEstimate.output_height}</small>{/if}
            {#if page === "youtube" && entry.profile?.hdr.dolby_vision}<small class="row-error">YouTube export skips Dolby Vision sources.</small>{/if}
            {#if page === "youtube" && hasHdr10plusSignal(entry)}<label class="youtube-loss-consent"><input type="checkbox" checked={youtubeHdr10plusLossAccepted(entry.path)} disabled={workflowBusy} onchange={(event) => setYoutubeHdr10plusLossAccepted(entry.path, event.currentTarget.checked)} /><span><strong>Allow copy without HDR10+ dynamic metadata</strong><small>{youtubeHdr10plusLossAccepted(entry.path) ? "Loss approved for this file; static HDR10/PQ signaling remains." : youtubeHdr10plusConsentMessage(entry)}</small></span></label>{/if}
            {#if page === "youtube" && (entry.profile?.subtitle_count ?? 0) > 0}<small class="file-cell-sub">Source subtitles are omitted; add supported caption files separately.</small>{/if}
          </td></tr>
      {/each}
    </tbody></table></div>
    {#if page === "library" || page === "youtube"}
      <nav class="media-pagination" aria-label="Media file pages">
        <span>Showing {mediaPageFirst}–{mediaPageLast} of {visibleEntries.length} files · page {mediaPage} of {mediaPageCount}</span>
        <label>Rows per page<select aria-label="Rows per page" value={mediaPageSize} onchange={(event) => { mediaPageSize = Number(event.currentTarget.value); mediaPage = 1; }}><option value={50}>50</option><option value={100}>100</option><option value={200}>200</option></select></label>
        <div><button type="button" class="button secondary" disabled={workflowBusy || mediaPage <= 1} onclick={() => mediaPage = Math.max(1, mediaPage - 1)}>Previous</button><button type="button" class="button secondary" disabled={workflowBusy || mediaPage >= mediaPageCount} onclick={() => mediaPage = Math.min(mediaPageCount, mediaPage + 1)}>Next</button></div>
      </nav>
    {/if}
  {:else if page === "library" || page === "youtube"}
    <div class="media-empty-filter"><div><strong>{search ? "No files match this search." : "No media candidates found."}</strong>{#if !search}<p>Try including subfolders or lowering the minimum file size.</p>{/if}</div>{#if search}<button type="button" class="text-button" onclick={() => { search = ""; mediaPage = 1; }}>Clear search</button>{/if}</div>
  {:else}
    <div class="empty-state compact"><div class="empty-icon"><Search size={19} /></div><div><strong>{search ? "No matching files" : "No media candidates found"}</strong><p>{search ? "Try another filename or status." : "Try enabling subfolder scanning or lowering the minimum file size."}</p></div>{#if !search}<button class="text-button" onclick={onOpenLibrary}>Adjust scan options <ArrowRight size={14} /></button>{/if}</div>
  {/if}
{/snippet}

{#snippet EnvironmentPanel(environment: Environment | null)}
  <section id="environment-capabilities" class="environment-section"><div class="section-title-row"><div><div class="eyebrow">MACHINE AWARENESS</div><h2>Environment & capabilities</h2></div><button class="text-button" onclick={loadEnvironment} disabled={environmentLoading}><RefreshCw size={14} class={environmentLoading ? "spin" : ""} /> Recheck</button></div>
    {#if environment}
      <div class="environment-layout"><div class="hardware-card"><div class="hardware-topline"><span class="hardware-avatar"><Monitor size={18} /></span><span class="hardware-os">{environment.os.name} {environment.os.version}</span><span class="status-tag" class:status-ready={environment.os.supported} class:status-error={!environment.os.supported}>{environment.os.supported ? "SUPPORTED" : "UNSUPPORTED"}</span></div><h3>{environment.hardware.cpu_model}</h3><div class="hardware-specs"><span><Cpu size={15} /> {environment.hardware.logical_cpus} logical CPUs</span><span><MemoryStick size={15} /> {environment.hardware.memory_total_gib?.toFixed(1) ?? "—"} GiB effective RAM</span><span><HardDrive size={15} /> {environment.hardware.memory_available_gib?.toFixed(1) ?? "—"} GiB available</span><span><Database size={15} /> Swap {environment.hardware.swap_used_gib?.toFixed(1) ?? "—"}/{environment.hardware.swap_total_gib?.toFixed(1) ?? "—"} GiB used{environment.hardware.swap_used_percent !== null ? ` (${environment.hardware.swap_used_percent.toFixed(0)}%)` : ""} · {environment.hardware.swap_free_gib?.toFixed(1) ?? "—"} free</span></div>
          <div class="gpu-strip"><Zap size={15} /><span>{environment.hardware.gpu_devices.length ? environment.hardware.gpu_devices.join(" · ") : "No GPU device detected"}</span><small>DETECTED · NOT USED FOR ENCODING</small></div>
        </div><div class="tool-check-card"><div class="tool-check-heading"><strong>Toolchain health</strong><span>{environment.checks.filter((item) => item.available).length}/{environment.checks.length} ready</span></div><div class="tool-check-list">{#each environment.checks as check (check.key)}{@const packageUpdate = packageUpdateForTool(check.key)}{@const pathOverride = toolPathOptions.find((tool) => tool.key === check.key && Boolean(toolPathValue(tool.key).trim()))}<div class="tool-check-row"><span class="check-mark" class:check-bad={!check.available}>{#if check.available}<Check size={13} />{:else}<X size={13} />{/if}</span><span class="tool-check-copy"><strong>{check.label}</strong><small>{pathOverride ? `Custom override · ${check.path ?? toolPathValue(pathOverride.key)}` : check.path ?? check.detail}</small>{#if pathOverride}<small class="tool-override-note">This app uses the selected path; package updates do not replace it.</small>{/if}{#if packageUpdate}<small class="tool-update-version">APT {packageUpdate.installed_version} → {packageUpdate.candidate_version}</small>{/if}</span><span class="tool-version" class:tool-update-available={Boolean(packageUpdate)}>{#if packageUpdate}Update available{:else}{check.available ? check.version?.replace(/^.*?\s/, "") ?? "Ready" : check.required ? "Required" : "Optional"}{/if}</span></div>{/each}</div>
          {#if !environment.can_analyze && environment.checks.some((check) => check.key === "ffprobe" && check.available)}
            <div class="encoder-note" role="status"><Info size={15} /><span><strong>Media scans are available.</strong> FFprobe can read supported files now. Without MediaInfo, some track details stay partial; rebuild and YouTube jobs remain unavailable until their required tools are ready.</span></div>
          {/if}
        </div></div>
      <div class="tool-path-settings">
        <div class="tool-path-heading"><div><strong>Executable paths</strong><p>Leave a field blank to use PATH. A selected override stays saved and is never replaced by package setup; if FFprobe is blank, a matching FFmpeg sibling is detected when available.</p></div><button class="button secondary" disabled={!desktopShell || workflowBusy || environmentLoading} onclick={() => saveToolPaths()}><RefreshCw size={15} /> Save paths & check</button></div>
        <div class="tool-path-grid">
          {#each toolPathOptions as tool (tool.key)}
            <div class="tool-path-row"><label for={`tool-path-${tool.key}`}><span>{tool.label}</span><small>{tool.note}</small></label><div class="tool-path-control"><input id={`tool-path-${tool.key}`} type="text" value={toolPathValue(tool.key)} placeholder={`Use ${tool.key} from PATH`} disabled={!desktopShell || workflowBusy || environmentLoading} oninput={(event) => setToolPathValue(tool.key, event.currentTarget.value)} /><button class="text-button" disabled={!desktopShell || workflowBusy || environmentLoading} onclick={() => chooseToolPath(tool.key, tool.label)}>Browse</button>{#if toolPathValue(tool.key)}<button class="text-button" disabled={workflowBusy || environmentLoading} onclick={() => clearToolPath(tool.key)}>PATH</button>{/if}</div></div>
          {/each}
        </div>
      </div>
      <section class="apt-update-panel">
        <div class="section-title-row"><div><div class="eyebrow">SYSTEM TOOL SETUP</div><h3>Installed versions and updates</h3><p>Versions are checked at launch against the enabled Ubuntu APT sources. Package changes always require review and administrator authentication; repository versions may lag upstream.</p></div><div class="apt-action-buttons"><button class="button secondary" disabled={!desktopShell || workflowBusy || aptChecking || !environment.os.supported} onclick={() => checkUbuntuPackageUpdates()}>{#if aptChecking}<LoaderCircle class="spin" size={15} /> Checking…{:else}<RefreshCw size={15} /> Check versions{/if}</button><button class="button primary" disabled={!desktopShell || workflowBusy || aptChecking || !environment.os.supported} onclick={() => repairApt()}><Wrench size={15} /> Review setup & updates</button></div></div>
        {#if aptUpdates}
          <div class="apt-update-status"><span class="status-tag" class:status-warning={aptUpdates.catalog_stale} class:status-ready={!aptUpdates.catalog_stale}>{aptUpdates.catalog_stale ? "CATALOG NEEDS REFRESH" : `${aptUpdates.packages.length} UPDATE${aptUpdates.packages.length === 1 ? "" : "S"}`}</span><small>Catalog last updated: {aptUpdates.catalog_updated_at ? new Date(aptUpdates.catalog_updated_at * 1000).toLocaleString() : "unknown"} · checked {new Date(aptUpdates.checked_at * 1000).toLocaleString()}</small></div>
          {#if aptUpdates.catalog_stale}
            <div class="inline-warning apt-update-warning"><Info size={17} /><div><strong>These versions may be out of date.</strong><span>Review setup refreshes the configured APT catalog first, then shows the package list before installing. Refreshing downloads package-index metadata; it does not install software.</span></div><button class="button secondary" disabled={!desktopShell || workflowBusy || !environment.os.supported || !environment.checks.some((check) => check.key === "pkexec" && check.available)} onclick={() => refreshUbuntuPackageCatalog()}><RefreshCw size={15} /> Refresh catalog</button></div>
          {:else if aptUpdates.packages.length}
            <div class="apt-update-section-label"><strong>Updates available</strong><small>Installed version → enabled repository version</small></div>
            <div class="apt-update-list">{#each aptUpdates.packages as update (update.name)}<div><code>{update.name}</code><span>{update.installed_version}</span><ArrowRight size={13} /><strong>{update.candidate_version}</strong></div>{/each}</div>
          {:else}
            <div class="apt-no-updates"><CheckCircle2 size={16} /><span>No updates are available for the checked packages.</span></div>
          {/if}
          {#if environment.missing_apt_packages.length}<div class="apt-update-section-label"><strong>Missing packages</strong><small>Included in the same reviewed setup</small></div><div class="repair-package-list">{#each environment.missing_apt_packages as pkg (pkg)}<code>{pkg}</code>{/each}</div>{/if}
          {#if environment.checks.some((check) => check.key === "hdr10plus_tool" && !check.available)}<div class="repair-helper-note"><Info size={16} /><span><strong>HDR10+ helper missing.</strong> The setup action installs the checksum-verified upstream helper after APT finishes.</span></div>{/if}
        {:else}
          <p class="apt-update-empty">Checking Python, FFmpeg, MediaInfo, MKVToolNix, Fontconfig, and certificate packages against the local APT catalog…</p>
        {/if}
        <ToolRepairActivity showLogHistory={true} {repairSummary} {repairSummaryIsError} {repairId} {hdrInstalling} {repairToolOutcomes} {repairLogWarning} {repairLines} {toolSetupLogs} bind:selectedToolSetupLog {toolSetupLogLoading} bind:viewedToolSetupLog {toolSetupLogExporting} {toolSetupLogError} {openToolSetupLog} {exportToolSetupLog} />
      </section>
      <div class="encoder-note"><Info size={15} /><span>Hardware inventory is used for resource planning and diagnostics. The current rebuild engine uses <strong>CPU libx265</strong>; GPU encoding is not enabled.</span><button class="text-button" onclick={() => openPage("run")}>Resource settings <ArrowRight size={14} /></button></div>
    {:else}<div class="empty-state compact"><LoaderCircle class="spin" size={18} /><div><strong>{environmentLoading || booting ? "Checking this computer…" : "Could not read the environment"}</strong><p>{environmentError || "The desktop backend may be unavailable. Recheck after setup."}</p></div>{#if !environmentLoading}<button class="button secondary" onclick={loadEnvironment}>Retry check</button>{/if}</div>{/if}
  </section>
{/snippet}
