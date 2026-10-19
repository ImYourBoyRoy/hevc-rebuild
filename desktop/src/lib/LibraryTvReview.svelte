<!-- ./desktop/src/lib/LibraryTvReview.svelte
     Poster-first review for one series, season, or episode.
     The parent owns the saved fields. This view edits title, plot, poster, and the animated mark. -->
<script lang="ts">
  import "../styles/components/metadata-review.css";
  import { Film, ImagePlus } from "@lucide/svelte";
  import type { Attachment } from "svelte/attachments";

  function indeterminate(value: boolean): Attachment<HTMLInputElement> {
    return (node) => {
      node.indeterminate = value;
    };
  }

  interface PosterChoice {
    path: string;
    url: string;
  }

  interface Props {
    level: "series" | "season" | "episode";
    kicker: string;
    title: string;
    year: string;
    certification: string;
    certificationNote: string;
    certificationCustom: boolean;
    plot: string;
    plotNote: string;
    plotCustom: boolean;
    genresText: string;
    ratingText: string;
    ratingNote: string;
    ratingCustom: boolean;
    seasonNumber: string;
    episodeNumber: string;
    airdate: string;
    comment: string;
    commentNote: string;
    commentCustom: boolean;
    posterUrl: string;
    posters: PosterChoice[];
    selectedPosterPath: string;
    animated: boolean;
    animatedMixed: boolean;
    onTitle: (value: string) => void;
    onYear: (value: string) => void;
    onCertification: (value: string) => void;
    onUseSharedCertification: () => void;
    onPlot: (value: string) => void;
    onUseSharedPlot: () => void;
    onGenres: (value: string) => void;
    onRating: (value: string) => void;
    onUseSharedRating: () => void;
    onSeasonNumber: (value: string) => void;
    onEpisodeNumber: (value: string) => void;
    onAirdate: (value: string) => void;
    onComment: (value: string) => void;
    onUseSharedComment: () => void;
    onSelectPoster: (path: string) => void;
    onChooseImage: () => void;
    onAnimated: (animated: boolean) => void;
  }

  let {
    level,
    kicker,
    title,
    year,
    certification,
    certificationNote,
    certificationCustom,
    plot,
    plotNote,
    plotCustom,
    genresText,
    ratingText,
    ratingNote,
    ratingCustom,
    seasonNumber,
    episodeNumber,
    airdate,
    comment,
    commentNote,
    commentCustom,
    posterUrl,
    posters,
    selectedPosterPath,
    animated,
    animatedMixed,
    onTitle,
    onYear,
    onCertification,
    onUseSharedCertification,
    onPlot,
    onUseSharedPlot,
    onGenres,
    onRating,
    onUseSharedRating,
    onSeasonNumber,
    onEpisodeNumber,
    onAirdate,
    onComment,
    onUseSharedComment,
    onSelectPoster,
    onChooseImage,
    onAnimated,
  }: Props = $props();

  let animatedHelp = $derived(
    animatedMixed
      ? "Some episodes are marked."
      : level === "episode"
        ? "Uses the animation encode tune."
        : level === "season"
          ? "Sets every episode in this season."
          : "Sets every episode in this series.",
  );
  let plotPlaceholder = $derived(
    level === "episode"
        ? "No episode description yet."
      : level === "season"
        ? "No season plot yet."
        : "",
  );
</script>

