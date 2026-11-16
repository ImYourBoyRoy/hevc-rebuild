<!-- ./desktop/src/lib/views/MediaTable.svelte
     Paged media table (selection, profile, track choices, review, run state and
     YouTube estimates) rendered inside the YouTube page. Pure view: state is
     route-owned and every action is a callback. -->
<script lang="ts">
  import type { Action } from "svelte/action";
  import { ArrowRight, Database, Film, Layers, LoaderCircle, Search, Video } from "@lucide/svelte";
  import type { createWorkspaceState } from "../workspace.svelte.js";
  import type { AppPreferences } from "../ui-contracts.js";
  import type { MediaEntry, MediaQueueRunState, Page, QueueMetadataSuggestion, QueueMovieReview, QueueSeriesReview, RunMode, Settings, YouTubeEstimate } from "../types.js";
  import { bytes } from "../format.js";
  import { proposedFilenameForEntry } from "../filename-template.js";
  import { queuePathKey } from "../queue-paths.js";
  import { queueRunStatusLabel } from "../queue-run-state.js";

  interface Props {
    entries: MediaEntry[];
    currentPageEntries: MediaEntry[];
    onOpenLibrary: () => void;
    visibleSelectAllCheckbox: HTMLInputElement | null;
    mediaPage: number;
    mediaPageSize: number;
    workspace: ReturnType<typeof createWorkspaceState>;
    page: Page;
    mode: RunMode;
    appPreferences: AppPreferences;
    desktopShell: boolean;
    scanning: boolean;
    workflowBusy: boolean;
    settings: Settings | null;
    fileTuneOverrides: Record<string, "animation" | "none">;
    queueArtworkPreviewBusy: Record<string, boolean>;
    queueArtworkPreviewErrors: Record<string, string>;
    queueArtworkPreviews: Record<string, string>;
    queueMatchSuggestions: Record<string, QueueMetadataSuggestion>;
    queueMovieReviews: Record<string, QueueMovieReview>;
    queueRunLogs: Record<string, { loading: boolean; text: string; error: string }>;
    search: string;
    selectedVisibleCount: number;
    visibleEntries: MediaEntry[];
    mediaPageCount: number;
    mediaPageFirst: number;
    mediaPageLast: number;
    youtubeHdr10plusHelperReady: boolean;
    chooseFileOutput: (path: string) => unknown;
    clearFileOutput: (path: string) => unknown;
    effectiveContentTuneForEntry: (entry: MediaEntry, review: QueueMovieReview | QueueSeriesReview | undefined, effectiveSettings: Settings | null | undefined) => string;
    formatMetadataValue: (value: unknown) => string;
    hasHdr10plusSignal: (entry: MediaEntry) => boolean;
    loadQueueRunDiagnostics: (entry: MediaEntry, state: MediaQueueRunState) => unknown;
    metadataSuggestsAnimation: (review: QueueMovieReview | QueueSeriesReview | undefined) => boolean;
    observeQueueArtwork: Action<HTMLElement, { mediaPath: string; artworkPath: string }>;
    openMetadataForEntry: (entry: MediaEntry) => unknown;
    openQueueRunReport: (path: string) => unknown;
    prepareQueueEntryForRetry: (entry: MediaEntry, state: MediaQueueRunState) => unknown;
    profileSummary: (entry: MediaEntry) => string;
    queueRunStateForPath: (path: string) => MediaQueueRunState | undefined;
    resourceForecastFor: (entry: MediaEntry) => MediaEntry["resource_forecast"];
    reviewedSeriesForPath: (path: string) => QueueSeriesReview | undefined;
    selectedStreamIndices: (entry: MediaEntry) => number[];
    selectedTrackCount: (entry: MediaEntry) => number;
    setFileStreamSelected: (entry: MediaEntry, index: number, included: boolean) => unknown;
    setFileTuneOverride: (path: string, value: string) => unknown;
    setVisibleSelection: (select: boolean) => unknown;
    setYoutubeHdr10plusLossAccepted: (path: string, accepted: boolean) => unknown;
    settingsForEntry: (entry: MediaEntry) => Settings | null | undefined;
    skipQueueEntry: (entry: MediaEntry, state: MediaQueueRunState) => unknown;
    statusLabel: (status: string) => string;
    streamIsSelectable: (type: string) => boolean;
    youtubeEstimateFor: (entry: MediaEntry) => YouTubeEstimate | undefined;
    youtubeHdr10plusConsentMessage: (entry: MediaEntry) => string;
    youtubeHdr10plusLossAccepted: (path: string) => boolean;
    toggleEntrySelection: (path: string, selected: boolean) => void;
  }
  let {
    entries, currentPageEntries, onOpenLibrary, visibleSelectAllCheckbox = $bindable(), mediaPage = $bindable(), mediaPageSize = $bindable(),
    workspace, page, mode, appPreferences, desktopShell, scanning, workflowBusy, settings, fileTuneOverrides, queueArtworkPreviewBusy, queueArtworkPreviewErrors,
    queueArtworkPreviews, queueMatchSuggestions, queueMovieReviews, queueRunLogs, search, selectedVisibleCount, visibleEntries, mediaPageCount, mediaPageFirst,
    mediaPageLast, youtubeHdr10plusHelperReady, chooseFileOutput, clearFileOutput, effectiveContentTuneForEntry, formatMetadataValue, hasHdr10plusSignal,
    loadQueueRunDiagnostics, metadataSuggestsAnimation, observeQueueArtwork, openMetadataForEntry, openQueueRunReport, prepareQueueEntryForRetry, profileSummary,
    queueRunStateForPath, resourceForecastFor, reviewedSeriesForPath, selectedStreamIndices, selectedTrackCount, setFileStreamSelected, setFileTuneOverride,
    setVisibleSelection, setYoutubeHdr10plusLossAccepted, settingsForEntry, skipQueueEntry, statusLabel, streamIsSelectable, youtubeEstimateFor,
    youtubeHdr10plusConsentMessage, youtubeHdr10plusLossAccepted, toggleEntrySelection,
  }: Props = $props();
