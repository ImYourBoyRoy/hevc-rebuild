<script lang="ts">
  import { onMount, tick } from "svelte";
  import { invoke, isTauri } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { Menu } from "@tauri-apps/api/menu";
  import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { currentMonitor, getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
  import ToolRepairConfirmation from "#lib/ToolRepairConfirmation.svelte";
  import { SETUP_CHECK_INTERVAL_MS, setupUpdateSummary } from "#lib/setup-updates.js";
  import RecoveryReview from "#lib/RecoveryReview.svelte";
  import { createWorkspaceState } from "#lib/workspace.svelte.js";
  import { RunEventBuffer, selectQueuePaths } from "#lib/workspace-state.js";
  import RunProgress from "#lib/RunProgress.svelte";
  import RunSettingsEditor from "#lib/RunSettingsEditor.svelte";
  import RunView from "#lib/views/RunView.svelte";
  import ShellSidebar from "#lib/views/ShellSidebar.svelte";
  import ShellHeader from "#lib/views/ShellHeader.svelte";
  import MediaTable from "#lib/views/MediaTable.svelte";
  import StopNowDialog from "#lib/views/StopNowDialog.svelte";
  import OutputPreviewDialog from "#lib/views/OutputPreviewDialog.svelte";
  import MetadataReviewDialog from "#lib/views/MetadataReviewDialog.svelte";
  import EnvironmentPanel from "#lib/views/EnvironmentPanel.svelte";
  import LibraryView from "#lib/views/LibraryView.svelte";
  import UpdateBanner from "#lib/UpdateBanner.svelte";
  import { ShieldCheck } from "@lucide/svelte";
  import type { OrganizerTemplates, MediaEntry, MediaQueueSource, MetadataLookupStatus, QueueMetadataSuggestion } from "#lib/types.js";
  import { fileName, queuePathKey } from "#lib/queue-paths.js";
  import { bytes, duration } from "#lib/format.js";
  import { createQueuePersistence } from "#lib/queue-persistence.js";
  import { createQueueController, type QueueHost } from "#lib/queue-controller.js";
  import { createRunOverrides } from "#lib/run-overrides.js";
  import { createWindowStateController, type WindowStateHost } from "#lib/window-state-controller.js";
  import { createLibraryCards } from "#lib/library-cards.js";
  import { AppState } from "#lib/state/app-state.svelte.js";
  import { ScanState } from "#lib/state/scan-state.svelte.js";
  import { ReviewState } from "#lib/state/review-state.svelte.js";
  import { LibraryUiState } from "#lib/state/library-ui-state.svelte.js";
  import { RunUiState } from "#lib/state/run-ui-state.svelte.js";
  import { YoutubeState } from "#lib/state/youtube-state.svelte.js";
  import { ReportsState } from "#lib/state/reports-state.svelte.js";
  import { defaultFilenameExcludePatterns, defaultOrganizerTemplates } from "#lib/state/defaults.js";
  import { composeHost, withGetters } from "#lib/state/bind-host.js";
  import { formatMetadataValue, hasHdr10plusSignal, mediaInfoLine, metadataSuggestsAnimation, profileSummary, statusLabel, streamIsSelectable, youtubeHdr10plusConsentMessage } from "#lib/entry-display.js";
  import { AppDerived } from "#lib/state/app-derived.svelte.js";
  import { createFileSettings } from "#lib/file-settings.js";
  import { createReportsController } from "#lib/reports-controller.js";
  import { createFilePickers } from "#lib/file-pickers.js";
  import { createToolPaths } from "#lib/tool-paths.js";
  import { createAppNavigation } from "#lib/app-navigation.js";
  import { createLibraryActions } from "#lib/library-actions.js";
  import { installAppEffects, type LocalAssetRecord } from "#lib/state/app-effects.svelte.js";
  import { CredentialsState } from "#lib/state/credentials-state.svelte.js";
  import { SettingsState } from "#lib/state/settings-state.svelte.js";
  import { ToolSetupState } from "#lib/state/tool-setup-state.svelte.js";
  import { ArtworkState } from "#lib/state/artwork-state.svelte.js";
  import { createAppInput, type AppInputHost } from "#lib/app-input.js";
  import { createLibraryRename, type LibraryRenameHost } from "#lib/library-rename.js";
  import { createAppBoot, type AppBootHost } from "#lib/app-boot.js";
  import { createCredentialsController } from "#lib/credentials-controller.js";
  import { createSettingsController } from "#lib/settings-controller.js";
  import { createArtworkController } from "#lib/artwork-controller.js";
  import { createToolSetupController } from "#lib/tool-setup-controller.js";
  import { createLibraryReviewController, type LibraryReviewHost } from "#lib/library-review.js";
  import { resolvedFileComment, type LibraryTvReviewTarget, type LocalLibraryRecord } from "#lib/review-notes.js";
  import { createRunLaunchController, type RunLaunchHost } from "#lib/run-launch.js";
  import { createRunController, type RunHost } from "#lib/run-controller.js";
  import { isFailure, isSuccess, outcomeLabel } from "#lib/outcomes.js";
  import { pageLabel } from "#lib/normalizers.js";
  import { themeAttributes } from "#lib/themes.js";
  import { createThemeTransition } from "#lib/theme-transition.js";
  import ThemeBackdrop from "#lib/ThemeBackdrop.svelte";
  import { queueRunStatusLabel } from "#lib/queue-run-state.js";
  import { createReviewFactories } from "#lib/review-factories.js";
  import { createPosterResolvers, providerPosterUrl, remoteArtworkUrl } from "#lib/poster-urls.js";





  const toolPathOptions = [
    { key: "ffmpeg", label: "FFmpeg", note: "Encoder and muxer" },
    { key: "ffprobe", label: "FFprobe", note: "Stream analysis" },
    { key: "mediainfo", label: "MediaInfo", note: "Track metadata" },
    { key: "mkvpropedit", label: "MKVToolNix editor", note: "Matroska tags and attachments" },
    { key: "mkvmerge", label: "MKVToolNix merger", note: "Matroska remuxing" },
    { key: "mkvextract", label: "MKVToolNix extractor", note: "Metadata and track extraction" },
  ] as const;
  const browserPreferencesKey = "hevc-rebuild-studio.preview-preferences.v1";

  const workspace = createWorkspaceState();
  const appState = new AppState();
  const scanState = new ScanState();
  const reviewState = new ReviewState();
  const libraryUi = new LibraryUiState();
  const runUi = new RunUiState();
  const youtubeState = new YoutubeState();
  const reportsState = new ReportsState();
  const credentials = new CredentialsState();
  credentials.setNotice = setNotice;
  const settingsState: SettingsState = withGetters(new SettingsState(), {
    appPreferences: () => appState.appPreferences, root: () => appState.root, activeRoot: () => workspace.queue.activeRoot, workflowBusy: () => derived.workflowBusy, settingsChanged: () => derived.settingsChanged,
  });
  settingsState.setNotice = setNotice;
  const toolSetup: ToolSetupState = withGetters(new ToolSetupState(), {
    aptHost: () => derived.aptHost, workflowBusy: () => derived.workflowBusy, scanning: () => scanState.scanning, runId: () => workspace.run.id, runStarting: () => workspace.run.starting,
  });
  toolSetup.setNotice = setNotice;
  toolSetup.currentToolPaths = () => currentToolPaths();
  const artwork: ArtworkState = withGetters(new ArtworkState(), { queueSources: () => workspace.queue.sources });
  artwork.setNotice = setNotice;
  const stateHost = composeHost(appState, scanState, reviewState, libraryUi, runUi, youtubeState, reportsState, credentials, settingsState, toolSetup, artwork);
  const derived: AppDerived = new AppDerived(
    { workspace, appState, scanState, reviewState, libraryUi, runUi, youtubeState, reportsState, credentials, settingsState, toolSetup, artwork },
    {
      hasHdr10plusSignal, youtubeHdr10plusLossAccepted: (path: string) => youtubeHdr10plusLossAccepted(path), youtubeEstimateFor: (entry: MediaEntry) => youtubeEstimateFor(entry),
      buildLibraryPosterCards: () => buildLibraryPosterCards(), buildLibraryCrumbs: () => buildLibraryCrumbs(), lookupMessageText: () => lookupMessageText(),
    },
  );
  const {
    youtubeEstimateFor, resourceForecastFor, setFileTuneOverride, settingsForEntry, effectiveContentTuneForEntry, fileTuneOverridesForBatch, defaultStreamIndices,
    selectedStreamIndices, runEntriesForFolder, runOverrideSummary, openRunSettingsEditor, streamIndicesForMode, applyRunSettingsOverride, clearRunSettingsOverride,
    setFileStreamSelected, selectedTrackCount, youtubeHdr10plusLossAccepted, setYoutubeHdr10plusLossAccepted,
  } = createFileSettings({
    host: composeHost({
      get queueSources() { return workspace.queue.sources; }, get activeRoot() { return workspace.queue.activeRoot; },
      get selectedRunEntries() { return derived.selectedRunEntries; }, get workflowBusy() { return derived.workflowBusy; }, setNotice,
    }, stateHost),
    actions: { reviewedSeriesForPath: (path: string) => reviewedSeriesForPath(path) },
  });
  const { deleteReportHistory, loadReports, recoverInterruptedSession, openLog, chooseReport, copyLog } = createReportsController({
    invoke,
    host: composeHost({ get runStates() { return workspace.queue.runStates; }, set runStates(value) { workspace.queue.runStates = value; }, setNotice }, stateHost),
    clipboard: { writeText: (value: string) => navigator.clipboard.writeText(value) },
  });
  const runEventBuffer = new RunEventBuffer();






  let metadataLookupClaimed = false;


  // These are deliberately session-only: they are sent with the next run and
  // never written into the persistent library queue or application defaults.

  const {
    localPosterUrl, displayPosterUrl, chosenPoster, seriesSuggestionArtwork,
    visiblePosterUrl, seasonArtworkPath, seasonPosterSources,
  } = createPosterResolvers(() => artwork.queueArtworkPreviews, () => libraryUi.libraryLocalPosters, () => reviewState.queueMatchSuggestions);
  const windowStateHost: WindowStateHost = composeHost({
    setNotice,
  }, stateHost);
  const { publishTheme, applyTheme, persistPreferences, restoreWindowState, scheduleWindowStateSave } = createWindowStateController({
    invoke, isDesktop: () => appState.desktopShell, host: windowStateHost,
    windowApi: { currentWindow: () => getCurrentWindow(), currentMonitor: () => currentMonitor(), logicalSize: (width, height) => new LogicalSize(width, height) },
    storage: { setItem: (key, value) => localStorage.setItem(key, value) },
    browserPreferencesKey,
    root: document.documentElement,
    themeTransition: createThemeTransition({
      document,
      prefersReducedMotion: () => window.matchMedia("(prefers-reduced-motion: reduce)").matches,
      settle: () => tick(),
      setTimeout: (callback, ms) => window.setTimeout(callback, ms),
      clearTimeout: (id) => window.clearTimeout(id),
    }),
  });
  const {
    refreshTmdbCredentialStatus, refreshTvdbCredentialStatus, saveTmdbCredential, revealTmdbCredential, toggleTmdbCredentialVisibility, removeTmdbCredential,
    saveTvdbCredential, revealTvdbCredentials, toggleTvdbCredentialVisibility, removeTvdbCredential,
  } = createCredentialsController({ invoke, isDesktop: () => appState.desktopShell, host: credentials });
  const {
    loadSettings, previewWorkDirectory, resetSettings, saveSettings, saveMetadataLanguageDraft, updateMetadataSources, addMetadataBlacklistPattern,
    removeMetadataBlacklistPattern, clearMetadataBlacklist, addFilenamePattern, removeFilenamePattern, clearFilenamePatterns,
  } = createSettingsController({ invoke, isDesktop: () => appState.desktopShell, host: settingsState, persistPreferences });
  const {
    loadEnvironment, packageUpdateForTool, checkUbuntuPackageUpdates, refreshUbuntuPackageCatalog, refreshToolSetupLogs, openToolSetupLog,
    exportToolSetupLog, reviewSetup, confirmSetup, checkSetupUpdates, handleRepairLine, handleRepairLogWarning, handleRepairFinished,
  } = createToolSetupController({ invoke, isDesktop: () => appState.desktopShell, host: toolSetup, dialogs: { save: (options) => saveDialog(options) } });
  const { openPage, navItemActive, openProgressDetails, openMetadataProviderSettings } = createAppNavigation({
    host: composeHost({ get runMode() { return workspace.run.mode; } }, stateHost),
    actions: { persistPreferences, tick },
    env: { getElementById: (id: string) => document.getElementById(id), requestAnimationFrame: (callback: FrameRequestCallback) => requestAnimationFrame(callback) },
  });
  const {
    copyHostSetupCommand, currentToolPaths, setToolPathValue, toolPathValue, applyToolPaths, saveToolPaths, chooseToolPath, clearToolPath, chooseFfmpeg, clearFfmpegOverride, openToolchainSettings,
  } = createToolPaths({
    host: composeHost({
      get runId() { return workspace.run.id; }, get runStarting() { return workspace.run.starting; }, get workflowBusy() { return derived.workflowBusy; },
      get hostSetupCommand() { return derived.hostSetupCommand; }, get hostSetupShell() { return derived.hostSetupShell; }, setNotice,
    }, stateHost),
    actions: { persistPreferences, loadEnvironment, openDialog: (options) => openDialog(options), openPage, tick },
    env: { clipboard: { writeText: (value: string) => navigator.clipboard.writeText(value) }, getElementById: (id: string) => document.getElementById(id) },
  });
  const {
    persistQueueSources, persistQueueRunStates, persistQueueSelection, persistQueueMovieReviews,
    persistQueueSeriesReviews, persistQueueMetadataSuggestions, scheduleQueueMetadataSuggestionSave,
  } = createQueuePersistence({
    invoke,
    isDesktop: () => appState.desktopShell,
    queue: workspace.queue,
    getFileTuneOverrides: () => runUi.fileTuneOverrides,
    getFileStreamOverrides: () => runUi.fileStreamOverrides,
    getMovieReviews: () => reviewState.queueMovieReviews,
    getSeriesReviews: () => reviewState.queueSeriesReviews,
    getMatchSuggestions: () => reviewState.queueMatchSuggestions,
  });
  // One typed view of the route's reactive state for the queue controller (see queue-controller.ts).
  const queueHost: QueueHost = composeHost({
    get metadataLookupClaimed() { return metadataLookupClaimed; }, set metadataLookupClaimed(value) { metadataLookupClaimed = value; },
    get settingsChanged() { return derived.settingsChanged; },
    get workflowBusy() { return derived.workflowBusy; },
    setNotice, persistPreferences, openPage, currentToolPaths, loadSettings, sourceNeedsMetadataLookup: (source: MediaQueueSource) => sourceNeedsMetadataLookup(source), recoverInterruptedSession,
  }, stateHost);
  const {
    activateQueueSource, removeQueueSource, scanQueueSource, addQueueSources, reorderQueueSources,
    moveQueueSource, reorderQueueFiles, moveQueueFile, removeQueueEntry, restoreExcludedQueueFiles, loadMediaQueue,
  } = createQueueController({ invoke, isDesktop: () => appState.desktopShell, workspace, host: queueHost, persistence: { persistQueueSources } });
  const { browseFolder, browseFile, chooseYoutubeOutput, chooseRebuildOutput, chooseFileOutput, clearFileOutput, analyzeFolder, refreshQueueSource, openFileLocation, cancelScan } = createFilePickers({
    invoke,
    host: composeHost({
      get activeRoot() { return workspace.queue.activeRoot; }, get runId() { return workspace.run.id; }, get runStarting() { return workspace.run.starting; }, get workflowBusy() { return derived.workflowBusy; }, setNotice,
    }, stateHost),
    actions: { openDialog: (options) => openDialog(options), addQueueSources, scanQueueSource, persistPreferences },
  });
  const { blankSeriesReview, seriesReviewStorageKey, episodeDraft } = createReviewFactories(() => derived.queuedEntries, () => reviewState.queueSeriesReviews, () => libraryUi.libraryLocalPosters);
  // One typed view of the route's reactive state for the library review controller (see library-review.ts).
  const { ensureQueueArtworkPreview, observeQueueArtwork, refreshArtworkCache, purgeUnusedArtworkCache } = createArtworkController({
    invoke, isDesktop: () => appState.desktopShell, host: artwork,
    createObserver: typeof IntersectionObserver === "undefined" ? null : (callback, options) => new IntersectionObserver(callback, options),
  });
  const libraryReviewHost: LibraryReviewHost = composeHost({
    get librarySeriesGroups() { return derived.librarySeriesGroups; },
    get queuedEntries() { return derived.queuedEntries; },
    get libraryPosterCards() { return derived.libraryPosterCards; },
    ensureQueueArtworkPreview, showLibraryReview: (target: LibraryTvReviewTarget) => showLibraryReview(target),
  }, stateHost);
  const {
    tvReviewForEpisode, seriesGroupForRoot,
    activeReviewPosterPath, openLibraryReview, reviewCurrentBrowse, setMovieComment, setReviewComment, selectReviewPoster, setPathsAnimated,
    patchSeriesReview, patchTvField, tvReviewHeading, tvReviewPaths, posterPathsForReview, reviewPosterChoices, sharedCommentFor, reviewedSeriesForPath,
  } = createLibraryReviewController({ host: libraryReviewHost, blankSeriesReview, seriesReviewStorageKey, episodeDraft, chosenPoster, seasonArtworkPath, displayPosterUrl });
  const { animationSelection, buildLibraryPosterCards, buildLibraryCrumbs, libraryLookupMessage: lookupMessageText } = createLibraryCards({
    getQueueMovieReviews: () => reviewState.queueMovieReviews, getQueueSeriesReviews: () => reviewState.queueSeriesReviews, getQueueMatchSuggestions: () => reviewState.queueMatchSuggestions,
    getQueueArtworkPreviews: () => artwork.queueArtworkPreviews, getLibraryLocalPosters: () => libraryUi.libraryLocalPosters, getAppPreferences: () => appState.appPreferences,
    getMetadataLookupStatus: () => reviewState.metadataLookupStatus, getQueuedEntries: () => derived.queuedEntries, getFileTuneOverrides: () => runUi.fileTuneOverrides,
    getSelectedPaths: () => workspace.queue.selectedPaths, getRunProgress: () => workspace.run.progress,
    getLibraryBrowse: () => libraryUi.libraryBrowse, setLibraryBrowse: (value) => { libraryUi.libraryBrowse = value; },
    getLibrarySeriesGroups: () => derived.librarySeriesGroups, getVisibleEntries: () => derived.visibleEntries, getCurrentPageEntries: () => derived.currentPageEntries,
    reviewedSeriesForPath, seriesReviewStorageKey, settingsForEntry, resourceForecastFor, queueRunStateForPath: (path) => queueRunStateForPath(path), effectiveContentTuneForEntry, statusLabel, profileSummary,
    chosenPoster, providerPosterUrl, remoteArtworkUrl, seriesSuggestionArtwork, visiblePosterUrl, seasonPosterSources, seasonArtworkPath, displayPosterUrl,
  });
  const { selectedPosterOverrides, selectedMetadataOverrides } = createRunOverrides({
    getMovieReviews: () => reviewState.queueMovieReviews,
    reviewedSeriesForPath,
    getLocalPosters: () => libraryUi.libraryLocalPosters,
    getSettingsComment: () => settingsState.settings?.metadata.comment,
    resolvedFileComment,
  });
  // One typed view of the route's reactive state for the run controller (see run-controller.ts).
  const runHost: RunHost = composeHost({
    get queuedEntries() { return derived.queuedEntries; },
    get runBatches() { return derived.runBatches; },
    get queueRunStatesByPath() { return derived.queueRunStatesByPath; },
    get workflowBusy() { return derived.workflowBusy; },
    setNotice, openPage, currentToolPaths, loadReports, selectedPosterOverrides, selectedMetadataOverrides, fileTuneOverridesForBatch, openLog, openMetadataForEntry: (entry: MediaEntry) => openMetadataForEntry(entry), chooseReport,
  }, stateHost);
  const {
    queueRunStateForPath, noteRunProgress, handleQueueRenameState, beginQueuedRun, finishRun,
    stopAfterCurrent, confirmStopNow, prepareQueueEntryForRetry, skipQueueEntry, openQueueRunReport, loadQueueRunDiagnostics,
  } = createRunController({
    invoke, workspace, host: runHost, runEventBuffer,
    persistence: { persistQueueRunStates, persistQueueSources },
    queue: { activateQueueSource },
  });
  // One typed view of the route's reactive state for the launch controller (see run-launch.ts).
  const runLaunchHost: RunLaunchHost = composeHost({
    get canStart() { return derived.canStart; },
    get unresolvedSelectedPaths() { return derived.unresolvedSelectedPaths; },
    get runBatches() { return derived.runBatches; },
    get queuedEntries() { return derived.queuedEntries; },
    get recoveryCurrentItem() { return derived.recoveryCurrentItem; },
    setNotice, loadReports, currentToolPaths, selectFilesToReview: () => selectFilesToReview(),
  }, stateHost);
  const { beginRun, confirmOutputPreview, inspectRecoveryTarget, finishRecoveryReview, openInterruptedRecovery } = createRunLaunchController({
    invoke, isDesktop: () => appState.desktopShell, workspace, host: runLaunchHost,
    persistence: { persistQueueRunStates },
    runs: { queueRunStateForPath, beginQueuedRun },
  });















  function setNotice(message: string, kind: "info" | "success" | "error" = "info") {
    workspace.notice.message = message;
    workspace.notice.kind = kind;
  }

  function claimMetadataLookup(): boolean {
    if (metadataLookupClaimed) return false;
    metadataLookupClaimed = true;
    return true;
  }

  function localRecordFromAsset(item: LocalAssetRecord): LocalLibraryRecord {
    return { artwork: item.artwork };
  }

  // The desktop webview does not open new windows, so web links go to the system browser.
  const libraryRenameHost: LibraryRenameHost = composeHost({
    get selectedPaths() { return workspace.queue.selectedPaths; }, set selectedPaths(value) { workspace.queue.selectedPaths = value; },
    get sources() { return workspace.queue.sources; }, set sources(value) { workspace.queue.sources = value; },
    get runStates() { return workspace.queue.runStates; }, set runStates(value) { workspace.queue.runStates = value; },
    get activeRoot() { return workspace.queue.activeRoot; }, set activeRoot(value) { workspace.queue.activeRoot = value; },
    get workflowBusy() { return derived.workflowBusy; },
    get queuedEntries() { return derived.queuedEntries; },
    setNotice,
  }, stateHost);
  const {
    openMetadataForEntry, closeMetadataDialog, openFullMetadataOrganizer, libraryRenameEntries, libraryRenameShape, insertRenameToken, resetRenamePattern,
    libraryRenameNote, applyLibraryRename, chooseLibraryTask, startMetadataOnlyFromOrganizer, handleQueuePathsRenamed,
  } = createLibraryRename({
    invoke, host: libraryRenameHost, constants: { defaultOrganizerTemplates },
    actions: {
      persistPreferences, persistQueueSources, openPage, beginRun: () => beginRun(), tick, ensureQueueArtworkPreview, seriesGroupForRoot, tvReviewForEpisode, posterPathsForReview,
      reviewedSeriesForPath,
    },
  });
  const {
    setVisibleSelection, setFilteredSelection, selectFilesToReview, sourceNeedsMetadataLookup, showLibraryReview, chooseReviewImage, openLibraryPoster, reviewMediaEntries, reviewLocationPath,
  } = createLibraryActions({
    host: composeHost({
      get selectedPaths() { return workspace.queue.selectedPaths; }, set selectedPaths(value) { workspace.queue.selectedPaths = value; },
      get libraryPosterCards() { return derived.libraryPosterCards; }, get visibleEntries() { return derived.visibleEntries; }, get currentPageEntries() { return derived.currentPageEntries; },
      get queuedEntries() { return derived.queuedEntries; }, get runReviewEntries() { return derived.runReviewEntries; }, get librarySeriesGroups() { return derived.librarySeriesGroups; }, setNotice,
    }, stateHost),
    actions: { queueRunStateForPath, reviewedSeriesForPath, seriesGroupForRoot, ensureQueueArtworkPreview, patchSeriesReview, openMetadataForEntry, openDialog: (options) => openDialog(options), tick },
  });
  const appInputHost: AppInputHost = composeHost({
    get mutatingRun() { return derived.mutatingRun; }, set mutatingRun(value) { derived.mutatingRun = value; },
    get canStart() { return derived.canStart; }, set canStart(value) { derived.canStart = value; },
    get workflowBusy() { return derived.workflowBusy; },
    get runId() { return workspace.run.id; },
    setNotice,
  }, stateHost);
  const { handleGlobalKeydown, handleAgentControlRequest, handleUtilityContextMenu, closeContextMenu } = createAppInput({
    host: appInputHost, pageLabel,
    actions: { openPage, stopAfterCurrent, beginRun, browseFolder, analyzeFolder, loadEnvironment, copyLog, tick },
    env: { document, window, navigator, KeyboardEvent, Menu, getCurrentWindow, invoke },
  });
  const appBootHost: AppBootHost = composeHost({
    get runLines() { return workspace.run.lines; }, set runLines(value) { workspace.run.lines = value; },
    get runResources() { return workspace.run.resources; }, set runResources(value) { workspace.run.resources = value; },
    get workflowBusy() { return derived.workflowBusy; },
    get runId() { return workspace.run.id; },
    get runStarting() { return workspace.run.starting; },
    get queueActiveRoot() { return workspace.queue.activeRoot; },
    setNotice,
  }, stateHost);
  const { startApp } = createAppBoot({
    invoke, host: appBootHost, runEventBuffer,
    constants: { browserPreferencesKey, defaultFilenameExcludePatterns, defaultOrganizerTemplates },
    env: { document, window, localStorage, isTauri, listen, getCurrentWindow, openUrl },
    actions: {
      restoreWindowState, publishTheme, applyToolPaths, persistPreferences, loadMediaQueue, loadEnvironment, loadSettings, loadReports, refreshToolSetupLogs,
      refreshTmdbCredentialStatus, refreshTvdbCredentialStatus, checkUbuntuPackageUpdates, checkSetupUpdates, noteRunProgress, finishRun, handleRepairLine, handleRepairLogWarning,
      handleRepairFinished, addQueueSources, openPage, scheduleWindowStateSave, handleAgentControlRequest, handleUtilityContextMenu, closeContextMenu,
    },
  });
  installAppEffects({ workspace, derived, appState, scanState, reviewState, libraryUi, runUi, invoke, localRecordFromAsset, seriesReviewStorageKey, ensureQueueArtworkPreview, reviewedSeriesForPath, seriesSuggestionArtwork, persistQueueSelection, persistQueueMovieReviews, persistQueueSeriesReviews, refreshArtworkCache });
  onMount(() => {
    const stopApp = startApp();
    const recheck = window.setInterval(() => void checkSetupUpdates(), SETUP_CHECK_INTERVAL_MS);
    return () => { window.clearInterval(recheck); stopApp(); };
  });
</script>

<svelte:window onkeydown={handleGlobalKeydown} />

<svelte:head>
  <title>HEVC Rebuild Studio</title>
  <meta name="description" content="A hardware-aware, safety-first desktop studio for HEVC media library rebuilds." />
</svelte:head>

{#snippet MetadataWorkbenchSurface(compactItemMode: boolean, backgroundWorker = false)}
  {#await import("#lib/MetadataWorkbench.svelte") then { default: MetadataWorkbench }}
  <MetadataWorkbench scanResult={scanState.scanResult} focusMediaPath={reviewState.metadataFocusPath} workbenchActive={backgroundWorker || (compactItemMode ? reviewState.metadataDialogOpen : appState.page === "metadata")} {compactItemMode} autoLookupPending={reviewState.metadataLookupPending} metadataAutoLookup={appState.appPreferences.metadataAutoLookup} metadataLanguage={appState.appPreferences.metadataLanguage} textBlacklistPatterns={settingsState.settings?.metadata.text_blacklist ?? []} savePosterBesideMedia={appState.appPreferences.savePosterBesideMedia} tmdbCredentialStatus={credentials.tmdbCredentialStatus} tvdbCredentialStatus={credentials.tvdbCredentialStatus} metadataSources={appState.appPreferences.metadataSources} queueSuggestions={reviewState.queueMatchSuggestions} onOpenSettings={openMetadataProviderSettings} onStartMetadataOnly={startMetadataOnlyFromOrganizer} onAutoLookupConsumed={() => reviewState.metadataLookupPending = false} onClaimAutoLookup={claimMetadataLookup} onLookupStatus={(status: MetadataLookupStatus) => {
    if (!status.sourceRoot || !workspace.queue.activeRoot || queuePathKey(status.sourceRoot) === queuePathKey(workspace.queue.activeRoot)) {
      reviewState.metadataLookupStatus = status;
      if (status.status !== "searching") void persistQueueMetadataSuggestions();
    }
  }} onMatchSuggestion={(mediaPath: string, suggestion: QueueMetadataSuggestion) => {
    reviewState.queueMatchSuggestions = { ...reviewState.queueMatchSuggestions, [mediaPath]: suggestion };
    scheduleQueueMetadataSuggestionSave();
  }} onPathsRenamed={handleQueuePathsRenamed} onRenameState={handleQueueRenameState} bind:selectedMediaPaths={workspace.queue.selectedPaths} bind:movieAssignments={reviewState.queueMovieReviews} bind:seriesAssignments={reviewState.queueSeriesReviews} bind:organizerTemplates={appState.appPreferences.organizerTemplates} onOrganizerTemplatesChange={(templates: OrganizerTemplates) => void persistPreferences({ organizerTemplates: templates })} animationForPath={(path: string) => { const entry = derived.queuedEntries.find((item) => item.path === path); if (!entry) return runUi.fileTuneOverrides[path] === "animation"; return effectiveContentTuneForEntry(entry, reviewState.queueMovieReviews[path] ?? reviewedSeriesForPath(path), settingsForEntry(entry)) === "animation"; }} onAnimationChange={(path: string, animated: boolean) => setFileTuneOverride(path, animated ? "animation" : "none")} />
  {/await}
{/snippet}



<div class="app-shell" data-theme={themeAttributes(appState.theme).theme} data-palette={appState.theme} data-instrument={derived.instrumentTheme ? appState.theme : undefined}>
  <ShellSidebar environment={toolSetup.environment} environmentLoading={toolSetup.environmentLoading} booting={appState.booting} root={appState.root} failedCount={derived.failedRows.length} {navItemActive} {openPage} {browseFolder} {fileName} />

  <main class="main-panel">
    <ThemeBackdrop theme={appState.theme} />
    <ShellHeader page={appState.page} theme={appState.theme} environment={toolSetup.environment} environmentLoading={toolSetup.environmentLoading} booting={appState.booting} setupGapDetail={derived.setupGapDetail} setupGapLabel={derived.setupGapLabel} notice={workspace.notice} updateLabel={setupUpdateSummary(toolSetup.setupUpdates)?.label ?? ""} updateDetail={setupUpdateSummary(toolSetup.setupUpdates)?.detail ?? ""} reviewUpdates={reviewSetup} {applyTheme} {openPage} {openToolchainSettings} {loadEnvironment} dismissNotice={() => (workspace.notice.message = "")} />

    <UpdateBanner blocked={derived.workflowBusy} />

    <RunProgress run={workspace.run} encodeMeasured={derived.encodeMeasured} overallRunPercent={derived.overallRunPercent} progressElapsedLabel={derived.progressElapsedLabel} batchCount={runUi.pendingRunBatches.length} batchIndex={runUi.activeRunBatchIndex} {bytes} {duration} {openProgressDetails} {stopAfterCurrent} stopNow={appState.desktopShell && derived.mutatingRun ? () => (runUi.abortConfirmOpen = true) : undefined} />

    <div class="page-wrap" class:page-wrap-library={appState.page === "library" || appState.page === "home"}>
      {#if appState.page === "help"}
        {#await import("#lib/HelpCenter.svelte") then { default: HelpCenter }}
          <HelpCenter />
        {/await}
      {:else if appState.page === "settings"}
        {#await import("#lib/SettingsPreferences.svelte") then { default: SettingsPreferences }}
        <SettingsPreferences preferenceStorage={appState.preferenceStorage} bind:settings={settingsState.settings} settingsChanged={derived.settingsChanged} workflowBusy={derived.workflowBusy} appPreferences={appState.appPreferences} bind:metadataLanguageDraft={settingsState.metadataLanguageDraft} tmdbCredentialStatus={credentials.tmdbCredentialStatus} tvdbCredentialStatus={credentials.tvdbCredentialStatus} desktopShell={appState.desktopShell} bind:tmdbTokenDraft={credentials.tmdbTokenDraft} bind:tmdbTokenVisible={credentials.tmdbTokenVisible} tmdbCredentialBusy={credentials.tmdbCredentialBusy} bind:tvdbApiKeyDraft={credentials.tvdbApiKeyDraft} bind:tvdbCredentialsVisible={credentials.tvdbCredentialsVisible} tvdbCredentialBusy={credentials.tvdbCredentialBusy} bind:tvdbSubscriberPinDraft={credentials.tvdbSubscriberPinDraft} bind:tvdbAccessConfirmed={credentials.tvdbAccessConfirmed} artworkCacheLoading={artwork.artworkCacheLoading} artworkCachePurging={artwork.artworkCachePurging} artworkCacheInfo={artwork.artworkCacheInfo} artworkCacheError={artwork.artworkCacheError} bind:artworkCacheConfirm={artwork.artworkCacheConfirm} bind:newFilenameIncludePattern={settingsState.newFilenameIncludePattern} bind:newFilenameExcludePattern={settingsState.newFilenameExcludePattern} filenameFilterError={settingsState.filenameFilterError} bind:newMetadataBlacklistPattern={settingsState.newMetadataBlacklistPattern} metadataBlacklistError={settingsState.metadataBlacklistError} workDirectory={settingsState.workDirectory} {previewWorkDirectory} {saveSettings} {persistPreferences} {saveMetadataLanguageDraft} {updateMetadataSources} {openMetadataProviderSettings} {toggleTmdbCredentialVisibility} {saveTmdbCredential} {removeTmdbCredential} {toggleTvdbCredentialVisibility} {saveTvdbCredential} {removeTvdbCredential} {refreshArtworkCache} {bytes} {purgeUnusedArtworkCache} {addFilenamePattern} {clearFilenamePatterns} {removeFilenamePattern} {addMetadataBlacklistPattern} {clearMetadataBlacklist} {removeMetadataBlacklistPattern}>
          {#snippet environmentPanel()}<EnvironmentPanel environment={toolSetup.environment} environmentLoading={toolSetup.environmentLoading} environmentError={toolSetup.environmentError} booting={appState.booting} desktopShell={appState.desktopShell} workflowBusy={derived.workflowBusy} settings={settingsState.settings} aptHost={derived.aptHost} aptChecking={toolSetup.aptChecking} aptUpdates={toolSetup.aptUpdates} packageManagerLabel={derived.packageManagerLabel} hostSetupShell={derived.hostSetupShell} hostSetupCommand={derived.hostSetupCommand} repairId={toolSetup.repairId} hdrInstalling={toolSetup.hdrInstalling} repairSummary={toolSetup.repairSummary} repairSummaryIsError={toolSetup.repairSummaryIsError} repairLogWarning={toolSetup.repairLogWarning} repairLines={toolSetup.repairLines} repairToolOutcomes={toolSetup.repairToolOutcomes} toolSetupLogs={toolSetup.toolSetupLogs} toolSetupLogLoading={toolSetup.toolSetupLogLoading} toolSetupLogError={toolSetup.toolSetupLogError} toolSetupLogExporting={toolSetup.toolSetupLogExporting} {toolPathOptions} {toolPathValue} {setToolPathValue} {chooseToolPath} {clearToolPath} {saveToolPaths} {loadEnvironment} {checkUbuntuPackageUpdates} {refreshUbuntuPackageCatalog} {reviewSetup} {packageUpdateForTool} {openToolSetupLog} {exportToolSetupLog} {copyHostSetupCommand} {openPage} bind:selectedToolSetupLog={toolSetup.selectedToolSetupLog} bind:viewedToolSetupLog={toolSetup.viewedToolSetupLog} />{/snippet}
        </SettingsPreferences>
        {/await}

      {:else if appState.page === "metadata"}
        <!-- The record workbench is mounted below so scans can look up matches from the queue. -->
      {:else if appState.page === "library" || appState.page === "home"}
        <LibraryView bind:root={appState.root} bind:recursive={appState.recursive} bind:minimumSizeMib={appState.minimumSizeMib} bind:search={libraryUi.search} bind:mediaPage={libraryUi.mediaPage} bind:mediaPageSize={libraryUi.mediaPageSize} bind:renameDraft={libraryUi.renameDraft} bind:libraryRenameOpen={libraryUi.libraryRenameOpen} bind:libraryRenameDialog={libraryUi.libraryRenameDialog} bind:moviePatternField={libraryUi.moviePatternField} bind:episodePatternField={libraryUi.episodePatternField} bind:libraryRenameError={libraryUi.libraryRenameError} {workspace} mode={runUi.mode} desktopShell={appState.desktopShell} workflowBusy={derived.workflowBusy} scanning={scanState.scanning} scanProgress={scanState.scanProgress} scanResult={scanState.scanResult} modeToolsReady={derived.modeToolsReady} libraryDropActive={libraryUi.libraryDropActive} analysisErrors={derived.analysisErrors} runReviewEntries={derived.runReviewEntries} unresolvedSelectedPaths={derived.unresolvedSelectedPaths} selectionHasNoRunnableFiles={derived.selectionHasNoRunnableFiles} selectedRunEntries={derived.selectedRunEntries} staleSelectedEntries={derived.staleSelectedEntries} staleRunSources={derived.staleRunSources} libraryPosterCards={derived.libraryPosterCards} selectedVisibleCount={derived.selectedVisibleCount} queueSelectionCount={derived.queueSelectionCount} libraryLookupMessage={derived.libraryLookupMessage} metadataLookupStatus={reviewState.metadataLookupStatus} libraryCrumbs={derived.libraryCrumbs} libraryBrowse={libraryUi.libraryBrowse} visibleEntries={derived.visibleEntries} mediaPageCount={derived.mediaPageCount} mediaPageFirst={derived.mediaPageFirst} mediaPageLast={derived.mediaPageLast} libraryRenameBusy={libraryUi.libraryRenameBusy} {libraryRenameEntries} {libraryRenameShape} {libraryRenameNote} {fileName} {queueRunStateForPath} {persistPreferences} {browseFolder} {browseFile} {analyzeFolder} {openPage} {openInterruptedRecovery} {refreshQueueSource} {scanQueueSource} {activateQueueSource} {removeQueueSource} {cancelScan} {reorderQueueSources} {moveQueueSource} {removeQueueEntry} {reorderQueueFiles} {moveQueueFile} {restoreExcludedQueueFiles} {reviewCurrentBrowse} {observeQueueArtwork} {setPathsAnimated} {openLibraryPoster} {openFileLocation} {openLibraryReview} {openRunSettingsEditor} {chooseLibraryTask} {chooseFileOutput} {clearFileOutput} {setVisibleSelection} {insertRenameToken} {resetRenamePattern} {applyLibraryRename} toggleSelect={(paths: string[], selected: boolean) => { workspace.queue.selectedPaths = selectQueuePaths(workspace.queue.selectedPaths, paths, selected); }} keepReadyFiles={() => workspace.queue.selectedPaths = workspace.queue.selectedPaths.filter((path) => derived.selectedRunEntries.some((entry) => entry.path === path))} />

      {:else if appState.page === "youtube"}
        {#await import("#lib/YouTubePreparation.svelte") then { default: YouTubePreparation }}
        <YouTubePreparation queue={workspace.queue} run={workspace.run} bind:root={appState.root} bind:recursive={appState.recursive} bind:minimumSizeMib={appState.minimumSizeMib} bind:youtubeResolution={youtubeState.youtubeResolution} bind:youtubeQualityMode={youtubeState.youtubeQualityMode} bind:youtubeSourceMultiplier={youtubeState.youtubeSourceMultiplier} bind:youtubeMode={youtubeState.youtubeMode} bind:youtubeSdrCodec={youtubeState.youtubeSdrCodec} bind:youtubePreset={youtubeState.youtubePreset} bind:youtubeShrink={youtubeState.youtubeShrink} bind:youtubeQualityCheck={youtubeState.youtubeQualityCheck} bind:youtubeOutputDir={youtubeState.youtubeOutputDir} bind:search={libraryUi.search} bind:mediaPage={libraryUi.mediaPage} scanResult={scanState.scanResult} scanProgress={scanState.scanProgress} scanning={scanState.scanning} workflowBusy={derived.workflowBusy} desktopShell={appState.desktopShell} youtubeEstimateSummary={derived.youtubeEstimateSummary} analysisErrors={derived.analysisErrors} toolsRequiredEntries={derived.toolsRequiredEntries} selectedRunEntries={derived.selectedRunEntries} visibleEntries={derived.visibleEntries} selectedVisibleCount={derived.selectedVisibleCount} selectedFilteredCount={derived.selectedFilteredCount} mediaPageSize={libraryUi.mediaPageSize} selectedRunInputBytes={derived.selectedRunInputBytes} selectedMediaPathsForRun={derived.selectedMediaPathsForRun} modeToolsReady={derived.modeToolsReady} canStart={derived.canStart} progressElapsedLabel={derived.progressElapsedLabel} {bytes} {fileName} {persistPreferences} {browseFolder} {browseFile} {analyzeFolder} {chooseYoutubeOutput} {openPage} {setFilteredSelection} {stopAfterCurrent} stopNow={() => (runUi.abortConfirmOpen = true)} {beginRun} onCancelScan={cancelScan}>
          {#snippet mediaTable()}<MediaTable entries={derived.currentPageEntries} currentPageEntries={derived.currentPageEntries} onOpenLibrary={() => {}} bind:visibleSelectAllCheckbox={libraryUi.visibleSelectAllCheckbox} bind:mediaPage={libraryUi.mediaPage} bind:mediaPageSize={libraryUi.mediaPageSize} {workspace} page={appState.page} mode={runUi.mode} appPreferences={appState.appPreferences} desktopShell={appState.desktopShell} scanning={scanState.scanning} workflowBusy={derived.workflowBusy} settings={settingsState.settings} fileTuneOverrides={runUi.fileTuneOverrides} queueArtworkPreviewBusy={artwork.queueArtworkPreviewBusy} queueArtworkPreviewErrors={artwork.queueArtworkPreviewErrors} queueArtworkPreviews={artwork.queueArtworkPreviews} queueMatchSuggestions={reviewState.queueMatchSuggestions} queueMovieReviews={reviewState.queueMovieReviews} queueRunLogs={libraryUi.queueRunLogs} search={libraryUi.search} selectedVisibleCount={derived.selectedVisibleCount} visibleEntries={derived.visibleEntries} mediaPageCount={derived.mediaPageCount} mediaPageFirst={derived.mediaPageFirst} mediaPageLast={derived.mediaPageLast} youtubeHdr10plusHelperReady={derived.youtubeHdr10plusHelperReady} {chooseFileOutput} {clearFileOutput} {effectiveContentTuneForEntry} {formatMetadataValue} {hasHdr10plusSignal} {loadQueueRunDiagnostics} {metadataSuggestsAnimation} {observeQueueArtwork} {openMetadataForEntry} {openQueueRunReport} {prepareQueueEntryForRetry} {profileSummary} {queueRunStateForPath} {resourceForecastFor} {reviewedSeriesForPath} {selectedStreamIndices} {selectedTrackCount} {setFileStreamSelected} {setFileTuneOverride} {setVisibleSelection} {setYoutubeHdr10plusLossAccepted} {settingsForEntry} {skipQueueEntry} {statusLabel} {streamIsSelectable} {youtubeEstimateFor} {youtubeHdr10plusConsentMessage} {youtubeHdr10plusLossAccepted} toggleEntrySelection={(path: string, selected: boolean) => { workspace.queue.selectedPaths = selected ? [...new Set([...workspace.queue.selectedPaths, path])] : workspace.queue.selectedPaths.filter((item) => item !== path); }} />{/snippet}
        </YouTubePreparation>
        {/await}

      {:else if appState.page === "run"}
        <RunView bind:mode={runUi.mode} bind:integrityLevel={runUi.integrityLevel} bind:stopOnError={runUi.stopOnError} bind:settings={settingsState.settings} bind:runLaunchButton={runUi.runLaunchButton} {workspace} appPreferences={appState.appPreferences} preferenceStorage={appState.preferenceStorage} environment={toolSetup.environment} desktopShell={appState.desktopShell} workflowBusy={derived.workflowBusy} mutatingRun={derived.mutatingRun} canStart={derived.canStart} modeToolsReady={derived.modeToolsReady} scanResult={scanState.scanResult} root={appState.root} settingsChanged={derived.settingsChanged} settingsPath={settingsState.settingsPath} settingsSources={settingsState.settingsSources} forecastOutdated={derived.forecastOutdated} fileSettingsOverrides={runUi.fileSettingsOverrides} runBatches={derived.runBatches} runPlanPreviewByRoot={derived.runPlanPreviewByRoot} selectedRunEntries={derived.selectedRunEntries} selectedMediaPathsForRun={derived.selectedMediaPathsForRun} selectedRunInputBytes={derived.selectedRunInputBytes} selectionInOtherSources={derived.selectionInOtherSources} unresolvedSelectedPaths={derived.unresolvedSelectedPaths} staleSelectedEntries={derived.staleSelectedEntries} staleRunSources={derived.staleRunSources} progressElapsedLabel={derived.progressElapsedLabel} {bytes} {fileName} {openPage} {beginRun} {stopAfterCurrent} onStopNow={() => (runUi.abortConfirmOpen = true)} keepReadyFiles={() => workspace.queue.selectedPaths = workspace.queue.selectedPaths.filter((path) => derived.selectedRunEntries.some((entry) => entry.path === path))} {chooseRebuildOutput} {persistPreferences} {openRunSettingsEditor} {clearRunSettingsOverride} {runEntriesForFolder} {runOverrideSummary} {refreshQueueSource} {loadSettings} {resetSettings} {saveSettings} />

      {:else if appState.page === "reports"}
        {#await import("#lib/ReportBrowser.svelte") then { default: ReportBrowser }}
        <ReportBrowser repairId={toolSetup.repairId} hdrInstalling={toolSetup.hdrInstalling} reportDeleteBusy={reportsState.reportDeleteBusy} bind:reportDeleteConfirm={reportsState.reportDeleteConfirm} failedRows={derived.failedRows} {loadReports} {deleteReportHistory} reports={reportsState.reports} activeReport={derived.activeReport} logLoading={reportsState.logLoading} selectedLog={reportsState.selectedLog} {fileName} {bytes} {isFailure} {isSuccess} {outcomeLabel} {chooseReport} {openLog} {copyLog} {openPage} hasScan={Boolean(scanState.scanResult)} />
        {/await}

      {/if}

      {#if appState.page === "metadata"}
        <div class="metadata-workbench-slot active">{@render MetadataWorkbenchSurface(false)}</div>
      {/if}

      {#if scanState.scanResult && appState.appPreferences.metadataAutoLookup && appState.page !== "metadata"}
        <div class="metadata-discovery-worker" aria-hidden="true" hidden>
          {@render MetadataWorkbenchSurface(false, true)}
        </div>
      {/if}

      <MetadataReviewDialog open={reviewState.metadataDialogOpen} bind:metadataDialogElement={reviewState.metadataDialogElement} metadataWorkbenchSurface={MetadataWorkbenchSurface} scanResult={scanState.scanResult} metadataFocusPath={reviewState.metadataFocusPath} libraryTvReview={reviewState.libraryTvReview} appPreferences={appState.appPreferences} settings={settingsState.settings} queueMovieReviews={reviewState.queueMovieReviews} queueSeriesReviews={reviewState.queueSeriesReviews} queueMatchSuggestions={reviewState.queueMatchSuggestions} {reviewedSeriesForPath} {seriesReviewStorageKey} {seriesGroupForRoot} {episodeDraft} {tvReviewHeading} {tvReviewPaths} {animationSelection} {activeReviewPosterPath} {displayPosterUrl} {reviewPosterChoices} {sharedCommentFor} {reviewMediaEntries} {reviewLocationPath} {mediaInfoLine} {formatMetadataValue} {patchTvField} {setReviewComment} {selectReviewPoster} {chooseReviewImage} {setPathsAnimated} {setMovieComment} {openFileLocation} {openFullMetadataOrganizer} onClose={() => { reviewState.metadataDialogOpen = false; reviewState.libraryTvReview = null; }} />

      <footer class="page-footer"><span><ShieldCheck size={14} /> Local processing · Originals protected</span><span>HEVC Rebuild Studio <i>•</i> Cross-platform desktop</span></footer>
    </div>
  </main>

<OutputPreviewDialog open={runUi.outputPreviewOpen} bind:fullAck={runUi.fullAck} outputPreview={runUi.outputPreview} outputPreviewBusy={runUi.outputPreviewBusy} canStart={derived.canStart} runLaunchButton={runUi.runLaunchButton} runBatches={derived.runBatches} selectedRunEntries={derived.selectedRunEntries} selectedMediaPathsForRun={derived.selectedMediaPathsForRun} selectedRunDiskBudget={derived.selectedRunDiskBudget} selectedRunFreeBytes={derived.selectedRunFreeBytes} onClose={() => (runUi.outputPreviewOpen = false)} adjustOutputFolders={() => { runUi.outputPreviewOpen = false; openPage("run"); }} onConfirm={confirmOutputPreview} />

<StopNowDialog open={runUi.abortConfirmOpen} {workspace} onClose={() => (runUi.abortConfirmOpen = false)} onConfirm={() => void confirmStopNow()} />

<RecoveryReview returnFocus={runUi.runLaunchButton} recoveryReviewOpen={runUi.recoveryReviewOpen} recoveryCurrentItem={derived.recoveryCurrentItem} recoveryReviewIndex={runUi.recoveryReviewIndex} recoveryReportStatus={derived.recoveryReportStatus} recoveryProbeEntry={runUi.recoveryProbeEntry} recoveryProbeError={runUi.recoveryProbeError} recoveryProbeAttempted={runUi.recoveryProbeAttempted} desktopShell={appState.desktopShell} recoveryProbeBusy={runUi.recoveryProbeBusy} workflowBusy={derived.workflowBusy} bind:recoveryAcknowledged={runUi.recoveryAcknowledged} recoveryCanMarkComplete={derived.recoveryCanMarkComplete} {queueRunStatusLabel} {inspectRecoveryTarget} {finishRecoveryReview} itemCount={runUi.recoveryReviewItems.length} onClose={() => runUi.recoveryReviewOpen = false} />

  <RunSettingsEditor
    open={runUi.runSettingsEditorOpen}
    entries={derived.selectedRunEntries.filter((entry) => runUi.runSettingsEditorPaths.some((path) => queuePathKey(path) === queuePathKey(entry.path)))}
    overrides={runUi.fileSettingsOverrides}
    settings={settingsState.settings}
    busy={derived.workflowBusy}
    {fileName}
    selectedStreams={(entry: MediaEntry) => runUi.fileSettingsOverrides[entry.path]?.stream_indices ?? selectedStreamIndices(entry)}
    {streamIndicesForMode}
    onApply={applyRunSettingsOverride}
    onClear={clearRunSettingsOverride}
    onClose={() => runUi.runSettingsEditorOpen = false}
  />

<ToolRepairConfirmation open={toolSetup.setupConfirmOpen} plan={toolSetup.setupPlan} workflowBusy={derived.workflowBusy} {confirmSetup} onClose={() => toolSetup.setupConfirmOpen = false} />
</div>
