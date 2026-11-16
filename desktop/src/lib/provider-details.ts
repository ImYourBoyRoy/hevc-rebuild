// ./desktop/src/lib/provider-details.ts
// Provider detail loading extracted from MetadataWorkbench (T10.7 slice 7): opening a match (one branch per provider and kind), loading a
// series' catalog into its assignment, applying TMDB film details to a draft, and the review shortcuts that open a candidate for a film, a
// series or the open search target. The state and props are reached through a typed host and the bridge is injected so every branch is
// unit-tested in node (pnpm test:provider-details). The code is the component's code moved verbatim.

import type { Artwork, Detail, Episode, MovieAssignment, OrganizerFile, OrganizerScan, PendingArtworkSave, RenamePreview, Result, SearchTarget, Season, SeriesAssignment, TmdbCredentialStatus, Provider, MediaKind } from "./metadata-workbench-types.js";
import type { QueueTvEpisode } from "./types.js";
import { dedupeArtwork, reviewableArtwork } from "./metadata-candidates.ts";

export interface ProviderDetailsHost {
  readonly metadataLanguage: string;
  readonly tmdbCredentialStatus: TmdbCredentialStatus;
  seriesAssignments: Record<string, SeriesAssignment>;
  movieAssignments: Record<string, MovieAssignment>;
  selectedMediaPaths: string[];
  provider: Provider;
  mediaKind: MediaKind;
  selected: Result | null;
  detail: Detail | null;
  loadingDetail: boolean;
  error: string;
  reviewDetailError: string;
  searchTarget: SearchTarget | null;
  seriesYearConflictConfirmed: boolean;
  selectedArtwork: Record<string, string>;
  organizerScan: OrganizerScan | null;
  renamePreview: RenamePreview | null;
  confirmRename: boolean;
  pendingArtworkSave: PendingArtworkSave | null;
  artworkError: string;
  artworkMessage: string;
  lookupMessage: string;
  lookupMessageScope: "analyzed films" | "series folders" | null;
}

export interface ProviderDetailsActions {
  syncEpisodeAssignments(files: OrganizerFile[]): void;
  assignCurrentMovie(paths: string[]): void;
  chooseAnalyzedSeries(root: string): void;
  applyCatalogToFiles(root: string, existing: SeriesAssignment, catalog: QueueTvEpisode[]): SeriesAssignment["episodeAssignments"];
  applySeasonPublication(root: string, existing: SeriesAssignment, providerSeasons: Season[]): SeriesAssignment["seasonRecords"];
}

export interface ProviderDetailsDeps {
  invoke: <T>(command: string, args?: Record<string, unknown>) => Promise<T>;
  host: ProviderDetailsHost;
  actions: ProviderDetailsActions;
}

