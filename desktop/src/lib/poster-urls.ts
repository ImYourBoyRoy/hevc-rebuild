// ./desktop/src/lib/poster-urls.ts
// Poster and artwork URL resolvers for the library views, extracted from the route.
// The resolvers that depend on route state read it through getters at call time, so
// Svelte `$state` reads inside them stay reactive. No IPC, no DOM.
// Tested by pnpm test:poster-urls.

import { queuePathKey } from './queue-paths.ts';
import type { QueueArtworkCandidate, QueueMetadataSuggestion, QueueSeriesReview } from './types.js';

export function posterCandidateForUrl(base: QueueArtworkCandidate, imageUrl: string, title: string, idSuffix: string): QueueArtworkCandidate | null {
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

export function providerPosterUrl(review: QueueSeriesReview | undefined): string {
  return review?.artworkCandidates.find((image) => image.imageUrl.startsWith("https://"))?.imageUrl
    || review?.artworkCandidates.find((image) => image.thumbnailUrl.startsWith("https://"))?.thumbnailUrl
    || "";
}

export function remoteArtworkUrl(candidate: QueueArtworkCandidate | null | undefined): string {
  if (!candidate) return "";
  return candidate.imageUrl.startsWith("https://") ? candidate.imageUrl : candidate.thumbnailUrl.startsWith("https://") ? candidate.thumbnailUrl : "";
}

export function createPosterResolvers(
  getArtworkPreviews: () => Record<string, string>,
  getLocalPosters: () => Record<string, string>,
  getMatchSuggestions: () => Record<string, QueueMetadataSuggestion>,
) {
  function localPosterUrl(path: string): string | null {
    return path ? getArtworkPreviews()[queuePathKey(path)] || null : null;
  }

  function displayPosterUrl(path: string): string | null {
    if (/^https:\/\//u.test(path)) return path;
    return localPosterUrl(path);
  }

  function knownLocalPoster(path: string): string {
    if (!path || path.startsWith("https://")) return "";
    const key = queuePathKey(path);
    return Object.values(getLocalPosters()).some((item) => queuePathKey(item) === key) ? path : "";
  }

  function chosenPoster(path: string): string {
    if (!path) return "";
    if (path.startsWith("https://")) return path;
    if (knownLocalPoster(path)) return path;
    return getArtworkPreviews()[queuePathKey(path)] ? path : "";
  }

  function seriesSuggestionArtwork(group: { episodePaths: string[] }): QueueArtworkCandidate | null {
    for (const path of group.episodePaths) {
      const artwork = getMatchSuggestions()[path]?.artwork;
      if (remoteArtworkUrl(artwork)) return artwork ?? null;
    }
    return null;
  }

  function visiblePosterUrl(localPath: string, remotePath: string): string | null {
    const localPreview = localPath && !localPath.startsWith("https://") ? displayPosterUrl(localPath) : null;
    return localPreview || displayPosterUrl(remotePath) || displayPosterUrl(localPath);
  }

  function seasonArtworkPath(review: QueueSeriesReview | undefined, season: number, directory: string): string {
    const onDisk = directory ? getLocalPosters()[queuePathKey(directory)] ?? "" : "";
    const saved = chosenPoster(review?.localSeasonPosterPaths?.[String(season)] ?? "");
    return onDisk || saved || providerPosterUrl(review);
  }

  function seasonPosterSources(review: QueueSeriesReview | undefined, season: number, directory: string): { localPath: string; remotePath: string } {
    const onDisk = directory ? getLocalPosters()[queuePathKey(directory)] ?? "" : "";
    const saved = review?.localSeasonPosterPaths?.[String(season)] ?? "";
    const remotePath = saved.startsWith("https://") ? saved : "";
    const localPath = onDisk || (saved.startsWith("https://") ? "" : chosenPoster(saved));
    return { localPath, remotePath };
  }

  return { localPosterUrl, displayPosterUrl, knownLocalPoster, chosenPoster, seriesSuggestionArtwork, visiblePosterUrl, seasonArtworkPath, seasonPosterSources };
}
