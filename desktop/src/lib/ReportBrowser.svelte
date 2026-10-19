<script lang="ts">
  import { ArrowRight, ChevronRight, FileText, History, Info, RefreshCw, TerminalSquare } from '@lucide/svelte';
  import type { Page, ReportRow, RunReport } from './types.js';
  import { metadataTextChanges } from './report-view.js';
  interface Props {
    repairId: string;
    hdrInstalling: boolean;
    reportDeleteBusy: boolean;
    reportDeleteConfirm: boolean;
    failedRows: ReportRow[];
    loadReports: () => unknown;
    deleteReportHistory: () => unknown;
    reports: RunReport[];
    activeReport: RunReport | null;
    logLoading: boolean;
    selectedLog: string;
    hasScan: boolean;
    fileName: (path: string) => string;
    bytes: (value: number | null | undefined) => string;
    isFailure: (status: string) => boolean;
    chooseReport: (path: string) => unknown;
    openLog: (path: string | null) => unknown;
    copyLog: () => unknown;
    openPage: (page: Page) => void;
  }
  let { repairId, hdrInstalling, reportDeleteBusy, reportDeleteConfirm = $bindable(), failedRows, loadReports, deleteReportHistory, reports, activeReport, logLoading, selectedLog, hasScan, fileName, bytes, isFailure, chooseReport, openLog, copyLog, openPage }: Props = $props();
