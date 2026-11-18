<script lang="ts">
  import { ArrowRight, Database, ExternalLink, FileText, Film, LoaderCircle, Search, TriangleAlert } from "@lucide/svelte";
  import { artworkFileExtension, assignedMoviePoster, movieArtworkTarget, safeHttpUrl, seasonArtworkTargets, seriesArtworkTargets } from "#lib/metadata-candidates.js";
  import type { MovieAssignment, SeriesAssignment, OrganizerTemplates } from "#lib/metadata-workbench-types.js";
  import type { ScanResult } from "#lib/types.js";
  import type { WorkbenchDerived } from "#lib/state/workbench-derived.svelte.js";
  import type { WorkbenchArtworkSaveState } from "#lib/state/workbench-artwork-save-state.svelte.js";
  import type { WorkbenchLocalAssetsState } from "#lib/state/workbench-local-assets-state.svelte.js";
  import type { WorkbenchOrganizerState } from "#lib/state/workbench-organizer-state.svelte.js";
  import type { WorkbenchRenameState } from "#lib/state/workbench-rename-state.svelte.js";
  import type { WorkbenchSearchState } from "#lib/state/workbench-search-state.svelte.js";
  import type { createArtworkSave } from "#lib/artwork-save.js";
  import type { createLocalAssets } from "#lib/local-assets.js";
  import type { createOrganizerRename } from "#lib/organizer-rename.js";
  import type { createOrganizerSession } from "#lib/organizer-session.js";
  import type { createPosterSelection } from "#lib/poster-selection.js";
  import type { createSearchPanel } from "#lib/search-panel.js";

  type Props = {
    derived: WorkbenchDerived;
    applyArtworkSave: ReturnType<typeof createArtworkSave>["applyArtworkSave"];
    applyNames: ReturnType<typeof createOrganizerRename>["applyNames"];
    artworkSaveState: WorkbenchArtworkSaveState;
    assignCurrentMovie: ReturnType<typeof createOrganizerRename>["assignCurrentMovie"];
    chooseMovieProviderPoster: ReturnType<typeof createPosterSelection>["chooseMovieProviderPoster"];
    chooseOrganizerFolder: ReturnType<typeof createOrganizerSession>["chooseOrganizerFolder"];
    chooseSeriesLocalPoster: ReturnType<typeof createPosterSelection>["chooseSeriesLocalPoster"];
    chooseSeriesLocalSeasonPoster: ReturnType<typeof createPosterSelection>["chooseSeriesLocalSeasonPoster"];
    chooseSeriesProviderPoster: ReturnType<typeof createPosterSelection>["chooseSeriesProviderPoster"];
    chooseSeriesProviderSeasonPoster: ReturnType<typeof createPosterSelection>["chooseSeriesProviderSeasonPoster"];
    compactItemMode: boolean;
    createMetadataOnlyCopies: ReturnType<typeof createOrganizerRename>["createMetadataOnlyCopies"];
    currentReviewedSeriesRoot: ReturnType<typeof createSearchPanel>["currentReviewedSeriesRoot"];
    currentSeriesYearMismatch: ReturnType<typeof createSearchPanel>["currentSeriesYearMismatch"];
    episodeForKey: ReturnType<typeof createOrganizerRename>["episodeForKey"];
    loadLocalArtworkPreviews: ReturnType<typeof createLocalAssets>["loadLocalArtworkPreviews"];
    localAssetsState: WorkbenchLocalAssetsState;
    localDirectoryAssets: ReturnType<typeof createLocalAssets>["localDirectoryAssets"];
    movieArtworkTargets: ReturnType<typeof createArtworkSave>["movieArtworkTargets"];
    movieAssignments: Record<string, MovieAssignment>;
    organizerState: WorkbenchOrganizerState;
    organizerTemplates: OrganizerTemplates;
    plannedArtworkPaths: ReturnType<typeof createArtworkSave>["plannedArtworkPaths"];
    previewNames: ReturnType<typeof createOrganizerRename>["previewNames"];
    renameState: WorkbenchRenameState;
    saveOrganizerTemplates: ReturnType<typeof createOrganizerRename>["saveOrganizerTemplates"];
    scanCurrentSeriesFolder: ReturnType<typeof createOrganizerSession>["scanCurrentSeriesFolder"];
    scanResult: ScanResult | null;
    searchCurrentSeriesRecord: ReturnType<typeof createSearchPanel>["searchCurrentSeriesRecord"];
    searchMovieRecord: ReturnType<typeof createSearchPanel>["searchMovieRecord"];
    searchState: WorkbenchSearchState;
    selectAllMedia: ReturnType<typeof createOrganizerRename>["selectAllMedia"];
    selectedMediaPaths: string[];
    selectedMovieArtwork: ReturnType<typeof createArtworkSave>["selectedMovieArtwork"];
    selectedPathsInOrganizer: ReturnType<typeof createOrganizerRename>["selectedPathsInOrganizer"];
    selectedSeasonArtwork: ReturnType<typeof createArtworkSave>["selectedSeasonArtwork"];
    selectedSeriesArtwork: ReturnType<typeof createArtworkSave>["selectedSeriesArtwork"];
    seriesAssignments: Record<string, SeriesAssignment>;
    stageArtworkSave: ReturnType<typeof createArtworkSave>["stageArtworkSave"];
    stageCurrentFilmPoster: ReturnType<typeof createArtworkSave>["stageCurrentFilmPoster"];
    toggleMedia: ReturnType<typeof createOrganizerRename>["toggleMedia"];
    updateEpisode: ReturnType<typeof createOrganizerRename>["updateEpisode"];
  };
  let { derived, applyArtworkSave, applyNames, artworkSaveState, assignCurrentMovie, chooseMovieProviderPoster, chooseOrganizerFolder, chooseSeriesLocalPoster, chooseSeriesLocalSeasonPoster, chooseSeriesProviderPoster, chooseSeriesProviderSeasonPoster, compactItemMode, createMetadataOnlyCopies, currentReviewedSeriesRoot, currentSeriesYearMismatch, episodeForKey, loadLocalArtworkPreviews, localAssetsState, localDirectoryAssets, movieArtworkTargets, movieAssignments = $bindable(), organizerState, organizerTemplates = $bindable(), plannedArtworkPaths, previewNames, renameState, saveOrganizerTemplates, scanCurrentSeriesFolder, scanResult, searchCurrentSeriesRecord, searchMovieRecord, searchState, selectAllMedia, selectedMediaPaths = $bindable(), selectedMovieArtwork, selectedPathsInOrganizer, selectedSeasonArtwork, selectedSeriesArtwork, seriesAssignments = $bindable(), stageArtworkSave, stageCurrentFilmPoster, toggleMedia, updateEpisode }: Props = $props();
