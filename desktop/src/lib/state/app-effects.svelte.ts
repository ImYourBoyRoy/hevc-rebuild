// ./desktop/src/lib/state/app-effects.svelte.ts
// The route's `$effect` blocks, moved out: local-poster scan, automatic poster downloads, paging clamp,
// select-all indeterminate state, debounced queue persistence (selection, movie and series reviews),
// artwork-cache refresh on the Settings page and the run progress clock. They are installed by one
// call from the component script, so they are created in the component's context in the same order
// as before. The state instances and the few route functions they use arrive in one context object.
import type { AppState } from "./app-state.svelte.ts";
import type { ScanState } from "./scan-state.svelte.ts";
import type { ReviewState } from "./review-state.svelte.ts";
import type { LibraryUiState } from "./library-ui-state.svelte.ts";
import type { RunUiState } from "./run-ui-state.svelte.ts";
import type { createWorkspaceState } from "../workspace.svelte.js";
import type { AppDerived } from "./app-derived.svelte.ts";
import type { QueueMovieReview, QueueSeriesReview } from "../types.js";
import type { LocalLibraryRecord } from "../review-notes.ts";
import type { createPosterResolvers } from "../poster-urls.ts";
import type { createQueuePersistence } from "../queue-persistence.ts";
import type { createArtworkController } from "../artwork-controller.ts";
import { queuePathKey } from "../queue-paths.js";
import { preferredLocalPoster, seasonPosterFromArtwork } from "../library-browse.js";
import { remoteArtworkUrl, providerPosterUrl, posterCandidateForUrl } from "../poster-urls.js";
import { uniquePosterChoices } from "../queue-document.js";

export type LocalAssetRecord = {
  path: string;
  artwork: { name: string; path: string; role: string }[];
};

export interface AppEffectsContext {
  workspace: ReturnType<typeof createWorkspaceState>;
  derived: AppDerived;
  appState: AppState;
  scanState: ScanState;
  reviewState: ReviewState;
  libraryUi: LibraryUiState;
  runUi: RunUiState;
  invoke: <T>(command: string, args?: Record<string, unknown>) => Promise<T>;
  localRecordFromAsset: (item: LocalAssetRecord) => LocalLibraryRecord;
  seriesReviewStorageKey: (root: string) => string;
  ensureQueueArtworkPreview: (mediaPath: string, artworkPath: string) => Promise<unknown> | unknown;
  reviewedSeriesForPath: (path: string) => QueueSeriesReview | undefined;
  seriesSuggestionArtwork: ReturnType<typeof createPosterResolvers>["seriesSuggestionArtwork"];
  persistQueueSelection: ReturnType<typeof createQueuePersistence>["persistQueueSelection"];
  persistQueueMovieReviews: ReturnType<typeof createQueuePersistence>["persistQueueMovieReviews"];
  persistQueueSeriesReviews: ReturnType<typeof createQueuePersistence>["persistQueueSeriesReviews"];
  refreshArtworkCache: ReturnType<typeof createArtworkController>["refreshArtworkCache"];
}

