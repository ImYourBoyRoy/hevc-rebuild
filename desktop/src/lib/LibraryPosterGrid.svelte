<!-- ./desktop/src/lib/LibraryPosterGrid.svelte
     Media library poster cards. Each queued file shows its poster, title, current name,
     and proposed name, with a matching indicator while metadata arrives. Selection,
     the Animated mark, and Rename / Metadata-only / Rebuild sit on the cards.
     Streams, memory, and output stay behind the card. -->
<script lang="ts">
  import { ChevronRight, Film, LoaderCircle } from "@lucide/svelte";
  import type { LibraryCrumb, LibraryPosterCard } from "./types.js";
  import type { Action } from "svelte/action";
  import type { Attachment } from "svelte/attachments";
  import "../styles/components/library-poster-grid.css";

  function indeterminate(value: boolean): Attachment<HTMLInputElement> {
    return (node) => {
      node.indeterminate = value;
    };
  }

  function trackPoster(): Attachment<HTMLImageElement> {
    return (node) => {
      const frame = node.parentElement;
      if (!frame) return;
      const finish = () => frame.classList.remove("is-downloading");
      frame.classList.add("is-downloading");
      if (node.complete && node.naturalWidth > 0) finish();
      node.addEventListener("load", finish);
      node.addEventListener("error", finish);
      return () => {
        node.removeEventListener("load", finish);
        node.removeEventListener("error", finish);
      };
    };
  }

  interface Props {
    cards: LibraryPosterCard[];
    busy: boolean;
    selectedOnPage: number;
    selectedTotal: number;
    readyCount: number;
    lookupMessage: string;
    lookupFailed: boolean;
    crumbs: LibraryCrumb[];
    reviewCurrentLabel: string;
    onReviewCurrent: (() => void) | null;
    canRename: boolean;
    canProcess: boolean;
    observeArtwork: Action<HTMLElement, { mediaPath: string; artworkPath: string }>;
    onToggleSelect: (paths: string[], selected: boolean) => void;
    onToggleAnimated: (paths: string[], animated: boolean) => void;
    onOpenReview: (path: string) => void;
    onOpenLocation: (path: string) => void;
    onReview: (path: string) => void;
    canAdjustRun: boolean;
    onAdjustRun: (paths: string[]) => void;
    onRename: () => void;
    onMetadata: () => void;
    onRebuild: () => void;
    onSetOutput: (path: string) => void;
    onClearOutput: (path: string) => void;
    onSelectPage: (selected: boolean) => void;
  }

  let {
    cards,
    busy,
    selectedOnPage,
    selectedTotal,
    readyCount,
    lookupMessage,
    lookupFailed,
    crumbs,
    reviewCurrentLabel,
    onReviewCurrent,
    canRename,
    canProcess,
    observeArtwork,
    onToggleSelect,
    onToggleAnimated,
    onOpenReview,
    onOpenLocation,
    onReview,
    canAdjustRun,
    onAdjustRun,
    onRename,
    onMetadata,
    onRebuild,
    onSetOutput,
    onClearOutput,
    onSelectPage,
  }: Props = $props();

  let groupAnimation = $derived.by(() => {
    if (!cards.length) return "none" as const;
    if (cards.every((card) => card.animation === "all")) return "all" as const;
    if (cards.every((card) => card.animation === "none")) return "none" as const;
    return "some" as const;
  });

</script>

