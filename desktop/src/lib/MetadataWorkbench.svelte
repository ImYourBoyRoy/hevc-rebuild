<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { ArrowRight, Database, Film, LoaderCircle, Search, Tv, TriangleAlert, X } from "@lucide/svelte";
  import "../styles/components/metadata-review.css";
  import type { MetadataLookupStatus, QueueMetadataSuggestion, ScanResult } from "#lib/types.js";
  import type { MetadataSources, TmdbCredentialStatus, TvdbCredentialStatus, Artwork, Season, OrganizerFile, MovieAssignment, SeriesAssignment, OrganizerTemplates } from "#lib/metadata-workbench-types.js";
  import { basename } from "#lib/organizer-analysis.js";
  import { candidateEvidence, candidatePreview, clueCaption, providerLabel, suggestionResult } from "#lib/metadata-candidates.js";
  import { WorkbenchSearchState } from "#lib/state/workbench-search-state.svelte.js";
  import { WorkbenchLookupState } from "#lib/state/workbench-lookup-state.svelte.js";
  import { WorkbenchOrganizerState } from "#lib/state/workbench-organizer-state.svelte.js";
  import { WorkbenchLocalAssetsState } from "#lib/state/workbench-local-assets-state.svelte.js";
  import { WorkbenchRenameState } from "#lib/state/workbench-rename-state.svelte.js";
  import { WorkbenchArtworkSaveState } from "#lib/state/workbench-artwork-save-state.svelte.js";
  import { createLocalAssets } from "#lib/local-assets.js";
  import { createProviderSearch, type ProviderSearchHost } from "#lib/provider-search.js";
  import { createSearchPanel, initialCandidateCount } from "#lib/search-panel.js";
  import { tmdbLogo } from "#lib/workbench-constants.js";
  import WorkbenchDetailPanel from "#lib/views/WorkbenchDetailPanel.svelte";
  import WorkbenchMoviesPanel from "#lib/views/WorkbenchMoviesPanel.svelte";
  import { createMatchAssignments } from "#lib/match-assignments.js";
  import { createProviderDetails } from "#lib/provider-details.js";
  import { createMatchLookup } from "#lib/match-lookup.js";
  import { createOrganizerRename } from "#lib/organizer-rename.js";
  import { createOrganizerSession } from "#lib/organizer-session.js";
  import { createPosterSelection } from "#lib/poster-selection.js";
  import { createArtworkSave } from "#lib/artwork-save.js";
  import { composeHost } from "#lib/state/bind-host.js";
  import { WorkbenchDerived } from "#lib/state/workbench-derived.svelte.js";
  import { installWorkbenchEffects } from "#lib/state/workbench-effects.svelte.js";
  import type { WorkbenchPropsView } from "#lib/state/workbench-props.js";

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

  const searchState = new WorkbenchSearchState();
  const lookupState = new WorkbenchLookupState();
  const organizerState = new WorkbenchOrganizerState();
  const localAssetsState = new WorkbenchLocalAssetsState();
  const renameState = new WorkbenchRenameState();
  const artworkSaveState = new WorkbenchArtworkSaveState();
  const propsView: WorkbenchPropsView = {
    get scanResult() { return scanResult; }, get focusMediaPath() { return focusMediaPath; }, get compactItemMode() { return compactItemMode; }, get workbenchActive() { return workbenchActive; },
    get autoLookupPending() { return autoLookupPending; }, get metadataAutoLookup() { return metadataAutoLookup; },
    get selectedMediaPaths() { return selectedMediaPaths; }, set selectedMediaPaths(value) { selectedMediaPaths = value; },
    get movieAssignments() { return movieAssignments; }, set movieAssignments(value) { movieAssignments = value; },
    get seriesAssignments() { return seriesAssignments; }, set seriesAssignments(value) { seriesAssignments = value; },
    onAutoLookupConsumed: () => onAutoLookupConsumed(), onClaimAutoLookup: () => onClaimAutoLookup(),
  };
  const derived = new WorkbenchDerived(propsView, organizerState);
  const { ensureLocalAssets, localFileAssets, localDirectoryAssets, previewLocalArtwork, loadLocalArtworkPreviews } = createLocalAssets({ invoke, isDesktop: () => derived.desktop, host: localAssetsState });
  const providerSearchHost: ProviderSearchHost = {
    get metadataSources() { return metadataSources; }, get tmdbCredentialStatus() { return tmdbCredentialStatus; }, get tvdbCredentialStatus() { return tvdbCredentialStatus; },
    get metadataLanguage() { return metadataLanguage; }, get textBlacklistPatterns() { return textBlacklistPatterns; },
  };
  const { filenameClue, searchPublicTitles } = createProviderSearch({ invoke, isDesktop: () => derived.desktop, host: providerSearchHost });
  const searchPanelHost = composeHost(searchState, lookupState, organizerState, renameState, artworkSaveState, {
    get selectedMediaPaths() { return selectedMediaPaths; }, set selectedMediaPaths(value) { selectedMediaPaths = value; },
  });
  const {
    prepareSearchTarget, searchMovieRecord, searchSeriesRecord, searchCurrentSeriesRecord, closeTitleSearch, searchTitles, currentSeriesYearMismatch, visibleCandidates,
    toggleCandidateList, candidateListExpanded, rankByFilenameYear, currentReviewedSeriesRoot,
  } = createSearchPanel({ host: searchPanelHost, actions: { chooseAnalyzedSeries: (root: string) => chooseAnalyzedSeries(root), filenameClue, searchPublicTitles }, isDesktop: () => derived.desktop });
  const matchAssignmentsHost = composeHost(organizerState, localAssetsState, lookupState, {
    get movieAssignments() { return movieAssignments; }, set movieAssignments(value) { movieAssignments = value; },
    get seriesAssignments() { return seriesAssignments; }, set seriesAssignments(value) { seriesAssignments = value; },
    get queueSuggestions() { return queueSuggestions; }, get scanResult() { return scanResult; }, get textBlacklistPatterns() { return textBlacklistPatterns; },
  });
  const {
    suggestionForPath, savedMovieMatchSet, savedSeriesMatchSet, movieNeedsSearch, movieSearchCount, movieLookupLabel, reviewPosterOptions, autoAssignMovieMatch, autoAssignSeriesMatch,
    seedSeriesAssignment, seedMovieDrafts, applyCatalogToFiles, applySeasonPublication, seriesGroupForHydrate,
  } = createMatchAssignments({ invoke, isDesktop: () => derived.desktop, host: matchAssignmentsHost, actions: { ensureLocalAssets } });
  const providerDetailsHost = composeHost(searchState, lookupState, organizerState, renameState, artworkSaveState, {
    get metadataLanguage() { return metadataLanguage; }, get tmdbCredentialStatus() { return tmdbCredentialStatus; },
    get seriesAssignments() { return seriesAssignments; }, set seriesAssignments(value) { seriesAssignments = value; },
    get movieAssignments() { return movieAssignments; }, set movieAssignments(value) { movieAssignments = value; },
    get selectedMediaPaths() { return selectedMediaPaths; }, set selectedMediaPaths(value) { selectedMediaPaths = value; },
  });
  const { hydrateSeriesProvider, applyTmdbMovieDetails, openMatch, reviewMovieCandidate, reviewSeriesCandidate, reviewTargetCandidate } = createProviderDetails({
    invoke, host: providerDetailsHost,
    actions: {
      syncEpisodeAssignments: (files: OrganizerFile[]) => syncEpisodeAssignments(files), assignCurrentMovie: (paths: string[]) => assignCurrentMovie(paths),
      chooseAnalyzedSeries: (root: string) => chooseAnalyzedSeries(root), applyCatalogToFiles, applySeasonPublication,
    },
  });
  const matchLookupHost = composeHost(organizerState, localAssetsState, lookupState, {
    get scanResult() { return scanResult; }, get metadataSources() { return metadataSources; }, get tvdbCredentialStatus() { return tvdbCredentialStatus; },
    get movieAssignments() { return movieAssignments; }, get seriesAssignments() { return seriesAssignments; },
  });
  const { findMovieMatches, findSeriesMatches, stopFilenameLookup } = createMatchLookup({
    isDesktop: () => derived.desktop, host: matchLookupHost,
    actions: {
      ensureLocalAssets, filenameClue, searchPublicTitles, rankByFilenameYear, autoAssignMovieMatch, autoAssignSeriesMatch, hydrateSeriesProvider, suggestionForPath,
      onLookupStatus: (status: MetadataLookupStatus) => onLookupStatus(status), onMatchSuggestion: (mediaPath: string, suggestion: QueueMetadataSuggestion) => onMatchSuggestion(mediaPath, suggestion),
    },
  });
  const organizerRenameHost = composeHost(searchState, organizerState, renameState, {
    get movieAssignments() { return movieAssignments; }, set movieAssignments(value) { movieAssignments = value; },
    get selectedMediaPaths() { return selectedMediaPaths; }, set selectedMediaPaths(value) { selectedMediaPaths = value; },
    get organizerTemplates() { return organizerTemplates; }, get textBlacklistPatterns() { return textBlacklistPatterns; },
    get compactItemMode() { return compactItemMode; }, get visibleOrganizerFiles() { return derived.visibleOrganizerFiles; },
  });
  const {
    buildRenameRequest, previewNames, applyNames, compactRenameReady, selectedPathsInOrganizer, episodeForKey, updateEpisode, toggleMedia, selectAllMedia, showYears,
    saveOrganizerTemplates, createMetadataOnlyCopies, syncEpisodeAssignments, assignCurrentMovie,
  } = createOrganizerRename({
    invoke, isDesktop: () => derived.desktop, host: organizerRenameHost,
    actions: {
      currentSeriesYearMismatch,
      onRenameState: (mediaPaths: string[], status: "renaming" | "renamed" | "failed", message?: string) => onRenameState(mediaPaths, status, message),
      onPathsRenamed: (paths: Record<string, string>) => onPathsRenamed(paths),
      onOrganizerTemplatesChange: (templates: OrganizerTemplates) => onOrganizerTemplatesChange(templates),
      onStartMetadataOnly: (mediaPaths: string[]) => onStartMetadataOnly(mediaPaths),
    },
  });
  const organizerSessionHost = composeHost(searchState, organizerState, renameState, artworkSaveState, localAssetsState, lookupState, {
    get scanResult() { return scanResult; }, get metadataAutoLookup() { return metadataAutoLookup; },
    get seriesAssignments() { return seriesAssignments; }, set seriesAssignments(value) { seriesAssignments = value; },
    get selectedMediaPaths() { return selectedMediaPaths; }, set selectedMediaPaths(value) { selectedMediaPaths = value; },
  });
  const {
    useAnalyzedLibrary, restoreSavedSeriesReview, makeCurrentSeriesReview, chooseAnalyzedSeries, resumeSeriesArtwork, chooseOrganizerFolder, scanCurrentSeriesFolder, changeMediaKind,
  } = createOrganizerSession({
    invoke, isDesktop: () => derived.desktop, host: organizerSessionHost,
    actions: {
      openDialog: (options) => openDialog(options), seedMovieDrafts, syncEpisodeAssignments, filenameClue, episodeForKey, ensureLocalAssets, findMovieMatches, findSeriesMatches,
      suggestionForPath, seedSeriesAssignment, hydrateSeriesProvider,
    },
  });
  const posterSelectionHost = composeHost(organizerState, artworkSaveState, {
    get seriesAssignments() { return seriesAssignments; }, set seriesAssignments(value) { seriesAssignments = value; },
    get movieAssignments() { return movieAssignments; }, set movieAssignments(value) { movieAssignments = value; },
  });
  const {
    updateCurrentSeriesArtwork, chooseSeriesLocalPoster, chooseSeriesLocalSeasonPoster, chooseSeriesProviderPoster, chooseSeriesProviderSeasonPoster, chooseMovieLocalPoster,
    batchLocalPosterTargets, previewBatchLocalPoster, applyBatchLocalPoster, selectMovieArtwork, chooseMovieProviderPoster,
  } = createPosterSelection({ host: posterSelectionHost, actions: { currentReviewedSeriesRoot, makeCurrentSeriesReview, selectedPathsInOrganizer } });
  const artworkSaveHost = composeHost(searchState, organizerState, artworkSaveState, {
    get savePosterBesideMedia() { return savePosterBesideMedia; },
    get movieAssignments() { return movieAssignments; }, set movieAssignments(value) { movieAssignments = value; },
    get seriesAssignments() { return seriesAssignments; }, set seriesAssignments(value) { seriesAssignments = value; },
  });
  const {
    stageArtworkSave, plannedArtworkPaths, applyArtworkSave, chooseLocalPoster, saveLocalPoster, movieArtworkTargets, movieArtworkBindings, assignSelectedPosterToMatchingCopies,
    stageCurrentFilmPoster, selectedSeasonArtwork, selectedMovieArtwork, selectedSeriesArtwork,
  } = createArtworkSave({
    invoke, isDesktop: () => derived.desktop, host: artworkSaveHost,
    actions: {
      openDialog: (options) => openDialog(options), selectedPathsInOrganizer: () => selectedPathsInOrganizer(),
      chooseMovieLocalPoster: (mediaPath: string, artworkPath: string) => chooseMovieLocalPoster(mediaPath, artworkPath), loadLocalArtworkPreviews,
    },
  });

  installWorkbenchEffects({ props: propsView, derived, searchState, lookupState, organizerState, localAssetsState, renameState, applyTmdbMovieDetails, changeMediaKind, chooseAnalyzedSeries, ensureLocalAssets, findMovieMatches, findSeriesMatches, loadLocalArtworkPreviews, localFileAssets, makeCurrentSeriesReview, openMatch, resumeSeriesArtwork, seedMovieDrafts, suggestionForPath, useAnalyzedLibrary });



