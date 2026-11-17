<!-- ./desktop/src/lib/views/EnvironmentPanel.svelte
     Toolchain health, executable path overrides, APT/Homebrew/winget setup and
     the repair activity inside Settings. Pure view: state is route-owned and
     every action is a callback into the tool-setup controller or the route. -->
<script lang="ts">
  import { AlertTriangle, ArrowRight, Check, CheckCircle2, Copy, Cpu, Database, HardDrive, Info, LoaderCircle, MemoryStick, Monitor, RefreshCw, Wrench, X, Zap } from "@lucide/svelte";
  import ToolRepairActivity from "../ToolRepairActivity.svelte";
  import type { createToolSetupController } from "../tool-setup-controller.js";
  import type { AptUpdateStatus, Environment, Page, Settings } from "../types.js";
  import type { RepairToolOutcome, ToolSetupLogSummary } from "../ui-contracts.js";

  type ToolSetup = ReturnType<typeof createToolSetupController>;
  type ToolPathKey = "ffmpeg" | "ffprobe" | "mediainfo" | "mkvpropedit" | "mkvmerge" | "mkvextract";

  interface Props {
    environment: Environment | null;
    environmentLoading: boolean;
    environmentError: string;
    booting: boolean;
    desktopShell: boolean;
    workflowBusy: boolean;
    settings: Settings | null;
    aptHost: boolean;
    aptChecking: boolean;
    aptUpdates: AptUpdateStatus | null;
    packageManagerLabel: string;
    hostSetupShell: string;
    hostSetupCommand: string;
    repairId: string;
    repairSummary: string;
    repairSummaryIsError: boolean;
    repairLogWarning: string;
    repairLines: string[];
    repairToolOutcomes: RepairToolOutcome[];
    toolSetupLogs: ToolSetupLogSummary[];
    selectedToolSetupLog: string;
    viewedToolSetupLog: string;
    toolSetupLogLoading: boolean;
    toolSetupLogError: string;
    toolSetupLogExporting: boolean;
    toolPathOptions: readonly { key: ToolPathKey; label: string; note: string }[];
    toolPathValue: (key: ToolPathKey) => string;
    setToolPathValue: (key: ToolPathKey, value: string) => void;
    chooseToolPath: (key: ToolPathKey, label: string) => unknown;
    clearToolPath: (key: ToolPathKey) => unknown;
    saveToolPaths: (recheck?: boolean) => unknown;
    loadEnvironment: ToolSetup["loadEnvironment"];
    checkUbuntuPackageUpdates: ToolSetup["checkUbuntuPackageUpdates"];
    reviewSetup: ToolSetup["reviewSetup"];
    packageUpdateForTool: ToolSetup["packageUpdateForTool"];
    openToolSetupLog: ToolSetup["openToolSetupLog"];
    exportToolSetupLog: ToolSetup["exportToolSetupLog"];
    copyHostSetupCommand: () => unknown;
    openPage: (page: Page) => void;
  }
  let {
    environment, environmentLoading, environmentError, booting, desktopShell, workflowBusy, settings, aptHost, aptChecking, aptUpdates, packageManagerLabel,
    hostSetupShell, hostSetupCommand, repairId, repairSummary, repairSummaryIsError, repairLogWarning, repairLines, repairToolOutcomes, toolSetupLogs,
    selectedToolSetupLog = $bindable(), viewedToolSetupLog = $bindable(), toolSetupLogLoading, toolSetupLogError, toolSetupLogExporting, toolPathOptions, toolPathValue,
    setToolPathValue, chooseToolPath, clearToolPath, saveToolPaths, loadEnvironment, checkUbuntuPackageUpdates, reviewSetup,
    packageUpdateForTool, openToolSetupLog, exportToolSetupLog, copyHostSetupCommand, openPage,
  }: Props = $props();
</script>

