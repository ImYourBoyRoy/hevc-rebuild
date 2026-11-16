// ./desktop/src/lib/match-lookup.ts
// Automatic match lookup extracted from MetadataWorkbench (T10.7 slice 8): looking up the analyzed films and series folders one by one
// (guards, nearby artwork check, which items are skipped, title cache, automatic association, per-item failures, progress and status
// reports, stopping early and the closing message). The state and props are reached through a typed host and every dependent action is
// injected so every branch is unit-tested in node (pnpm test:match-lookup). The code is the component's code moved verbatim.

import type { MetadataLookupStatus, QueueMetadataSuggestion, ScanResult } from "./types.js";
import type { AnalysisSeriesGroup, FilenameClue, LocalAssetsScan, MatchSuggestion, MediaKind, MetadataSources, MovieAssignment, OrganizerFile, OrganizerScan, Provider, PublicSearchResponse, Result, SeriesAssignment, TvdbCredentialStatus } from "./metadata-workbench-types.js";
import { candidatePreview } from "./metadata-candidates.ts";
import { automaticMetadataMatch, normalizeMatchTitle } from "./metadata-match.ts";
import { analyzedLibraryRoot, basename, seriesFilenameTitle } from "./organizer-analysis.ts";

export interface MatchLookupHost {
  readonly scanResult: ScanResult | null;
  readonly metadataSources: MetadataSources;
  readonly tvdbCredentialStatus: TvdbCredentialStatus;
  readonly movieAssignments: Record<string, MovieAssignment>;
  readonly seriesAssignments: Record<string, SeriesAssignment>;
  organizerSource: "analysis" | "folder" | null;
  organizerScan: OrganizerScan | null;
  localAssetScanError: string;
  lookupBusy: boolean;
  lookupProgress: { done: number; total: number; label: string } | null;
  lookupMessage: string;
  lookupMessageScope: "analyzed films" | "series folders" | null;
  movieSuggestions: Record<string, MatchSuggestion>;
  seriesSuggestions: Record<string, MatchSuggestion>;
  cancelLookupRequested: boolean;
  lastLookupStopped: boolean;
}

export interface MatchLookupActions {
  ensureLocalAssets(root: string, files: OrganizerFile[], directories?: string[]): Promise<LocalAssetsScan | null>;
  filenameClue(filename: string): Promise<FilenameClue>;
  searchPublicTitles(kind: MediaKind, title: string): Promise<PublicSearchResponse>;
  rankByFilenameYear(rows: Result[], clue: FilenameClue, kind?: MediaKind): Result[];
  autoAssignMovieMatch(path: string, clue: FilenameClue, candidate: Result): boolean;
  autoAssignSeriesMatch(root: string, clue: FilenameClue, candidate: Result): boolean;
  hydrateSeriesProvider(root: string, row: Result): Promise<boolean>;
  suggestionForPath(path: string): QueueMetadataSuggestion | undefined;
  onLookupStatus(status: MetadataLookupStatus): void;
  onMatchSuggestion(mediaPath: string, suggestion: QueueMetadataSuggestion): void;
}

export interface MatchLookupDeps {
  isDesktop: () => boolean;
  host: MatchLookupHost;
  actions: MatchLookupActions;
}

