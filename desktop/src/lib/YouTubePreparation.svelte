<script lang="ts">
  import { AlertTriangle, ChevronRight, FolderOpen, Info, LoaderCircle, ScanSearch, Search, Square, Video, Wrench } from '@lucide/svelte';
  import type { Snippet } from 'svelte';
  import RunConsole from './RunConsole.svelte';
  import type { QueueState, RunState } from './workspace-state.js';
  import type { MediaEntry, Page, ScanProgress, ScanResult } from './types.js';
  interface Preferences { root?: string | null; recursive?: boolean; minimumSizeMib?: string; youtubeResolution?: 'source'|'1440p'|'4k'; youtubeQualityMode?: 'balanced'|'recommended'; youtubeSourceMultiplier?: number; youtubeOutputDir?: string | null }
  interface Props {
    root: string; recursive: boolean; minimumSizeMib: string; youtubeResolution: 'source'|'1440p'|'4k'; youtubeQualityMode: 'balanced'|'recommended'; youtubeSourceMultiplier: number; youtubeOutputDir: string; search: string; mediaPage: number;
    queue: QueueState; run: RunState; scanResult: ScanResult | null; scanProgress: ScanProgress | null; scanning: boolean; workflowBusy: boolean; desktopShell: boolean;
    youtubeEstimateSummary: {files:number;sourceBytes:number;outputBytes:number} | null;
    analysisErrors: MediaEntry[]; toolsRequiredEntries: MediaEntry[]; selectedRunEntries: MediaEntry[]; visibleEntries: MediaEntry[];
    selectedVisibleCount: number; selectedFilteredCount: number; mediaPageSize: number; selectedRunInputBytes: number; selectedMediaPathsForRun: string[];
    modeToolsReady: boolean | undefined; canStart: boolean; progressElapsedLabel: string;
    mediaTable: Snippet; bytes: (value:number|null|undefined)=>string; fileName:(path:string)=>string;
    persistPreferences:(patch:Preferences)=>unknown; browseFolder:()=>unknown; browseFile:()=>unknown; analyzeFolder:()=>unknown; chooseYoutubeOutput:()=>unknown;
    openPage:(page:Page)=>void; setFilteredSelection:(selected:boolean)=>void; stopAfterCurrent:()=>unknown; beginRun:()=>unknown;
  }
  let {
    root=$bindable(), recursive=$bindable(), minimumSizeMib=$bindable(), youtubeResolution=$bindable(), youtubeQualityMode=$bindable(), youtubeSourceMultiplier=$bindable(), youtubeOutputDir=$bindable(), search=$bindable(), mediaPage=$bindable(),
    queue, run, scanResult, scanProgress, scanning, workflowBusy, desktopShell, youtubeEstimateSummary, analysisErrors, toolsRequiredEntries, selectedRunEntries, visibleEntries,
    selectedVisibleCount, selectedFilteredCount, mediaPageSize, selectedRunInputBytes, selectedMediaPathsForRun, modeToolsReady, canStart, progressElapsedLabel,
    mediaTable, bytes, fileName, persistPreferences, browseFolder, browseFile, analyzeFolder, chooseYoutubeOutput, openPage, setFilteredSelection, stopAfterCurrent, beginRun,
  }: Props=$props();