</script>

{#if entries.length}
  <div class="table-scroll"><table><thead><tr><th class="selection-header">{#if page === "library" || page === "youtube"}<input bind:this={visibleSelectAllCheckbox} type="checkbox" aria-label={`Select all ${currentPageEntries.length} media files on this page`} checked={currentPageEntries.length > 0 && selectedVisibleCount === currentPageEntries.length} disabled={workflowBusy || !currentPageEntries.length} onchange={(event) => setVisibleSelection(event.currentTarget.checked)} />{:else}<span class="sr-only">Select file</span>{/if}</th><th>MEDIA FILE</th><th>VIDEO PROFILE</th><th>STREAMS</th><th>{page === "youtube" ? "YOUTUBE MEMORY PLAN" : "REBUILD MEMORY PLAN"}</th><th>STATUS</th></tr></thead><tbody>
    {#each entries as entry (entry.path)}
      {@const youtubeEstimate = youtubeEstimateFor(entry)}
      {@const memoryForecast = resourceForecastFor(entry)}
      {@const queueRun = page === "library" ? queueRunStateForPath(entry.path) : undefined}
      {@const queueRunLog = queueRun ? queueRunLogs[queuePathKey(entry.path)] : undefined}
      {@const rowProgress = workspace.run.progress?.filePath && queuePathKey(workspace.run.progress.filePath) === queuePathKey(entry.path) ? workspace.run.progress : null}
      {@const entrySettings = page === "library" ? settingsForEntry(entry) : settings}
      {@const reviewedMovie = queueMovieReviews[entry.path]}
      {@const reviewedSeries = page === "library" ? reviewedSeriesForPath(entry.path) : undefined}
      {@const reviewedMetadata = reviewedMovie ?? reviewedSeries}
      {@const matchSuggestion = page === "library" ? queueMatchSuggestions[entry.path] : undefined}
      {@const filenameProposal = page === "library" ? proposedFilenameForEntry(entry, reviewedMovie, reviewedSeries, matchSuggestion, appPreferences.organizerTemplates) : null}
      {@const reviewedPoster = reviewedMetadata?.artworkCandidates.find((candidate) => candidate.id === reviewedMetadata.posterId)}
      {@const localPosterPath = reviewedMetadata?.localPosterPath ?? ""}
      {@const localPosterKey = localPosterPath ? queuePathKey(localPosterPath) : ""}
      <tr data-media-path={entry.path}>
        <td><input type="checkbox" aria-label={`Select ${entry.name}`} checked={workspace.queue.selectedPaths.includes(entry.path)} disabled={workflowBusy || (entry.queue_stale === "missing" && !workspace.queue.selectedPaths.includes(entry.path))} onchange={(event) => toggleEntrySelection(entry.path, event.currentTarget.checked)} /></td>
        <td>
          <div class="file-cell">
            {#if page === "library" && reviewedMetadata}
              <button class="queue-poster-button" use:observeQueueArtwork={{ mediaPath: entry.path, artworkPath: localPosterPath }} type="button" aria-label={`Edit title and poster for ${entry.name}`} title={localPosterPath ? `${reviewedMetadata.title} · local poster` : reviewedPoster ? `${reviewedMetadata.title} poster · ${reviewedPoster.provider}` : `Edit ${reviewedMetadata.title || "reviewed title"}`} onclick={() => openMetadataForEntry(entry)}>
                {#if localPosterPath && queueArtworkPreviews[localPosterKey]}<img src={queueArtworkPreviews[localPosterKey]} alt="" loading="lazy" />{:else if reviewedPoster && (!localPosterPath || queueArtworkPreviewErrors[localPosterKey])}<img src={reviewedPoster.thumbnailUrl || reviewedPoster.imageUrl} alt="" loading="lazy" referrerpolicy="no-referrer" />{:else}<span class="queue-poster-placeholder"><Film size={17} /></span>{/if}
                <span class="queue-poster-edit"><Database size={12} /></span>
              </button>
            {:else if page === "library" && matchSuggestion}
              <button class="queue-poster-button queue-poster-suggested" type="button" aria-label={`Review suggested title and poster for ${entry.name}`} title={matchSuggestion.title ? `Suggested: ${matchSuggestion.title}${matchSuggestion.year ? ` (${matchSuggestion.year})` : ""} · review before assigning` : "Review title and poster"} onclick={() => openMetadataForEntry(entry)}>
                {#if matchSuggestion.artwork}<img src={matchSuggestion.artwork.thumbnailUrl || matchSuggestion.artwork.imageUrl} alt="" loading="lazy" referrerpolicy="no-referrer" />{:else}<span class="queue-poster-placeholder">{#if matchSuggestion.status === "searching"}<LoaderCircle size={17} class="spin" />{:else}<Film size={17} />{/if}</span>{/if}
                <span class="queue-poster-edit"><Database size={12} /></span>
              </button>
            {:else}
              <div class="file-icon"><Video size={17} /></div>
            {/if}
            <div class="file-cell-copy">
              <strong class="file-cell-title" title={entry.path}>{entry.name}</strong>
              <small class="file-cell-sub" title={entry.path}>{entry.relative_path}</small>
              {#if page === "library" && reviewedMetadata}<small class="queue-reviewed-metadata" title={`${reviewedMetadata.sourceUrl}${reviewedPoster ? ` · Poster: ${reviewedPoster.title}` : ""}`}><strong>{reviewedMetadata.title || "Untitled review"}</strong><span>{matchSuggestion?.autoAssociated ? "Auto-associated · exact title and year" : reviewedMetadata.recordId === "local-nfo" || reviewedMetadata.recordId.startsWith("local-nfo:") || reviewedMetadata.recordId.startsWith("local:") ? "Existing local metadata" : reviewedMetadata.recordId.split(":")[0].toUpperCase()}{reviewedMetadata.year ? ` · ${reviewedMetadata.year}` : ""}{localPosterPath ? " · poster saved for MKV output" : reviewedPoster ? " · poster preview only; save to include" : ""}{reviewedSeries && !reviewedMovie ? " · series" : ""}</span></small>{/if}
              {#if page === "library" && !reviewedMetadata && matchSuggestion?.status === "ready"}
                {#if matchSuggestion.title}<small class="queue-match-suggestion"><strong>{matchSuggestion.autoAssociated ? "Auto-associated: " : "Suggested: "}{matchSuggestion.title}{matchSuggestion.year ? ` (${matchSuggestion.year})` : ""}</strong><span>{matchSuggestion.autoAssociated ? "Exact title and year; no competing result" : `${matchSuggestion.provider?.toUpperCase() ?? "Public source"} · review before assigning`}</span></small>
                {:else}<small class="queue-match-suggestion"><strong>No likely title match found</strong><span>Review the filename and search other sources.</span></small>{/if}
              {:else if page === "library" && matchSuggestion?.status === "searching"}<small class="queue-match-suggestion">Checking the title and poster…</small>
              {:else if page === "library" && matchSuggestion?.status === "error"}<small class="row-error" title={matchSuggestion.error}>Title lookup failed: {matchSuggestion.error.slice(0, 150)}</small>{/if}
              {#if filenameProposal}<small class="queue-filename-proposal" class:needs-review={filenameProposal.needsTitleReview} title={`Proposed filename: ${filenameProposal.name}`}><span>{filenameProposal.needsTitleReview ? "Candidate filename · review title first" : "Proposed filename"}</span><code>{filenameProposal.name}</code></small>{/if}
              {#if page === "library"}
                <label class="per-file-tune"><span>Content tune</span><select aria-label={`Content tune for ${entry.name}`} value={fileTuneOverrides[entry.path] ?? ""} disabled={workflowBusy} onchange={(event) => setFileTuneOverride(entry.path, event.currentTarget.value)}><option value="">Automatic · {effectiveContentTuneForEntry(entry, reviewedMetadata, entrySettings) === "animation" ? "Animated" : effectiveContentTuneForEntry(entry, reviewedMetadata, entrySettings) === "grain" ? "Film grain" : "Normal"}</option><option value="animation">Animated</option><option value="none">Normal</option></select></label>
                {#if metadataSuggestsAnimation(reviewedMetadata)}<small class="file-cell-sub tune-suggestion">Animation is in the genres, so this file uses the animation tune unless you choose Normal.</small>{:else if entry.profile?.content?.animation_detected}<small class="file-cell-sub tune-suggestion">Animation was detected in source metadata and is reflected in the Automatic setting above.</small>{/if}
              {/if}
              {#if page === "library" && localPosterPath && queueArtworkPreviewErrors[localPosterKey]}<small class="queue-poster-error" title={queueArtworkPreviewErrors[localPosterKey]}>Local poster could not be previewed</small>{/if}
              {#if page === "library" && localPosterPath && queueArtworkPreviewBusy[localPosterKey]}<small class="queue-poster-loading">Loading local poster…</small>{/if}
              {#if entry.error}<small class="row-error">{entry.error}</small>{/if}
              {#if entry.warning}<small class="row-warning">{entry.warning}</small>{/if}
              {#if entry.queue_stale === "missing"}<small class="row-error">This file is missing since the last scan. Refresh the source after restoring or removing it.</small>{:else if entry.queue_stale === "changed"}<small class="row-warning">This file changed after its last scan. Refresh this source before running it.</small>{/if}
              {#if page === "library"}<button class="text-button file-metadata-action" type="button" aria-label={`${reviewedMetadata ? "Edit" : "Review"} title and poster for ${entry.name}`} onclick={() => openMetadataForEntry(entry)}><Database size={14} /> {reviewedMetadata ? `Edit ${reviewedSeries && !reviewedMovie ? "series" : "title"} and poster` : "Review title and poster"}</button>{/if}
              {#if entry.profile?.tracks.length}
                <details class="stream-details">
                  <summary>Tracks & stream metadata · {entry.profile.tracks.length}{#if page === "library"} · {selectedTrackCount(entry)}/{entry.profile.tracks.length} included{/if}</summary>
                  {#if entry.profile.mediainfo_available === false}<small class="file-cell-sub">MediaInfo is unavailable; FFprobe still provides the visible stream and container data below.</small>{/if}
                  {#if entry.profile.frame_metadata_available === false}<small class="file-cell-sub">Per-frame side data could not be read. HDR10+ indicators are candidates and have not been confirmed from frame metadata.</small>{/if}
                  {#if page === "library"}
                    <p class="track-selection-note">Default selection follows the saved secondary-video and data-stream policies. After you customize this file, included real-video tracks are encoded to HEVC, while included audio, subtitles, attachments, and data tracks are copied. Keep both 4K and 1080p selected to retain both in one MKV.</p>
                  {/if}
                  {#if Object.keys(entry.profile.mediainfo_general ?? {}).length}
                    <details class="track-item">
                      <summary><strong>Container / general</strong></summary>
                      <dl class="track-metadata">{#each Object.entries(entry.profile.mediainfo_general) as [key, value] (key)}<div><dt>{key.replaceAll("_", " ")}</dt><dd>{formatMetadataValue(value)}</dd></div>{/each}</dl>
                    </details>
                  {/if}
                  {#if entry.profile.source_metadata}
                    <details class="track-item source-metadata-item">
                      <summary><strong>Source container & chapters</strong><span>{entry.profile.chapters_count} chapters</span></summary>
                      {#if entry.profile.source_metadata.format}<details class="raw-metadata-details"><summary>FFprobe format fields</summary><pre class="raw-metadata">{formatMetadataValue(entry.profile.source_metadata.format)}</pre></details>{/if}
                      {#if entry.profile.source_metadata.chapters?.length}<details class="raw-metadata-details"><summary>Chapter data</summary><pre class="raw-metadata">{formatMetadataValue(entry.profile.source_metadata.chapters)}</pre></details>{/if}
                    </details>
                  {/if}
                  <div class="track-list">
                    {#each entry.profile.tracks as track (track.index)}
                      <div class="track-selection-row">
                        {#if page === "library" && streamIsSelectable(track.type)}
                          <label class="track-selection-toggle"><input type="checkbox" checked={selectedStreamIndices(entry).includes(track.index)} disabled={workflowBusy} onchange={(event) => setFileStreamSelected(entry, track.index, event.currentTarget.checked)} /><span>{selectedStreamIndices(entry).includes(track.index) ? "Included" : "Excluded"}<small>stream #{track.index}</small></span></label>
                        {:else if page === "library"}
                          <span class="track-selection-toggle unavailable">View only<small>unsupported type</small></span>
                        {/if}
                        <details class="track-item">
                          <summary>
                            <strong>{track.attached_picture ? "Cover image" : `${track.type} ${track.ordinal + 1}`}</strong><span>{track.codec ?? "unknown codec"}</span>
                            {#if track.width && track.height}<span>{track.width} × {track.height}</span>{/if}
                            {#if track.bit_depth}<span>{track.bit_depth}-bit</span>{/if}
                            {#if track.language}<span>{track.language}</span>{/if}
                            {#if track.title}<span>{track.title}</span>{/if}
                            {#if track.default}<span class="track-flag">Default</span>{/if}
                            {#if track.forced}<span class="track-flag">Forced</span>{/if}
                          </summary>
                          <dl class="track-metadata">
                            {#if track.pixel_format}<div><dt>Pixel format</dt><dd>{track.pixel_format}</dd></div>{/if}
                            {#if track.color_primaries || track.color_transfer || track.color_matrix}<div><dt>Color</dt><dd>{[track.color_primaries, track.color_transfer, track.color_matrix].filter(Boolean).join(" · ")}</dd></div>{/if}
                            {#if track.channels || track.channel_layout || track.sample_rate}<div><dt>Audio</dt><dd>{[track.channels ? `${track.channels} channels` : "", track.channel_layout, track.sample_rate ? `${track.sample_rate} Hz` : ""].filter(Boolean).join(" · ")}</dd></div>{/if}
                            {#if track.filename || track.mime_type}<div><dt>Attachment</dt><dd>{[track.filename, track.mime_type].filter(Boolean).join(" · ")}</dd></div>{/if}
                            {#each Object.entries(track.mediainfo ?? {}) as [key, value] (key)}<div><dt>{key.replaceAll("_", " ")}</dt><dd>{formatMetadataValue(value)}</dd></div>{/each}
                          </dl>
                          {#if track.ffprobe}<details class="raw-metadata-details"><summary>FFprobe stream fields</summary><pre class="raw-metadata">{formatMetadataValue(track.ffprobe)}</pre></details>{/if}
                        </details>
                      </div>
                    {/each}
                  </div>
                </details>
              {/if}
              {#if page === "library" && entry.profile}
                {@const globalTune = entrySettings?.content.tune ?? "auto"}
                {@const resolvedTune = effectiveContentTuneForEntry(entry, reviewedMetadata, entrySettings)}
                <small class="file-cell-sub">Applied tune: {resolvedTune === "animation" ? "Animated" : resolvedTune === "grain" ? "Film grain" : "Normal"}{fileTuneOverrides[entry.path] ? " · file choice" : ` · ${globalTune === "auto" ? "Automatic" : "global setting"}`}</small>
                {#if entry.profile.content?.animation_detected}<small class="file-cell-sub">Animation genre found in {entry.profile.content.sources.join(" + ") || "source metadata"}. {#if globalTune === "auto" && settings?.content.detect_animation}Automatic mode selects Animated.{:else}Choose Animated above to apply it to this file.{/if}</small>{/if}
                <div class="file-output-control"><small>{appPreferences.fileOutputDirs[entry.path] ? `Output: ${appPreferences.fileOutputDirs[entry.path]}` : appPreferences.rebuildOutputDir ? `Default: ${appPreferences.rebuildOutputDir}` : "Output: source folder"}</small><div><button class="text-button" disabled={!desktopShell || workflowBusy} onclick={() => chooseFileOutput(entry.path)}>Set output folder</button>{#if appPreferences.fileOutputDirs[entry.path]}<button class="text-button" disabled={workflowBusy} onclick={() => clearFileOutput(entry.path)}>Use default</button>{/if}</div></div>
              {/if}
            </div>
          </div>
        </td>
        <td><span class="profile-primary">{profileSummary(entry)}</span><small class="file-cell-sub">{entry.profile?.video.codec ?? "—"}{entry.profile?.interlaced ? " · Interlaced" : ""}</small></td>
        <td><span class="stream-count"><Layers size={14} /> {entry.profile?.video.secondary_count !== undefined ? entry.profile.video.secondary_count + 1 : "—"} video <i>·</i> {entry.profile?.audio_count ?? "—"} audio <i>·</i> {entry.profile?.subtitle_count ?? "—"} subs <i>·</i> {entry.profile?.attachment_count ?? "—"} attachments</span>{#if entry.profile?.hdr.dolby_vision}<small class="file-cell-sub">Profile {entry.profile.hdr.dolby_vision_profile ?? "?"} · Dolby Vision</small>{/if}</td>
        <td>
          {#if memoryForecast}
            <div class="resource-plan-summary"><span class="status-tag" class:status-ready={memoryForecast.level === "guarded"} class:status-warning={memoryForecast.level === "constrained" || memoryForecast.level === "unknown"} class:status-error={memoryForecast.level === "high" || memoryForecast.level === "blocked"}>{memoryForecast.level.toUpperCase()}</span><small>{memoryForecast.planned_workers ?? "—"} workers · {memoryForecast.planned_frame_threads ?? "—"} frame threads</small></div>
            <details class="resource-plan-details">
              <summary>Memory and retry details</summary>
              <small>{memoryForecast.planning_budget_gib !== null && memoryForecast.planning_budget_gib !== undefined ? `${memoryForecast.planning_budget_gib.toFixed(1)} GiB planning budget` : "Memory budget unavailable"}{memoryForecast.swap_used_percent !== null && memoryForecast.swap_used_percent !== undefined ? ` · swap ${memoryForecast.swap_used_percent.toFixed(0)}% used` : ""}</small>
              {#if memoryForecast.messages.length}<p>{memoryForecast.messages.join(" ")}</p>{/if}
              {#if memoryForecast.retry_plan && memoryForecast.level !== "guarded"}
                {#if memoryForecast.retry_plan.reduces_parallelism}<p>First pressure retry: {memoryForecast.retry_plan.planned_workers} workers · {memoryForecast.retry_plan.planned_frame_threads} frame thread{memoryForecast.retry_plan.planned_frame_threads === 1 ? "" : "s"} · {memoryForecast.retry_plan.level.toUpperCase()}.</p>
                {:else}<p>Already at minimum parallelism. If pressure persists, the encode stops instead of repeating the same plan.</p>{/if}
              {/if}
            </details>
          {:else}<small class="file-cell-sub">No forecast</small>{/if}
        </td>
        <td>
          <span class="status-tag" class:status-ready={entry.status === "Ready"} class:status-warning={entry.status === "Backed up"} class:status-error={entry.status === "Analysis failed" || entry.status === "Tools required"}>{statusLabel(entry.status)}</span>
          <small class="file-cell-sub">Source {bytes(entry.size_bytes)}</small>
          {#if rowProgress}<div class="queue-live-progress" aria-label={`${rowProgress.phase} for ${entry.name}`}><div><span>{rowProgress.phase}</span>{#if rowProgress.filePercent != null && rowProgress.filePercent > 0}<strong>{rowProgress.filePercent.toFixed(0)}%</strong>{/if}</div><div class="queue-live-progress-track" role="progressbar" aria-label={`${rowProgress.phase} progress`} aria-valuemin="0" aria-valuemax="100" aria-valuenow={rowProgress.filePercent != null && rowProgress.filePercent > 0 ? rowProgress.filePercent : undefined} aria-busy={rowProgress.filePercent == null || rowProgress.filePercent <= 0}><span class:indeterminate={rowProgress.filePercent == null || rowProgress.filePercent <= 0} style={rowProgress.filePercent != null && rowProgress.filePercent > 0 ? `width:${rowProgress.filePercent}%` : ""}></span></div>{#if rowProgress.outputBytes != null || rowProgress.estimatedOutputBytes != null}<small>{rowProgress.outputBytes != null ? `Output ${bytes(rowProgress.outputBytes)}` : "Output is being created"}{rowProgress.estimatedOutputBytes != null ? ` · estimated ${bytes(rowProgress.estimatedOutputBytes)}` : ""}</small>{/if}</div>{/if}
          {#if queueRun}
            <small class="queue-run-state" class:queue-run-completed={queueRun.status === "completed" || queueRun.status === "renamed"} class:queue-run-warning={["queued", "running", "scanning", "renaming", "remuxing", "rebuilding", "validating", "skipped"].includes(queueRun.status)} class:queue-run-error={queueRun.status === "failed" || queueRun.status === "interrupted" || queueRun.status === "needs_review"}>{queueRunStatusLabel(queueRun.status)}{queueRun.mode ? ` · ${queueRun.mode === "organize" ? "RENAME" : queueRun.mode.toUpperCase()}` : ""}</small>
            {#if queueRun.message || queueRun.outputPath || queueRun.reportPath}
              <details class="queue-run-details">
                <summary>Run details</summary>
                {#if queueRun.message}<p>{queueRun.message}</p>{/if}
                {#if queueRun.reportStatus}<small>Report result · {queueRun.reportStatus}</small>{/if}
                {#if queueRun.outputPath}<small title={queueRun.outputPath}>Output · {queueRun.outputPath}</small>{/if}
                {#if queueRun.reportPath}
                  <div class="queue-run-detail-actions">
                    <button class="text-button" type="button" disabled={workflowBusy} onclick={() => openQueueRunReport(queueRun.reportPath!)}>Open report <ArrowRight size={13} /></button>
                    <button class="text-button" type="button" disabled={queueRunLog?.loading} onclick={() => loadQueueRunDiagnostics(entry, queueRun)}>{queueRunLog?.loading ? "Loading diagnostics…" : queueRunLog?.text ? "Reload diagnostics" : "Load bounded diagnostics"}</button>
                  </div>
                  {#if queueRunLog?.error}<small class="queue-run-log-error">{queueRunLog.error}</small>{/if}
                  {#if queueRunLog?.text}<pre class="queue-run-log">{queueRunLog.text}</pre>{/if}
                {/if}
              </details>
            {/if}
            {#if ["queued", "failed", "skipped", "interrupted", "needs_review"].includes(queueRun.status)}
              <div class="queue-row-actions">
                <button class="text-button" type="button" disabled={!desktopShell || workflowBusy || Boolean(entry.queue_stale)} onclick={() => prepareQueueEntryForRetry(entry, queueRun)}>{queueRun.mode === "organize" ? "Review rename" : queueRun.status === "queued" ? "Run this file" : queueRun.status === "interrupted" || queueRun.status === "needs_review" ? "Select to review" : "Select to retry"}</button>
                {#if queueRun.status !== "skipped"}<button class="text-button queue-skip-action" type="button" disabled={!desktopShell || workflowBusy || Boolean(entry.queue_stale)} onclick={() => skipQueueEntry(entry, queueRun)}>Skip {queueRun.mode?.toUpperCase() ?? "task"}</button>{/if}
              </div>
            {/if}
          {/if}
          {#if page === "youtube" && youtubeEstimate && (!hasHdr10plusSignal(entry) || youtubeHdr10plusHelperReady || youtubeHdr10plusLossAccepted(entry.path))}<small class="file-cell-sub">Upload estimate {bytes(youtubeEstimate.estimated_output_bytes)} · {youtubeEstimate.video_bitrate_kbps} kb/s video · {youtubeEstimate.output_width}×{youtubeEstimate.output_height}</small>{/if}
          {#if page === "youtube" && entry.profile?.hdr.dolby_vision}<small class="row-error">YouTube export skips Dolby Vision sources.</small>{/if}
          {#if page === "youtube" && hasHdr10plusSignal(entry) && youtubeHdr10plusHelperReady}<small class="file-cell-sub">HDR10+ will be preserved after re-encode with hdr10plus_tool inject (stream copy keeps it as-is).</small>{/if}
          {#if page === "youtube" && hasHdr10plusSignal(entry) && !youtubeHdr10plusHelperReady}<label class="youtube-loss-consent"><input type="checkbox" checked={youtubeHdr10plusLossAccepted(entry.path)} disabled={workflowBusy} onchange={(event) => setYoutubeHdr10plusLossAccepted(entry.path, event.currentTarget.checked)} /><span><strong>Allow copy without HDR10+ dynamic metadata</strong><small>{youtubeHdr10plusLossAccepted(entry.path) ? "Loss approved for this file; static HDR10/PQ signaling remains. Install hdr10plus_tool to preserve it instead." : youtubeHdr10plusConsentMessage(entry)}</small></span></label>{/if}
          {#if page === "youtube" && (entry.profile?.subtitle_count ?? 0) > 0}<small class="file-cell-sub">Source subtitles are omitted; add supported caption files separately.</small>{/if}
        </td></tr>
    {/each}
  </tbody></table></div>
  {#if page === "library" || page === "youtube"}
    <nav class="media-pagination" aria-label="Media file pages">
      <span>Showing {mediaPageFirst}–{mediaPageLast} of {visibleEntries.length} files · page {mediaPage} of {mediaPageCount}</span>
      <label>Rows per page<select aria-label="Rows per page" value={mediaPageSize} onchange={(event) => { mediaPageSize = Number(event.currentTarget.value); mediaPage = 1; }}><option value={50}>50</option><option value={100}>100</option><option value={200}>200</option></select></label>
      <div><button type="button" class="button secondary" disabled={workflowBusy || mediaPage <= 1} onclick={() => mediaPage = Math.max(1, mediaPage - 1)}>Previous</button><button type="button" class="button secondary" disabled={workflowBusy || mediaPage >= mediaPageCount} onclick={() => mediaPage = Math.min(mediaPageCount, mediaPage + 1)}>Next</button></div>
    </nav>
  {/if}
{:else if page === "library" || page === "youtube"}
  <div class="media-empty-filter"><div><strong>{search ? "No files match this search." : "No media candidates found."}</strong>{#if !search}<p>Try including subfolders or lowering the minimum file size.</p>{/if}</div>{#if search}<button type="button" class="text-button" onclick={() => { search = ""; mediaPage = 1; }}>Clear search</button>{/if}</div>
{:else}
  <div class="empty-state compact"><div class="empty-icon"><Search size={19} /></div><div><strong>{search ? "No matching files" : "No media candidates found"}</strong><p>{search ? "Try another filename or status." : "Try enabling subfolder scanning or lowering the minimum file size."}</p></div>{#if !search}<button class="text-button" onclick={onOpenLibrary}>Adjust scan options <ArrowRight size={14} /></button>{/if}</div>
{/if}
