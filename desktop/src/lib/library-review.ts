// ./desktop/src/lib/library-review.ts
// Library review controller extracted from the route (review slice B): creating and patching
// series/season/episode and movie reviews, opening review dialogs, poster selection and the comment
// rules. The route's reactive state is reached through one typed `LibraryReviewHost`; the review
// factories and poster resolvers are injected. The dialog itself (focus, `showModal`) stays in the
// route behind `showLibraryReview`. Tested with a fake host (pnpm test:library-review). No DOM.

import { episodeIdentity, fillEpisodeFromCatalog, groupLibraryEntries, type LibrarySeriesGroup } from './library-browse.ts';
import { queuePathKey } from './queue-paths.ts';
import { providerPosterUrl } from './poster-urls.ts';
import { blankSeasonReview } from './review-factories.ts';
import {
  applyTvField, posterPathsForReview as posterPathsFor, reviewPosterChoices as reviewPosterChoicesFor, reviewedSeriesForPath as reviewedSeriesFor,
  seasonPremiereYear, sharedCommentFor as sharedCommentForReview, tvReviewHeading as tvReviewHeadingFor, tvReviewPaths as tvReviewPathsFor,
  type LibraryTvReviewTarget, type LocalLibraryRecord,
} from './review-notes.ts';
import type { LibraryPosterCard, MediaEntry, QueueMovieReview, QueueSeriesReview, QueueTvEpisode, ScanResult, Settings } from './types.js';

/** Everything the review controller reads or writes on the route. Property names match the route's variables. */
export interface LibraryReviewHost {
  queueSeriesReviews: Record<string, QueueSeriesReview>;
  queueMovieReviews: Record<string, QueueMovieReview>;
  fileTuneOverrides: Record<string, "animation" | "none">;
  readonly libraryLocalPosters: Record<string, string>;
  readonly libraryLocalRecords: Record<string, LocalLibraryRecord>;
  readonly libraryBrowse: { seriesRoot: string; season: number | null } | null;
  readonly librarySeriesGroups: LibrarySeriesGroup[];
  readonly scanResult: ScanResult | null;
  readonly queuedEntries: MediaEntry[];
  readonly libraryPosterCards: LibraryPosterCard[];
  readonly settings: Settings | null;
  ensureQueueArtworkPreview(mediaPath: string, artworkPath: string): void | Promise<void>;
  showLibraryReview(target: LibraryTvReviewTarget): void | Promise<void>;
}

export interface LibraryReviewDeps {
  host: LibraryReviewHost;
  blankSeriesReview: (group: LibrarySeriesGroup) => QueueSeriesReview;
  seriesReviewStorageKey: (root: string) => string;
  episodeDraft: (path: string) => QueueTvEpisode;
  chosenPoster: (path: string) => string;
  seasonArtworkPath: (review: QueueSeriesReview | undefined, season: number, directory: string) => string;
  displayPosterUrl: (path: string) => string | null;
}

