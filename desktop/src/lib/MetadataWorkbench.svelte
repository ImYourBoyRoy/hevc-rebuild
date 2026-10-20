<script lang="ts">
  import { invoke, isTauri } from "@tauri-apps/api/core";
  import type { Attachment } from "svelte/attachments";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { ArrowRight, Database, ExternalLink, FileText, Film, ImagePlus, LoaderCircle, Pencil, Search, Tv, TriangleAlert, X } from "@lucide/svelte";
  import "../styles/components/metadata-review.css";
  import { episodeIdentity, fillEpisodeFromCatalog, fillSeasonFromProvider, preferredLocalPoster } from "#lib/library-browse.js";
  import { automaticMetadataMatch, metadataMatchLabel, normalizeMatchTitle, rankMetadataMatches } from "#lib/metadata-match.js";
  import type { MediaEntry, MetadataLookupStatus, QueueArtworkCandidate, QueueMetadataSuggestion, QueueMovieReview, QueueSeriesReview, QueueTvEpisode, ScanResult } from "#lib/types.js";

  type Provider = "wikidata" | "tvmaze" | "tvdb" | "tmdb" | "local";
  type MetadataSources = { tmdb: boolean; wikidata: boolean; tvmaze: boolean; tvdb: boolean };
  type TmdbCredentialStatus = { configured: boolean; secureStoreAvailable: boolean; message: string | null };
  type TvdbCredentialStatus = { configured: boolean; secureStoreAvailable: boolean; subscriberPinConfigured: boolean; message: string | null };
  type SearchTarget = { kind: "movie"; path: string; label: string } | { kind: "tv"; root: string; label: string };
  type MediaKind = "movie" | "tv";
  type Artwork = QueueArtworkCandidate;
  type Result = {
    id: string;
    provider: Provider;
    kind: MediaKind;
    title: string;
    year: string | null;
    description: string | null;
    rating: number | null;
    sourceUrl: string;
    confidence: number;
    artwork: Artwork[];
  };
  type FilenameClue = { title: string; year: string | null; yearEnd: string | null; certification: string | null };
  type MatchSuggestion = { clue: FilenameClue; results: Result[]; warnings: string[]; status: "searching" | "ready" | "error"; error: string };
  type Season = { number: number | null; title: string | null; yearStart: string | null; yearEnd: string | null; overview?: string | null; rating?: number | null; artwork: Artwork[] };
  type Episode = { season: number; episode: number; title: string; airdate: string | null; summary: string | null; sourceUrl: string; artwork: Artwork[] };
  type Detail = {
    title: string;
    originalTitle: string | null;
    year: string | null;
    sourceUrl: string;
    shortDescription: string;
    longDescription: string;
    certification: string | null;
    runtime: number | null;
    genres: string[];
    rating: number | null;
    artwork: Artwork[];
    seasons: Season[];
    episodes: Episode[];
    attribution: string;
    episodeOrderLabel?: string;
  };
  type OrganizerFile = {
    path: string; name: string; parentPath: string; extension: string;
    season: number | null; episode: number | null; multipleEpisodes: boolean;
  };
  type OrganizerEpisodeGap = { season: number; missingCount: number; examples: number[] };
  type LocalArtworkFile = { path: string; name: string; role: string };
  type LocalNfoMetadata = {
    found: boolean; paths: string[]; title: string | null; showTitle: string | null;
    year: string | null; certification: string | null; shortDescription: string | null;
    plot: string | null; genres: string[]; rating: number | null; source: string | null;
    season: number | null; episode: number | null; airdate: string | null; error: string | null;
  };
  type LocalMediaAssets = { path: string; artwork: LocalArtworkFile[]; metadata: LocalNfoMetadata };
  type LocalDirectoryAssets = { path: string; artwork: LocalArtworkFile[]; metadata: LocalNfoMetadata };
  type LocalAssetsScan = { root: string; files: LocalMediaAssets[]; directories: LocalDirectoryAssets[] };
  type PublicSearchResponse = { results: Result[]; warnings: string[]; successfulProviders: Provider[] };
  type MovieAssignment = QueueMovieReview;
  type SeriesAssignment = QueueSeriesReview;
  type SeasonDirectory = { path: string; name: string; season: number };
  type AnalysisSeriesGroup = { root: string; files: OrganizerFile[]; seasonDirectories: SeasonDirectory[]; ambiguous: boolean };
  type OrganizerScan = {
    root: string; renameRoot: string; files: OrganizerFile[];
    seasonDirectories: SeasonDirectory[]; episodeGaps: OrganizerEpisodeGap[]; errors: string[]; truncated: boolean;
  };
  type TemplateKind = "movie" | "series" | "season" | "episode";
  type TvRenameScope = "series-tree" | "episodes-only";
  type OrganizerTemplates = { movie: string; series: string; season: string; episode: string };
  type ArtworkTarget = { directory: string; basename: string };
  type ArtworkQueueBinding =
    | { kind: "movie"; mediaPath: string; candidateId: string }
    | { kind: "series"; seriesRoot: string; candidateId: string }
    | { kind: "season"; seriesRoot: string; seasonNumber: number; candidateId: string };
  type PendingArtworkSave = { candidate: Artwork; targets: ArtworkTarget[]; label: string; queueBindings: (ArtworkQueueBinding | null)[]; saveBesideMedia: boolean };
  type PendingLocalPoster = { sourcePath: string; targetPath: string; target: ArtworkTarget; mediaPath: string; saveBesideMedia: boolean };
  type ArtworkSaveResult = { files: { path: string; attributionPath: string }[]; bytesPerFile: number; message: string };
  type Props = {
    scanResult?: ScanResult | null;
    focusMediaPath?: string | null;
    compactItemMode?: boolean;
    workbenchActive?: boolean;
    autoLookupPending?: boolean;
    metadataAutoLookup?: boolean;
    metadataLanguage?: string;
    textBlacklistPatterns?: string[];
    savePosterBesideMedia?: boolean;
    metadataSources?: MetadataSources;
    tmdbCredentialStatus?: TmdbCredentialStatus;
    tvdbCredentialStatus?: TvdbCredentialStatus;
    onOpenSettings?: (provider?: "tmdb" | "tvdb") => void;
    onStartMetadataOnly?: (mediaPaths: string[]) => void;
    onRenameState?: (mediaPaths: string[], status: "renaming" | "renamed" | "failed", message?: string) => void;
    onAutoLookupConsumed?: () => void;
    onClaimAutoLookup?: () => boolean;
    onLookupStatus?: (status: MetadataLookupStatus) => void;
    onMatchSuggestion?: (mediaPath: string, suggestion: QueueMetadataSuggestion) => void;
    queueSuggestions?: Record<string, QueueMetadataSuggestion>;
    onPathsRenamed?: (paths: Record<string, string>) => void;
    selectedMediaPaths?: string[];
    movieAssignments?: Record<string, MovieAssignment>;
    seriesAssignments?: Record<string, SeriesAssignment>;
    organizerTemplates?: OrganizerTemplates;
    onOrganizerTemplatesChange?: (templates: OrganizerTemplates) => void;
    animationForPath?: (path: string) => boolean;
    onAnimationChange?: (path: string, animated: boolean) => void;
  };
  type RenameTemplateInput = { sourcePath: string; kind: TemplateKind; template: string; values: Record<string, string> };
  type RenamePreview = {
    previewId: string; root: string; canApply: boolean;
    items: { sourcePath: string; targetPath: string; targetName: string; isDirectory: boolean; unchanged: boolean }[];
    conflicts: { sourcePath: string | null; targetPath: string; reason: string }[];
  };
  type MetadataDocument = {
    sourcePath: string; kind: "movie" | "tv_show" | "tv_episode"; title: string; year: string | null;
    certification: string | null; shortDescription: string | null; plot: string | null; genres: string[];
    rating: number | null; sourceUrl: string; showTitle: string | null; season: number | null;
    episode: number | null; airdate: string | null;
  };
  type MetadataWriteRequest = { root: string; documents: MetadataDocument[] };
  type MetadataWritePreview = {
    previewId: string; root: string; canApply: boolean;
    items: { sourcePath: string; targetPath: string; kind: string; replacesAppSidecar: boolean }[];
  };
  type MetadataWriteResult = { filesWritten: number; filesReplaced: number; items: MetadataWritePreview["items"]; message: string };

  let {
    scanResult = null,
    focusMediaPath = null,
    compactItemMode = false,
    workbenchActive = true,
    autoLookupPending = false,
    metadataAutoLookup = true,
    metadataLanguage = "en-US",
    textBlacklistPatterns = [],
    savePosterBesideMedia = true,
    metadataSources = { tmdb: true, wikidata: true, tvmaze: true, tvdb: false },
    tmdbCredentialStatus = { configured: false, secureStoreAvailable: false, message: null },
    tvdbCredentialStatus = { configured: false, secureStoreAvailable: false, subscriberPinConfigured: false, message: null },
    onOpenSettings = () => {},
    onStartMetadataOnly = () => {},
    onRenameState = () => {},
    onAutoLookupConsumed = () => {},
    onClaimAutoLookup = () => true,
    onLookupStatus = () => {},
    onMatchSuggestion = () => {},
    queueSuggestions = {},
    onPathsRenamed = () => {},
    selectedMediaPaths = $bindable<string[]>([]),
    movieAssignments = $bindable<Record<string, MovieAssignment>>({}),
    seriesAssignments = $bindable<Record<string, SeriesAssignment>>({}),
    organizerTemplates = $bindable({
      movie: "{title} [[({year} {certification})]]{ext}",
      series: "{series_title} [[({year_range})]]",
      season: "Season {season:02}",
      episode: "S{season:02}E{episode:02}.{episode_title}{ext}",
    }),
    onOrganizerTemplatesChange = () => {},
    animationForPath = () => false,
    onAnimationChange = () => {},
  }: Props = $props();

  let provider = $state<Provider>("wikidata");
  let mediaKind = $state<MediaKind>("movie");
  let query = $state("");
  let searching = $state(false);
  let loadingDetail = $state(false);
  let error = $state("");
  let reviewDetailError = $state("");
  let results = $state<Result[]>([]);
  let searchWarnings = $state<string[]>([]);
  let searchTarget = $state<SearchTarget | null>(null);
  let titleSearchInput = $state<HTMLInputElement | null>(null);
  let selected = $state<Result | null>(null);
  let detail = $state<Detail | null>(null);
  let manualSearchClue = $state<FilenameClue | null>(null);
  let movieSuggestions = $state<Record<string, MatchSuggestion>>({});
  let seriesSuggestions = $state<Record<string, MatchSuggestion>>({});
  let expandedMatchLists = $state<Record<string, boolean>>({});
  let movieEditorOpen = $state<Record<string, boolean>>({});
  let lookupBusy = $state(false);
  let lookupProgress = $state<{ done: number; total: number; label: string } | null>(null);
  let lookupMessage = $state("");
  let lookupMessageScope = $state<"analyzed films" | "series folders" | null>(null);
  let cancelLookupRequested = false;
  let lastLookupStopped = false;
  let selectedArtwork = $state<Record<string, string>>({});
  let organizerScan = $state<OrganizerScan | null>(null);
  let localAssets = $state<LocalAssetsScan | null>(null);
  let localAssetScanError = $state("");
  let localAssetGeneration = 0;
  let localArtworkPreviews = $state<Record<string, string>>({});
  let localArtworkPreviewErrors = $state<Record<string, string>>({});
  let localArtworkPreviewBusy = $state<Record<string, boolean>>({});
  let organizerSource = $state<"analysis" | "folder" | null>(null);
  let selectedAnalysisSeriesRoot = $state("");
  let tvRenameScope = $state<TvRenameScope>("series-tree");
  let seriesYearConflictConfirmed = $state(false);
  let episodeAssignments = $state<Record<string, string>>({});
  let renamePreview = $state<RenamePreview | null>(null);
  let renameError = $state("");
  let renameMessage = $state("");
  let renameBusy = $state(false);
  let confirmRename = $state(false);
  let metadataWriteRequest = $state<MetadataWriteRequest | null>(null);
  let metadataWritePreview = $state<MetadataWritePreview | null>(null);
  let metadataWriteError = $state("");
  let metadataWriteMessage = $state("");
  let metadataWriteBusy = $state(false);
  let pendingArtworkSave = $state<PendingArtworkSave | null>(null);
  let artworkBusy = $state(false);
  let artworkError = $state("");
  let artworkMessage = $state("");
  let pendingLocalPoster = $state<PendingLocalPoster | null>(null);
  let pendingBatchLocalPoster = $state<{ sourcePath: string; artworkPath: string; mediaPaths: string[] } | null>(null);
  let localPosterBusy = $state(false);
  let localPosterError = $state("");
  let localPosterMessage = $state("");
  let desktop = $derived(isTauri());
  let analysisSeriesGroups = $derived(scanResult ? buildAnalysisSeriesGroups(scanResult) : []);
  let visibleAnalysisSeriesGroups = $derived(
    compactItemMode && focusMediaPath
      ? analysisSeriesGroups.filter((group) => group.files.some((file) => normalizedPath(file.path) === normalizedPath(focusMediaPath)))
      : analysisSeriesGroups,
  );
  let visibleOrganizerFiles = $derived(
    compactItemMode && focusMediaPath && organizerScan
      ? organizerScan.files.filter((file) => normalizedPath(file.path) === normalizedPath(focusMediaPath))
      : organizerScan?.files ?? [],
  );
  let lastAppliedAnalysis: ScanResult | null = null;
  let lastAppliedKind: MediaKind | null = null;
  let lastAutoLookupKey: string | null = null;
  let lastAutoDetailKey: string | null = null;
  let lastFocusedMediaPath: string | null = null;
  let lastFocusedLookupPath: string | null = null;

  $effect(() => {
    const targetPath = focusMediaPath;
    const currentScan = scanResult;
    if (!targetPath || !currentScan || targetPath === lastFocusedMediaPath) return;
    lastFocusedMediaPath = targetPath;
    const group = buildAnalysisSeriesGroups(currentScan).find((candidate) =>
      candidate.files.some((file) => normalizedPath(file.path) === normalizedPath(targetPath)),
    );
    if (group && !group.ambiguous) {
      changeMediaKind("tv");
      chooseAnalyzedSeries(group.root);
    } else {
      changeMediaKind("movie");
      selectedMediaPaths = [...new Set([...selectedMediaPaths, targetPath])];
    }
  });

  $effect(() => {
    const targetPath = focusMediaPath;
    if (!compactItemMode || !targetPath || !movieAssignments[targetPath] || movieEditorOpen[targetPath]) return;
    movieEditorOpen = { ...movieEditorOpen, [targetPath]: true };
  });

  $effect(() => {
    const targetPath = focusMediaPath;
    const suggestion = targetPath ? suggestionForPath(targetPath) : undefined;
    if (!compactItemMode || !workbenchActive || !targetPath || suggestion?.status !== "ready"
      || !suggestion.provider || !suggestion.recordId) return;
    const key = `${normalizedPath(targetPath)}\0${suggestion.recordId}\0details`;
    if (lastAutoDetailKey === key) return;
    const [providerId, ...recordIdParts] = suggestion.recordId.split(":");
    if (providerId !== suggestion.provider || !recordIdParts.length) return;
    const providerKey = providerId as Provider;
    if (!["tmdb", "tvdb", "tvmaze", "wikidata"].includes(providerKey)) return;
    const currentScan = scanResult;
    const entry = currentScan?.entries.find((item) => normalizedPath(item.path) === normalizedPath(targetPath));
    if (!currentScan || !entry) return;
    const seriesRoot = suggestion.kind === "tv" ? inferredSeriesRoot(currentScan, entry) : "";
    const currentAssignment = suggestion.kind === "movie" ? movieAssignments[targetPath] : seriesAssignments[seriesRoot];
    if (!currentAssignment) return;
    const linkedElsewhere = Boolean(currentAssignment.recordId)
      && !currentAssignment.recordId.startsWith("local")
      && currentAssignment.recordId !== suggestion.recordId;
    if (linkedElsewhere) return;
    lastAutoDetailKey = key;
    if (suggestion.kind === "movie" && (!currentAssignment.recordId || currentAssignment.recordId.startsWith("local"))) {
      const art = suggestion.artwork;
      movieAssignments = {
        ...movieAssignments,
        [targetPath]: {
          ...currentAssignment,
          recordId: suggestion.recordId,
          year: currentAssignment.year.trim() ? currentAssignment.year : suggestion.year ?? "",
          plot: currentAssignment.plot.trim() ? currentAssignment.plot : suggestion.description ?? "",
          shortDescription: currentAssignment.shortDescription.trim() ? currentAssignment.shortDescription : suggestion.description ?? "",
          sourceUrl: currentAssignment.sourceUrl.trim() ? currentAssignment.sourceUrl : suggestion.sourceUrl ?? "",
          artworkCandidates: art && !currentAssignment.artworkCandidates.some((image) => image.id === art.id)
            ? [art, ...currentAssignment.artworkCandidates]
            : currentAssignment.artworkCandidates,
          posterId: currentAssignment.posterId || art?.id || "",
        },
      };
    }
    const row: Result = {
      id: recordIdParts.join(":"),
      provider: providerKey,
      kind: suggestion.kind,
      title: suggestion.title ?? currentAssignment.title,
      year: (suggestion.year ?? currentAssignment.year) || null,
      description: suggestion.description ?? null,
      rating: currentAssignment.ratingText ? Number(currentAssignment.ratingText) : null,
      sourceUrl: suggestion.sourceUrl ?? currentAssignment.sourceUrl,
      confidence: 0,
      artwork: suggestion.artwork ? [suggestion.artwork] : currentAssignment.artworkCandidates,
    };
    queueMicrotask(async () => {
      if (focusMediaPath !== targetPath || !workbenchActive) return;
      if (suggestion.kind === "movie" && providerKey === "tmdb") {
        try {
          await applyTmdbMovieDetails(targetPath, row.id);
        } catch (cause) {
          reviewDetailError = String(cause);
        }
        return;
      }
      await openMatch(row);
      if (focusMediaPath !== targetPath || !workbenchActive) return;
      const loaded = detail;
      if (!loaded || selected?.id !== row.id || selected.provider !== row.provider) return;
      if (suggestion.kind === "movie") {
        const existing = movieAssignments[targetPath];
        if (!existing) return;
        if (existing.recordId && !existing.recordId.startsWith("local") && existing.recordId !== suggestion.recordId) return;
        const artwork = loaded.artwork.length ? loaded.artwork : existing.artworkCandidates;
        const suggestionText = suggestion.description ?? "";
        const plotUntouched = !existing.plot.trim() || existing.plot === suggestionText;
        const summaryUntouched = !existing.shortDescription.trim() || existing.shortDescription === suggestionText;
        movieAssignments = {
          ...movieAssignments,
          [targetPath]: {
            ...existing,
            recordId: suggestion.recordId ?? existing.recordId,
            title: existing.title === suggestion.title || !existing.title.trim() ? loaded.title : existing.title,
            year: existing.year === suggestion.year || !existing.year.trim() ? loaded.year ?? existing.year : existing.year,
            certification: existing.certification || loaded.certification || "",
            shortDescription: summaryUntouched ? loaded.shortDescription || suggestionText || existing.shortDescription : existing.shortDescription,
            plot: plotUntouched ? loaded.longDescription || suggestionText || existing.plot : existing.plot,
            genresText: existing.genresText || loaded.genres.join(", "),
            ratingText: existing.ratingText || (loaded.rating === null ? "" : String(loaded.rating)),
            sourceUrl: existing.sourceUrl.trim() ? existing.sourceUrl : loaded.sourceUrl || suggestion.sourceUrl || "",
            artworkCandidates: artwork,
            posterId: artwork.some((image) => image.id === existing.posterId) ? existing.posterId : artwork[0]?.id ?? "",
          },
        };
        selectedArtwork = { ...selectedArtwork, movie: movieAssignments[targetPath].posterId };
      } else {
        const existing = seriesAssignments[seriesRoot];
        if (!existing || existing.recordId !== suggestion.recordId) return;
        const artwork = loaded.artwork.length ? loaded.artwork : existing.artworkCandidates;
        const suggestionText = suggestion.description ?? "";
        seriesAssignments = {
          ...seriesAssignments,
          [seriesRoot]: {
            ...existing,
            title: existing.title === suggestion.title ? loaded.title : existing.title,
            year: existing.year === suggestion.year ? loaded.year ?? existing.year : existing.year,
            certification: existing.certification || loaded.certification || "",
            shortDescription: existing.shortDescription === suggestionText ? loaded.shortDescription || existing.shortDescription : existing.shortDescription,
            plot: existing.plot === suggestionText ? loaded.longDescription || existing.plot : existing.plot,
            genresText: existing.genresText || loaded.genres.join(", "),
            ratingText: existing.ratingText || (loaded.rating === null ? "" : String(loaded.rating)),
            sourceUrl: loaded.sourceUrl || existing.sourceUrl,
            artworkCandidates: artwork,
            posterId: artwork.some((image) => image.id === existing.posterId) ? existing.posterId : artwork[0]?.id ?? "",
            episodes: existing.episodes.length ? existing.episodes : loaded.episodes.slice(0, 10_000).map((episode) => ({
              season: episode.season,
              episode: episode.episode,
              title: episode.title,
              airdate: episode.airdate,
              summary: episode.summary,
              sourceUrl: episode.sourceUrl,
            })),
          },
        };
        selectedArtwork = { ...selectedArtwork, series: seriesAssignments[seriesRoot].posterId };
      }
    });
  });

  $effect(() => {
    const targetPath = focusMediaPath;
    const currentScan = scanResult;
    const active = workbenchActive;
    const busy = lookupBusy;
    const savedSuggestion = targetPath ? suggestionForPath(targetPath) : undefined;
    if (!active || !targetPath || !currentScan || busy || savedSuggestion?.status !== "ready"
      || targetPath === lastFocusedLookupPath) return;
    const entry = currentScan.entries.find((candidate) => candidate.path === targetPath);
    if (!entry) return;
    const file = organizerFileFromEntry(entry);
    const group = buildAnalysisSeriesGroups(currentScan).find((candidate) =>
      candidate.files.some((candidateFile) => normalizedPath(candidateFile.path) === normalizedPath(targetPath)),
    );
    if (group && !group.ambiguous) {
      if (seriesAssignments[group.root] || seriesSuggestions[group.root]) return;
      lastFocusedLookupPath = targetPath;
      queueMicrotask(() => {
        if (workbenchActive && focusMediaPath === targetPath && !lookupBusy) {
          void findSeriesMatches([group], false, analyzedLibraryRoot(currentScan));
        }
      });
    } else {
      if (movieAssignments[targetPath] || localNfoForFile(targetPath)?.found || movieSuggestions[targetPath]) return;
      lastFocusedLookupPath = targetPath;
      queueMicrotask(() => {
        if (workbenchActive && focusMediaPath === targetPath && !lookupBusy) {
          void findMovieMatches([file], false, analyzedLibraryRoot(currentScan));
        }
      });
    }
  });

  $effect(() => {
    if (mediaKind !== "tv" || !selected || !detail || renameBusy) return;
    const root = searchTarget?.kind === "tv"
      ? searchTarget.root
      : organizerScan?.root || selectedAnalysisSeriesRoot;
    if (!root) return;
    const review = makeCurrentSeriesReview();
    if (!review) return;
    const previous = seriesAssignments[root];
    if (JSON.stringify(previous) !== JSON.stringify(review)) {
      seriesAssignments = { ...seriesAssignments, [root]: review };
    }
  });

  $effect(() => {
    if (!searchTarget) return;
    requestAnimationFrame(() => {
      titleSearchInput?.scrollIntoView({ behavior: "smooth", block: "center" });
      titleSearchInput?.focus();
    });
  });

  $effect(() => {
    const targetPath = focusMediaPath;
    if (!targetPath || organizerSource !== "analysis" || !organizerScan?.files.some((file) => file.path === targetPath)) return;
    queueMicrotask(() => {
      if (focusMediaPath !== targetPath) return;
      const row = [...document.querySelectorAll<HTMLElement>(".metadata-analysis-movie-row[data-media-path]")]
        .find((element) => element.dataset.mediaPath === targetPath);
      if (!row) return;
      row.scrollIntoView({ behavior: "smooth", block: "center" });
      row.classList.add("metadata-focus-target");
      window.setTimeout(() => row.classList.remove("metadata-focus-target"), 1800);
    });
  });

  function normalizedPath(path: string): string {
    const separators = path.replaceAll("\\", "/");
    const networkPrefix = separators.startsWith("//") ? "//" : "";
    return networkPrefix + separators.slice(networkPrefix.length).replace(/\/{2,}/g, "/");
  }

  function parentPath(path: string): string {
    const normalized = normalizedPath(path).replace(/\/$/, "");
    const separator = normalized.lastIndexOf("/");
    if (separator < 0) return "";
    if (separator === 0) return "/";
    if (separator === 2 && /^[a-z]:\//i.test(normalized)) return normalized.slice(0, 3);
    return normalized.slice(0, separator);
  }

  function basename(path: string): string {
    const normalized = normalizedPath(path).replace(/\/$/, "");
    return normalized.slice(normalized.lastIndexOf("/") + 1);
  }

  function joinPath(root: string, child: string): string {
    const normalizedRoot = normalizedPath(root).replace(/\/$/, "");
    if (/^[a-z]:$/i.test(normalizedRoot)) return `${normalizedRoot}/${child}`;
    return normalizedRoot === "/" ? `/${child}` : `${normalizedRoot}/${child}`;
  }

  function seasonNumberFromDirectory(name: string): number | null {
    if (name.toLowerCase() === "specials") return 0;
    const match = name.match(/^season[\s_-]*(\d{1,4})$/i);
    return match ? Number(match[1]) : null;
  }

  function genericSeriesRoot(name: string): boolean {
    return new Set(["tv", "tv shows", "shows", "series", "library", "media", "videos", "movies"])
      .has(name.trim().toLocaleLowerCase());
  }

  function seriesFilenameTitle(group: AnalysisSeriesGroup): string {
    const rootName = basename(group.root);
    if (!genericSeriesRoot(rootName)) return rootName;
    const prefixCounts = new Map<string, number>();
    for (const file of group.files) {
      const stem = file.name.replace(/\.[^.]+$/, "");
      const marker = /(?:^|[\s._-])s\d{1,4}[\s._-]*e\d{1,4}/i.exec(stem);
      if (!marker) continue;
      const prefix = stem.slice(0, marker.index).replace(/[._-]+/g, " ").trim();
      if (prefix) prefixCounts.set(prefix, (prefixCounts.get(prefix) ?? 0) + 1);
    }
    return [...prefixCounts.entries()].sort((left, right) => right[1] - left[1])[0]?.[0] ?? rootName;
  }

  function episodeIdFromName(name: string): { season: number | null; episode: number | null; multipleEpisodes: boolean } {
    const stem = name.replace(/\.[^.]+$/, "");
    const match = stem.match(/(?:^|[^a-z0-9])s(\d{1,4})[\s._-]*e(\d{1,4})(?:[\s._-]*e\d{1,4})?/i);
    if (!match) return { season: null, episode: null, multipleEpisodes: false };
    const compactMatch = match[0].replace(/[\s._-]+/g, "");
    return {
      season: Number(match[1]),
      episode: Number(match[2]),
      multipleEpisodes: /(?:e\d{1,4}){2}$/i.test(compactMatch),
    };
  }

  function organizerFileFromEntry(entry: MediaEntry): OrganizerFile {
    const path = normalizedPath(entry.path);
    const name = entry.name || basename(path);
    const id = episodeIdFromName(name);
    return {
      path,
      name,
      parentPath: parentPath(path),
      extension: name.includes(".") ? name.slice(name.lastIndexOf(".") + 1) : "",
      ...id,
    };
  }

  function isAnalyzedEpisode(scan: ScanResult, entry: MediaEntry): boolean {
    const file = organizerFileFromEntry(entry);
    if (file.season !== null && file.episode !== null) return true;
    if (seasonNumberFromDirectory(basename(analyzedLibraryRoot(scan))) !== null) return true;
    if (scan.input_kind === "file") return false;
    const relativeDirectories = normalizedPath(entry.relative_path).split("/").filter(Boolean).slice(0, -1);
    return relativeDirectories.some((directory) => seasonNumberFromDirectory(directory) !== null);
  }

  function analyzedLibraryRoot(scan: ScanResult): string {
    return normalizedPath(scan.input_kind === "file" ? parentPath(scan.root) : scan.root);
  }

  function inferredSeriesRoot(scan: ScanResult, entry: MediaEntry): string {
    const root = analyzedLibraryRoot(scan);
    const relativeParts = scan.input_kind === "file"
      ? []
      : normalizedPath(entry.relative_path).split("/").filter(Boolean).slice(0, -1);
    const directoryChain = [root];
    for (const part of relativeParts) directoryChain.push(joinPath(directoryChain.at(-1) ?? root, part));

    const firstSeasonDirectory = directoryChain.findIndex((path) => seasonNumberFromDirectory(basename(path)) !== null);
    if (firstSeasonDirectory >= 0) {
      return firstSeasonDirectory === 0
        ? parentPath(directoryChain[0]) || directoryChain[0]
        : directoryChain[firstSeasonDirectory - 1];
    }
    return relativeParts.length ? directoryChain[1] : root;
  }

  function buildAnalysisSeriesGroups(scan: ScanResult): AnalysisSeriesGroup[] {
    const groups = new Map<string, OrganizerFile[]>();
    for (const entry of scan.entries) {
      if (!isAnalyzedEpisode(scan, entry)) continue;
      const root = inferredSeriesRoot(scan, entry);
      const files = groups.get(root) ?? [];
      files.push(organizerFileFromEntry(entry));
      groups.set(root, files);
    }
    return [...groups.entries()].map(([root, files]) => {
      const seasons = new Map<string, SeasonDirectory>();
      for (const file of files) {
        const relativeParent = normalizedPath(file.parentPath).slice(normalizedPath(root).replace(/\/$/, "").length).replace(/^\//, "");
        const firstDirectory = relativeParent.split("/").filter(Boolean)[0];
        if (!firstDirectory) continue;
        const season = seasonNumberFromDirectory(firstDirectory);
        if (season === null) continue;
        const path = joinPath(root, firstDirectory);
        seasons.set(path, { path, name: firstDirectory, season });
      }
      const titlePrefixes = new Set(files.flatMap((file) => {
        const stem = file.name.replace(/\.[^.]+$/, "");
        const marker = /(?:^|[\s._-])s\d{1,4}[\s._-]*e\d{1,4}/i.exec(stem);
        if (!marker) return [];
        const prefix = stem.slice(0, marker.index).replace(/[._-]+/g, " ").trim().toLocaleLowerCase();
        return prefix ? [prefix] : [];
      }));
      const genericRoot = genericSeriesRoot(basename(root));
      const seasonDirectories = [...seasons.values()].sort((left, right) => left.season - right.season);
      return {
        root,
        files: files.sort((left, right) => left.path.localeCompare(right.path)),
        seasonDirectories,
        ambiguous: genericRoot ? titlePrefixes.size !== 1 : seasonDirectories.length === 0 && titlePrefixes.size > 1,
      };
    }).sort((left, right) => left.root.localeCompare(right.root));
  }

  function episodeGapsFromFiles(files: OrganizerFile[]): OrganizerEpisodeGap[] {
    const bySeason = new Map<number, { episodes: Set<number>; hasMultiEpisodeFile: boolean }>();
    for (const file of files) {
      if (file.season === null || file.episode === null) continue;
      const season = bySeason.get(file.season) ?? { episodes: new Set<number>(), hasMultiEpisodeFile: false };
      season.episodes.add(file.episode);
      season.hasMultiEpisodeFile ||= file.multipleEpisodes;
      bySeason.set(file.season, season);
    }
    return [...bySeason].flatMap(([season, observed]) => {
      if (observed.hasMultiEpisodeFile || observed.episodes.size < 2) return [];
      const sorted = [...observed.episodes].sort((left, right) => left - right);
      const first = sorted[0];
      const last = sorted.at(-1) ?? first;
      const present = observed.episodes;
      const examples: number[] = [];
      let missingCount = 0;
      for (let episode = first + 1; episode < last; episode += 1) {
        if (present.has(episode)) continue;
        missingCount += 1;
        if (examples.length < 8) examples.push(episode);
      }
      return missingCount ? [{ season, missingCount, examples }] : [];
    }).sort((left, right) => left.season - right.season);
  }

  function analysisOrganizerScan(scan: ScanResult, kind: MediaKind, seriesRoot = ""): OrganizerScan | null {
    if (!scan.entries.length) return null;
    if (kind === "movie") {
      const root = analyzedLibraryRoot(scan);
      const files = scan.entries.filter((entry) => !isAnalyzedEpisode(scan, entry)).map(organizerFileFromEntry);
      return {
        root,
        renameRoot: root,
        files: files.sort((left, right) => left.path.localeCompare(right.path)),
        seasonDirectories: [],
        episodeGaps: [],
        errors: [],
        truncated: false,
      };
    }
    const group = buildAnalysisSeriesGroups(scan).find((candidate) => candidate.root === seriesRoot);
    if (!group || group.ambiguous) return null;
    return { root: group.root, renameRoot: group.root, files: group.files, seasonDirectories: group.seasonDirectories, episodeGaps: episodeGapsFromFiles(group.files), errors: [], truncated: false };
  }

  async function ensureLocalAssets(root: string, files: OrganizerFile[], directories: string[] = []): Promise<LocalAssetsScan | null> {
    if (!desktop) return null;
    let rootKey = normalizedPath(root);
    const containsPath = (parent: string, child: string) => {
      const normalizedParent = parent.replace(/\/$/, "");
      return child === parent || child.startsWith(`${normalizedParent}/`);
    };
    if (localAssets && containsPath(normalizedPath(localAssets.root), rootKey)) {
      rootKey = normalizedPath(localAssets.root);
    } else if (localAssets && !containsPath(rootKey, normalizedPath(localAssets.root))) {
      localAssets = null;
      localAssetGeneration += 1;
      localArtworkPreviews = {};
      localArtworkPreviewErrors = {};
    } else if (localAssets && containsPath(rootKey, normalizedPath(localAssets.root))
      && normalizedPath(localAssets.root) !== rootKey) {
      localAssets = null;
      localAssetGeneration += 1;
      localArtworkPreviews = {};
      localArtworkPreviewErrors = {};
    }
    if (!localAssets) {
      localAssets = { root: rootKey, files: [], directories: [] };
      localAssetGeneration += 1;
    }
    const generation = localAssetGeneration;
    const knownFiles = new Set(localAssets.files.map((entry) => normalizedPath(entry.path)));
    const knownDirectories = new Set(localAssets.directories.map((entry) => normalizedPath(entry.path)));
    const mediaPaths = files.map((file) => file.path).filter((path) => !knownFiles.has(normalizedPath(path)));
    const directoryPaths = [...new Set(directories.map(normalizedPath))].filter((path) => !knownDirectories.has(path));
    if (!mediaPaths.length && !directoryPaths.length) return localAssets;
    localAssetScanError = "";
    try {
      const found = await invoke<LocalAssetsScan>("scan_local_library_assets", {
        request: { root: rootKey, mediaPaths, directoryPaths },
      });
      if (generation !== localAssetGeneration) return null;
      const mergedFiles = new Map(localAssets?.files.map((entry) => [normalizedPath(entry.path), entry]) ?? []);
      const mergedDirectories = new Map(localAssets?.directories.map((entry) => [normalizedPath(entry.path), entry]) ?? []);
      for (const entry of found.files) mergedFiles.set(normalizedPath(entry.path), entry);
      for (const entry of found.directories) mergedDirectories.set(normalizedPath(entry.path), entry);
      localAssets = { root: found.root, files: [...mergedFiles.values()], directories: [...mergedDirectories.values()] };
      return localAssets;
    } catch (cause) {
      if (generation === localAssetGeneration) localAssetScanError = String(cause);
      return null;
    }
  }

  function localFileAssets(path: string): LocalMediaAssets | undefined {
    return localAssets?.files.find((entry) => normalizedPath(entry.path) === normalizedPath(path));
  }

  function localDirectoryAssets(path: string): LocalDirectoryAssets | undefined {
    return localAssets?.directories.find((entry) => normalizedPath(entry.path) === normalizedPath(path));
  }

  function localNfoForFile(path: string): LocalNfoMetadata | undefined {
    return localFileAssets(path)?.metadata;
  }

  function normalizedSeriesTitle(value: string): string {
    return value.normalize("NFKC").trim().replace(/\s+/g, " ").toLowerCase();
  }

  function localSeriesNfo(root: string): LocalNfoMetadata | undefined {
    const showMetadata = localDirectoryAssets(root)?.metadata;
    const files = organizerScan && normalizedPath(organizerScan.root) === normalizedPath(root)
      ? organizerScan.files
      : analysisSeriesGroups.find((group) => normalizedPath(group.root) === normalizedPath(root))?.files ?? [];
    const fileMetadata = files.map((file) => ({ file, metadata: localNfoForFile(file.path) }));
    const episodeNfos = fileMetadata.filter((item) => item.metadata?.found && Boolean(item.metadata.showTitle));
    const unreadableEpisodeNfos = fileMetadata.filter((item) => item.metadata?.found && item.metadata.error);
    if (showMetadata?.found) {
      if (!showMetadata.error && unreadableEpisodeNfos.length) {
        const examples = unreadableEpisodeNfos.slice(0, 3).map((item) => item.file.name);
        return {
          ...showMetadata,
          error: `The show NFO is readable, but episode NFOs could not be checked: ${examples.join(", ")}${unreadableEpisodeNfos.length > examples.length ? ` and ${unreadableEpisodeNfos.length - examples.length} more` : ""}. Review those files before applying local episode metadata.`,
        };
      }
      const authoritativeTitle = showMetadata.title ?? showMetadata.showTitle;
      const conflicts = authoritativeTitle
        ? episodeNfos.filter((item) => normalizedSeriesTitle(item.metadata?.showTitle ?? "") !== normalizedSeriesTitle(authoritativeTitle))
        : [];
      if (!showMetadata.error && conflicts.length) {
        const examples = conflicts.slice(0, 3).map((item) => `${item.file.name} (${item.metadata?.showTitle})`);
        return {
          ...showMetadata,
          error: `The show NFO identifies “${authoritativeTitle}”, but episode NFOs disagree: ${examples.join("; ")}${conflicts.length > examples.length ? `; and ${conflicts.length - examples.length} more` : ""}. No episode NFO metadata was applied. Review these files or compare a public source.`,
        };
      }
      return showMetadata;
    }
    if (unreadableEpisodeNfos.length) {
      const unreadable = unreadableEpisodeNfos[0].metadata!;
      const examples = unreadableEpisodeNfos.slice(0, 3).map((item) => item.file.name);
      return {
        ...unreadable,
        title: null,
        showTitle: null,
        year: null,
        shortDescription: null,
        plot: null,
        genres: [],
        rating: null,
        source: null,
        error: `Episode NFO metadata could not be read for ${examples.join(", ")}${unreadableEpisodeNfos.length > examples.length ? ` and ${unreadableEpisodeNfos.length - examples.length} more` : ""}. No local series association was chosen. Review the NFO files or compare a public source.`,
      };
    }
    const episodeMetadata = episodeNfos[0]?.metadata;
    if (episodeMetadata) {
      const titles = new Map<string, { title: string; files: string[] }>();
      for (const item of episodeNfos) {
        const title = item.metadata?.showTitle?.trim();
        if (!title) continue;
        const key = normalizedSeriesTitle(title);
        const existing = titles.get(key) ?? { title, files: [] };
        existing.files.push(item.file.name);
        titles.set(key, existing);
      }
      if (titles.size > 1) {
        const examples = [...titles.values()].slice(0, 3).map((entry) => `${entry.title} (${entry.files.slice(0, 2).join(", ")})`);
        return {
          ...episodeMetadata,
          title: null,
          showTitle: null,
          year: null,
          shortDescription: null,
          plot: null,
          genres: [],
          rating: null,
          source: null,
          error: `Episode NFOs identify more than one series: ${examples.join("; ")}. No local series association was chosen. Review the NFO files or compare a public source.`,
        };
      }
      return {
        ...episodeMetadata,
        title: episodeMetadata.showTitle,
        showTitle: null,
        year: null,
        shortDescription: null,
        plot: null,
        genres: [],
        rating: null,
        source: null,
      };
    }
    const existingEpisodeNfo = fileMetadata.find((item) => item.metadata?.found)?.metadata;
    if (!existingEpisodeNfo) return undefined;
    return {
      ...existingEpisodeNfo,
      title: null,
      showTitle: null,
      year: null,
      shortDescription: null,
      plot: null,
      genres: [],
      rating: null,
      source: null,
      error: existingEpisodeNfo.error
        ?? "An episode NFO exists but does not identify its show. Public matching is paused; use Search public alternative to compare public records.",
    };
  }

  function previewLocalArtwork(images: LocalArtworkFile[]): Attachment<HTMLElement> {
    return () => {
      void loadLocalArtworkPreviews(images);
    };
  }

  async function loadLocalArtworkPreviews(images: LocalArtworkFile[]) {
    if (!desktop || !localAssets) return;
    for (const image of images.slice(0, 4)) {
      if (localArtworkPreviews[image.path] || localArtworkPreviewBusy[image.path]) continue;
      localArtworkPreviewBusy = { ...localArtworkPreviewBusy, [image.path]: true };
      localArtworkPreviewErrors = { ...localArtworkPreviewErrors, [image.path]: "" };
      try {
        const preview = await invoke<{ dataUrl: string }>("read_local_library_artwork_preview", {
          request: { root: localAssets.root, path: image.path },
        });
        localArtworkPreviews = { ...localArtworkPreviews, [image.path]: preview.dataUrl };
      } catch (cause) {
        localArtworkPreviewErrors = { ...localArtworkPreviewErrors, [image.path]: String(cause) };
      } finally {
        localArtworkPreviewBusy = { ...localArtworkPreviewBusy, [image.path]: false };
      }
    }
  }

  $effect(() => {
    const root = localAssets?.root;
    if (!desktop || !root) return;
    const selectedPaths = [
      ...Object.values(movieAssignments).map((assignment) => assignment.localPosterPath),
      ...Object.values(seriesAssignments).flatMap((assignment) => [
        assignment.localPosterPath,
        ...Object.values(assignment.localSeasonPosterPaths),
      ]),
    ].filter((path): path is string => Boolean(path));
    const uniquePaths = [...new Set(selectedPaths)];
    if (uniquePaths.length) {
      void loadLocalArtworkPreviews(uniquePaths.map((path) => ({ path, name: basename(path), role: "Selected poster" })));
    }
  });

  $effect(() => {
    const files = visibleOrganizerFiles;
    if (!desktop || mediaKind !== "movie" || organizerSource !== "analysis" || !localAssets || !files.length) return;
    const images = files
      .map((file) => {
        const artwork = localFileAssets(file.path)?.artwork ?? [];
        const path = preferredLocalPoster(artwork, null);
        return path ? artwork.find((image) => image.path === path) : undefined;
      })
      .filter((image): image is LocalArtworkFile => Boolean(image));
    for (const image of images.slice(0, 24)) void loadLocalArtworkPreviews([image]);
  });

  function useAnalyzedLibrary(kind: MediaKind, seriesRoot?: string) {
    if (!scanResult) return;
    organizerSource = "analysis";
    const groups = kind === "tv" ? buildAnalysisSeriesGroups(scanResult) : [];
    const targetSeriesRoot = seriesRoot ?? (groups.length === 1 ? groups[0].root : "");
    selectedAnalysisSeriesRoot = targetSeriesRoot;
    organizerScan = analysisOrganizerScan(scanResult, kind, targetSeriesRoot);
    if (kind === "movie" && organizerScan) void seedMovieDrafts(organizerScan.files);
    const filenameQuery = kind === "tv"
      ? targetSeriesRoot ? basename(targetSeriesRoot) : ""
      : organizerScan?.files.length === 1 ? organizerScan.files[0].name : "";
    query = filenameQuery;
    if (filenameQuery) {
      void filenameClue(filenameQuery).then((clue) => {
        if (query === filenameQuery) query = clue.title;
      }).catch(() => {});
    }
    episodeAssignments = {};
    if (kind === "tv" && organizerScan) {
      const savedReview = seriesAssignments[targetSeriesRoot];
      if (savedReview) restoreSavedSeriesReview(savedReview);
      if (detail) syncEpisodeAssignments(organizerScan.files);
    }
  }

  function restoreSavedSeriesReview(review: SeriesAssignment) {
    const [providerKey, ...recordParts] = review.recordId.split(":");
    const validProviders: Provider[] = ["wikidata", "tvmaze", "tvdb", "tmdb", "local"];
    provider = validProviders.includes(providerKey as Provider) ? providerKey as Provider : "local";
    const recordId = recordParts.join(":");
    const episodes: Episode[] = review.episodes.map((episode) => ({ ...episode, artwork: [] }));
    const seasonNumbers = [...new Set(episodes.map((episode) => episode.season))].sort((left, right) => left - right);
    selected = {
      id: recordId,
      provider,
      kind: "tv",
      title: review.title,
      year: review.year || null,
      description: review.plot || review.shortDescription || null,
      rating: review.ratingText ? Number(review.ratingText) : null,
      sourceUrl: review.sourceUrl,
      confidence: 1,
      artwork: review.artworkCandidates,
    };
    detail = {
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
      attribution: provider === "local" ? "Saved local NFO review" : `Saved review from ${provider.toUpperCase()}`,
    };
    selectedArtwork = { series: review.posterId };
    episodeAssignments = Object.fromEntries(Object.entries(review.episodeAssignments).map(([path, episode]) => [
      path,
      `${episode.season}/${episode.episode}`,
    ]));
  }

  function makeCurrentSeriesReview(): SeriesAssignment | null {
    if (!detail || !selected || mediaKind !== "tv") return null;
    const root = searchTarget?.kind === "tv" ? searchTarget.root : organizerScan?.root || selectedAnalysisSeriesRoot;
    if (!root) return null;
    const previous = seriesAssignments[root];
    const episodeFromDetail = (episode: Episode): QueueTvEpisode => ({
      season: episode.season,
      episode: episode.episode,
      title: episode.title,
      airdate: episode.airdate,
      summary: episode.summary,
      sourceUrl: episode.sourceUrl,
    });
    const assignedEpisodes: Record<string, QueueTvEpisode> = { ...(previous?.episodeAssignments ?? {}) };
    for (const [path, key] of Object.entries(episodeAssignments)) {
      if (!key) {
        delete assignedEpisodes[path];
        continue;
      }
      const episode = episodeForKey(key);
      if (episode && !assignedEpisodes[path]) assignedEpisodes[path] = episodeFromDetail(episode);
    }
    const local = provider === "local" ? localSeriesNfo(root) : undefined;
    const catalog = detail.episodes.length ? detail.episodes.map(episodeFromDetail) : previous?.episodes ?? [];
    for (const [path, episode] of Object.entries(assignedEpisodes)) {
      if (episode.airdate?.trim()) continue;
      const match = catalog.find((item) => item.season === episode.season && item.episode === episode.episode);
      if (match?.airdate?.trim()) assignedEpisodes[path] = { ...episode, airdate: match.airdate };
    }
    const nextRecord = `${provider}:${selected.id}`;
    const recordId = nextRecord.startsWith("local") && previous?.recordId && !previous.recordId.startsWith("local")
      ? previous.recordId
      : nextRecord;
    return {
      title: detail.title,
      year: detail.year ?? "",
      certification: detail.certification || previous?.certification || "",
      recordId,
      shortDescription: detail.shortDescription,
      plot: detail.longDescription,
      genresText: detail.genres.join(", "),
      ratingText: detail.rating === null ? "" : String(detail.rating),
      sourceUrl: detail.sourceUrl || previous?.sourceUrl || "",
      comment: previous?.comment ?? "",
      artworkCandidates: detail.artwork.length ? detail.artwork.map((image) => ({ ...image })) : previous?.artworkCandidates ?? [],
      posterId: selectedArtwork.series || previous?.posterId || "",
      localPosterPath: previous?.localPosterPath ?? "",
      localSeasonPosterPaths: previous?.localSeasonPosterPaths ?? {},
      seasonRecords: previous?.seasonRecords ?? {},
      nfoPaths: local?.paths ?? previous?.nfoPaths ?? [],
      nfoError: local?.error ?? previous?.nfoError ?? "",
      episodes: catalog.slice(0, 10_000),
      episodeAssignments: assignedEpisodes,
    };
  }

  function currentReviewedSeriesRoot(): string {
    return searchTarget?.kind === "tv" ? searchTarget.root : organizerScan?.root || selectedAnalysisSeriesRoot;
  }

  function updateCurrentSeriesArtwork(update: (review: SeriesAssignment) => SeriesAssignment) {
    const root = currentReviewedSeriesRoot();
    if (!root) return;
    const review = seriesAssignments[root] ?? makeCurrentSeriesReview();
    if (!review) return;
    seriesAssignments = { ...seriesAssignments, [root]: update(review) };
  }

  function chooseSeriesLocalPoster(path: string) {
    const root = currentReviewedSeriesRoot();
    const selectedPath = seriesAssignments[root]?.localPosterPath === path ? "" : path;
    selectedArtwork = { ...selectedArtwork, series: "" };
    updateCurrentSeriesArtwork((review) => ({ ...review, posterId: "", localPosterPath: selectedPath }));
    invalidateMetadataPreview();
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
    selectedArtwork = { ...selectedArtwork, series: id };
    updateCurrentSeriesArtwork((review) => ({ ...review, posterId: id, localPosterPath: "" }));
    pendingArtworkSave = null;
    artworkMessage = "";
    invalidateMetadataPreview();
  }

  function chooseSeriesProviderSeasonPoster(season: number, id: string) {
    selectedArtwork = { ...selectedArtwork, [`season:${season}`]: id };
    updateCurrentSeriesArtwork((review) => {
      const localSeasonPosterPaths = { ...review.localSeasonPosterPaths };
      delete localSeasonPosterPaths[String(season)];
      return { ...review, localSeasonPosterPaths };
    });
    pendingArtworkSave = null;
    artworkMessage = "";
  }

  function chooseMovieLocalPoster(mediaPath: string, artworkPath: string) {
    const review = movieAssignments[mediaPath];
    if (!review) return;
    const selectedPath = review.localPosterPath === artworkPath ? "" : artworkPath;
    movieAssignments = {
      ...movieAssignments,
      [mediaPath]: { ...review, posterId: "", localPosterPath: selectedPath },
    };
    pendingBatchLocalPoster = null;
    invalidateMetadataPreview();
  }

  function batchLocalPosterTargets(): string[] {
    if (!organizerScan) return [];
    const files = new Set(organizerScan.files.map((file) => file.path));
    return [...new Set(selectedPathsInOrganizer())]
      .filter((path) => files.has(path) && Boolean(movieAssignments[path]));
  }

  function previewBatchLocalPoster(mediaPath: string, artworkPath: string) {
    const mediaPaths = batchLocalPosterTargets();
    if (mediaPaths.length < 2) {
      chooseMovieLocalPoster(mediaPath, artworkPath);
      return;
    }
    pendingBatchLocalPoster = { sourcePath: mediaPath, artworkPath, mediaPaths };
  }

  function applyBatchLocalPoster() {
    const pending = pendingBatchLocalPoster;
    if (!pending || !organizerScan) return;
    const files = new Set(organizerScan.files.map((file) => file.path));
    const mediaPaths = pending.mediaPaths.filter((path) => files.has(path) && Boolean(movieAssignments[path]));
    if (!mediaPaths.length || !files.has(pending.sourcePath)) {
      pendingBatchLocalPoster = null;
      return;
    }
    movieAssignments = {
      ...movieAssignments,
      ...Object.fromEntries(mediaPaths.map((path) => [path, {
        ...movieAssignments[path],
        posterId: "",
        localPosterPath: pending.artworkPath,
      }])),
    };
    pendingBatchLocalPoster = null;
    invalidateMetadataPreview();
  }

  function suggestionForPath(path: string): QueueMetadataSuggestion | undefined {
    return queueSuggestions[path] ?? Object.entries(queueSuggestions).find(([key]) => normalizedPath(key) === normalizedPath(path))?.[1];
  }

  function suggestionResult(suggestion: QueueMetadataSuggestion | undefined, expectedKind?: MediaKind): Result | null {
    if (!suggestion || suggestion.status !== "ready" || (expectedKind && suggestion.kind !== expectedKind)) return null;
    const record = suggestion.recordId ?? "";
    const split = record.indexOf(":");
    const providerValue = split > 0 ? record.slice(0, split) : suggestion.provider ?? "";
    const id = split > 0 ? record.slice(split + 1) : "";
    const allowed = suggestion.kind === "movie" ? ["tmdb", "wikidata"] : ["tmdb", "tvdb", "tvmaze"];
    if (!id || !allowed.includes(providerValue)) return null;
    const provider = providerValue as Provider;
    return {
      id,
      provider,
      kind: suggestion.kind === "movie" ? "movie" : "tv",
      title: suggestion.title || suggestion.artwork?.title || "Untitled",
      year: suggestion.year,
      description: suggestion.description,
      rating: null,
      sourceUrl: suggestion.sourceUrl || suggestion.artwork?.sourceUrl || "",
      confidence: suggestion.autoAssociated ? 1 : 0,
      artwork: suggestion.artwork ? [suggestion.artwork] : [],
    };
  }

  function suggestionMatchSet(suggestion: QueueMetadataSuggestion | undefined, fallbackTitle: string, expectedKind: MediaKind): MatchSuggestion | null {
    if (!suggestion || suggestion.kind !== expectedKind) return null;
    const result = suggestionResult(suggestion, expectedKind);
    return {
      clue: {
        title: suggestion.title?.trim() || suggestion.artwork?.title?.trim() || fallbackTitle.replace(/\.[^.]+$/, ""),
        year: suggestion.year,
        yearEnd: null,
        certification: null,
      },
      results: result ? [result] : [],
      warnings: [],
      status: suggestion.status,
      error: suggestion.error,
    };
  }

  function savedMovieMatchSet(file: OrganizerFile): MatchSuggestion | null {
    return suggestionMatchSet(suggestionForPath(file.path), file.name, "movie");
  }

  function savedSeriesMatchSet(group: AnalysisSeriesGroup): MatchSuggestion | null {
    const suggestion = group.files.map((file) => suggestionForPath(file.path)).find((item) => item?.kind === "tv");
    return suggestionMatchSet(suggestion, basename(group.root), "tv");
  }

  function movieNeedsSearch(file: OrganizerFile): boolean {
    const suggestion = suggestionForPath(file.path);
    return !localNfoForFile(file.path)?.found
      && !movieAssignments[file.path]?.recordId
      && suggestion?.status !== "ready";
  }

  function movieSearchCount(files: OrganizerFile[]): number {
    return files.filter(movieNeedsSearch).length;
  }

  function movieLookupLabel(file: OrganizerFile, localFound: boolean, matchSet: MatchSuggestion | null): string {
    if (matchSet?.status === "searching") return "Searching…";
    if (localFound) return "Compare sources";
    if (suggestionForPath(file.path)?.status === "ready") return "Review suggestion";
    return "Find matches";
  }

  function reviewPosterOptions(path: string, assignment: MovieAssignment | undefined, results: Result[] = []): Artwork[] {
    const choices = [...(assignment?.artworkCandidates ?? [])];
    const add = (image: Artwork | null | undefined, first = false) => {
      if (!image || (!image.imageUrl && !image.thumbnailUrl)) return;
      if (choices.some((existing) => existing.id === image.id)) return;
      if (first) choices.unshift(image);
      else choices.push(image);
    };
    add(suggestionForPath(path)?.artwork, true);
    for (const row of results) add(candidatePreview(row));
    return choices;
  }

  function dedupeArtwork(images: Artwork[]): Artwork[] {
    const seen = new Set<string>();
    return images.filter((image) => {
      const key = image.imageUrl || image.thumbnailUrl || image.id;
      if (!key || seen.has(key)) return false;
      seen.add(key);
      return Boolean(image.imageUrl || image.thumbnailUrl);
    });
  }

  async function applyTmdbMovieDetails(mediaPath: string, id: string) {
    reviewDetailError = "";
    const value = await invoke<Record<string, unknown>>("load_tmdb_movie_details", { id: Number(id), language: metadataLanguage });
    const movie = (value.movie ?? {}) as Record<string, unknown>;
    const genres = Array.isArray(value.genres) ? value.genres.filter((item): item is string => typeof item === "string") : [];
    const rating = typeof movie.rating === "number" ? movie.rating : null;
    const title = typeof movie.title === "string" ? movie.title : "";
    const artwork = dedupeArtwork(reviewableArtwork("tmdb", title, Array.isArray(value.artwork) && value.artwork.length ? value.artwork : movie.artwork));
    const existing = movieAssignments[mediaPath];
    if (!existing) return;
    const tagline = typeof value.tagline === "string" ? value.tagline.trim() : "";
    const overview = typeof movie.overview === "string" ? movie.overview : "";
    movieAssignments = {
      ...movieAssignments,
      [mediaPath]: {
        ...existing,
        certification: existing.certification.trim() || (typeof value.certification === "string" ? value.certification : ""),
        shortDescription: tagline || existing.shortDescription,
        plot: overview || existing.plot,
        genresText: genres.join(", ") || existing.genresText,
        ratingText: rating === null ? existing.ratingText : rating.toFixed(1),
        sourceUrl: (typeof movie.sourceUrl === "string" && movie.sourceUrl) || existing.sourceUrl,
        artworkCandidates: artwork.length ? artwork : existing.artworkCandidates,
        posterId: artwork.some((image) => image.id === existing.posterId) ? existing.posterId : artwork[0]?.id || existing.posterId,
      },
    };
  }

  function selectMovieArtwork(mediaPath: string, posterId: string, artwork?: Artwork) {
    const review = movieAssignments[mediaPath];
    if (!review) return;
    const artworkCandidates = artwork && !review.artworkCandidates.some((image) => image.id === artwork.id)
      ? [artwork, ...review.artworkCandidates]
      : review.artworkCandidates;
    movieAssignments = {
      ...movieAssignments,
      [mediaPath]: { ...review, artworkCandidates, posterId, localPosterPath: "" },
    };
    invalidateMetadataPreview();
  }

  function chooseMovieProviderPoster(mediaPath: string) {
    const review = movieAssignments[mediaPath];
    if (!review) return;
    movieAssignments = {
      ...movieAssignments,
      [mediaPath]: { ...review, localPosterPath: "" },
    };
    invalidateMetadataPreview();
  }

  function chooseAnalyzedSeries(root: string) {
    results = [];
    selected = null;
    detail = null;
    seriesYearConflictConfirmed = false;
    error = "";
    renamePreview = null;
    confirmRename = false;
    invalidateMetadataPreview();
    metadataWriteMessage = "";
    pendingArtworkSave = null;
    artworkError = "";
    artworkMessage = "";
    useAnalyzedLibrary("tv", root);
  }

  const seriesArtworkLoads = new Set<string>();

  function suggestionDetail(suggestion: QueueMetadataSuggestion | undefined): Result | null {
    return suggestionResult(suggestion, "tv");
  }

  function seedSeriesAssignment(root: string, row: Result) {
    if (seriesAssignments[root]) return;
    const artwork = row.artwork ?? [];
    seriesAssignments = {
      ...seriesAssignments,
      [root]: {
        title: row.title,
        year: row.year ?? "",
        certification: "",
        recordId: `${row.provider}:${row.id}`,
        shortDescription: row.description ?? "",
        plot: row.description ?? "",
        genresText: "",
        ratingText: row.rating === null || row.rating === undefined ? "" : String(row.rating),
        sourceUrl: row.sourceUrl,
        artworkCandidates: artwork,
        posterId: artwork[0]?.id ?? "",
        localPosterPath: "",
        localSeasonPosterPaths: {},
        seasonRecords: {},
        nfoPaths: [],
        nfoError: "",
        episodes: [],
        episodeAssignments: {},
      },
    };
  }

  function resumeSeriesArtwork(scan: ScanResult) {
    if (!desktop || !metadataAutoLookup || lookupBusy) return;
    for (const group of buildAnalysisSeriesGroups(scan)) {
      if (group.ambiguous || !group.files.length) continue;
      const suggestion = group.files.map((file) => suggestionForPath(file.path)).find((item) => suggestionDetail(item));
      const row = suggestionDetail(suggestion);
      if (!row) continue;
      if (!seriesAssignments[group.root]) seedSeriesAssignment(group.root, row);
      const review = seriesAssignments[group.root];
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

  $effect(() => {
    const currentScan = scanResult;
    const currentKind = mediaKind;
    if (!currentScan) {
      lastAppliedAnalysis = null;
      lastAutoLookupKey = null;
      movieSuggestions = {};
      seriesSuggestions = {};
      expandedMatchLists = {};
      movieEditorOpen = {};
      return;
    }
    if (organizerSource === "folder") return;
    const shouldPrepareAnalysis = workbenchActive;
    const analysisChanged = lastAppliedAnalysis !== currentScan || lastAppliedKind !== currentKind;
    if (shouldPrepareAnalysis && analysisChanged) {
      if (lastAppliedAnalysis !== currentScan) {
        const lookupAlreadyRan = lastAutoLookupKey === metadataScanKey(currentScan);
        if (!lookupAlreadyRan) {
          movieSuggestions = {};
          seriesSuggestions = {};
        }
        expandedMatchLists = {};
        movieEditorOpen = {};
      }
      lastAppliedAnalysis = currentScan;
      lastAppliedKind = currentKind;
      useAnalyzedLibrary(currentKind);
      queueMicrotask(async () => {
        if (scanResult !== currentScan || organizerSource === "folder") return;
        const groups = buildAnalysisSeriesGroups(currentScan);
        const files = currentScan.entries.map(organizerFileFromEntry);
        const directories = [
          analyzedLibraryRoot(currentScan),
          ...groups.flatMap((group) => [group.root, ...group.seasonDirectories.map((season) => season.path)]),
        ];
        await ensureLocalAssets(analyzedLibraryRoot(currentScan), files, directories);
        if (currentKind === "movie" && scanResult === currentScan) {
          await seedMovieDrafts(analysisOrganizerScan(currentScan, "movie")?.files ?? []);
        }
      });
    }
    if (autoLookupPending && !metadataAutoLookup) {
      onAutoLookupConsumed();
    }
    if (!autoLookupPending) resumeSeriesArtwork(currentScan);
    const currentLookupKey = metadataScanKey(currentScan);
    const certificateMissing = Object.values(seriesAssignments).some((review) => {
      const record = review.recordId ?? "";
      return Boolean(record) && !record.startsWith("local") && !review.certification.trim();
    });
    if (metadataAutoLookup && desktop && autoLookupPending && currentLookupKey === lastAutoLookupKey && !certificateMissing) {
      onAutoLookupConsumed();
    } else if (metadataAutoLookup && desktop && autoLookupPending) {
      // Keep the pending request until the current provider batch releases the
      // shared lookup lock; switching queue sources must not consume it.
      if (lookupBusy) return;
      if (!onClaimAutoLookup()) return;
      lastAutoLookupKey = currentLookupKey;
      movieSuggestions = {};
      seriesSuggestions = {};
      onAutoLookupConsumed();
      queueMicrotask(async () => {
        if (!metadataAutoLookup || scanResult !== currentScan) return;
        await findMovieMatches(analysisOrganizerScan(currentScan, "movie")?.files ?? [], false, analyzedLibraryRoot(currentScan));
        if (metadataAutoLookup && scanResult === currentScan && !lastLookupStopped) {
          await findSeriesMatches(buildAnalysisSeriesGroups(currentScan), false, analyzedLibraryRoot(currentScan));
        }
      });
    }
  });

  function metadataScanKey(scan: ScanResult): string {
    let hash = 2166136261;
    const source = `${normalizedPath(scan.root)}\n${scan.entries.map((entry) => `${normalizedPath(entry.path)}\0${entry.size_bytes}\0${entry.modified_unix_ms ?? ""}`).join("\n")}`;
    for (let index = 0; index < source.length; index += 1) {
      hash ^= source.charCodeAt(index);
      hash = Math.imul(hash, 16777619);
    }
    return `${scan.entries.length}:${hash >>> 0}`;
  }

  const normalizeArt = (items: unknown): Artwork[] => Array.isArray(items) ? items as Artwork[] : [];
  const tmdbLogo = "https://www.themoviedb.org/assets/2/v4/logos/v2/blue_square_2-d537fb228cf3ded904ef09b136fe3fec72548ebc1fea3fbbd1ad9e36364db38b.svg";

  async function filenameClue(filename: string): Promise<FilenameClue> {
    if (!desktop) throw new Error("Filename-assisted matching is available in the installed desktop app.");
    return invoke<FilenameClue>("metadata_filename_clue", { filename, patterns: textBlacklistPatterns });
  }

  async function seedMovieDrafts(files: OrganizerFile[]) {
    if (!desktop || !files.length) return;
    try {
      const assets = organizerScan
        ? await ensureLocalAssets(organizerScan.root, files, [organizerScan.root, ...organizerScan.seasonDirectories.map((season) => season.path)])
        : null;
      if (!assets) {
        if (localAssetScanError) {
          lookupMessage = `Could not inspect local NFO and artwork files: ${localAssetScanError}`;
          lookupMessageScope = "analyzed films";
        }
        return;
      }
      const clues = await invoke<FilenameClue[]>("metadata_filename_clues", {
        filenames: files.map((file) => {
          const local = assets.files.find((entry) => normalizedPath(entry.path) === normalizedPath(file.path))?.metadata;
          return local?.title ?? file.name;
        }),
        patterns: textBlacklistPatterns,
      });
      const next = { ...movieAssignments };
      for (const [index, file] of files.entries()) {
        if (next[file.path] || !clues[index]) continue;
        const clue = clues[index];
        const local = assets.files.find((entry) => normalizedPath(entry.path) === normalizedPath(file.path))?.metadata;
        const rating = local?.rating;
        next[file.path] = {
          title: clue.title, year: local?.year ?? clue.year ?? "",
          certification: local?.certification ?? clue.certification ?? "", recordId: local?.found ? "local-nfo" : "",
          shortDescription: local?.shortDescription ?? "", plot: local?.plot ?? "",
          genresText: local?.genres.join(", ") ?? "", ratingText: rating === null || rating === undefined ? "" : String(rating),
          sourceUrl: safeHttpsUrl(local?.source) ?? "", artworkCandidates: [], posterId: "",
          localPosterPath: "",
          nfoPaths: local?.paths ?? [], nfoError: local?.error ?? "",
        };
      }
      movieAssignments = next;
    } catch (cause) {
      lookupMessage = `Could not prepare filename-based metadata drafts: ${String(cause)}`;
      lookupMessageScope = "analyzed films";
    }
  }

  async function searchPublicTitles(kind: MediaKind, title: string): Promise<PublicSearchResponse> {
    const requests: { source: string; provider: Provider; run: Promise<Result[]> }[] = [];
    const warnings: string[] = [];

    if (metadataSources.tmdb) {
      if (!tmdbCredentialStatus.configured) {
        warnings.push("TMDB was skipped because no API Read Access Token is saved. Add one in Settings to include TMDB results.");
      } else {
        requests.push({
          source: "TMDB",
          provider: "tmdb",
          run: invoke<Array<Record<string, unknown>>>(kind === "movie" ? "search_tmdb_movies" : "search_tmdb_tv", {
            query: title, language: metadataLanguage,
          }).then((rows) => rows.map((row) => ({
            id: String(row.id), provider: "tmdb", kind,
            title: String(row.title ?? "Untitled"),
            year: typeof row.year === "string" ? row.year : null,
            description: typeof row.overview === "string" ? row.overview : null,
            rating: typeof row.rating === "number" ? row.rating : null,
            sourceUrl: String(row.sourceUrl ?? "https://www.themoviedb.org"),
            confidence: Number(row.confidence ?? 0), artwork: normalizeArt(row.artwork),
          }))),
        });
      }
    }

    if (kind === "movie" && metadataSources.wikidata) {
      requests.push({
        source: "Wikidata",
        provider: "wikidata",
        run: invoke<Array<Record<string, unknown>>>("search_public_films", { query: title, language: metadataLanguage }).then((rows) => rows.map((row) => ({
          id: String(row.entityId), provider: "wikidata", kind,
          title: String(row.title ?? "Untitled"),
          year: typeof row.year === "string" ? row.year : null,
          description: typeof row.description === "string" ? row.description : null,
          rating: null, sourceUrl: String(row.sourceUrl ?? "https://www.wikidata.org"),
          confidence: Number(row.confidence ?? 0), artwork: [],
        }))),
      });
    }

    if (metadataSources.tvdb) {
      if (!tvdbCredentialStatus.configured) {
        warnings.push("TheTVDB was skipped because its project API key is not saved. Set up TheTVDB access in Settings to include its results.");
      } else {
        requests.push({
          source: "TheTVDB",
          provider: "tvdb",
          run: invoke<Array<Record<string, unknown>>>("search_tvdb_titles", {
            query: title, kind: kind === "movie" ? "movie" : "series", language: metadataLanguage,
          }).then((rows) => rows.map((row) => ({
            id: String(row.id), provider: "tvdb", kind,
            title: String(row.title ?? "Untitled"),
            year: typeof row.year === "string" ? row.year : null,
            description: typeof row.overview === "string" ? row.overview : null,
            rating: null, sourceUrl: String(row.sourceUrl ?? "https://www.thetvdb.com"),
            confidence: Number(row.confidence ?? 0), artwork: normalizeArt(row.artwork),
          }))),
        });
      }
    }

    if (kind === "tv" && metadataSources.tvmaze) {
      requests.push({
        source: "TVmaze",
        provider: "tvmaze",
        run: invoke<Array<Record<string, unknown>>>("search_public_tv_shows", { query: title }).then((rows) => rows.map((row) => ({
          id: String(row.id), provider: "tvmaze", kind,
          title: String(row.title ?? "Untitled"),
          year: [row.yearStart, row.yearEnd].filter((year) => typeof year === "string").join("–") || null,
          description: typeof row.summary === "string" ? row.summary : null,
          rating: typeof row.audienceRating === "number" ? row.audienceRating : null,
          sourceUrl: String(row.sourceUrl ?? "https://www.tvmaze.com"),
          confidence: Number(row.confidence ?? 0), artwork: normalizeArt(row.artwork),
        }))),
      });
    }

    if (!requests.length) {
      const enableHint = kind === "movie"
        ? "No movie source is ready. Turn on Wikidata, save a TMDB token, or set up TheTVDB in Settings."
        : "No series source is ready. Turn on TVmaze, save a TMDB token, or set up TheTVDB in Settings.";
      throw new Error([...warnings, enableHint].join(" "));
    }

    const settled = await Promise.allSettled(requests.map((request) => request.run));
    const results: Result[] = [];
    const successfulProviders: Provider[] = [];
    settled.forEach((outcome, index) => {
      if (outcome.status === "fulfilled") {
        successfulProviders.push(requests[index].provider);
        results.push(...outcome.value);
      } else {
        warnings.push(`${requests[index].source} lookup failed: ${String(outcome.reason)}`);
      }
    });
    if (!successfulProviders.length) {
      throw new Error(warnings.join(" ") || "All enabled metadata sources failed.");
    }

    const unique = new Map<string, Result>();
    for (const result of results) unique.set(`${result.provider}:${result.id}`, result);
    return { results: [...unique.values()].slice(0, 45), warnings, successfulProviders };
  }

  function providerLabel(value: Provider): string {
    return ({ tmdb: "TMDB", wikidata: "Wikidata", tvmaze: "TVmaze", tvdb: "TheTVDB", local: "Local NFO" })[value];
  }

  function candidateEvidence(result: Result, clue: FilenameClue): { text: string; conflict: boolean } {
    return metadataMatchLabel(result, { ...clue, kind: result.kind });
  }

  function currentSeriesYearMismatch(): { text: string; clue: FilenameClue } | null {
    if (mediaKind !== "tv" || !selected || !detail) return null;
    const root = currentReviewedSeriesRoot();
    const clue = seriesSuggestions[root]?.clue ?? manualSearchClue;
    if (!clue || !candidateEvidence(selected, clue).conflict) return null;
    return { text: candidateEvidence(selected, clue).text, clue };
  }

  const initialCandidateCount = 4;

  function visibleCandidates<T>(items: T[], key: string): T[] {
    return expandedMatchLists[key] ? items : items.slice(0, initialCandidateCount);
  }

  function toggleCandidateList(key: string) {
    expandedMatchLists = { ...expandedMatchLists, [key]: !expandedMatchLists[key] };
  }

  function candidateListExpanded(key: string): boolean {
    return Boolean(expandedMatchLists[key]);
  }

  function uniqueArtwork(artwork: Artwork[]): Artwork[] {
    const seen = new Set<string>();
    return artwork.filter((image) => {
      if (!image.id || image.id.length > 1024 || seen.has(image.id)) return false;
      seen.add(image.id);
      return true;
    });
  }

  function reviewableArtwork(provider: Provider, title: string, value: unknown): Artwork[] {
    const artwork = uniqueArtwork(normalizeArt(value));
    if (provider !== "wikidata") return artwork;

    // Commons search can return books, documents, and other weakly related images.
    // Show a Wikidata image only when its file title contains every meaningful title word.
    const ignored = new Set(["the", "and", "for", "from", "with", "film", "movie", "poster", "artwork"]);
    const titleWords = title.toLowerCase().match(/[a-z0-9]+/g)?.filter((word) => word.length > 2 && !ignored.has(word)) ?? [];
    if (!titleWords.length) return [];
    return artwork.filter((image) => {
      const imageTitle = image.title.toLowerCase().replace(/^file\s*:\s*/, "");
      const imageWords = new Set(imageTitle.match(/[a-z0-9]+/g) ?? []);
      return titleWords.every((word) => imageWords.has(word));
    });
  }

  function candidatePreview(result: Result): Artwork | undefined {
    return reviewableArtwork(result.provider, result.title, result.artwork)[0];
  }

  function rankByFilenameYear(results: Result[], clue: FilenameClue, kind: MediaKind = mediaKind): Result[] {
    return rankMetadataMatches(results, { ...clue, kind });
  }

  function autoAssignMovieMatch(path: string, clue: FilenameClue, candidate: Result): boolean {
    const previous = movieAssignments[path];
    if (previous?.recordId) return false;
    if (previous && normalizeMatchTitle(previous.title) !== normalizeMatchTitle(clue.title)) return false;
    const artwork = reviewableArtwork(candidate.provider, candidate.title, candidate.artwork);
    const year = candidate.year?.match(/(?:19|20)\d{2}/)?.[0] ?? clue.year ?? "";
    movieAssignments = {
      ...movieAssignments,
      [path]: {
        ...previous,
        title: candidate.title,
        year,
        certification: previous?.certification ?? "",
        recordId: `${candidate.provider}:${candidate.id}`,
        shortDescription: previous?.shortDescription || candidate.description || "",
        plot: previous?.plot || candidate.description || "",
        genresText: previous?.genresText ?? "",
        ratingText: previous?.ratingText || (candidate.rating === null ? "" : String(candidate.rating)),
        sourceUrl: candidate.sourceUrl,
        artworkCandidates: artwork,
        posterId: previous?.posterId || artwork[0]?.id || "",
        localPosterPath: previous?.localPosterPath ?? "",
        nfoPaths: previous?.nfoPaths ?? [],
        nfoError: previous?.nfoError ?? "",
      },
    };
    return true;
  }

  function autoAssignSeriesMatch(root: string, clue: FilenameClue, candidate: Result): boolean {
    const previous = seriesAssignments[root];
    if (previous?.recordId) return false;
    if (previous && normalizeMatchTitle(previous.title) !== normalizeMatchTitle(clue.title)) return false;
    const artwork = reviewableArtwork(candidate.provider, candidate.title, candidate.artwork);
    const year = candidate.year?.match(/(?:19|20)\d{2}/)?.[0] ?? clue.year ?? "";
    seriesAssignments = {
      ...seriesAssignments,
      [root]: {
        ...previous,
        title: candidate.title,
        year,
        certification: previous?.certification ?? "",
        recordId: `${candidate.provider}:${candidate.id}`,
        shortDescription: previous?.shortDescription || candidate.description || "",
        plot: previous?.plot || candidate.description || "",
        genresText: previous?.genresText ?? "",
        ratingText: previous?.ratingText || (candidate.rating === null ? "" : String(candidate.rating)),
        sourceUrl: candidate.sourceUrl,
        artworkCandidates: artwork,
        posterId: previous?.posterId || artwork[0]?.id || "",
        localPosterPath: previous?.localPosterPath ?? "",
        localSeasonPosterPaths: previous?.localSeasonPosterPaths ?? {},
        seasonRecords: previous?.seasonRecords ?? {},
        nfoPaths: previous?.nfoPaths ?? [],
        nfoError: previous?.nfoError ?? "",
        episodes: previous?.episodes ?? [],
        episodeAssignments: previous?.episodeAssignments ?? {},
      },
    };
    return true;
  }

  function clueCaption(clue: FilenameClue): string {
    const years = clue.year ? ` · ${clue.year}${clue.yearEnd ? `–${clue.yearEnd}` : ""}` : "";
    const certification = clue.certification ? ` · ${clue.certification}` : "";
    return `${clue.title}${years}${certification}`;
  }

  function prepareSearchTarget(target: SearchTarget, title: string) {
    searchTarget = target;
    mediaKind = target.kind;
    query = title;
    results = [];
    searchWarnings = [];
    selected = null;
    detail = null;
    manualSearchClue = null;
    seriesYearConflictConfirmed = false;
    expandedMatchLists = { ...expandedMatchLists, "manual-search": false };
    error = "";
    renamePreview = null;
    confirmRename = false;
    invalidateMetadataPreview();
    metadataWriteMessage = "";
    pendingArtworkSave = null;
    artworkError = "";
    artworkMessage = "";
    void filenameClue(title).then((clue) => {
      if (searchTarget === target && query === title) query = clue.title;
    }).catch(() => {});
  }

  function searchMovieRecord(file: OrganizerFile, preferredTitle?: string | null) {
    selectedMediaPaths = [...new Set([...selectedMediaPaths, file.path])];
    prepareSearchTarget({ kind: "movie", path: file.path, label: file.name }, preferredTitle?.trim() || file.name);
  }

  function searchSeriesRecord(group: AnalysisSeriesGroup, preferredTitle?: string | null) {
    if (group.ambiguous) return;
    if (organizerSource === "analysis" && selectedAnalysisSeriesRoot !== group.root) chooseAnalyzedSeries(group.root);
    prepareSearchTarget({ kind: "tv", root: group.root, label: basename(group.root) }, preferredTitle?.trim() || seriesFilenameTitle(group));
  }

  function searchCurrentSeriesRecord() {
    const current = organizerScan;
    if (!current || mediaKind !== "tv") return;
    searchSeriesRecord({
      root: current.root,
      files: current.files,
      seasonDirectories: current.seasonDirectories,
      ambiguous: false,
    }, localSeriesNfo(current.root)?.title);
  }

  function closeTitleSearch() {
    searchTarget = null;
    results = [];
    searchWarnings = [];
    error = "";
  }

  async function searchTitles() {
    const entered = query.trim();
    if (!desktop) {
      error = "Metadata search runs in the installed desktop app. The browser preview cannot access the native provider client.";
      return;
    }
    if (!searchTarget) {
      error = "Open title search from a movie or series record so the result stays connected to that item.";
      return;
    }
    if (!entered) {
      error = "Enter a movie or series title to search.";
      return;
    }
    searching = true;
    error = "";
    searchWarnings = [];
    selected = null;
    detail = null;
    renamePreview = null;
    confirmRename = false;
    invalidateMetadataPreview();
    metadataWriteMessage = "";
    pendingArtworkSave = null;
    artworkError = "";
    artworkMessage = "";
    try {
      const clue = await filenameClue(entered);
      manualSearchClue = clue;
      const response = await searchPublicTitles(mediaKind, clue.title);
      results = rankByFilenameYear(response.results, clue);
      searchWarnings = response.warnings;
      if (!results.length) error = "No likely matches came back. Try the original title, a shorter title, or a year.";
    } catch (cause) {
      error = String(cause);
      results = [];
    } finally {
      searching = false;
    }
  }

  async function findMovieMatches(files: OrganizerFile[], includeExistingNfo = false, analysisRoot?: string) {
    if (!desktop || lookupBusy || !files.length) return;
    const root = analysisRoot ?? (scanResult && organizerSource === "analysis" ? analyzedLibraryRoot(scanResult) : organizerScan?.root);
    if (!root) return;
    cancelLookupRequested = false;
    lastLookupStopped = false;
    lookupBusy = true;
    lookupProgress = { done: 0, total: files.length, label: "analyzed films" };
    onLookupStatus({ status: "searching", ...lookupProgress, label: "films", message: "Checking local NFO files and poster art before public searches.", sourceRoot: root });
    const assets = await ensureLocalAssets(root, files, [root]);
    if (!assets) {
      lookupBusy = false;
      lookupProgress = null;
      lookupMessage = `Could not check local NFO files: ${localAssetScanError || "local asset scan failed"}`;
      lookupMessageScope = "analyzed films";
      onLookupStatus({ status: "error", done: 0, total: files.length, label: "films", message: lookupMessage, sourceRoot: root });
      return;
    }
    const targets = files.filter((file) => {
      if (!file.path || !file.name) return false;
      if (includeExistingNfo) return true;
      const alreadyReviewed = Boolean(localNfoForFile(file.path)?.found);
      const reviewedAssignment = Boolean(movieAssignments[file.path]?.recordId);
      const savedSuggestion = suggestionForPath(file.path);
      const savedQueueResult = analysisRoot
        && (savedSuggestion?.status === "ready" || reviewedAssignment);
      return !alreadyReviewed && !savedQueueResult;
    });
    if (!targets.length) {
      lookupBusy = false;
      lookupProgress = null;
      const savedResultsCoverFiles = analysisRoot && files.every((file) =>
        suggestionForPath(file.path)?.status === "ready" || Boolean(movieAssignments[file.path]?.recordId),
      );
      lookupMessage = includeExistingNfo
        ? "No movie files were available for a public search."
        : savedResultsCoverFiles
          ? "Saved title suggestions or reviewed details are already available. Open a file to compare other sources."
          : "Every selected movie already has local NFO metadata. Open a file to search for a public alternative.";
      lookupMessageScope = "analyzed films";
      onLookupStatus({ status: "ready", done: files.length, total: files.length, label: "films", message: lookupMessage, sourceRoot: root });
      return;
    }
    lookupMessage = "";
    lookupMessageScope = null;
    lookupProgress = { done: 0, total: targets.length, label: "analyzed films" };
    onLookupStatus({ status: "searching", ...lookupProgress, label: "films", message: "Checking likely film titles and poster art.", sourceRoot: root });
    const titleCache = new Map<string, PublicSearchResponse>();
    let completed = 0;
    let failed = 0;
    for (const file of targets) {
      if (cancelLookupRequested) break;
      try {
        const local = localNfoForFile(file.path);
        const clue = await filenameClue(local?.title ?? file.name);
        if (local?.year) {
          const years = local.year.match(/(?:19|20)\d{2}/g) ?? [];
          clue.year = years[0] ?? clue.year;
          clue.yearEnd = years.at(-1) ?? clue.yearEnd;
        }
        movieSuggestions = {
          ...movieSuggestions,
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
        movieSuggestions = {
          ...movieSuggestions,
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
        const previous = movieSuggestions[file.path];
        movieSuggestions = {
          ...movieSuggestions,
          [file.path]: {
            clue: previous?.clue ?? { title: file.name, year: null, yearEnd: null, certification: null },
            results: [], warnings: [], status: "error", error: String(cause),
          },
        };
        onMatchSuggestion(file.path, { kind: "movie", status: "error", title: null, year: null, provider: null, recordId: null, sourceUrl: null, description: null, autoAssociated: false, artwork: null, error: String(cause) });
      }
      completed += 1;
      lookupProgress = { done: completed, total: targets.length, label: "analyzed films" };
      onLookupStatus({ status: "searching", ...lookupProgress, label: "films", message: `Checked ${completed} of ${targets.length} films.`, sourceRoot: root });
    }
    const stopped = cancelLookupRequested;
    lastLookupStopped = stopped;
    lookupBusy = false;
    lookupProgress = null;
    cancelLookupRequested = false;
    lookupMessage = stopped
      ? `Stopped after ${completed} of ${targets.length} film files.`
      : `Checked ${completed} film file${completed === 1 ? "" : "s"}${failed ? `; ${failed} lookup${failed === 1 ? "" : "s"} failed` : ""}. Exact title-and-year matches without a conflict were associated; review the rest.`;
    lookupMessageScope = "analyzed films";
    onLookupStatus({ status: failed === completed && failed > 0 ? "error" : "ready", done: completed, total: targets.length, label: "films", message: lookupMessage, sourceRoot: root });
  }

  function providerEpisodeTextReady(review: SeriesAssignment, group: AnalysisSeriesGroup): boolean {
    if (!review.episodes.length) return false;
    for (const file of group.files) {
      if (file.season === null || file.episode === null || file.episode <= 0 || file.multipleEpisodes) continue;
      const match = review.episodes.find((item) => item.season === file.season && item.episode === file.episode);
      if (!match) continue;
      const saved = review.episodeAssignments[file.path];
      if (!saved) return false;
      if ((match.summary ?? "").trim() && !(saved.summary ?? "").trim()) return false;
      if ((match.airdate ?? "").trim() && !(saved.airdate ?? "").trim()) return false;
      if (match.title.trim() && !saved.title.trim()) return false;
    }
    return group.seasonDirectories
      .filter((season) => season.season > 0)
      .every((season) => Boolean(review.seasonRecords[String(season.season)]));
  }

  function seriesGroupForHydrate(root: string): AnalysisSeriesGroup | undefined {
    if (!scanResult) return undefined;
    const wanted = normalizedPath(root);
    return buildAnalysisSeriesGroups(scanResult).find((group) => normalizedPath(group.root) === wanted);
  }

  function applyCatalogToFiles(root: string, existing: SeriesAssignment, catalog: QueueTvEpisode[]): SeriesAssignment["episodeAssignments"] {
    const assignments = { ...existing.episodeAssignments };
    const seen = new Set<string>();
    for (const file of seriesGroupForHydrate(root)?.files ?? []) {
      seen.add(file.path);
      if (file.season === null || file.episode === null || file.episode <= 0 || file.multipleEpisodes) continue;
      const match = catalog.find((item) => item.season === file.season && item.episode === file.episode);
      const current = assignments[file.path];
      if (!match && !current) continue;
      const identityTitle = episodeIdentity(file.name)?.title ?? "";
      const base = current ?? {
        season: file.season,
        episode: file.episode,
        title: identityTitle || file.name,
        airdate: null,
        summary: "",
        sourceUrl: "",
      };
      assignments[file.path] = fillEpisodeFromCatalog(base, match, existing.title, existing.plot, identityTitle);
    }
    for (const [path, episode] of Object.entries(assignments)) {
      if (seen.has(path)) continue;
      const match = catalog.find((item) => item.season === episode.season && item.episode === episode.episode);
      const name = path.split(/[\\/]/u).filter(Boolean).at(-1) ?? "";
      assignments[path] = fillEpisodeFromCatalog(episode, match, existing.title, existing.plot, episodeIdentity(name)?.title ?? "");
    }
    return assignments;
  }

  function applySeasonPublication(root: string, existing: SeriesAssignment, providerSeasons: Season[]): SeriesAssignment["seasonRecords"] {
    const records = { ...existing.seasonRecords };
    const numbers = new Set<number>();
    for (const season of providerSeasons) {
      if (typeof season.number === "number" && season.number >= 0) numbers.add(season.number);
    }
    for (const season of seriesGroupForHydrate(root)?.seasonDirectories ?? []) {
      if (season.season > 0) numbers.add(season.season);
    }
    for (const number of numbers) {
      const provider = providerSeasons.find((season) => season.number === number);
      const key = String(number);
      records[key] = fillSeasonFromProvider(records[key], {
        number,
        title: provider?.title ?? "",
        overview: provider?.overview ?? "",
        rating: typeof provider?.rating === "number" ? provider.rating : null,
      }, existing.genresText);
    }
    return records;
  }

  async function hydrateSeriesProvider(root: string, row: Result) {
    try {
      let episodes: Episode[] = [];
      let artwork: Artwork[] = row.artwork;
      let sourceUrl = row.sourceUrl;
      let providerSeasons: Season[] = [];
      let loadedCertification = "";
      if (row.provider === "tvdb") {
        const value = await invoke<Record<string, unknown>>("load_tvdb_details", {
          id: Number(row.id), kind: "series", language: metadataLanguage,
        });
        episodes = Array.isArray(value.episodes) ? value.episodes as Episode[] : [];
        providerSeasons = Array.isArray(value.seasons) ? value.seasons as typeof providerSeasons : [];
        artwork = reviewableArtwork("tvdb", row.title, value.artwork);
        sourceUrl = String(value.sourceUrl ?? row.sourceUrl);
        loadedCertification = typeof value.certification === "string" ? value.certification.trim() : "";
        if (!loadedCertification && tmdbCredentialStatus.configured) {
          const fallback = await invoke<string | null>("tmdb_tv_certification_for_tvdb", {
            tvdbId: Number(row.id), language: metadataLanguage,
          });
          loadedCertification = fallback?.trim() ?? "";
        }
      } else if (row.provider === "tmdb") {
        const value = await invoke<Record<string, unknown>>("load_tmdb_tv_details", { id: Number(row.id), language: metadataLanguage });
        const show = value.show as Record<string, unknown>;
        episodes = Array.isArray(value.episodes) ? value.episodes as Episode[] : [];
        providerSeasons = Array.isArray(value.seasons) ? value.seasons as typeof providerSeasons : [];
        artwork = reviewableArtwork("tmdb", row.title, show.artwork);
        sourceUrl = String(show.sourceUrl ?? row.sourceUrl);
        loadedCertification = typeof value.certification === "string" ? value.certification.trim() : "";
      } else if (row.provider === "tvmaze") {
        const value = await invoke<Record<string, unknown>>("load_public_tv_details", { showId: Number(row.id) });
        const show = value.show as Record<string, unknown>;
        episodes = Array.isArray(value.episodes) ? value.episodes as Episode[] : [];
        providerSeasons = Array.isArray(value.seasons) ? value.seasons as typeof providerSeasons : [];
        artwork = reviewableArtwork("tvmaze", row.title, show.artwork);
        sourceUrl = String(show.sourceUrl ?? row.sourceUrl);
      } else {
        return false;
      }
      const existing = seriesAssignments[root] ?? {
        title: row.title,
        year: row.year ?? "",
        certification: "",
        recordId: `${row.provider}:${row.id}`,
        shortDescription: row.description ?? "",
        plot: row.description ?? "",
        genresText: "",
        ratingText: "",
        sourceUrl: row.sourceUrl,
        artworkCandidates: row.artwork ?? [],
        posterId: row.artwork[0]?.id ?? "",
        localPosterPath: "",
        localSeasonPosterPaths: {},
        seasonRecords: {},
        nfoPaths: [],
        nfoError: "",
        episodes: [],
        episodeAssignments: {},
      };
      const catalog = episodes.filter((episode) => Number.isInteger(episode.season) && Number.isInteger(episode.episode) && episode.episode > 0);
      const savedEpisodes = catalog.slice(0, 10_000).map((episode) => ({
        season: episode.season,
        episode: episode.episode,
        title: episode.title,
        airdate: episode.airdate,
        summary: episode.summary,
        sourceUrl: episode.sourceUrl,
      }));
      const assignments = applyCatalogToFiles(root, existing, savedEpisodes);
      const seasonRecords = applySeasonPublication(root, existing, providerSeasons);
      const episodeStills = new Set(catalog.flatMap((episode) => (episode.artwork ?? []).map((image) => image.imageUrl).filter((url) => url.startsWith("https://"))));
      const seasonPosters = { ...existing.localSeasonPosterPaths };
      for (const season of providerSeasons) {
        if (typeof season.number !== "number" || season.number < 0) continue;
        const url = season.artwork?.find((image) => image.imageUrl?.startsWith("https://"))?.imageUrl;
        if (!url) continue;
        const key = String(season.number);
        const current = seasonPosters[key] ?? "";
        if (current && !current.startsWith("https://")) continue;
        const chosenRemote = current.startsWith("https://") && !episodeStills.has(current);
        if (!chosenRemote) seasonPosters[key] = url;
      }
      seriesAssignments = {
        ...seriesAssignments,
        [root]: {
          ...existing,
          episodes: savedEpisodes,
          episodeAssignments: assignments,
          seasonRecords,
          localSeasonPosterPaths: seasonPosters,
          artworkCandidates: artwork.length ? artwork : existing.artworkCandidates,
          posterId: existing.posterId || artwork[0]?.id || "",
          sourceUrl: existing.sourceUrl.trim() ? existing.sourceUrl : sourceUrl,
          certification: existing.certification.trim() || loadedCertification,
          recordId: !existing.recordId || existing.recordId.startsWith("local") ? `${row.provider}:${row.id}` : existing.recordId,
        },
      };
      return savedEpisodes.length > 0 || providerSeasons.some((season) => (season.overview ?? "").trim() || (typeof season.rating === "number" && season.rating > 0));
    } catch (cause) {
      lookupMessage = `Could not load episode details from ${row.provider}: ${String(cause)}`;
      lookupMessageScope = "series folders";
      return false;
    }
  }

  async function findSeriesMatches(groups: AnalysisSeriesGroup[], includeExistingNfo = false, analysisRoot?: string) {
    if (!desktop || lookupBusy || !groups.length) return;
    const assetRoot = analysisRoot ?? (scanResult && organizerSource === "analysis" ? analyzedLibraryRoot(scanResult) : organizerScan?.root ?? groups[0]?.root);
    const allFiles = groups.flatMap((group) => group.files);
    const allDirectories = groups.flatMap((group) => [group.root, ...group.seasonDirectories.map((season) => season.path)]);
    if (!assetRoot) return;
    cancelLookupRequested = false;
    lastLookupStopped = false;
    lookupBusy = true;
    lookupProgress = { done: 0, total: groups.length, label: "series folders" };
    onLookupStatus({ status: "searching", ...lookupProgress, label: "series", message: "Checking local show and episode NFO files and nearby poster art.", sourceRoot: assetRoot });
    if (!assetRoot || !await ensureLocalAssets(assetRoot, allFiles, allDirectories)) {
      lookupBusy = false;
      lookupProgress = null;
      lookupMessage = `Could not check local series NFO files: ${localAssetScanError || "local asset scan failed"}`;
      lookupMessageScope = "series folders";
      onLookupStatus({ status: "error", done: 0, total: groups.length, label: "series", message: lookupMessage, sourceRoot: assetRoot ?? undefined });
      return;
    }
    const targets = groups.filter((group) => {
      if (group.ambiguous || !group.files.length) return false;
      if (includeExistingNfo) return true;
      const alreadyReviewed = Boolean(localSeriesNfo(group.root)?.found);
      const recordId = seriesAssignments[group.root]?.recordId ?? "";
      const savedProvider = Boolean(recordId) && !recordId.startsWith("local");
      const hasProviderArt = (seriesAssignments[group.root]?.artworkCandidates ?? []).some((image) => image.imageUrl.startsWith("https://") || image.thumbnailUrl.startsWith("https://"));
      const seasonNumbers = group.seasonDirectories.map((item) => item.season).filter((number) => number > 0);
      const seasonPaths = seriesAssignments[group.root]?.localSeasonPosterPaths ?? {};
      const hasSeasonArt = seasonNumbers.length
        ? seasonNumbers.every((number) => (seasonPaths[String(number)] ?? "").startsWith("https://"))
        : Object.values(seasonPaths).some((path) => path.startsWith("https://"));
      const hasCertificate = Boolean((seriesAssignments[group.root]?.certification ?? "").trim());
      const savedQueueResult = analysisRoot && savedProvider && hasProviderArt && hasSeasonArt && hasCertificate;
      return !alreadyReviewed && !savedQueueResult;
    });
    if (!targets.length) {
      lookupBusy = false;
      lookupProgress = null;
      const savedResultsCoverGroups = analysisRoot && groups.every((group) =>
        Boolean(seriesAssignments[group.root]?.recordId)
        || group.files.every((file) => suggestionForPath(file.path)?.status === "ready"),
      );
      lookupMessage = includeExistingNfo
        ? "No show-specific series folders were recognized in this analysis."
        : savedResultsCoverGroups
          ? "Saved title suggestions or reviewed show details are already available. Open a series to compare other sources."
          : "Every recognized series has local NFO data. Review it or use the public-alternative action to compare provider records.";
      lookupMessageScope = "series folders";
      onLookupStatus({ status: "ready", done: groups.length, total: groups.length, label: "series", message: lookupMessage, sourceRoot: assetRoot });
      return;
    }
    lookupMessage = "";
    lookupMessageScope = null;
    lookupProgress = { done: 0, total: targets.length, label: "series folders" };
    onLookupStatus({ status: "searching", ...lookupProgress, label: "series", message: "Checking likely series titles and poster art.", sourceRoot: assetRoot });
    const titleCache = new Map<string, PublicSearchResponse>();
    let completed = 0;
    let failed = 0;
    for (const group of targets) {
      if (cancelLookupRequested) break;
      const root = group.root;
      try {
        const local = localSeriesNfo(root);
        const clue = await filenameClue(local?.title ?? seriesFilenameTitle(group));
        if (local?.year) {
          const years = local.year.match(/(?:19|20)\d{2}/g) ?? [];
          clue.year = years[0] ?? clue.year;
          clue.yearEnd = years.at(-1) ?? clue.yearEnd;
        }
        seriesSuggestions = {
          ...seriesSuggestions,
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
        const exactProvider = (provider: Provider) => rankedResults.find((row) => row.provider === provider && normalizeMatchTitle(row.title) === normalizeMatchTitle(clue.title));
        const detailSource = (metadataSources.tvdb && tvdbCredentialStatus.configured ? exactProvider("tvdb") : undefined) ?? autoMatch;
        const autoAssociated = autoMatch ? autoAssignSeriesMatch(root, clue, autoMatch) : false;
        if (detailSource) {
          void hydrateSeriesProvider(root, detailSource).then((loaded) => {
            if (!loaded && autoMatch && autoMatch !== detailSource) void hydrateSeriesProvider(root, autoMatch);
          });
        }
        seriesSuggestions = {
          ...seriesSuggestions,
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
        const previous = seriesSuggestions[root];
        seriesSuggestions = {
          ...seriesSuggestions,
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
      lookupProgress = { done: completed, total: targets.length, label: "series folders" };
      onLookupStatus({ status: "searching", ...lookupProgress, label: "series", message: `Checked ${completed} of ${targets.length} series.`, sourceRoot: assetRoot });
    }
    const stopped = cancelLookupRequested;
    lastLookupStopped = stopped;
    lookupBusy = false;
    lookupProgress = null;
    cancelLookupRequested = false;
    lookupMessage = stopped
      ? `Stopped after ${completed} of ${targets.length} series folders.`
      : `Checked ${completed} series folder${completed === 1 ? "" : "s"}${failed ? `; ${failed} lookup${failed === 1 ? "" : "s"} failed` : ""}. Exact title-and-year matches without a conflict were associated; review the rest.`;
    lookupMessageScope = "series folders";
    onLookupStatus({ status: failed === completed && failed > 0 ? "error" : "ready", done: completed, total: targets.length, label: "series", message: lookupMessage, sourceRoot: assetRoot });
  }

  function stopFilenameLookup() {
    cancelLookupRequested = true;
  }

  async function reviewMovieCandidate(path: string, row: Result, assignAfterReview = true) {
    selectedMediaPaths = [...new Set([...selectedMediaPaths, path])];
    await openMatch(row);
    if (detail && assignAfterReview) assignCurrentMovie([path]);
  }

  async function reviewSeriesCandidate(root: string, row: Result) {
    chooseAnalyzedSeries(root);
    await openMatch(row);
  }

  async function reviewTargetCandidate(row: Result) {
    const target = searchTarget;
    if (!target) return;
    if (target.kind === "movie") await reviewMovieCandidate(target.path, row);
    else await reviewSeriesCandidate(target.root, row);
  }

  async function useLocalSeriesMetadata(root: string) {
    const local = localSeriesNfo(root);
    if (!local?.found || local.error) return;
    if (organizerSource === "analysis" && selectedAnalysisSeriesRoot !== root) chooseAnalyzedSeries(root);
    const title = (await filenameClue(local.title ?? local.showTitle ?? basename(root))).title;
    const source = safeHttpsUrl(local.source) ?? "";
    const current = organizerScan;
    const seasons: Season[] = (current?.seasonDirectories ?? []).map((season) => ({
      number: season.season, title: season.name, yearStart: null, yearEnd: null, artwork: [],
    }));
    const episodeByKey = new Map<string, Episode>();
    for (const file of current?.files ?? []) {
      const metadata = localNfoForFile(file.path);
      if (metadata?.season === null || metadata?.season === undefined || metadata.episode === null || metadata.episode === undefined || !metadata.title) continue;
      const episode: Episode = {
        season: metadata.season, episode: metadata.episode, title: metadata.title,
        airdate: metadata.airdate, summary: metadata.plot, sourceUrl: safeHttpUrl(metadata.source) ?? "", artwork: [],
      };
      episodeByKey.set(`${episode.season}/${episode.episode}`, episode);
    }
    provider = "local";
    selected = {
      id: `local-nfo:${root}`, provider: "local", kind: "tv", title,
      year: local.year, description: local.plot, rating: local.rating, sourceUrl: source, confidence: 1, artwork: [],
    };
    detail = {
      title, originalTitle: null, year: local.year,
      sourceUrl: source, shortDescription: local.shortDescription ?? "", longDescription: local.plot ?? "",
      certification: local.certification, runtime: null, genres: local.genres, rating: local.rating,
      artwork: [], seasons, episodes: [...episodeByKey.values()].sort((left, right) => left.season - right.season || left.episode - right.episode),
      attribution: `Local NFO metadata${local.paths.length ? ` · ${local.paths.map(basename).join(", ")}` : ""}`,
    };
    episodeAssignments = {};
    if (current) syncEpisodeAssignments(current.files);
    selectedArtwork = {};
    renamePreview = null;
    invalidateMetadataPreview();
  }

  function safeHttpUrl(value: string | null | undefined): string | null {
    if (!value) return null;
    try {
      const parsed = new URL(value);
      return parsed.protocol === "http:" || parsed.protocol === "https:" ? parsed.href : null;
    } catch {
      return null;
    }
  }

  function safeHttpsUrl(value: string | null | undefined): string | null {
    const parsed = safeHttpUrl(value);
    return parsed?.startsWith("https://") ? parsed : null;
  }

  async function openMatch(row: Result) {
    provider = row.provider;
    mediaKind = row.kind;
    selected = row;
    detail = null;
    seriesYearConflictConfirmed = false;
    renamePreview = null;
    confirmRename = false;
    invalidateMetadataPreview();
    metadataWriteMessage = "";
    pendingArtworkSave = null;
    artworkError = "";
    artworkMessage = "";
    metadataWriteMessage = "";
    loadingDetail = true;
    error = "";
    try {
      if (row.provider === "tmdb" && row.kind === "movie") {
        const value = await invoke<Record<string, unknown>>("load_tmdb_movie_details", { id: Number(row.id), language: metadataLanguage });
        const movie = value.movie as Record<string, unknown>;
        detail = {
          title: String(movie.title ?? row.title), originalTitle: typeof movie.originalTitle === "string" ? movie.originalTitle : null,
          year: typeof movie.year === "string" ? movie.year : row.year,
          sourceUrl: String(movie.sourceUrl ?? row.sourceUrl), shortDescription: typeof value.tagline === "string" ? value.tagline : "",
          longDescription: typeof movie.overview === "string" ? movie.overview : "",
          certification: typeof value.certification === "string" ? value.certification : null,
          runtime: typeof value.runtimeMinutes === "number" ? value.runtimeMinutes : null,
          genres: Array.isArray(value.genres) ? value.genres as string[] : [],
          rating: typeof movie.rating === "number" ? movie.rating : row.rating,
          artwork: reviewableArtwork("tmdb", row.title, value.artwork), seasons: [], episodes: [], attribution: "This product uses the TMDB API but is not endorsed or certified by TMDB.",
        };
      } else if (row.provider === "tmdb") {
        const value = await invoke<Record<string, unknown>>("load_tmdb_tv_details", { id: Number(row.id), language: metadataLanguage });
        const show = value.show as Record<string, unknown>;
        detail = {
          title: String(show.title ?? row.title), originalTitle: typeof show.originalTitle === "string" ? show.originalTitle : null,
          year: typeof show.year === "string" ? show.year : row.year,
          sourceUrl: String(show.sourceUrl ?? row.sourceUrl), shortDescription: "",
          longDescription: typeof value.overview === "string" ? value.overview : "",
          certification: typeof value.certification === "string" ? value.certification : null, runtime: null, genres: Array.isArray(value.genres) ? value.genres as string[] : [],
          rating: typeof show.rating === "number" ? show.rating : row.rating,
          artwork: reviewableArtwork("tmdb", row.title, show.artwork),
          seasons: Array.isArray(value.seasons) ? value.seasons as Season[] : [],
          episodes: Array.isArray(value.episodes) ? value.episodes as Episode[] : [],
          attribution: "This product uses the TMDB API but is not endorsed or certified by TMDB.",
        };
      } else if (row.provider === "tvdb") {
        const value = await invoke<Record<string, unknown>>("load_tvdb_details", {
          id: Number(row.id), kind: row.kind === "movie" ? "movie" : "series", language: metadataLanguage,
        });
        detail = {
          title: String(value.title ?? row.title),
          originalTitle: typeof value.originalTitle === "string" ? value.originalTitle : null,
          year: typeof value.year === "string" ? value.year : row.year,
          sourceUrl: String(value.sourceUrl ?? row.sourceUrl),
          shortDescription: typeof value.shortDescription === "string" ? value.shortDescription : "",
          longDescription: typeof value.longDescription === "string" ? value.longDescription : (row.description ?? ""),
          certification: typeof value.certification === "string" ? value.certification : null,
          runtime: typeof value.runtimeMinutes === "number" ? value.runtimeMinutes : null,
          genres: Array.isArray(value.genres) ? value.genres as string[] : [],
          rating: typeof value.rating === "number" ? value.rating : null,
          artwork: reviewableArtwork("tvdb", row.title, value.artwork),
          seasons: Array.isArray(value.seasons) ? value.seasons as Season[] : [],
          episodes: Array.isArray(value.episodes) ? value.episodes as Episode[] : [],
          attribution: typeof value.attribution === "string" ? value.attribution : "TheTVDB metadata. Review the source access and reuse terms.",
          episodeOrderLabel: typeof value.episodeOrderLabel === "string" ? value.episodeOrderLabel : undefined,
        };
      } else if (row.kind === "movie" && row.provider === "wikidata") {
        const value = await invoke<Record<string, unknown>>("load_public_film_details", { entityId: row.id, language: metadataLanguage });
        detail = {
          title: String(value.title ?? row.title), originalTitle: null,
          year: typeof value.year === "string" ? value.year : row.year,
          sourceUrl: String(value.sourceUrl ?? row.sourceUrl),
          shortDescription: typeof value.shortDescription === "string" ? value.shortDescription : "",
          longDescription: typeof value.longDescription === "string" ? value.longDescription : "",
          certification: typeof value.certification === "string" ? value.certification : null,
          runtime: null, genres: [], rating: null, artwork: reviewableArtwork("wikidata", row.title, value.artwork), seasons: [], episodes: [],
          attribution: "Metadata from Wikidata; the long description may come from the selected language’s Wikipedia.",
        };
      } else {
        const value = await invoke<Record<string, unknown>>("load_public_tv_details", { showId: Number(row.id) });
        const show = value.show as Record<string, unknown>;
        detail = {
          title: String(show.title ?? row.title), originalTitle: null,
          year: [show.yearStart, show.yearEnd].filter((year) => typeof year === "string").join("–") || row.year,
          sourceUrl: String(show.sourceUrl ?? row.sourceUrl),
          shortDescription: "", longDescription: typeof show.summary === "string" ? show.summary : (row.description ?? ""),
          certification: null, runtime: null, genres: [],
          rating: typeof show.audienceRating === "number" ? show.audienceRating : row.rating,
          artwork: reviewableArtwork("tvmaze", row.title, show.artwork),
          seasons: Array.isArray(value.seasons) ? value.seasons as Season[] : [],
          episodes: Array.isArray(value.episodes) ? value.episodes as Episode[] : [],
          attribution: `TVmaze program data, as published by TVmaze. Requested metadata language: ${metadataLanguage}; TVmaze may return fields in their submitted language. Verify each image's source and license.`,
        };
      }
    } catch (cause) {
      error = String(cause);
    } finally {
      loadingDetail = false;
    }
    selectedArtwork = {};
    pendingArtworkSave = null;
    renamePreview = null;
    if (mediaKind === "tv" && organizerScan) syncEpisodeAssignments(organizerScan.files);
  }

  async function chooseOrganizerFolder() {
    if (!desktop) { renameError = "Folder selection is available in the installed desktop app."; return; }
    const path = await openDialog({ directory: true, multiple: false, title: mediaKind === "tv" ? "Choose one TV series folder" : "Choose a movie folder" });
    if (!path || Array.isArray(path)) return;
    renameBusy = true;
    renameError = "";
    renamePreview = null;
    invalidateMetadataPreview();
    metadataWriteMessage = "";
    pendingArtworkSave = null;
    artworkError = "";
    artworkMessage = "";
    try {
      organizerScan = await invoke<OrganizerScan>("scan_organizer_folder", { root: path });
      organizerSource = "folder";
      selectedAnalysisSeriesRoot = "";
      localAssets = null;
      localAssetGeneration += 1;
      localArtworkPreviews = {};
      localArtworkPreviewErrors = {};
      const assets = await ensureLocalAssets(organizerScan.root, organizerScan.files, [organizerScan.root, ...organizerScan.seasonDirectories.map((season) => season.path)]);
      if (mediaKind === "tv") {
        const episodePaths = organizerScan.files.filter((file) => file.season !== null && file.episode !== null && !file.multipleEpisodes).map((file) => file.path);
        selectedMediaPaths = [...new Set([...selectedMediaPaths, ...episodePaths])];
      } else if (assets) {
        await seedMovieDrafts(organizerScan.files);
      }
      syncEpisodeAssignments(organizerScan.files);
      if (metadataAutoLookup && assets) {
        if (mediaKind === "movie") void findMovieMatches(organizerScan.files);
        else void findSeriesMatches([{ root: organizerScan.root, files: organizerScan.files, seasonDirectories: organizerScan.seasonDirectories, ambiguous: false }]);
      }
    } catch (cause) {
      organizerScan = null;
      renameError = String(cause);
    } finally {
      renameBusy = false;
    }
  }

  async function scanCurrentSeriesFolder() {
    if (!desktop || !organizerScan || mediaKind !== "tv") return;
    const path = organizerScan.root;
    renameBusy = true;
    renameError = "";
    renamePreview = null;
    confirmRename = false;
    invalidateMetadataPreview();
    metadataWriteMessage = "";
    try {
      organizerScan = await invoke<OrganizerScan>("scan_organizer_folder", { root: path });
      organizerSource = "folder";
      localAssets = null;
      localAssetGeneration += 1;
      localArtworkPreviews = {};
      localArtworkPreviewErrors = {};
      const assets = await ensureLocalAssets(organizerScan.root, organizerScan.files, [organizerScan.root, ...organizerScan.seasonDirectories.map((season) => season.path)]);
      const episodePaths = organizerScan.files
        .filter((file) => file.season !== null && file.episode !== null && !file.multipleEpisodes)
        .map((file) => file.path);
      selectedMediaPaths = [...new Set([...selectedMediaPaths, ...episodePaths])];
      syncEpisodeAssignments(organizerScan.files);
      if (metadataAutoLookup && assets) {
        void findSeriesMatches([{ root: organizerScan.root, files: organizerScan.files, seasonDirectories: organizerScan.seasonDirectories, ambiguous: false }]);
      }
    } catch (cause) {
      renameError = String(cause);
    } finally {
      renameBusy = false;
    }
  }

  function changeMediaKind(kind: MediaKind) {
    mediaKind = kind;
    query = "";
    results = [];
    searchWarnings = [];
    searchTarget = null;
    selected = null;
    detail = null;
    error = "";
    organizerScan = null;
    organizerSource = null;
    tvRenameScope = "series-tree";
    episodeAssignments = {};
    renamePreview = null;
    renameError = "";
    confirmRename = false;
    pendingArtworkSave = null;
    artworkError = "";
    artworkMessage = "";
    invalidateMetadataPreview();
    metadataWriteMessage = "";
    if (scanResult) useAnalyzedLibrary(kind);
  }

  function syncEpisodeAssignments(files: OrganizerFile[]) {
    if (!detail) return;
    const known = new Set(detail.episodes.filter((episode) => episode.season !== null && episode.episode !== null)
      .map((episode) => `${episode.season}/${episode.episode}`));
    const next = { ...episodeAssignments };
    for (const file of files) {
      if (next[file.path] || file.multipleEpisodes || file.season === null || file.episode === null) continue;
      const key = `${file.season}/${file.episode}`;
      if (known.has(key)) next[file.path] = key;
    }
    episodeAssignments = next;
  }

  function selectAllMedia(select: boolean) {
    if (!organizerScan) return;
    const scopeFiles = compactItemMode ? visibleOrganizerFiles : organizerScan.files;
    const eligible = new Set(scopeFiles.filter((file) => !file.multipleEpisodes).map((file) => file.path));
    selectedMediaPaths = select
      ? [...new Set([...selectedMediaPaths, ...eligible])]
      : selectedMediaPaths.filter((path) => !eligible.has(path));
    renamePreview = null;
    confirmRename = false;
    invalidateMetadataPreview();
  }

  function selectedPathsInOrganizer(): string[] {
    if (!organizerScan) return [];
    const known = new Set(visibleOrganizerFiles.map((file) => file.path));
    return selectedMediaPaths.filter((path) => known.has(path));
  }

  function createMetadataOnlyCopies() {
    const paths = selectedPathsInOrganizer();
    if (paths.length) onStartMetadataOnly(paths);
  }

  function compactRenameReady(): boolean {
    const paths = selectedPathsInOrganizer();
    if (!compactItemMode || !paths.length) return false;
    if (mediaKind === "movie") return paths.every((path) => Boolean(movieAssignments[path]));
    return Boolean(detail && selected && paths.every((path) => {
      const episode = episodeForKey(episodeAssignments[path]);
      return episode && episode.season !== null && episode.episode !== null;
    }));
  }

  function assignCurrentMovie(paths: string[]) {
    if (!detail || mediaKind !== "movie" || paths.length === 0) return;
    const assignment = {
      title: detail.title,
      year: detail.year ?? "",
      certification: detail.certification ?? "",
      recordId: `${provider}:${selected?.id ?? ""}`,
      shortDescription: detail.shortDescription,
      plot: detail.longDescription,
      genresText: detail.genres.join(", "),
      ratingText: detail.rating === null ? "" : String(detail.rating),
      sourceUrl: detail.sourceUrl,
      artworkCandidates: detail.artwork.map((image) => ({ ...image })),
      posterId: selectedArtwork.movie ?? "",
      localPosterPath: "",
      nfoPaths: [], nfoError: "",
    };
    movieAssignments = {
      ...movieAssignments,
      ...Object.fromEntries(paths.map((path) => [path, { ...assignment }])),
    };
    renamePreview = null;
    renameError = "";
    invalidateMetadataPreview();
  }

  function toggleMedia(path: string, selectedNow: boolean) {
    selectedMediaPaths = selectedNow
      ? [...new Set([...selectedMediaPaths, path])]
      : selectedMediaPaths.filter((item) => item !== path);
    renamePreview = null;
    invalidateMetadataPreview();
  }

  function episodeForKey(key: string | undefined): Episode | undefined {
    if (!key || !detail) return undefined;
    const [season, episode] = key.split("/").map(Number);
    return detail.episodes.find((item) => item.season === season && item.episode === episode);
  }

  function invalidateMetadataPreview() {
    metadataWriteRequest = null;
    metadataWritePreview = null;
    metadataWriteError = "";
  }

  function updateEpisode(episode: Episode, field: "title" | "airdate" | "summary", value: string) {
    if (!detail) return;
    const sameEpisode = (item: Episode) => item.season === episode.season && item.episode === episode.episode;
    detail = {
      ...detail,
      episodes: detail.episodes.map((item) => sameEpisode(item) ? { ...item, [field]: value || null } : item),
    };
    renamePreview = null;
    invalidateMetadataPreview();
  }

  function nullableText(value: string | null | undefined): string | null {
    const normalized = value?.trim() ?? "";
    return normalized ? normalized : null;
  }

  function assignedMoviePoster(assignment: MovieAssignment): Artwork | undefined {
    return assignment.artworkCandidates.find((image) => image.id === assignment.posterId);
  }

  function metadataWriteInput(): MetadataWriteRequest {
    if (!organizerScan) throw new Error("Choose a media folder and select a reviewed title first.");
    const documents: MetadataDocument[] = [];
    const selectedForOrganizer = selectedPathsInOrganizer();
    if (mediaKind === "movie") {
      if (!selectedForOrganizer.length) throw new Error("Select one or more movie files to receive this metadata.");
      for (const path of selectedForOrganizer) {
        const assignment = movieAssignments[path];
        if (!assignment) {
          throw new Error(`Assign a reviewed film to ${organizerScan.files.find((file) => file.path === path)?.name ?? path} before writing metadata.`);
        }
        const ratingText = assignment.ratingText.trim();
        const rating = ratingText ? Number(ratingText) : null;
        if (rating !== null && (!Number.isFinite(rating) || rating < 0 || rating > 10)) {
          throw new Error(`Rating for ${organizerScan.files.find((file) => file.path === path)?.name ?? path} must be between 0 and 10.`);
        }
        documents.push({
          sourcePath: path, kind: "movie", title: assignment.title, year: nullableText(assignment.year),
          certification: nullableText(assignment.certification),
          shortDescription: nullableText(assignment.shortDescription), plot: nullableText(assignment.plot),
          genres: assignment.genresText.split(/[;,]/).map((genre) => genre.trim()).filter(Boolean),
          rating, sourceUrl: assignment.sourceUrl,
          showTitle: null, season: null, episode: null, airdate: null,
        });
      }
    } else {
      if (!detail || !selected) throw new Error("Search for and select a reviewed TV series before writing metadata.");
      const common = {
        shortDescription: nullableText(detail.shortDescription), plot: nullableText(detail.longDescription),
        genres: detail.genres, rating: detail.rating, sourceUrl: detail.sourceUrl,
      };
      documents.push({
        sourcePath: organizerScan.root, kind: "tv_show", title: detail.title, year: nullableText(detail.year),
        certification: nullableText(detail.certification), ...common,
        showTitle: null, season: null, episode: null, airdate: null,
      });
      for (const path of selectedForOrganizer) {
        const file = organizerScan.files.find((item) => item.path === path);
        const match = episodeForKey(episodeAssignments[path]);
        if (!file || file.multipleEpisodes || !match || match.season === null || match.episode === null) {
          throw new Error(`Choose a reviewed episode for ${file?.name ?? path} before writing TV metadata.`);
        }
        documents.push({
          sourcePath: path, kind: "tv_episode", title: match.title, year: null, certification: null,
          shortDescription: null, plot: nullableText(match.summary), genres: [], rating: null,
          sourceUrl: match.sourceUrl, showTitle: detail.title, season: match.season, episode: match.episode,
          airdate: nullableText(match.airdate),
        });
      }
    }
    return { root: organizerScan.root, documents };
  }

  async function previewMetadataWrite() {
    if (!desktop) { metadataWriteError = "Metadata writes are available in the installed desktop app."; return; }
    metadataWriteBusy = true;
    metadataWriteError = "";
    metadataWriteMessage = "";
    try {
      const request = metadataWriteInput();
      metadataWriteRequest = request;
      metadataWritePreview = await invoke<MetadataWritePreview>("preview_library_metadata_write", { request });
    } catch (cause) {
      metadataWriteRequest = null;
      metadataWritePreview = null;
      metadataWriteError = String(cause);
    } finally {
      metadataWriteBusy = false;
    }
  }

  async function applyMetadataWrite() {
    if (!desktop || !metadataWriteRequest || !metadataWritePreview?.canApply) return;
    metadataWriteBusy = true;
    metadataWriteError = "";
    metadataWriteMessage = "";
    try {
      const result = await invoke<MetadataWriteResult>("apply_library_metadata_write", {
        request: metadataWriteRequest, previewId: metadataWritePreview.previewId, confirmed: true,
      });
      metadataWriteMessage = `${result.message} Wrote ${result.filesWritten} file${result.filesWritten === 1 ? "" : "s"}${result.filesReplaced ? `; replaced ${result.filesReplaced} app-managed sidecar${result.filesReplaced === 1 ? "" : "s"}` : ""}.`;
      metadataWriteRequest = null;
      metadataWritePreview = null;
    } catch (cause) {
      metadataWriteError = String(cause);
    } finally {
      metadataWriteBusy = false;
    }
  }

  function showYears() {
    const candidates = [detail?.year, ...(detail?.seasons ?? []).flatMap((season) => [season.yearStart, season.yearEnd])]
      .filter((value): value is string => typeof value === "string")
      .flatMap((value) => value.match(/\b\d{4}\b/g) ?? []);
    const years = [...new Set(candidates)].sort();
    const first = years[0] ?? "";
    const last = years.at(-1) ?? "";
    return { first, last, range: first && last && first !== last ? `${first}-${last}` : first || last };
  }

  function saveOrganizerTemplates() {
    onOrganizerTemplatesChange({ ...organizerTemplates });
  }

  function stageArtworkSave(
    candidate: Artwork | undefined,
    targets: ArtworkTarget[],
    label: string,
    queueBindings: (ArtworkQueueBinding | null)[] = [],
  ) {
    if (!candidate || !organizerScan || !targets.length) return;
    pendingArtworkSave = { candidate, targets, label, queueBindings, saveBesideMedia: savePosterBesideMedia };
    artworkError = "";
    artworkMessage = "";
  }

  function artworkFileExtension(candidate: Artwork): string {
    try {
      const extension = new URL(candidate.imageUrl).pathname.split(".").at(-1)?.toLowerCase() ?? "";
      return ["jpg", "jpeg", "png", "webp"].includes(extension) ? extension : "jpg";
    } catch {
      return "jpg";
    }
  }

  function plannedArtworkPaths(): string[] {
    if (!pendingArtworkSave) return [];
    if (!pendingArtworkSave.saveBesideMedia) return ["Private application artwork cache", "Poster source and license details saved with the cached image"];
    const extension = artworkFileExtension(pendingArtworkSave.candidate);
    return pendingArtworkSave.targets.map((target) => `${target.directory.replace(/[\\/]$/, "")}/${target.basename}.${extension}`);
  }

  async function applyArtworkSave() {
    if (!desktop || !organizerScan || !pendingArtworkSave) return;
    const pending = pendingArtworkSave;
    artworkBusy = true;
    artworkError = "";
    artworkMessage = "";
    try {
      const result = pending.saveBesideMedia
        ? await invoke<ArtworkSaveResult>("save_library_artwork", {
          request: { root: organizerScan.root, candidate: pending.candidate, targets: pending.targets },
        })
        : await invoke<ArtworkSaveResult>("cache_library_artwork", { candidate: pending.candidate });
      let nextMovieAssignments = movieAssignments;
      let nextSeriesAssignments = seriesAssignments;
      let assignedPosterCount = 0;
      for (const [index, binding] of pending.queueBindings.entries()) {
        const savedPath = (pending.saveBesideMedia ? result.files[index] : result.files[0])?.path;
        if (!binding || !savedPath) continue;
        if (binding.kind === "movie") {
          const review = nextMovieAssignments[binding.mediaPath];
          if (review?.posterId === binding.candidateId) {
            nextMovieAssignments = {
              ...nextMovieAssignments,
              [binding.mediaPath]: { ...review, localPosterPath: savedPath },
            };
            assignedPosterCount += 1;
          }
        } else {
          const review = nextSeriesAssignments[binding.seriesRoot];
          if (binding.kind === "series") {
            if (review?.posterId !== binding.candidateId) continue;
            nextSeriesAssignments = {
              ...nextSeriesAssignments,
              [binding.seriesRoot]: { ...review, localPosterPath: savedPath },
            };
            assignedPosterCount += 1;
          } else {
            if (selectedArtwork[`season:${binding.seasonNumber}`] !== binding.candidateId || !review) continue;
            nextSeriesAssignments = {
              ...nextSeriesAssignments,
              [binding.seriesRoot]: {
                ...review,
                localSeasonPosterPaths: { ...review.localSeasonPosterPaths, [String(binding.seasonNumber)]: savedPath },
              },
            };
            assignedPosterCount += 1;
          }
        }
      }
      movieAssignments = nextMovieAssignments;
      seriesAssignments = nextSeriesAssignments;
      invalidateMetadataPreview();
      artworkMessage = `${result.message} Saved ${result.files.length} image${result.files.length === 1 ? "" : "s"}.${assignedPosterCount ? ` ${assignedPosterCount} reviewed poster selection${assignedPosterCount === 1 ? " is" : "s are"} ready for the next MKV output.` : " The saved image was not attached to this review because its selected match changed while the download was in progress. Choose it as a local poster before creating an MKV."}`;
      pendingArtworkSave = null;
    } catch (cause) {
      artworkError = String(cause);
    } finally {
      artworkBusy = false;
    }
  }

  function movieArtworkTarget(file: OrganizerFile): ArtworkTarget {
    const extension = file.name.lastIndexOf(".");
    const stem = extension > 0 ? file.name.slice(0, extension) : file.name;
    return { directory: file.parentPath, basename: `${stem}-poster` };
  }

  async function chooseLocalPoster(file: OrganizerFile) {
    if (!desktop || !organizerScan) return;
    const path = await openDialog({
      directory: false,
      multiple: false,
      title: `Choose poster for ${file.name}`,
      filters: [{ name: "Poster image", extensions: ["jpg", "jpeg", "png", "webp"] }],
    });
    if (!path || Array.isArray(path)) return;
    const extension = path.replaceAll("\\", "/").split("/").at(-1)?.split(".").at(-1)?.toLowerCase() ?? "";
    if (!["jpg", "jpeg", "png", "webp"].includes(extension)) {
      localPosterError = "Choose a JPEG, PNG, or WebP image.";
      return;
    }
    const target = movieArtworkTarget(file);
    pendingLocalPoster = {
      sourcePath: path,
      target,
      targetPath: savePosterBesideMedia ? joinPath(target.directory, `${target.basename}.${extension === "jpeg" ? "jpg" : extension}`) : "Private application artwork cache",
      mediaPath: file.path,
      saveBesideMedia: savePosterBesideMedia,
    };
    localPosterError = "";
    localPosterMessage = "";
  }

  async function saveLocalPoster() {
    if (!desktop || !organizerScan || !pendingLocalPoster) return;
    localPosterBusy = true;
    localPosterError = "";
    try {
      const result = pendingLocalPoster.saveBesideMedia
        ? await invoke<{ files: string[]; message: string }>("save_local_library_artwork", {
          request: { root: organizerScan.root, sourcePath: pendingLocalPoster.sourcePath, targets: [pendingLocalPoster.target] },
        })
        : await invoke<{ files: string[]; message: string }>("cache_local_library_artwork", { sourcePath: pendingLocalPoster.sourcePath });
      if (pendingLocalPoster.mediaPath && result.files[0]) {
        chooseMovieLocalPoster(pendingLocalPoster.mediaPath, result.files[0]);
        await loadLocalArtworkPreviews([{ path: result.files[0], name: basename(result.files[0]), role: "Selected poster" }]);
      }
      localPosterMessage = `${result.message} Saved to ${result.files.join(", ")}`;
      pendingLocalPoster = null;
    } catch (cause) {
      localPosterError = String(cause);
    } finally {
      localPosterBusy = false;
    }
  }

  function seriesArtworkTargets(seriesRoot: string): ArtworkTarget[] {
    return [
      { directory: seriesRoot, basename: "folder" },
      { directory: seriesRoot, basename: "poster" },
    ];
  }

  function seasonArtworkTargets(seriesRoot: string, seasonDirectory: string, seasonNumber: number): ArtworkTarget[] {
    const kodiName = seasonNumber === 0 ? "season-specials-poster" : `season${seasonNumber}-poster`;
    return [
      { directory: seasonDirectory, basename: "poster" },
      { directory: seasonDirectory, basename: seasonNumber === 0 ? "season-specials-poster" : `Season${String(seasonNumber).padStart(2, "0")}` },
      { directory: seriesRoot, basename: kodiName },
    ];
  }

  function movieArtworkTargets(): ArtworkTarget[] {
    const image = selectedMovieArtwork();
    const selectedId = selected ? `${provider}:${selected.id}` : undefined;
    const currentScan = organizerScan;
    if (!image || !currentScan || !selectedId) return [];
    return selectedPathsInOrganizer().flatMap((path) => {
      const assignment = movieAssignments[path];
      const file = currentScan.files.find((item) => item.path === path);
      if (!assignment || !file || assignment.recordId !== selectedId
        || !assignment.artworkCandidates.some((candidate) => candidate.id === image.id)) return [];
      return [movieArtworkTarget(file)];
    });
  }

  function movieArtworkBindings(image: Artwork): Extract<ArtworkQueueBinding, { kind: "movie" }>[] {
    const selectedId = selected ? `${provider}:${selected.id}` : undefined;
    const currentScan = organizerScan;
    if (!currentScan || !selectedId) return [];
    return selectedPathsInOrganizer().flatMap((path) => {
      const assignment = movieAssignments[path];
      const file = currentScan.files.find((item) => item.path === path);
      if (!assignment || !file || assignment.recordId !== selectedId
        || !assignment.artworkCandidates.some((candidate) => candidate.id === image.id)) return [];
      return [{ kind: "movie" as const, mediaPath: path, candidateId: image.id }];
    });
  }

  function assignSelectedPosterToMatchingCopies() {
    const image = selectedMovieArtwork();
    const selectedId = selected ? `${provider}:${selected.id}` : undefined;
    if (!image || !selectedId) return;
    const next = { ...movieAssignments };
    for (const path of selectedPathsInOrganizer()) {
      const assignment = next[path];
      if (assignment?.recordId === selectedId
        && assignment.artworkCandidates.some((candidate) => candidate.id === image.id)) {
        next[path] = { ...assignment, posterId: image.id, localPosterPath: "" };
      }
    }
    movieAssignments = next;
    invalidateMetadataPreview();
  }

  function stageCurrentFilmPoster() {
    const image = selectedMovieArtwork();
    if (!image) return;
    assignSelectedPosterToMatchingCopies();
    const currentScan = organizerScan;
    const bindings = movieArtworkBindings(image);
    if (!currentScan) return;
    const destinations = bindings.flatMap((binding) => {
      const file = currentScan.files.find((item) => item.path === binding.mediaPath);
      return file ? [{ target: movieArtworkTarget(file), binding }] : [];
    });
    stageArtworkSave(image, destinations.map(({ target }) => target), "poster for selected copies assigned to this film", destinations.map(({ binding }) => binding));
  }

  function selectedSeasonArtwork(seasonNumber: number | null): Artwork | undefined {
    if (seasonNumber === null || !detail) return undefined;
    const selectedId = selectedArtwork[`season:${seasonNumber}`];
    return detail.seasons.find((season) => season.number === seasonNumber)?.artwork.find((image) => image.id === selectedId);
  }

  function selectedMovieArtwork(): Artwork | undefined {
    return detail?.artwork.find((image) => image.id === selectedArtwork.movie);
  }

  function selectedSeriesArtwork(): Artwork | undefined {
    return detail?.artwork.find((image) => image.id === selectedArtwork.series);
  }

  function buildRenameRequest(): { root: string; items: RenameTemplateInput[]; textBlacklistPatterns: string[] } {
    if (!organizerScan) throw new Error("Choose a media folder first.");
    const items: RenameTemplateInput[] = [];
    const selectedForOrganizer = selectedPathsInOrganizer();
    if (mediaKind === "movie") {
      if (!selectedForOrganizer.length) throw new Error("Select at least one movie file for this reviewed title.");
      for (const path of selectedForOrganizer) {
        const assignment = movieAssignments[path];
        const file = organizerScan.files.find((item) => item.path === path);
        if (!assignment) {
          throw new Error(`Assign a reviewed film to ${file?.name ?? path} before previewing or applying this batch.`);
        }
        items.push({ sourcePath: path, kind: "movie", template: organizerTemplates.movie, values: {
          title: assignment.title, year: assignment.year, certification: assignment.certification,
        } });
      }
      return { root: organizerScan.root, items, textBlacklistPatterns };
    }

    if (!detail || !selected) throw new Error("Search for and review a TV series before planning its rename.");
    const yearMismatch = currentSeriesYearMismatch();
    if (yearMismatch && !seriesYearConflictConfirmed) {
      throw new Error(`${yearMismatch.text}. Confirm that this is the correct series before preparing a rename.`);
    }

    const includeSeriesTree = !compactItemMode && tvRenameScope === "series-tree";
    if (includeSeriesTree) {
      if (organizerSource === "analysis") {
        throw new Error("The current library analysis may exclude files below its size filter. Scan this series folder before planning a full-series rename.");
      }
      if (organizerScan.truncated) throw new Error("Folder analysis reached its 10,000-file safety limit. Narrow the selected series folder before a full-series rename.");
      if (organizerScan.errors.length) throw new Error(`Folder analysis found ${organizerScan.errors.length} unreadable entries. Resolve these before a full-series rename.`);
      const selectedPaths = new Set(selectedForOrganizer);
      const unselected = organizerScan.files.filter((file) => !file.multipleEpisodes && !selectedPaths.has(file.path));
      if (unselected.length) {
        throw new Error(`${unselected.length} supported episode file(s) are not selected. Select all supported episodes or choose “Selected episode files only.”`);
      }
      const years = showYears();
      items.push({ sourcePath: organizerScan.root, kind: "series", template: organizerTemplates.series, values: {
        series_title: detail.title, first_year: years.first, last_year: years.last, year_range: years.range,
      } });
      for (const season of organizerScan.seasonDirectories) {
        items.push({ sourcePath: season.path, kind: "season", template: organizerTemplates.season, values: { season: String(season.season) } });
      }
    } else if (!selectedForOrganizer.length) {
      throw new Error("Select one or more episode files, or choose the full-series rename scope.");
    }

    for (const path of selectedForOrganizer) {
      const file = organizerScan.files.find((item) => item.path === path);
      if (!file) throw new Error("A selected episode is no longer present in the folder scan. Scan the folder again.");
      if (file.multipleEpisodes) {
        throw new Error(`${file.name} contains multiple episode numbers and cannot be safely assigned to one episode filename. Handle it separately.`);
      }
      const match = episodeForKey(episodeAssignments[path]);
      if (!match || match.season === null || match.episode === null) {
        throw new Error(`Choose and review a provider episode for ${file.name} before previewing or applying the rename.`);
      }
      items.push({ sourcePath: path, kind: "episode", template: organizerTemplates.episode, values: {
        series_title: detail.title, season: String(match.season), episode: String(match.episode), episode_title: match.title,
      } });
    }
    return { root: includeSeriesTree ? organizerScan.renameRoot : organizerScan.root, items, textBlacklistPatterns };
  }

  async function previewNames() {
    if (!desktop || !organizerScan) return;
    renameBusy = true;
    renameError = "";
    renameMessage = "";
    confirmRename = false;
    try {
      renamePreview = await invoke<RenamePreview>("preview_library_rename", buildRenameRequest());
    } catch (cause) {
      renamePreview = null;
      renameError = String(cause);
    } finally {
      renameBusy = false;
    }
  }

  async function applyNames() {
    if (!desktop || !organizerScan || !renamePreview?.canApply) return;
    renameBusy = true;
    renameError = "";
    const appliedPreview = renamePreview;
    const affectedSources = appliedPreview.items.filter((item) => !item.isDirectory && !item.unchanged).map((item) => item.sourcePath);
    onRenameState(affectedSources, "renaming", "Applying the reviewed filename changes.");
    let pathsApplied = false;
    try {
      const request = buildRenameRequest();
      await invoke("apply_library_rename", {
        ...request, previewId: appliedPreview.previewId, confirmed: true,
      });
      const movedPaths = new Map(appliedPreview.items.map((item) => [item.sourcePath, item.targetPath]));
      const changedPaths = [...movedPaths].filter(([source, target]) => source !== target);
      onPathsRenamed(Object.fromEntries(movedPaths));
      pathsApplied = true;
      onRenameState(changedPaths.map(([, target]) => target), "renamed", `Renamed ${changedPaths.length} path${changedPaths.length === 1 ? "" : "s"}.`);
      const nextRoot = movedPaths.get(organizerScan.root) ?? organizerScan.root;
      selectedMediaPaths = selectedMediaPaths.map((path) => movedPaths.get(path) ?? path);
      if (mediaKind === "movie") {
        const updated = { ...movieAssignments };
        for (const [sourcePath, targetPath] of movedPaths) {
          const assignment = updated[sourcePath];
          if (assignment && sourcePath !== targetPath) {
            updated[targetPath] = assignment;
            delete updated[sourcePath];
          }
        }
        movieAssignments = updated;
      } else {
        const updated = { ...episodeAssignments };
        for (const [sourcePath, targetPath] of movedPaths) {
          const assignment = episodeAssignments[sourcePath];
          if (assignment && sourcePath !== targetPath) {
            updated[targetPath] = assignment;
            delete updated[sourcePath];
          }
        }
        episodeAssignments = updated;
      }
      organizerScan = await invoke<OrganizerScan>("scan_organizer_folder", { root: nextRoot });
      if (mediaKind === "tv") syncEpisodeAssignments(organizerScan.files);
      invalidateMetadataPreview();
      renamePreview = null;
      confirmRename = false;
      renameMessage = `Renamed ${changedPaths.length} path${changedPaths.length === 1 ? "" : "s"}. Media contents were not changed.`;
    } catch (cause) {
      renameError = String(cause);
      onRenameState(pathsApplied ? [...new Set(affectedSources.map((path) => movedPathForSource(path, appliedPreview.items)))]: affectedSources, pathsApplied ? "renamed" : "failed", pathsApplied ? `Files were renamed, but the follow-up refresh failed: ${String(cause)}` : `Rename did not complete: ${String(cause)}`);
      renamePreview = null;
    } finally {
      renameBusy = false;
    }
  }

  function movedPathForSource(sourcePath: string, items: RenamePreview["items"]): string {
    return items.find((item) => item.sourcePath === sourcePath)?.targetPath ?? sourcePath;
  }
</script>

<section class="metadata-page" class:metadata-focused-review={compactItemMode} class:metadata-focused-review-tv={compactItemMode && mediaKind === "tv"}>
  {#if !compactItemMode}<div class="page-heading">
    <div>
      <div class="eyebrow">LIBRARY ORGANIZATION</div>
      <h1>Metadata workbench</h1>
      <p>Review titles, artwork and episode details alongside the movie or series they belong to.</p>
    </div>
    <div class="metadata-workbench-preferences"><span>{metadataAutoLookup ? "Suggestions run after a scan" : "Automatic suggestions are off"} · {metadataLanguage}</span><button class="link-button" onclick={() => onOpenSettings()}>Metadata settings <ArrowRight size={14} /></button></div>
  </div>{/if}

  {#if !compactItemMode}<div class="metadata-kind-switch" role="group" aria-label="Media type to organize">
    <button class:active={mediaKind === "movie"} aria-pressed={mediaKind === "movie"} onclick={() => changeMediaKind("movie")}><Film size={16} /> Movies</button>
    <button class:active={mediaKind === "tv"} aria-pressed={mediaKind === "tv"} onclick={() => changeMediaKind("tv")}><Tv size={16} /> TV series</button>
  </div>{/if}

  {#if scanResult && organizerSource === "analysis" && !compactItemMode}
    <section class="metadata-analysis-source card-surface" aria-label="Analyzed media association">
      <div><div class="eyebrow">CURRENT MEDIA ANALYSIS</div><strong>{compactItemMode ? "1 selected media file" : `${scanResult.entries.length} scanned file${scanResult.entries.length === 1 ? "" : "s"}`}</strong><small>{scanResult.root}</small></div>
      {#if mediaKind === "tv"}
        <div class="metadata-analysis-series">
          <div class="metadata-analysis-movies-heading"><div><div class="eyebrow">DETECTED SERIES</div><strong>{visibleAnalysisSeriesGroups.length} series folder{visibleAnalysisSeriesGroups.length === 1 ? "" : "s"}</strong></div><div class="metadata-movie-lookup-actions"><small>Names and year clues come from folders and episode filenames</small><button class="button primary" disabled={!desktop || lookupBusy || !visibleAnalysisSeriesGroups.some((group) => !group.ambiguous && !localSeriesNfo(group.root)?.found)} onclick={() => findSeriesMatches(visibleAnalysisSeriesGroups)}>{lookupBusy && lookupProgress?.label === "series folders" ? "Searching sources…" : `Search ${visibleAnalysisSeriesGroups.filter((group) => !group.ambiguous && !localSeriesNfo(group.root)?.found).length} series without NFO`}</button>{#if lookupBusy}<button class="link-button" onclick={stopFilenameLookup}>Stop after current lookup</button>{/if}</div></div>
          {#if lookupProgress?.label === "series folders"}<div class="metadata-lookup-progress" role="status">Searching {lookupProgress.done} of {lookupProgress.total} series folders. Duplicate title queries are reused.</div>{/if}
          {#if lookupMessage && lookupMessageScope === "series folders"}<div class="metadata-lookup-progress" role="status">{lookupMessage}</div>{/if}
          {#if visibleAnalysisSeriesGroups.length}
            <div class="metadata-analysis-series-list">
              {#each visibleAnalysisSeriesGroups as group (group.root)}
                {@const localSeries = localDirectoryAssets(group.root)}
                {@const localMetadata = localSeriesNfo(group.root)}
                {@const savedSeriesSuggestion = group.files.map((file) => suggestionForPath(file.path)).find((item) => item?.kind === "tv")}
                {@const savedSeriesResult = suggestionResult(savedSeriesSuggestion, "tv")}
                {@const seriesMatchSet = seriesSuggestions[group.root] ?? savedSeriesMatchSet(group)}
                <article class="metadata-analysis-series-row">
                  <div class="metadata-analysis-series-main"><div><strong>{basename(group.root)}</strong><small>{group.files.length} scanned episode file{group.files.length === 1 ? "" : "s"} · {group.seasonDirectories.length} season folder{group.seasonDirectories.length === 1 ? "" : "s"}</small></div><div class="metadata-record-actions"><button class="button secondary" disabled={!desktop || lookupBusy || group.ambiguous} onclick={() => savedSeriesResult && savedSeriesSuggestion?.status === "ready" && !localMetadata?.found ? reviewSeriesCandidate(group.root, savedSeriesResult) : findSeriesMatches([group], Boolean(localMetadata?.found))}>{seriesMatchSet?.status === "searching" ? "Searching…" : localMetadata?.found ? "Compare sources" : savedSeriesSuggestion?.status === "ready" ? "Review suggestion" : "Find matches"}</button><button class="link-button" disabled={!desktop || group.ambiguous} onclick={() => searchSeriesRecord(group, localMetadata?.title ?? localMetadata?.showTitle)}><Search size={14} /> Search another title</button></div></div>
                  {#if localMetadata?.found}
                    <div class="metadata-local-source"><strong>Local NFO found</strong><span>{localMetadata.title ?? localMetadata.showTitle ?? basename(group.root)}{#if localMetadata.year} · {localMetadata.year}{/if}{#if localMetadata.genres.length} · {localMetadata.genres.join(" · ")}{/if}</span>{#if localMetadata.error}<small class="metadata-error-text">{localMetadata.error}</small>{:else}<small>Local metadata stays on this device and takes priority. Use public search only if you want to compare or replace it.</small><button class="link-button" onclick={() => useLocalSeriesMetadata(group.root)}>Review and edit local show metadata</button>{/if}</div>
                  {/if}
                  {#if localSeries?.artwork.length}
                    <details class="metadata-local-artwork" ontoggle={(event) => event.currentTarget.open && loadLocalArtworkPreviews(localSeries.artwork)}><summary>Local show artwork · {localSeries.artwork.length} image{localSeries.artwork.length === 1 ? "" : "s"}</summary><div class="metadata-local-artwork-grid">{#each localSeries.artwork.slice(0, 4) as image (image.path)}<figure>{#if localArtworkPreviews[image.path]}<img src={localArtworkPreviews[image.path]} alt={`${image.role}: ${image.name}`} />{:else if localArtworkPreviewBusy[image.path]}<span>Loading preview…</span>{:else}<button class="link-button" onclick={() => loadLocalArtworkPreviews([image])}>Load preview</button>{/if}<figcaption>{image.role} · {image.name}</figcaption></figure>{/each}</div></details>
                  {/if}
                  {#if group.ambiguous}<small class="metadata-warning">This folder is shared or does not identify one show clearly. Scan a show-specific folder before attaching metadata.</small>{/if}
                  {#if seriesMatchSet}
                    {@const matchSet = seriesMatchSet}
                    <div class="metadata-inline-match-set"><span class="metadata-filename-clue">Series title · {clueCaption(matchSet.clue)} · providers receive the cleaned title only</span>
                      {#each matchSet.warnings as warning (warning)}<small class="metadata-warning">{warning}</small>{/each}
                      {#if matchSet.status === "searching"}<small>Searching public series records…</small>
                      {:else if matchSet.status === "error"}<span class="metadata-error-text">{matchSet.error}</span>
                      {:else if !matchSet.results.length}<small>No likely series matches. Search this series by another title or spelling.</small>
                      {:else}<div class="metadata-inline-candidates">{#each visibleCandidates(matchSet.results, `series:${group.root}`) as row (`${row.provider}:${row.id}`)}
                        {@const preview = candidatePreview(row)}
                        {@const evidence = candidateEvidence(row, matchSet.clue)}
                        <article class="metadata-inline-candidate">
                          {#if preview}<img class="metadata-inline-poster" src={preview.thumbnailUrl} alt={`Artwork preview for ${row.title} from ${providerLabel(row.provider)}`} loading="lazy" />{/if}
                          <div><strong>{row.title}</strong><small>{providerLabel(row.provider)} · {row.year ?? "Year unknown"} · <span class:metadata-year-conflict={evidence.conflict}>{evidence.text}</span></small></div>
                          <button class="button secondary" disabled={loadingDetail} onclick={() => reviewSeriesCandidate(group.root, row)}>{evidence.conflict ? "Review year conflict" : "Review & assign to this series"}</button>
                        </article>
                      {/each}</div>
                        {#if matchSet.results.length > initialCandidateCount}<button class="link-button metadata-show-more" aria-expanded={candidateListExpanded(`series:${group.root}`)} onclick={() => toggleCandidateList(`series:${group.root}`)}>{candidateListExpanded(`series:${group.root}`) ? "Show fewer matches" : `Show all ${matchSet.results.length} matches`}</button>{/if}
                      {/if}
                    </div>
                  {/if}
                </article>
              {/each}
            </div>
          {:else if scanResult.entries.length}<span class="metadata-warning">No episode IDs or season folders were recognized in this analysis. Scan one series folder to associate show metadata.</span>{/if}
        </div>
      {:else}
      <p>Each film keeps its own match, editable details and artwork. Search is opened from the film it belongs to.</p>
      {/if}
      {#if !scanResult.entries.length}<span class="metadata-warning">This analysis found no media files. Scan a file or folder first.</span>{/if}
    </section>
  {/if}

  {#if mediaKind === "movie" && organizerSource === "analysis" && organizerScan && compactItemMode}
    {#each visibleOrganizerFiles as file (file.path)}
      {@const localFile = localFileAssets(file.path)}
      {@const localMetadata = localNfoForFile(file.path)}
      {@const draft = movieAssignments[file.path]}
      {@const matchSet = movieSuggestions[file.path] ?? savedMovieMatchSet(file)}
      {@const posterChoices = reviewPosterOptions(file.path, draft, matchSet?.results ?? [])}
      {@const stagePoster = posterChoices.find((image) => image.id === draft?.posterId) ?? posterChoices[0]}
      {@const stageImage = (draft?.localPosterPath ? localArtworkPreviews[draft.localPosterPath] : "") || stagePoster?.imageUrl || stagePoster?.thumbnailUrl || ""}
      <section class="review-studio" aria-label="Review this film">
        <div class="review-poster-column">
          <figure class="review-poster-stage">
            {#if stageImage}<img src={stageImage} alt="" referrerpolicy="no-referrer" />{:else}<Film size={42} /><strong>No poster yet</strong>{/if}
          </figure>
          {#if posterChoices.length > 0 || (localFile?.artwork.length ?? 0) > 0}
            <div class="review-poster-rail" role="listbox" aria-label="Poster choices" {@attach previewLocalArtwork(localFile?.artwork ?? [])}>
              {#each localFile?.artwork ?? [] as image (image.path)}
                <button type="button" role="option" aria-selected={draft?.localPosterPath === image.path} class:is-selected={draft?.localPosterPath === image.path} onclick={() => chooseMovieLocalPoster(file.path, image.path)}>
                  {#if localArtworkPreviews[image.path]}<img src={localArtworkPreviews[image.path]} alt="" />{:else}<span>Local</span>{/if}
                </button>
              {/each}
              {#each posterChoices as image (image.id)}
                <button type="button" role="option" aria-selected={draft?.posterId === image.id && !draft?.localPosterPath} class:is-selected={(draft?.posterId === image.id || (!draft?.posterId && image.id === stagePoster?.id)) && !draft?.localPosterPath} onclick={() => selectMovieArtwork(file.path, image.id, image)}>
                  {#if image.thumbnailUrl || image.imageUrl}<img src={image.thumbnailUrl || image.imageUrl} alt="" referrerpolicy="no-referrer" />{:else}<span>{image.provider}</span>{/if}
                </button>
              {/each}
            </div>
          {/if}
          {#if !stageImage && !tmdbCredentialStatus.configured}<small class="review-note">Add a TMDB token in Settings to download poster choices.</small>{/if}
          {#if matchSet?.status === "searching" || loadingDetail}<p class="review-status" role="status"><LoaderCircle size={16} /> Loading the matched plot, genres, and posters…</p>{/if}
          {#if matchSet?.status === "error"}<p class="metadata-error-text" role="alert">{matchSet.error}</p>{/if}
          {#if reviewDetailError}<p class="metadata-error-text" role="alert">{reviewDetailError}</p>{/if}
        </div>
        {#if draft}
          <div class="review-fields">
            <label>Title<input bind:value={movieAssignments[file.path].title} /></label>
            <div class="review-field-row">
              <label>Year<input bind:value={movieAssignments[file.path].year} inputmode="numeric" /></label>
              <label>Certification<input bind:value={movieAssignments[file.path].certification} title="Content certificate for this film, such as PG or 12. This is not limited to the MPAA." /></label>
            </div>
            <label class="review-plot">Plot<textarea rows="8" bind:value={movieAssignments[file.path].plot}></textarea></label>
            <div class="review-field-row">
              <label>Genres<input bind:value={movieAssignments[file.path].genresText} /></label>
              <label>Audience rating<input bind:value={movieAssignments[file.path].ratingText} inputmode="decimal" title="Audience score from the matched source, from 0 to 10." /></label>
            </div>
            <label>Short description<input bind:value={movieAssignments[file.path].shortDescription} /></label>
            <label>Source URL<input bind:value={movieAssignments[file.path].sourceUrl} /></label>
            <label class="review-animated">
              <input
                type="checkbox"
                checked={animationForPath(file.path)}
                aria-label={`Animated encode for ${file.name}`}
                onchange={(event) => onAnimationChange(file.path, event.currentTarget.checked)}
              />
              <span>Animated<small>Selected when the genres include animation, anime, or cartoon. This file then uses the animation encode tune.</small></span>
            </label>
            {#if matchSet?.results.length}
              <details class="review-other-matches" open={!draft.artworkCandidates.length}>
                <summary>Other title matches · {matchSet.results.length}</summary>
                <div class="review-match-rail">
                  {#each matchSet.results.slice(0, 8) as row (`${row.provider}:${row.id}`)}
                    {@const preview = candidatePreview(row)}
                    <button type="button" onclick={() => reviewMovieCandidate(file.path, row, true)}>
                      {#if preview?.thumbnailUrl || preview?.imageUrl}<img src={preview.thumbnailUrl || preview.imageUrl} alt="" />{:else}<Film size={16} />{/if}
                      <span>{row.title}{row.year ? ` · ${row.year}` : ""}</span>
                    </button>
                  {/each}
                </div>
              </details>
            {/if}
          </div>
        {:else}
          <p class="review-status">Preparing this film…</p>
        {/if}
        <div class="review-poster-actions">
          <button class="button secondary" type="button" disabled={!desktop || lookupBusy} onclick={() => findMovieMatches([file], Boolean(localMetadata?.found))}>{matchSet?.status === "searching" ? "Searching…" : "Find posters"}</button>
          <button class="button secondary" type="button" disabled={!desktop} onclick={() => searchMovieRecord(file, draft?.title)}><Search size={15} /> Search another title</button>
          <button class="button secondary" type="button" disabled={!desktop} onclick={() => chooseLocalPoster(file)}><ImagePlus size={15} /> Use your own image</button>
        </div>
      </section>
    {/each}
  {/if}

  {#if mediaKind === "movie" && organizerSource === "analysis" && organizerScan && !compactItemMode}
    {@const movieScan = organizerScan}
    <section class="metadata-analysis-movies card-surface" aria-label="Scanned movie selection">
      <div class="metadata-analysis-movies-heading"><div><div class="eyebrow">ANALYZED MOVIES</div><strong>{visibleOrganizerFiles.length} film file{visibleOrganizerFiles.length === 1 ? "" : "s"}</strong></div><div class="metadata-movie-lookup-actions"><small>{visibleOrganizerFiles.filter((file) => selectedMediaPaths.includes(file.path)).length} selected · title clues stay on this device</small><button class="button primary" disabled={!desktop || lookupBusy || movieSearchCount(visibleOrganizerFiles) === 0} onclick={() => findMovieMatches(visibleOrganizerFiles)}>{lookupBusy && lookupProgress?.label === "analyzed films" ? "Searching sources…" : movieSearchCount(visibleOrganizerFiles) ? `Search ${movieSearchCount(visibleOrganizerFiles)} needing a match` : "All films have saved matches"}</button>{#if lookupBusy}<button class="link-button" onclick={stopFilenameLookup}>Stop after current lookup</button>{/if}</div></div>
      {#if lookupProgress?.label === "analyzed films"}<div class="metadata-lookup-progress" role="status">Searching {lookupProgress.done} of {lookupProgress.total} analyzed films. Duplicate title queries are reused.</div>{/if}
      {#if lookupMessage && lookupMessageScope === "analyzed films"}<div class="metadata-lookup-progress" role="status">{lookupMessage}</div>{/if}
      {#if visibleOrganizerFiles.length}
        <div class="metadata-analysis-movies-list">
          {#each visibleOrganizerFiles as file (file.path)}
            {@const localFile = localFileAssets(file.path)}
            {@const batchPosterTargets = batchLocalPosterTargets()}
            {@const savedSuggestion = suggestionForPath(file.path)}
            {@const matchSet = movieSuggestions[file.path] ?? savedMovieMatchSet(file)}
            {@const savedResult = suggestionResult(savedSuggestion, "movie")}
            {@const reviewedPoster = movieAssignments[file.path] ? assignedMoviePoster(movieAssignments[file.path]) : undefined}
            {@const selectedLocalPosterPreview = movieAssignments[file.path]?.localPosterPath ? localArtworkPreviews[movieAssignments[file.path].localPosterPath] : ""}
            {@const discoveredLocalPosterPath = preferredLocalPoster(localFile?.artwork ?? [], null)}
            {@const discoveredLocalPosterPreview = discoveredLocalPosterPath ? localArtworkPreviews[discoveredLocalPosterPath] : ""}
            {@const matchImage = selectedLocalPosterPreview || reviewedPoster?.thumbnailUrl || reviewedPoster?.imageUrl || savedSuggestion?.artwork?.thumbnailUrl || savedSuggestion?.artwork?.imageUrl || discoveredLocalPosterPreview || ""}
            <article class="metadata-analysis-movie-row" data-media-path={file.path}>
              <div class="metadata-analysis-movie-main"><label><input type="checkbox" checked={selectedMediaPaths.includes(file.path)} onchange={(event) => toggleMedia(file.path, event.currentTarget.checked)} /><span><strong>{file.name}</strong><small>{file.parentPath}{#if localFile?.metadata.found} · local NFO takes priority{/if}</small></span></label><div class="metadata-record-actions"><button class="button secondary" disabled={!desktop || lookupBusy} onclick={() => savedResult && savedSuggestion?.status === "ready" && !localFile?.metadata.found ? reviewMovieCandidate(file.path, savedResult, true) : findMovieMatches([file], Boolean(localFile?.metadata.found))}>{movieLookupLabel(file, Boolean(localFile?.metadata.found), matchSet)}</button><button class="link-button" disabled={!desktop} onclick={() => searchMovieRecord(file, localFile?.metadata.title)}><Search size={14} /> Search another title</button></div></div>
              <div class="metadata-movie-match-card" class:metadata-movie-match-reviewed={Boolean(movieAssignments[file.path])} class:metadata-movie-match-suggested={Boolean(!movieAssignments[file.path] && savedSuggestion?.status === "ready")} class:metadata-movie-match-empty={!movieAssignments[file.path] && savedSuggestion?.status !== "ready"}>
                <div class="metadata-movie-match-poster">{#if matchImage}<img src={matchImage} alt="" loading="lazy" referrerpolicy="no-referrer" />{:else}<span><Film size={20} /></span>{/if}</div>
                <div class="metadata-movie-match-copy"><small>{movieAssignments[file.path] ? (movieAssignments[file.path].recordId.startsWith("local") ? "LOCAL NFO" : "REVIEWED MATCH") : savedSuggestion?.status === "ready" ? "SAVED SUGGESTION" : localFile?.artwork.length ? "LOCAL ARTWORK FOUND" : matchSet?.status === "searching" ? "SEARCHING" : "NO MATCH YET"}</small><strong>{movieAssignments[file.path]?.title || savedSuggestion?.title || localFile?.metadata.title || "Needs a title match"}{movieAssignments[file.path]?.year ? ` (${movieAssignments[file.path].year})` : savedSuggestion?.year ? ` (${savedSuggestion.year})` : ""}</strong><span>{movieAssignments[file.path] ? (selectedLocalPosterPreview ? "Local poster selected" : reviewedPoster ? `${reviewedPoster.provider} artwork selected` : discoveredLocalPosterPreview ? "Local artwork found below · choose it to use it" : "Review details and choose a poster below") : savedSuggestion?.status === "ready" ? `${savedSuggestion.provider?.toUpperCase() ?? "Public source"} · review before assigning` : localFile?.artwork.length ? "Artwork was found beside this file · open it below to preview" : matchSet?.status === "searching" ? "Checking public film records…" : "Search this film or use Search another title"}</span></div>
                <div class="metadata-movie-match-actions">{#if movieAssignments[file.path]}<button class="link-button" type="button" onclick={() => movieEditorOpen = { ...movieEditorOpen, [file.path]: true }}><Pencil size={14} /> Edit</button>{:else if savedResult}<button class="button secondary" type="button" disabled={loadingDetail} onclick={() => reviewMovieCandidate(file.path, savedResult, true)}>Review & assign</button>{/if}</div>
              </div>
              {#if localFile?.metadata.found}<div class="metadata-local-source"><strong>Local NFO</strong><span>{localFile.metadata.title ?? "Title not set"}{#if localFile.metadata.year} · {localFile.metadata.year}{/if}</span>{#if localFile.metadata.error}<small class="metadata-error-text">{localFile.metadata.error}</small>{:else}<small>{localFile.metadata.paths.map(basename).join(" · ")} · local details stay first unless you compare sources</small>{/if}</div>{/if}
              {#if localFile?.artwork.length}
                <details class="metadata-local-artwork" ontoggle={(event) => event.currentTarget.open && loadLocalArtworkPreviews(localFile.artwork)}>
                  <summary>Local artwork · {localFile.artwork.length} image{localFile.artwork.length === 1 ? "" : "s"}</summary>
                  <div class="metadata-local-artwork-grid">
                    {#each localFile.artwork.slice(0, 4) as image (image.path)}
                      <figure>
                        {#if localArtworkPreviews[image.path]}<img src={localArtworkPreviews[image.path]} alt={`${image.role}: ${image.name}`} />{:else if localArtworkPreviewBusy[image.path]}<span>Loading preview…</span>{:else}<button class="link-button" onclick={() => loadLocalArtworkPreviews([image])}>Load preview</button>{/if}
                        {#if movieAssignments[file.path]}
                          <button type="button" class="metadata-artwork-choice" class:selected={movieAssignments[file.path].localPosterPath === image.path} onclick={() => chooseMovieLocalPoster(file.path, image.path)}>{movieAssignments[file.path].localPosterPath === image.path ? "Clear local poster" : "Use as film poster"}</button>
                          {#if batchPosterTargets.length > 1}<button type="button" class="metadata-artwork-choice metadata-artwork-batch-choice" onclick={() => previewBatchLocalPoster(file.path, image.path)}>Use for {batchPosterTargets.length} selected films</button>{/if}
                          {#if pendingBatchLocalPoster?.sourcePath === file.path && pendingBatchLocalPoster.artworkPath === image.path}
                            <div class="metadata-local-batch-review">
                              <strong>Apply this image to {pendingBatchLocalPoster.mediaPaths.length} films?</strong>
                              <small>The existing image will be selected for these files. No image file will be copied or replaced.</small>
                              <ul>{#each pendingBatchLocalPoster.mediaPaths as path (path)}<li><strong>{movieScan.files.find((item) => item.path === path)?.name ?? basename(path)}</strong><code>{path}</code></li>{/each}</ul>
                              <div><button class="text-button" type="button" onclick={() => pendingBatchLocalPoster = null}>Cancel</button><button class="button secondary" type="button" onclick={applyBatchLocalPoster}>Apply to selected films</button></div>
                            </div>
                          {/if}
                        {:else}<small>Assign a film title before choosing its poster.</small>{/if}
                        <figcaption>{image.role} · {image.name}</figcaption>
                      </figure>
                    {/each}
                  </div>
                </details>
              {/if}
              {#if matchSet}
                <details class="metadata-inline-match-set metadata-inline-match-details" open={matchSet.status !== "ready" || (!savedSuggestion && !movieAssignments[file.path])}>
                  <summary>{#if matchSet.status === "searching"}Searching public film records…{:else if matchSet.status === "error"}Lookup needs attention{:else if matchSet.results.length}{matchSet.results.length} possible match{matchSet.results.length === 1 ? "" : "es"} · review details{:else}No likely match · search another title{/if}</summary>
                  <div class="metadata-inline-match-content">
                    <span class="metadata-filename-clue">Title clue · {clueCaption(matchSet.clue)} · providers receive the cleaned title only</span>
                    {#each matchSet.warnings as warning (warning)}<small class="metadata-warning">{warning}</small>{/each}
                    {#if matchSet.status === "searching"}<small>Searching public film records…</small>
                    {:else if matchSet.status === "error"}<span class="metadata-error-text">{matchSet.error}</span>
                    {:else if !matchSet.results.length}<small>No likely matches. Search this film by another title or spelling.</small>
                    {:else}<div class="metadata-inline-candidates">
                      {#each visibleCandidates(matchSet.results, `movie:${file.path}`) as row (`${row.provider}:${row.id}`)}
                        {@const preview = candidatePreview(row)}
                        {@const evidence = candidateEvidence(row, matchSet.clue)}
                        <article class="metadata-inline-candidate">
                          {#if preview}<img class="metadata-inline-poster" src={preview.thumbnailUrl || preview.imageUrl} alt={`Artwork preview for ${row.title} from ${providerLabel(row.provider)}`} loading="lazy" referrerpolicy="no-referrer" />{/if}
                          <div><strong>{row.title}</strong><small>{providerLabel(row.provider)} · {row.year ?? "Year unknown"} · <span class:metadata-year-conflict={evidence.conflict}>{evidence.text}</span></small></div>
                          <button class="button secondary" disabled={loadingDetail} onclick={() => reviewMovieCandidate(file.path, row, !evidence.conflict)}>{evidence.conflict ? "Review year conflict" : "Review & assign to this film"}</button>
                        </article>
                      {/each}
                    </div>
                      {#if matchSet.results.length > initialCandidateCount}<button class="link-button metadata-show-more" aria-expanded={candidateListExpanded(`movie:${file.path}`)} onclick={() => toggleCandidateList(`movie:${file.path}`)}>{candidateListExpanded(`movie:${file.path}`) ? "Show fewer matches" : `Show all ${matchSet.results.length} matches`}</button>{/if}
                    {/if}
                  </div>
                </details>
              {/if}
              {#if movieAssignments[file.path]}
                {@const draft = movieAssignments[file.path]}
                <details class="metadata-inline-draft" bind:open={movieEditorOpen[file.path]}>
                  <summary>Edit this film’s metadata</summary>
                  <label>Title<input bind:value={movieAssignments[file.path].title} oninput={() => { renamePreview = null; invalidateMetadataPreview(); }} /></label>
                  <div class="metadata-inline-draft-row"><label>Year<input bind:value={movieAssignments[file.path].year} inputmode="numeric" oninput={() => { renamePreview = null; invalidateMetadataPreview(); }} /></label><label>Certification<input bind:value={movieAssignments[file.path].certification} oninput={() => { renamePreview = null; invalidateMetadataPreview(); }} /></label></div>
                  <label>Short description<input bind:value={movieAssignments[file.path].shortDescription} oninput={invalidateMetadataPreview} /></label>
                  <label>Plot<textarea rows="3" bind:value={movieAssignments[file.path].plot} oninput={invalidateMetadataPreview}></textarea></label>
                  <div class="metadata-inline-draft-row"><label>Genres<input bind:value={movieAssignments[file.path].genresText} oninput={invalidateMetadataPreview} /><small>Separate with commas or semicolons.</small></label><label>Rating · 0 to 10<input type="number" min="0" max="10" step="0.1" bind:value={movieAssignments[file.path].ratingText} oninput={invalidateMetadataPreview} /></label></div>
                  <label>Source URL · optional<input type="url" placeholder="https://…" bind:value={movieAssignments[file.path].sourceUrl} oninput={invalidateMetadataPreview} /></label>
                  {#if draft.artworkCandidates.length}<label>Poster<select disabled={artworkBusy} bind:value={movieAssignments[file.path].posterId} onchange={() => { chooseMovieProviderPoster(file.path); invalidateMetadataPreview(); }}><option value="">No poster selected</option>{#each draft.artworkCandidates as image (image.id)}<option value={image.id}>{image.provider} · {image.title}{image.license ? ` · ${image.license}` : ""}</option>{/each}</select></label>{/if}
                  <div class="metadata-local-poster-actions"><button class="link-button" type="button" disabled={localPosterBusy} onclick={() => chooseLocalPoster(file)}>Choose your own poster image</button><small>JPEG, PNG, or WebP · copied beside this video · existing files are never overwritten</small></div>
                  {#if pendingLocalPoster?.target.directory === movieArtworkTarget(file).directory && pendingLocalPoster?.target.basename === movieArtworkTarget(file).basename}
                    <div class="metadata-local-poster-review"><strong>{pendingLocalPoster.saveBesideMedia ? "Copy this poster beside the film?" : "Save this poster to the app artwork cache?"}</strong><small>Image: {pendingLocalPoster.sourcePath}</small><code>{pendingLocalPoster.targetPath}</code><div><button class="link-button" disabled={localPosterBusy} onclick={() => pendingLocalPoster = null}>Cancel</button><button class="button secondary" disabled={localPosterBusy} onclick={saveLocalPoster}>{localPosterBusy ? "Saving poster…" : "Save poster"}</button></div></div>
                  {/if}
                  {#if localPosterError}<small class="metadata-error-text" role="status">{localPosterError}</small>{/if}
                  {#if localPosterMessage}<small class="metadata-success-text" role="status">{localPosterMessage}</small>{/if}
                </details>
              {:else}<div class="metadata-inline-draft-loading" role="status"><LoaderCircle size={14} class="spin" /> Preparing editable fields from the filename…</div>{/if}
            </article>
          {/each}
        </div>
      {:else}
        <p>The current analysis contains no movie files. Switch to TV series or scan a movie folder.</p>
      {/if}
    </section>
  {/if}

  {#if !compactItemMode}<p class="metadata-lookup-privacy">Search results stay attached to the film or series that started the search. Enabled providers receive only the cleaned title; filenames, paths, year clues, NFO contents, local artwork and video bytes stay on this device. Existing NFO details remain available for review. Matches are suggestions until you assign and confirm them.</p>{/if}
  {#if localAssetScanError}<div class="metadata-error" role="status"><TriangleAlert size={17} /><span>Local NFO and artwork inspection could not finish: {localAssetScanError}. Public lookup is paused until the local check succeeds.</span></div>{/if}

  <div class="metadata-grid" class:metadata-focused-grid={compactItemMode}>
    <section class="metadata-search card-surface" class:metadata-search-active={Boolean(searchTarget)}>
      {#if searchTarget}
        <div class="metadata-card-title"><span class="settings-icon"><Search size={19} /></span><div><h2>Search for {searchTarget.kind === "movie" ? "this film" : "this series"}</h2><p>{searchTarget.label} · searches use enabled sources in Settings.</p></div><button class="icon-button" aria-label="Close title search" title="Close title search" onclick={closeTitleSearch}><X size={17} /></button></div>
        <div class="metadata-query-row">
          <label>Title or alternate spelling
            <input bind:this={titleSearchInput} bind:value={query} onkeydown={(event) => event.key === "Enter" && !searching && searchTitles()} placeholder={searchTarget.kind === "movie" ? "e.g. Gnomeo & Juliet" : "e.g. The Expanse"} />
          </label>
          <button class="button primary" disabled={searching || loadingDetail || !query.trim()} onclick={searchTitles}>
            {#if searching}<LoaderCircle size={16} class="spin" /> Searching sources{:else}<Search size={16} /> Search sources{/if}
          </button>
        </div>
        {#if metadataSources.tmdb && !tmdbCredentialStatus.configured}
          <div class="metadata-setup-notice" role="status"><div><strong>TMDB needs an API token</strong><span>TMDB results are skipped until you save one in the desktop app. Other selected sources still work.</span></div><button class="button secondary" onclick={() => onOpenSettings("tmdb")}>Set up TMDB <ArrowRight size={14} /></button></div>
        {/if}
        {#if metadataSources.tvdb && !tvdbCredentialStatus.configured}
          <div class="metadata-setup-notice" role="status"><div><strong>TheTVDB needs a project API key</strong><span>Its results are skipped until valid project credentials are saved in Settings.</span></div><button class="button secondary" onclick={() => onOpenSettings("tvdb")}>Set up TheTVDB <ArrowRight size={14} /></button></div>
        {/if}
      {:else}
        <div class="metadata-search-empty"><span class="settings-icon"><Search size={19} /></span><div><strong>Search from a library record</strong><p>Choose “Search another title” on a scanned film or series to keep the match, artwork and edits connected to it.</p><button class="link-button" onclick={() => onOpenSettings()}>Review metadata sources and language <ArrowRight size={14} /></button></div></div>
      {/if}
      {#if results.some((row) => row.provider === "tmdb") || selected?.provider === "tmdb"}
        <div class="metadata-attribution"><div><a href="https://www.themoviedb.org" target="_blank" rel="noreferrer" aria-label="The Movie Database (TMDB)"><img src={tmdbLogo} alt="TMDB" /></a><strong>Credits · The Movie Database API</strong></div><span>This product uses the TMDB API but is not endorsed or certified by TMDB. API data and images are provided by TMDB and their contributors; review applicable terms before reuse.</span></div>
      {/if}
      {#each searchWarnings as warning (warning)}<div class="metadata-search-note" role="status"><TriangleAlert size={15} />{warning}</div>{/each}
      {#if error}<div class="metadata-error"><TriangleAlert size={17} /><span>{error}</span></div>{/if}
      {#if results.length}
          <div class="metadata-results-heading"><div><span class="eyebrow">POSSIBLE MATCHES</span><strong>{results.length} results · showing {Math.min(results.length, candidateListExpanded("manual-search") ? results.length : initialCandidateCount)}</strong></div><small>Order uses title, year and provider agreement. It is not a probability score.</small></div>
        <div class="metadata-results">
          {#each visibleCandidates(results, "manual-search") as row (`${row.provider}:${row.id}`)}
            {@const preview = candidatePreview(row)}
            {@const evidence = manualSearchClue ? candidateEvidence(row, manualSearchClue) : null}
            <article class="metadata-result" class:selected={selected?.provider === row.provider && selected?.id === row.id}>
              {#if preview}<img src={preview.thumbnailUrl} alt={`Artwork preview for ${row.title} from ${providerLabel(row.provider)}`} loading="lazy" />{:else}<span class="metadata-result-placeholder">{#if row.kind === "movie"}<Film size={20} />{:else}<Tv size={20} />{/if}</span>{/if}
              <span class="metadata-result-copy"><strong>{row.title}</strong><small>{providerLabel(row.provider)} · {row.year ?? "Year unknown"}{#if row.description} · {row.description.slice(0, 125)}{/if}{#if evidence}<span class:metadata-year-conflict={evidence.conflict}>{evidence.conflict ? ` · ${evidence.text}` : ` · ${evidence.text}`}</span>{/if}</small></span>
              {#if evidence?.conflict}<span class="metadata-score metadata-score-conflict">Year conflict<small>review first</small></span>{:else}<span class="metadata-score">{evidence?.text.startsWith("Exact title") ? "Exact title" : "Review match"}<small>{evidence?.text.includes("year matches") ? "year matches" : "check details"}</small></span>{/if}
              <button class="button secondary" disabled={loadingDetail} onclick={() => row.kind === "movie" && searchTarget?.kind === "movie" ? reviewMovieCandidate(searchTarget.path, row, !evidence?.conflict) : reviewTargetCandidate(row)}>{loadingDetail && selected?.id === row.id ? "Loading…" : evidence?.conflict ? "Review year conflict" : "Review & assign"}</button>
            </article>
          {/each}
        </div>
        {#if results.length > initialCandidateCount}<button class="link-button metadata-show-more" aria-expanded={candidateListExpanded("manual-search")} onclick={() => toggleCandidateList("manual-search")}>{candidateListExpanded("manual-search") ? "Show fewer matches" : `Show all ${results.length} matches`}</button>{/if}
      {/if}
    </section>

    <section class="metadata-detail card-surface">
      {#if loadingDetail}
        <div class="metadata-empty"><LoaderCircle size={24} class="spin" /><strong>Loading {mediaKind === "movie" ? "film" : "series"} details</strong><span>{mediaKind === "movie" ? "Getting title information, certification and artwork." : "Getting show details, seasons, episodes and series artwork."}</span></div>
      {:else if detail}
        <div class="metadata-detail-heading"><div><div class="eyebrow">{detail.attribution.startsWith("Local NFO metadata") ? "LOCAL NFO RECORD" : "REVIEWED SOURCE RECORD"}</div><h2>{detail.title}{#if detail.year} <span>({detail.year})</span>{/if}</h2><p>{detail.originalTitle && detail.originalTitle !== detail.title ? `Original title: ${detail.originalTitle} · ` : ""}{detail.genres.join(" · ") || detail.attribution}</p><small class="metadata-provider-attribution">{detail.attribution}</small>{#if detail.episodeOrderLabel}<div class="metadata-episode-order-note"><strong>Episode numbering:</strong> {detail.episodeOrderLabel}. Check it against the source files before assigning episodes.</div>{/if}</div>{#if safeHttpUrl(detail.sourceUrl)}<a class="button secondary" href={safeHttpUrl(detail.sourceUrl) ?? undefined} target="_blank" rel="noreferrer"><ExternalLink size={15} /> Open source</a>{/if}</div>
        {#if currentSeriesYearMismatch()}
          {@const mismatch = currentSeriesYearMismatch()}
          <div class="metadata-episode-order-note metadata-series-year-warning" role="alert">
            <strong>Check the series year before renaming</strong>
            <span>{mismatch?.text}. This may be a different series or a different listing year.</span>
            <label><input type="checkbox" bind:checked={seriesYearConflictConfirmed} onclick={() => { renamePreview = null; confirmRename = false; }} /> I checked the mismatch and want to use this record for TV renaming.</label>
          </div>
        {/if}
        <div class="metadata-facts">
          {#if detail.certification}<div><small>RATING</small><strong>{detail.certification}</strong></div>{/if}
          {#if detail.runtime}<div><small>RUNTIME</small><strong>{detail.runtime} min</strong></div>{/if}
          {#if detail.rating !== null}<div><small>USER RATING</small><strong>{detail.rating.toFixed(1)} / 10</strong></div>{/if}
          {#if mediaKind === "tv"}<div><small>SEASONS</small><strong>{detail.seasons.length}</strong></div><div><small>EPISODES</small><strong>{detail.episodes.length}</strong></div>{/if}
        </div>
        {#if mediaKind === "tv" && organizerScan && localDirectoryAssets(organizerScan.root)?.artwork.length}
          {@const localShowArt = localDirectoryAssets(organizerScan.root)?.artwork ?? []}
          <section class="metadata-local-artwork-section">
            <div class="metadata-artwork-heading"><h3>Artwork already beside this series</h3><small>Choose a local image to use as the show poster. The original image stays where it is.</small></div>
            <div class="metadata-local-artwork-grid">
              {#each localShowArt.slice(0, 4) as image (image.path)}
                <figure>
                  {#if localArtworkPreviews[image.path]}<img src={localArtworkPreviews[image.path]} alt={`${image.role}: ${image.name}`} />{:else if localArtworkPreviewBusy[image.path]}<span>Loading preview…</span>{:else}<button class="link-button" onclick={() => loadLocalArtworkPreviews([image])}>Load preview</button>{/if}
                  <button type="button" class="metadata-artwork-choice" class:selected={seriesAssignments[currentReviewedSeriesRoot()]?.localPosterPath === image.path} onclick={() => chooseSeriesLocalPoster(image.path)}>{seriesAssignments[currentReviewedSeriesRoot()]?.localPosterPath === image.path ? "Clear local poster" : "Use as series poster"}</button>
                  <figcaption>{image.role} · {image.name}</figcaption>
                </figure>
              {/each}
            </div>
          </section>
        {/if}
        <div class="metadata-edit-fields">
          <label>Short description
            <input bind:value={detail.shortDescription} oninput={() => { renamePreview = null; invalidateMetadataPreview(); }} placeholder="Short summary, tagline, or subtitle" />
          </label>
          <label>Long description
            <textarea bind:value={detail.longDescription} rows="5" oninput={() => { renamePreview = null; invalidateMetadataPreview(); }} placeholder="Long synopsis or plot"></textarea>
          </label>
          <small>{mediaKind === "movie" ? "These values are copied when you assign this film to selected files. You can then edit each film's text, rating, and poster separately." : "Review and edit the series text saved in a Kodi-compatible sidecar. Episode descriptions are edited on their individual file assignments below."} A later confirmed rebuild embeds supported fields into the new Matroska file.</small>
        </div>
        {#if detail.artwork.length}
          <div class="metadata-artwork-heading"><h3>{mediaKind === "movie" ? "Poster options" : "Series poster options"}</h3><small>Select a preferred poster after checking its source and reuse terms.</small></div>
          <div class="metadata-artwork-grid">
            {#each detail.artwork as image (image.id)}
              <article class="metadata-artwork" class:selected={selectedArtwork[mediaKind === "movie" ? "movie" : "series"] === image.id}>
                <a href={image.sourceUrl} target="_blank" rel="noreferrer"><img src={image.thumbnailUrl} alt={image.title} loading="lazy" /></a>
                <div><strong>{image.title}</strong><small>{image.attribution ?? image.provider}{#if image.license} · {image.license}{/if}</small>{#if image.licenseUrl}<a href={image.licenseUrl} target="_blank" rel="noreferrer">Review image license</a>{/if}</div>
                <button class="metadata-artwork-choice" disabled={artworkBusy} onclick={() => mediaKind === "tv" ? chooseSeriesProviderPoster(image.id) : (selectedArtwork = { ...selectedArtwork, movie: image.id }, pendingArtworkSave = null, artworkMessage = "")}>
                  {selectedArtwork[mediaKind === "movie" ? "movie" : "series"] === image.id ? "Selected" : mediaKind === "movie" ? "Use film poster" : "Use series poster"}
                </button>
              </article>
            {/each}
          </div>
        {/if}
        {#if organizerScan && mediaKind === "movie" && selectedMovieArtwork()}
          <div class="metadata-artwork-save-actions">
            <button class="button secondary" disabled={!movieArtworkTargets().length || artworkBusy} onclick={stageCurrentFilmPoster}>Review poster destinations for matching files</button>
            {#if selectedPathsInOrganizer().length && !movieArtworkTargets().length}<small class="metadata-warning">Assign this reviewed title to the selected movie files before saving its poster.</small>{/if}
          </div>
        {:else if organizerScan && mediaKind === "tv"}
          {@const seriesRoot = organizerScan.root}
          {@const seriesArt = selectedSeriesArtwork()}
          {#if seriesArt}
            {@const seriesExtension = artworkFileExtension(seriesArt)}
            <div class="metadata-artwork-save-actions"><button class="button secondary" disabled={artworkBusy} onclick={() => stageArtworkSave(seriesArt, seriesArtworkTargets(seriesRoot), "series poster for Kodi, Jellyfin, and Plex", [{ kind: "series", seriesRoot, candidateId: seriesArt.id }, null])}>Review series poster destinations</button><small>Copies will use <code>folder.{seriesExtension}</code> and <code>poster.{seriesExtension}</code> at the series root, with one source and license record for each. The saved folder image will be used in rebuilds.</small></div>
          {/if}
        {/if}
        {#if detail.seasons.length}
          <section class="metadata-season-list"><h3>Series seasons</h3>{#each detail.seasons as season (season.number)}
            <div class="metadata-season-row"><strong>{season.title ?? `Season ${season.number}`}</strong><span>{season.yearStart ?? "Dates unknown"}</span>
              {#if season.artwork.length}<div class="metadata-season-artwork">{#each season.artwork as image (image.id)}
                <button class:selected={selectedArtwork[`season:${season.number}`] === image.id} disabled={artworkBusy} title={`Choose ${image.title} for ${season.title ?? `Season ${season.number}`}`} onclick={() => chooseSeriesProviderSeasonPoster(season.number ?? 0, image.id)}>
                  <img src={image.thumbnailUrl} alt={image.title} loading="lazy" />{selectedArtwork[`season:${season.number}`] === image.id ? "Selected" : "Use season poster"}
                </button>
              {/each}</div>{/if}
              {#if organizerScan && season.number !== null}
                {@const seasonNumber = season.number}
                {@const seasonRoot = organizerScan.root}
                {@const seasonDirectory = organizerScan.seasonDirectories.find((item) => item.season === seasonNumber)}
                {@const chosenSeasonPoster = selectedSeasonArtwork(seasonNumber)}
                {#if seasonDirectory && localDirectoryAssets(seasonDirectory.path)?.artwork.length}
                  {@const localSeasonArt = localDirectoryAssets(seasonDirectory.path)?.artwork ?? []}
                  <details class="metadata-local-artwork" ontoggle={(event) => event.currentTarget.open && loadLocalArtworkPreviews(localSeasonArt)}>
                    <summary>Local season artwork · {localSeasonArt.length} image{localSeasonArt.length === 1 ? "" : "s"}</summary>
                    <div class="metadata-local-artwork-grid">
                      {#each localSeasonArt.slice(0, 4) as image (image.path)}
                        <figure>
                          {#if localArtworkPreviews[image.path]}<img src={localArtworkPreviews[image.path]} alt={`${image.role}: ${image.name}`} />{:else if localArtworkPreviewBusy[image.path]}<span>Loading preview…</span>{:else}<button class="link-button" onclick={() => loadLocalArtworkPreviews([image])}>Load preview</button>{/if}
                          <button type="button" class="metadata-artwork-choice" class:selected={seriesAssignments[currentReviewedSeriesRoot()]?.localSeasonPosterPaths[String(seasonNumber)] === image.path} onclick={() => chooseSeriesLocalSeasonPoster(seasonNumber, image.path)}>{seriesAssignments[currentReviewedSeriesRoot()]?.localSeasonPosterPaths[String(seasonNumber)] === image.path ? "Clear season poster" : `Use for Season ${String(seasonNumber).padStart(2, "0")}`}</button>
                          <figcaption>{image.role} · {image.name}</figcaption>
                        </figure>
                      {/each}
                    </div>
                  </details>
                {/if}
                {#if seasonDirectory && chosenSeasonPoster}<button class="link-button" disabled={artworkBusy} onclick={() => stageArtworkSave(chosenSeasonPoster, seasonArtworkTargets(seasonRoot, seasonDirectory.path, seasonNumber), `season poster for Kodi, Jellyfin, and Plex (Season ${String(seasonNumber).padStart(2, "0")})`, [{ kind: "season", seriesRoot: seasonRoot, seasonNumber, candidateId: chosenSeasonPoster.id }, null, null])}>Save season poster</button>{/if}
              {/if}
            </div>
          {/each}</section>
        {/if}
        {#if detail.episodes.length}
          <details class="metadata-episodes"><summary>Episode metadata · {detail.episodes.length} entries</summary><div>{#each detail.episodes as episode (String(episode.season) + "-" + episode.episode)}<article><strong>S{String(episode.season).padStart(2, "0")}E{String(episode.episode).padStart(2, "0")} · {episode.title}</strong><small>{episode.airdate ?? "Air date unknown"}{#if episode.summary} · {episode.summary}{/if}</small><a href={episode.sourceUrl} target="_blank" rel="noreferrer">Open source</a></article>{/each}</div></details>
        {/if}

        <section class="metadata-organizer" class:compact-item-organizer={compactItemMode}>
          <div class="metadata-artwork-heading"><h3>{mediaKind === "movie" ? "Review film filenames" : "Organize this series"}</h3><small>{organizerSource === "analysis" ? "Using the media files from your current analysis. Metadata writes use this selected film or series association." : "Folder scans stay local. Nothing is renamed until you inspect the before/after list and confirm."}</small></div>
          {#if organizerSource === "analysis" && mediaKind === "tv" && organizerScan}
            <button class="button secondary" disabled={renameBusy || artworkBusy} onclick={scanCurrentSeriesFolder}><Film size={16} /> Scan complete series folder for full rename</button>
          {/if}
          <button class="button secondary" disabled={renameBusy || artworkBusy} onclick={chooseOrganizerFolder}>
            {#if renameBusy}<LoaderCircle size={16} class="spin" /> Scanning{:else}<Film size={16} /> {organizerSource === "analysis" ? "Choose a different folder" : mediaKind === "movie" ? "Choose movie folder" : "Choose one series folder"}{/if}
          </button>
          {#if mediaKind === "tv" && !organizerScan && scanResult}
            <div class="metadata-empty"><strong>{selectedAnalysisSeriesRoot ? "Choose a show-specific folder" : "No series folder selected from the analysis."}</strong><span>{selectedAnalysisSeriesRoot ? "A flat or shared folder cannot safely hold one show's metadata. Select a show folder with its own season or episode files." : "Choose a detected series folder above, or select a series folder to scan manually."}</span></div>
          {/if}
          {#if organizerScan}
          <div class="metadata-folder-summary"><strong>{organizerScan.root}</strong><span>{organizerScan.files.length} video files · {organizerScan.seasonDirectories.length} season folders</span>
              <div><button class="link-button" onclick={() => selectAllMedia(true)}>{compactItemMode ? mediaKind === "movie" ? "Select this film" : "Select this episode" : mediaKind === "movie" ? "Select these files" : "Select supported episodes"}</button><button class="link-button" onclick={() => selectAllMedia(false)}>Clear selection here</button><small>{selectedPathsInOrganizer().length} selected here · shared with Media library</small>{#if mediaKind === "tv"}<button class="link-button" disabled={!desktop} onclick={searchCurrentSeriesRecord}><Search size={14} /> Search another series title</button>{/if}</div>
              {#if mediaKind === "tv" && organizerScan.files.some((file) => file.multipleEpisodes)}<small class="metadata-warning">Multi-episode files stay out of this rename plan because one filename cannot safely represent several episode matches.</small>{/if}
              {#if mediaKind === "tv" && organizerScan.episodeGaps.length}<small class="metadata-warning">Possible gaps in the filename numbering: {#each organizerScan.episodeGaps as gap, index}{#if index > 0} · {/if}Season {gap.season}: {gap.examples.map((episode) => String(episode).padStart(2, "0")).join(", ")}{#if gap.missingCount > gap.examples.length} and {gap.missingCount - gap.examples.length} more{/if}{/each}. Check against the episodes you expect; this filename check cannot identify specials or alternate episode orders.</small>{/if}
              {#if organizerScan.truncated}<small class="metadata-warning">The 10,000-file scan limit was reached; series-wide rename is disabled.</small>{/if}
            {#if organizerScan.errors.length}<small class="metadata-warning">{organizerScan.errors.length} entries could not be read. Resolve those before a series-wide rename.</small>{/if}
            </div>
            {#if pendingArtworkSave}
              <section class="metadata-artwork-save-review">
                <div><strong>{pendingArtworkSave.saveBesideMedia ? `Save ${pendingArtworkSave.label} beside the media?` : `Save ${pendingArtworkSave.label} to the app cache?`}</strong><span>{pendingArtworkSave.saveBesideMedia ? "Poster and source/license files will be placed beside the media. Existing files are never overwritten." : "The poster and source/license details will stay in the private app artwork cache."} The selected image is attached when you rebuild the media into a new Matroska file if artwork embedding is enabled in Settings.</span></div>
                <div class="metadata-artwork-save-paths">{#each plannedArtworkPaths() as path (path)}<code>{path}</code><code>{path}.attribution.json</code>{/each}</div>
                <div class="metadata-rename-actions"><button class="button secondary" disabled={artworkBusy} onclick={() => pendingArtworkSave = null}>Cancel</button><button class="button primary" disabled={artworkBusy} onclick={applyArtworkSave}>{#if artworkBusy}<LoaderCircle size={16} class="spin" /> Downloading poster{:else if pendingArtworkSave.saveBesideMedia}Save poster files{:else}Save to app cache{/if}</button></div>
              </section>
            {/if}
            {#if artworkError}<div class="metadata-error"><TriangleAlert size={17} /><span>{artworkError}</span></div>{/if}
            {#if artworkMessage}<div class="metadata-success"><strong>Poster saved</strong><span>{artworkMessage}</span></div>{/if}
            {#if mediaKind === "movie"}
              <div class="metadata-movie-assign-actions">
                <span>Assign a separate reviewed match to each film. Current title details and the selected poster copy into chosen files; edit any file independently below.</span>
                <button class="button secondary" disabled={!detail || !selectedPathsInOrganizer().length || renameBusy} onclick={() => assignCurrentMovie(selectedPathsInOrganizer())}>Copy current match to selected files ({selectedPathsInOrganizer().length})</button>
              </div>
              <label class="metadata-template-label">Film pattern
                <input bind:value={organizerTemplates.movie} oninput={() => renamePreview = null} onchange={saveOrganizerTemplates} />
                <small>Shortcodes: {`{title}`}, {`{year}`}, {`{certification}`}, {`{ext}`}. Optional parts use [[ ... ]].</small>
              </label>
            {:else}
              <div class="metadata-rename-scope" role="radiogroup" aria-label="TV rename scope">
                <label class:selected={tvRenameScope === "series-tree"}>
                  <input type="radio" name="tv-rename-scope" checked={tvRenameScope === "series-tree"} disabled={organizerSource === "analysis"} onchange={() => { tvRenameScope = "series-tree"; renamePreview = null; confirmRename = false; }} />
                  <span><strong>Organize the full series</strong><small>{organizerSource === "analysis" ? "Scan the complete series folder first; the library analysis may omit files below its size filter." : "Rename the series folder, season folders, and every selected supported episode. The scan must be complete and all supported episodes must be selected and matched."}</small></span>
                </label>
                <label class:selected={tvRenameScope === "episodes-only"}>
                  <input type="radio" name="tv-rename-scope" checked={tvRenameScope === "episodes-only"} onchange={() => { tvRenameScope = "episodes-only"; renamePreview = null; confirmRename = false; }} />
                  <span><strong>Rename selected episode files only</strong><small>Keep series and season folders exactly as they are. Select one episode for an individual rename or several for a batch.</small></span>
                </label>
              </div>
              <div class="metadata-template-grid">
                <label class="metadata-template-label">Series folder pattern<input bind:value={organizerTemplates.series} oninput={() => renamePreview = null} onchange={saveOrganizerTemplates} /><small>{`{series_title}`}, {`{year_range}`}, {`{first_year}`}, {`{last_year}`}</small></label>
                <label class="metadata-template-label">Season folder pattern<input bind:value={organizerTemplates.season} oninput={() => renamePreview = null} onchange={saveOrganizerTemplates} /><small>{`{season}`}, {`{season:02}`}</small></label>
                <label class="metadata-template-label">Episode filename pattern<input bind:value={organizerTemplates.episode} oninput={() => renamePreview = null} onchange={saveOrganizerTemplates} /><small>{`{season:02}`}, {`{episode:02}`}, {`{episode_title}`}, {`{ext}`}</small></label>
              </div>
            {/if}
            <div class="metadata-organizer-files">
              {#each visibleOrganizerFiles as file (file.path)}
                {@const checked = selectedMediaPaths.includes(file.path)}
                {@const assigned = episodeForKey(episodeAssignments[file.path])}
                <article>
                  <label class="metadata-file-check"><input type="checkbox" checked={checked} disabled={mediaKind === "tv" && file.multipleEpisodes} onchange={(event) => toggleMedia(file.path, event.currentTarget.checked)} /><span><strong>{file.name}</strong><small>{file.parentPath}</small></span></label>
                  {#if mediaKind === "movie"}<button class="link-button" disabled={!desktop} onclick={() => searchMovieRecord(file, movieAssignments[file.path]?.title)}><Search size={14} /> Search this film title</button>{/if}
                  {#if mediaKind === "tv"}
                    {#if file.multipleEpisodes}<span class="metadata-warning">Multi-episode file needs manual split handling</span>{:else}
                      <select aria-label={`Match episode for ${file.name}`} value={episodeAssignments[file.path] ?? ""} onchange={(event) => { episodeAssignments = { ...episodeAssignments, [file.path]: event.currentTarget.value }; renamePreview = null; invalidateMetadataPreview(); }}>
                        <option value="">Choose episode…</option>{#each detail.episodes as episode (String(episode.season) + "/" + episode.episode)}
                          {#if episode.season !== null && episode.episode !== null}<option value={`${episode.season}/${episode.episode}`}>S{String(episode.season).padStart(2, "0")}E{String(episode.episode).padStart(2, "0")} · {episode.title}</option>{/if}
                        {/each}
                      </select>
                      {#if assigned}<small class="metadata-assignment">Matched from episode code · review before applying</small>
                        <div class="metadata-episode-edit">
                          <label>Episode title<input value={assigned.title} oninput={(event) => updateEpisode(assigned, "title", event.currentTarget.value)} /></label>
                          <label>Air date<input value={assigned.airdate ?? ""} placeholder="YYYY-MM-DD" oninput={(event) => updateEpisode(assigned, "airdate", event.currentTarget.value)} /></label>
                          <label>Episode description<textarea rows="3" value={assigned.summary ?? ""} oninput={(event) => updateEpisode(assigned, "summary", event.currentTarget.value)}></textarea></label>
                        </div>
                      {/if}
                    {/if}
                  {/if}
                  {#if mediaKind === "movie"}
                    {#if movieAssignments[file.path]}
                      {@const assignment = movieAssignments[file.path]}
                      {@const poster = assignedMoviePoster(assignment)}
                      <div class="metadata-movie-assignment">
                        <strong>Assigned: {assignment.title}{assignment.year ? ` (${assignment.year})` : ""}</strong>
                        <label>Title<input bind:value={movieAssignments[file.path].title} oninput={() => { renamePreview = null; invalidateMetadataPreview(); }} /></label>
                        <div><label>Year<input bind:value={movieAssignments[file.path].year} inputmode="numeric" oninput={() => { renamePreview = null; invalidateMetadataPreview(); }} /></label><label>Certification<input bind:value={movieAssignments[file.path].certification} oninput={() => { renamePreview = null; invalidateMetadataPreview(); }} /></label></div>
                        <details class="metadata-movie-extra"><summary>Edit descriptions, genres, rating and poster</summary>
                          <label>Short description<input bind:value={movieAssignments[file.path].shortDescription} oninput={invalidateMetadataPreview} /></label>
                          <label>Long description<textarea rows="4" bind:value={movieAssignments[file.path].plot} oninput={invalidateMetadataPreview}></textarea></label>
                          <div><label>Genres<input bind:value={movieAssignments[file.path].genresText} oninput={invalidateMetadataPreview} /><small>Separate genres with commas or semicolons.</small></label><label>Rating (0–10)<input type="number" min="0" max="10" step="0.1" bind:value={movieAssignments[file.path].ratingText} oninput={invalidateMetadataPreview} /></label></div>
                          <label>Poster option
                            <select aria-label={`Poster for ${file.name}`} disabled={artworkBusy} bind:value={movieAssignments[file.path].posterId} onchange={() => { chooseMovieProviderPoster(file.path); invalidateMetadataPreview(); }}>
                              <option value="">No poster selected</option>
                              {#each assignment.artworkCandidates as image (image.id)}<option value={image.id}>{image.provider} · {image.title}{image.license ? ` · ${image.license}` : ""}</option>{/each}
                            </select>
                          </label>
                          {#if poster}
                            <div class="metadata-assigned-poster"><img src={poster.thumbnailUrl} alt={`Selected poster: ${poster.title}`} loading="lazy" /><span><strong>{poster.title}</strong><small>{poster.attribution ?? poster.provider}{#if poster.license} · {poster.license}{/if}</small></span></div>
                            <button class="link-button" disabled={artworkBusy} onclick={() => stageArtworkSave(poster, [movieArtworkTarget(file)], `poster for ${assignment.title}`, [{ kind: "movie", mediaPath: file.path, candidateId: poster.id }])}>Save poster for this film</button>
                          {:else}<small class="metadata-assignment">No poster selected for this file. Choose an option above or continue without artwork.</small>{/if}
                        </details>
                      </div>
                    {:else}<small class="metadata-warning">Assign a reviewed movie match before previewing this file.</small>{/if}
                    <button class="link-button" disabled={!detail || renameBusy} onclick={() => assignCurrentMovie([file.path])}>{movieAssignments[file.path] ? "Replace assignment with current match" : "Assign current match to this file"}</button>
                  {/if}
                </article>
              {/each}
            </div>
            <div class="metadata-rename-actions metadata-write-actions">
              <button class="button secondary" disabled={metadataWriteBusy || !selectedPathsInOrganizer().length && mediaKind === "movie"} onclick={previewMetadataWrite}>
                {#if metadataWriteBusy}<LoaderCircle size={16} class="spin" /> Preparing preview{:else}Preview Kodi NFO sidecars{/if}
              </button>
              <small>NFO files are optional sidecars for library managers. This does not write tags into the video.</small>
            </div>
            {#if metadataWriteError}<div class="metadata-error"><TriangleAlert size={17} /><span>{metadataWriteError}</span></div>{/if}
            {#if metadataWriteMessage}<div class="metadata-success"><strong>Metadata sidecars saved</strong><span>{metadataWriteMessage}</span></div>{/if}
            {#if metadataWritePreview && metadataWriteRequest}
              <section class="metadata-artwork-save-review metadata-nfo-preview">
                <div><strong>Review Kodi NFO sidecars</strong><span>Existing standard NFO files stay untouched. Existing HEVC Rebuild sidecars will be replaced only after confirmation. To write reviewed tags into a new video file, use the metadata-only MKV action.</span></div>
                <div class="metadata-artwork-save-paths">{#each metadataWritePreview.items as item (item.targetPath)}<code>{item.sourcePath} → {item.targetPath}{#if item.replacesAppSidecar} · replace app sidecar{/if}</code>{/each}</div>
                <div class="metadata-rename-actions"><button class="button secondary" disabled={metadataWriteBusy} onclick={() => invalidateMetadataPreview()}>Cancel</button><button class="button primary" disabled={metadataWriteBusy || !metadataWritePreview.canApply} onclick={applyMetadataWrite}>{#if metadataWriteBusy}<LoaderCircle size={16} class="spin" /> Saving metadata{:else}Confirm and save metadata{/if}</button></div>
              </section>
            {/if}
            {#if !compactItemMode}
              <section class="metadata-mkv-action" aria-label="Create a metadata-only MKV copy">
                <div>
                  <span class="eyebrow">NO VIDEO ENCODING</span>
                  <h3>Create a tagged MKV copy</h3>
                  <p>Use the reviewed title, metadata and selected poster in a new MKV. Source streams are copied and checked; the original stays in place. Apply the filename changes above first if you want the new copy to use those names.</p>
                  <small>{selectedPathsInOrganizer().length} file{selectedPathsInOrganizer().length === 1 ? "" : "s"} selected in this organizer · destination and collisions are reviewed before the copy starts</small>
                </div>
                <button class="button primary" type="button" disabled={!desktop || metadataWriteBusy || renameBusy || !selectedPathsInOrganizer().length} onclick={createMetadataOnlyCopies}>
                  <FileText size={16} /> Review MKV output plan <ArrowRight size={15} />
                </button>
              </section>
            {/if}
            <div class="metadata-rename-actions">
              <button class="button secondary" disabled={renameBusy || !selectedPathsInOrganizer().length && mediaKind === "movie" || mediaKind === "movie" && selectedPathsInOrganizer().some((path) => !movieAssignments[path]) || Boolean(currentSeriesYearMismatch() && !seriesYearConflictConfirmed)} onclick={previewNames}>Preview before/after</button>
              {#if renamePreview?.canApply}<button class="button primary" disabled={renameBusy} onclick={() => confirmRename = true}>Apply reviewed rename</button>{/if}
            </div>
            {#if renameError}<div class="metadata-error"><TriangleAlert size={17} /><span>{renameError}</span></div>{/if}
            {#if renamePreview}
              <div class="metadata-rename-preview">
                <div><strong>Before → after</strong><span>{renamePreview.items.length} planned paths · {renamePreview.conflicts.length} conflicts</span></div>
                <div class="metadata-rename-table"><div><strong>Current path</strong><strong>After confirmation</strong></div>
                  {#each renamePreview.items as item (item.sourcePath)}<div class:unchanged={item.unchanged}><span>{item.sourcePath}</span><span>{item.targetPath}{#if item.unchanged}<small>Unchanged</small>{/if}</span></div>{/each}
                </div>
                {#if renamePreview.conflicts.length}<div class="metadata-conflicts"><strong>Rename blocked · resolve every duplicate or occupied destination</strong>{#each renamePreview.conflicts as conflict}<p>{conflict.reason}<small>{conflict.targetPath}</small></p>{/each}</div>
                {:else if !renamePreview.canApply}<small>Every selected name is already correct; no changes will be made.</small>{/if}
              </div>
            {/if}
            {#if confirmRename && renamePreview?.canApply}
              <div class="metadata-rename-confirm"><strong>Apply this rename batch?</strong><span>The utility will recheck the preview and collisions before changing names. Media contents remain untouched; matching NFOs, recognized local artwork and attribution, plus same-base subtitle and external audio files follow their video. An operation error triggers rollback.</span>
                <div><button class="button secondary" disabled={renameBusy} onclick={() => confirmRename = false}>Review again</button><button class="button danger" disabled={renameBusy} onclick={applyNames}>{renameBusy ? "Applying…" : "Confirm rename"}</button></div>
              </div>
            {/if}
          {/if}
        </section>
      {:else if organizerScan && mediaKind === "tv" && localSeriesNfo(organizerScan.root)?.found}
        {@const seriesScan = organizerScan!}
        {@const local = localSeriesNfo(seriesScan.root)}
        <div class="metadata-empty metadata-local-record"><Database size={25} /><strong>Local NFO data takes priority</strong><span>{local?.title ?? local?.showTitle ?? basename(seriesScan.root)}{#if local?.year} · {local.year}{/if}{#if local?.genres.length} · {local.genres.join(" · ")}{/if}</span>{#if local?.error}<small class="metadata-error-text">{local.error}</small><button class="button secondary" onclick={() => findSeriesMatches([{ root: seriesScan.root, files: seriesScan.files, seasonDirectories: seriesScan.seasonDirectories, ambiguous: false }], true)}>Search public alternative</button>{:else}<small>Review and edit the local NFO record, then save a managed sidecar if you want to preserve the original.</small><button class="button secondary" onclick={() => useLocalSeriesMetadata(seriesScan.root)}>Review local show metadata</button>{/if}{#if localDirectoryAssets(seriesScan.root)?.artwork.length}<div class="metadata-local-artwork-grid">{#each localDirectoryAssets(seriesScan.root)?.artwork.slice(0, 4) ?? [] as image (image.path)}<figure>{#if localArtworkPreviews[image.path]}<img src={localArtworkPreviews[image.path]} alt={`${image.role}: ${image.name}`} />{:else}<button class="link-button" onclick={() => loadLocalArtworkPreviews([image])}>Load preview</button>{/if}<figcaption>{image.role} · {image.name}</figcaption></figure>{/each}</div>{/if}</div>
      {:else}
        <div class="metadata-empty"><Database size={25} /><strong>Source details appear here</strong><span>Search a title and select a likely match. Nothing is applied to files automatically.</span></div>
      {/if}
    </section>
  </div>
  {#if compactItemMode && organizerScan && selectedPathsInOrganizer().length}
    <section class="metadata-focused-rename" aria-label="Rename this media file">
      <div class="metadata-focused-rename-heading">
        <span class="eyebrow">FILENAME</span>
        <strong>{visibleOrganizerFiles.find((file) => file.path === selectedPathsInOrganizer()[0])?.name ?? "Selected media"}</strong>
        <small>{compactRenameReady() ? "Preview the proposed name, then confirm the move. The video itself is not changed." : mediaKind === "movie" ? "Review or assign a film title before preparing a filename." : "Choose and review an episode before preparing a filename."}</small>
      </div>
      <button class="button secondary" type="button" disabled={!desktop || renameBusy || !compactRenameReady() || Boolean(currentSeriesYearMismatch() && !seriesYearConflictConfirmed)} onclick={previewNames}>
        {#if renameBusy}<LoaderCircle size={15} class="spin" /> Preparing rename preview{:else}Preview filename change{/if}
      </button>
      {#if renameError}<div class="metadata-error" role="alert"><TriangleAlert size={16} /><span>{renameError}</span></div>{/if}
      {#if renameMessage}<div class="metadata-success" role="status"><strong>Filename updated</strong><span>{renameMessage}</span></div>{/if}
      {#if renamePreview}
        <div class="metadata-focused-rename-preview">
          {#each renamePreview.items as item (item.sourcePath)}<div><span>Before</span><code>{item.sourcePath}</code><span>After</span><code>{item.targetPath}</code></div>{/each}
          {#if renamePreview.conflicts.length}<div class="metadata-conflicts"><strong>Rename blocked · resolve the conflicts first</strong>{#each renamePreview.conflicts as conflict}<p>{conflict.reason}<small>{conflict.targetPath}</small></p>{/each}</div>{:else if renamePreview.canApply}<p>Only the path and matching supported sidecars will move. The next step can make a separately validated, no-encode MKV copy.</p><button class="button primary" type="button" disabled={renameBusy} onclick={() => confirmRename = true}>Review rename confirmation</button>{:else}<small>This file already has the proposed name.</small>{/if}
        </div>
      {/if}
      {#if confirmRename && renamePreview?.canApply}
        <div class="metadata-rename-confirm"><strong>Rename this file?</strong><span>The app rechecks the current preview and destination before moving the file. Video contents remain unchanged; an operation error triggers rollback.</span><div><button class="button secondary" type="button" disabled={renameBusy} onclick={() => confirmRename = false}>Back to preview</button><button class="button danger" type="button" disabled={renameBusy} onclick={applyNames}>{renameBusy ? "Renaming…" : "Confirm rename"}</button></div></div>
      {/if}
    </section>
  {/if}
          <p class="metadata-boundary">Review matches and edit the fields you want to use. Renaming, poster saving, optional Kodi NFO sidecars, and MKV creation have separate previews and confirmations. A metadata-only MKV embeds supported fields and selected poster art in a new file, then validates it while keeping the source in place.</p>
</section>
