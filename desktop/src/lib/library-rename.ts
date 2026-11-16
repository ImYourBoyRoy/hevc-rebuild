// ./desktop/src/lib/library-rename.ts
// The library rename dialog helpers, applyLibraryRename (preview, confirm, apply), the library task
// shortcuts, metadata-only organizer selection, the remap of every piece of queue state after files
// are renamed, and the metadata review dialog open/close, extracted from the route (rename slice).
// The route's state is reached through a typed `LibraryRenameHost`; the native bridge and the route's
// actions are injected so every branch is unit-tested in node (pnpm test:library-rename). The code is
// the route's code moved verbatim with `h.` prefixes.

import type { AppPreferences } from './ui-contracts.js';
import type {
  FileRunSettingsOverride, MediaEntry, MediaQueueRunState, MediaQueueSource, OrganizerTemplates, Page, QueueMetadataSuggestion, QueueMovieReview, QueueSeriesReview, RunMode, ScanResult, Settings,
} from './types.js';
import type { LibraryTvReviewTarget } from './review-notes.js';
import type { LibrarySeriesGroup } from './library-browse.ts';
import { proposedFilenameForEntry } from './filename-template.ts';
import { fileName, queuePathKey, relativeQueuePath, remapQueuePath } from './queue-paths.ts';
import { makeQueueRunState } from './queue-run-state.ts';
import { scanResultWithQueueStatus } from './queue-document.ts';
import { insertRenameTemplateToken } from './rename-template.ts';

export interface LibraryRenameHost {
  renameDraft: OrganizerTemplates;
  libraryRenameError: string;
  libraryRenameBusy: boolean;
  libraryRenameOpen: boolean;
  libraryRenameDialog: HTMLDialogElement | null;
  metadataFocusPath: string | null;
  libraryTvReview: LibraryTvReviewTarget | null;
  metadataDialogOpen: boolean;
  metadataDialogElement: HTMLDialogElement | null;
  mode: RunMode;
  root: string;
  scanResult: ScanResult | null;
  queueMovieReviews: Record<string, QueueMovieReview>;
  queueSeriesReviews: Record<string, QueueSeriesReview>;
  queueMatchSuggestions: Record<string, QueueMetadataSuggestion>;
  fileTuneOverrides: Record<string, "animation" | "none">;
  fileStreamOverrides: Record<string, number[]>;
  fileSettingsOverrides: Record<string, FileRunSettingsOverride>;
  queueArtworkPreviews: Record<string, string>;
  queueArtworkPreviewErrors: Record<string, string>;
  queueArtworkPreviewBusy: Record<string, boolean>;
  selectedPaths: string[];
  sources: MediaQueueSource[];
  runStates: Record<string, MediaQueueRunState>;
  activeRoot: string | null;
  readonly appPreferences: AppPreferences;
  readonly settings: Settings | null;
  readonly workflowBusy: boolean;
  readonly queuedEntries: MediaEntry[];
  setNotice(message: string, kind?: "info" | "success" | "error"): void;
}

export interface LibraryRenameActions {
  persistPreferences(patch: Partial<AppPreferences>, successMessage?: string): Promise<boolean> | boolean | void;
  persistQueueSources(): Promise<unknown> | unknown;
  openPage(page: Page): void;
  beginRun(): Promise<unknown> | unknown;
  tick(): Promise<void>;
  ensureQueueArtworkPreview(mediaPath: string, artworkPath: string): Promise<unknown> | unknown;
  seriesGroupForRoot(root: string): LibrarySeriesGroup | undefined;
  tvReviewForEpisode(entry: MediaEntry): LibraryTvReviewTarget | null;
  posterPathsForReview(group: LibrarySeriesGroup, season: number | null): string[];
  reviewedSeriesForPath(path: string): QueueSeriesReview | undefined;
}

export interface LibraryRenameDeps {
  invoke: <T>(command: string, args?: Record<string, unknown>) => Promise<T>;
  host: LibraryRenameHost;
  actions: LibraryRenameActions;
  constants: { defaultOrganizerTemplates: OrganizerTemplates };
}

