<!-- ./desktop/src/lib/views/RunView.svelte
     The Run page: task picker, output folder, per-file review, rebuild
     parameters, queue plan and launch card. Pure view; every action is a
     callback and every value a prop owned by the route. -->
<script lang="ts">
  import { AlertTriangle, Cpu, FileText, Film, FolderOpen, Gauge, Info, OctagonX, Play, ScanSearch, Settings2, ShieldCheck, SlidersHorizontal, Square, TerminalSquare, Video } from "@lucide/svelte";
  import InfoTip from "../InfoTip.svelte";
  import { CRF_EXPLANATION, crfBand, crfSizeHint, encoderAdvisories, presetHelp, settingHelp, tuneHelp } from "../explanations.js";
  import RunConsole from "../RunConsole.svelte";
  import RunModePicker from "../RunModePicker.svelte";
  import type { createWorkspaceState } from "../workspace.svelte.js";
  import type { SourceRunBatch } from "../run-controller.js";
  import type { AppPreferences, PreferencesPayload } from "../ui-contracts.js";
  import type { Environment, FileRunSettingsOverride, MediaEntry, Page, RunMode, ScanResult, Settings } from "../types.js";
  import { queuePathKey } from "../queue-paths.js";

  interface Props {
    mode: RunMode;
    integrityLevel: "quick" | "full";
    stopOnError: boolean;
    settings: Settings | null;
    runLaunchButton: HTMLButtonElement | null;
    workspace: ReturnType<typeof createWorkspaceState>;
    appPreferences: AppPreferences;
    preferenceStorage: PreferencesPayload | null;
    environment: Environment | null;
    desktopShell: boolean;
    workflowBusy: boolean;
    mutatingRun: boolean;
    canStart: boolean;
    modeToolsReady: boolean | undefined;
    scanResult: ScanResult | null;
    root: string;
    settingsChanged: boolean;
    settingsPath: string;
    settingsSources: string[];
    forecastOutdated: boolean;
    fileSettingsOverrides: Record<string, FileRunSettingsOverride>;
    runBatches: SourceRunBatch[];
    runPlanPreviewByRoot: Map<string, MediaEntry[]>;
    selectedRunEntries: MediaEntry[];
    selectedMediaPathsForRun: string[];
    selectedRunInputBytes: number;
    selectionInOtherSources: boolean;
    unresolvedSelectedPaths: number;
    staleSelectedEntries: MediaEntry[];
    staleRunSources: SourceRunBatch[];
    progressElapsedLabel: string;
    bytes: (value: number | null | undefined) => string;
    fileName: (path: string) => string;
    openPage: (page: Page) => void;
    beginRun: () => unknown;
    stopAfterCurrent: () => unknown;
    onStopNow: () => void;
    keepReadyFiles: () => void;
    chooseRebuildOutput: () => unknown;
    persistPreferences: (patch: Partial<AppPreferences>) => unknown;
    openRunSettingsEditor: (paths: string[]) => void;
    clearRunSettingsOverride: (paths: string[]) => void;
    runEntriesForFolder: (folder: string) => MediaEntry[];
    runOverrideSummary: (entry: MediaEntry) => string;
    refreshQueueSource: (sourceRoot: string) => unknown;
    loadSettings: (folder?: string) => unknown;
    resetSettings: () => unknown;
    saveSettings: () => unknown;
  }
  let {
    mode = $bindable(), integrityLevel = $bindable(), stopOnError = $bindable(), settings = $bindable(), runLaunchButton = $bindable(),
    workspace, appPreferences, preferenceStorage, environment, desktopShell, workflowBusy, mutatingRun, canStart, modeToolsReady, scanResult, root,
    settingsChanged, settingsPath, settingsSources, forecastOutdated, fileSettingsOverrides, runBatches, runPlanPreviewByRoot, selectedRunEntries,
    selectedMediaPathsForRun, selectedRunInputBytes, selectionInOtherSources, unresolvedSelectedPaths, staleSelectedEntries, staleRunSources,
    progressElapsedLabel, bytes, fileName, openPage, beginRun, stopAfterCurrent, onStopNow, keepReadyFiles, chooseRebuildOutput, persistPreferences,
    openRunSettingsEditor, clearRunSettingsOverride, runEntriesForFolder, runOverrideSummary, refreshQueueSource, loadSettings, resetSettings, saveSettings,
  }: Props = $props();
