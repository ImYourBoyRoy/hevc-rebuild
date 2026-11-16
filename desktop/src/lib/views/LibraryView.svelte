<!-- ./desktop/src/lib/views/LibraryView.svelte
     The Library page: scan options, queue status, warnings, poster grid,
     rename dialog, pagination and the empty state. Pure view; the route owns
     all state and every action is a callback. -->
<script lang="ts">
  import type { ComponentProps } from "svelte";
  import { AlertTriangle, FolderOpen, Layers, Search, Wrench } from "@lucide/svelte";
  import LibraryPosterGrid from "../LibraryPosterGrid.svelte";
  import LibraryQueueStatus from "../LibraryQueueStatus.svelte";
  import LibraryRenameDialog from "../LibraryRenameDialog.svelte";
  import type { createWorkspaceState } from "../workspace.svelte.js";
  import type { SourceRunBatch } from "../run-controller.js";
  import type { MediaEntry, MediaQueueRunState, MetadataLookupStatus, OrganizerTemplates, Page, RunMode, ScanProgress, ScanResult, LibraryCrumb, LibraryPosterCard } from "../types.js";

  type GridProps = ComponentProps<typeof LibraryPosterGrid>;
  type QueueStatusProps = ComponentProps<typeof LibraryQueueStatus>;
  type RenameProps = ComponentProps<typeof LibraryRenameDialog>;

  interface Props {
    root: string;
    recursive: boolean;
    minimumSizeMib: string;
    search: string;
    mediaPage: number;
    mediaPageSize: number;
    renameDraft: OrganizerTemplates;
    libraryRenameOpen: boolean;
    libraryRenameDialog: HTMLDialogElement | null;
    moviePatternField: HTMLInputElement | null;
    episodePatternField: HTMLInputElement | null;
    libraryRenameError: string;
    workspace: ReturnType<typeof createWorkspaceState>;
    mode: RunMode;
    desktopShell: boolean;
    workflowBusy: boolean;
    scanning: boolean;
    scanProgress: ScanProgress | null;
    scanResult: ScanResult | null;
    modeToolsReady: boolean | undefined;
    libraryDropActive: boolean;
    analysisErrors: MediaEntry[];
    runReviewEntries: MediaEntry[];
    unresolvedSelectedPaths: number;
    selectionHasNoRunnableFiles: boolean;
    selectedRunEntries: MediaEntry[];
    staleSelectedEntries: MediaEntry[];
    staleRunSources: SourceRunBatch[];
    libraryPosterCards: LibraryPosterCard[];
    selectedVisibleCount: number;
    queueSelectionCount: number;
    libraryLookupMessage: string;
    metadataLookupStatus: MetadataLookupStatus | null;
    libraryCrumbs: LibraryCrumb[];
    libraryBrowse: { seriesRoot: string; season: number | null } | null;
    visibleEntries: MediaEntry[];
    mediaPageCount: number;
    mediaPageFirst: number;
    mediaPageLast: number;
    libraryRenameBusy: boolean;
    libraryRenameEntries: RenameProps["libraryRenameEntries"];
    libraryRenameShape: RenameProps["libraryRenameShape"];
    libraryRenameNote: RenameProps["libraryRenameNote"];
    fileName: (path: string) => string;
    queueRunStateForPath: (path: string) => MediaQueueRunState | undefined;
    persistPreferences: (patch: Record<string, unknown>) => unknown;
    browseFolder: () => unknown;
    browseFile: () => unknown;
    analyzeFolder: () => unknown;
    openPage: (page: Page) => void;
    openInterruptedRecovery: () => unknown;
    refreshQueueSource: (sourceRoot: string) => unknown;
    scanQueueSource: (path: string, activate: boolean, refresh: boolean) => unknown;
    activateQueueSource: QueueStatusProps["onActivate"];
    removeQueueSource: QueueStatusProps["onRemove"];
    cancelScan: () => unknown;
    reorderQueueSources: NonNullable<QueueStatusProps["onReorderSources"]>;
    moveQueueSource: NonNullable<QueueStatusProps["onMoveSource"]>;
    removeQueueEntry: (path: string) => void;
    reorderQueueFiles: NonNullable<QueueStatusProps["onReorderFiles"]>;
    moveQueueFile: NonNullable<QueueStatusProps["onMoveFile"]>;
    restoreExcludedQueueFiles: NonNullable<QueueStatusProps["onRestoreExcluded"]>;
    reviewCurrentBrowse: () => void;
    observeQueueArtwork: GridProps["observeArtwork"];
    setPathsAnimated: GridProps["onToggleAnimated"];
    openLibraryPoster: GridProps["onOpenReview"];
    openFileLocation: (path: string) => unknown;
    openLibraryReview: GridProps["onReview"];
    openRunSettingsEditor: GridProps["onAdjustRun"];
    chooseLibraryTask: (task: "organize" | "metadata" | "rebuild" | "integrity") => void;
    chooseFileOutput: GridProps["onSetOutput"];
    clearFileOutput: GridProps["onClearOutput"];
    setVisibleSelection: GridProps["onSelectPage"];
    insertRenameToken: RenameProps["insertRenameToken"];
    resetRenamePattern: RenameProps["resetRenamePattern"];
    applyLibraryRename: RenameProps["applyLibraryRename"];
    toggleSelect: GridProps["onToggleSelect"];
    keepReadyFiles: () => void;
  }
  let {
    root = $bindable(), recursive = $bindable(), minimumSizeMib = $bindable(), search = $bindable(), mediaPage = $bindable(), mediaPageSize = $bindable(),
    renameDraft = $bindable(), libraryRenameOpen = $bindable(), libraryRenameDialog = $bindable(), moviePatternField = $bindable(), episodePatternField = $bindable(),
    libraryRenameError = $bindable(),
    workspace, mode, desktopShell, workflowBusy, scanning, scanProgress, scanResult, modeToolsReady, libraryDropActive, analysisErrors, runReviewEntries,
    unresolvedSelectedPaths, selectionHasNoRunnableFiles, selectedRunEntries, staleSelectedEntries, staleRunSources, libraryPosterCards, selectedVisibleCount,
    queueSelectionCount, libraryLookupMessage, metadataLookupStatus, libraryCrumbs, libraryBrowse, visibleEntries, mediaPageCount, mediaPageFirst, mediaPageLast,
    libraryRenameBusy, libraryRenameEntries, libraryRenameShape, libraryRenameNote, fileName, queueRunStateForPath, persistPreferences, browseFolder, browseFile,
    analyzeFolder, openPage, openInterruptedRecovery, refreshQueueSource, scanQueueSource, activateQueueSource, removeQueueSource, cancelScan, reorderQueueSources,
    moveQueueSource, removeQueueEntry, reorderQueueFiles, moveQueueFile, restoreExcludedQueueFiles, reviewCurrentBrowse, observeQueueArtwork, setPathsAnimated,
    openLibraryPoster, openFileLocation, openLibraryReview, openRunSettingsEditor, chooseLibraryTask, chooseFileOutput, clearFileOutput, setVisibleSelection,
    insertRenameToken, resetRenamePattern, applyLibraryRename, toggleSelect, keepReadyFiles,
  }: Props = $props();
