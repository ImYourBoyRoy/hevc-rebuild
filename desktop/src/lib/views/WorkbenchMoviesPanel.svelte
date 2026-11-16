<script lang="ts">
  import { Film, ImagePlus, LoaderCircle, Pencil, Search } from "@lucide/svelte";
  import { assignedMoviePoster, candidateEvidence, candidatePreview, clueCaption, movieArtworkTarget, providerLabel, suggestionResult } from "#lib/metadata-candidates.js";
  import { basename } from "#lib/organizer-analysis.js";
  import { initialCandidateCount } from "#lib/search-panel.js";
  import { preferredLocalPoster } from "#lib/library-browse.js";
  import type { MovieAssignment, TmdbCredentialStatus } from "#lib/metadata-workbench-types.js";
  import type { WorkbenchDerived } from "#lib/state/workbench-derived.svelte.js";
  import type { WorkbenchArtworkSaveState } from "#lib/state/workbench-artwork-save-state.svelte.js";
  import type { WorkbenchLocalAssetsState } from "#lib/state/workbench-local-assets-state.svelte.js";
  import type { WorkbenchLookupState } from "#lib/state/workbench-lookup-state.svelte.js";
  import type { WorkbenchOrganizerState } from "#lib/state/workbench-organizer-state.svelte.js";
  import type { WorkbenchRenameState } from "#lib/state/workbench-rename-state.svelte.js";
  import type { WorkbenchSearchState } from "#lib/state/workbench-search-state.svelte.js";
  import type { createArtworkSave } from "#lib/artwork-save.js";
  import type { createLocalAssets } from "#lib/local-assets.js";
  import type { createMatchAssignments } from "#lib/match-assignments.js";
  import type { createMatchLookup } from "#lib/match-lookup.js";
  import type { createOrganizerRename } from "#lib/organizer-rename.js";
  import type { createPosterSelection } from "#lib/poster-selection.js";
  import type { createProviderDetails } from "#lib/provider-details.js";
  import type { createSearchPanel } from "#lib/search-panel.js";

  type Props = {
    derived: WorkbenchDerived;
    animationForPath: (path: string) => boolean;
    applyBatchLocalPoster: ReturnType<typeof createPosterSelection>["applyBatchLocalPoster"];
    artworkSaveState: WorkbenchArtworkSaveState;
    batchLocalPosterTargets: ReturnType<typeof createPosterSelection>["batchLocalPosterTargets"];
    candidateListExpanded: ReturnType<typeof createSearchPanel>["candidateListExpanded"];
    chooseLocalPoster: ReturnType<typeof createArtworkSave>["chooseLocalPoster"];
    chooseMovieLocalPoster: ReturnType<typeof createPosterSelection>["chooseMovieLocalPoster"];
    chooseMovieProviderPoster: ReturnType<typeof createPosterSelection>["chooseMovieProviderPoster"];
    compactItemMode: boolean;
    findMovieMatches: ReturnType<typeof createMatchLookup>["findMovieMatches"];
    loadLocalArtworkPreviews: ReturnType<typeof createLocalAssets>["loadLocalArtworkPreviews"];
    localAssetsState: WorkbenchLocalAssetsState;
    localFileAssets: ReturnType<typeof createLocalAssets>["localFileAssets"];
    lookupState: WorkbenchLookupState;
    movieAssignments: Record<string, MovieAssignment>;
    movieLookupLabel: ReturnType<typeof createMatchAssignments>["movieLookupLabel"];
    movieSearchCount: ReturnType<typeof createMatchAssignments>["movieSearchCount"];
    onAnimationChange: (path: string, animated: boolean) => void;
    organizerState: WorkbenchOrganizerState;
    previewBatchLocalPoster: ReturnType<typeof createPosterSelection>["previewBatchLocalPoster"];
    previewLocalArtwork: ReturnType<typeof createLocalAssets>["previewLocalArtwork"];
    renameState: WorkbenchRenameState;
    reviewMovieCandidate: ReturnType<typeof createProviderDetails>["reviewMovieCandidate"];
    reviewPosterOptions: ReturnType<typeof createMatchAssignments>["reviewPosterOptions"];
    saveLocalPoster: ReturnType<typeof createArtworkSave>["saveLocalPoster"];
    savedMovieMatchSet: ReturnType<typeof createMatchAssignments>["savedMovieMatchSet"];
    searchMovieRecord: ReturnType<typeof createSearchPanel>["searchMovieRecord"];
    searchState: WorkbenchSearchState;
    selectMovieArtwork: ReturnType<typeof createPosterSelection>["selectMovieArtwork"];
    selectedMediaPaths: string[];
    stopFilenameLookup: ReturnType<typeof createMatchLookup>["stopFilenameLookup"];
    suggestionForPath: ReturnType<typeof createMatchAssignments>["suggestionForPath"];
    tmdbCredentialStatus: TmdbCredentialStatus;
    toggleCandidateList: ReturnType<typeof createSearchPanel>["toggleCandidateList"];
    toggleMedia: ReturnType<typeof createOrganizerRename>["toggleMedia"];
    visibleCandidates: ReturnType<typeof createSearchPanel>["visibleCandidates"];
  };
  let { derived, animationForPath, applyBatchLocalPoster, artworkSaveState, batchLocalPosterTargets, candidateListExpanded, chooseLocalPoster, chooseMovieLocalPoster, chooseMovieProviderPoster, compactItemMode, findMovieMatches, loadLocalArtworkPreviews, localAssetsState, localFileAssets, lookupState, movieAssignments = $bindable(), movieLookupLabel, movieSearchCount, onAnimationChange, organizerState, previewBatchLocalPoster, previewLocalArtwork, renameState, reviewMovieCandidate, reviewPosterOptions, saveLocalPoster, savedMovieMatchSet, searchMovieRecord, searchState, selectMovieArtwork, selectedMediaPaths = $bindable(), stopFilenameLookup, suggestionForPath, tmdbCredentialStatus, toggleCandidateList, toggleMedia, visibleCandidates }: Props = $props();