export function createMatchLookup(deps: MatchLookupDeps) {
  const { host: h, actions, isDesktop } = deps;
  const { ensureLocalAssets, filenameClue, searchPublicTitles, rankByFilenameYear, autoAssignMovieMatch, autoAssignSeriesMatch, hydrateSeriesProvider, suggestionForPath, onLookupStatus, onMatchSuggestion } = actions;

  async function findMovieMatches(files: OrganizerFile[], analysisRoot?: string) {
    if (!isDesktop() || h.lookupBusy || !files.length) return;
    const root = analysisRoot ?? (h.scanResult && h.organizerSource === "analysis" ? analyzedLibraryRoot(h.scanResult) : h.organizerScan?.root);
    if (!root) return;
    h.cancelLookupRequested = false;
    h.lastLookupStopped = false;
    h.lookupBusy = true;
    h.lookupProgress = { done: 0, total: files.length, label: "analyzed films" };
    onLookupStatus({ status: "searching", ...h.lookupProgress, label: "films", message: "Checking nearby poster art before public searches.", sourceRoot: root });
    const assets = await ensureLocalAssets(root, files, [root]);
    if (!assets) {
      h.lookupBusy = false;
      h.lookupProgress = null;
      h.lookupMessage = `Could not check nearby artwork: ${h.localAssetScanError || "local asset scan failed"}`;
      h.lookupMessageScope = "analyzed films";
      onLookupStatus({ status: "error", done: 0, total: files.length, label: "films", message: h.lookupMessage, sourceRoot: root });
      return;
    }
    const targets = files.filter((file) => {
      if (!file.path || !file.name) return false;
      const reviewedAssignment = Boolean(h.movieAssignments[file.path]?.recordId);
      const savedSuggestion = suggestionForPath(file.path);
      const savedQueueResult = analysisRoot
        && (savedSuggestion?.status === "ready" || reviewedAssignment);
      return !savedQueueResult;
    });
    if (!targets.length) {
      h.lookupBusy = false;
      h.lookupProgress = null;
      const savedResultsCoverFiles = analysisRoot && files.every((file) =>
        suggestionForPath(file.path)?.status === "ready" || Boolean(h.movieAssignments[file.path]?.recordId),
      );
      h.lookupMessage = savedResultsCoverFiles
        ? "Saved title suggestions or reviewed details are already available. Open a file to compare other sources."
        : "No movie files were available for a public search.";
      h.lookupMessageScope = "analyzed films";
      onLookupStatus({ status: "ready", done: files.length, total: files.length, label: "films", message: h.lookupMessage, sourceRoot: root });
      return;
    }
    h.lookupMessage = "";
    h.lookupMessageScope = null;
    h.lookupProgress = { done: 0, total: targets.length, label: "analyzed films" };
    onLookupStatus({ status: "searching", ...h.lookupProgress, label: "films", message: "Checking likely film titles and poster art.", sourceRoot: root });
    const titleCache = new Map<string, PublicSearchResponse>();
    let completed = 0;
    let failed = 0;
    for (const file of targets) {
      if (h.cancelLookupRequested) break;
      try {
        const clue = await filenameClue(file.name);
        h.movieSuggestions = {
          ...h.movieSuggestions,
          [file.path]: { clue, results: [], warnings: [], status: "searching", error: "" },
        };
        onMatchSuggestion(file.path, { kind: "movie", status: "searching", title: clue.title, year: clue.year, provider: null, recordId: null, sourceUrl: null, description: null, autoAssociated: false, artwork: null, error: "" });
        const cacheKey = clue.title.normalize("NFKC").toLocaleLowerCase();
        let response = titleCache.get(cacheKey);
        if (!response) {
          // The provider receives only the cleaned title. Filename, year, path,
          // and media bytes stay local; the year is used only for local ranking.
          response = await searchPublicTitles("movie", clue.title);
          titleCache.set(cacheKey, response);
        }
        const rankedResults = rankByFilenameYear(response.results, clue, "movie");
        const autoMatch = automaticMetadataMatch(rankedResults, { ...clue, kind: "movie" });
        const autoAssociated = autoMatch ? autoAssignMovieMatch(file.path, clue, autoMatch) : false;
        h.movieSuggestions = {
          ...h.movieSuggestions,
          [file.path]: { clue, results: rankedResults, warnings: response.warnings, status: "ready", error: "" },
        };
        const likely = autoMatch ?? rankedResults[0];
        const poster = likely ? candidatePreview(likely) ?? null : null;
        onMatchSuggestion(file.path, {
          kind: "movie", status: "ready", title: likely?.title ?? null,
          year: likely?.year ?? null, provider: likely?.provider ?? null,
          recordId: likely ? `${likely.provider}:${likely.id}` : null,
          sourceUrl: likely?.sourceUrl ?? null, description: likely?.description ?? null,
          autoAssociated, artwork: poster, error: "",
        });
      } catch (cause) {
        failed += 1;
        const previous = h.movieSuggestions[file.path];
        h.movieSuggestions = {
          ...h.movieSuggestions,
          [file.path]: {
            clue: previous?.clue ?? { title: file.name, year: null, yearEnd: null, certification: null },
            results: [], warnings: [], status: "error", error: String(cause),
          },
        };
        onMatchSuggestion(file.path, { kind: "movie", status: "error", title: null, year: null, provider: null, recordId: null, sourceUrl: null, description: null, autoAssociated: false, artwork: null, error: String(cause) });
      }
      completed += 1;
      h.lookupProgress = { done: completed, total: targets.length, label: "analyzed films" };
      onLookupStatus({ status: "searching", ...h.lookupProgress, label: "films", message: `Checked ${completed} of ${targets.length} films.`, sourceRoot: root });
    }
    const stopped = h.cancelLookupRequested;
    h.lastLookupStopped = stopped;
    h.lookupBusy = false;
    h.lookupProgress = null;
    h.cancelLookupRequested = false;
    h.lookupMessage = stopped
      ? `Stopped after ${completed} of ${targets.length} film files.`
      : `Checked ${completed} film file${completed === 1 ? "" : "s"}${failed ? `; ${failed} lookup${failed === 1 ? "" : "s"} failed` : ""}. Exact title-and-year matches without a conflict were associated; review the rest.`;
    h.lookupMessageScope = "analyzed films";
    onLookupStatus({ status: failed === completed && failed > 0 ? "error" : "ready", done: completed, total: targets.length, label: "films", message: h.lookupMessage, sourceRoot: root });
  }

  async function findSeriesMatches(groups: AnalysisSeriesGroup[], analysisRoot?: string) {
    if (!isDesktop() || h.lookupBusy || !groups.length) return;
    const assetRoot = analysisRoot ?? (h.scanResult && h.organizerSource === "analysis" ? analyzedLibraryRoot(h.scanResult) : h.organizerScan?.root ?? groups[0]?.root);
    const allFiles = groups.flatMap((group) => group.files);
    const allDirectories = groups.flatMap((group) => [group.root, ...group.seasonDirectories.map((season) => season.path)]);
    if (!assetRoot) return;
    h.cancelLookupRequested = false;
    h.lastLookupStopped = false;
    h.lookupBusy = true;
    h.lookupProgress = { done: 0, total: groups.length, label: "series folders" };
    onLookupStatus({ status: "searching", ...h.lookupProgress, label: "series", message: "Checking nearby poster art before public searches.", sourceRoot: assetRoot });
    if (!assetRoot || !await ensureLocalAssets(assetRoot, allFiles, allDirectories)) {
      h.lookupBusy = false;
      h.lookupProgress = null;
      h.lookupMessage = `Could not check nearby artwork: ${h.localAssetScanError || "local asset scan failed"}`;
      h.lookupMessageScope = "series folders";
      onLookupStatus({ status: "error", done: 0, total: groups.length, label: "series", message: h.lookupMessage, sourceRoot: assetRoot ?? undefined });
      return;
    }
    const targets = groups.filter((group) => {
      if (group.ambiguous || !group.files.length) return false;
      const recordId = h.seriesAssignments[group.root]?.recordId ?? "";
      const savedProvider = Boolean(recordId) && !recordId.startsWith("local");
      const hasProviderArt = (h.seriesAssignments[group.root]?.artworkCandidates ?? []).some((image) => image.imageUrl.startsWith("https://") || image.thumbnailUrl.startsWith("https://"));
      const seasonNumbers = group.seasonDirectories.map((item) => item.season).filter((number) => number > 0);
      const seasonPaths = h.seriesAssignments[group.root]?.localSeasonPosterPaths ?? {};
      const hasSeasonArt = seasonNumbers.length
        ? seasonNumbers.every((number) => (seasonPaths[String(number)] ?? "").startsWith("https://"))
        : Object.values(seasonPaths).some((path) => path.startsWith("https://"));
      const hasCertificate = Boolean((h.seriesAssignments[group.root]?.certification ?? "").trim());
      const savedQueueResult = analysisRoot && savedProvider && hasProviderArt && hasSeasonArt && hasCertificate;
      return !savedQueueResult;
    });
    if (!targets.length) {
      h.lookupBusy = false;
      h.lookupProgress = null;
      const savedResultsCoverGroups = analysisRoot && groups.every((group) =>
        Boolean(h.seriesAssignments[group.root]?.recordId)
        || group.files.every((file) => suggestionForPath(file.path)?.status === "ready"),
      );
      h.lookupMessage = savedResultsCoverGroups
        ? "Saved title suggestions or reviewed show details are already available. Open a series to compare other sources."
        : "No show-specific series folders were recognized in this analysis.";
      h.lookupMessageScope = "series folders";
      onLookupStatus({ status: "ready", done: groups.length, total: groups.length, label: "series", message: h.lookupMessage, sourceRoot: assetRoot });
      return;
    }
    h.lookupMessage = "";
    h.lookupMessageScope = null;
    h.lookupProgress = { done: 0, total: targets.length, label: "series folders" };
    onLookupStatus({ status: "searching", ...h.lookupProgress, label: "series", message: "Checking likely series titles and poster art.", sourceRoot: assetRoot });
    const titleCache = new Map<string, PublicSearchResponse>();
    let completed = 0;
    let failed = 0;
    for (const group of targets) {
      if (h.cancelLookupRequested) break;
      const root = group.root;
      try {
        const clue = await filenameClue(seriesFilenameTitle(group));
        h.seriesSuggestions = {
          ...h.seriesSuggestions,
          [root]: { clue, results: [], warnings: [], status: "searching", error: "" },
        };
        for (const file of group.files) {
          onMatchSuggestion(file.path, { kind: "tv", status: "searching", title: clue.title, year: clue.year, provider: null, recordId: null, sourceUrl: null, description: null, autoAssociated: false, artwork: null, error: "" });
        }
        const cacheKey = clue.title.normalize("NFKC").toLocaleLowerCase();
        let response = titleCache.get(cacheKey);
        if (!response) {
          // Only the cleaned show title leaves the device; the folder path and
          // local year hints are excluded from the provider request.
          response = await searchPublicTitles("tv", clue.title);
          titleCache.set(cacheKey, response);
        }
        const rankedResults = rankByFilenameYear(response.results, clue, "tv");
        const autoMatch = automaticMetadataMatch(rankedResults, { ...clue, kind: "tv" });
        const exactProvider = (wanted: Provider) => rankedResults.find((row) => row.provider === wanted && normalizeMatchTitle(row.title) === normalizeMatchTitle(clue.title));
        const detailSource = (h.metadataSources.tvdb && h.tvdbCredentialStatus.configured ? exactProvider("tvdb") : undefined) ?? autoMatch;
        const autoAssociated = autoMatch ? autoAssignSeriesMatch(root, clue, autoMatch) : false;
        if (detailSource) {
          void hydrateSeriesProvider(root, detailSource).then((loaded) => {
            if (!loaded && autoMatch && autoMatch !== detailSource) void hydrateSeriesProvider(root, autoMatch);
          });
        }
        h.seriesSuggestions = {
          ...h.seriesSuggestions,
          [root]: { clue, results: rankedResults, warnings: response.warnings, status: "ready", error: "" },
        };
        const likely = autoMatch ?? rankedResults[0];
        const poster = likely ? candidatePreview(likely) ?? null : null;
        for (const file of group.files) {
          onMatchSuggestion(file.path, {
            kind: "tv", status: "ready", title: likely?.title ?? null,
            year: likely?.year ?? null, provider: likely?.provider ?? null,
            recordId: likely ? `${likely.provider}:${likely.id}` : null,
            sourceUrl: likely?.sourceUrl ?? null, description: likely?.description ?? null,
            autoAssociated, artwork: poster, error: "",
          });
        }
      } catch (cause) {
        failed += 1;
        const previous = h.seriesSuggestions[root];
        h.seriesSuggestions = {
          ...h.seriesSuggestions,
          [root]: {
            clue: previous?.clue ?? { title: basename(root), year: null, yearEnd: null, certification: null },
            results: [], warnings: [], status: "error", error: String(cause),
          },
        };
        for (const file of group.files) {
          onMatchSuggestion(file.path, { kind: "tv", status: "error", title: null, year: null, provider: null, recordId: null, sourceUrl: null, description: null, autoAssociated: false, artwork: null, error: String(cause) });
        }
      }
      completed += 1;
      h.lookupProgress = { done: completed, total: targets.length, label: "series folders" };
      onLookupStatus({ status: "searching", ...h.lookupProgress, label: "series", message: `Checked ${completed} of ${targets.length} series.`, sourceRoot: assetRoot });
    }
    const stopped = h.cancelLookupRequested;
    h.lastLookupStopped = stopped;
    h.lookupBusy = false;
    h.lookupProgress = null;
    h.cancelLookupRequested = false;
    h.lookupMessage = stopped
      ? `Stopped after ${completed} of ${targets.length} series folders.`
      : `Checked ${completed} series folder${completed === 1 ? "" : "s"}${failed ? `; ${failed} lookup${failed === 1 ? "" : "s"} failed` : ""}. Exact title-and-year matches without a conflict were associated; review the rest.`;
    h.lookupMessageScope = "series folders";
    onLookupStatus({ status: failed === completed && failed > 0 ? "error" : "ready", done: completed, total: targets.length, label: "series", message: h.lookupMessage, sourceRoot: assetRoot });
  }

  function stopFilenameLookup() {
    h.cancelLookupRequested = true;
  }

  return { findMovieMatches, findSeriesMatches, stopFilenameLookup };
}