<section id="environment-capabilities" class="environment-section"><div class="section-title-row"><div><div class="eyebrow">MACHINE AWARENESS</div><h2>Environment & capabilities</h2></div><button class="text-button" onclick={loadEnvironment} disabled={environmentLoading}><RefreshCw size={14} class={environmentLoading ? "spin" : ""} /> Recheck</button></div>
  {#if environment}
    <div class="environment-layout"><div class="hardware-card"><div class="hardware-topline"><span class="hardware-avatar"><Monitor size={18} /></span><span class="hardware-os">{environment.os.name} {environment.os.version}</span><span class="status-tag" class:status-ready={environment.os.supported} class:status-error={!environment.os.supported}>{environment.os.supported ? "SUPPORTED" : "UNSUPPORTED"}</span></div><h3>{environment.hardware.cpu_model}</h3><div class="hardware-specs"><span><Cpu size={15} /> {environment.hardware.logical_cpus} logical CPUs</span><span><MemoryStick size={15} /> {environment.hardware.memory_total_gib?.toFixed(1) ?? "—"} GiB effective RAM</span><span><HardDrive size={15} /> {environment.hardware.memory_available_gib?.toFixed(1) ?? "—"} GiB available</span><span><Database size={15} /> Swap {environment.hardware.swap_used_gib?.toFixed(1) ?? "—"}/{environment.hardware.swap_total_gib?.toFixed(1) ?? "—"} GiB used{environment.hardware.swap_used_percent !== null ? ` (${environment.hardware.swap_used_percent.toFixed(0)}%)` : ""} · {environment.hardware.swap_free_gib?.toFixed(1) ?? "—"} free</span></div>
        <div class="gpu-strip"><Zap size={15} /><span>{environment.hardware.gpu_devices.length ? environment.hardware.gpu_devices.join(" · ") : "No GPU device detected"}</span><small>DETECTED · NOT USED FOR ENCODING</small></div>
      </div><div class="tool-check-card"><div class="tool-check-heading"><strong>Toolchain health</strong><span>{environment.checks.filter((item) => item.available).length}/{environment.checks.length} ready</span></div><div class="tool-check-list">{#each environment.checks as check (check.key)}{@const packageUpdate = packageUpdateForTool(check.key)}{@const pathOverride = toolPathOptions.find((tool) => tool.key === check.key && Boolean(toolPathValue(tool.key).trim()))}<div class="tool-check-row"><span class="check-mark" class:check-bad={!check.available}>{#if check.available}<Check size={13} />{:else}<X size={13} />{/if}</span><span class="tool-check-copy"><strong>{check.label}</strong><small>{pathOverride ? `Custom override · ${check.path ?? toolPathValue(pathOverride.key)}` : check.path ?? check.detail}</small>{#if pathOverride}<small class="tool-override-note">This app uses the selected path; package updates do not replace it.</small>{/if}{#if packageUpdate}<small class="tool-update-version">APT {packageUpdate.installed_version} → {packageUpdate.candidate_version}</small>{/if}</span><span class="tool-version" class:tool-update-available={Boolean(packageUpdate)}>{#if packageUpdate}Update available{:else}{check.available ? check.version?.replace(/^.*?\s/, "") ?? "Ready" : check.required ? "Required" : "Optional"}{/if}</span></div>{/each}</div>
        {#if !environment.can_analyze && environment.checks.some((check) => check.key === "ffprobe" && check.available)}
          <div class="encoder-note" role="status"><Info size={15} /><span><strong>Media scans are available.</strong> FFprobe can read supported files now. Without MediaInfo, some track details stay partial; rebuild and YouTube jobs remain unavailable until their required tools are ready.</span></div>
        {/if}
      </div></div>
    <div class="tool-path-settings">
      <div class="tool-path-heading"><div><strong>Executable paths</strong><p>Leave a field blank to use PATH. A selected override stays saved and is never replaced by package setup; if FFprobe is blank, a matching FFmpeg sibling is detected when available.</p></div><button class="button secondary" disabled={!desktopShell || workflowBusy || environmentLoading} onclick={() => saveToolPaths()}><RefreshCw size={15} /> Save paths & check</button></div>
      <div class="tool-path-grid">
        {#each toolPathOptions as tool (tool.key)}
          <div class="tool-path-row"><label for={`tool-path-${tool.key}`}><span>{tool.label}</span><small>{tool.note}</small></label><div class="tool-path-control"><input id={`tool-path-${tool.key}`} type="text" value={toolPathValue(tool.key)} placeholder={`Use ${tool.key} from PATH`} disabled={!desktopShell || workflowBusy || environmentLoading} oninput={(event) => setToolPathValue(tool.key, event.currentTarget.value)} /><button class="text-button" disabled={!desktopShell || workflowBusy || environmentLoading} onclick={() => chooseToolPath(tool.key, tool.label)}>Browse</button>{#if toolPathValue(tool.key)}<button class="text-button" disabled={workflowBusy || environmentLoading} onclick={() => clearToolPath(tool.key)}>PATH</button>{/if}</div></div>
        {/each}
      </div>
    </div>
    <section class="apt-update-panel">
      {#if aptHost}
      <div class="section-title-row"><div><div class="eyebrow">SYSTEM TOOL SETUP</div><h3>Installed versions and updates</h3><p>Versions are checked at launch against the enabled Ubuntu APT sources. One button installs and updates everything, including the HDR10+ helper, and asks for your password at most once. Repository versions may lag upstream.</p></div><div class="apt-action-buttons"><button class="button secondary" disabled={!desktopShell || workflowBusy || aptChecking} onclick={() => checkUbuntuPackageUpdates()}>{#if aptChecking}<LoaderCircle class="spin" size={15} /> Checking…{:else}<RefreshCw size={15} /> Check versions{/if}</button><button class="button primary" disabled={!desktopShell || workflowBusy || aptChecking} onclick={() => reviewSetup()}><Wrench size={15} /> Install & update everything</button></div></div>
      {#if aptUpdates}
        <div class="apt-update-status"><span class="status-tag" class:status-warning={aptUpdates.catalog_stale} class:status-ready={!aptUpdates.catalog_stale}>{aptUpdates.catalog_stale ? "CATALOG NEEDS REFRESH" : `${aptUpdates.packages.length} UPDATE${aptUpdates.packages.length === 1 ? "" : "S"}`}</span><small>Catalog last updated: {aptUpdates.catalog_updated_at ? new Date(aptUpdates.catalog_updated_at * 1000).toLocaleString() : "unknown"} · checked {new Date(aptUpdates.checked_at * 1000).toLocaleString()}</small></div>
        {#if aptUpdates.catalog_stale}
          <div class="inline-warning apt-update-warning"><Info size={17} /><div><strong>These versions may be out of date.</strong><span>Install & update everything refreshes the package catalog and applies the plan in one authenticated step, after showing you the list. Refreshing downloads package-index metadata; it does not install software.</span></div></div>
        {:else if aptUpdates.packages.length}
          <div class="apt-update-section-label"><strong>Updates available</strong><small>Installed version → enabled repository version</small></div>
          <div class="apt-update-list">{#each aptUpdates.packages as update (update.name)}<div><code>{update.name}</code><span>{update.installed_version}</span><ArrowRight size={13} /><strong>{update.candidate_version}</strong></div>{/each}</div>
        {:else}
          <div class="apt-no-updates"><CheckCircle2 size={16} /><span>No updates are available for the checked packages.</span></div>
        {/if}
        {#if environment.missing_apt_packages.length}<div class="apt-update-section-label"><strong>Missing packages</strong><small>Included in the same reviewed setup</small></div><div class="repair-package-list">{#each environment.missing_apt_packages as pkg (pkg)}<code>{pkg}</code>{/each}</div>{/if}
        {#if environment.checks.some((check) => check.key === "hdr10plus_tool" && !check.available)}<div class="repair-helper-note"><Info size={16} /><span><strong>HDR10+ helper missing.</strong> The setup action installs the checksum-verified upstream helper after APT finishes.</span></div>{/if}
      {:else}
        <p class="apt-update-empty">Checking FFmpeg, MediaInfo, MKVToolNix, Fontconfig, and certificate packages against the local APT catalog…</p>
      {/if}
      {:else}
        <div class="section-title-row"><div><div class="eyebrow">HOST TOOL SETUP</div><h3>Install the media tools</h3><p>{hostSetupCommand ? `Run this in ${hostSetupShell}, then choose Recheck. Packages that are already installed are left as they are.` : `${environment.os.note}. Install FFmpeg, MediaInfo, and MKVToolNix, then choose Recheck or select each executable above.`}</p></div></div>
        {#if hostSetupCommand}
          <pre class="host-setup-command"><code>{hostSetupCommand}</code><button type="button" class="button secondary" onclick={() => copyHostSetupCommand()}><Copy size={15} /> Copy</button></pre>
          {#if environment.os.package_manager_available === false}<p class="apt-update-empty">{packageManagerLabel} was not found on this computer. {packageManagerLabel === "Homebrew" ? "Install Homebrew from brew.sh first." : "Install App Installer from the Microsoft Store to get winget."}</p>{/if}
        {/if}
        {#if environment.checks.some((check) => check.key === "hdr10plus_tool" && !check.available)}<div class="repair-helper-note"><Info size={16} /><span><strong>HDR10+ helper missing.</strong> Only HDR10+ sources need it. Download <a href="https://github.com/quietvoid/hdr10plus_tool/releases/latest" target="_blank" rel="noreferrer">hdr10plus_tool</a> and select it above.</span></div>{/if}
      {/if}
      <ToolRepairActivity showLogHistory={true} {repairSummary} {repairSummaryIsError} {repairId} {repairToolOutcomes} {repairLogWarning} {repairLines} {toolSetupLogs} bind:selectedToolSetupLog {toolSetupLogLoading} bind:viewedToolSetupLog {toolSetupLogExporting} {toolSetupLogError} {openToolSetupLog} {exportToolSetupLog} />
    </section>
    <div class="encoder-note"><Info size={15} /><span>Hardware inventory is used for resource planning and diagnostics. The current rebuild engine uses <strong>CPU libx265</strong>; GPU encoding is not enabled.</span><button class="text-button" onclick={() => openPage("run")}>Resource settings <ArrowRight size={14} /></button></div>
  {:else}<div class="empty-state compact">{#if environmentLoading || booting}<LoaderCircle class="spin" size={18} />{:else}<AlertTriangle size={18} />{/if}<div><strong>{environmentLoading || booting ? "Checking this computer…" : "Could not read the environment"}</strong><p>{environmentError || "The desktop backend may be unavailable. Recheck after setup."}</p></div>{#if !environmentLoading}<button class="button secondary" onclick={loadEnvironment}>Retry check</button>{/if}</div>{/if}
</section>
