// ./desktop/src/lib/organizer-session.ts
// Organizer session extracted from MetadataWorkbench (T10.7 slice 12): using the analyzed library, restoring and rebuilding a saved series
// review, choosing an analyzed series, resuming provider artwork for queued series, choosing and rescanning organizer folders and changing the
// media kind. The state and props are reached through a typed host and the bridge, dialog and every dependent action are injected so every
// branch is unit-tested in node (pnpm test:organizer-session). The code is the component's code moved verbatim.

import type { QueueTvEpisode, ScanResult } from "./types.js";
import type { Detail, Episode, FilenameClue, LocalAssetsScan, MatchSuggestion, MediaKind, OrganizerFile, OrganizerScan, PendingArtworkSave, Provider, RenamePreview, Result, SearchTarget, SeriesAssignment, AnalysisSeriesGroup } from "./metadata-workbench-types.js";
import { providerEpisodeTextReady, suggestionDetail } from "./metadata-candidates.ts";
import { analysisOrganizerScan, basename, buildAnalysisSeriesGroups } from "./organizer-analysis.ts";

export interface OrganizerSessionHost {
  readonly scanResult: ScanResult | null;
  readonly metadataAutoLookup: boolean;
  seriesAssignments: Record<string, SeriesAssignment>;
  selectedMediaPaths: string[];
  organizerSource: "analysis" | "folder" | null;
  selectedAnalysisSeriesRoot: string;
  organizerScan: OrganizerScan | null;
  episodeAssignments: Record<string, string>;
  selectedArtwork: Record<string, string>;
  tvRenameScope: "series-tree" | "episodes-only";
  seriesYearConflictConfirmed: boolean;
  query: string;
  mediaKind: MediaKind;
  provider: Provider;
  selected: Result | null;
  detail: Detail | null;
  results: Result[];
  searchWarnings: string[];
  searchTarget: SearchTarget | null;
  error: string;
  renamePreview: RenamePreview | null;
  renameError: string;
  renameBusy: boolean;
  confirmRename: boolean;
  pendingArtworkSave: PendingArtworkSave | null;
  artworkError: string;
  artworkMessage: string;
  lookupBusy: boolean;
  localAssets: LocalAssetsScan | null;
  localAssetGeneration: number;
  localArtworkPreviews: Record<string, string>;
  localArtworkPreviewErrors: Record<string, string>;
}

export interface OrganizerSessionActions {
  openDialog(options: Record<string, unknown>): Promise<string | string[] | null>;
  seedMovieDrafts(files: OrganizerFile[]): Promise<void>;
  syncEpisodeAssignments(files: OrganizerFile[]): void;
  filenameClue(filename: string): Promise<FilenameClue>;
  episodeForKey(key: string | undefined): Episode | undefined;
  ensureLocalAssets(root: string, files: OrganizerFile[], directories?: string[]): Promise<LocalAssetsScan | null>;
  findMovieMatches(files: OrganizerFile[], analysisRoot?: string): Promise<void>;
  findSeriesMatches(groups: AnalysisSeriesGroup[], analysisRoot?: string): Promise<void>;
  suggestionForPath(path: string): import("./types.js").QueueMetadataSuggestion | undefined;
  seedSeriesAssignment(root: string, row: Result): void;
  hydrateSeriesProvider(root: string, row: Result): Promise<boolean>;
}

export interface OrganizerSessionDeps {
  invoke: <T>(command: string, args?: Record<string, unknown>) => Promise<T>;
  isDesktop: () => boolean;
  host: OrganizerSessionHost;
  actions: OrganizerSessionActions;
}

