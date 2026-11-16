// ./desktop/src/lib/state/workbench-effects.svelte.ts
// The metadata workbench's `$effect` blocks, moved out of the component: following the focused file (series or film, details,
// match lookup), the automatic lookup after a scan, keeping the series review in step with the open match, focusing the title
// search, scrolling to the focused row and loading the selected and discovered poster previews. They are installed by one call
// from the component script, so they are created in the component's context in the same order as before. Props, derived
// values, state instances and the component functions they use arrive in one context object.

import { WorkbenchDerived } from "./workbench-derived.svelte.js";
import type { WorkbenchPropsView } from "./workbench-props.js";
import type { WorkbenchSearchState } from "./workbench-search-state.svelte.js";
import type { WorkbenchLookupState } from "./workbench-lookup-state.svelte.js";
import type { WorkbenchOrganizerState } from "./workbench-organizer-state.svelte.js";
import type { WorkbenchLocalAssetsState } from "./workbench-local-assets-state.svelte.js";
import type { WorkbenchRenameState } from "./workbench-rename-state.svelte.js";
import type { QueueMetadataSuggestion, ScanResult } from "../types.js";
import type { AnalysisSeriesGroup, LocalAssetsScan, LocalMediaAssets, MediaKind, OrganizerFile, SeriesAssignment } from "../metadata-workbench-types.js";
import { analysisOrganizerScan, analyzedLibraryRoot, basename, buildAnalysisSeriesGroups, inferredSeriesRoot, normalizedPath, organizerFileFromEntry } from "../organizer-analysis.js";
import type { LocalArtworkFile, Provider, Result } from "../metadata-workbench-types.js";
import { preferredLocalPoster } from "../library-browse.js";
import { metadataScanKey } from "../metadata-candidates.js";

export interface WorkbenchEffectsContext {
  props: WorkbenchPropsView;
  derived: WorkbenchDerived;
  searchState: WorkbenchSearchState;
  lookupState: WorkbenchLookupState;
  organizerState: WorkbenchOrganizerState;
  localAssetsState: WorkbenchLocalAssetsState;
  renameState: WorkbenchRenameState;
  applyTmdbMovieDetails: (mediaPath: string, id: string) => Promise<void>;
  changeMediaKind: (kind: MediaKind) => void;
  chooseAnalyzedSeries: (root: string) => void;
  ensureLocalAssets: (root: string, files: OrganizerFile[], directories?: string[]) => Promise<LocalAssetsScan | null>;
  findMovieMatches: (files: OrganizerFile[], analysisRoot?: string) => Promise<void>;
  findSeriesMatches: (groups: AnalysisSeriesGroup[], analysisRoot?: string) => Promise<void>;
  loadLocalArtworkPreviews: (images: LocalArtworkFile[]) => Promise<void>;
  localFileAssets: (path: string) => LocalMediaAssets | undefined;
  makeCurrentSeriesReview: () => SeriesAssignment | null;
  openMatch: (row: Result) => Promise<void>;
  resumeSeriesArtwork: (scan: ScanResult) => void;
  seedMovieDrafts: (files: OrganizerFile[]) => Promise<void>;
  suggestionForPath: (path: string) => QueueMetadataSuggestion | undefined;
  useAnalyzedLibrary: (kind: MediaKind, seriesRoot?: string) => void;
}