export function createLibraryRename(deps: LibraryRenameDeps) {
  const { invoke, host: h, actions, constants } = deps;
  const { persistPreferences, persistQueueSources, openPage, beginRun, tick, ensureQueueArtworkPreview, seriesGroupForRoot, tvReviewForEpisode, posterPathsForReview, reviewedSeriesForPath } = actions;
  const { defaultOrganizerTemplates } = constants;
  const setNotice = h.setNotice.bind(h);

  async function openMetadataForEntry(entry: MediaEntry) {
    h.selectedPaths = [...new Set([...h.selectedPaths, entry.path])];
    h.metadataFocusPath = entry.path;
    h.libraryTvReview = tvReviewForEpisode(entry);
    const opened = h.libraryTvReview;
    if (opened?.level === "episode" && opened.season !== null) {
      const group = seriesGroupForRoot(opened.seriesRoot);
      if (group) {
        for (const path of posterPathsForReview(group, opened.season)) void ensureQueueArtworkPreview(entry.path, path);
      }
    }
    h.metadataDialogOpen = true;
    await tick();
    if (h.metadataDialogOpen && h.metadataDialogElement && !h.metadataDialogElement.open) {
      h.metadataDialogElement.showModal();
    }
  }

  function closeMetadataDialog() {
    if (h.metadataDialogElement?.open) h.metadataDialogElement.close();
    else {
      h.metadataDialogOpen = false;
      h.libraryTvReview = null;
    }
  }

  function openFullMetadataOrganizer() {
    closeMetadataDialog();
    openPage("metadata");
  }

  function libraryRenameEntries(): MediaEntry[] {
    const selected = new Set(h.selectedPaths);
    return h.queuedEntries.filter((entry) => selected.has(entry.path));
  }

  function libraryRenameShape(): "movie" | "episode" | "both" {
    let movie = false;
    let episode = false;
    for (const entry of libraryRenameEntries()) {
      if (h.queueMovieReviews[entry.path]?.title.trim()) movie = true;
      else episode = true;
    }
    if (movie && episode) return "both";
    return movie ? "movie" : "episode";
  }

  function insertRenameToken(kind: "movie" | "episode", token: string, field: HTMLInputElement | null) {
    const current = h.renameDraft[kind];
    const insertion = insertRenameTemplateToken(current, token, field?.selectionStart ?? null, field?.selectionEnd ?? null);
    h.renameDraft = { ...h.renameDraft, [kind]: insertion.value };
    h.libraryRenameError = "";
    queueMicrotask(() => {
      field?.focus();
      field?.setSelectionRange(insertion.caret, insertion.caret);
    });
  }

  function resetRenamePattern(kind: "movie" | "episode") {
    h.renameDraft = { ...h.renameDraft, [kind]: defaultOrganizerTemplates[kind] };
    h.libraryRenameError = "";
  }

  function libraryRenameNote(entry: MediaEntry, templates: OrganizerTemplates = h.renameDraft): { next: string | null; note: string } {
    const movie = h.queueMovieReviews[entry.path];
    const series = reviewedSeriesForPath(entry.path);
    const proposal = proposedFilenameForEntry(entry, movie, series, h.queueMatchSuggestions[entry.path], templates);
    const next = proposal?.name && proposal.name !== entry.name ? proposal.name : null;
    if (next && proposal?.needsTitleReview) return { next, note: "Suggested name" };
    if (next) return { next, note: "New name" };
    return { next: null, note: "Name stays the same" };
  }

  async function applyLibraryRename() {
    if (h.libraryRenameBusy) return;
    const entries = libraryRenameEntries();
    h.libraryRenameError = "";
    if (!entries.length) {
      h.libraryRenameError = "Select the files to rename.";
      return;
    }
    const roots = [...new Set(entries.flatMap((entry) => {
      const source = h.sources.find((item) => item.scanResult?.entries.some((candidate) => candidate.path === entry.path));
      return source ? [source.root] : [];
    }))];
    if (roots.length !== 1) {
      h.libraryRenameError = "Rename one folder at a time.";
      return;
    }
    const templates = h.renameDraft;
    const items: { sourcePath: string; kind: "movie" | "episode"; template: string; values: Record<string, string> }[] = [];
    for (const entry of entries) {
      const movie = h.queueMovieReviews[entry.path];
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
      h.libraryRenameError = "Confirm a title on each selected file before renaming. A file that already has the right name stays as it is.";
      return;
    }
    h.libraryRenameBusy = true;
    try {
      if (JSON.stringify(h.renameDraft) !== JSON.stringify(h.appPreferences.organizerTemplates)) {
        await persistPreferences({ organizerTemplates: h.renameDraft });
      }
      const request = { root: roots[0], items, textBlacklistPatterns: h.settings?.metadata.text_blacklist ?? [] };
      const preview = await invoke<{ previewId: string; canApply: boolean; conflicts: { reason: string }[]; items: { sourcePath: string; targetPath: string; unchanged: boolean }[] }>("preview_library_rename", request);
      if (!preview.canApply) {
        h.libraryRenameError = preview.conflicts[0]?.reason ?? "These files already use the proposed names.";
        return;
      }
      await invoke("apply_library_rename", { ...request, previewId: preview.previewId, confirmed: true });
      handleQueuePathsRenamed(Object.fromEntries(preview.items.map((item) => [item.sourcePath, item.targetPath])));
      const changed = preview.items.filter((item) => !item.unchanged).length;
      h.libraryRenameOpen = false;
      h.setNotice(`Renamed ${changed} file${changed === 1 ? "" : "s"}. The video itself was not changed.`, "success");
    } catch (cause) {
      h.libraryRenameError = cause instanceof Error ? cause.message : String(cause);
    } finally {
      h.libraryRenameBusy = false;
    }
  }

  function chooseLibraryTask(task: "organize" | "metadata" | "rebuild" | "integrity") {
    if (h.workflowBusy) return;
    if (task === "organize") {
      h.libraryRenameError = "";
      h.renameDraft = { ...h.appPreferences.organizerTemplates };
      h.libraryRenameOpen = true;
      void tick().then(() => {
        if (h.libraryRenameOpen && h.libraryRenameDialog && !h.libraryRenameDialog.open) h.libraryRenameDialog.showModal();
      });
      return;
    }
    h.mode = task === "metadata" ? "metadata" : task === "integrity" ? "integrity" : "full";
    openPage("run");
    void tick().then(() => {
      if (!h.workflowBusy) void beginRun();
    });
  }

  function startMetadataOnlyFromOrganizer(mediaPaths: string[]) {
    if (h.workflowBusy) return;
    const queuedPaths = new Map<string, string>();
    for (const source of h.sources) {
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
      h.setNotice("Add these files to the Media Library queue and finish their scan before creating MKV copies.", "error");
      return;
    }
    h.selectedPaths = uniquePaths;
    h.mode = "metadata";
    openPage("run");
    h.setNotice(`${uniquePaths.length} file${uniquePaths.length === 1 ? "" : "s"} selected for metadata-only MKV output. Review the destination and collision check before starting.`, "success");
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
    h.sources = h.sources.map((source) => {
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
        excludedPaths: (source.excludedPaths ?? []).map((path) => remapQueuePath(path, movedPaths)),
        entryOrder: (source.entryOrder ?? []).map((path) => remapQueuePath(path, movedPaths)),
      };
    });
    h.selectedPaths = [...new Set(h.selectedPaths.map((path) => remapQueuePath(path, movedPaths)))];
    h.fileTuneOverrides = remapRecord(h.fileTuneOverrides);
    h.fileStreamOverrides = remapRecord(h.fileStreamOverrides);
    h.fileSettingsOverrides = remapRecord(h.fileSettingsOverrides);
    h.queueMovieReviews = Object.fromEntries(Object.entries(remapRecord(h.queueMovieReviews)).map(([path, review]) => [
      path,
      { ...review, localPosterPath: review.localPosterPath ? remapQueuePath(review.localPosterPath, movedPaths) : "" },
    ]));
    h.queueSeriesReviews = Object.fromEntries(Object.entries(h.queueSeriesReviews).map(([seriesRoot, review]) => [
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
    h.runStates = Object.fromEntries(Object.entries(h.runStates).map(([path, state]) => {
      const target = remapQueuePath(path, movedPaths);
      if (target === path) return [path, state];
      return [target, makeQueueRunState(state.status === "queued" ? "queued" : "needs_review", state.mode ?? h.mode, {
        message: state.status === "queued" ? state.message : "The media path changed after this run. Review the report and output before retrying.",
        reportPath: state.reportPath,
        reportStatus: state.reportStatus,
      })];
    }));
    h.queueArtworkPreviews = {};
    h.queueArtworkPreviewErrors = {};
    h.queueArtworkPreviewBusy = {};
    h.activeRoot = h.activeRoot ? remapQueuePath(h.activeRoot, movedPaths) : h.activeRoot;
    h.root = h.root ? remapQueuePath(h.root, movedPaths) : h.root;
      h.metadataFocusPath = h.metadataFocusPath ? remapQueuePath(h.metadataFocusPath, movedPaths) : h.metadataFocusPath;
    h.queueMatchSuggestions = remapRecord(h.queueMatchSuggestions);
    if (h.scanResult) {
      const active = h.sources.find((source) => source.root === h.activeRoot);
      if (active?.scanResult) h.scanResult = scanResultWithQueueStatus(active);
    }
    const fileOutputDirs = remapRecord(h.appPreferences.fileOutputDirs);
    void persistPreferences({ root: h.root, fileOutputDirs });
    void persistQueueSources();
  }

  return { openMetadataForEntry, closeMetadataDialog, openFullMetadataOrganizer, libraryRenameEntries, libraryRenameShape, insertRenameToken, resetRenamePattern, libraryRenameNote, applyLibraryRename, chooseLibraryTask, startMetadataOnlyFromOrganizer, handleQueuePathsRenamed };
}