export function createOrganizerSession(deps: OrganizerSessionDeps) {
  const { host: h, actions, invoke, isDesktop } = deps;
  const { openDialog, seedMovieDrafts, syncEpisodeAssignments, filenameClue, episodeForKey, ensureLocalAssets, findMovieMatches, findSeriesMatches, suggestionForPath, seedSeriesAssignment, hydrateSeriesProvider } = actions;
  const seriesArtworkLoads = new Set<string>();

  function useAnalyzedLibrary(kind: MediaKind, seriesRoot?: string) {
    if (!h.scanResult) return;
    h.organizerSource = "analysis";
    const groups = kind === "tv" ? buildAnalysisSeriesGroups(h.scanResult) : [];
    const targetSeriesRoot = seriesRoot ?? (groups.length === 1 ? groups[0].root : "");
    h.selectedAnalysisSeriesRoot = targetSeriesRoot;
    h.organizerScan = analysisOrganizerScan(h.scanResult, kind, targetSeriesRoot);
    if (kind === "movie" && h.organizerScan) void seedMovieDrafts(h.organizerScan.files);
    const filenameQuery = kind === "tv"
      ? targetSeriesRoot ? basename(targetSeriesRoot) : ""
      : h.organizerScan?.files.length === 1 ? h.organizerScan.files[0].name : "";
    h.query = filenameQuery;
    if (filenameQuery) {
      void filenameClue(filenameQuery).then((clue) => {
        if (h.query === filenameQuery) h.query = clue.title;
      }).catch(() => {});
    }
    h.episodeAssignments = {};
    if (kind === "tv" && h.organizerScan) {
      const savedReview = h.seriesAssignments[targetSeriesRoot];
      if (savedReview) restoreSavedSeriesReview(savedReview);
      if (h.detail) syncEpisodeAssignments(h.organizerScan.files);
    }
  }

  function restoreSavedSeriesReview(review: SeriesAssignment) {
    const [providerKey, ...recordParts] = review.recordId.split(":");
    const validProviders: Provider[] = ["wikidata", "tvmaze", "tvdb", "tmdb", "local"];
    h.provider = validProviders.includes(providerKey as Provider) ? providerKey as Provider : "local";
    const recordId = recordParts.join(":");
    const episodes: Episode[] = review.episodes.map((episode) => ({ ...episode, artwork: [] }));
    const seasonNumbers = [...new Set(episodes.map((episode) => episode.season))].sort((left, right) => left - right);
    h.selected = {
      id: recordId,
      provider: h.provider,
      kind: "tv",
      title: review.title,
      year: review.year || null,
      description: review.plot || review.shortDescription || null,
      rating: review.ratingText ? Number(review.ratingText) : null,
      sourceUrl: review.sourceUrl,
      confidence: 1,
      artwork: review.artworkCandidates,
    };
    h.detail = {
      title: review.title,
      originalTitle: null,
      year: review.year || null,
      sourceUrl: review.sourceUrl,
      shortDescription: review.shortDescription,
      longDescription: review.plot,
      certification: review.certification || null,
      runtime: null,
      genres: review.genresText.split(/[;,]/).map((genre) => genre.trim()).filter(Boolean),
      rating: review.ratingText ? Number(review.ratingText) : null,
      artwork: review.artworkCandidates,
      seasons: seasonNumbers.map((number) => ({ number, title: `Season ${number}`, yearStart: null, yearEnd: null, artwork: [] })),
      episodes,
      attribution: `Saved review from ${h.provider === "local" ? "an earlier session" : h.provider.toUpperCase()}`,
    };
    h.selectedArtwork = { series: review.posterId };
    h.episodeAssignments = Object.fromEntries(Object.entries(review.episodeAssignments).map(([path, episode]) => [
      path,
      `${episode.season}/${episode.episode}`,
    ]));
  }

  function makeCurrentSeriesReview(): SeriesAssignment | null {
    if (!h.detail || !h.selected || h.mediaKind !== "tv") return null;
    const root = h.searchTarget?.kind === "tv" ? h.searchTarget.root : h.organizerScan?.root || h.selectedAnalysisSeriesRoot;
    if (!root) return null;
    const previous = h.seriesAssignments[root];
    const episodeFromDetail = (episode: Episode): QueueTvEpisode => ({
      season: episode.season,
      episode: episode.episode,
      title: episode.title,
      airdate: episode.airdate,
      summary: episode.summary,
      sourceUrl: episode.sourceUrl,
    });
    const assignedEpisodes: Record<string, QueueTvEpisode> = { ...(previous?.episodeAssignments ?? {}) };
    for (const [path, key] of Object.entries(h.episodeAssignments)) {
      if (!key) {
        delete assignedEpisodes[path];
        continue;
      }
      const episode = episodeForKey(key);
      if (episode && !assignedEpisodes[path]) assignedEpisodes[path] = episodeFromDetail(episode);
    }
    const catalog = h.detail.episodes.length ? h.detail.episodes.map(episodeFromDetail) : previous?.episodes ?? [];
    for (const [path, episode] of Object.entries(assignedEpisodes)) {
      if (episode.airdate?.trim()) continue;
      const match = catalog.find((item) => item.season === episode.season && item.episode === episode.episode);
      if (match?.airdate?.trim()) assignedEpisodes[path] = { ...episode, airdate: match.airdate };
    }
    const nextRecord = `${h.provider}:${h.selected.id}`;
    const recordId = nextRecord.startsWith("local") && previous?.recordId && !previous.recordId.startsWith("local")
      ? previous.recordId
      : nextRecord;
    return {
      title: h.detail.title,
      year: h.detail.year ?? "",
      certification: h.detail.certification || previous?.certification || "",
      recordId,
      shortDescription: h.detail.shortDescription,
      plot: h.detail.longDescription,
      genresText: h.detail.genres.join(", "),
      ratingText: h.detail.rating === null ? "" : String(h.detail.rating),
      sourceUrl: h.detail.sourceUrl || previous?.sourceUrl || "",
      comment: previous?.comment ?? "",
      artworkCandidates: h.detail.artwork.length ? h.detail.artwork.map((image) => ({ ...image })) : previous?.artworkCandidates ?? [],
      posterId: h.selectedArtwork.series || previous?.posterId || "",
      localPosterPath: previous?.localPosterPath ?? "",
      localSeasonPosterPaths: previous?.localSeasonPosterPaths ?? {},
      seasonRecords: previous?.seasonRecords ?? {},
      episodes: catalog.slice(0, 10_000),
      episodeAssignments: assignedEpisodes,
    };
  }

  function chooseAnalyzedSeries(root: string) {
    h.results = [];
    h.selected = null;
    h.detail = null;
    h.seriesYearConflictConfirmed = false;
    h.error = "";
    h.renamePreview = null;
    h.confirmRename = false;
    h.pendingArtworkSave = null;
    h.artworkError = "";
    h.artworkMessage = "";
    useAnalyzedLibrary("tv", root);
  }

  function resumeSeriesArtwork(scan: ScanResult) {
    if (!isDesktop() || !h.metadataAutoLookup || h.lookupBusy) return;
    for (const group of buildAnalysisSeriesGroups(scan)) {
      if (group.ambiguous || !group.files.length) continue;
      const suggestion = group.files.map((file) => suggestionForPath(file.path)).find((item) => suggestionDetail(item));
      const row = suggestionDetail(suggestion);
      if (!row) continue;
      if (!h.seriesAssignments[group.root]) seedSeriesAssignment(group.root, row);
      const review = h.seriesAssignments[group.root];
      if (!review) continue;
      const hasSeriesArt = review.artworkCandidates.some((image) => image.imageUrl.startsWith("https://") || image.thumbnailUrl.startsWith("https://"));
      const seasonNumbers = group.seasonDirectories.map((item) => item.season).filter((number) => number > 0);
      const hasSeasonArt = seasonNumbers.length > 0 && seasonNumbers.every((number) => {
        const path = review.localSeasonPosterPaths[String(number)] ?? "";
        return path.startsWith("https://") || path.startsWith("/");
      });
      if (hasSeriesArt && hasSeasonArt && providerEpisodeTextReady(review, group)) continue;
      const key = `${group.root}\0${row.provider}:${row.id}`;
      if (seriesArtworkLoads.has(key)) continue;
      seriesArtworkLoads.add(key);
      void hydrateSeriesProvider(group.root, row);
    }
  }

  async function chooseOrganizerFolder() {
    if (!isDesktop()) { h.renameError = "Folder selection is available in the installed desktop app."; return; }
    const path = await openDialog({ directory: true, multiple: false, title: h.mediaKind === "tv" ? "Choose one TV series folder" : "Choose a movie folder" });
    if (!path || Array.isArray(path)) return;
    h.renameBusy = true;
    h.renameError = "";
    h.renamePreview = null;
    h.pendingArtworkSave = null;
    h.artworkError = "";
    h.artworkMessage = "";
    try {
      h.organizerScan = await invoke<OrganizerScan>("scan_organizer_folder", { root: path });
      h.organizerSource = "folder";
      h.selectedAnalysisSeriesRoot = "";
      h.localAssets = null;
      h.localAssetGeneration += 1;
      h.localArtworkPreviews = {};
      h.localArtworkPreviewErrors = {};
      const assets = await ensureLocalAssets(h.organizerScan.root, h.organizerScan.files, [h.organizerScan.root, ...h.organizerScan.seasonDirectories.map((season) => season.path)]);
      if (h.mediaKind === "tv") {
        const episodePaths = h.organizerScan.files.filter((file) => file.season !== null && file.episode !== null && !file.multipleEpisodes).map((file) => file.path);
        h.selectedMediaPaths = [...new Set([...h.selectedMediaPaths, ...episodePaths])];
      } else if (assets) {
        await seedMovieDrafts(h.organizerScan.files);
      }
      syncEpisodeAssignments(h.organizerScan.files);
      if (h.metadataAutoLookup && assets) {
        if (h.mediaKind === "movie") void findMovieMatches(h.organizerScan.files);
        else void findSeriesMatches([{ root: h.organizerScan.root, files: h.organizerScan.files, seasonDirectories: h.organizerScan.seasonDirectories, ambiguous: false }]);
      }
    } catch (cause) {
      h.organizerScan = null;
      h.renameError = String(cause);
    } finally {
      h.renameBusy = false;
    }
  }

  async function scanCurrentSeriesFolder() {
    if (!isDesktop() || !h.organizerScan || h.mediaKind !== "tv") return;
    const path = h.organizerScan.root;
    h.renameBusy = true;
    h.renameError = "";
    h.renamePreview = null;
    h.confirmRename = false;
    try {
      h.organizerScan = await invoke<OrganizerScan>("scan_organizer_folder", { root: path });
      h.organizerSource = "folder";
      h.localAssets = null;
      h.localAssetGeneration += 1;
      h.localArtworkPreviews = {};
      h.localArtworkPreviewErrors = {};
      const assets = await ensureLocalAssets(h.organizerScan.root, h.organizerScan.files, [h.organizerScan.root, ...h.organizerScan.seasonDirectories.map((season) => season.path)]);
      const episodePaths = h.organizerScan.files
        .filter((file) => file.season !== null && file.episode !== null && !file.multipleEpisodes)
        .map((file) => file.path);
      h.selectedMediaPaths = [...new Set([...h.selectedMediaPaths, ...episodePaths])];
      syncEpisodeAssignments(h.organizerScan.files);
      if (h.metadataAutoLookup && assets) {
        void findSeriesMatches([{ root: h.organizerScan.root, files: h.organizerScan.files, seasonDirectories: h.organizerScan.seasonDirectories, ambiguous: false }]);
      }
    } catch (cause) {
      h.renameError = String(cause);
    } finally {
      h.renameBusy = false;
    }
  }

  function changeMediaKind(kind: MediaKind) {
    h.mediaKind = kind;
    h.query = "";
    h.results = [];
    h.searchWarnings = [];
    h.searchTarget = null;
    h.selected = null;
    h.detail = null;
    h.error = "";
    h.organizerScan = null;
    h.organizerSource = null;
    h.tvRenameScope = "series-tree";
    h.episodeAssignments = {};
    h.renamePreview = null;
    h.renameError = "";
    h.confirmRename = false;
    h.pendingArtworkSave = null;
    h.artworkError = "";
    h.artworkMessage = "";
    if (h.scanResult) useAnalyzedLibrary(kind);
  }

  return { useAnalyzedLibrary, restoreSavedSeriesReview, makeCurrentSeriesReview, chooseAnalyzedSeries, resumeSeriesArtwork, chooseOrganizerFolder, scanCurrentSeriesFolder, changeMediaKind };
}