export function createLibraryReviewController(deps: LibraryReviewDeps) {
  const { host: h, blankSeriesReview, seriesReviewStorageKey, episodeDraft, chosenPoster, seasonArtworkPath, displayPosterUrl } = deps;

  // The pure review helpers, bound to the route state they used to read.
  const posterPathsForReview = (group: LibrarySeriesGroup, season: number | null) => posterPathsFor(group, season, h.libraryLocalRecords);
  const reviewPosterChoices = (group: LibrarySeriesGroup, season: number | null, review: QueueSeriesReview | undefined) => reviewPosterChoicesFor(group, season, review, h.libraryLocalRecords, displayPosterUrl);
  const sharedCommentFor = (target: LibraryTvReviewTarget, review: QueueSeriesReview) => sharedCommentForReview(target, review, h.settings?.metadata.comment);
  const reviewedSeriesForPath = (path: string): QueueSeriesReview | undefined => reviewedSeriesFor(path, new Map(Object.entries(h.queueSeriesReviews).map(([root, review]) => [queuePathKey(root), review])));

  function ensureSeriesReview(group: LibrarySeriesGroup): string {
    const key = seriesReviewStorageKey(group.root);
    const current = h.queueSeriesReviews[key];
    const seeded = current ?? blankSeriesReview(group);
    const next: QueueSeriesReview = {
      ...seeded,
      seasonRecords: seeded.seasonRecords ?? {},
      localSeasonPosterPaths: seeded.localSeasonPosterPaths ?? {},
      episodeAssignments: seeded.episodeAssignments ?? {},
      title: seeded.title.trim() || group.folderName,
      localPosterPath: seeded.localPosterPath || h.libraryLocalPosters[queuePathKey(group.root)] || "",
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
    if (changed) h.queueSeriesReviews = { ...h.queueSeriesReviews, [key]: next };
    return key;
  }

  function ensureSeasonReview(group: LibrarySeriesGroup, season: number): { key: string; seasonKey: string } {
    const key = ensureSeriesReview(group);
    const seasonKey = String(season);
    const review = h.queueSeriesReviews[key];
    const generated = group.seasons.find((item) => item.season === season)?.label ?? `Season ${season}`;
    const current = review.seasonRecords[seasonKey];
    const premiereYear = seasonPremiereYear(review, season);
    if (!current) {
      const created = blankSeasonReview(generated, review);
      if (premiereYear && premiereYear !== review.year) created.year = premiereYear;
      h.queueSeriesReviews = {
        ...h.queueSeriesReviews,
        [key]: { ...review, seasonRecords: { ...review.seasonRecords, [seasonKey]: created } },
      };
    } else if (premiereYear && current.year === review.year && premiereYear !== review.year) {
      h.queueSeriesReviews = {
        ...h.queueSeriesReviews,
        [key]: { ...review, seasonRecords: { ...review.seasonRecords, [seasonKey]: {
          ...current,
          year: premiereYear,
        } } },
      };
    }
    return { key, seasonKey };
  }

  function ensureEpisodeReview(group: LibrarySeriesGroup, path: string): { key: string; episode: QueueTvEpisode } {
    const key = ensureSeriesReview(group);
    const review = h.queueSeriesReviews[key];
    const draft = episodeDraft(path);
    const existing = review.episodeAssignments[path];
    const identityTitle = episodeIdentity(h.queuedEntries.find((item) => item.path === path)?.name ?? "")?.title ?? "";
    const match = review.episodes.find((item) => item.season === (existing?.season ?? draft.season) && item.episode === (existing?.episode ?? draft.episode));
    const starting = existing
      ? { ...existing, summary: review.plot && existing.summary === review.plot ? draft.summary : existing.summary }
      : draft;
    const episode = fillEpisodeFromCatalog(starting, match, review.title, review.plot, identityTitle);
    if (existing && episode.title === existing.title && episode.summary === existing.summary && episode.airdate === existing.airdate && episode.sourceUrl === existing.sourceUrl) {
      return { key, episode: existing };
    }
    h.queueSeriesReviews = {
      ...h.queueSeriesReviews,
      [key]: { ...review, episodeAssignments: { ...review.episodeAssignments, [path]: episode } },
    };
    return { key, episode };
  }

  function tvReviewForEpisode(entry: MediaEntry): LibraryTvReviewTarget | null {
    const identity = episodeIdentity(entry.name);
    const group = h.librarySeriesGroups.find((item) => item.episodePaths.some((path) => queuePathKey(path) === queuePathKey(entry.path)))
      ?? (h.scanResult ? groupLibraryEntries(h.scanResult.root, h.queuedEntries) : []).find((item) => item.episodePaths.some((path) => queuePathKey(path) === queuePathKey(entry.path)));
    if (!group || !identity) return null;
    ensureEpisodeReview(group, entry.path);
    return { level: "episode", seriesRoot: group.root, season: identity.season, episodePath: entry.path };
  }

  function seriesGroupForRoot(root: string): LibrarySeriesGroup | undefined {
    return h.librarySeriesGroups.find((item) => queuePathKey(item.root) === queuePathKey(root))
      ?? (h.scanResult ? groupLibraryEntries(h.scanResult.root, h.queuedEntries) : []).find((item) => queuePathKey(item.root) === queuePathKey(root));
  }

  function openSeriesReview(root: string) {
    const group = seriesGroupForRoot(root);
    if (!group) return;
    const key = ensureSeriesReview(group);
    const posters = posterPathsForReview(group, null);
    const sample = group.episodePaths[0] ?? "";
    for (const path of posters) if (sample) void h.ensureQueueArtworkPreview(sample, path);
    void h.showLibraryReview({ level: "series", seriesRoot: key, season: null, episodePath: sample || null });
  }

  function openSeasonReview(root: string, season: number) {
    const group = seriesGroupForRoot(root);
    if (!group) return;
    const { key } = ensureSeasonReview(group, season);
    const posters = posterPathsForReview(group, season);
    const sample = group.seasons.find((item) => item.season === season)?.episodePaths[0] ?? group.episodePaths[0] ?? "";
    for (const path of posters) if (sample) void h.ensureQueueArtworkPreview(sample, path);
    void h.showLibraryReview({ level: "season", seriesRoot: key, season, episodePath: sample || null });
  }

  function activeReviewPosterPath(target: LibraryTvReviewTarget, review: QueueSeriesReview | undefined): string {
    const group = seriesGroupForRoot(target.seriesRoot);
    if (target.level === "series") {
      return chosenPoster(review?.localPosterPath ?? "")
        || (group ? h.libraryLocalPosters[queuePathKey(group.root)] ?? "" : "")
        || providerPosterUrl(review);
    }
    const directory = group?.seasons.find((item) => item.season === target.season)?.directory ?? "";
    return seasonArtworkPath(review, target.season ?? 0, directory);
  }

  function openLibraryReview(path: string) {
    const card = h.libraryPosterCards.find((item) => item.path === path);
    if (card?.kind === "series") {
      openSeriesReview(path);
      return;
    }
    if (card?.kind === "season" && card.seasonNumber !== null && h.libraryBrowse) {
      openSeasonReview(h.libraryBrowse.seriesRoot, card.seasonNumber);
    }
  }

  function reviewCurrentBrowse() {
    if (!h.libraryBrowse) return;
    if (h.libraryBrowse.season === null) openSeriesReview(h.libraryBrowse.seriesRoot);
    else openSeasonReview(h.libraryBrowse.seriesRoot, h.libraryBrowse.season);
  }

  function setMovieComment(path: string, value: string) {
    const review = h.queueMovieReviews[path];
    if (!review) return;
    const globalComment = h.settings?.metadata.comment?.trim() ?? "";
    const next = value.trim() === globalComment ? "" : value;
    h.queueMovieReviews = { ...h.queueMovieReviews, [path]: { ...review, comment: next } };
  }

  function setReviewComment(target: LibraryTvReviewTarget, value: string) {
    const review = h.queueSeriesReviews[seriesReviewStorageKey(target.seriesRoot)];
    if (!review) return;
    const next = value.trim() === sharedCommentFor(target, review) ? "" : value;
    patchTvField(target, "comment", next);
  }

  function selectReviewPoster(target: LibraryTvReviewTarget, path: string) {
    const sample = target.episodePath ?? seriesGroupForRoot(target.seriesRoot)?.episodePaths[0] ?? "";
    if (sample && !path.startsWith("https://")) void h.ensureQueueArtworkPreview(sample, path);
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
    const next = { ...h.fileTuneOverrides };
    for (const path of paths) next[path] = animated ? "animation" : "none";
    h.fileTuneOverrides = next;
  }

  function patchSeriesReview(root: string, update: (review: QueueSeriesReview) => QueueSeriesReview) {
    const key = seriesReviewStorageKey(root);
    const group = seriesGroupForRoot(root);
    const current = h.queueSeriesReviews[key] ?? (group ? blankSeriesReview(group) : null);
    if (!current) return;
    h.queueSeriesReviews = { ...h.queueSeriesReviews, [key]: update(current) };
  }

  function patchTvField(target: LibraryTvReviewTarget, field: string, value: string) {
    patchSeriesReview(target.seriesRoot, (review) => applyTvField(review, target, field, value, episodeDraft));
  }

  function tvReviewHeading(target: LibraryTvReviewTarget): string {
    return tvReviewHeadingFor(target, h.queueSeriesReviews[seriesReviewStorageKey(target.seriesRoot)], episodeDraft);
  }

  function tvReviewPaths(target: LibraryTvReviewTarget): string[] {
    return tvReviewPathsFor(target, seriesGroupForRoot(target.seriesRoot));
  }

  return {
    ensureSeriesReview, ensureSeasonReview, ensureEpisodeReview, tvReviewForEpisode, seriesGroupForRoot, openSeriesReview, openSeasonReview,
    activeReviewPosterPath, openLibraryReview, reviewCurrentBrowse, setMovieComment, setReviewComment, selectReviewPoster, setPathsAnimated,
    patchSeriesReview, patchTvField, tvReviewHeading, tvReviewPaths, posterPathsForReview, reviewPosterChoices, sharedCommentFor, reviewedSeriesForPath,
  };
}
