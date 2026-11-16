// ./desktop/src/lib/library-cards.ts
// Poster-card builders for the Library grid, extracted from the route (poster-card slice): media,
// series, season and episode cards, the animation/selection tri-states, the browse views (root,
// series, season), the breadcrumb trail and the lookup banner. The route's state is read through
// injected getters, so every branch is unit-tested in node (pnpm test:library-cards). The card code
// is the route's code moved verbatim; reads of route state became calls to the getters.

import type { AppPreferences } from './ui-contracts.js';
import type {
  LibraryCrumb, LibraryPosterCard, MediaEntry, MediaQueueRunState, MetadataLookupStatus, QueueMetadataSuggestion, QueueMovieReview, QueueSeriesReview, Settings,
} from './types.js';
import { episodeIdentity, type LibrarySeriesGroup } from './library-browse.ts';
import { bytes } from './format.ts';
import { proposedFilenameForEntry } from './filename-template.ts';
import { queuePathKey } from './queue-paths.ts';
import { queueRunStatusLabel } from './queue-run-state.ts';

export interface LibraryBrowseState { seriesRoot: string; season: number | null }
export interface RunProgressLike { filePath?: string | null; phase?: string; filePercent?: number | null }

export interface LibraryCardsDeps {
  getQueueMovieReviews(): Record<string, QueueMovieReview>;
  getQueueSeriesReviews(): Record<string, QueueSeriesReview>;
  getQueueMatchSuggestions(): Record<string, QueueMetadataSuggestion>;
  getQueueArtworkPreviews(): Record<string, string>;
  getLibraryLocalPosters(): Record<string, string>;
  getAppPreferences(): AppPreferences;
  getMetadataLookupStatus(): MetadataLookupStatus | null;
  getQueuedEntries(): MediaEntry[];
  getFileTuneOverrides(): Record<string, string>;
  getSelectedPaths(): string[];
  getRunProgress(): RunProgressLike | null;
  getLibraryBrowse(): LibraryBrowseState | null;
  setLibraryBrowse(value: LibraryBrowseState | null): void;
  getLibrarySeriesGroups(): LibrarySeriesGroup[];
  getVisibleEntries(): MediaEntry[];
  getCurrentPageEntries(): MediaEntry[];
  reviewedSeriesForPath(path: string): QueueSeriesReview | undefined;
  seriesReviewStorageKey(root: string): string;
  settingsForEntry(entry: MediaEntry): Settings | null | undefined;
  resourceForecastFor(entry: MediaEntry): MediaEntry['resource_forecast'];
  queueRunStateForPath(path: string): MediaQueueRunState | undefined;
  effectiveContentTuneForEntry(entry: MediaEntry, review: QueueMovieReview | QueueSeriesReview | undefined, settings: Settings | null | undefined): string;
  statusLabel(status: string): string;
  profileSummary(entry: MediaEntry): string;
  chosenPoster(path: string): string;
  providerPosterUrl(review: QueueSeriesReview | undefined): string;
  remoteArtworkUrl(image: unknown): string;
  seriesSuggestionArtwork(group: LibrarySeriesGroup): { provider?: string } | null | undefined;
  visiblePosterUrl(localPath: string, remotePath: string): string | null;
  seasonPosterSources(review: QueueSeriesReview | undefined, season: number, directory: string): { localPath: string; remotePath: string };
  seasonArtworkPath(review: QueueSeriesReview | undefined, season: number, directory: string): string;
  displayPosterUrl(path: string): string | null;
}