export function installAppEffects(ctx: AppEffectsContext) {
  const { workspace, derived, appState, scanState, reviewState, libraryUi, runUi, invoke, localRecordFromAsset, seriesReviewStorageKey, ensureQueueArtworkPreview, reviewedSeriesForPath, seriesSuggestionArtwork, persistQueueSelection, persistQueueMovieReviews, persistQueueSeriesReviews, refreshArtworkCache } = ctx;
  let artworkCacheSettingsActive = false;
  const automaticPosterDownloads = new Set<string>();

  $effect(() => {
    const scan = scanState.scanResult;
    const groups = derived.librarySeriesGroups;
    if (!scan || !groups.length) return;
    const directoryPaths = [...new Set(groups.flatMap((group) => [group.root, ...group.seasons.map((season) => season.directory).filter(Boolean)]))];
    const mediaPaths = groups.flatMap((group) => group.episodePaths);
    let cancelled = false;
    void invoke<{ directories: LocalAssetRecord[]; files: LocalAssetRecord[] }>("scan_local_library_assets", {
      request: { root: scan.root, mediaPaths, directoryPaths },
    }).then((found) => {
      if (cancelled) return;
      const next: Record<string, string> = {};
      const records: typeof libraryUi.libraryLocalRecords = {};
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
      libraryUi.libraryLocalPosters = next;
      libraryUi.libraryLocalRecords = records;
      for (const group of groups) {
        const storage = seriesReviewStorageKey(group.root);
        const review = reviewState.queueSeriesReviews[storage];
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
        if (postersChanged) reviewState.queueSeriesReviews = { ...reviewState.queueSeriesReviews, [storage]: { ...review, localSeasonPosterPaths: seasonPaths } };
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
        libraryUi.libraryLocalPosters = {};
        libraryUi.libraryLocalRecords = {};
      }
    });
    return () => {
      cancelled = true;
    };
  });
  $effect(() => {
    if (!appState.desktopShell || !scanState.scanResult || !appState.appPreferences.savePosterBesideMedia || !derived.librarySeriesGroups.length) return;
    const libraryRoot = scanState.scanResult.root;
    for (const group of derived.librarySeriesGroups) {
      const review = reviewedSeriesForPath(group.episodePaths[0] ?? group.root);
      const candidate = review?.artworkCandidates.find((image) => remoteArtworkUrl(image)) ?? seriesSuggestionArtwork(group);
      if (!candidate) continue;
      if (review && !providerPosterUrl(review)) {
        const storage = seriesReviewStorageKey(group.root);
        const current = reviewState.queueSeriesReviews[storage];
        if (current && !providerPosterUrl(current)) {
          reviewState.queueSeriesReviews = {
            ...reviewState.queueSeriesReviews,
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
      if (!libraryUi.libraryLocalPosters[seriesKey] && !(savedSeriesPoster && !savedSeriesPoster.startsWith("https://"))) {
        const token = `series:${seriesKey}:${candidate.id}`;
        if (!automaticPosterDownloads.has(token)) {
          automaticPosterDownloads.add(token);
          const sample = group.episodePaths[0] ?? "";
          void invoke<{ files: { path: string }[] }>("save_library_artwork", {
            request: { root: libraryRoot, candidate, targets: [{ directory: group.root, basename: "poster" }] },
          }).then((result) => {
            const saved = result.files[0]?.path ?? "";
            if (!saved) return;
            libraryUi.libraryLocalPosters = { ...libraryUi.libraryLocalPosters, [seriesKey]: saved };
            const storage = seriesReviewStorageKey(group.root);
            const current = reviewState.queueSeriesReviews[storage];
            if (current) {
              const currentPoster = current.localPosterPath ?? "";
              reviewState.queueSeriesReviews = {
                ...reviewState.queueSeriesReviews,
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
        if (!seasonUrl.startsWith("https://") || libraryUi.libraryLocalPosters[queuePathKey(season.directory)]) continue;
        const seasonCandidate = posterCandidateForUrl(candidate, seasonUrl, `${review.title || group.folderName} season ${season.season} poster`, `season-${season.season}`);
        if (!seasonCandidate) continue;
        const seasonKey = queuePathKey(season.directory);
        const token = `season:${seasonKey}:${seasonCandidate.id}`;
        if (automaticPosterDownloads.has(token)) continue;
        automaticPosterDownloads.add(token);
        const sample = season.episodePaths[0] ?? group.episodePaths[0] ?? "";
        void invoke<{ files: { path: string }[] }>("save_library_artwork", {
          request: { root: libraryRoot, candidate: seasonCandidate, targets: [{ directory: season.directory, basename: "poster" }] },
        }).then((result) => {
          const saved = result.files[0]?.path ?? "";
          if (!saved) return;
          libraryUi.libraryLocalPosters = { ...libraryUi.libraryLocalPosters, [seasonKey]: saved };
          const storage = seriesReviewStorageKey(group.root);
          const current = reviewState.queueSeriesReviews[storage];
          const currentSeason = current?.localSeasonPosterPaths?.[String(season.season)] ?? "";
          if (current && (currentSeason.startsWith("https://") || !currentSeason)) {
            reviewState.queueSeriesReviews = {
              ...reviewState.queueSeriesReviews,
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
  $effect(() => {
    if (libraryUi.mediaPage > derived.mediaPageCount) libraryUi.mediaPage = derived.mediaPageCount;
  });
  $effect(() => {
    if (libraryUi.visibleSelectAllCheckbox) {
      libraryUi.visibleSelectAllCheckbox.indeterminate = derived.selectedVisibleCount > 0 && derived.selectedVisibleCount < derived.currentPageEntries.length;
    }
  });
  $effect(() => {
    const selectedSnapshot = [...workspace.queue.selectedPaths];
    const activeSnapshot = workspace.queue.activeRoot;
    const tuneSnapshot = JSON.parse(JSON.stringify(runUi.fileTuneOverrides)) as Record<string, "animation" | "none">;
    const streamSnapshot = JSON.parse(JSON.stringify(runUi.fileStreamOverrides)) as Record<string, number[]>;
    if (!appState.desktopShell || !workspace.queue.loaded) return;
    const timer = window.setTimeout(() => {
      void persistQueueSelection(selectedSnapshot, activeSnapshot, tuneSnapshot, streamSnapshot);
    }, 200);
    return () => window.clearTimeout(timer);
  });
  $effect(() => {
    const snapshot = JSON.parse(JSON.stringify(reviewState.queueMovieReviews)) as Record<string, QueueMovieReview>;
    if (!appState.desktopShell || !workspace.queue.loaded) return;
    const timer = window.setTimeout(() => {
      void persistQueueMovieReviews(snapshot);
    }, 450);
    return () => window.clearTimeout(timer);
  });
  $effect(() => {
    const snapshot = JSON.parse(JSON.stringify(reviewState.queueSeriesReviews)) as Record<string, QueueSeriesReview>;
    if (!appState.desktopShell || !workspace.queue.loaded) return;
    const timer = window.setTimeout(() => {
      void persistQueueSeriesReviews(snapshot);
    }, 450);
    return () => window.clearTimeout(timer);
  });
  $effect(() => {
    const active = appState.desktopShell && appState.page === "settings";
    if (active && !artworkCacheSettingsActive) void refreshArtworkCache();
    artworkCacheSettingsActive = active;
  });
  $effect(() => {
    if (!workspace.run.id) return;
    const timer = setInterval(() => {
      runUi.progressClock = Date.now();
    }, 1000);
    return () => clearInterval(timer);
  });
}