</script>

<section class="page-heading">
<div><div class="eyebrow">CONTROLLED PROCESSING</div><h1>Run</h1><p>{mode === "check" || mode === "integrity" ? "Read-only checks write nothing. Pick a task and start." : mode === "metadata" ? "Choose where tagged copies are written, then review destinations before starting." : "Review destinations and encoder choices before starting."}</p></div>
{#if scanResult}<div class="library-chip"><FolderOpen size={15} /><span title={root}>{fileName(root)}</span><span class="chip-divider"></span>{scanResult.candidate_count} files</div>{/if}
</section>

{#if !scanResult}<div class="inline-warning"><Info size={17} /><div><strong>Add media to the library before choosing a task.</strong><span>The scan lists media details, resource forecasts, and file-specific warnings.</span></div><button class="button secondary" onclick={() => openPage("library")}>Open library</button></div>{/if}
<RunModePicker {mode} disabled={workflowBusy} toolsReady={Boolean(modeToolsReady)} onSelect={(next: Exclude<RunMode, "youtube">) => (mode = next)} />
{#if mode === "integrity"}
<div class="integrity-level" role="radiogroup" aria-label="Integrity depth">
  <button type="button" class="button" class:primary={integrityLevel === "quick"} class:secondary={integrityLevel !== "quick"} disabled={workflowBusy} onclick={() => (integrityLevel = "quick")}>Quick structure check</button>
  <button type="button" class="button" class:primary={integrityLevel === "full"} class:secondary={integrityLevel !== "full"} disabled={workflowBusy} onclick={() => (integrityLevel = "full")}>Full decode check</button>
  <p>{integrityLevel === "full" ? "Full reads every frame. A two-hour film can take as long as playing it, and nothing is written." : "Quick checks the container, index, and a short decode. It catches most damaged MKVs in seconds."}</p>
</div>
{/if}

{#if mode !== "check" && mode !== "integrity"}
<section class="settings-card output-folder-settings" aria-labelledby="rebuild-output-title">
<div class="settings-card-heading"><div class="settings-icon blue"><FolderOpen size={18} /></div><div><h3 id="rebuild-output-title">Output folders</h3><p>Choose where new MKVs and short tests are written.</p></div></div>
<div class="upload-destination"><span>Default folder</span><strong>{appPreferences.rebuildOutputDir ? `${appPreferences.rebuildOutputDir} · source subfolders mirrored` : mode === "metadata" ? "Beside each source, named <name>.metadata.mkv" : mode === "test" ? "Beside each source, named Test_<name>" : "In place: the new file replaces the source after validation, and the original moves to old/"}</strong><div><button class="text-button" disabled={!desktopShell || workflowBusy} onclick={chooseRebuildOutput}>Choose folder</button><button class="text-button" disabled={!appPreferences.rebuildOutputDir || workflowBusy} onclick={() => void persistPreferences({ rebuildOutputDir: null })}>Use default location</button></div></div>
<p class="filename-filter-help">A folder set on a single file in Media Library wins over this default. Existing files are never overwritten.</p>
</section>
{/if}

{#if runBatches.length && (mode === "test" || mode === "full")}
<section class="run-review-settings card-surface" aria-labelledby="run-review-settings-title">
  <div class="run-review-settings-heading">
    <div>
      <div class="eyebrow">CURRENT RUN ONLY</div>
      <h2 id="run-review-settings-title">Review per-file choices</h2>
      <p>Adjust encoder quality, film-grain handling, HDR metadata, or copied tracks for this run without changing saved defaults.</p>
    </div>
    <span class="run-review-settings-count">{Object.keys(fileSettingsOverrides).filter((path) => selectedRunEntries.some((entry) => queuePathKey(entry.path) === queuePathKey(path))).length} files customized</span>
  </div>
  <div class="run-review-settings-actions">
    <button class="button primary" type="button" disabled={workflowBusy || !selectedRunEntries.length} onclick={() => openRunSettingsEditor(selectedRunEntries.map((entry) => entry.path))}><SlidersHorizontal size={16} />{selectedMediaPathsForRun.length ? "Adjust selected files" : "Adjust all files in this run"}<span>{selectedRunEntries.length}</span></button>
    {#each runBatches as batch (batch.source.root)}
      <button class="button secondary run-folder-button" type="button" disabled={workflowBusy || !batch.entries.length} onclick={() => openRunSettingsEditor(runEntriesForFolder(batch.source.root).map((entry) => entry.path))}><FolderOpen size={15} />Adjust {fileName(batch.source.root)}<span>{batch.entries.length}</span></button>
    {/each}
    {#if Object.keys(fileSettingsOverrides).some((path) => selectedRunEntries.some((entry) => queuePathKey(entry.path) === queuePathKey(path)))}
      <button class="text-button" type="button" disabled={workflowBusy} onclick={() => clearRunSettingsOverride(selectedRunEntries.map((entry) => entry.path))}>Clear current-run choices</button>
    {/if}
  </div>
  <p class="run-review-settings-note"><Info size={15} />Use a folder button for a whole source batch, the selected-files button for a mixed group, or <strong>Adjust</strong> beside one file below. Hover each setting in the editor for a short explanation of its FFmpeg effect.</p>
</section>
{/if}

{#if settings && (mode === "full" || mode === "test")}
<details class="run-encoder-details" open={mode === "full"}>
  <summary>{mode === "test" ? "Encoder used for the 5-second sample" : "Rebuild parameters"}</summary>
<section class="settings-intro" id="rebuild-parameters"><div class="section-title-row"><div><div class="eyebrow">EFFECTIVE CONFIGURATION</div><h2>Rebuild parameters</h2></div><div class="settings-intro-actions"><span class="edit-badge"><Settings2 size={14} /> {settingsChanged ? "Unsaved changes" : appPreferences.settings ? "Saved app profile" : "Library values"}</span><button class="text-button" disabled={(!settingsChanged && !appPreferences.settings) || workflowBusy} onclick={resetSettings}>Reset to library values</button><button class="text-button" disabled={!settingsChanged || workflowBusy} onclick={saveSettings}>Save as app defaults</button></div></div><p>Application, user, and library values load first. Save your edits as app defaults to keep them across launches; unsaved edits apply to the next run only.</p>{#if forecastOutdated}<div class="inline-warning"><Info size={17} /><div><strong>Resource settings changed after the folder scan.</strong><span>The per-file forecast on the library page is from that scan. Actual worker/frame counts are recomputed immediately before encoding each file.</span></div></div>{/if}</section>
<fieldset class="settings-grid" aria-label="Rebuild settings" disabled={workflowBusy}>
  <article class="settings-card"><div class="settings-card-heading"><div class="settings-icon purple"><Film size={18} /></div><div><h3>Picture encoding</h3><p>CPU libx265 <InfoTip label="About libx265" title={settingHelp("libx265").title} text={settingHelp("libx265").text} /> · video stream only</p></div></div>
    <label class="form-row range-row"><span><strong>Quality (CRF)</strong><InfoTip label="About CRF quality" title="What is CRF?" text={CRF_EXPLANATION} /><small>CRF {settings.video.crf} · {crfBand(settings.video.crf).label}. {crfBand(settings.video.crf).summary}</small><small class="crf-size-hint">{crfSizeHint(settings.video.crf)}</small></span><input aria-label="CRF quality" type="range" min="0" max="51" step="1" bind:value={settings.video.crf} /><b>{settings.video.crf}</b></label>
    <label class="form-row"><span><strong>Preset</strong><InfoTip label="About presets" title="What is a preset?" text="A preset trades time for size. Slower presets spend more computing effort to make a smaller file at the same quality. Slow is the recommended balance." /><small>{presetHelp(settings.video.preset)}</small></span><select aria-label="Preset" bind:value={settings.video.preset}><option value="ultrafast">ultrafast</option><option value="superfast">superfast</option><option value="veryfast">veryfast</option><option value="faster">faster</option><option value="fast">fast</option><option value="medium">medium</option><option value="slow">slow · recommended</option><option value="slower">slower</option><option value="veryslow">veryslow</option></select></label>
    <label class="toggle-row"><span><strong>Promote 8-bit sources to 10-bit</strong><InfoTip label="About 10-bit promotion" title="8-bit versus 10-bit" text="10-bit video has more shades per color, which reduces banding in dark gradients and compresses a little better, even when the source is 8-bit. It does not add detail that was never there, and 10-bit files need a 10-bit capable player." /><small>Off by default; does not add missing color detail</small></span><input type="checkbox" aria-label="Promote 8-bit sources to 10-bit" bind:checked={settings.video.promote_8bit_to_10bit} /></label>
    <label class="form-row"><span><strong>Content tune</strong><InfoTip label="About content tune" title={tuneHelp(settings.content.tune).title} text={tuneHelp(settings.content.tune).detail} /><small>{tuneHelp(settings.content.tune).summary}</small></span><select aria-label="Content tune" bind:value={settings.content.tune}><option value="auto">Automatic</option><option value="animation">Animation</option><option value="grain">Film grain</option><option value="none">Normal</option></select></label>
    <label class="toggle-row"><span><strong>Detect animation</strong><InfoTip label="About animation detection" title={settingHelp("detectAnimation").title} text={settingHelp("detectAnimation").text} /><small>Only when content tune is automatic</small></span><input type="checkbox" aria-label="Detect animation" bind:checked={settings.content.detect_animation} /></label>
    {#each encoderAdvisories({ tune: settings.content.tune, crf: settings.video.crf }) as advisory (advisory.text)}
      <div class="encoder-advisory" data-level={advisory.level} role="note"><Info size={16} aria-hidden="true" /><span>{advisory.text}</span></div>
    {/each}
  </article>

  <article class="settings-card"><div class="settings-card-heading"><div class="settings-icon blue"><ShieldCheck size={18} /></div><div><h3>HDR & stream safety</h3><p>Preserve by default. Never transcode audio or subtitles.</p></div></div>
    <label class="toggle-row"><span><strong>Preserve HDR10+ dynamic metadata</strong><InfoTip label="About HDR10+" title={settingHelp("hdr10plus").title} text={settingHelp("hdr10plus").text} /><small>Keeps the per-scene brightness data some HDR films carry</small></span><input type="checkbox" aria-label="Preserve HDR10+ dynamic metadata" bind:checked={settings.hdr.preserve_hdr10plus} /></label>
    <label class="form-row"><span><strong>Dolby Vision policy</strong><InfoTip label="About Dolby Vision handling" title={settingHelp("dolbyVision").title} text={settingHelp("dolbyVision").text} /><small>Safe skips unsupported profiles instead of damaging them</small></span><select aria-label="Dolby Vision policy" bind:value={settings.hdr.dolby_vision}><option value="safe">Safe preservation</option><option value="skip">Skip all Dolby Vision</option><option value="hdr10-fallback">Verified HDR10 fallback</option></select></label>
    <label class="form-row"><span><strong>Secondary video tracks</strong><InfoTip label="About extra video tracks" title={settingHelp("secondaryVideo").title} text={settingHelp("secondaryVideo").text} /><small>Choose how additional selected video tracks are handled</small></span><select aria-label="Secondary video tracks" bind:value={settings.streams.secondary_video}><option value="encode">Encode all selected tracks</option><option value="copy">Copy additional tracks</option><option value="drop">Keep primary track only</option></select></label>
    <label class="form-row"><span><strong>Data streams</strong><InfoTip label="About data streams" title={settingHelp("dataStreams").title} text={settingHelp("dataStreams").text} /><small>Preserve Matroska-compatible data</small></span><select aria-label="Data streams" bind:value={settings.streams.data_streams}><option value="preserve">Preserve</option><option value="drop">Drop</option></select></label>
    <label class="toggle-row"><span><strong>SHA-256 verify copied streams</strong><InfoTip label="About verifying copied tracks" title={settingHelp("copyVerification").title} text={settingHelp("copyVerification").text} /><small>Audio and subtitle packets are copied, never encoded</small></span><input type="checkbox" aria-label="SHA-256 verify copied streams" bind:checked={settings.safety.verify_copy_hashes} /></label>
    {#if settings.quality}
    <label class="form-row"><span><strong>Picture quality check</strong><InfoTip label="About the picture quality check" title={settingHelp("qualityCheck").title} text={settingHelp("qualityCheck").text} /><small>Compares sampled scenes of the new file with the original (SSIM) before it replaces anything. Enforce keeps the original when the picture falls below the minimum.</small></span><select aria-label="Picture quality check" bind:value={settings.quality.ssim_gate}><option value="enforce">Enforce · recommended</option><option value="report">Report only</option><option value="off">Off</option></select></label>
    <label class="form-row"><span><strong>Minimum SSIM</strong><InfoTip label="About the minimum SSIM score" title={settingHelp("minSsim").title} text={settingHelp("minSsim").text} /><small>0.95 catches broken output. Raise it for a stricter check; very grainy sources score lower without looking worse.</small></span><input aria-label="Minimum SSIM" type="number" min="0.5" max="1" step="0.01" bind:value={settings.quality.min_ssim} disabled={settings.quality.ssim_gate === "off"} /></label>
    {/if}
    <label class="text-setting"><span><strong>Matroska COMMENT</strong><small>Default comment for every new file. A series, season, episode, or film can keep its own.</small></span><input maxlength="500" bind:value={settings.metadata.comment} /></label>
  </article>

  <article class="settings-card wide-settings"><div class="settings-card-heading"><div class="settings-icon green-bg"><Gauge size={18} /></div><div><h3>Resource guardrails</h3><p>One file at a time · automatic memory and disk protection</p></div></div>
    <div class="resource-groups">
      <section>
        <h4>CPU</h4>
        <div class="resource-grid">
          <label><span>CPU policy</span><select bind:value={settings.resources.threading}><option value="auto">Automatic</option><option value="unlimited">Use all CPU threads · memory-limited</option></select></label>
          <label><span>Leave CPU threads free</span><input type="number" min="0" max="64" bind:value={settings.resources.reserved_cpu_threads} /></label>
          <label><span>Worker thread cap · 0 = auto</span><input type="number" min="0" max="64" bind:value={settings.resources.max_worker_threads} /></label>
          <label><span>Frame thread cap · 0 = auto</span><input type="number" min="0" max="16" bind:value={settings.resources.frame_threads} /></label>
        </div>
      </section>
      <section>
        <h4>Memory</h4>
        <div class="resource-grid">
          <label><span>Minimum free memory · GiB</span><input type="number" min="0" step="0.25" bind:value={settings.resources.min_free_memory_gib} /></label>
          <label><span>Memory reserve · percent</span><input type="number" min="0" max="50" step="0.5" bind:value={settings.resources.memory_reserve_percent} /></label>
        </div>
      </section>
      <section>
        <h4>Disk</h4>
        <div class="resource-grid">
          <label><span>Extra free-disk reserve · GiB</span><input type="number" min="0" step="0.5" bind:value={settings.resources.min_free_disk_gib} /></label>
          <label><span>Temporary output estimate · ratio</span><input type="number" min="0.25" max="4" step="0.05" bind:value={settings.resources.preflight_output_ratio} /></label>
          <label><span>Live size projection margin · percent</span><input type="number" min="0" max="100" step="1" bind:value={settings.resources.disk_projection_margin_percent} /></label>
        </div>
      </section>
      <section>
        <h4>Calibration</h4>
        <div class="resource-grid">
          <label><span>Hardware calibration</span><select bind:value={settings.calibration.mode}><option value="auto">Automatic · 4K/large files</option><option value="ask">Ask in interactive CLI when uncached</option><option value="always">Every file · benchmark each time</option><option value="off">Disabled</option></select></label>
          <label><span>Calibration cache · hours</span><input type="number" min="0" max="720" step="1" bind:value={settings.calibration.cache_hours} /></label>
          <label><span>Large-file threshold · GiB</span><input type="number" min="0" max="1000" step="1" bind:value={settings.calibration.large_file_gib} /></label>
          <label><span>Sample duration · seconds</span><input type="number" min="1" max="15" step="1" bind:value={settings.calibration.sample_seconds} /></label>
          <label><span>Maximum layouts to compare</span><input type="number" min="1" max="4" step="1" bind:value={settings.calibration.max_candidates} /></label>
          <label><span>Throughput tie margin · percent</span><input type="number" min="0" max="20" step="0.5" bind:value={settings.calibration.throughput_tolerance_percent} /></label>
        </div>
      </section>
      <div class="hardware-note"><Cpu size={17} /><span>Detected: <strong>{environment?.hardware.cpu_model ?? "CPU details unavailable"}</strong>. Automatic calibration samples 4K and large files, then caches the worker and frame counts. RAM, swap, and memory-stall checks stay on during the encode. Swap already on disk stays out of the free-memory count.</span></div>
    </div>
  </article>
</fieldset>
<div class="config-source"><Info size={14} /><span title={settingsSources.join(" · ")}>Configuration: {settingsSources.length ? settingsSources.map((source) => fileName(source)).join(" · ") : fileName(settingsPath) || "application defaults"}{preferenceStorage ? ` · ${fileName(preferenceStorage.settingsFile)}` : !desktopShell ? " · browser preview" : ""}</span><button class="text-button" disabled={workflowBusy} onclick={() => loadSettings(root)}>Reload effective values</button></div>
</details>
{/if}

{#if selectionInOtherSources}<div class="inline-warning"><Info size={17} /><div><strong>This workflow will process {runBatches.length} queued sources.</strong><span>{workspace.queue.selectedPaths.length ? `${selectedRunEntries.length} selected files` : `${selectedRunEntries.length} queued files`} will run sequentially, with each source’s saved scan settings.</span></div></div>{/if}
{#if runBatches.length}<details class="queue-run-plan"><summary>Review {selectedRunEntries.length} files in {runBatches.length} source batch{runBatches.length === 1 ? "" : "es"}</summary>{#each runBatches as batch (batch.source.root)}{@const previewEntries = runPlanPreviewByRoot.get(batch.source.root) ?? []}<details class="queue-run-source"><summary><strong>{fileName(batch.source.root)}</strong><span>{batch.entries.length} file{batch.entries.length === 1 ? "" : "s"}</span><code title={batch.source.root}>{batch.source.root}</code></summary><ul>{#each previewEntries as entry (entry.path)}<li><div><strong>{entry.name}</strong><code title={entry.path}>{entry.path}</code>{#if mode !== "metadata" && mode !== "youtube"}<small class="queue-run-override-summary">{runOverrideSummary(entry)}</small>{/if}</div>{#if mode !== "metadata" && mode !== "youtube"}<button class="text-button" type="button" disabled={workflowBusy} onclick={(event) => { event.stopPropagation(); openRunSettingsEditor([entry.path]); }}>Adjust</button>{/if}</li>{/each}</ul></details>{/each}{#if selectedRunEntries.length > 250}<small>Showing the first 250 files across the queue plan. All {selectedRunEntries.length} files remain included; use the source or selected-files controls above to edit the rest.</small>{/if}</details>{/if}
{#if unresolvedSelectedPaths > 0}<div class="inline-warning"><AlertTriangle size={17} /><div><strong>{unresolvedSelectedPaths} selected file{unresolvedSelectedPaths === 1 ? " is" : "s are"} not available from a completed scan.</strong><span>Refresh or remove that source before starting. Ready files can be kept in the selection.</span></div><button class="text-button" onclick={keepReadyFiles}>Keep ready files</button></div>{/if}
{#if staleSelectedEntries.length}<div class="inline-warning"><AlertTriangle size={17} /><div><strong>Refresh {staleRunSources.length} affected source{staleRunSources.length === 1 ? "" : "s"} before running.</strong><span>{staleSelectedEntries.length} file{staleSelectedEntries.length === 1 ? " has" : "s have"} changed or disappeared since scanning. Refresh only the source groups shown here.</span><details><summary>Sources to refresh</summary>{#each staleRunSources as batch (batch.source.root)}<button class="text-button" disabled={workflowBusy} onclick={() => refreshQueueSource(batch.source.root)}>Refresh {fileName(batch.source.root)}</button>{/each}</details></div></div>{/if}
<section class="run-launch card-surface"><div class="launch-left"><div class="launch-icon"><TerminalSquare size={19} /></div><div><strong>{workspace.run.id ? "A workflow is running" : mode === "full" ? "Full rebuild requires confirmation" : mode === "test" ? "Short test run" : mode === "metadata" ? "Create validated MKV copies without re-encoding" : mode === "youtube" ? "Create a YouTube upload copy" : mode === "integrity" ? (integrityLevel === "full" ? "Full decode integrity check" : "Quick integrity check") : "Ready for a check-only pass"}</strong><span>{scanResult ? `${selectedRunEntries.length} file${selectedRunEntries.length === 1 ? "" : "s"}${selectedMediaPathsForRun.length ? " selected" : ""} · ${bytes(selectedRunInputBytes)} total` : "Add a file or folder to the queue"}{mode !== "metadata" && mode !== "check" && mode !== "integrity" && environment?.hardware.gpu_devices.length ? ` · ${environment.hardware.gpu_devices.length} GPU detected, CPU encoder selected` : ""}</span></div></div>
<label class="check-label stop-error"><input type="checkbox" bind:checked={stopOnError} onchange={() => void persistPreferences({ stopOnError })} /> Stop queue on first error</label>
{#if workspace.run.id}<div class="run-stop-pair"><button class="button danger-outline" onclick={stopAfterCurrent}><Square size={15} /> Stop after this file</button>{#if desktopShell && mutatingRun}<button class="button danger" type="button" onclick={onStopNow} disabled={workspace.run.aborting}><OctagonX size={15} /> Stop now</button>{/if}</div>{:else}<button bind:this={runLaunchButton} class="button primary" disabled={!canStart} onclick={beginRun}>{#if mode === "full"}<ShieldCheck size={17} />{:else if mode === "test"}<Play size={17} />{:else if mode === "metadata"}<FileText size={17} />{:else if mode === "youtube"}<Video size={17} />{:else}<ScanSearch size={17} />{/if}{mode === "full" ? "Review & start full rebuild" : mode === "test" ? "Review & start short tests" : mode === "metadata" ? "Review & create metadata MKVs" : mode === "youtube" ? "Create upload copies" : mode === "integrity" ? (integrityLevel === "full" ? "Start full integrity check" : "Start quick integrity check") : "Start check-only pass"}</button>{/if}
</section>

{#if workspace.run.message || workspace.run.lines.length}
<RunConsole run={workspace.run} title="Run activity" elapsedLabel={progressElapsedLabel} onStop={stopAfterCurrent} />
{/if}