<section class="review-studio" aria-label={level === "series" ? "Review this series" : level === "season" ? "Review this season" : "Review this episode"}>
  <div class="review-poster-column">
    <figure class="review-poster-stage">
      {#if posterUrl}<img src={posterUrl} alt="" referrerpolicy="no-referrer" />{:else}<Film size={42} /><strong>No poster yet</strong>{/if}
    </figure>
    {#if posters.length}
      <div class="review-poster-rail" role="listbox" aria-label="Poster choices">
        {#each posters as image (image.path)}
          <button type="button" role="option" aria-selected={image.path === selectedPosterPath} class:is-selected={image.path === selectedPosterPath} onclick={() => onSelectPoster(image.path)}>
            {#if image.url}<img src={image.url} alt="" referrerpolicy="no-referrer" />{:else}<span>Poster</span>{/if}
          </button>
        {/each}
      </div>
    {/if}
    {#if level === "episode"}<p class="review-note">Episodes in this season share this poster.</p>{/if}
  </div>
  <div class="review-fields">
    {#if kicker}<p class="review-kicker">{kicker}</p>{/if}
    <label>Title<input value={title} oninput={(event) => onTitle(event.currentTarget.value)} /></label>
    {#if level === "episode"}
      <div class="review-field-row">
        <label>Season<input value={seasonNumber} inputmode="numeric" oninput={(event) => onSeasonNumber(event.currentTarget.value)} /></label>
        <label>Episode<input value={episodeNumber} inputmode="numeric" oninput={(event) => onEpisodeNumber(event.currentTarget.value)} /></label>
      </div>
      <label class="review-plot">Plot<textarea rows="8" placeholder={plotPlaceholder} value={plot} oninput={(event) => onPlot(event.currentTarget.value)}></textarea></label>
      <div class="review-field-row">
        <label>Air date<input placeholder="No air date yet." value={airdate} oninput={(event) => onAirdate(event.currentTarget.value)} /></label>
        <label>Certification<input placeholder="No certificate yet." value={certification} title="Content certificate, such as TV-PG or PG. This is not limited to the MPAA." oninput={(event) => onCertification(event.currentTarget.value)} /></label>
      </div>
      {#if certificationNote}<p class="review-note">{certificationNote}{#if certificationCustom} <button class="text-button" type="button" onclick={onUseSharedCertification}>Use the shared certificate</button>{/if}</p>{/if}
    {:else}
      <div class="review-field-row">
        <label>Year<input value={year} inputmode="numeric" oninput={(event) => onYear(event.currentTarget.value)} /></label>
        <label>Certification<input placeholder={level === "season" ? "No certificate yet." : ""} value={certification} title="Content certificate, such as TV-PG or PG. This is not limited to the MPAA." oninput={(event) => onCertification(event.currentTarget.value)} /></label>
      </div>
      {#if certificationNote}<p class="review-note">{certificationNote}{#if certificationCustom} <button class="text-button" type="button" onclick={onUseSharedCertification}>Use the shared certificate</button>{/if}</p>{/if}
      <label class="review-plot">Plot<textarea rows="8" placeholder={plotPlaceholder} value={plot} oninput={(event) => onPlot(event.currentTarget.value)}></textarea></label>
      {#if plotNote}<p class="review-note">{plotNote}{#if plotCustom} <button class="text-button" type="button" onclick={onUseSharedPlot}>Use the series description</button>{/if}</p>{/if}
      <div class="review-field-row">
        <label>Genres<input value={genresText} oninput={(event) => onGenres(event.currentTarget.value)} /></label>
        <label>Audience rating<input placeholder={level === "season" ? "No season score yet." : ""} value={ratingText} inputmode="decimal" title="Audience score from 0 to 10." oninput={(event) => onRating(event.currentTarget.value)} /></label>
      </div>
      {#if ratingNote}<p class="review-note">{ratingNote}{#if ratingCustom} <button class="text-button" type="button" onclick={onUseSharedRating}>Use the series score</button>{/if}</p>{/if}
    {/if}
    <label>Comment<input maxlength="500" value={comment} title="Matroska COMMENT written into a new file" oninput={(event) => onComment(event.currentTarget.value)} /></label>
    <p class="review-note">{commentNote}{#if commentCustom} <button class="text-button" type="button" onclick={onUseSharedComment}>Use the shared comment</button>{/if}</p>
    <label class="review-animated">
      <input
        type="checkbox"
        checked={animated}
        {@attach indeterminate(animatedMixed)}
        aria-label={level === "episode" ? "Animated encode for this episode" : "Animated encode for every episode here"}
        onchange={(event) => onAnimated(event.currentTarget.checked)}
      />
      <span>Animated<small>{animatedHelp}</small></span>
    </label>
  </div>
  <div class="review-poster-actions">
    <button class="button secondary" type="button" onclick={onChooseImage}><ImagePlus size={15} /> Use your own image</button>
  </div>
</section>