export function createProviderDetails(deps: ProviderDetailsDeps) {
  const { host: h, actions, invoke } = deps;
  const { syncEpisodeAssignments, assignCurrentMovie, chooseAnalyzedSeries, applyCatalogToFiles, applySeasonPublication } = actions;

  async function hydrateSeriesProvider(root: string, row: Result) {
    try {
      let episodes: Episode[] = [];
      let artwork: Artwork[] = row.artwork;
      let sourceUrl = row.sourceUrl;
      let providerSeasons: Season[] = [];
      let loadedCertification = "";
      if (row.provider === "tvdb") {
        const value = await invoke<Record<string, unknown>>("load_tvdb_details", {
          id: Number(row.id), kind: "series", language: h.metadataLanguage,
        });
        episodes = Array.isArray(value.episodes) ? value.episodes as Episode[] : [];
        providerSeasons = Array.isArray(value.seasons) ? value.seasons as typeof providerSeasons : [];
        artwork = reviewableArtwork("tvdb", row.title, value.artwork);
        sourceUrl = String(value.sourceUrl ?? row.sourceUrl);
        loadedCertification = typeof value.certification === "string" ? value.certification.trim() : "";
        if (!loadedCertification && h.tmdbCredentialStatus.configured) {
          const fallback = await invoke<string | null>("tmdb_tv_certification_for_tvdb", {
            tvdbId: Number(row.id), language: h.metadataLanguage,
          });
          loadedCertification = fallback?.trim() ?? "";
        }
      } else if (row.provider === "tmdb") {
        const value = await invoke<Record<string, unknown>>("load_tmdb_tv_details", { id: Number(row.id), language: h.metadataLanguage });
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
      const existing = h.seriesAssignments[root] ?? {
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
      h.seriesAssignments = {
        ...h.seriesAssignments,
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
      h.lookupMessage = `Could not load episode details from ${row.provider}: ${String(cause)}`;
      h.lookupMessageScope = "series folders";
      return false;
    }
  }

  async function applyTmdbMovieDetails(mediaPath: string, id: string) {
    h.reviewDetailError = "";
    const value = await invoke<Record<string, unknown>>("load_tmdb_movie_details", { id: Number(id), language: h.metadataLanguage });
    const movie = (value.movie ?? {}) as Record<string, unknown>;
    const genres = Array.isArray(value.genres) ? value.genres.filter((item): item is string => typeof item === "string") : [];
    const rating = typeof movie.rating === "number" ? movie.rating : null;
    const title = typeof movie.title === "string" ? movie.title : "";
    const artwork = dedupeArtwork(reviewableArtwork("tmdb", title, Array.isArray(value.artwork) && value.artwork.length ? value.artwork : movie.artwork));
    const existing = h.movieAssignments[mediaPath];
    if (!existing) return;
    const tagline = typeof value.tagline === "string" ? value.tagline.trim() : "";
    const overview = typeof movie.overview === "string" ? movie.overview : "";
    h.movieAssignments = {
      ...h.movieAssignments,
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

  async function openMatch(row: Result) {
    h.provider = row.provider;
    h.mediaKind = row.kind;
    h.selected = row;
    h.detail = null;
    h.seriesYearConflictConfirmed = false;
    h.renamePreview = null;
    h.confirmRename = false;
    h.pendingArtworkSave = null;
    h.artworkError = "";
    h.artworkMessage = "";
    h.loadingDetail = true;
    h.error = "";
    try {
      if (row.provider === "tmdb" && row.kind === "movie") {
        const value = await invoke<Record<string, unknown>>("load_tmdb_movie_details", { id: Number(row.id), language: h.metadataLanguage });
        const movie = value.movie as Record<string, unknown>;
        h.detail = {
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
        const value = await invoke<Record<string, unknown>>("load_tmdb_tv_details", { id: Number(row.id), language: h.metadataLanguage });
        const show = value.show as Record<string, unknown>;
        h.detail = {
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
          id: Number(row.id), kind: row.kind === "movie" ? "movie" : "series", language: h.metadataLanguage,
        });
        h.detail = {
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
        const value = await invoke<Record<string, unknown>>("load_public_film_details", { entityId: row.id, language: h.metadataLanguage });
        h.detail = {
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
        h.detail = {
          title: String(show.title ?? row.title), originalTitle: null,
          year: [show.yearStart, show.yearEnd].filter((year) => typeof year === "string").join("–") || row.year,
          sourceUrl: String(show.sourceUrl ?? row.sourceUrl),
          shortDescription: "", longDescription: typeof show.summary === "string" ? show.summary : (row.description ?? ""),
          certification: null, runtime: null, genres: [],
          rating: typeof show.audienceRating === "number" ? show.audienceRating : row.rating,
          artwork: reviewableArtwork("tvmaze", row.title, show.artwork),
          seasons: Array.isArray(value.seasons) ? value.seasons as Season[] : [],
          episodes: Array.isArray(value.episodes) ? value.episodes as Episode[] : [],
          attribution: `TVmaze program data, as published by TVmaze. Requested metadata language: ${h.metadataLanguage}; TVmaze may return fields in their submitted language. Verify each image's source and license.`,
        };
      }
    } catch (cause) {
      h.error = String(cause);
    } finally {
      h.loadingDetail = false;
    }
    h.selectedArtwork = {};
    h.pendingArtworkSave = null;
    h.renamePreview = null;
    if (h.mediaKind === "tv" && h.organizerScan) syncEpisodeAssignments(h.organizerScan.files);
  }

  async function reviewMovieCandidate(path: string, row: Result, assignAfterReview = true) {
    h.selectedMediaPaths = [...new Set([...h.selectedMediaPaths, path])];
    await openMatch(row);
    if (h.detail && assignAfterReview) assignCurrentMovie([path]);
  }

  async function reviewSeriesCandidate(root: string, row: Result) {
    chooseAnalyzedSeries(root);
    await openMatch(row);
  }

  async function reviewTargetCandidate(row: Result) {
    const target = h.searchTarget;
    if (!target) return;
    if (target.kind === "movie") await reviewMovieCandidate(target.path, row);
    else await reviewSeriesCandidate(target.root, row);
  }

  return { hydrateSeriesProvider, applyTmdbMovieDetails, openMatch, reviewMovieCandidate, reviewSeriesCandidate, reviewTargetCandidate };
}
