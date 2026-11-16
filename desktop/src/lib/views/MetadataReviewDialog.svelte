<!-- ./desktop/src/lib/views/MetadataReviewDialog.svelte
     Compact media-file/series/season/episode review dialog around the metadata
     workbench and LibraryTvReview. Pure view: the route owns all review state;
     every edit is a callback. -->
<script lang="ts">
  import type { Snippet } from "svelte";
  import { ArrowRight, X } from "@lucide/svelte";
  import LibraryTvReview from "../LibraryTvReview.svelte";
  import Modal from "../Modal.svelte";
  import type { AppPreferences } from "../ui-contracts.js";
  import type { LibrarySeriesGroup } from "../library-browse.js";
  import type { MediaEntry, QueueMetadataSuggestion, QueueMovieReview, QueueSeriesReview, QueueTvEpisode, ScanResult, Settings } from "../types.js";
  import { proposedFilenameForEntry, reviewDestinationPath } from "../filename-template.js";
  import {
    certificateNoteFor, commentNoteFor, ownCertificateFor, ownCommentFor, seasonPlotNote, seasonPremiereYear, seasonRatingNote, sharedCertificateFor,
    type LibraryTvReviewTarget,
  } from "../review-notes.js";

  interface Props {
    open: boolean;
    metadataDialogElement: HTMLDialogElement | null;
    metadataWorkbenchSurface: Snippet<[boolean]>;
    scanResult: ScanResult | null;
    metadataFocusPath: string | null;
    libraryTvReview: LibraryTvReviewTarget | null;
    appPreferences: AppPreferences;
    settings: Settings | null;
    queueMovieReviews: Record<string, QueueMovieReview>;
    queueSeriesReviews: Record<string, QueueSeriesReview>;
    queueMatchSuggestions: Record<string, QueueMetadataSuggestion>;
    reviewedSeriesForPath: (path: string) => QueueSeriesReview | undefined;
    seriesReviewStorageKey: (root: string) => string;
    seriesGroupForRoot: (root: string) => LibrarySeriesGroup | undefined;
    episodeDraft: (path: string) => QueueTvEpisode;
    tvReviewHeading: (target: LibraryTvReviewTarget) => string;
    tvReviewPaths: (target: LibraryTvReviewTarget) => string[];
    animationSelection: (paths: string[]) => "all" | "some" | "none" | string;
    activeReviewPosterPath: (target: LibraryTvReviewTarget, review: QueueSeriesReview | undefined) => string;
    displayPosterUrl: (path: string) => string | null;
    reviewPosterChoices: (group: LibrarySeriesGroup, season: number | null, review: QueueSeriesReview | undefined) => { path: string; url: string }[];
    sharedCommentFor: (target: LibraryTvReviewTarget, review: QueueSeriesReview) => string;
    reviewMediaEntries: (target: LibraryTvReviewTarget) => MediaEntry[];
    reviewLocationPath: (target: LibraryTvReviewTarget | null, fallback: string) => string;
    mediaInfoLine: (entry: MediaEntry) => string;
    formatMetadataValue: (value: unknown) => string;
    patchTvField: (target: LibraryTvReviewTarget, field: string, value: string) => unknown;
    setReviewComment: (target: LibraryTvReviewTarget, value: string) => unknown;
    selectReviewPoster: (target: LibraryTvReviewTarget, path: string) => unknown;
    chooseReviewImage: () => unknown;
    setPathsAnimated: (paths: string[], animated: boolean) => unknown;
    setMovieComment: (path: string, value: string) => unknown;
    openFileLocation: (path: string) => unknown;
    openFullMetadataOrganizer: () => unknown;
    onClose: () => void;
  }
  let {
    open, metadataDialogElement = $bindable(), metadataWorkbenchSurface, scanResult, metadataFocusPath, libraryTvReview, appPreferences, settings, queueMovieReviews,
    queueSeriesReviews, queueMatchSuggestions, reviewedSeriesForPath, seriesReviewStorageKey, seriesGroupForRoot, episodeDraft, tvReviewHeading, tvReviewPaths,
    animationSelection, activeReviewPosterPath, displayPosterUrl, reviewPosterChoices, sharedCommentFor, reviewMediaEntries, reviewLocationPath, mediaInfoLine,
    formatMetadataValue, patchTvField, setReviewComment, selectReviewPoster, chooseReviewImage, setPathsAnimated, setMovieComment, openFileLocation,
    openFullMetadataOrganizer, onClose,
  }: Props = $props();
</script>

