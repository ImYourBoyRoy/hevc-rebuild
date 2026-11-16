// ./desktop/src/lib/review-factories.ts
// Factories for blank library review records and episode drafts, extracted from the route.
// Functions that need route state read it through getters at call time so Svelte `$state`
// reads stay reactive. No IPC, no DOM. Tested by pnpm test:review-factories.

import { episodeIdentity, type LibrarySeriesGroup } from './library-browse.ts';
import { queuePathKey } from './queue-paths.ts';
import type { MediaEntry, QueueSeasonReview, QueueSeriesReview, QueueTvEpisode } from './types.js';

export function blankSeasonReview(title: string, series: QueueSeriesReview | undefined): QueueSeasonReview {
  return {
    title,
    year: series?.year ?? "",
    certification: series?.certification ?? "",
    plot: "",
    genresText: series?.genresText ?? "",
    ratingText: "",
  };
}

export function containerTags(entry: MediaEntry | undefined): Record<string, string> {
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

export function createReviewFactories(
  getQueuedEntries: () => MediaEntry[],
  getSeriesReviews: () => Record<string, QueueSeriesReview>,
  getLocalPosters: () => Record<string, string>,
) {
  function blankSeriesReview(group: LibrarySeriesGroup): QueueSeriesReview {
    return {
      title: group.folderName,
      year: "",
      certification: "",
      recordId: "",
      shortDescription: "",
      plot: "",
      genresText: "",
      ratingText: "",
      sourceUrl: "",
      artworkCandidates: [],
      posterId: "",
      localPosterPath: getLocalPosters()[queuePathKey(group.root)] ?? "",
      localSeasonPosterPaths: {},
      seasonRecords: {},
      episodes: [],
      episodeAssignments: {},
    };
  }

  function seriesReviewStorageKey(root: string): string {
    return Object.keys(getSeriesReviews()).find((key) => queuePathKey(key) === queuePathKey(root)) ?? root;
  }

  function episodeFileFacts(path: string): { title: string; plot: string; airdate: string | null } {
    const entry = getQueuedEntries().find((item) => item.path === path);
    const tags = containerTags(entry);
    const title = tags.title && tags.title !== tags.showtitle ? tags.title : "";
    const plot = tags.synopsis || tags.description || "";
    const dated = tags.date_released || "";
    const seriesDir = path.split(/[\\/]/u).slice(0, -2).join("/");
    const dates = getQueuedEntries()
      .filter((item) => seriesDir && item.path.startsWith(`${seriesDir}/`))
      .map((item) => containerTags(item).date_released)
      .filter((value): value is string => Boolean(value));
    const airdate = dated && new Set(dates).size > 1 ? dated : null;
    return { title, plot, airdate };
  }

  function episodeDraft(path: string): QueueTvEpisode {
    const entry = getQueuedEntries().find((item) => item.path === path);
    const identity = episodeIdentity(entry?.name ?? "");
    const file = episodeFileFacts(path);
    return {
      season: identity?.season ?? 0,
      episode: identity?.episode && identity.episode > 0 ? identity.episode : 1,
      title: file.title || identity?.title || entry?.name || "Episode",
      airdate: file.airdate,
      summary: file.plot,
      sourceUrl: "",
    };
  }

  return { blankSeriesReview, seriesReviewStorageKey, episodeFileFacts, episodeDraft };
}
