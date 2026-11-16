// ./desktop/src/lib/poster-selection.ts
// Poster choices extracted from MetadataWorkbench (T10.7 slice 11): choosing and toggling a series or season poster (local or provider),
// choosing a film poster, the batch apply of a local poster to several films and the film artwork choices. The state and props are
// reached through a typed host and the review being edited comes from injected actions, so every branch is unit-tested in node
// (pnpm test:poster-selection). The code is the component's code moved verbatim.

import type { Artwork, MovieAssignment, OrganizerScan, PendingArtworkSave, SeriesAssignment } from "./metadata-workbench-types.js";


export interface PosterSelectionHost {
  seriesAssignments: Record<string, SeriesAssignment>;
  movieAssignments: Record<string, MovieAssignment>;
  selectedArtwork: Record<string, string>;
  organizerScan: OrganizerScan | null;
  pendingArtworkSave: PendingArtworkSave | null;
  artworkMessage: string;
  pendingBatchLocalPoster: { sourcePath: string; artworkPath: string; mediaPaths: string[] } | null;
}

export interface PosterSelectionActions {
  currentReviewedSeriesRoot(): string;
  makeCurrentSeriesReview(): SeriesAssignment | null;
  selectedPathsInOrganizer(): string[];
}

export interface PosterSelectionDeps {

  host: PosterSelectionHost;
  actions: PosterSelectionActions;
}

export function createPosterSelection(deps: PosterSelectionDeps) {
  const { host: h, actions } = deps;
  const { currentReviewedSeriesRoot, makeCurrentSeriesReview, selectedPathsInOrganizer } = actions;

  function updateCurrentSeriesArtwork(update: (review: SeriesAssignment) => SeriesAssignment) {
    const root = currentReviewedSeriesRoot();
    if (!root) return;
    const review = h.seriesAssignments[root] ?? makeCurrentSeriesReview();
    if (!review) return;
    h.seriesAssignments = { ...h.seriesAssignments, [root]: update(review) };
  }

  function chooseSeriesLocalPoster(path: string) {
    const root = currentReviewedSeriesRoot();
    const selectedPath = h.seriesAssignments[root]?.localPosterPath === path ? "" : path;
    h.selectedArtwork = { ...h.selectedArtwork, series: "" };
    updateCurrentSeriesArtwork((review) => ({ ...review, posterId: "", localPosterPath: selectedPath }));
  }

  function chooseSeriesLocalSeasonPoster(season: number, path: string) {
    updateCurrentSeriesArtwork((review) => ({
      ...review,
      localSeasonPosterPaths: (() => {
        const next = { ...review.localSeasonPosterPaths };
        if (next[String(season)] === path) delete next[String(season)];
        else next[String(season)] = path;
        return next;
      })(),
    }));
  }

  function chooseSeriesProviderPoster(id: string) {
    h.selectedArtwork = { ...h.selectedArtwork, series: id };
    updateCurrentSeriesArtwork((review) => ({ ...review, posterId: id, localPosterPath: "" }));
    h.pendingArtworkSave = null;
    h.artworkMessage = "";
  }

  function chooseSeriesProviderSeasonPoster(season: number, id: string) {
    h.selectedArtwork = { ...h.selectedArtwork, [`season:${season}`]: id };
    updateCurrentSeriesArtwork((review) => {
      const localSeasonPosterPaths = { ...review.localSeasonPosterPaths };
      delete localSeasonPosterPaths[String(season)];
      return { ...review, localSeasonPosterPaths };
    });
    h.pendingArtworkSave = null;
    h.artworkMessage = "";
  }

  function chooseMovieLocalPoster(mediaPath: string, artworkPath: string) {
    const review = h.movieAssignments[mediaPath];
    if (!review) return;
    const selectedPath = review.localPosterPath === artworkPath ? "" : artworkPath;
    h.movieAssignments = {
      ...h.movieAssignments,
      [mediaPath]: { ...review, posterId: "", localPosterPath: selectedPath },
    };
    h.pendingBatchLocalPoster = null;
  }

  function batchLocalPosterTargets(): string[] {
    if (!h.organizerScan) return [];
    const files = new Set(h.organizerScan.files.map((file) => file.path));
    return [...new Set(selectedPathsInOrganizer())]
      .filter((path) => files.has(path) && Boolean(h.movieAssignments[path]));
  }

  function previewBatchLocalPoster(mediaPath: string, artworkPath: string) {
    const mediaPaths = batchLocalPosterTargets();
    if (mediaPaths.length < 2) {
      chooseMovieLocalPoster(mediaPath, artworkPath);
      return;
    }
    h.pendingBatchLocalPoster = { sourcePath: mediaPath, artworkPath, mediaPaths };
  }

  function applyBatchLocalPoster() {
    const pending = h.pendingBatchLocalPoster;
    if (!pending || !h.organizerScan) return;
    const files = new Set(h.organizerScan.files.map((file) => file.path));
    const mediaPaths = pending.mediaPaths.filter((path) => files.has(path) && Boolean(h.movieAssignments[path]));
    if (!mediaPaths.length || !files.has(pending.sourcePath)) {
      h.pendingBatchLocalPoster = null;
      return;
    }
    h.movieAssignments = {
      ...h.movieAssignments,
      ...Object.fromEntries(mediaPaths.map((path) => [path, {
        ...h.movieAssignments[path],
        posterId: "",
        localPosterPath: pending.artworkPath,
      }])),
    };
    h.pendingBatchLocalPoster = null;
  }

  function selectMovieArtwork(mediaPath: string, posterId: string, artwork?: Artwork) {
    const review = h.movieAssignments[mediaPath];
    if (!review) return;
    const artworkCandidates = artwork && !review.artworkCandidates.some((image) => image.id === artwork.id)
      ? [artwork, ...review.artworkCandidates]
      : review.artworkCandidates;
    h.movieAssignments = {
      ...h.movieAssignments,
      [mediaPath]: { ...review, artworkCandidates, posterId, localPosterPath: "" },
    };
  }

  function chooseMovieProviderPoster(mediaPath: string) {
    const review = h.movieAssignments[mediaPath];
    if (!review) return;
    h.movieAssignments = {
      ...h.movieAssignments,
      [mediaPath]: { ...review, localPosterPath: "" },
    };
  }

  return { updateCurrentSeriesArtwork, chooseSeriesLocalPoster, chooseSeriesLocalSeasonPoster, chooseSeriesProviderPoster, chooseSeriesProviderSeasonPoster, chooseMovieLocalPoster, batchLocalPosterTargets, previewBatchLocalPoster, applyBatchLocalPoster, selectMovieArtwork, chooseMovieProviderPoster };
}