export function installWorkbenchEffects(ctx: WorkbenchEffectsContext) {
  const { props: p, derived, searchState, lookupState, organizerState, localAssetsState, renameState, applyTmdbMovieDetails, changeMediaKind, chooseAnalyzedSeries, ensureLocalAssets, findMovieMatches, findSeriesMatches, loadLocalArtworkPreviews, localFileAssets, makeCurrentSeriesReview, openMatch, resumeSeriesArtwork, seedMovieDrafts, suggestionForPath, useAnalyzedLibrary } = ctx;
  let lastAppliedAnalysis: ScanResult | null = null;
  let lastAppliedKind: MediaKind | null = null;
  let lastAutoLookupKey: string | null = null;
  let lastAutoDetailKey: string | null = null;
  let lastFocusedMediaPath: string | null = null;
  let lastFocusedLookupPath: string | null = null;

  $effect(() => {
    const targetPath = p.focusMediaPath;
    const currentScan = p.scanResult;
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
      p.selectedMediaPaths = [...new Set([...p.selectedMediaPaths, targetPath])];
    }
  });

  $effect(() => {
    const targetPath = p.focusMediaPath;
    if (!p.compactItemMode || !targetPath || !p.movieAssignments[targetPath] || lookupState.movieEditorOpen[targetPath]) return;
    lookupState.movieEditorOpen = { ...lookupState.movieEditorOpen, [targetPath]: true };
  });

  $effect(() => {
    const targetPath = p.focusMediaPath;
    const suggestion = targetPath ? suggestionForPath(targetPath) : undefined;
    if (!p.compactItemMode || !p.workbenchActive || !targetPath || suggestion?.status !== "ready"
      || !suggestion.provider || !suggestion.recordId) return;
    const key = `${normalizedPath(targetPath)}\0${suggestion.recordId}\0details`;
    if (lastAutoDetailKey === key) return;
    const [providerId, ...recordIdParts] = suggestion.recordId.split(":");
    if (providerId !== suggestion.provider || !recordIdParts.length) return;
    const providerKey = providerId as Provider;
    if (!["tmdb", "tvdb", "tvmaze", "wikidata"].includes(providerKey)) return;
    const currentScan = p.scanResult;
    const entry = currentScan?.entries.find((item) => normalizedPath(item.path) === normalizedPath(targetPath));
    if (!currentScan || !entry) return;
    const seriesRoot = suggestion.kind === "tv" ? inferredSeriesRoot(currentScan, entry) : "";
    const currentAssignment = suggestion.kind === "movie" ? p.movieAssignments[targetPath] : p.seriesAssignments[seriesRoot];
    if (!currentAssignment) return;
    const linkedElsewhere = Boolean(currentAssignment.recordId)
      && !currentAssignment.recordId.startsWith("local")
      && currentAssignment.recordId !== suggestion.recordId;
    if (linkedElsewhere) return;
    lastAutoDetailKey = key;
    if (suggestion.kind === "movie" && (!currentAssignment.recordId || currentAssignment.recordId.startsWith("local"))) {
      const art = suggestion.artwork;
      p.movieAssignments = {
        ...p.movieAssignments,
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
      if (p.focusMediaPath !== targetPath || !p.workbenchActive) return;
      if (suggestion.kind === "movie" && providerKey === "tmdb") {
        try {
          await applyTmdbMovieDetails(targetPath, row.id);
        } catch (cause) {
          searchState.reviewDetailError = String(cause);
        }
        return;
      }
      await openMatch(row);
      if (p.focusMediaPath !== targetPath || !p.workbenchActive) return;
      const loaded = searchState.detail;
      if (!loaded || searchState.selected?.id !== row.id || searchState.selected.provider !== row.provider) return;
      if (suggestion.kind === "movie") {
        const existing = p.movieAssignments[targetPath];
        if (!existing) return;
        if (existing.recordId && !existing.recordId.startsWith("local") && existing.recordId !== suggestion.recordId) return;
        const artwork = loaded.artwork.length ? loaded.artwork : existing.artworkCandidates;
        const suggestionText = suggestion.description ?? "";
        const plotUntouched = !existing.plot.trim() || existing.plot === suggestionText;
        const summaryUntouched = !existing.shortDescription.trim() || existing.shortDescription === suggestionText;
        p.movieAssignments = {
          ...p.movieAssignments,
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
        organizerState.selectedArtwork = { ...organizerState.selectedArtwork, movie: p.movieAssignments[targetPath].posterId };
      } else {
        const existing = p.seriesAssignments[seriesRoot];
        if (!existing || existing.recordId !== suggestion.recordId) return;
        const artwork = loaded.artwork.length ? loaded.artwork : existing.artworkCandidates;
        const suggestionText = suggestion.description ?? "";
        p.seriesAssignments = {
          ...p.seriesAssignments,
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
        organizerState.selectedArtwork = { ...organizerState.selectedArtwork, series: p.seriesAssignments[seriesRoot].posterId };
      }
    });
  });

  $effect(() => {
    const targetPath = p.focusMediaPath;
    const currentScan = p.scanResult;
    const active = p.workbenchActive;
    const busy = lookupState.lookupBusy;
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
      if (p.seriesAssignments[group.root] || lookupState.seriesSuggestions[group.root]) return;
      lastFocusedLookupPath = targetPath;
      queueMicrotask(() => {
        if (p.workbenchActive && p.focusMediaPath === targetPath && !lookupState.lookupBusy) {
          void findSeriesMatches([group], analyzedLibraryRoot(currentScan));
        }
      });
    } else {
      if (p.movieAssignments[targetPath] || lookupState.movieSuggestions[targetPath]) return;
      lastFocusedLookupPath = targetPath;
      queueMicrotask(() => {
        if (p.workbenchActive && p.focusMediaPath === targetPath && !lookupState.lookupBusy) {
          void findMovieMatches([file], analyzedLibraryRoot(currentScan));
        }
      });
    }
  });

  $effect(() => {
    if (searchState.mediaKind !== "tv" || !searchState.selected || !searchState.detail || renameState.renameBusy) return;
    const root = searchState.searchTarget?.kind === "tv"
      ? searchState.searchTarget.root
      : organizerState.organizerScan?.root || organizerState.selectedAnalysisSeriesRoot;
    if (!root) return;
    const review = makeCurrentSeriesReview();
    if (!review) return;
    const previous = p.seriesAssignments[root];
    if (JSON.stringify(previous) !== JSON.stringify(review)) {
      p.seriesAssignments = { ...p.seriesAssignments, [root]: review };
    }
  });

  $effect(() => {
    if (!searchState.searchTarget) return;
    requestAnimationFrame(() => {
      searchState.titleSearchInput?.scrollIntoView({ behavior: "smooth", block: "center" });
      searchState.titleSearchInput?.focus();
    });
  });

  $effect(() => {
    const targetPath = p.focusMediaPath;
    if (!targetPath || organizerState.organizerSource !== "analysis" || !organizerState.organizerScan?.files.some((file) => file.path === targetPath)) return;
    queueMicrotask(() => {
      if (p.focusMediaPath !== targetPath) return;
      const row = [...document.querySelectorAll<HTMLElement>(".metadata-analysis-movie-row[data-media-path]")]
        .find((element) => element.dataset.mediaPath === targetPath);
      if (!row) return;
      row.scrollIntoView({ behavior: "smooth", block: "center" });
      row.classList.add("metadata-focus-target");
      window.setTimeout(() => row.classList.remove("metadata-focus-target"), 1800);
    });
  });

  $effect(() => {
    const root = localAssetsState.localAssets?.root;
    if (!derived.desktop || !root) return;
    const selectedPaths = [
      ...Object.values(p.movieAssignments).map((assignment) => assignment.localPosterPath),
      ...Object.values(p.seriesAssignments).flatMap((assignment) => [
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
    const files = derived.visibleOrganizerFiles;
    if (!derived.desktop || searchState.mediaKind !== "movie" || organizerState.organizerSource !== "analysis" || !localAssetsState.localAssets || !files.length) return;
    const images = files
      .map((file) => {
        const artwork = localFileAssets(file.path)?.artwork ?? [];
        const path = preferredLocalPoster(artwork, null);
        return path ? artwork.find((image) => image.path === path) : undefined;
      })
      .filter((image): image is LocalArtworkFile => Boolean(image));
    for (const image of images.slice(0, 24)) void loadLocalArtworkPreviews([image]);
  });

  $effect(() => {
    const currentScan = p.scanResult;
    const currentKind = searchState.mediaKind;
    if (!currentScan) {
      lastAppliedAnalysis = null;
      lastAutoLookupKey = null;
      lookupState.movieSuggestions = {};
      lookupState.seriesSuggestions = {};
      searchState.expandedMatchLists = {};
      lookupState.movieEditorOpen = {};
      return;
    }
    if (organizerState.organizerSource === "folder") return;
    const shouldPrepareAnalysis = p.workbenchActive;
    const analysisChanged = lastAppliedAnalysis !== currentScan || lastAppliedKind !== currentKind;
    if (shouldPrepareAnalysis && analysisChanged) {
      if (lastAppliedAnalysis !== currentScan) {
        const lookupAlreadyRan = lastAutoLookupKey === metadataScanKey(currentScan);
        if (!lookupAlreadyRan) {
          lookupState.movieSuggestions = {};
          lookupState.seriesSuggestions = {};
        }
        searchState.expandedMatchLists = {};
        lookupState.movieEditorOpen = {};
      }
      lastAppliedAnalysis = currentScan;
      lastAppliedKind = currentKind;
      useAnalyzedLibrary(currentKind);
      queueMicrotask(async () => {
        if (p.scanResult !== currentScan || organizerState.organizerSource === "folder") return;
        const groups = buildAnalysisSeriesGroups(currentScan);
        const files = currentScan.entries.map(organizerFileFromEntry);
        const directories = [
          analyzedLibraryRoot(currentScan),
          ...groups.flatMap((group) => [group.root, ...group.seasonDirectories.map((season) => season.path)]),
        ];
        await ensureLocalAssets(analyzedLibraryRoot(currentScan), files, directories);
        if (currentKind === "movie" && p.scanResult === currentScan) {
          await seedMovieDrafts(analysisOrganizerScan(currentScan, "movie")?.files ?? []);
        }
      });
    }
    if (p.autoLookupPending && !p.metadataAutoLookup) {
      p.onAutoLookupConsumed();
    }
    if (!p.autoLookupPending) resumeSeriesArtwork(currentScan);
    const currentLookupKey = metadataScanKey(currentScan);
    const certificateMissing = Object.values(p.seriesAssignments).some((review) => {
      const record = review.recordId ?? "";
      return Boolean(record) && !record.startsWith("local") && !review.certification.trim();
    });
    if (p.metadataAutoLookup && derived.desktop && p.autoLookupPending && currentLookupKey === lastAutoLookupKey && !certificateMissing) {
      p.onAutoLookupConsumed();
    } else if (p.metadataAutoLookup && derived.desktop && p.autoLookupPending) {
      // Keep the pending request until the current provider batch releases the
      // shared lookup lock; switching queue sources must not consume it.
      if (lookupState.lookupBusy) return;
      if (!p.onClaimAutoLookup()) return;
      lastAutoLookupKey = currentLookupKey;
      lookupState.movieSuggestions = {};
      lookupState.seriesSuggestions = {};
      p.onAutoLookupConsumed();
      queueMicrotask(async () => {
        if (!p.metadataAutoLookup || p.scanResult !== currentScan) return;
        await findMovieMatches(analysisOrganizerScan(currentScan, "movie")?.files ?? [], analyzedLibraryRoot(currentScan));
        if (p.metadataAutoLookup && p.scanResult === currentScan && !lookupState.lastLookupStopped) {
          await findSeriesMatches(buildAnalysisSeriesGroups(currentScan), analyzedLibraryRoot(currentScan));
        }
      });
    }
  });
}