</script>

<section class="metadata-page" class:metadata-focused-review={compactItemMode} class:metadata-focused-review-tv={compactItemMode && searchState.mediaKind === "tv"}>
  {#if !compactItemMode}<div class="page-heading">
    <div>
      <div class="eyebrow">LIBRARY ORGANIZATION</div>
      <h1>Identify & organize</h1>
      <p>Review titles, artwork and episode details alongside the movie or series they belong to.</p>
    </div>
    <div class="metadata-workbench-preferences"><span>{metadataAutoLookup ? "Suggestions run after a scan" : "Automatic suggestions are off"} · {metadataLanguage}</span><button class="link-button" onclick={() => onOpenSettings()}>Metadata settings <ArrowRight size={14} /></button></div>
  </div>{/if}

  {#if !compactItemMode}<div class="metadata-kind-switch" role="group" aria-label="Media type to organize">
    <button class:active={searchState.mediaKind === "movie"} aria-pressed={searchState.mediaKind === "movie"} onclick={() => changeMediaKind("movie")}><Film size={16} /> Movies</button>
    <button class:active={searchState.mediaKind === "tv"} aria-pressed={searchState.mediaKind === "tv"} onclick={() => changeMediaKind("tv")}><Tv size={16} /> TV series</button>
  </div>{/if}

  {#if scanResult && organizerState.organizerSource === "analysis" && !compactItemMode}
    <section class="metadata-analysis-source card-surface" aria-label="Analyzed media association">
      <div><div class="eyebrow">CURRENT MEDIA ANALYSIS</div><strong>{compactItemMode ? "1 selected media file" : `${scanResult.entries.length} scanned file${scanResult.entries.length === 1 ? "" : "s"}`}</strong><small>{scanResult.root}</small></div>
      {#if searchState.mediaKind === "tv"}
        <div class="metadata-analysis-series">
          <div class="metadata-analysis-movies-heading"><div><div class="eyebrow">DETECTED SERIES</div><strong>{derived.visibleAnalysisSeriesGroups.length} series folder{derived.visibleAnalysisSeriesGroups.length === 1 ? "" : "s"}</strong></div><div class="metadata-movie-lookup-actions"><small>Names and year clues come from folders and episode filenames</small><button class="button primary" disabled={!derived.desktop || lookupState.lookupBusy || !derived.visibleAnalysisSeriesGroups.some((group) => !group.ambiguous)} onclick={() => findSeriesMatches(derived.visibleAnalysisSeriesGroups)}>{lookupState.lookupBusy && lookupState.lookupProgress?.label === "series folders" ? "Searching sources…" : `Search ${derived.visibleAnalysisSeriesGroups.filter((group) => !group.ambiguous).length} series`}</button>{#if lookupState.lookupBusy}<button class="link-button" onclick={stopFilenameLookup}>Stop after current lookup</button>{/if}</div></div>
          {#if lookupState.lookupProgress?.label === "series folders"}<div class="metadata-lookup-progress" role="status">Searching {lookupState.lookupProgress.done} of {lookupState.lookupProgress.total} series folders. Duplicate title queries are reused.</div>{/if}
          {#if lookupState.lookupMessage && lookupState.lookupMessageScope === "series folders"}<div class="metadata-lookup-progress" role="status">{lookupState.lookupMessage}</div>{/if}
          {#if derived.visibleAnalysisSeriesGroups.length}
            <div class="metadata-analysis-series-list">
              {#each derived.visibleAnalysisSeriesGroups as group (group.root)}
                {@const localSeries = localDirectoryAssets(group.root)}
                {@const savedSeriesSuggestion = group.files.map((file) => suggestionForPath(file.path)).find((item) => item?.kind === "tv")}
                {@const savedSeriesResult = suggestionResult(savedSeriesSuggestion, "tv")}
                {@const seriesMatchSet = lookupState.seriesSuggestions[group.root] ?? savedSeriesMatchSet(group)}
                <article class="metadata-analysis-series-row">
                  <div class="metadata-analysis-series-main"><div><strong>{basename(group.root)}</strong><small>{group.files.length} scanned episode file{group.files.length === 1 ? "" : "s"} · {group.seasonDirectories.length} season folder{group.seasonDirectories.length === 1 ? "" : "s"}</small></div><div class="metadata-record-actions"><button class="button secondary" disabled={!derived.desktop || lookupState.lookupBusy || group.ambiguous} onclick={() => savedSeriesResult && savedSeriesSuggestion?.status === "ready" ? reviewSeriesCandidate(group.root, savedSeriesResult) : findSeriesMatches([group])}>{seriesMatchSet?.status === "searching" ? "Searching…" : savedSeriesSuggestion?.status === "ready" ? "Review suggestion" : "Find matches"}</button><button class="link-button" disabled={!derived.desktop || group.ambiguous} onclick={() => searchSeriesRecord(group)}><Search size={14} /> Search another title</button></div></div>
                  {#if localSeries?.artwork.length}
                    <details class="metadata-local-artwork" ontoggle={(event) => event.currentTarget.open && loadLocalArtworkPreviews(localSeries.artwork)}><summary>Local show artwork · {localSeries.artwork.length} image{localSeries.artwork.length === 1 ? "" : "s"}</summary><div class="metadata-local-artwork-grid">{#each localSeries.artwork.slice(0, 4) as image (image.path)}<figure>{#if localAssetsState.localArtworkPreviews[image.path]}<img src={localAssetsState.localArtworkPreviews[image.path]} alt={`${image.role}: ${image.name}`} />{:else if localAssetsState.localArtworkPreviewBusy[image.path]}<span>Loading preview…</span>{:else}<button class="link-button" onclick={() => loadLocalArtworkPreviews([image])}>Load preview</button>{/if}<figcaption>{image.role} · {image.name}</figcaption></figure>{/each}</div></details>
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
                          <button class="button secondary" disabled={searchState.loadingDetail} onclick={() => reviewSeriesCandidate(group.root, row)}>{evidence.conflict ? "Review year conflict" : "Review & assign to this series"}</button>
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

  <WorkbenchMoviesPanel {derived} {animationForPath} {applyBatchLocalPoster} {artworkSaveState} {batchLocalPosterTargets} {candidateListExpanded} {chooseLocalPoster} {chooseMovieLocalPoster} {chooseMovieProviderPoster} {compactItemMode} {findMovieMatches} {loadLocalArtworkPreviews} {localAssetsState} {localFileAssets} {lookupState} bind:movieAssignments {movieLookupLabel} {movieSearchCount} {onAnimationChange} {organizerState} {previewBatchLocalPoster} {previewLocalArtwork} {renameState} {reviewMovieCandidate} {reviewPosterOptions} {saveLocalPoster} {savedMovieMatchSet} {searchMovieRecord} {searchState} {selectMovieArtwork} bind:selectedMediaPaths {stopFilenameLookup} {suggestionForPath} {tmdbCredentialStatus} {toggleCandidateList} {toggleMedia} {visibleCandidates} />


  {#if !compactItemMode}<p class="metadata-lookup-privacy">Search results stay attached to the film or series that started the search. Enabled providers receive only the cleaned title; filenames, paths, year clues, local artwork and video bytes stay on this device. Matches are suggestions until you assign and confirm them.</p>{/if}
  {#if localAssetsState.localAssetScanError}<div class="metadata-error" role="status"><TriangleAlert size={17} /><span>Local artwork inspection could not finish: {localAssetsState.localAssetScanError}. Public lookup is paused until the local check succeeds.</span></div>{/if}

  <div class="metadata-grid" class:metadata-focused-grid={compactItemMode}>
    <section class="metadata-search card-surface" class:metadata-search-active={Boolean(searchState.searchTarget)}>
      {#if searchState.searchTarget}
        <div class="metadata-card-title"><span class="settings-icon"><Search size={19} /></span><div><h2>Search for {searchState.searchTarget.kind === "movie" ? "this film" : "this series"}</h2><p>{searchState.searchTarget.label} · searches use enabled sources in Settings.</p></div><button class="icon-button" aria-label="Close title search" title="Close title search" onclick={closeTitleSearch}><X size={17} /></button></div>
        <div class="metadata-query-row">
          <label>Title or alternate spelling
            <input bind:this={searchState.titleSearchInput} bind:value={searchState.query} onkeydown={(event) => event.key === "Enter" && !searchState.searching && searchTitles()} placeholder={searchState.searchTarget.kind === "movie" ? "e.g. Gnomeo & Juliet" : "e.g. The Expanse"} />
          </label>
          <button class="button primary" disabled={searchState.searching || searchState.loadingDetail || !searchState.query.trim()} onclick={searchTitles}>
            {#if searchState.searching}<LoaderCircle size={16} class="spin" /> Searching sources{:else}<Search size={16} /> Search sources{/if}
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
      {#if searchState.results.some((row) => row.provider === "tmdb") || searchState.selected?.provider === "tmdb"}
        <div class="metadata-attribution"><div><a href="https://www.themoviedb.org" target="_blank" rel="noreferrer" aria-label="The Movie Database (TMDB)"><img src={tmdbLogo} alt="TMDB" /></a><strong>Credits · The Movie Database API</strong></div><span>This product uses the TMDB API but is not endorsed or certified by TMDB. API data and images are provided by TMDB and their contributors; review applicable terms before reuse.</span></div>
      {/if}
      {#each searchState.searchWarnings as warning (warning)}<div class="metadata-search-note" role="status"><TriangleAlert size={15} />{warning}</div>{/each}
      {#if searchState.error}<div class="metadata-error"><TriangleAlert size={17} /><span>{searchState.error}</span></div>{/if}
      {#if searchState.results.length}
          <div class="metadata-results-heading"><div><span class="eyebrow">POSSIBLE MATCHES</span><strong>{searchState.results.length} results · showing {Math.min(searchState.results.length, candidateListExpanded("manual-search") ? searchState.results.length : initialCandidateCount)}</strong></div><small>Order uses title, year and provider agreement. It is not a probability score.</small></div>
        <div class="metadata-results">
          {#each visibleCandidates(searchState.results, "manual-search") as row (`${row.provider}:${row.id}`)}
            {@const preview = candidatePreview(row)}
            {@const evidence = searchState.manualSearchClue ? candidateEvidence(row, searchState.manualSearchClue) : null}
            <article class="metadata-result" class:selected={searchState.selected?.provider === row.provider && searchState.selected?.id === row.id}>
              {#if preview}<img src={preview.thumbnailUrl} alt={`Artwork preview for ${row.title} from ${providerLabel(row.provider)}`} loading="lazy" />{:else}<span class="metadata-result-placeholder">{#if row.kind === "movie"}<Film size={20} />{:else}<Tv size={20} />{/if}</span>{/if}
              <span class="metadata-result-copy"><strong>{row.title}</strong><small>{providerLabel(row.provider)} · {row.year ?? "Year unknown"}{#if row.description} · {row.description.slice(0, 125)}{/if}{#if evidence}<span class:metadata-year-conflict={evidence.conflict}>{evidence.conflict ? ` · ${evidence.text}` : ` · ${evidence.text}`}</span>{/if}</small></span>
              {#if evidence?.conflict}<span class="metadata-score metadata-score-conflict">Year conflict<small>review first</small></span>{:else}<span class="metadata-score">{evidence?.text.startsWith("Exact title") ? "Exact title" : "Review match"}<small>{evidence?.text.includes("year matches") ? "year matches" : "check details"}</small></span>{/if}
              <button class="button secondary" disabled={searchState.loadingDetail} onclick={() => row.kind === "movie" && searchState.searchTarget?.kind === "movie" ? reviewMovieCandidate(searchState.searchTarget.path, row, !evidence?.conflict) : reviewTargetCandidate(row)}>{searchState.loadingDetail && searchState.selected?.id === row.id ? "Loading…" : evidence?.conflict ? "Review year conflict" : "Review & assign"}</button>
            </article>
          {/each}
        </div>
        {#if searchState.results.length > initialCandidateCount}<button class="link-button metadata-show-more" aria-expanded={candidateListExpanded("manual-search")} onclick={() => toggleCandidateList("manual-search")}>{candidateListExpanded("manual-search") ? "Show fewer matches" : `Show all ${searchState.results.length} matches`}</button>{/if}
      {/if}
    </section>

  <WorkbenchDetailPanel {derived} {applyArtworkSave} {applyNames} {artworkSaveState} {assignCurrentMovie} {chooseMovieProviderPoster} {chooseOrganizerFolder} {chooseSeriesLocalPoster} {chooseSeriesLocalSeasonPoster} {chooseSeriesProviderPoster} {chooseSeriesProviderSeasonPoster} {compactItemMode} {createMetadataOnlyCopies} {currentReviewedSeriesRoot} {currentSeriesYearMismatch} {episodeForKey} {loadLocalArtworkPreviews} {localAssetsState} {localDirectoryAssets} {movieArtworkTargets} bind:movieAssignments {organizerState} bind:organizerTemplates {plannedArtworkPaths} {previewNames} {renameState} {saveOrganizerTemplates} {scanCurrentSeriesFolder} {scanResult} {searchCurrentSeriesRecord} {searchMovieRecord} {searchState} {selectAllMedia} bind:selectedMediaPaths {selectedMovieArtwork} {selectedPathsInOrganizer} {selectedSeasonArtwork} {selectedSeriesArtwork} bind:seriesAssignments {stageArtworkSave} {stageCurrentFilmPoster} {toggleMedia} {updateEpisode} />
  </div>
  {#if compactItemMode && organizerState.organizerScan && selectedPathsInOrganizer().length}
    <section class="metadata-focused-rename" aria-label="Rename this media file">
      <div class="metadata-focused-rename-heading">
        <span class="eyebrow">FILENAME</span>
        <strong>{derived.visibleOrganizerFiles.find((file) => file.path === selectedPathsInOrganizer()[0])?.name ?? "Selected media"}</strong>
        <small>{compactRenameReady() ? "Preview the proposed name, then confirm the move. The video itself is not changed." : searchState.mediaKind === "movie" ? "Review or assign a film title before preparing a filename." : "Choose and review an episode before preparing a filename."}</small>
      </div>
      <button class="button secondary" type="button" disabled={!derived.desktop || renameState.renameBusy || !compactRenameReady() || Boolean(currentSeriesYearMismatch() && !organizerState.seriesYearConflictConfirmed)} onclick={previewNames}>
        {#if renameState.renameBusy}<LoaderCircle size={15} class="spin" /> Preparing rename preview{:else}Preview filename change{/if}
      </button>
      {#if renameState.renameError}<div class="metadata-error" role="alert"><TriangleAlert size={16} /><span>{renameState.renameError}</span></div>{/if}
      {#if renameState.renameMessage}<div class="metadata-success" role="status"><strong>Filename updated</strong><span>{renameState.renameMessage}</span></div>{/if}
      {#if renameState.renamePreview}
        <div class="metadata-focused-rename-preview">
          {#each renameState.renamePreview.items as item (item.sourcePath)}<div><span>Before</span><code>{item.sourcePath}</code><span>After</span><code>{item.targetPath}</code></div>{/each}
          {#if renameState.renamePreview.conflicts.length}<div class="metadata-conflicts"><strong>Rename blocked · resolve the conflicts first</strong>{#each renameState.renamePreview.conflicts as conflict}<p>{conflict.reason}<small>{conflict.targetPath}</small></p>{/each}</div>{:else if renameState.renamePreview.canApply}<p>Only the path and matching supported sidecars will move. The next step can make a separately validated, no-encode MKV copy.</p><button class="button primary" type="button" disabled={renameState.renameBusy} onclick={() => renameState.confirmRename = true}>Review rename confirmation</button>{:else}<small>This file already has the proposed name.</small>{/if}
        </div>
      {/if}
      {#if renameState.confirmRename && renameState.renamePreview?.canApply}
        <div class="metadata-rename-confirm"><strong>Rename this file?</strong><span>The app rechecks the current preview and destination before moving the file. Video contents remain unchanged; an operation error triggers rollback.</span><div><button class="button secondary" type="button" disabled={renameState.renameBusy} onclick={() => renameState.confirmRename = false}>Back to preview</button><button class="button danger" type="button" disabled={renameState.renameBusy} onclick={applyNames}>{renameState.renameBusy ? "Renaming…" : "Confirm rename"}</button></div></div>
      {/if}
    </section>
  {/if}
          <p class="metadata-boundary">Review matches and edit the fields you want to use. Renaming, poster saving, and MKV creation have separate previews and confirmations. A metadata-only MKV embeds supported fields and selected poster art in a new file, then validates it while keeping the source in place.</p>
</section>