</script>

  {#if searchState.mediaKind === "movie" && organizerState.organizerSource === "analysis" && organizerState.organizerScan && compactItemMode}
    {#each derived.visibleOrganizerFiles as file (file.path)}
      {@const localFile = localFileAssets(file.path)}
      {@const draft = movieAssignments[file.path]}
      {@const matchSet = lookupState.movieSuggestions[file.path] ?? savedMovieMatchSet(file)}
      {@const posterChoices = reviewPosterOptions(file.path, draft, matchSet?.results ?? [])}
      {@const stagePoster = posterChoices.find((image) => image.id === draft?.posterId) ?? posterChoices[0]}
      {@const stageImage = (draft?.localPosterPath ? localAssetsState.localArtworkPreviews[draft.localPosterPath] : "") || stagePoster?.imageUrl || stagePoster?.thumbnailUrl || ""}
      <section class="review-studio" aria-label="Review this film">
        <div class="review-poster-column">
          <figure class="review-poster-stage">
            {#if stageImage}<img src={stageImage} alt="" referrerpolicy="no-referrer" />{:else}<Film size={42} /><strong>No poster yet</strong>{/if}
          </figure>
          {#if posterChoices.length > 0 || (localFile?.artwork.length ?? 0) > 0}
            <div class="review-poster-rail" role="listbox" aria-label="Poster choices" {@attach previewLocalArtwork(localFile?.artwork ?? [])}>
              {#each localFile?.artwork ?? [] as image (image.path)}
                <button type="button" role="option" aria-selected={draft?.localPosterPath === image.path} class:is-selected={draft?.localPosterPath === image.path} onclick={() => chooseMovieLocalPoster(file.path, image.path)}>
                  {#if localAssetsState.localArtworkPreviews[image.path]}<img src={localAssetsState.localArtworkPreviews[image.path]} alt="" />{:else}<span>Local</span>{/if}
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
          {#if matchSet?.status === "searching" || searchState.loadingDetail}<p class="review-status" role="status"><LoaderCircle size={16} /> Loading the matched plot, genres, and posters…</p>{/if}
          {#if matchSet?.status === "error"}<p class="metadata-error-text" role="alert">{matchSet.error}</p>{/if}
          {#if searchState.reviewDetailError}<p class="metadata-error-text" role="alert">{searchState.reviewDetailError}</p>{/if}
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
          <button class="button secondary" type="button" disabled={!derived.desktop || lookupState.lookupBusy} onclick={() => findMovieMatches([file])}>{matchSet?.status === "searching" ? "Searching…" : "Find posters"}</button>
          <button class="button secondary" type="button" disabled={!derived.desktop} onclick={() => searchMovieRecord(file, draft?.title)}><Search size={15} /> Search another title</button>
          <button class="button secondary" type="button" disabled={!derived.desktop} onclick={() => chooseLocalPoster(file)}><ImagePlus size={15} /> Use your own image</button>
        </div>
      </section>
    {/each}
  {/if}

  {#if searchState.mediaKind === "movie" && organizerState.organizerSource === "analysis" && organizerState.organizerScan && !compactItemMode}
    {@const movieScan = organizerState.organizerScan}
    <section class="metadata-analysis-movies card-surface" aria-label="Scanned movie selection">
      <div class="metadata-analysis-movies-heading"><div><div class="eyebrow">ANALYZED MOVIES</div><strong>{derived.visibleOrganizerFiles.length} film file{derived.visibleOrganizerFiles.length === 1 ? "" : "s"}</strong></div><div class="metadata-movie-lookup-actions"><small>{derived.visibleOrganizerFiles.filter((file) => selectedMediaPaths.includes(file.path)).length} selected · title clues stay on this device</small><button class="button primary" disabled={!derived.desktop || lookupState.lookupBusy || movieSearchCount(derived.visibleOrganizerFiles) === 0} onclick={() => findMovieMatches(derived.visibleOrganizerFiles)}>{lookupState.lookupBusy && lookupState.lookupProgress?.label === "analyzed films" ? "Searching sources…" : movieSearchCount(derived.visibleOrganizerFiles) ? `Search ${movieSearchCount(derived.visibleOrganizerFiles)} needing a match` : "All films have saved matches"}</button>{#if lookupState.lookupBusy}<button class="link-button" onclick={stopFilenameLookup}>Stop after current lookup</button>{/if}</div></div>
      {#if lookupState.lookupProgress?.label === "analyzed films"}<div class="metadata-lookup-progress" role="status">Searching {lookupState.lookupProgress.done} of {lookupState.lookupProgress.total} analyzed films. Duplicate title queries are reused.</div>{/if}
      {#if lookupState.lookupMessage && lookupState.lookupMessageScope === "analyzed films"}<div class="metadata-lookup-progress" role="status">{lookupState.lookupMessage}</div>{/if}
      {#if derived.visibleOrganizerFiles.length}
        <div class="metadata-analysis-movies-list">
          {#each derived.visibleOrganizerFiles as file (file.path)}
            {@const localFile = localFileAssets(file.path)}
            {@const batchPosterTargets = batchLocalPosterTargets()}
            {@const savedSuggestion = suggestionForPath(file.path)}
            {@const matchSet = lookupState.movieSuggestions[file.path] ?? savedMovieMatchSet(file)}
            {@const savedResult = suggestionResult(savedSuggestion, "movie")}
            {@const reviewedPoster = movieAssignments[file.path] ? assignedMoviePoster(movieAssignments[file.path]) : undefined}
            {@const selectedLocalPosterPreview = movieAssignments[file.path]?.localPosterPath ? localAssetsState.localArtworkPreviews[movieAssignments[file.path].localPosterPath] : ""}
            {@const discoveredLocalPosterPath = preferredLocalPoster(localFile?.artwork ?? [], null)}
            {@const discoveredLocalPosterPreview = discoveredLocalPosterPath ? localAssetsState.localArtworkPreviews[discoveredLocalPosterPath] : ""}
            {@const matchImage = selectedLocalPosterPreview || reviewedPoster?.thumbnailUrl || reviewedPoster?.imageUrl || savedSuggestion?.artwork?.thumbnailUrl || savedSuggestion?.artwork?.imageUrl || discoveredLocalPosterPreview || ""}
            <article class="metadata-analysis-movie-row" data-media-path={file.path}>
              <div class="metadata-analysis-movie-main"><label><input type="checkbox" checked={selectedMediaPaths.includes(file.path)} onchange={(event) => toggleMedia(file.path, event.currentTarget.checked)} /><span><strong>{file.name}</strong><small>{file.parentPath}</small></span></label><div class="metadata-record-actions"><button class="button secondary" disabled={!derived.desktop || lookupState.lookupBusy} onclick={() => savedResult && savedSuggestion?.status === "ready" ? reviewMovieCandidate(file.path, savedResult, true) : findMovieMatches([file])}>{movieLookupLabel(file, matchSet)}</button><button class="link-button" disabled={!derived.desktop} onclick={() => searchMovieRecord(file)}><Search size={14} /> Search another title</button></div></div>
              <div class="metadata-movie-match-card" class:metadata-movie-match-reviewed={Boolean(movieAssignments[file.path])} class:metadata-movie-match-suggested={Boolean(!movieAssignments[file.path] && savedSuggestion?.status === "ready")} class:metadata-movie-match-empty={!movieAssignments[file.path] && savedSuggestion?.status !== "ready"}>
                <div class="metadata-movie-match-poster">{#if matchImage}<img src={matchImage} alt="" loading="lazy" referrerpolicy="no-referrer" />{:else}<span><Film size={20} /></span>{/if}</div>
                <div class="metadata-movie-match-copy"><small>{movieAssignments[file.path] ? "REVIEWED MATCH" : savedSuggestion?.status === "ready" ? "SAVED SUGGESTION" : localFile?.artwork.length ? "LOCAL ARTWORK FOUND" : matchSet?.status === "searching" ? "SEARCHING" : "NO MATCH YET"}</small><strong>{movieAssignments[file.path]?.title || savedSuggestion?.title || "Needs a title match"}{movieAssignments[file.path]?.year ? ` (${movieAssignments[file.path].year})` : savedSuggestion?.year ? ` (${savedSuggestion.year})` : ""}</strong><span>{movieAssignments[file.path] ? (selectedLocalPosterPreview ? "Local poster selected" : reviewedPoster ? `${reviewedPoster.provider} artwork selected` : discoveredLocalPosterPreview ? "Local artwork found below · choose it to use it" : "Review details and choose a poster below") : savedSuggestion?.status === "ready" ? `${savedSuggestion.provider?.toUpperCase() ?? "Public source"} · review before assigning` : localFile?.artwork.length ? "Artwork was found beside this file · open it below to preview" : matchSet?.status === "searching" ? "Checking public film records…" : "Search this film or use Search another title"}</span></div>
                <div class="metadata-movie-match-actions">{#if movieAssignments[file.path]}<button class="link-button" type="button" onclick={() => lookupState.movieEditorOpen = { ...lookupState.movieEditorOpen, [file.path]: true }}><Pencil size={14} /> Edit</button>{:else if savedResult}<button class="button secondary" type="button" disabled={searchState.loadingDetail} onclick={() => reviewMovieCandidate(file.path, savedResult, true)}>Review & assign</button>{/if}</div>
              </div>
              {#if localFile?.artwork.length}
                <details class="metadata-local-artwork" ontoggle={(event) => event.currentTarget.open && loadLocalArtworkPreviews(localFile.artwork)}>
                  <summary>Local artwork · {localFile.artwork.length} image{localFile.artwork.length === 1 ? "" : "s"}</summary>
                  <div class="metadata-local-artwork-grid">
                    {#each localFile.artwork.slice(0, 4) as image (image.path)}
                      <figure>
                        {#if localAssetsState.localArtworkPreviews[image.path]}<img src={localAssetsState.localArtworkPreviews[image.path]} alt={`${image.role}: ${image.name}`} />{:else if localAssetsState.localArtworkPreviewBusy[image.path]}<span>Loading preview…</span>{:else}<button class="link-button" onclick={() => loadLocalArtworkPreviews([image])}>Load preview</button>{/if}
                        {#if movieAssignments[file.path]}
                          <button type="button" class="metadata-artwork-choice" class:selected={movieAssignments[file.path].localPosterPath === image.path} onclick={() => chooseMovieLocalPoster(file.path, image.path)}>{movieAssignments[file.path].localPosterPath === image.path ? "Clear local poster" : "Use as film poster"}</button>
                          {#if batchPosterTargets.length > 1}<button type="button" class="metadata-artwork-choice metadata-artwork-batch-choice" onclick={() => previewBatchLocalPoster(file.path, image.path)}>Use for {batchPosterTargets.length} selected films</button>{/if}
                          {#if artworkSaveState.pendingBatchLocalPoster?.sourcePath === file.path && artworkSaveState.pendingBatchLocalPoster.artworkPath === image.path}
                            <div class="metadata-local-batch-review">
                              <strong>Apply this image to {artworkSaveState.pendingBatchLocalPoster.mediaPaths.length} films?</strong>
                              <small>The existing image will be selected for these files. No image file will be copied or replaced.</small>
                              <ul>{#each artworkSaveState.pendingBatchLocalPoster.mediaPaths as path (path)}<li><strong>{movieScan.files.find((item) => item.path === path)?.name ?? basename(path)}</strong><code>{path}</code></li>{/each}</ul>
                              <div><button class="text-button" type="button" onclick={() => artworkSaveState.pendingBatchLocalPoster = null}>Cancel</button><button class="button secondary" type="button" onclick={applyBatchLocalPoster}>Apply to selected films</button></div>
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
                          <button class="button secondary" disabled={searchState.loadingDetail} onclick={() => reviewMovieCandidate(file.path, row, !evidence.conflict)}>{evidence.conflict ? "Review year conflict" : "Review & assign to this film"}</button>
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
                <details class="metadata-inline-draft" bind:open={lookupState.movieEditorOpen[file.path]}>
                  <summary>Edit this film’s metadata</summary>
                  <label>Title<input bind:value={movieAssignments[file.path].title} oninput={() => { renameState.renamePreview = null; }} /></label>
                  <div class="metadata-inline-draft-row"><label>Year<input bind:value={movieAssignments[file.path].year} inputmode="numeric" oninput={() => { renameState.renamePreview = null; }} /></label><label>Certification<input bind:value={movieAssignments[file.path].certification} oninput={() => { renameState.renamePreview = null; }} /></label></div>
                  <label>Short description<input bind:value={movieAssignments[file.path].shortDescription} /></label>
                  <label>Plot<textarea rows="3" bind:value={movieAssignments[file.path].plot}></textarea></label>
                  <div class="metadata-inline-draft-row"><label>Genres<input bind:value={movieAssignments[file.path].genresText} /><small>Separate with commas or semicolons.</small></label><label>Rating · 0 to 10<input type="number" min="0" max="10" step="0.1" bind:value={movieAssignments[file.path].ratingText} /></label></div>
                  <label>Source URL · optional<input type="url" placeholder="https://…" bind:value={movieAssignments[file.path].sourceUrl} /></label>
                  {#if draft.artworkCandidates.length}<label>Poster<select disabled={artworkSaveState.artworkBusy} bind:value={movieAssignments[file.path].posterId} onchange={() => { chooseMovieProviderPoster(file.path); }}><option value="">No poster selected</option>{#each draft.artworkCandidates as image (image.id)}<option value={image.id}>{image.provider} · {image.title}{image.license ? ` · ${image.license}` : ""}</option>{/each}</select></label>{/if}
                  <div class="metadata-local-poster-actions"><button class="link-button" type="button" disabled={artworkSaveState.localPosterBusy} onclick={() => chooseLocalPoster(file)}>Choose your own poster image</button><small>JPEG, PNG, or WebP · copied beside this video · existing files are never overwritten</small></div>
                  {#if artworkSaveState.pendingLocalPoster?.target.directory === movieArtworkTarget(file).directory && artworkSaveState.pendingLocalPoster?.target.basename === movieArtworkTarget(file).basename}
                    <div class="metadata-local-poster-review"><strong>{artworkSaveState.pendingLocalPoster.saveBesideMedia ? "Copy this poster beside the film?" : "Save this poster to the app artwork cache?"}</strong><small>Image: {artworkSaveState.pendingLocalPoster.sourcePath}</small><code>{artworkSaveState.pendingLocalPoster.targetPath}</code><div><button class="link-button" disabled={artworkSaveState.localPosterBusy} onclick={() => artworkSaveState.pendingLocalPoster = null}>Cancel</button><button class="button secondary" disabled={artworkSaveState.localPosterBusy} onclick={saveLocalPoster}>{artworkSaveState.localPosterBusy ? "Saving poster…" : "Save poster"}</button></div></div>
                  {/if}
                  {#if artworkSaveState.localPosterError}<small class="metadata-error-text" role="status">{artworkSaveState.localPosterError}</small>{/if}
                  {#if artworkSaveState.localPosterMessage}<small class="metadata-success-text" role="status">{artworkSaveState.localPosterMessage}</small>{/if}
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
