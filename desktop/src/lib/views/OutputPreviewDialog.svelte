<!-- ./desktop/src/lib/views/OutputPreviewDialog.svelte
     Review-before-start dialog: destinations, collisions, disk budget and the
     rebuild acknowledgement. Pure view; the route owns the preview, the flags
     and the start action. -->
<script lang="ts">
  import { AlertTriangle, CheckCircle2, Cpu, Film, HardDrive, X } from "@lucide/svelte";
  import Modal from "../Modal.svelte";
  import type { SourceRunBatch } from "../run-controller.js";
  import type { MediaEntry, OutputPreview } from "../types.js";
  import { bytes } from "../format.js";
  import { archivedOriginalPath, fileName } from "../queue-paths.js";

  interface Props {
    open: boolean;
    fullAck: boolean;
    outputPreview: OutputPreview | null;
    outputPreviewBusy: boolean;
    canStart: boolean;
    runLaunchButton: HTMLButtonElement | null;
    runBatches: SourceRunBatch[];
    selectedRunEntries: MediaEntry[];
    selectedMediaPathsForRun: string[];
    selectedRunDiskBudget: { working: number; floored: boolean };
    selectedRunFreeBytes: number;
    onClose: () => void;
    adjustOutputFolders: () => void;
    onConfirm: () => unknown;
  }
  let {
    open, fullAck = $bindable(), outputPreview, outputPreviewBusy, canStart, runLaunchButton, runBatches, selectedRunEntries, selectedMediaPathsForRun,
    selectedRunDiskBudget, selectedRunFreeBytes, onClose, adjustOutputFolders, onConfirm,
  }: Props = $props();
</script>

{#if open && outputPreview}
  <Modal returnFocus={runLaunchButton} open={open} class="confirm-modal queue-output-modal" labelId="output-preview-title" onClose={onClose}>
      <button class="icon-button modal-close" aria-label="Close output review" onclick={onClose}><X size={18} /></button>
      <div class="eyebrow">REVIEW BEFORE START · {outputPreview.mode.toUpperCase()}</div>
      <h2 id="output-preview-title">{outputPreview.mode === "full" ? "Review destinations, then start the rebuild" : "Review where these files will go"}</h2>
      <p>{outputPreview.fileCount} file{outputPreview.fileCount === 1 ? "" : "s"} across {outputPreview.sourceCount} source{outputPreview.sourceCount === 1 ? "" : "s"}. Destinations, collisions, and disk space are checked here before anything is written.</p>
      <div class="confirm-facts"><span><Film size={16} /> {selectedRunEntries.length} {selectedMediaPathsForRun.length ? "selected" : "queued"} file{selectedRunEntries.length === 1 ? "" : "s"}</span><span class="confirm-disk"><HardDrive size={16} /> About {bytes(selectedRunDiskBudget.working)} of temporary space for {selectedRunEntries.length === 1 ? "this file" : "the largest file"}{#if selectedRunDiskBudget.floored}. The finished size is not known yet, so the budget stays at 2 GB even for a small file{/if}. {Number.isFinite(selectedRunFreeBytes) ? `${bytes(selectedRunFreeBytes)} is already free on ${runBatches.length === 1 ? "this volume" : "the tightest volume"}` : "Free space is checked again before the file starts"}.</span><span><Cpu size={16} /> CPU libx265 · one file at a time</span></div>
      {#if outputPreview.eligible}
        <div class="queue-output-status clear"><CheckCircle2 size={17} /><span>No existing-output or duplicate-path collisions were found.</span></div>
      {:else}
        <div class="queue-output-status blocked"><AlertTriangle size={17} /><span>{outputPreview.collisionCount} collision{outputPreview.collisionCount === 1 ? "" : "s"} need{outputPreview.collisionCount === 1 ? "s" : ""} a different output folder before this queue can start.</span></div>
      {/if}
      {#if outputPreview.mode === "full"}
        <div class="queue-output-note">The new file is encoded beside the original. After it passes validation, the original moves into a hidden <code>.hevc-rebuild-originals</code> folder beside it and the new file takes the current name. If validation fails, the original stays where it is.</div>
      {:else if outputPreview.mode === "metadata"}
        <div class="queue-output-note">Metadata-only mode creates a new MKV copy and keeps the original file in place.</div>
      {:else if outputPreview.testOutputsMayReceiveUniqueSuffixes}
        <div class="queue-output-note">Short-test outputs use a <code>Test_</code> prefix. If that name already exists, the engine adds a unique timestamp.</div>
      {/if}

      {#if outputPreview.collisions.length}
        <div class="queue-output-list-heading"><strong>Paths that block the run</strong>{#if outputPreview.omittedCollisionCount}<span>Showing {outputPreview.collisions.length} of {outputPreview.collisionCount}</span>{/if}</div>
        <div class="queue-output-list queue-output-conflicts">
          {#each outputPreview.collisions as collision, index (`${collision.source}:${collision.target}:${index}`)}
            <article><span>{collision.kind === "existing_output" ? "Already exists" : "Same destination"}</span><code title={collision.target}>{collision.target}</code><small>{fileName(collision.source)}{#if collision.conflictsWith} · also planned for {fileName(collision.conflictsWith)}{/if}</small></article>
          {/each}
        </div>
        {#if outputPreview.omittedCollisionCount}<small class="queue-output-truncated">{outputPreview.omittedCollisionCount} additional collision records are omitted from this view.</small>{/if}
      {/if}

      <div class="queue-output-list-heading"><strong>{outputPreview.targets.every((target) => target.replacesSource) ? "After a successful rebuild" : "Planned destinations"}</strong><span>{Math.min(outputPreview.targets.length, 250)}{#if outputPreview.omittedTargetCount} shown · {outputPreview.omittedTargetCount} more{/if}</span></div>
      <div class="queue-output-list">
        {#each outputPreview.targets as target (target.source)}
          <article>
            <span title={target.source}>{fileName(target.source)}</span>
            {#if target.replacesSource}
              <small>Original moves here</small>
              <code title={archivedOriginalPath(target.source)}>{archivedOriginalPath(target.source)}</code>
              <small>Rebuilt file keeps the current name</small>
              <code title={target.target}>{target.target}</code>
            {:else}
              <code title={target.target}>{target.target}</code>
              {#if target.note}<small>{target.note}</small>{/if}
            {/if}
          </article>
        {/each}
      </div>
      {#if outputPreview.omittedTargetCount}<small class="queue-output-truncated">The preview is capped at 250 rows. All {outputPreview.fileCount} files were checked for collisions.</small>{/if}

      {#if outputPreview.mode === "full"}
        <label class="confirm-check"><input type="checkbox" bind:checked={fullAck} /> I reviewed the destinations and understand this starts real encodes.</label>
      {/if}
      <div class="modal-actions">
        <button class="button secondary" onclick={onClose}>Back</button>
        {#if !outputPreview.eligible}<button class="button secondary" onclick={adjustOutputFolders}>Adjust output folders</button>{/if}
        <button class="button primary" disabled={!outputPreview.eligible || outputPreviewBusy || !canStart || (outputPreview.mode === "full" && !fullAck)} onclick={onConfirm}>{outputPreviewBusy ? "Checking paths…" : outputPreview.eligible ? (outputPreview.mode === "full" ? "Start full rebuild" : outputPreview.targets.every((target) => target.replacesSource) ? "Confirm and start" : "Confirm destinations and start") : "Resolve collisions first"}</button>
      </div>
  </Modal>
{/if}