</script>

    <section class="metadata-detail card-surface">
      {#if searchState.loadingDetail}
        <div class="metadata-empty"><LoaderCircle size={24} class="spin" /><strong>Loading {searchState.mediaKind === "movie" ? "film" : "series"} details</strong><span>{searchState.mediaKind === "movie" ? "Getting title information, certification and artwork." : "Getting show details, seasons, episodes and series artwork."}</span></div>
      {:else if searchState.detail}
        <div class="metadata-detail-heading"><div><div class="eyebrow">REVIEWED SOURCE RECORD</div><h2>{searchState.detail.title}{#if searchState.detail.year} <span>({searchState.detail.year})</span>{/if}</h2><p>{searchState.detail.originalTitle && searchState.detail.originalTitle !== searchState.detail.title ? `Original title: ${searchState.detail.originalTitle} · ` : ""}{searchState.detail.genres.join(" · ") || searchState.detail.attribution}</p><small class="metadata-provider-attribution">{searchState.detail.attribution}</small>{#if searchState.detail.episodeOrderLabel}<div class="metadata-episode-order-note"><strong>Episode numbering:</strong> {searchState.detail.episodeOrderLabel}. Check it against the source files before assigning episodes.</div>{/if}</div>{#if safeHttpUrl(searchState.detail.sourceUrl)}<a class="button secondary" href={safeHttpUrl(searchState.detail.sourceUrl) ?? undefined} target="_blank" rel="noreferrer"><ExternalLink size={15} /> Open source</a>{/if}</div>
        {#if currentSeriesYearMismatch()}
          {@const mismatch = currentSeriesYearMismatch()}
          <div class="metadata-episode-order-note metadata-series-year-warning" role="alert">
            <strong>Check the series year before renaming</strong>
            <span>{mismatch?.text}. This may be a different series or a different listing year.</span>
            <label><input type="checkbox" bind:checked={organizerState.seriesYearConflictConfirmed} onclick={() => { renameState.renamePreview = null; renameState.confirmRename = false; }} /> I checked the mismatch and want to use this record for TV renaming.</label>
          </div>
        {/if}
        <div class="metadata-facts">
          {#if searchState.detail.certification}<div><small>RATING</small><strong>{searchState.detail.certification}</strong></div>{/if}
          {#if searchState.detail.runtime}<div><small>RUNTIME</small><strong>{searchState.detail.runtime} min</strong></div>{/if}
          {#if searchState.detail.rating !== null}<div><small>USER RATING</small><strong>{searchState.detail.rating.toFixed(1)} / 10</strong></div>{/if}
          {#if searchState.mediaKind === "tv"}<div><small>SEASONS</small><strong>{searchState.detail.seasons.length}</strong></div><div><small>EPISODES</small><strong>{searchState.detail.episodes.length}</strong></div>{/if}
        </div>
        {#if searchState.mediaKind === "tv" && organizerState.organizerScan && localDirectoryAssets(organizerState.organizerScan.root)?.artwork.length}
          {@const localShowArt = localDirectoryAssets(organizerState.organizerScan.root)?.artwork ?? []}
          <section class="metadata-local-artwork-section">
            <div class="metadata-artwork-heading"><h3>Artwork already beside this series</h3><small>Choose a local image to use as the show poster. The original image stays where it is.</small></div>
            <div class="metadata-local-artwork-grid">
              {#each localShowArt.slice(0, 4) as image (image.path)}
                <figure>
                  {#if localAssetsState.localArtworkPreviews[image.path]}<img src={localAssetsState.localArtworkPreviews[image.path]} alt={`${image.role}: ${image.name}`} />{:else if localAssetsState.localArtworkPreviewBusy[image.path]}<span>Loading preview…</span>{:else}<button class="link-button" onclick={() => loadLocalArtworkPreviews([image])}>Load preview</button>{/if}
                  <button type="button" class="metadata-artwork-choice" class:selected={seriesAssignments[currentReviewedSeriesRoot()]?.localPosterPath === image.path} onclick={() => chooseSeriesLocalPoster(image.path)}>{seriesAssignments[currentReviewedSeriesRoot()]?.localPosterPath === image.path ? "Clear local poster" : "Use as series poster"}</button>
                  <figcaption>{image.role} · {image.name}</figcaption>
                </figure>
              {/each}
            </div>
          </section>
        {/if}
        <div class="metadata-edit-fields">
          <label>Short description
            <input bind:value={searchState.detail.shortDescription} oninput={() => { renameState.renamePreview = null; }} placeholder="Short summary, tagline, or subtitle" />
          </label>
          <label>Long description
            <textarea bind:value={searchState.detail.longDescription} rows="5" oninput={() => { renameState.renamePreview = null; }} placeholder="Long synopsis or plot"></textarea>
          </label>
          <small>{searchState.mediaKind === "movie" ? "These values are copied when you assign this film to selected files. You can then edit each film's text, rating, and poster separately." : "Review and edit the series text used for tagging. Episode descriptions are edited on their individual file assignments below."} A later confirmed rebuild embeds supported fields into the new Matroska file.</small>
        </div>
        {#if searchState.detail.artwork.length}
          <div class="metadata-artwork-heading"><h3>{searchState.mediaKind === "movie" ? "Poster options" : "Series poster options"}</h3><small>Select a preferred poster after checking its source and reuse terms.</small></div>
          <div class="metadata-artwork-grid">
            {#each searchState.detail.artwork as image (image.id)}
              <article class="metadata-artwork" class:selected={organizerState.selectedArtwork[searchState.mediaKind === "movie" ? "movie" : "series"] === image.id}>
                <a href={safeHttpUrl(image.sourceUrl) ?? undefined} target="_blank" rel="noreferrer"><img src={image.thumbnailUrl} alt={image.title} loading="lazy" /></a>
                <div><strong>{image.title}</strong><small>{image.attribution ?? image.provider}{#if image.license} · {image.license}{/if}</small>{#if image.licenseUrl}<a href={safeHttpUrl(image.licenseUrl) ?? undefined} target="_blank" rel="noreferrer">Review image license</a>{/if}</div>
                <button class="metadata-artwork-choice" disabled={artworkSaveState.artworkBusy} onclick={() => searchState.mediaKind === "tv" ? chooseSeriesProviderPoster(image.id) : (organizerState.selectedArtwork = { ...organizerState.selectedArtwork, movie: image.id }, artworkSaveState.pendingArtworkSave = null, artworkSaveState.artworkMessage = "")}>
                  {organizerState.selectedArtwork[searchState.mediaKind === "movie" ? "movie" : "series"] === image.id ? "Selected" : searchState.mediaKind === "movie" ? "Use film poster" : "Use series poster"}
                </button>
              </article>
            {/each}
          </div>
        {/if}
        {#if organizerState.organizerScan && searchState.mediaKind === "movie" && selectedMovieArtwork()}
          <div class="metadata-artwork-save-actions">
            <button class="button secondary" disabled={!movieArtworkTargets().length || artworkSaveState.artworkBusy} onclick={stageCurrentFilmPoster}>Review poster destinations for matching files</button>
            {#if selectedPathsInOrganizer().length && !movieArtworkTargets().length}<small class="metadata-warning">Assign this reviewed title to the selected movie files before saving its poster.</small>{/if}
          </div>
        {:else if organizerState.organizerScan && searchState.mediaKind === "tv"}
          {@const seriesRoot = organizerState.organizerScan.root}
          {@const seriesArt = selectedSeriesArtwork()}
          {#if seriesArt}
            {@const seriesExtension = artworkFileExtension(seriesArt)}
            <div class="metadata-artwork-save-actions"><button class="button secondary" disabled={artworkSaveState.artworkBusy} onclick={() => stageArtworkSave(seriesArt, seriesArtworkTargets(seriesRoot), "series poster for Kodi, Jellyfin, and Plex", [{ kind: "series", seriesRoot, candidateId: seriesArt.id }, null])}>Review series poster destinations</button><small>Copies will use <code>folder.{seriesExtension}</code> and <code>poster.{seriesExtension}</code> at the series root, with one source and license record for each. The saved folder image will be used in rebuilds.</small></div>
          {/if}
        {/if}
        {#if searchState.detail.seasons.length}
          <section class="metadata-season-list"><h3>Series seasons</h3>{#each searchState.detail.seasons as season (season.number)}
            <div class="metadata-season-row"><strong>{season.title ?? `Season ${season.number}`}</strong><span>{season.yearStart ?? "Dates unknown"}</span>
              {#if season.artwork.length}<div class="metadata-season-artwork">{#each season.artwork as image (image.id)}
                <button class:selected={organizerState.selectedArtwork[`season:${season.number}`] === image.id} disabled={artworkSaveState.artworkBusy} title={`Choose ${image.title} for ${season.title ?? `Season ${season.number}`}`} onclick={() => chooseSeriesProviderSeasonPoster(season.number ?? 0, image.id)}>
                  <img src={image.thumbnailUrl} alt={image.title} loading="lazy" />{organizerState.selectedArtwork[`season:${season.number}`] === image.id ? "Selected" : "Use season poster"}
                </button>
              {/each}</div>{/if}
              {#if organizerState.organizerScan && season.number !== null}
                {@const seasonNumber = season.number}
                {@const seasonRoot = organizerState.organizerScan.root}
                {@const seasonDirectory = organizerState.organizerScan.seasonDirectories.find((item) => item.season === seasonNumber)}
                {@const chosenSeasonPoster = selectedSeasonArtwork(seasonNumber)}
                {#if seasonDirectory && localDirectoryAssets(seasonDirectory.path)?.artwork.length}
                  {@const localSeasonArt = localDirectoryAssets(seasonDirectory.path)?.artwork ?? []}
                  <details class="metadata-local-artwork" ontoggle={(event) => event.currentTarget.open && loadLocalArtworkPreviews(localSeasonArt)}>
                    <summary>Local season artwork · {localSeasonArt.length} image{localSeasonArt.length === 1 ? "" : "s"}</summary>
                    <div class="metadata-local-artwork-grid">
                      {#each localSeasonArt.slice(0, 4) as image (image.path)}
                        <figure>
                          {#if localAssetsState.localArtworkPreviews[image.path]}<img src={localAssetsState.localArtworkPreviews[image.path]} alt={`${image.role}: ${image.name}`} />{:else if localAssetsState.localArtworkPreviewBusy[image.path]}<span>Loading preview…</span>{:else}<button class="link-button" onclick={() => loadLocalArtworkPreviews([image])}>Load preview</button>{/if}
                          <button type="button" class="metadata-artwork-choice" class:selected={seriesAssignments[currentReviewedSeriesRoot()]?.localSeasonPosterPaths[String(seasonNumber)] === image.path} onclick={() => chooseSeriesLocalSeasonPoster(seasonNumber, image.path)}>{seriesAssignments[currentReviewedSeriesRoot()]?.localSeasonPosterPaths[String(seasonNumber)] === image.path ? "Clear season poster" : `Use for Season ${String(seasonNumber).padStart(2, "0")}`}</button>
                          <figcaption>{image.role} · {image.name}</figcaption>
                        </figure>
                      {/each}
                    </div>
                  </details>
                {/if}
                {#if seasonDirectory && chosenSeasonPoster}<button class="link-button" disabled={artworkSaveState.artworkBusy} onclick={() => stageArtworkSave(chosenSeasonPoster, seasonArtworkTargets(seasonRoot, seasonDirectory.path, seasonNumber), `season poster for Kodi, Jellyfin, and Plex (Season ${String(seasonNumber).padStart(2, "0")})`, [{ kind: "season", seriesRoot: seasonRoot, seasonNumber, candidateId: chosenSeasonPoster.id }, null, null])}>Save season poster</button>{/if}
              {/if}
            </div>
          {/each}</section>
        {/if}
        {#if searchState.detail.episodes.length}
          <details class="metadata-episodes"><summary>Episode metadata · {searchState.detail.episodes.length} entries</summary><div>{#each searchState.detail.episodes as episode (String(episode.season) + "-" + episode.episode)}<article><strong>S{String(episode.season).padStart(2, "0")}E{String(episode.episode).padStart(2, "0")} · {episode.title}</strong><small>{episode.airdate ?? "Air date unknown"}{#if episode.summary} · {episode.summary}{/if}</small><a href={safeHttpUrl(episode.sourceUrl) ?? undefined} target="_blank" rel="noreferrer">Open source</a></article>{/each}</div></details>
        {/if}

        <section class="metadata-organizer" class:compact-item-organizer={compactItemMode}>
          <div class="metadata-artwork-heading"><h3>{searchState.mediaKind === "movie" ? "Review film filenames" : "Organize this series"}</h3><small>{organizerState.organizerSource === "analysis" ? "Using the media files from your current analysis. Metadata writes use this selected film or series association." : "Folder scans stay local. Nothing is renamed until you inspect the before/after list and confirm."}</small></div>
          {#if organizerState.organizerSource === "analysis" && searchState.mediaKind === "tv" && organizerState.organizerScan}
            <button class="button secondary" disabled={renameState.renameBusy || artworkSaveState.artworkBusy} onclick={scanCurrentSeriesFolder}><Film size={16} /> Scan complete series folder for full rename</button>
          {/if}
          <button class="button secondary" disabled={renameState.renameBusy || artworkSaveState.artworkBusy} onclick={chooseOrganizerFolder}>
            {#if renameState.renameBusy}<LoaderCircle size={16} class="spin" /> Scanning{:else}<Film size={16} /> {organizerState.organizerSource === "analysis" ? "Choose a different folder" : searchState.mediaKind === "movie" ? "Choose movie folder" : "Choose one series folder"}{/if}
          </button>
          {#if searchState.mediaKind === "tv" && !organizerState.organizerScan && scanResult}
            <div class="metadata-empty"><strong>{organizerState.selectedAnalysisSeriesRoot ? "Choose a show-specific folder" : "No series folder selected from the analysis."}</strong><span>{organizerState.selectedAnalysisSeriesRoot ? "A flat or shared folder cannot safely hold one show's metadata. Select a show folder with its own season or episode files." : "Choose a detected series folder above, or select a series folder to scan manually."}</span></div>
          {/if}
          {#if organizerState.organizerScan}
          <div class="metadata-folder-summary"><strong>{organizerState.organizerScan.root}</strong><span>{organizerState.organizerScan.files.length} video files · {organizerState.organizerScan.seasonDirectories.length} season folders</span>
              <div><button class="link-button" onclick={() => selectAllMedia(true)}>{compactItemMode ? searchState.mediaKind === "movie" ? "Select this film" : "Select this episode" : searchState.mediaKind === "movie" ? "Select these files" : "Select supported episodes"}</button><button class="link-button" onclick={() => selectAllMedia(false)}>Clear selection here</button><small>{selectedPathsInOrganizer().length} selected here · shared with Media library</small>{#if searchState.mediaKind === "tv"}<button class="link-button" disabled={!derived.desktop} onclick={searchCurrentSeriesRecord}><Search size={14} /> Search another series title</button>{/if}</div>
              {#if searchState.mediaKind === "tv" && organizerState.organizerScan.files.some((file) => file.multipleEpisodes)}<small class="metadata-warning">Multi-episode files stay out of this rename plan because one filename cannot safely represent several episode matches.</small>{/if}
              {#if searchState.mediaKind === "tv" && organizerState.organizerScan.episodeGaps.length}<small class="metadata-warning">Possible gaps in the filename numbering: {#each organizerState.organizerScan.episodeGaps as gap, index (index)}{#if index > 0} · {/if}Season {gap.season}: {gap.examples.map((episode) => String(episode).padStart(2, "0")).join(", ")}{#if gap.missingCount > gap.examples.length} and {gap.missingCount - gap.examples.length} more{/if}{/each}. Check against the episodes you expect; this filename check cannot identify specials or alternate episode orders.</small>{/if}
              {#if organizerState.organizerScan.truncated}<small class="metadata-warning">The 10,000-file scan limit was reached; series-wide rename is disabled.</small>{/if}
            {#if organizerState.organizerScan.errors.length}<small class="metadata-warning">{organizerState.organizerScan.errors.length} entries could not be read. Resolve those before a series-wide rename.</small>{/if}
            </div>
            {#if artworkSaveState.pendingArtworkSave}
              <section class="metadata-artwork-save-review">
                <div><strong>{artworkSaveState.pendingArtworkSave.saveBesideMedia ? `Save ${artworkSaveState.pendingArtworkSave.label} beside the media?` : `Save ${artworkSaveState.pendingArtworkSave.label} to the app cache?`}</strong><span>{artworkSaveState.pendingArtworkSave.saveBesideMedia ? "Poster and source/license files will be placed beside the media. Existing files are never overwritten." : "The poster and source/license details will stay in the private app artwork cache."} The selected image is attached when you rebuild the media into a new Matroska file if artwork embedding is enabled in Settings.</span></div>
                <div class="metadata-artwork-save-paths">{#each plannedArtworkPaths() as path (path)}<code>{path}</code><code>{path}.attribution.json</code>{/each}</div>
                <div class="metadata-rename-actions"><button class="button secondary" disabled={artworkSaveState.artworkBusy} onclick={() => artworkSaveState.pendingArtworkSave = null}>Cancel</button><button class="button primary" disabled={artworkSaveState.artworkBusy} onclick={applyArtworkSave}>{#if artworkSaveState.artworkBusy}<LoaderCircle size={16} class="spin" /> Downloading poster{:else if artworkSaveState.pendingArtworkSave.saveBesideMedia}Save poster files{:else}Save to app cache{/if}</button></div>
              </section>
            {/if}
            {#if artworkSaveState.artworkError}<div class="metadata-error"><TriangleAlert size={17} /><span>{artworkSaveState.artworkError}</span></div>{/if}
            {#if artworkSaveState.artworkMessage}<div class="metadata-success"><strong>Poster saved</strong><span>{artworkSaveState.artworkMessage}</span></div>{/if}
            {#if searchState.mediaKind === "movie"}
              <div class="metadata-movie-assign-actions">
                <span>Assign a separate reviewed match to each film. Current title details and the selected poster copy into chosen files; edit any file independently below.</span>
                <button class="button secondary" disabled={!searchState.detail || !selectedPathsInOrganizer().length || renameState.renameBusy} onclick={() => assignCurrentMovie(selectedPathsInOrganizer())}>Copy current match to selected files ({selectedPathsInOrganizer().length})</button>
              </div>
              <label class="metadata-template-label">Film pattern
                <input bind:value={organizerTemplates.movie} oninput={() => renameState.renamePreview = null} onchange={saveOrganizerTemplates} />
                <small>Shortcodes: {`{title}`}, {`{year}`}, {`{certification}`}, {`{ext}`}. Optional parts use [[ ... ]].</small>
              </label>
            {:else}
              <div class="metadata-rename-scope" role="radiogroup" aria-label="TV rename scope">
                <label class:selected={organizerState.tvRenameScope === "series-tree"}>
                  <input type="radio" name="tv-rename-scope" checked={organizerState.tvRenameScope === "series-tree"} disabled={organizerState.organizerSource === "analysis"} onchange={() => { organizerState.tvRenameScope = "series-tree"; renameState.renamePreview = null; renameState.confirmRename = false; }} />
                  <span><strong>Organize the full series</strong><small>{organizerState.organizerSource === "analysis" ? "Scan the complete series folder first; the library analysis may omit files below its size filter." : "Rename the series folder, season folders, and every selected supported episode. The scan must be complete and all supported episodes must be selected and matched."}</small></span>
                </label>
                <label class:selected={organizerState.tvRenameScope === "episodes-only"}>
                  <input type="radio" name="tv-rename-scope" checked={organizerState.tvRenameScope === "episodes-only"} onchange={() => { organizerState.tvRenameScope = "episodes-only"; renameState.renamePreview = null; renameState.confirmRename = false; }} />
                  <span><strong>Rename selected episode files only</strong><small>Keep series and season folders exactly as they are. Select one episode for an individual rename or several for a batch.</small></span>
                </label>
              </div>
              <div class="metadata-template-grid">
                <label class="metadata-template-label">Series folder pattern<input bind:value={organizerTemplates.series} oninput={() => renameState.renamePreview = null} onchange={saveOrganizerTemplates} /><small>{`{series_title}`}, {`{year_range}`}, {`{first_year}`}, {`{last_year}`}</small></label>
                <label class="metadata-template-label">Season folder pattern<input bind:value={organizerTemplates.season} oninput={() => renameState.renamePreview = null} onchange={saveOrganizerTemplates} /><small>{`{season}`}, {`{season:02}`}</small></label>
                <label class="metadata-template-label">Episode filename pattern<input bind:value={organizerTemplates.episode} oninput={() => renameState.renamePreview = null} onchange={saveOrganizerTemplates} /><small>{`{season:02}`}, {`{episode:02}`}, {`{episode_title}`}, {`{ext}`}</small></label>
              </div>
            {/if}
            <div class="metadata-organizer-files">
              {#each derived.visibleOrganizerFiles as file (file.path)}
                {@const checked = selectedMediaPaths.includes(file.path)}
                {@const assigned = episodeForKey(organizerState.episodeAssignments[file.path])}
                <article>
                  <label class="metadata-file-check"><input type="checkbox" checked={checked} disabled={searchState.mediaKind === "tv" && file.multipleEpisodes} onchange={(event) => toggleMedia(file.path, event.currentTarget.checked)} /><span><strong>{file.name}</strong><small>{file.parentPath}</small></span></label>
                  {#if searchState.mediaKind === "movie"}<button class="link-button" disabled={!derived.desktop} onclick={() => searchMovieRecord(file, movieAssignments[file.path]?.title)}><Search size={14} /> Search this film title</button>{/if}
                  {#if searchState.mediaKind === "tv"}
                    {#if file.multipleEpisodes}<span class="metadata-warning">Multi-episode file needs manual split handling</span>{:else}
                      <select aria-label={`Match episode for ${file.name}`} value={organizerState.episodeAssignments[file.path] ?? ""} onchange={(event) => { organizerState.episodeAssignments = { ...organizerState.episodeAssignments, [file.path]: event.currentTarget.value }; renameState.renamePreview = null; }}>
                        <option value="">Choose episode…</option>{#each searchState.detail.episodes as episode (String(episode.season) + "/" + episode.episode)}
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
                  {#if searchState.mediaKind === "movie"}
                    {#if movieAssignments[file.path]}
                      {@const assignment = movieAssignments[file.path]}
                      {@const poster = assignedMoviePoster(assignment)}
                      <div class="metadata-movie-assignment">
                        <strong>Assigned: {assignment.title}{assignment.year ? ` (${assignment.year})` : ""}</strong>
                        <label>Title<input bind:value={movieAssignments[file.path].title} oninput={() => { renameState.renamePreview = null; }} /></label>
                        <div><label>Year<input bind:value={movieAssignments[file.path].year} inputmode="numeric" oninput={() => { renameState.renamePreview = null; }} /></label><label>Certification<input bind:value={movieAssignments[file.path].certification} oninput={() => { renameState.renamePreview = null; }} /></label></div>
                        <details class="metadata-movie-extra"><summary>Edit descriptions, genres, rating and poster</summary>
                          <label>Short description<input bind:value={movieAssignments[file.path].shortDescription} /></label>
                          <label>Long description<textarea rows="4" bind:value={movieAssignments[file.path].plot}></textarea></label>
                          <div><label>Genres<input bind:value={movieAssignments[file.path].genresText} /><small>Separate genres with commas or semicolons.</small></label><label>Rating (0–10)<input type="number" min="0" max="10" step="0.1" bind:value={movieAssignments[file.path].ratingText} /></label></div>
                          <label>Poster option
                            <select aria-label={`Poster for ${file.name}`} disabled={artworkSaveState.artworkBusy} bind:value={movieAssignments[file.path].posterId} onchange={() => { chooseMovieProviderPoster(file.path); }}>
                              <option value="">No poster selected</option>
                              {#each assignment.artworkCandidates as image (image.id)}<option value={image.id}>{image.provider} · {image.title}{image.license ? ` · ${image.license}` : ""}</option>{/each}
                            </select>
                          </label>
                          {#if poster}
                            <div class="metadata-assigned-poster"><img src={poster.thumbnailUrl} alt={`Selected poster: ${poster.title}`} loading="lazy" /><span><strong>{poster.title}</strong><small>{poster.attribution ?? poster.provider}{#if poster.license} · {poster.license}{/if}</small></span></div>
                            <button class="link-button" disabled={artworkSaveState.artworkBusy} onclick={() => stageArtworkSave(poster, [movieArtworkTarget(file)], `poster for ${assignment.title}`, [{ kind: "movie", mediaPath: file.path, candidateId: poster.id }])}>Save poster for this film</button>
                          {:else}<small class="metadata-assignment">No poster selected for this file. Choose an option above or continue without artwork.</small>{/if}
                        </details>
                      </div>
                    {:else}<small class="metadata-warning">Assign a reviewed movie match before previewing this file.</small>{/if}
                    <button class="link-button" disabled={!searchState.detail || renameState.renameBusy} onclick={() => assignCurrentMovie([file.path])}>{movieAssignments[file.path] ? "Replace assignment with current match" : "Assign current match to this file"}</button>
                  {/if}
                </article>
              {/each}
            </div>
            {#if !compactItemMode}
              <section class="metadata-mkv-action" aria-label="Create a metadata-only MKV copy">
                <div>
                  <span class="eyebrow">NO VIDEO ENCODING</span>
                  <h3>Create a tagged MKV copy</h3>
                  <p>Use the reviewed title, metadata and selected poster in a new MKV. Source streams are copied and checked; the original stays in place. Apply the filename changes above first if you want the new copy to use those names.</p>
                  <small>{selectedPathsInOrganizer().length} file{selectedPathsInOrganizer().length === 1 ? "" : "s"} selected in this organizer · destination and collisions are reviewed before the copy starts</small>
                </div>
                <button class="button primary" type="button" disabled={!derived.desktop || renameState.renameBusy || !selectedPathsInOrganizer().length} onclick={createMetadataOnlyCopies}>
                  <FileText size={16} /> Review MKV output plan <ArrowRight size={15} />
                </button>
              </section>
            {/if}
            <div class="metadata-rename-actions">
              <button class="button secondary" disabled={renameState.renameBusy || !selectedPathsInOrganizer().length && searchState.mediaKind === "movie" || searchState.mediaKind === "movie" && selectedPathsInOrganizer().some((path) => !movieAssignments[path]) || Boolean(currentSeriesYearMismatch() && !organizerState.seriesYearConflictConfirmed)} onclick={previewNames}>Preview before/after</button>
              {#if renameState.renamePreview?.canApply}<button class="button primary" disabled={renameState.renameBusy} onclick={() => renameState.confirmRename = true}>Apply reviewed rename</button>{/if}
            </div>
            {#if renameState.renameError}<div class="metadata-error"><TriangleAlert size={17} /><span>{renameState.renameError}</span></div>{/if}
            {#if renameState.renamePreview}
              <div class="metadata-rename-preview">
                <div><strong>Before → after</strong><span>{renameState.renamePreview.items.length} planned paths · {renameState.renamePreview.conflicts.length} conflicts</span></div>
                <div class="metadata-rename-table"><div><strong>Current path</strong><strong>After confirmation</strong></div>
                  {#each renameState.renamePreview.items as item (item.sourcePath)}<div class:unchanged={item.unchanged}><span>{item.sourcePath}</span><span>{item.targetPath}{#if item.unchanged}<small>Unchanged</small>{/if}</span></div>{/each}
                </div>
                {#if renameState.renamePreview.conflicts.length}<div class="metadata-conflicts"><strong>Rename blocked · resolve every duplicate or occupied destination</strong>{#each renameState.renamePreview.conflicts as conflict, index (index)}<p>{conflict.reason}<small>{conflict.targetPath}</small></p>{/each}</div>
                {:else if !renameState.renamePreview.canApply}<small>Every selected name is already correct; no changes will be made.</small>{/if}
              </div>
            {/if}
            {#if renameState.confirmRename && renameState.renamePreview?.canApply}
              <div class="metadata-rename-confirm"><strong>Apply this rename batch?</strong><span>The utility will recheck the preview and collisions before changing names. Media contents remain untouched; recognized local artwork and attribution, plus same-base subtitle and external audio files follow their video. An operation error triggers rollback.</span>
                <div><button class="button secondary" disabled={renameState.renameBusy} onclick={() => renameState.confirmRename = false}>Review again</button><button class="button danger" disabled={renameState.renameBusy} onclick={applyNames}>{renameState.renameBusy ? "Applying…" : "Confirm rename"}</button></div>
              </div>
            {/if}
          {/if}
        </section>
      {:else}
        <div class="metadata-empty"><Database size={25} /><strong>Source details appear here</strong><span>Search a title and select a likely match. Nothing is applied to files automatically.</span></div>
      {/if}
    </section>