</script>
<!-- Shared app.css scopes upload-profile, YouTube, source table and summary rules; instrument selectors retain their shared overrides. Native operations and per-file HDR consent remain owned by the route. -->
        <section class="page-heading">
          <div><div class="eyebrow">UPLOAD COPY WORKFLOW</div><h1>YouTube <em>prep.</em></h1><p>Set the upload profile, inspect every source, and create separate copies for YouTube.</p></div>
          <div class="heading-actions"><button class="button secondary" disabled={!desktopShell || workflowBusy} onclick={browseFolder}><FolderOpen size={17} /> Add folder</button><button class="button secondary" disabled={!desktopShell || workflowBusy} onclick={browseFile}><Video size={17} /> Add files</button><button class="button primary" disabled={!desktopShell || workflowBusy || !root} onclick={analyzeFolder}>{#if scanning}<LoaderCircle class="spin" size={17} /> Scanning{:else}<ScanSearch size={17} /> Add path to queue{/if}</button></div>
        </section>

        <section class="library-toolbar card-surface">
          <div class="folder-field"><FolderOpen size={18} /><input aria-label="Path to add to the media queue" bind:value={root} placeholder="Paste a media file or folder path to add" disabled={workflowBusy} onblur={() => void persistPreferences({ root })} /><button class="icon-button" aria-label="Add media folders" disabled={workflowBusy} onclick={browseFolder}><ChevronRight size={17} /></button></div>
          <div class="toolbar-options"><label class="check-label"><input type="checkbox" bind:checked={recursive} disabled={workflowBusy} onchange={() => void persistPreferences({ recursive })} /> Include subfolders on new scans</label><label class="small-field">Ignore smaller than <input type="number" min="0" max="4096" step="1" bind:value={minimumSizeMib} disabled={workflowBusy} onblur={() => void persistPreferences({ minimumSizeMib })} /> MiB</label></div>
        </section>

        {#if scanning}
          <div class="scan-progress card-surface"><LoaderCircle class="spin" size={19} /><div><strong>{scanProgress ? `Reading ${scanProgress.current} of ${scanProgress.total}` : "Preparing the analysis…"}</strong><span>{scanProgress?.file ?? "Finding media files and checking stream metadata"}</span></div><div class="progress-track"><div style={`width:${scanProgress?.total ? Math.max(4, scanProgress.current / scanProgress.total * 100) : 9}%`}></div></div></div>
        {/if}

        <section class="upload-profile-card">
          <p class="filename-filter-help">Preparation creates separate files for normal YouTube uploads. YouTube Live has separate encoder and ingestion requirements. See the <a href="https://support.google.com/youtube/answer/7126552?hl=en" target="_blank" rel="noreferrer">HDR upload guide</a> and <a href="https://developers.google.com/youtube/v3/live/guides/hls-ingestion" target="_blank" rel="noreferrer">Live HLS guide</a>. Upload processing and playback have not been verified by this app.</p>
          <div class="upload-profile-intro"><div class="settings-icon blue"><Video size={18} /></div><div><div class="eyebrow">YOUTUBE OUTPUT SETTINGS</div><h2>Choose your upload profile</h2><p>SDR sources use H.264. Compatible HDR sources use 10-bit HEVC Main 10. Audio is encoded to AAC, and exports go to a separate folder.</p></div></div>
          <div class="upload-profile-controls youtube-settings-grid">
            <label><span>Minimum output tier</span><select bind:value={youtubeResolution} onchange={() => void persistPreferences({ youtubeResolution })}><option value="source">Keep source size</option><option value="1440p">1440p · smaller sources upscale</option><option value="4k">4K · larger upload copies</option></select></label>
            <label><span>Bitrate profile</span><select bind:value={youtubeQualityMode} onchange={() => void persistPreferences({ youtubeQualityMode })}><option value="balanced">Balanced · source-aware size</option><option value="recommended">YouTube bitrate guidance</option></select></label>
            <label class="youtube-multiplier" class:disabled={youtubeQualityMode !== "balanced"}><span>Balanced size target <strong>{youtubeSourceMultiplier.toFixed(2)}× source</strong></span><input aria-label="Balanced source size target multiplier" type="range" min="0.5" max="4" step="0.25" value={youtubeSourceMultiplier} disabled={youtubeQualityMode !== "balanced" || workflowBusy} oninput={(event) => { youtubeSourceMultiplier = Number(event.currentTarget.value); }} onchange={() => void persistPreferences({ youtubeSourceMultiplier })} /><small>Video target floor: 250 kb/s. YouTube's rate is the ceiling; this is not a strict size cap.</small></label>
            <div class="upload-destination"><span>Output folder</span><strong>{youtubeOutputDir || (root ? `Default: YouTube Uploads beside ${fileName(root)}` : "Default: YouTube Uploads beside the selected media")}</strong><div><button class="text-button" disabled={!desktopShell || workflowBusy} onclick={chooseYoutubeOutput}>Choose folder</button><button class="text-button" disabled={!youtubeOutputDir || workflowBusy} onclick={() => { youtubeOutputDir = ""; void persistPreferences({ youtubeOutputDir: null }); }}>Use default</button></div></div>
          </div>
          {#if youtubeEstimateSummary}<div class="youtube-estimate"><div><span>ESTIMATED OUTPUT</span><strong>{bytes(youtubeEstimateSummary.outputBytes)}</strong></div><small>{youtubeEstimateSummary.files} currently eligible file{youtubeEstimateSummary.files === 1 ? "" : "s"} · eligible source total {bytes(youtubeEstimateSummary.sourceBytes)} · estimate varies with content and is not a hard size limit</small></div>{/if}
          <div class="upload-profile-note"><Info size={17} /><span>The Balanced slider targets an approximate multiple of each source's average total bitrate, then caps video bitrate at YouTube's published recommendation. YouTube bitrate guidance may create much larger files. Upscaling adds no source detail and does not guarantee a codec or playback tier. HDR export requires 10-bit video with Rec. 2020 primaries/matrix and PQ or HLG; Dolby Vision files are skipped. HDR10+ files are skipped unless you explicitly approve the dynamic-metadata loss for that individual file below. Subtitle streams are omitted from the MP4; prepare supported caption files separately.</span></div>
        </section>

        {#if !modeToolsReady}
          <div class="inline-warning"><Wrench size={17} /><div><strong>YouTube export needs FFmpeg, FFprobe, MediaInfo, libx264, and AAC.</strong><span>Check the environment and install missing tools only after reviewing the package list.</span></div><button class="text-button" onclick={() => openPage("settings")}>Open Settings</button></div>
        {/if}
        {#if scanResult}
          <section class="library-summary-grid">
            <div class="summary-tile"><span>FILES ANALYZED</span><strong>{scanResult.candidate_count}</strong><small>{recursive ? "Recursive scan" : "Top-level folder"}</small></div>
            <div class="summary-tile"><span>SOURCE TOTAL</span><strong>{bytes(scanResult.total_input_bytes)}</strong><small>Originals stay in place</small></div>
            <div class="summary-tile"><span>ESTIMATED COPIES</span><strong>{youtubeEstimateSummary ? bytes(youtubeEstimateSummary.outputBytes) : "—"}</strong><small>{youtubeResolution.toUpperCase()} · {youtubeQualityMode === "balanced" ? `${youtubeSourceMultiplier.toFixed(2)}× source` : "YouTube bitrate guidance"}</small></div>
            <div class="summary-tile"><span>NEEDS REVIEW</span><strong>{analysisErrors.length + toolsRequiredEntries.length}</strong><small>Analysis errors or missing tools</small></div>
          </section>
          {#if scanResult.warnings?.length}
            <div class="inline-warning"><Info size={17} /><div><strong>Scan notices</strong><ul class="scan-warning-list">{#each scanResult.warnings as warning (warning)}<li>{warning}</li>{/each}</ul></div></div>
          {/if}
          {#if analysisErrors.length}<div class="inline-warning"><AlertTriangle size={17} /><div><strong>{analysisErrors.length} file{analysisErrors.length === 1 ? "" : "s"} could not be analyzed</strong><span>They will not be silently treated as eligible exports. Open each row for the exact error.</span></div></div>{/if}
          <section class="table-card"><div class="table-header"><div><div class="eyebrow">ACTIVE SOURCE · {queue.activeRoot ? fileName(queue.activeRoot) : "NONE"}</div><h2>{visibleEntries.length} files</h2><small>{selectedVisibleCount} selected on this page · exports use selected files from this source, or all its files when none are selected</small></div><div class="table-header-tools">{#if visibleEntries.length > mediaPageSize}<button class="text-button" disabled={workflowBusy} onclick={() => setFilteredSelection(selectedFilteredCount !== visibleEntries.length)}>{selectedFilteredCount === visibleEntries.length ? `Clear ${visibleEntries.length} filtered selections` : `Select all ${visibleEntries.length} matches`}</button>{/if}<button class="text-button" disabled={!queue.selectedPaths.length || workflowBusy} onclick={() => queue.selectedPaths = []}>Clear selection</button><button class="text-button" disabled={workflowBusy} onclick={() => openPage("library")}>Choose another source</button><label class="search-box"><Search size={16} /><input aria-label="Search files" value={search} oninput={(event) => { search = event.currentTarget.value; mediaPage = 1; }} placeholder="Search files or status" /></label></div></div>{@render mediaTable()}</section>
        {:else if !scanning}
          <div class="empty-state large"><div class="empty-icon"><Video size={26} /></div><div><div class="eyebrow">READ-ONLY SOURCE ANALYSIS</div><h2>Add a source before exporting.</h2><p>The review includes video and audio tracks, HDR metadata, output-size estimates, memory forecasts, and any file that needs attention.</p></div><button class="button primary" disabled={!desktopShell || !root || workflowBusy} onclick={analyzeFolder}><ScanSearch size={17} /> Add path to queue</button></div>
        {/if}
        <section class="run-launch card-surface"><div class="launch-left"><div class="launch-icon"><Video size={19} /></div><div><strong>{run.id ? "YouTube export is running" : "Create separate upload copies"}</strong><span>{scanResult ? `${selectedRunEntries.length} files · ${bytes(selectedRunInputBytes)} source data${selectedMediaPathsForRun.length ? " · selected files only" : ""}` : "Analyze a video or folder before starting"} · originals remain in place</span></div></div>
          {#if run.id}<button class="button danger-outline" onclick={stopAfterCurrent}><Square size={15} /> Stop after current file</button>{:else}<button class="button primary" disabled={!canStart} onclick={beginRun}><Video size={17} /> Create upload copies</button>{/if}
        </section>
        {#if run.message || run.lines.length}
          <RunConsole run={run} title="Export activity" elapsedLabel={progressElapsedLabel} onStop={stopAfterCurrent} />
        {/if}

