// ./desktop/src/lib/library-actions.ts
// Library actions extracted from the route (helper slice): bulk selection of visible cards or rows, selecting files to
// review, whether a source still needs a metadata lookup, opening the metadata review dialog, choosing a poster image,
// opening a library poster and resolving the media entries and folder a review targets. Dialogs and DOM timing are
// injected so every branch is unit-tested in node (pnpm test:library-actions). The code is the route's code moved verbatim.

import type { LibraryPosterCard, MediaEntry, MediaQueueRunState, MediaQueueSource, QueueMetadataSuggestion, QueueMovieReview, QueueSeriesReview } from "./types.js";
import type { AppPreferences } from "./ui-contracts.js";
import type { LibrarySeriesGroup } from "./library-browse.js";
import type { LibraryTvReviewTarget } from "./review-notes.js";
import { queuePathKey } from "./queue-paths.ts";
import { selectQueuePaths } from "./workspace-state.ts";

export interface LibraryActionsHost {
  selectedPaths: string[];
  appPreferences: AppPreferences;
  queueMovieReviews: Record<string, QueueMovieReview>;
  queueMatchSuggestions: Record<string, QueueMetadataSuggestion>;
  metadataFocusPath: string | null;
  libraryTvReview: LibraryTvReviewTarget | null;
  metadataDialogOpen: boolean;
  metadataDialogElement: HTMLDialogElement | null;
  libraryBrowse: { seriesRoot: string; season: number | null } | null;
  readonly libraryPosterCards: LibraryPosterCard[];
  readonly visibleEntries: MediaEntry[];
  readonly currentPageEntries: MediaEntry[];
  readonly queuedEntries: MediaEntry[];
  readonly runReviewEntries: MediaEntry[];
  readonly librarySeriesGroups: LibrarySeriesGroup[];
  setNotice(message: string, kind?: "info" | "success" | "error"): void;
}

export interface LibraryActionsActions {
  queueRunStateForPath(path: string): MediaQueueRunState | undefined;
  reviewedSeriesForPath(path: string): QueueSeriesReview | undefined;
  seriesGroupForRoot(root: string): LibrarySeriesGroup | undefined;
  ensureQueueArtworkPreview(entryPath: string, imagePath: string): Promise<void>;
  patchSeriesReview(root: string, update: (review: QueueSeriesReview) => QueueSeriesReview): void;
  openMetadataForEntry(entry: MediaEntry): Promise<void>;
  openDialog(options: Record<string, unknown>): Promise<string | string[] | null>;
  tick(): Promise<void>;
}

export interface LibraryActionsDeps {

  host: LibraryActionsHost;
  actions: LibraryActionsActions;
}

export function createLibraryActions(deps: LibraryActionsDeps) {
  const { host: h, actions } = deps;
  const { queueRunStateForPath, reviewedSeriesForPath, seriesGroupForRoot, ensureQueueArtworkPreview, patchSeriesReview, openMetadataForEntry, openDialog, tick } = actions;
  const setNotice = h.setNotice.bind(h);

  function setVisibleSelection(select: boolean) {
    const visible = new Set(h.libraryPosterCards.flatMap((card) => card.mediaPaths));
    h.selectedPaths = selectQueuePaths(h.selectedPaths, visible, select);
  }

  function setFilteredSelection(select: boolean) {
    const filtered = new Set(h.visibleEntries.map((entry) => entry.path));
    h.selectedPaths = selectQueuePaths(h.selectedPaths, filtered, select);
  }

  function selectFilesToReview() {
    h.selectedPaths = h.runReviewEntries.map((entry) => entry.path);
    if (h.runReviewEntries.some((entry) => queueRunStateForPath(entry.path)?.status === "interrupted")) {
      setNotice("Interrupted files are selected. Check their last report and output before retrying; existing outputs are never overwritten.", "info");
    } else {
      setNotice(`${h.selectedPaths.length} unfinished or skipped file${h.selectedPaths.length === 1 ? "" : "s"} selected for review.`, "info");
    }
  }

  function sourceNeedsMetadataLookup(source: MediaQueueSource): boolean {
    if (!h.appPreferences.metadataAutoLookup || !source.scanResult) return false;
    return source.scanResult.entries.some((entry) => {
      if (h.queueMovieReviews[entry.path]?.recordId) return false;
      const series = reviewedSeriesForPath(entry.path);
      const seriesRecord = series?.recordId ?? "";
      if (seriesRecord && !seriesRecord.startsWith("local")) {
        const hasProviderArt = (series?.artworkCandidates ?? []).some((image) => image.imageUrl.startsWith("https://") || image.thumbnailUrl.startsWith("https://"));
        const group = h.librarySeriesGroups.find((item) => item.episodePaths.some((path) => queuePathKey(path) === queuePathKey(entry.path)));
        const seasonNumbers = group?.seasons.map((item) => item.season).filter((number) => number > 0) ?? [];
        const hasSeasonArt = seasonNumbers.length
          ? seasonNumbers.every((number) => (series?.localSeasonPosterPaths?.[String(number)] ?? "").startsWith("https://"))
          : Object.values(series?.localSeasonPosterPaths ?? {}).some((path) => path.startsWith("https://"));
        const hasCertificate = Boolean(series?.certification.trim());
        return !hasProviderArt || !hasSeasonArt || !hasCertificate;
      }
      const status = h.queueMatchSuggestions[entry.path]?.status;
      return status === undefined || status === "searching";
    });
  }

  async function showLibraryReview(target: LibraryTvReviewTarget) {
    const sample = target.episodePath ?? seriesGroupForRoot(target.seriesRoot)?.episodePaths[0] ?? "";
    h.metadataFocusPath = sample || h.metadataFocusPath;
    h.libraryTvReview = target;
    h.metadataDialogOpen = true;
    await tick();
    if (h.metadataDialogOpen && h.metadataDialogElement && !h.metadataDialogElement.open) h.metadataDialogElement.showModal();
  }

  async function chooseReviewImage() {
    const target = h.libraryTvReview;
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
    const card = h.libraryPosterCards.find((item) => item.path === path);
    if (card?.kind === "series") {
      h.libraryBrowse = { seriesRoot: path, season: null };
      return;
    }
    if (card?.kind === "season" && card.seasonNumber !== null && h.libraryBrowse) {
      h.libraryBrowse = { seriesRoot: h.libraryBrowse.seriesRoot, season: card.seasonNumber };
      return;
    }
    const entry = h.visibleEntries.find((candidate) => candidate.path === path)
      ?? h.currentPageEntries.find((candidate) => candidate.path === path)
      ?? h.queuedEntries.find((candidate) => candidate.path === path);
    if (entry) void openMetadataForEntry(entry);
  }

  function reviewMediaEntries(target: LibraryTvReviewTarget): MediaEntry[] {
    const group = seriesGroupForRoot(target.seriesRoot);
    const paths = target.level === "episode" && target.episodePath
      ? [target.episodePath]
      : target.level === "season"
        ? group?.seasons.find((item) => item.season === target.season)?.episodePaths ?? []
        : group?.episodePaths ?? [];
    return paths.flatMap((path) => {
      const entry = h.queuedEntries.find((item) => queuePathKey(item.path) === queuePathKey(path));
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

  return { setVisibleSelection, setFilteredSelection, selectFilesToReview, sourceNeedsMetadataLookup, showLibraryReview, chooseReviewImage, openLibraryPoster, reviewMediaEntries, reviewLocationPath };
}
