<script lang="ts">
  import type { RunState } from './workspace-state.js';
  import { finiteNumber } from './telemetry-format.js';
  interface Props {
    run: RunState; encodeMeasured: boolean; overallRunPercent: number | null; progressElapsedLabel: string;
    batchCount: number; batchIndex: number;
    bytes: (value: number | null | undefined) => string; duration: (seconds: number | undefined) => string;
    openProgressDetails: () => void; stopAfterCurrent: () => unknown;
  }
  let { run, encodeMeasured, overallRunPercent, progressElapsedLabel, batchCount, batchIndex, bytes, duration, openProgressDetails, stopAfterCurrent }: Props = $props();
</script>
<!-- Shared app.css owns the explicitly scoped .global-run-progress family, tokens and responsive rules. -->
    {#if run.id || run.starting}
      <section class="global-run-progress" aria-label="Workflow progress" aria-live="polite">
        <div class="global-run-progress-heading">
          <span class="global-run-indicator" class:running={Boolean(run.id)}></span>
          <div class="global-run-progress-title">
            <span>{run.starting ? "STARTING WORKFLOW" : "WORKFLOW IN PROGRESS"}</span>
            <strong>{encodeMeasured ? (run.progress?.fileName ?? run.message ?? "Preparing the selected files…") : (run.progress?.phase || run.message || "Preparing the selected files")}</strong>
            <small>
              {#if run.progress}
                {encodeMeasured ? run.progress.phase : run.progress.fileName} · file {run.progress.batchNumber} of {run.progress.batchTotal} in this source
              {:else}
                {run.message || "The engine is preparing the selected files."}
              {/if}
              {#if batchCount && batchIndex >= 0} · source {batchIndex + 1} of {batchCount}{/if}
            </small>
          </div>
        </div>
        <div class="global-run-progress-body">
          <div class="global-run-progress-metrics">
            {#if finiteNumber(run.progress?.inputBytes)}<span><small>INPUT</small><strong>{bytes(run.progress.inputBytes)}</strong></span>{/if}
            {#if finiteNumber(run.progress?.outputBytes)}<span><small>OUTPUT SO FAR</small><strong>{bytes(run.progress.outputBytes)}</strong></span>{/if}
            {#if finiteNumber(run.progress?.estimatedOutputBytes)}<span><small>LIVE SIZE ESTIMATE</small><strong>{bytes(run.progress.estimatedOutputBytes)}</strong></span>{/if}
            {#if finiteNumber(run.progress?.speed)}<span><small>ENCODE SPEED</small><strong>{run.progress.speed.toFixed(2)}×</strong></span>{/if}
            {#if finiteNumber(run.progress?.etaSeconds)}<span><small>ENCODE ETA</small><strong>{duration(run.progress.etaSeconds)}</strong></span>{/if}
            {#if finiteNumber(run.resources?.memory_available_gib)}<span><small>RAM AVAILABLE</small><strong>{run.resources.memory_available_gib.toFixed(1)} GiB</strong></span>{/if}
            {#if finiteNumber(run.resources?.swap_used_percent)}<span><small>SWAP USED</small><strong>{run.resources.swap_used_percent.toFixed(0)}%</strong></span>{/if}
          </div>
          <div class="global-run-progress-track" role="progressbar" aria-label="Selected queue progress" aria-valuemin="0" aria-valuemax="100" aria-valuenow={encodeMeasured ? overallRunPercent ?? undefined : undefined} aria-busy={!encodeMeasured}>
            <span class:indeterminate={!encodeMeasured} style={encodeMeasured ? `width:${overallRunPercent ?? 0}%` : ""}></span>
          </div>
          <div class="global-run-progress-footer">
            <small>{encodeMeasured ? `Selected queue · ${overallRunPercent?.toFixed(0) ?? 0}%` : "Preparing"}{encodeMeasured && finiteNumber(run.progress?.etaSeconds) ? ` · ${duration(run.progress.etaSeconds)} left` : ""} · {progressElapsedLabel} elapsed</small>
            <div>
              <button class="text-button" type="button" onclick={openProgressDetails}>Open progress details</button>
              {#if run.id}<button class="text-button" type="button" disabled={run.stopAfterCurrent} onclick={stopAfterCurrent}>{run.stopAfterCurrent ? "Stopping safely…" : "Stop after this file"}</button>{/if}
            </div>
          </div>
        </div>
      </section>
    {/if}

