<script lang="ts">
  import { AlertTriangle, CheckCircle2, LoaderCircle } from '@lucide/svelte';
  import type { RepairToolOutcome, ToolSetupLogSummary } from './ui-contracts.js';
  interface Props {
    showLogHistory: boolean;
    repairSummary: string;
    repairSummaryIsError: boolean;
    repairId: string;
    hdrInstalling: boolean;
    repairToolOutcomes: RepairToolOutcome[];
    repairLogWarning: string;
    repairLines: string[];
    toolSetupLogs: ToolSetupLogSummary[];
    selectedToolSetupLog: string;
    toolSetupLogLoading: boolean;
    viewedToolSetupLog: string;
    toolSetupLogExporting: boolean;
    toolSetupLogError: string;
    openToolSetupLog: () => unknown;
    exportToolSetupLog: () => unknown;
  }
  let { showLogHistory, repairSummary, repairSummaryIsError, repairId, hdrInstalling, repairToolOutcomes, repairLogWarning, repairLines, toolSetupLogs, selectedToolSetupLog = $bindable(), toolSetupLogLoading, viewedToolSetupLog = $bindable(), toolSetupLogExporting, toolSetupLogError, openToolSetupLog, exportToolSetupLog }: Props = $props();
</script>
<!-- app.css scopes repair-activity, repair-tool, repair-output and repair-log families. -->
  {#if repairSummary}
    <div class="repair-activity-summary" class:error={repairSummaryIsError} role="status" aria-live="polite">
      {#if repairId || hdrInstalling}<LoaderCircle size={16} class="spin" />{:else if repairSummaryIsError}<AlertTriangle size={16} />{:else}<CheckCircle2 size={16} />{/if}
      <strong>{repairSummary}</strong>
    </div>
  {/if}
  {#if repairToolOutcomes.length}
    <details class="repair-tool-results">
      <summary>Tool-by-tool recheck · {repairToolOutcomes.length} executable{repairToolOutcomes.length === 1 ? "" : "s"}</summary>
      <div class="repair-tool-result-list">{#each repairToolOutcomes as outcome (outcome.key)}
        <article><div><strong>{outcome.label}</strong><span class:success={outcome.status === "Installed" || outcome.status === "Updated or re-resolved"} class:missing={outcome.status === "Still missing"}>{outcome.status}</span></div><small>Before · {outcome.before}</small><small>After · {outcome.after}</small></article>
      {/each}</div>
    </details>
  {/if}
  {#if repairLogWarning}<p class="repair-log-warning" role="status">{repairLogWarning}</p>{/if}
  {#if repairLines.length}
    <details class="repair-diagnostics">
      <summary>Live setup output · last {Math.min(repairLines.length, 120)} lines</summary>
      <pre class="repair-output">{repairLines.slice(-120).join("\n")}</pre>
    </details>
  {/if}
  {#if showLogHistory}<details class="repair-diagnostics repair-log-history">
    <summary>Saved setup logs · {toolSetupLogs.length}</summary>
    <div class="repair-log-history-body">
      <p class="repair-log-empty">Saved in this app’s state folder; URL credentials are redacted before storage.</p>
      {#if toolSetupLogs.length}
        <label class="repair-log-picker">Saved operation
          <select bind:value={selectedToolSetupLog} disabled={toolSetupLogLoading} onchange={() => viewedToolSetupLog = ""}>
            {#each toolSetupLogs as log (log.jobId)}
              <option value={log.jobId}>{new Date(log.modifiedUnix * 1000).toLocaleString()} · {log.sizeBytes >= 1048576 ? `${(log.sizeBytes / 1048576).toFixed(1)} MiB` : `${Math.max(1, Math.round(log.sizeBytes / 1024))} KiB`}{log.truncated ? " · size limit reached" : ""}</option>
            {/each}
          </select>
        </label>
        <div class="repair-log-actions">
          <button class="button secondary compact" type="button" disabled={!selectedToolSetupLog || toolSetupLogLoading} onclick={() => openToolSetupLog()}>{toolSetupLogLoading ? "Opening…" : "View complete log"}</button>
          <button class="button secondary compact" type="button" disabled={!selectedToolSetupLog || toolSetupLogExporting} onclick={exportToolSetupLog}>{toolSetupLogExporting ? "Exporting…" : "Export log…"}</button>
        </div>
        {#if toolSetupLogs.find((log) => log.jobId === selectedToolSetupLog)?.truncated}<p class="repair-log-warning">This operation exceeded 16 MiB. The saved log marks where output was truncated.</p>{/if}
        {#if viewedToolSetupLog && selectedToolSetupLog}<pre class="repair-output repair-full-output">{viewedToolSetupLog}</pre>{/if}
      {:else}<p class="repair-log-empty">No package or helper setup logs have been saved on this device yet.</p>{/if}
      {#if toolSetupLogError}<p class="repair-log-warning" role="alert">{toolSetupLogError}</p>{/if}
    </div>
  </details>{/if}