</script>
<!-- app.css scopes report and diagnostic families. This view never scans, mutates a report, or resumes a job. -->
        <section class="page-heading">
          <div><div class="eyebrow">AUDIT TRAIL</div><h1>Reports & <em>errors.</em></h1><p>Review completed runs, per-file outcomes, and diagnostic logs from the media engine.</p></div>
          <div class="heading-actions">
            <button class="button secondary" disabled={Boolean(repairId) || hdrInstalling || reportDeleteBusy} onclick={loadReports}><RefreshCw size={16} /> Refresh history</button>
            {#if reportDeleteConfirm}
              <button class="button danger-outline" type="button" disabled={reportDeleteBusy} onclick={deleteReportHistory}>{reportDeleteBusy ? "Deleting…" : "Delete permanently"}</button>
              <button class="button secondary" type="button" disabled={reportDeleteBusy} onclick={() => (reportDeleteConfirm = false)}>Cancel</button>
            {:else}
              <button class="button secondary" type="button" disabled={!reports.length || reportDeleteBusy} onclick={() => (reportDeleteConfirm = true)}>Delete history</button>
            {/if}
          </div>
        </section>
        {#if failedRows.length}
          <section class="report-failures" aria-label="Failed results">
            <h2>{failedRows.length} failed result{failedRows.length === 1 ? "" : "s"}</h2>
            <ul>
              {#each failedRows as row, index (`${row.input}:${index}`)}
                <li>
                  <strong>{fileName(row.input || "Unknown file")}</strong>
                  <span>{row.status || "Failed"}</span>
                  <p>{row.message || "No detail was saved for this result."}</p>
                </li>
              {/each}
            </ul>
          </section>
        {/if}
        {#if reports.length}
          <section class="reports-layout">
            <aside class="report-list card-surface"><div class="report-list-heading"><div class="eyebrow">RECENT RUNS</div><span>{reports.length} reports</span></div>
              {#each reports as report (report.csv_path)}
                {@const reportName = fileName(report.csv_path).replace(/\.csv$/i, "")}
                <button class="report-item" class:active={activeReport?.csv_path === report.csv_path} onclick={() => chooseReport(report.csv_path)}><span class="report-item-icon" class:failed={Object.entries(report.counts ?? {}).some(([status, count]) => count > 0 && isFailure(status))}><FileText size={17} /></span><span class="report-item-copy"><strong>{reportName}</strong><small>{new Date(report.created * 1000).toLocaleString()}</small></span><ChevronRight size={15} /></button>
              {/each}
            </aside>
            {#if activeReport}
              <section class="report-detail">
                <div class="report-heading card-surface"><div><div class="eyebrow">RUN REPORT</div><h2>{fileName(activeReport.csv_path).replace(/\.csv$/i, "")}</h2><p>{new Date(activeReport.created * 1000).toLocaleString()}</p></div><button class="button secondary" onclick={() => openLog(activeReport.log_path)}><TerminalSquare size={16} /> {logLoading ? "Loading…" : "Open diagnostics"}</button></div>
                {#if activeReport.rows.some((row) => row.hdr10plus_note)}<div class="inline-warning"><Info size={17} /><div><strong>HDR10+ metadata loss was approved for some YouTube copies.</strong><span>Only the individually approved files were exported without dynamic metadata; static HDR10/PQ signaling is retained. Other HDR10+ files are skipped.</span></div></div>{/if}
                {#if activeReport.rows.some((row) => row.subtitle_note)}<div class="inline-warning"><Info size={17} /><div><strong>Source subtitle streams are not embedded in YouTube upload copies.</strong><span>The affected rows are identified in the CSV report; add supported caption files separately through YouTube.</span></div></div>{/if}
                <div class="report-counts">{#each Object.entries(activeReport.counts) as [status, count] (status)}<div class="report-count"><span class:bad-count={isFailure(status)}>{status}</span><strong>{count}</strong></div>{/each}</div>
                <div class="table-card report-table"><div class="table-header"><div><div class="eyebrow">FILE OUTCOMES</div><h2>{activeReport.rows.length} entries</h2></div></div>
                  <div class="table-scroll"><table><thead><tr><th>INPUT</th><th>RESULT</th><th>DETAIL</th><th>SIZE</th></tr></thead><tbody>
                    {#each activeReport.rows as row, rowIndex (row.input + rowIndex)}<tr><td><strong class="file-cell-title">{fileName(row.input)}</strong><small class="file-cell-sub">{row.resolution ?? row.primary_video ?? "Media file"}</small>{#if row.content_tune}<small class="file-cell-sub">Content tune: {row.content_tune}</small>{/if}{#if row.output_resolution}<small class="file-cell-sub">Output: {row.output_resolution}</small>{/if}</td><td><span class="status-tag" class:status-error={isFailure(row.status)} class:status-ready={!isFailure(row.status)}>{row.status}</span></td><td class="detail-cell" title={row.memory_forecast_message ?? ""}>{row.message || row.dolby_vision_action || "—"}{#if row.estimated_output_bytes || row.output_bytes}<small class="file-cell-sub">YouTube size: estimated {row.estimated_output_bytes ? bytes(Number(row.estimated_output_bytes)) : "—"} · actual {row.output_bytes ? bytes(Number(row.output_bytes)) : "pending"}{row.upload_video_bitrate_kbps ? ` · ${row.upload_video_bitrate_kbps} kb/s video` : ""}{row.upload_source_multiplier ? ` · ${row.upload_source_multiplier}× source` : ""}{row.youtube_recommended_video_bitrate_kbps ? ` · YouTube guide ${row.youtube_recommended_video_bitrate_kbps} kb/s` : ""}</small>{/if}{#if row.calibration_source}<small class="file-cell-sub">Calibration: {row.calibration_source}{row.calibration_workers ? ` · ${row.calibration_workers} workers / ${row.calibration_frame_threads ?? "—"} frame threads` : ""}{Number(row.calibration_candidate_count) > 0 ? ` · ${row.calibration_candidate_count} candidate(s)` : ""}{row.calibration_note ? ` · ${row.calibration_note}` : ""}</small>{/if}{#if row.memory_forecast}<small class="file-cell-sub">Memory: {row.memory_forecast.toUpperCase()} · {row.planned_workers ?? "—"} workers · {row.planned_frame_threads ?? "—"} frame threads</small>{/if}{#if row.min_memory_available_gib}<small class="file-cell-sub">Min available RAM: {Number(row.min_memory_available_gib).toFixed(1)} GiB</small>{/if}{#if row.max_encoder_rss_gib}<small class="file-cell-sub">Peak encoder RSS: {Number(row.max_encoder_rss_gib).toFixed(2)} GiB</small>{/if}{#if row.max_swap_used_percent || row.max_encoder_swap_gib || row.max_swap_in_mib_s || row.max_swap_out_mib_s}<small class="file-cell-sub">Peak swap: {row.max_swap_used_percent ? `${Number(row.max_swap_used_percent).toFixed(0)}% system used` : "usage unknown"}{row.max_encoder_swap_gib ? ` · FFmpeg process ${Number(row.max_encoder_swap_gib).toFixed(2)} GiB` : ""} · in {row.max_swap_in_mib_s ? `${Number(row.max_swap_in_mib_s).toFixed(1)} MiB/s` : "—"} · out {row.max_swap_out_mib_s ? `${Number(row.max_swap_out_mib_s).toFixed(1)} MiB/s` : "—"}{row.max_memory_full_psi_percent ? ` · memory stalls ${Number(row.max_memory_full_psi_percent).toFixed(1)}%` : ""}</small>{/if}</td><td>{row.input_bytes ? bytes(Number(row.input_bytes)) : "—"}</td></tr>{/each}
                  </tbody></table></div>
                  {#if activeReport.rows.length === 0}<div class="empty-state compact"><div><strong>This report has no readable rows.</strong><p>The CSV may be incomplete; inspect the companion log for context.</p></div></div>{/if}
                </div>
                {#if activeReport.rows.some((row) => Number(row.metadata_text_scrub_count ?? row.release_group_scrub_count) > 0)}
                  <section class="report-scrub-card card-surface">
                    <div class="report-scrub-heading"><div><div class="eyebrow">METADATA CLEANUP</div><h3>Text removed from metadata</h3><p>Only matching text was removed. Technical encoder and stream data remain unchanged.</p></div></div>
                    {#each activeReport.rows.filter((row) => Number(row.metadata_text_scrub_count ?? row.release_group_scrub_count) > 0) as row (row.input)}
                      {@const changes = metadataTextChanges(row)}
                      <details class="report-scrub-file">
                        <summary><span>{fileName(row.input)}</span><small>{row.metadata_text_scrub_count ?? row.release_group_scrub_count} field(s)</small></summary>
                        {#if changes.length}
                          <ul>{#each changes as change, changeIndex (`${row.input}-${changeIndex}`)}
                            <li><small>{change.scope ?? "metadata"} · {change.field ?? "text"}{change.rules?.length ? ` · ${change.rules.join(", ")}` : ""}</small><div><code>{change.before ?? ""}</code><span aria-hidden="true">→</span><code>{change.after || "(removed)"}</code></div></li>
                          {/each}</ul>
                        {:else}<p class="report-scrub-empty">Detailed values are available in the CSV report.</p>{/if}
                      </details>
                    {/each}
                  </section>
                {/if}
                <section class="diagnostic-card"><div class="diagnostic-heading"><div><div class="eyebrow">DIAGNOSTIC LOG</div><h3>Engine output</h3></div><button class="text-button" disabled={!selectedLog} onclick={copyLog}>Copy excerpt</button></div><pre>{selectedLog || "Select Open diagnostics to load the companion log. The viewer is limited to the latest 1 MB."}</pre></section>
              </section>
            {/if}
          </section>
        {:else}
          <div class="empty-state large"><div class="empty-icon"><History size={26} /></div><div><div class="eyebrow">NO RUN HISTORY YET</div><h2>Reports appear here after your first analysis or rebuild.</h2><p>File-level statuses and diagnostic logs are stored locally in your user state directory.</p></div><button class="button primary" onclick={() => openPage(hasScan ? "run" : "library")}>Go to workflow <ArrowRight size={16} /></button></div>
        {/if}