</script>

<section class="library-toolbar" class:card-surface={libraryDropActive} class:drag-active={libraryDropActive} class:library-toolbar-idle={!libraryDropActive}>
{#if libraryDropActive}
  <div class="library-drop-hint" aria-live="polite"><div class="library-drop-icon"><Layers size={17} /></div><div><strong>Release to add media to the queue</strong><span>The dropped paths will be scanned with the current library filters.</span></div></div>
{/if}
<details class="library-scan-options">
  <summary>Scan options</summary>
  <div class="folder-field"><FolderOpen size={18} /><input aria-label="Path to add to queue" bind:value={root} placeholder="Paste a media file or folder path to add" disabled={workflowBusy} onblur={() => void persistPreferences({ root })} /><button class="button secondary" type="button" disabled={!desktopShell || workflowBusy || !root} onclick={analyzeFolder}>{scanning ? "Scanning" : "Add path"}</button></div>
  <div class="toolbar-options"><label class="check-label"><input type="checkbox" bind:checked={recursive} disabled={workflowBusy} onchange={() => void persistPreferences({ recursive })} /> Include subfolders on new scans</label><label class="small-field">Ignore smaller than <input type="number" min="0" max="4096" step="1" bind:value={minimumSizeMib} disabled={workflowBusy} onblur={() => void persistPreferences({ minimumSizeMib })} /> MiB</label></div>
</details>
</section>

<LibraryQueueStatus queue={workspace.queue} busy={workflowBusy} {scanning} {scanProgress} {fileName} onActivate={activateQueueSource} onRefresh={(path: string) => scanQueueSource(path, true, true)} onRemove={removeQueueSource} onCancelScan={cancelScan} onAddFolder={browseFolder} onAddFiles={browseFile} onReorderSources={reorderQueueSources} onMoveSource={moveQueueSource} onRemoveFile={removeQueueEntry} onReorderFiles={reorderQueueFiles} onMoveFile={moveQueueFile} onRestoreExcluded={restoreExcludedQueueFiles} />

{#if scanResult}
{#if runReviewEntries.some((entry) => queueRunStateForPath(entry.path)?.status === "interrupted")}<div class="inline-warning"><AlertTriangle size={17} /><div><strong>Some files were still active when the last session ended.</strong><span>The encode was not resumed. Originals and existing outputs were left untouched. Review the last report before retrying.</span></div><button class="button secondary" type="button" disabled={workflowBusy} onclick={() => void openInterruptedRecovery()}>Review interrupted files</button></div>{/if}
{#if unresolvedSelectedPaths > 0}<div class="inline-warning"><AlertTriangle size={17} /><div><strong>{selectionHasNoRunnableFiles ? "These selected files are not ready to run." : `${unresolvedSelectedPaths} selected file${unresolvedSelectedPaths === 1 ? " is" : "s are"} not in a completed scan.`}</strong><span>Rescan or remove the interrupted source before starting this selection.</span></div><button class="text-button" onclick={keepReadyFiles}>Keep ready files</button></div>{/if}
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
  onToggleSelect={toggleSelect}
  onToggleAnimated={setPathsAnimated}
  onOpenReview={openLibraryPoster}
  onOpenLocation={(path: string) => void openFileLocation(path)}
  onReview={openLibraryReview}
  canAdjustRun={(mode === "test" || mode === "full") && !workflowBusy}
  onAdjustRun={openRunSettingsEditor}
  onRename={() => chooseLibraryTask("organize")}
  onOrganize={() => openPage("metadata")}
  onMetadata={() => chooseLibraryTask("metadata")}
  onIntegrity={() => chooseLibraryTask("integrity")}
  onRebuild={() => chooseLibraryTask("rebuild")}
  onSetOutput={chooseFileOutput}
  onClearOutput={clearFileOutput}
  onSelectPage={setVisibleSelection}
  onRemoveFromQueue={(paths: string[]) => { for (const path of paths) removeQueueEntry(path); }}
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
<div class="empty-state large"><div class="empty-icon"><Layers size={26} /></div><div><div class="eyebrow">LIBRARY QUEUE</div><h2>{workspace.queue.sources.length ? "Choose a folder to review." : "Add your first media."}</h2><p>{workspace.queue.sources.length ? "Select a folder above, or add another folder or a group of files." : "Drop files here, or add a folder. Posters and titles appear as each file is matched."}</p></div><div class="empty-state-actions"><button class="button secondary" disabled={!desktopShell || workflowBusy} onclick={browseFolder}>Add folder</button><button class="button secondary" disabled={!desktopShell || workflowBusy} onclick={browseFile}>Add files</button></div></div>
{/if}
