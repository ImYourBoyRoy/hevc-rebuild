<script lang="ts">
  import { ArrowRight, CheckCircle2, LoaderCircle, ScanSearch, X } from '@lucide/svelte';
  import Modal from './Modal.svelte';
  import type { MediaEntry, MediaQueueRunStatus } from './types.js';
  import type { RecoveryReviewItem } from './ui-contracts.js';
  interface Props {
    recoveryReviewOpen: boolean;
    recoveryCurrentItem: RecoveryReviewItem | null;
    recoveryReviewIndex: number;
    itemCount: number;
    recoveryReportStatus: string;
    recoveryProbeEntry: MediaEntry | null;
    recoveryProbeError: string;
    recoveryProbeAttempted: boolean;
    desktopShell: boolean;
    recoveryProbeBusy: boolean;
    workflowBusy: boolean;
    recoveryAcknowledged: boolean;
    recoveryCanMarkComplete: boolean;
    queueRunStatusLabel: (status: MediaQueueRunStatus) => string;
    inspectRecoveryTarget: () => unknown;
    finishRecoveryReview: (completed?: boolean) => unknown;
    onClose: () => void;
    returnFocus?: HTMLElement | null;
  }
  let { recoveryReviewOpen, recoveryCurrentItem, recoveryReviewIndex, itemCount, recoveryReportStatus, recoveryProbeEntry, recoveryProbeError, recoveryProbeAttempted, desktopShell, recoveryProbeBusy, workflowBusy, recoveryAcknowledged = $bindable(), recoveryCanMarkComplete, queueRunStatusLabel, inspectRecoveryTarget, finishRecoveryReview, onClose, returnFocus = null }: Props = $props();
</script>
<!-- Shared queue-recovery and confirm-modal CSS preserves acknowledgment gates. Inspection remains distinct from encoder validation. -->
{#if recoveryReviewOpen && recoveryCurrentItem}
  <Modal open={recoveryReviewOpen} class="confirm-modal queue-recovery-modal" labelId="recovery-review-title" {returnFocus} onClose={onClose}>
      <button class="icon-button modal-close" aria-label="Close recovery review" onclick={onClose}><X size={18} /></button>
      <div class="eyebrow">RECOVERY REVIEW · {recoveryReviewIndex + 1} OF {itemCount}</div>
      <h2 id="recovery-review-title">Check this file before another run</h2>
      <p>An earlier run ended without a saved per-file result. The app will not resume it automatically. Review the report and probe the recorded output (or the source path when no output was recorded). A readable file alone does not prove the rebuild passed validation.</p>

      <div class="queue-recovery-file">
        <strong>{recoveryCurrentItem.entry.name}</strong>
        <code title={recoveryCurrentItem.targetPath}>{recoveryCurrentItem.targetPath}</code>
        <small>{queueRunStatusLabel(recoveryCurrentItem.state.status)}{recoveryCurrentItem.state.mode ? ` · ${recoveryCurrentItem.state.mode.toUpperCase()}` : ""} · source {recoveryCurrentItem.entry.status}</small>
      </div>

      <section class="queue-recovery-evidence" aria-label="Last run report">
        <div class="queue-recovery-section-title"><strong>Last report</strong><span class="status-tag" class:status-ready={recoveryReportStatus === "OK" || recoveryReportStatus === "METADATA_OK" || recoveryReportStatus === "TEST_OK"} class:status-error={recoveryReportStatus === "FAILED"}>{recoveryReportStatus || "NOT RECORDED"}</span></div>
        {#if recoveryCurrentItem.state.reportPath}<code class="queue-recovery-report-path">{recoveryCurrentItem.state.reportPath}</code>{/if}
        {#if recoveryCurrentItem.report}
          <p>{recoveryCurrentItem.report.message || "The report has no additional message for this file."}</p>
          {#if recoveryCurrentItem.report.output}<small>Reported output · {recoveryCurrentItem.report.output}</small>{/if}
          {#if recoveryCurrentItem.report.resolution || recoveryCurrentItem.report.primary_video}<small>{[recoveryCurrentItem.report.resolution, recoveryCurrentItem.report.primary_video].filter(Boolean).join(" · ")}</small>{/if}
        {:else}
          <p>No matching per-file result is available from the saved report. Check the report history or output folder before deciding what to do.</p>
        {/if}
        {#if recoveryCurrentItem.state.message}<small>Queue note · {recoveryCurrentItem.state.message}</small>{/if}
      </section>

      <div class="queue-recovery-inspection">
        <div class="queue-recovery-section-title"><strong>Output check</strong>{#if recoveryProbeEntry}<span class="status-tag" class:status-ready={recoveryProbeEntry.status === "Ready"} class:status-error={recoveryProbeEntry.status !== "Ready"}>{recoveryProbeEntry.status}</span>{/if}</div>
        {#if recoveryProbeEntry?.profile}
          <p>{recoveryProbeEntry.profile.video.width} × {recoveryProbeEntry.profile.video.height} · {recoveryProbeEntry.profile.video.source_bit_depth}-bit · {recoveryProbeEntry.profile.video.codec ?? "video codec unknown"}{recoveryProbeEntry.profile.hdr.dolby_vision ? " · Dolby Vision" : recoveryProbeEntry.profile.hdr.hdr10plus ? " · HDR10+" : recoveryProbeEntry.profile.hdr.hdr10 ? " · HDR10" : recoveryProbeEntry.profile.hdr.hlg ? " · HLG" : " · SDR"}</p>
          <small>{recoveryProbeEntry.profile.tracks.length} streams · {recoveryProbeEntry.profile.chapters_count} chapters · {(recoveryProbeEntry.size_bytes / (1024 ** 3)).toFixed(2)} GiB</small>
          {#if recoveryProbeEntry.error}<small class="row-error">{recoveryProbeEntry.error}</small>{/if}
        {:else if recoveryProbeError}
          <p class="row-error">{recoveryProbeError}</p>
        {:else if recoveryProbeAttempted}
          <p>No media profile was returned for this path.</p>
        {:else}
          <p>Probe the existing file to confirm FFprobe can read its container and streams.</p>
        {/if}
        <button class="button secondary" disabled={!desktopShell || recoveryProbeBusy || workflowBusy} onclick={inspectRecoveryTarget}>{#if recoveryProbeBusy}<LoaderCircle class="spin" size={15} /> Checking file…{:else}<ScanSearch size={15} /> {recoveryProbeAttempted ? "Check again" : "Inspect recorded target"}{/if}</button>
      </div>

      <label class="confirm-check queue-recovery-ack"><input type="checkbox" bind:checked={recoveryAcknowledged} disabled={!recoveryProbeAttempted || recoveryProbeBusy} /><span>I reviewed the report and file check above. I understand this inspection does not substitute for the encoder’s validation report.</span></label>
      <div class="modal-actions queue-recovery-actions">
        <button class="button secondary" onclick={onClose}>Cancel</button>
        {#if recoveryCanMarkComplete}<button class="button secondary" disabled={!recoveryAcknowledged || recoveryProbeBusy} onclick={() => finishRecoveryReview(true)}><CheckCircle2 size={15} /> Record as complete</button>{/if}
        <button class="button primary" disabled={!recoveryAcknowledged || !recoveryProbeAttempted || recoveryProbeBusy} onclick={() => finishRecoveryReview(false)}>{recoveryReviewIndex + 1 < itemCount ? "Review next file" : "Continue to output review"}<ArrowRight size={15} /></button>
      </div>
  </Modal>
{/if}