{#if open && scanResult?.entries.find((entry) => entry.path === metadataFocusPath)}
  {@const focusedEntry = scanResult.entries.find((entry) => entry.path === metadataFocusPath)!}
  {@const focusedMovie = queueMovieReviews[focusedEntry.path]}
  {@const focusedSeries = reviewedSeriesForPath(focusedEntry.path)}
  {@const focusedSuggestion = queueMatchSuggestions[focusedEntry.path]}
  {@const focusedFilename = proposedFilenameForEntry(focusedEntry, focusedMovie, focusedSeries, focusedSuggestion, appPreferences.organizerTemplates)}
  {@const focusedProposedName = focusedFilename?.name ?? focusedEntry.name}
  {@const focusedDestination = reviewDestinationPath(focusedEntry, focusedProposedName, appPreferences)}
  {@const focusedFolder = focusedEntry.path.slice(0, Math.max(focusedEntry.path.lastIndexOf("/"), focusedEntry.path.lastIndexOf("\\")))}
  <Modal {open} class="metadata-review-dialog" bind:element={metadataDialogElement} labelId="metadata-review-dialog-title" closeOnBackdrop={true} onClose={onClose} initialFocus=".metadata-review-dialog-header button">
    <header class="metadata-review-dialog-header">
      <div class="metadata-review-dialog-title">
        <span class="eyebrow">{libraryTvReview?.level === "series" ? "SERIES REVIEW" : libraryTvReview?.level === "season" ? "SEASON REVIEW" : libraryTvReview ? "EPISODE REVIEW" : "MEDIA FILE REVIEW"}</span>
        <h2 id="metadata-review-dialog-title">{libraryTvReview ? (tvReviewHeading(libraryTvReview) || focusedEntry.name) : (focusedMovie?.title || focusedSeries?.title || focusedSuggestion?.title || focusedEntry.name)}</h2>
        {#if libraryTvReview?.level === "series" || libraryTvReview?.level === "season"}
          <p class="metadata-review-stay">{libraryTvReview.level === "series" ? libraryTvReview.seriesRoot.split(/[\\/]/u).filter(Boolean).at(-1) : `${queueSeriesReviews[seriesReviewStorageKey(libraryTvReview.seriesRoot)]?.title || "Series"} · season ${libraryTvReview.season}`}</p>
        {:else}
        <div class="metadata-review-filenames">
          {#if focusedProposedName === focusedEntry.name}
            <div><span>Name</span><strong>{focusedEntry.name}</strong></div>
          {:else}
            <div><span>Original name</span><strong>{focusedEntry.name}</strong></div>
            <ArrowRight class="review-name-arrow" size={16} />
            <div><span>New name</span><strong>{focusedProposedName}</strong></div>
          {/if}
          {#if focusedDestination.path === focusedEntry.path}
            <p class="metadata-review-stay">Stays in {focusedFolder}</p>
          {:else}
            <p class="metadata-review-move">{focusedDestination.note} <code>{focusedDestination.path}</code></p>
          {/if}
        </div>
        {/if}
      </div>
      <button class="icon-button" type="button" aria-label="Close media file review" title="Close" onclick={onClose}><X size={18} /></button>
    </header>
    <div class="metadata-review-dialog-content">
      {#if libraryTvReview}
        {@const target = libraryTvReview}
        {@const review = queueSeriesReviews[seriesReviewStorageKey(target.seriesRoot)]}
        {@const seasonKey = target.season === null ? "" : String(target.season)}
        {@const seasonRecord = seasonKey ? review?.seasonRecords?.[seasonKey] : undefined}
        {@const episodeRecord = target.episodePath ? (review?.episodeAssignments?.[target.episodePath] ?? episodeDraft(target.episodePath)) : undefined}
        {@const group = seriesGroupForRoot(target.seriesRoot)}
        {@const posterPath = activeReviewPosterPath(target, review)}
        {@const markedPaths = tvReviewPaths(target)}
        {@const marked = animationSelection(markedPaths)}
        {@const premiereYear = seasonPremiereYear(review, target.season)}
        {@const seasonYear = seasonRecord?.year && seasonRecord.year !== review.year ? seasonRecord.year : (premiereYear || seasonRecord?.year || "")}
        {#if review && (target.level !== "season" || seasonRecord) && (target.level !== "episode" || target.episodePath)}
          <LibraryTvReview
            level={target.level}
            kicker={target.level === "series" ? "" : (review.title || "")}
            title={target.level === "season" ? (seasonRecord?.title ?? "") : target.level === "episode" ? (episodeRecord?.title ?? "") : review.title}
            year={target.level === "season" ? seasonYear : review.year}
            certification={target.level === "series" ? review.certification : (ownCertificateFor(target, review) || sharedCertificateFor(target, review))}
            certificationNote={certificateNoteFor(target, review)}
            certificationCustom={Boolean(ownCertificateFor(target, review))}
            plot={target.level === "season" ? (seasonRecord?.plot.trim() || review.plot) : target.level === "episode" ? (episodeRecord?.summary ?? "") : review.plot}
            plotNote={target.level === "season" ? seasonPlotNote(review, seasonRecord) : ""}
            plotCustom={target.level === "season" && Boolean(seasonRecord?.plot.trim())}
            genresText={target.level === "season" ? (seasonRecord?.genresText ?? "") : review.genresText}
            ratingText={target.level === "season" ? (seasonRecord?.ratingText.trim() || review.ratingText) : review.ratingText}
            ratingNote={target.level === "season" ? seasonRatingNote(review, seasonRecord) : ""}
            ratingCustom={target.level === "season" && Boolean(seasonRecord?.ratingText.trim())}
            seasonNumber={target.level === "episode" ? String(episodeRecord?.season ?? "") : ""}
            episodeNumber={target.level === "episode" ? String(episodeRecord?.episode ?? "") : ""}
            airdate={episodeRecord?.airdate ?? ""}
            comment={review ? (ownCommentFor(target, review) || sharedCommentFor(target, review)) : (settings?.metadata.comment ?? "")}
            commentNote={review ? commentNoteFor(target, review) : "Using the comment from Settings."}
            commentCustom={review ? Boolean(ownCommentFor(target, review)) : false}
            posterUrl={displayPosterUrl(posterPath) ?? ""}
            posters={group ? reviewPosterChoices(group, target.level === "series" ? null : target.season, review) : []}
            selectedPosterPath={posterPath}
            animated={marked === "all"}
            animatedMixed={marked === "some"}
            onTitle={(value: string) => patchTvField(target, "title", value)}
            onYear={(value: string) => patchTvField(target, "year", value)}
            onCertification={(value: string) => patchTvField(target, "certification", value)}
            onUseSharedCertification={() => patchTvField(target, "certification", "")}
            onPlot={(value: string) => patchTvField(target, "plot", value)}
            onUseSharedPlot={() => patchTvField(target, "plot", "")}
            onGenres={(value: string) => patchTvField(target, "genresText", value)}
            onRating={(value: string) => patchTvField(target, "ratingText", value)}
            onUseSharedRating={() => patchTvField(target, "ratingText", "")}
            onSeasonNumber={(value: string) => patchTvField(target, "season", value)}
            onEpisodeNumber={(value: string) => patchTvField(target, "episode", value)}
            onAirdate={(value: string) => patchTvField(target, "airdate", value)}
            onComment={(value: string) => setReviewComment(target, value)}
            onUseSharedComment={() => setReviewComment(target, "")}
            onSelectPoster={(path: string) => selectReviewPoster(target, path)}
            onChooseImage={() => void chooseReviewImage()}
            onAnimated={(animated: boolean) => setPathsAnimated(markedPaths, animated)}
          />
        {:else}
          <p class="review-status">Preparing this review…</p>
        {/if}
      {:else}
        {@render metadataWorkbenchSurface(true)}
        {#if focusedEntry && queueMovieReviews[focusedEntry.path]}
          {@const movieComment = queueMovieReviews[focusedEntry.path].comment?.trim() ?? ""}
          <div class="review-fields review-movie-comment">
            <label>Comment<input maxlength="500" value={movieComment || settings?.metadata.comment || ""} oninput={(event) => setMovieComment(focusedEntry.path, event.currentTarget.value)} /></label>
            {#if movieComment}<p class="review-note"><button class="text-button" type="button" onclick={() => setMovieComment(focusedEntry.path, "")}>Use the shared comment</button></p>{/if}
          </div>
        {/if}
      {/if}
      {#if focusedEntry}
      {@const mediaEntries = libraryTvReview ? reviewMediaEntries(libraryTvReview) : [focusedEntry]}
      {#if mediaEntries.length}
        <details class="review-more review-media-info">
          <summary>Media info · {mediaEntries.length} file{mediaEntries.length === 1 ? "" : "s"}</summary>
          {#each mediaEntries as entry (entry.path)}
            <details class="review-media-file" open={mediaEntries.length === 1}>
              <summary><strong>{entry.name}</strong><small>{mediaInfoLine(entry)}</small></summary>
              <div class="review-media-facts">
                <button class="text-button" type="button" onclick={() => void openFileLocation(entry.path)}>Open file location</button>
                {#if entry.profile}
                  {#each entry.profile.tracks as track (track.index)}
                    <p>{track.type} · {track.codec || "Unknown codec"}{track.language ? ` · ${track.language}` : ""}{track.title ? ` · ${track.title}` : ""}{track.width && track.height ? ` · ${track.width}×${track.height}` : ""}{track.channels ? ` · ${track.channels} ch` : ""}</p>
                  {/each}
                  {#if Object.keys(entry.profile.mediainfo_general ?? {}).length}
                    <dl class="track-metadata">{#each Object.entries(entry.profile.mediainfo_general) as [key, value] (key)}<div><dt>{key.replaceAll("_", " ")}</dt><dd>{formatMetadataValue(value)}</dd></div>{/each}</dl>
                  {/if}
                {:else}
                  <p>This file has no scanned media info yet. Refresh the source and open the review again.</p>
                {/if}
              </div>
            </details>
          {/each}
        </details>
      {/if}
      {/if}
    </div>
    <footer class="metadata-review-dialog-footer">
      <span>{libraryTvReview ? "Edits are saved as you type. Done closes this review." : "Choose a poster and confirm the title. Batch rename stays in the organizer."}</span>
      <div><button class="button secondary" type="button" onclick={() => void openFileLocation(reviewLocationPath(libraryTvReview, focusedEntry.path))}>Open file location</button><button class="button secondary" type="button" onclick={openFullMetadataOrganizer}>Open full organizer <ArrowRight size={15} /></button><button class="button primary" type="button" onclick={onClose}>Done</button></div>
    </footer>
  </Modal>
{/if}