export function createLibraryCards(d: LibraryCardsDeps) {
  const {
    reviewedSeriesForPath, seriesReviewStorageKey, settingsForEntry, resourceForecastFor, queueRunStateForPath, effectiveContentTuneForEntry, statusLabel, profileSummary,
    chosenPoster, providerPosterUrl, remoteArtworkUrl, seriesSuggestionArtwork, visiblePosterUrl, seasonPosterSources, seasonArtworkPath, displayPosterUrl,
  } = d;

  function libraryPosterCard(entry: MediaEntry): LibraryPosterCard {
    const reviewedMovie = d.getQueueMovieReviews()[entry.path];
    const reviewedSeries = reviewedSeriesForPath(entry.path);
    const reviewed = reviewedMovie ?? reviewedSeries;
    const suggestion = d.getQueueMatchSuggestions()[entry.path];
    const appPreferences = d.getAppPreferences();
    const selectedPaths = d.getSelectedPaths();
    const proposal = proposedFilenameForEntry(entry, reviewedMovie, reviewedSeries, suggestion, appPreferences.organizerTemplates);
    const reviewedPoster = reviewed?.artworkCandidates.find((candidate) => candidate.id === reviewed.posterId);
    const localArtworkPath = reviewed?.localPosterPath ?? "";
    const localPreview = localArtworkPath ? d.getQueueArtworkPreviews()[queuePathKey(localArtworkPath)] : "";
    const suggestionArt = suggestion?.artwork?.thumbnailUrl || suggestion?.artwork?.imageUrl || "";
    const reviewedArt = reviewedPoster?.thumbnailUrl || reviewedPoster?.imageUrl || "";
    const posterUrl = localPreview || reviewedArt || suggestionArt || null;
    const title = reviewed?.title.trim()
      || (suggestion?.status === "ready" ? suggestion.title?.trim() ?? "" : "")
      || entry.name.replace(/\.[^.]+$/, "");
    const year = reviewed?.year || (suggestion?.status === "ready" ? suggestion.year ?? "" : "");
    let matchState: LibraryPosterCard["matchState"] = "none";
    let matchLabel = posterUrl ? "Poster ready" : "No artwork yet";
    if (suggestion?.status === "searching" && !reviewed) {
      matchState = "matching";
      matchLabel = "Matching";
    } else if (suggestion?.status === "error" && !reviewed) {
      matchState = "error";
      matchLabel = "Lookup failed";
    } else if (reviewed || suggestion?.autoAssociated) {
      matchState = "matched";
      matchLabel = "Matched";
    } else if (suggestion?.status === "ready" && suggestion.title) {
      matchState = "suggested";
      matchLabel = "Suggested";
    } else if (d.getMetadataLookupStatus()?.status === "searching" || suggestion?.status === "searching") {
      matchState = "matching";
      matchLabel = "Matching";
    }
    const provider = reviewed
      ? reviewed.recordId.split(":")[0].toUpperCase()
      : suggestion?.provider?.toUpperCase() ?? "";
    const tune = effectiveContentTuneForEntry(entry, reviewed, settingsForEntry(entry));
    const forecast = resourceForecastFor(entry);
    const queueRun = queueRunStateForPath(entry.path);
    const runProgress = d.getRunProgress();
    const rowProgress = runProgress?.filePath && queuePathKey(runProgress.filePath) === queuePathKey(entry.path) ? runProgress : null;
    const profile = entry.profile;
    return {
      path: entry.path,
      filename: entry.name,
      relativePath: entry.relative_path,
      selected: selectedPaths.includes(entry.path),
      title,
      year,
      proposedName: proposal?.name && proposal.name !== entry.name ? proposal.name : null,
      proposedNeedsReview: proposal?.needsTitleReview ?? false,
      posterUrl,
      localArtworkPath,
      matchState,
      matchLabel,
      provider,
      animated: tune === "animation",
      statusLabel: statusLabel(entry.status),
      sourceBytes: bytes(entry.size_bytes),
      profileLine: profileSummary(entry),
      streamLine: profile
        ? `${profile.video.secondary_count + 1} video · ${profile.audio_count} audio · ${profile.subtitle_count} subtitles · ${profile.attachment_count} attachments`
        : "Streams not available yet",
      memoryLine: forecast
        ? `${forecast.level} · ${forecast.planned_workers ?? "—"} workers · ${forecast.planned_frame_threads ?? "—"} frame threads`
        : "",
      outputLine: appPreferences.fileOutputDirs[entry.path]
        ? `Output: ${appPreferences.fileOutputDirs[entry.path]}`
        : appPreferences.rebuildOutputDir
          ? `Default output: ${appPreferences.rebuildOutputDir}`
          : "Output: source folder",
      outputIsCustom: Boolean(appPreferences.fileOutputDirs[entry.path]),
      error: entry.error ?? "",
      warning: entry.warning ?? "",
      stale: entry.queue_stale ?? "",
      runLabel: queueRun ? `${queueRunStatusLabel(queueRun.status)}${queueRun.mode ? ` · ${queueRun.mode === "organize" ? "Rename" : queueRun.mode}` : ""}` : "",
      progressLabel: rowProgress?.phase ?? "",
      progressPercent: rowProgress?.filePercent ?? null,
      kind: "media",
      seasonNumber: null,
      mediaPaths: [entry.path],
      selection: selectedPaths.includes(entry.path) ? "all" : "none",
      animation: animationSelection([entry.path]),
      summary: "",
      reviewLabel: "",
    };
  }

  function animationSelection(paths: string[]): LibraryPosterCard["animation"] {
    if (!paths.length) return "none";
    const marked = paths.filter((path) => {
      const entry = d.getQueuedEntries().find((item) => item.path === path);
      if (!entry) return d.getFileTuneOverrides()[path] === "animation";
      return effectiveContentTuneForEntry(entry, d.getQueueMovieReviews()[path] ?? reviewedSeriesForPath(path), settingsForEntry(entry)) === "animation";
    }).length;
    if (!marked) return "none";
    return marked === paths.length ? "all" : "some";
  }

  function cardSelection(paths: string[]): LibraryPosterCard["selection"] {
    const selected = paths.filter((path) => d.getSelectedPaths().includes(path)).length;
    if (!selected) return "none";
    return selected === paths.length ? "all" : "some";
  }

  function seriesProviderLabel(group: LibrarySeriesGroup, review: QueueSeriesReview | undefined): string {
    const record = review?.recordId ?? "";
    if (record && !record.startsWith("local")) return record.split(":")[0].toUpperCase();
    const provider = seriesSuggestionArtwork(group)?.provider ?? "";
    return provider ? provider.toUpperCase() : "";
  }

  function groupPosterCard(group: LibrarySeriesGroup): LibraryPosterCard {
    const review = reviewedSeriesForPath(group.episodePaths[0] ?? group.root);
    const localPath = d.getLibraryLocalPosters()[queuePathKey(group.root)] || chosenPoster((review?.localPosterPath ?? "").startsWith("https://") ? "" : review?.localPosterPath ?? "");
    const remotePath = providerPosterUrl(review) || remoteArtworkUrl(seriesSuggestionArtwork(group));
    const artworkPath = localPath || remotePath;
    const seasonCount = group.seasons.length;
    const episodeCount = group.episodePaths.length;
    return {
      ...libraryPosterCard({
        path: group.root,
        relative_path: group.folderName,
        name: group.folderName,
        size_bytes: 0,
        status: "Ready",
        already_processed: false,
        profile: null,
        error: null,
      }),
      path: group.root,
      filename: group.folderName,
      title: review?.title.trim() || group.folderName,
      year: review?.year ?? "",
      posterUrl: visiblePosterUrl(localPath, remotePath),
      localArtworkPath: artworkPath,
      provider: seriesProviderLabel(group, review),
      matchState: artworkPath ? "local" : "none",
      matchLabel: "",
      kind: "series",
      seasonNumber: null,
      mediaPaths: group.episodePaths,
      selection: cardSelection(group.episodePaths),
      animation: animationSelection(group.episodePaths),
      summary: `${seasonCount} season${seasonCount === 1 ? "" : "s"} · ${episodeCount} episode${episodeCount === 1 ? "" : "s"}`,
      reviewLabel: "Review series",
      proposedName: null,
    };
  }

  function seasonPosterCard(group: LibrarySeriesGroup, season: LibrarySeriesGroup["seasons"][number]): LibraryPosterCard {
    const review = reviewedSeriesForPath(season.episodePaths[0] ?? group.root);
    const sources = seasonPosterSources(review, season.season, season.directory);
    const artworkPath = sources.localPath || sources.remotePath;
    const waitingForSeasonPoster = !artworkPath && Boolean(seriesSuggestionArtwork(group) || review?.recordId);
    const episodeCount = season.episodePaths.length;
    return {
      ...groupPosterCard(group),
      path: season.directory || `${group.root}::season:${season.season}`,
      filename: season.label,
      title: season.label,
      year: "",
      posterUrl: visiblePosterUrl(sources.localPath, sources.remotePath),
      localArtworkPath: artworkPath,
      matchLabel: "",
      matchState: artworkPath ? "local" : waitingForSeasonPoster ? "matched" : "none",
      kind: "season",
      seasonNumber: season.season,
      mediaPaths: season.episodePaths,
      selection: cardSelection(season.episodePaths),
      animation: animationSelection(season.episodePaths),
      summary: `${episodeCount} episode${episodeCount === 1 ? "" : "s"}`,
      reviewLabel: "Review season",
    };
  }

  function episodePosterCard(entry: MediaEntry, artworkPath: string): LibraryPosterCard {
    const card = libraryPosterCard(entry);
    const identity = episodeIdentity(entry.name);
    const seasonPoster = artworkPath ? displayPosterUrl(artworkPath) : "";
    return {
      ...card,
      title: identity?.title || card.title,
      year: "",
      posterUrl: seasonPoster || card.posterUrl,
      localArtworkPath: artworkPath || card.localArtworkPath,
      matchState: artworkPath || card.posterUrl ? (card.matchState === "none" ? "local" : card.matchState) : card.matchState,
      matchLabel: card.matchState === "matched" || card.matchState === "suggested" || card.matchState === "matching" || card.matchState === "error" ? card.matchLabel : "",
    };
  }

  // The route's `libraryPosterCards` derived: the root grid, a series' seasons, or a season's episodes.
  function buildLibraryPosterCards(): LibraryPosterCard[] {
    const librarySeriesGroups = d.getLibrarySeriesGroups();
    const libraryBrowse = d.getLibraryBrowse();
    const groupedPaths = new Set(librarySeriesGroups.flatMap((group) => group.episodePaths));
    if (!libraryBrowse) {
      const seriesCards = librarySeriesGroups.map((group) => groupPosterCard(group));
      const movies = d.getCurrentPageEntries().filter((entry) => !groupedPaths.has(entry.path)).map((entry) => libraryPosterCard(entry));
      return [...seriesCards, ...movies];
    }
    const group = librarySeriesGroups.find((item) => queuePathKey(item.root) === queuePathKey(libraryBrowse?.seriesRoot ?? ""));
    if (!group) return [];
    if (libraryBrowse.season === null) return group.seasons.map((season) => seasonPosterCard(group, season));
    const season = group.seasons.find((item) => item.season === libraryBrowse?.season);
    if (!season) return [];
    const seasonReview = reviewedSeriesForPath(season.episodePaths[0] ?? group.root);
    const seasonPoster = seasonArtworkPath(seasonReview, season.season, season.directory);
    return d.getVisibleEntries()
      .filter((entry) => season.episodePaths.includes(entry.path))
      .map((entry) => episodePosterCard(entry, seasonPoster));
  }

  // The route's `libraryCrumbs` derived. The crumb callbacks read the browse state at click time.
  function buildLibraryCrumbs(): LibraryCrumb[] {
    const libraryBrowse = d.getLibraryBrowse();
    if (!libraryBrowse) return [];
    const group = d.getLibrarySeriesGroups().find((item) => queuePathKey(item.root) === queuePathKey(libraryBrowse?.seriesRoot ?? ""));
    const seriesLabel = group ? (reviewedSeriesForPath(group.episodePaths[0] ?? group.root)?.title.trim() || group.folderName) : "Series";
    const crumbs: LibraryCrumb[] = [
      { label: "All media", onSelect: () => { d.setLibraryBrowse(null); } },
      {
        label: seriesLabel,
        onSelect: libraryBrowse.season === null ? null : () => { d.setLibraryBrowse({ seriesRoot: d.getLibraryBrowse()?.seriesRoot ?? "", season: null }); },
      },
    ];
    if (libraryBrowse.season !== null) {
      const season = group?.seasons.find((item) => item.season === libraryBrowse?.season);
      const savedTitle = group ? d.getQueueSeriesReviews()[seriesReviewStorageKey(group.root)]?.seasonRecords?.[String(libraryBrowse.season)]?.title : "";
      crumbs.push({ label: savedTitle || season?.label || `Season ${libraryBrowse.season}`, onSelect: null });
    }
    return crumbs;
  }

  function libraryLookupMessage(): string {
    const status = d.getMetadataLookupStatus();
    return status?.status === "searching"
      ? `Matching titles and posters · ${status.done} of ${status.total}`
      : status?.status === "error"
        ? status.message
        : "";
  }

  return {
    libraryPosterCard, animationSelection, cardSelection, seriesProviderLabel, groupPosterCard, seasonPosterCard, episodePosterCard,
    buildLibraryPosterCards, buildLibraryCrumbs, libraryLookupMessage,
  };
}