<section class="library-poster-grid" aria-label="Queued media">
  <div class="library-poster-toolbar">
    {#if crumbs.length}
      <nav class="library-crumbs" aria-label="Library location">
        {#each crumbs as crumb, index (crumb.label)}
          {#if index > 0}<ChevronRight size={14} />{/if}
          {#if crumb.onSelect}
            <button class="text-button" type="button" onclick={crumb.onSelect}>{crumb.label}</button>
          {:else}
            <span>{crumb.label}</span>
          {/if}
        {/each}
      </nav>
    {/if}
    {#if onReviewCurrent}
      <button class="text-button library-review-current" type="button" onclick={onReviewCurrent}>{reviewCurrentLabel}</button>
    {/if}
    {#if crumbs.length}
      <label class="library-poster-select">
        <input
          type="checkbox"
          {@attach indeterminate(groupAnimation === "some")}
          aria-label="Animated encode for everything shown"
          checked={groupAnimation === "all"}
          disabled={busy || !cards.length}
          onchange={(event) => onToggleAnimated(cards.flatMap((card) => card.mediaPaths), event.currentTarget.checked)}
        />
        <span>Animated</span>
      </label>
    {/if}
    <label class="library-poster-select">
      <input
        type="checkbox"
        {@attach indeterminate(selectedOnPage > 0 && selectedOnPage < cards.length)}
        aria-label={`Select all ${cards.length} shown`}
        checked={cards.length > 0 && selectedOnPage === cards.length}
        disabled={busy || !cards.length}
        onchange={(event) => onSelectPage(event.currentTarget.checked)}
      />
      <span>{cards.length} shown</span>
    </label>
    {#if lookupMessage}
      <p class="library-poster-lookup" class:error={lookupFailed} role="status">
        {#if !lookupFailed}<LoaderCircle class="spin" size={16} />{/if}
        {lookupMessage}
      </p>
    {/if}
  </div>

  <div class="library-poster-cards">
    {#each cards as card (card.path)}
      <article class="library-poster-card" class:selected={card.selection === "all"}>
        <div class="library-poster-topline">
          <label class="library-poster-check">
            <input
              type="checkbox"
              {@attach indeterminate(card.selection === "some")}
              aria-label={`Select ${card.title}`}
              checked={card.selection === "all"}
              disabled={busy || (card.stale === "missing" && card.selection === "none")}
              onchange={(event) => onToggleSelect(card.mediaPaths, event.currentTarget.checked)}
            />
            <span>{card.matchState === "matching" ? "Matching" : card.selection === "all" ? "Selected" : "Select"}</span>
          </label>
          {#if card.matchLabel && card.matchState !== "local" && card.matchState !== "none" && card.matchState !== "matched"}
            <span class="library-poster-badge {card.matchState}">{card.matchLabel}</span>
          {/if}
        </div>

        <button
          class="library-poster-frame"
          type="button"
          aria-label={card.kind === "media" ? `Review title and poster for ${card.filename}` : `Open ${card.title}`}
          use:observeArtwork={{ mediaPath: card.mediaPaths[0] ?? card.path, artworkPath: card.localArtworkPath }}
          onclick={() => onOpenReview(card.path)}
        >
          {#if card.posterUrl}
            <img src={card.posterUrl} alt="" referrerpolicy="no-referrer" {@attach trackPoster()} />
            <span class="library-poster-download"><LoaderCircle class="spin" size={22} /> Downloading</span>
          {:else}
            <span class="library-poster-empty">
              {#if card.matchState === "matching" || card.matchState === "matched" || card.matchState === "suggested" || card.localArtworkPath}
                <LoaderCircle class="spin" size={28} />
                {card.matchState === "matching" ? "Matching" : "Downloading"}
              {:else}
                <Film size={28} />
                {card.matchLabel || "No poster yet"}
              {/if}
            </span>
          {/if}
          {#if card.progressLabel}
            <span class="library-poster-progress">{card.progressLabel}{card.progressPercent != null ? ` · ${card.progressPercent.toFixed(0)}%` : ""}</span>
          {/if}
        </button>

        <button class="library-poster-copy" type="button" onclick={() => onOpenReview(card.path)}>
          <strong title={card.title}>{card.title}{card.year ? ` (${card.year})` : ""}</strong>
          {#if card.summary}<small>{card.summary}</small>{/if}
          {#if card.kind === "media"}<small title={card.relativePath}>{card.filename}</small>{/if}
          {#if card.proposedName}
            <span class="library-poster-proposed" title={card.proposedName}>
              {card.proposedNeedsReview ? "Proposed, review first" : "Proposed"} · <code>{card.proposedName}</code>
            </span>
          {/if}
          {#if card.provider}<small>{card.provider}</small>{/if}
        </button>
        <button class="text-button library-poster-location" type="button" onclick={() => onOpenLocation(card.path)}>Open file location</button>

        {#if card.error}<p class="library-poster-note">{card.error}</p>{/if}
        {#if card.stale === "missing"}<p class="library-poster-note">This file is missing. Refresh the folder after you restore or remove it.</p>
        {:else if card.stale === "changed"}<p class="library-poster-note warning">This file changed after its scan. Refresh the folder before running it.</p>{/if}
        {#if card.warning}<p class="library-poster-note warning">{card.warning}</p>{/if}
        {#if card.runLabel}<small class="library-poster-proposed">{card.runLabel}</small>{/if}

        <label class="library-poster-animated">
          <input
            type="checkbox"
            {@attach indeterminate(card.animation === "some")}
            aria-label={card.kind === "media" ? `Animated encode for ${card.filename}` : `Animated encode for every episode in ${card.title}`}
            checked={card.animation === "all"}
            disabled={busy || !card.mediaPaths.length}
            onchange={(event) => onToggleAnimated(card.mediaPaths, event.currentTarget.checked)}
          />
          Animated
        </label>
        {#if card.reviewLabel}
          <button class="text-button library-poster-review" type="button" onclick={() => onReview(card.path)}>{card.reviewLabel}</button>
        {/if}
        {#if canAdjustRun && card.kind === "media" && card.selected}
          <button class="text-button library-poster-adjust" type="button" onclick={() => onAdjustRun(card.mediaPaths)}>Adjust this run</button>
        {/if}

        {#if card.kind === "media"}
        <details class="library-poster-details">
          <summary>Streams and output</summary>
          <p>{card.statusLabel} · {card.sourceBytes}</p>
          <p>{card.profileLine}</p>
          <p>{card.streamLine}</p>
          {#if card.memoryLine}<p>{card.memoryLine}</p>{/if}
          <small>{card.outputLine}</small>
          <div>
            <button class="text-button" type="button" disabled={busy} onclick={() => onSetOutput(card.path)}>Set output folder</button>
            {#if card.outputIsCustom}<button class="text-button" type="button" disabled={busy} onclick={() => onClearOutput(card.path)}>Use default</button>{/if}
          </div>
        </details>
        {/if}
      </article>
    {/each}
  </div>

  <div class="library-poster-actions">
    <p>
      {#if selectedTotal}
        {selectedTotal} selected · {readyCount} ready to process
      {:else}
        Nothing selected · {readyCount} ready file{readyCount === 1 ? "" : "s"} will be included
      {/if}
    </p>
    <button class="button secondary" type="button" disabled={busy || !canRename} onclick={onRename}>Rename files</button>
    <button class="button secondary" type="button" disabled={busy || !canProcess} onclick={onMetadata}>Metadata-only MKV</button>
    <button class="button primary" type="button" disabled={busy || !canProcess} onclick={onRebuild}>Rebuild video</button>
  </div>
</section>
