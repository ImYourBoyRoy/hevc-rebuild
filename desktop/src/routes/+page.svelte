<script lang="ts">
  import { onMount } from "svelte";
  import { invoke, isTauri } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import {
    Activity, AlertTriangle, ArrowRight, Check, CheckCircle2, ChevronRight,
    Clapperboard, Cpu, Database, FileText, Film, FolderOpen, Gauge, HardDrive,
    History, Info, Layers, LoaderCircle, MemoryStick, Monitor, Moon, Play, RefreshCw,
    ScanSearch, Search, Settings2, ShieldCheck, Square, Sun, Terminal, TerminalSquare, Timer,
    Video, Wrench, X, Zap,
  } from "@lucide/svelte";
  import type {
    Environment, MediaEntry, Page, ReportRow, RunFinishedEvent, RunLineEvent,
    RunMode, RunReport, ScanProgress, ScanResult, Settings,
  } from "$lib/types";

  type SettingsLoad = { settings: Settings; sources: string[]; user_config: string };
  type ReportsPayload = { reports: RunReport[] };
  type RepairFinished = { jobId: string; success: boolean; error: string | null };
  type Theme = "light" | "dark" | "hacker";
  type AppPreferences = {
    theme: Theme;
    root: string | null;
    ffmpegPath: string | null;
    recursive: boolean;
    minimumSizeMib: string;
    stopOnError: boolean;
    settings: Settings | null;
  };
  type PreferencesPayload = {
    preferences: AppPreferences;
    settingsFile: string;
    hasSavedSettings: boolean;
    portable: boolean;
    storageWarning: string | null;
  };

  const navigation: { id: Page; label: string; icon: typeof Activity }[] = [
    { id: "overview", label: "Overview", icon: Activity },
    { id: "library", label: "Media library", icon: Film },
    { id: "run", label: "Run & settings", icon: Settings2 },
    { id: "reports", label: "Reports & errors", icon: History },
  ];

  let page = $state<Page>("overview");
  let theme = $state<Theme>("light");
  let appPreferences = $state<AppPreferences>({ theme: "light", root: null, ffmpegPath: null, recursive: true, minimumSizeMib: "5", stopOnError: false, settings: null });
  let preferenceStorage = $state<PreferencesPayload | null>(null);
  let preferenceSaveQueue: Promise<void> = Promise.resolve();
  let booting = $state(true);
  let desktopShell = $state(false);
  let environment = $state<Environment | null>(null);
  let environmentLoading = $state(false);
  let environmentError = $state("");
  let root = $state("");
  let recursive = $state(true);
  let minimumSizeMib = $state("5");
  let ffmpegPath = $state("");
  let settings = $state<Settings | null>(null);
  let settingsSources = $state<string[]>([]);
  let settingsPath = $state("");
  let settingsBaseline = $state("");
  let librarySettingsBaseline = $state("");
  let settingsRoot = $state("");
  let scanning = $state(false);
  let scanProgress = $state<ScanProgress | null>(null);
  let scanResult = $state<ScanResult | null>(null);
  let search = $state("");
  let mode = $state<RunMode>("check");
  let stopOnError = $state(false);
  let activeRunId = $state("");
  let runStarting = $state(false);
  let runLines = $state<string[]>([]);
  let queuedRunLines: RunLineEvent[] = [];
  let queuedRunFinished: RunFinishedEvent | null = null;
  let runMessage = $state("");
  let fullConfirmOpen = $state(false);
  let fullAck = $state(false);
  let repairConfirmOpen = $state(false);
  let reports = $state<RunReport[]>([]);
  let selectedReportPath = $state("");
  let selectedLog = $state("");
  let logLoading = $state(false);
  let repairId = $state("");
  let repairLines = $state<string[]>([]);
  let hdrInstalling = $state(false);
  let notice = $state("");
  let noticeKind = $state<"info" | "success" | "error">("info");
  let unlisteners: UnlistenFn[] = [];

  let visibleEntries = $derived((scanResult?.entries ?? []).filter((entry) => {
    const query = search.trim().toLowerCase();
    return !query || entry.relative_path.toLowerCase().includes(query) || entry.status.toLowerCase().includes(query) || (entry.resource_forecast?.level ?? "").includes(query);
  }));
  let analysisErrors = $derived((scanResult?.entries ?? []).filter((entry) => entry.error));
  let highMemoryEntries = $derived((scanResult?.entries ?? []).filter((entry) => entry.resource_forecast?.level === "high" || entry.resource_forecast?.level === "blocked"));
  let constrainedMemoryEntries = $derived((scanResult?.entries ?? []).filter((entry) => entry.resource_forecast?.level === "constrained"));
  let failedRows = $derived((reports.flatMap((report) => report.rows)).filter((row) => isFailure(row.status)));
  let activeReport = $derived(reports.find((report) => report.csv_path === selectedReportPath) ?? reports[0] ?? null);
  let settingsChanged = $derived(Boolean(settings && settingsBaseline && JSON.stringify(settings) !== settingsBaseline));
  let forecastOutdated = $derived(Boolean(scanResult && settings && (
    JSON.stringify(scanResult.settings.resources) !== JSON.stringify(settings.resources) ||
    scanResult.settings.video.preset !== settings.video.preset ||
    scanResult.settings.video.promote_8bit_to_10bit !== settings.video.promote_8bit_to_10bit
  )));
  let workflowBusy = $derived(Boolean(scanning || runStarting || activeRunId || repairId || hdrInstalling));
  let canStart = $derived(Boolean(root.trim() && environment?.ready && scanResult && scanResult.candidate_count > 0 && !scanning && !runStarting && !activeRunId && !repairId && !hdrInstalling));

  function setNotice(message: string, kind: "info" | "success" | "error" = "info") {
    notice = message;
    noticeKind = kind;
  }

  function applyTheme(value: string) {
    theme = value === "dark" || value === "hacker" ? value : "light";
    document.documentElement.dataset.theme = theme;
    void persistPreferences({ theme });
  }

  function persistPreferences(patch: Partial<AppPreferences>, successMessage?: string) {
    appPreferences = { ...appPreferences, ...patch };
    const snapshot = JSON.parse(JSON.stringify(appPreferences)) as AppPreferences;
    const operation = preferenceSaveQueue.catch(() => {}).then(async () => {
      preferenceStorage = await invoke<PreferencesPayload>("save_app_preferences", { preferences: snapshot });
      if (successMessage) setNotice(successMessage, "success");
    });
    preferenceSaveQueue = operation.then(() => {}, () => {});
    return operation.then(() => true, (error) => {
      setNotice(`Could not save application settings: ${String(error)}`, "error");
      return false;
    });
  }

  function isFailure(status: string | undefined) {
    const normalized = (status ?? "").toLowerCase();
    return normalized.includes("fail") || normalized.includes("error") || normalized.includes("skip");
  }

  function bytes(value: number | null | undefined) {
    if (value == null || !Number.isFinite(value)) return "—";
    const units = ["B", "KB", "MB", "GB", "TB"];
    let size = value;
    let unit = 0;
    while (size >= 1024 && unit < units.length - 1) {
      size /= 1024;
      unit += 1;
    }
    return `${size.toFixed(unit > 1 ? 1 : 0)} ${units[unit]}`;
  }

  function duration(seconds: number | undefined) {
    if (!seconds || !Number.isFinite(seconds)) return "—";
    const minutes = Math.floor(seconds / 60);
    const remaining = Math.round(seconds % 60);
    return minutes ? `${minutes}m ${remaining}s` : `${remaining}s`;
  }

  function fileName(path: string) {
    return path.split(/[\\/]/).filter(Boolean).at(-1) ?? path;
  }

  function profileSummary(entry: MediaEntry) {
    if (!entry.profile) return "Profile unavailable";
    const p = entry.profile;
    const hdr = p.hdr.dolby_vision ? "Dolby Vision" : p.hdr.hdr10plus ? "HDR10+" : p.hdr.hdr10 ? "HDR10" : p.hdr.hlg ? "HLG" : "SDR";
    return `${p.video.width} × ${p.video.height} · ${p.video.source_bit_depth}-bit · ${hdr}`;
  }

  function statusLabel(status: string) {
    if (status === "Ready") return "Ready";
    if (status === "Backed up") return "Has backup";
    if (status === "Tools required") return "Tools needed";
    return status;
  }

  async function loadEnvironment() {
    environmentLoading = true;
    environmentError = "";
    try {
      environment = await invoke<Environment>("inspect_environment", { ffmpegPath: ffmpegPath.trim() || null });
    } catch (error) {
      environmentError = String(error);
      environment = null;
    } finally {
      environmentLoading = false;
    }
  }

  async function loadSettings(folder = root) {
    if (!folder.trim()) return;
    try {
      const payload = await invoke<SettingsLoad>("load_settings", { root: folder.trim() });
      librarySettingsBaseline = JSON.stringify(payload.settings);
      settings = payload.settings;
      settingsBaseline = JSON.stringify(settings);
      settingsRoot = folder.trim();
      settingsSources = payload.sources;
      settingsPath = payload.user_config;
    } catch (error) {
      setNotice(`Could not load the effective settings: ${String(error)}`, "error");
    }
  }

  async function resetSettings() {
    if (!settingsBaseline || workflowBusy) return;
    try {
      if (appPreferences.settings) {
        const folder = root.trim() || settingsRoot;
        if (await persistPreferences({ settings: null }, "Removed the saved application profile.") && folder) await loadSettings(folder);
      } else {
        settings = JSON.parse(settingsBaseline) as Settings;
        setNotice("Restored the effective settings for this library.", "success");
      }
    } catch (error) {
      setNotice(`Could not restore the effective settings: ${String(error)}`, "error");
    }
  }

  async function saveSettings() {
    if (!settings || !settingsChanged || workflowBusy) return;
    const snapshot = JSON.parse(JSON.stringify(settings)) as Settings;
    if (await persistPreferences({ settings: snapshot }, "Saved these rebuild settings as this app's defaults.")) {
      settingsBaseline = JSON.stringify(snapshot);
      librarySettingsBaseline = librarySettingsBaseline || settingsBaseline;
      if (!settingsSources.includes("Saved application profile")) settingsSources = [...settingsSources, "Saved application profile"];
    }
  }

  async function loadReports() {
    try {
      const payload = await invoke<ReportsPayload>("list_reports", { limit: 20 });
      reports = payload.reports;
      if (!reports.some((report) => report.csv_path === selectedReportPath)) {
        selectedReportPath = reports[0]?.csv_path ?? "";
      }
    } catch (error) {
      setNotice(`Could not read report history: ${String(error)}`, "error");
    }
  }

  async function initialize() {
    try {
      const stored = await invoke<PreferencesPayload>("load_app_preferences");
      preferenceStorage = stored;
      appPreferences = stored.preferences;
      const migration: Partial<AppPreferences> = {};
      if (!stored.hasSavedSettings) {
        try {
          const oldTheme = localStorage.getItem("hevc-rebuild-theme");
          const oldRoot = localStorage.getItem("hevc-rebuild.root");
          const oldFfmpeg = localStorage.getItem("hevc-rebuild.ffmpeg");
          const oldRecursive = localStorage.getItem("hevc-rebuild.recursive");
          const oldMinimum = localStorage.getItem("hevc-rebuild.minimumSizeMib");
          if (oldTheme === "light" || oldTheme === "dark" || oldTheme === "hacker") migration.theme = oldTheme;
          if (oldRoot) migration.root = oldRoot;
          if (oldFfmpeg) migration.ffmpegPath = oldFfmpeg;
          if (oldRecursive !== null) migration.recursive = oldRecursive !== "false";
          if (oldMinimum !== null) migration.minimumSizeMib = oldMinimum;
        } catch { /* Native preferences remain available if legacy browser storage is disabled. */ }
      }
      appPreferences = { ...appPreferences, ...migration };
      theme = appPreferences.theme;
      document.documentElement.dataset.theme = theme;
      ffmpegPath = appPreferences.ffmpegPath ?? "";
      minimumSizeMib = appPreferences.minimumSizeMib;
      recursive = appPreferences.recursive;
      stopOnError = appPreferences.stopOnError;
      root = appPreferences.root ?? await invoke<string>("default_media_root");
      if (!stored.hasSavedSettings || migration.root === undefined) {
        const saved = await persistPreferences({ ...migration, root });
        if (saved && !stored.hasSavedSettings) {
          try {
            ["hevc-rebuild-theme", "hevc-rebuild.root", "hevc-rebuild.ffmpeg", "hevc-rebuild.recursive", "hevc-rebuild.minimumSizeMib"].forEach((key) => localStorage.removeItem(key));
          } catch { /* The native preference file is already authoritative. */ }
        }
      } else if (Object.keys(migration).length) {
        await persistPreferences(migration);
      }
      if (stored.storageWarning) setNotice(stored.storageWarning, "error");
      await Promise.all([loadEnvironment(), loadSettings(root), loadReports()]);
    } catch (error) {
      environmentError = String(error);
      setNotice(`The desktop backend did not start: ${String(error)}`, "error");
    } finally {
      booting = false;
    }
  }

  async function browseFolder() {
    if (!desktopShell || activeRunId || runStarting || repairId || hdrInstalling || scanning) return;
    const picked = await openDialog({ directory: true, multiple: false, title: "Choose a media library" });
    if (typeof picked !== "string") return;
    root = picked;
    scanResult = null;
    await persistPreferences({ root });
    await loadSettings(root);
    setNotice("Folder selected. Analyze it to build a safe, file-by-file plan.", "success");
  }

  async function chooseFfmpeg() {
    if (!desktopShell || activeRunId || runStarting || repairId || hdrInstalling) return;
    const picked = await openDialog({ directory: false, multiple: false, title: "Choose an FFmpeg executable" });
    if (typeof picked !== "string") return;
    ffmpegPath = picked;
    await persistPreferences({ ffmpegPath });
    scanResult = null;
    await loadEnvironment();
    setNotice("Selected FFmpeg was checked. Its sibling FFprobe is used when available.", "success");
  }

  async function clearFfmpegOverride() {
    if (activeRunId || runStarting || repairId || hdrInstalling) return;
    ffmpegPath = "";
    await persistPreferences({ ffmpegPath: null });
    scanResult = null;
    await loadEnvironment();
    setNotice("Using the FFmpeg executable found on PATH.", "success");
  }

  async function analyzeFolder() {
    if (!desktopShell) {
      setNotice("Open HEVC Rebuild Studio from the desktop app to connect to system tools.", "error");
      return;
    }
    if (activeRunId || runStarting || repairId || hdrInstalling || scanning) return;
    if (!root.trim()) {
      setNotice("Choose a media folder first.", "error");
      return;
    }
    scanning = true;
    const preserveRunSettings = settingsChanged && settingsRoot === root.trim() ? settings : null;
    scanProgress = null;
    scanResult = null;
    setNotice("Analyzing folders and media streams. Source files are only read.", "info");
    await persistPreferences({ root: root.trim(), recursive, minimumSizeMib, stopOnError });
    try {
      const result = await invoke<ScanResult>("scan_folder", {
        root: root.trim(),
        recursive,
        minimumSizeMib: Math.max(0, Math.min(4096, Number(minimumSizeMib) || 0)),
        ffmpegPath: ffmpegPath.trim() || null,
      });
      scanResult = result;
      root = result.root;
      settingsRoot = result.root;
      librarySettingsBaseline = JSON.stringify(result.settings);
      settingsBaseline = JSON.stringify(result.settings);
      void persistPreferences({ root });
      environment = result.environment;
      settings = preserveRunSettings ?? result.settings;
      settingsSources = result.settings_sources;
      page = "library";
      setNotice(`${result.candidate_count} media file${result.candidate_count === 1 ? "" : "s"} analyzed.`, "success");
      if (result.candidate_count > 0 && result.environment.ready) mode = "check";
    } catch (error) {
      setNotice(`Folder analysis failed: ${String(error)}`, "error");
    } finally {
      scanning = false;
      scanProgress = null;
    }
  }

  async function beginRun() {
    if (!canStart) {
      setNotice("Analyze a folder and resolve required tools before starting.", "error");
      return;
    }
    if (mode === "full") {
      fullAck = false;
      fullConfirmOpen = true;
      return;
    }
    await launchRun(false);
  }

  async function launchRun(fullRebuildConfirmed: boolean) {
    if (!settings) return;
    fullConfirmOpen = false;
    runLines = [];
    queuedRunLines = [];
    queuedRunFinished = null;
    runStarting = true;
    runMessage = "Starting the selected workflow…";
    try {
      const runId = await invoke<string>("start_run", {
        request: {
          root: root.trim(),
          recursive,
          minimumSizeMib: Math.max(0, Math.min(4096, Number(minimumSizeMib) || 0)),
          ffmpegPath: ffmpegPath.trim() || null,
          mode,
          settings,
          stopOnError,
          fullRebuildConfirmed,
        },
      });
      activeRunId = runId;
      runStarting = false;
      runLines = queuedRunLines.filter((event) => event.runId === runId).map((event) => event.line).slice(-240);
      runMessage = mode === "full" ? "Full rebuild running. Originals are retained until the rebuilt file validates." : mode === "test" ? "Short test encodes running. Originals will remain untouched." : "Check-only pass running. No media will be encoded.";
      page = "run";
      setNotice("Run started. Live progress and diagnostics are shown below.", "success");
      const earlyFinish = queuedRunFinished as RunFinishedEvent | null;
      if (earlyFinish?.runId === runId) await finishRun(earlyFinish);
    } catch (error) {
      activeRunId = "";
      runStarting = false;
      runMessage = "Could not start the selected workflow.";
      setNotice(String(error), "error");
    }
  }

  async function finishRun(payload: RunFinishedEvent) {
    activeRunId = "";
    if (payload.error) runMessage = payload.error;
    else if (payload.stopRequested) runMessage = "Queue stopped safely after the active file.";
    else if (payload.exitCode === 0) runMessage = "Workflow finished. Review the report and any skipped files.";
    else runMessage = `Workflow finished with exit code ${payload.exitCode ?? "unknown"}. Review the diagnostics.`;
    if (payload.logPath) void openLog(payload.logPath);
    await loadReports();
    setNotice(runMessage, payload.exitCode === 0 ? "success" : "error");
  }

  async function stopAfterCurrent() {
    if (!activeRunId) return;
    try {
      await invoke("request_stop", { runId: activeRunId });
      runMessage = "Safe stop requested. The current file will finish before the queue stops.";
      setNotice("Safe stop requested; the active file will finish first.", "success");
    } catch (error) {
      setNotice(`Could not request a safe stop: ${String(error)}`, "error");
    }
  }

  function repairApt() {
    if (activeRunId || runStarting || repairId || hdrInstalling || scanning) return;
    const packages = environment?.missing_apt_packages ?? [];
    if (!packages.length) return;
    if (!environment?.os.supported) {
      setNotice("Automatic APT repair is available only on supported Ubuntu and Linux Mint releases.", "error");
      return;
    }
    repairConfirmOpen = true;
  }

  async function confirmAptRepair() {
    if (activeRunId || runStarting || repairId || hdrInstalling || scanning) return;
    const packages = environment?.missing_apt_packages ?? [];
    if (!packages.length) return;
    repairConfirmOpen = false;
    repairLines = [];
    try {
      repairId = await invoke<string>("install_apt_tools", { packages });
      setNotice("System repair started. Follow the administrator authentication prompt and watch its output here.", "info");
    } catch (error) {
      repairId = "";
      setNotice(`Could not start package repair: ${String(error)}`, "error");
    }
  }

  async function installHdrHelper() {
    if (activeRunId || runStarting || repairId || hdrInstalling || scanning) return;
    hdrInstalling = true;
    setNotice("Installing the HDR10+ helper from its verified upstream release…", "info");
    try {
      const path = await invoke<string>("install_hdr10plus");
      setNotice(`HDR10+ helper installed at ${path}. Rechecking the environment.`, "success");
      await loadEnvironment();
    } catch (error) {
      setNotice(`HDR10+ helper setup failed: ${String(error)}`, "error");
    } finally {
      hdrInstalling = false;
    }
  }

  async function openLog(path: string | null | undefined) {
    if (!path) {
      selectedLog = "This report has no companion diagnostic log.";
      return;
    }
    logLoading = true;
    try {
      selectedLog = await invoke<string>("read_log", { path });
    } catch (error) {
      selectedLog = `Could not read diagnostic log: ${String(error)}`;
    } finally {
      logLoading = false;
    }
  }

  function chooseReport(path: string) {
    selectedReportPath = path;
    const report = reports.find((item) => item.csv_path === path);
    if (report) void openLog(report.log_path);
  }

  async function copyLog() {
    try {
      await navigator.clipboard.writeText(selectedLog);
      setNotice("Diagnostic excerpt copied to the clipboard.", "success");
    } catch (error) {
      setNotice(`Could not copy diagnostic text: ${String(error)}`, "error");
    }
  }

  onMount(() => {
    let disposed = false;
    document.documentElement.dataset.theme = theme;
    desktopShell = isTauri();
    if (!desktopShell) {
      environmentError = "Open HEVC Rebuild Studio from the desktop app to inspect this computer and connect to the media tools.";
      booting = false;
      return () => { disposed = true; };
    }
    const register = async () => {
      const listeners = await Promise.all([
        listen<ScanProgress>("scan-progress", (event) => { scanProgress = event.payload; }),
        listen<RunLineEvent>("run-line", (event) => {
          if (event.payload.runId === activeRunId) runLines = [...runLines, event.payload.line].slice(-240);
          else if (runStarting) queuedRunLines = [...queuedRunLines, event.payload].slice(-240);
        }),
        listen<RunFinishedEvent>("run-finished", async (event) => {
          if (event.payload.runId !== activeRunId) {
            if (runStarting) queuedRunFinished = event.payload;
            return;
          }
          await finishRun(event.payload);
        }),
        listen<RunLineEvent>("repair-line", (event) => {
          if (event.payload.runId === repairId) repairLines = [...repairLines, event.payload.line].slice(-120);
        }),
        listen<RepairFinished>("repair-finished", async (event) => {
          if (event.payload.jobId !== repairId) return;
          repairId = "";
          if (event.payload.success) setNotice("Package repair completed. Rechecking the installed tools.", "success");
          else setNotice(event.payload.error ?? "Package repair did not finish successfully.", "error");
          await loadEnvironment();
        }),
      ]);
      if (disposed) listeners.forEach((unlisten) => unlisten());
      else unlisteners = listeners;
    };
    void register().then(() => initialize()).catch((error) => {
      environmentError = String(error);
      booting = false;
      setNotice(`The desktop backend did not start: ${String(error)}`, "error");
    });
    return () => {
      disposed = true;
      unlisteners.forEach((unlisten) => unlisten());
    };
  });
</script>

<svelte:head>
  <title>HEVC Rebuild Studio</title>
  <meta name="description" content="A hardware-aware, safety-first desktop studio for HEVC media library rebuilds." />
</svelte:head>

<div class="app-shell" data-theme={theme}>
  <aside class="sidebar">
    <div class="brand-lockup">
      <div class="brand-mark"><Clapperboard size={20} strokeWidth={2.2} /></div>
      <div><div class="brand-name">HEVC <span>REBUILD</span></div><div class="brand-edition">STUDIO · UBUNTU</div></div>
    </div>

    <div class="workspace-label">WORKSPACE</div>
    <nav aria-label="Main navigation">
      {#each navigation as item}
        {@const Icon = item.icon}
        <button class="nav-item" class:active={page === item.id} onclick={() => (page = item.id)} aria-current={page === item.id ? "page" : undefined}>
          <Icon size={18} strokeWidth={1.8} /><span>{item.label}</span>
          {#if item.id === "reports" && failedRows.length}<span class="nav-count">{failedRows.length}</span>{/if}
          {#if page === item.id}<ChevronRight class="nav-arrow" size={15} />{/if}
        </button>
      {/each}
    </nav>

    <div class="sidebar-grow"></div>
    <div class="sidebar-library">
      <div class="sidebar-library-icon"><FolderOpen size={17} /></div>
      <div class="sidebar-library-copy"><span>ACTIVE LIBRARY</span><strong title={root}>{root ? fileName(root) : "Choose a folder"}</strong></div>
      <button class="icon-button subdued" aria-label="Choose media folder" title="Choose media folder" onclick={browseFolder}><ChevronRight size={16} /></button>
    </div>
    <div class="sidebar-bottom">
      <span class="connection-dot" class:green={environment?.ready}></span>
      <span>{environment?.ready ? "Engine ready" : environmentLoading || booting ? "Checking engine…" : "Setup required"}</span>
      <span class="version-tag">v1.3.0</span>
    </div>
  </aside>

  <main class="main-panel">
    <header class="topbar">
      <div class="breadcrumbs"><span>HEVC REBUILD</span><ChevronRight size={14} /><strong>{navigation.find((item) => item.id === page)?.label}</strong></div>
      <div class="top-actions">
        <label class="theme-control" title="Choose the application appearance">
          {#if theme === "light"}<Sun size={15} aria-hidden="true" />{:else if theme === "dark"}<Moon size={15} aria-hidden="true" />{:else}<Terminal size={15} aria-hidden="true" />{/if}
          <span class="sr-only">Appearance</span>
          <select aria-label="Appearance" value={theme} onchange={(event) => applyTheme(event.currentTarget.value)}>
            <option value="light">Light</option>
            <option value="dark">Dark</option>
            <option value="hacker">Hacker</option>
          </select>
        </label>
        {#if environment?.ready}
          <span class="status-pill good"><span class="status-dot"></span> SYSTEM READY</span>
        {:else if environmentLoading || booting}
          <span class="status-pill neutral"><LoaderCircle size={14} class="spin" /> CHECKING</span>
        {:else}
          <button class="status-pill warning" onclick={() => (page = "overview")}><AlertTriangle size={14} /> SETUP NEEDED</button>
        {/if}
        <button class="icon-button" aria-label="Refresh environment" title="Refresh environment" disabled={environmentLoading} onclick={loadEnvironment}><RefreshCw size={17} class={environmentLoading ? "spin" : ""} /></button>
      </div>
    </header>

    {#if notice}
      <div class="notice" class:error={noticeKind === "error"} class:success={noticeKind === "success"} role="status">
        {#if noticeKind === "error"}<AlertTriangle size={16} />{:else if noticeKind === "success"}<CheckCircle2 size={16} />{:else}<Info size={16} />{/if}
        <span>{notice}</span><button class="notice-close" aria-label="Dismiss notification" onclick={() => (notice = "")}><X size={15} /></button>
      </div>
    {/if}

    <div class="page-wrap">
      {#if page === "overview"}
        <section class="page-heading">
          <div><div class="eyebrow">SAFE LIBRARY REBUILD</div><h1>Your media, <em>refined.</em></h1><p>Analyze first. Test next. Rebuild only when everything checks out.</p></div>
          <div class="heading-actions"><button class="button secondary" onclick={browseFolder} disabled={!desktopShell || workflowBusy}><FolderOpen size={17} /> Choose folder</button><button class="button primary" disabled={!desktopShell || workflowBusy || booting} onclick={analyzeFolder}>{#if scanning}<LoaderCircle class="spin" size={17} /> Analyzing{:else}<ScanSearch size={17} /> Analyze library{/if}</button></div>
        </section>

        <section class="hero-card">
          <div class="hero-copy">
            <div class="hero-kicker"><span class="hero-orbit"><Zap size={13} fill="currentColor" /></span> CAREFUL BY DESIGN</div>
            <h2>Smaller files.<br /><span>Same story.</span></h2>
            <p>Rebuild the video stream with CPU x265 while audio, subtitles, chapters and the original stay protected.</p>
            <button class="hero-link" onclick={() => (page = scanResult ? "library" : "run")}>Explore the workflow <ArrowRight size={16} /></button>
          </div>
          <div class="hero-visual" aria-hidden="true">
            <div class="visual-glow"></div><div class="visual-frame frame-back"><div class="frame-scene scene-back"></div></div>
            <div class="visual-frame frame-front"><div class="frame-scene"><div class="scene-sun"></div><div class="scene-mountain mountain-a"></div><div class="scene-mountain mountain-b"></div><div class="scene-ground"></div></div><span class="visual-frame-label">SOURCE · HEVC</span></div>
            <div class="visual-arrow"><ArrowRight size={18} /></div><div class="visual-chip"><Check size={14} /> VALIDATED</div>
          </div>
          <div class="hero-grain"></div>
        </section>

        <section class="metric-grid" aria-label="Library summary">
          <article class="metric-card"><div class="metric-icon lilac"><Video size={18} /></div><span class="metric-label">MEDIA FILES</span><strong>{scanResult?.candidate_count ?? "—"}</strong><small>{scanResult ? `${bytes(scanResult.total_input_bytes)} analyzed` : "Analyze a folder to begin"}</small></article>
          <article class="metric-card"><div class="metric-icon green-bg"><HardDrive size={18} /></div><span class="metric-label">AVAILABLE DISK</span><strong>{scanResult ? bytes(scanResult.disk_free_bytes) : "—"}</strong><small>Reserve target {settings ? `${settings.resources.min_free_disk_gib} GiB` : "8 GiB"}</small></article>
          <article class="metric-card"><div class="metric-icon amber"><ShieldCheck size={18} /></div><span class="metric-label">SOURCE PROTECTION</span><strong>Transactional</strong><small>Originals move only after validation</small></article>
          <article class="metric-card"><div class="metric-icon blue"><Cpu size={18} /></div><span class="metric-label">ENCODER</span><strong>CPU x265</strong><small>{environment?.hardware.logical_cpus ?? "—"} logical threads detected</small></article>
        </section>

        {#if !environment?.ready}
          <section class="setup-card">
            <div class="setup-symbol"><Wrench size={21} /></div>
            <div class="setup-content"><div class="eyebrow">ENVIRONMENT CHECK</div><h3>One quick setup, then you’re ready.</h3><p>{environment?.os.note ?? environmentError ?? "Checking this computer for the media tools this workflow needs."}</p>
              {#if environment?.missing_apt_packages.length}
                <div class="missing-packages"><span>MISSING SYSTEM PACKAGES</span>{#each environment.missing_apt_packages as pkg}<code>{pkg}</code>{/each}</div>
              {/if}
            </div>
            <div class="setup-actions">
              {#if environment?.missing_apt_packages.length && environment.os.supported}
                {#if environment.checks.some((check) => check.key === "pkexec" && check.available)}<button class="button primary" disabled={!desktopShell || workflowBusy} onclick={repairApt}>{#if repairId}<LoaderCircle class="spin" size={16} /> Repair running{:else}<Wrench size={16} /> Install missing tools{/if}</button>
                {:else}<span class="setup-note">Desktop administrator authentication (pkexec) is missing. Install the listed tools through Ubuntu Software or a terminal.</span>{/if}
              {/if}
              {#if ffmpegPath}<span class="selected-binary" title={ffmpegPath}>Using {ffmpegPath}</span><button class="text-button" disabled={!desktopShell || workflowBusy || environmentLoading} onclick={clearFfmpegOverride}>Use FFmpeg from PATH</button>{/if}
              {#if environment?.checks.some((check) => check.key === "hdr10plus_tool" && !check.available)}<button class="text-button" disabled={!desktopShell || workflowBusy} onclick={installHdrHelper}>{hdrInstalling ? "Installing helper…" : "Install HDR10+ helper"}</button>{/if}
              <button class="text-button" disabled={!desktopShell || workflowBusy} onclick={chooseFfmpeg}>Point to an FFmpeg binary</button>
            </div>
            {#if repairLines.length}<pre class="repair-output">{repairLines.slice(-8).join("\n")}</pre>{/if}
          </section>
        {/if}

        <section class="section-block overview-lower">
          <div class="section-title-row"><div><div class="eyebrow">YOUR WORKSPACE</div><h2>Recent analysis</h2></div><button class="text-button" onclick={() => (page = "library")}>Open media library <ArrowRight size={15} /></button></div>
          {#if scanResult?.entries.length}
            {@render MediaTable(scanResult.entries.slice(0, 5), () => (page = "library"))}
          {:else}
            <div class="empty-state compact"><div class="empty-icon"><ScanSearch size={22} /></div><div><strong>No folder analysis yet</strong><p>Select your Movies or TV folder to see codecs, HDR formats, streams, and files that need attention.</p></div><button class="button secondary" disabled={!desktopShell || workflowBusy} onclick={browseFolder}>Choose a folder</button></div>
          {/if}
        </section>

        {@render EnvironmentPanel(environment)}

      {:else if page === "library"}
        <section class="page-heading">
          <div><div class="eyebrow">FOLDER ANALYSIS</div><h1>Media <em>library.</em></h1><p>Build a read-only inventory and review every file before you run anything.</p></div>
          <div class="heading-actions"><button class="button secondary" disabled={!desktopShell || workflowBusy} onclick={browseFolder}><FolderOpen size={17} /> Browse</button><button class="button primary" disabled={!desktopShell || workflowBusy || !root} onclick={analyzeFolder}>{#if scanning}<LoaderCircle class="spin" size={17} /> Analyzing{:else}<ScanSearch size={17} /> Analyze folder{/if}</button></div>
        </section>

        <section class="library-toolbar card-surface">
          <div class="folder-field"><FolderOpen size={18} /><input aria-label="Media library folder" bind:value={root} placeholder="Choose a media folder" disabled={workflowBusy} oninput={() => (scanResult = null)} onblur={() => void persistPreferences({ root })} /><button class="icon-button" aria-label="Browse folders" disabled={workflowBusy} onclick={browseFolder}><ChevronRight size={17} /></button></div>
          <div class="toolbar-options"><label class="check-label"><input type="checkbox" bind:checked={recursive} disabled={workflowBusy} onchange={() => { scanResult = null; void persistPreferences({ recursive }); }} /> Include subfolders</label><label class="small-field">Ignore smaller than <input type="number" min="0" max="4096" step="1" bind:value={minimumSizeMib} disabled={workflowBusy} oninput={() => (scanResult = null)} onblur={() => void persistPreferences({ minimumSizeMib })} /> MiB</label></div>
        </section>

        {#if scanning}
          <div class="scan-progress card-surface"><LoaderCircle class="spin" size={19} /><div><strong>{scanProgress ? `Reading ${scanProgress.current} of ${scanProgress.total}` : "Preparing the analysis…"}</strong><span>{scanProgress?.file ?? "Finding media files and checking stream metadata"}</span></div><div class="progress-track"><div style={`width:${scanProgress?.total ? Math.max(4, scanProgress.current / scanProgress.total * 100) : 9}%`}></div></div></div>
        {/if}

        {#if scanResult}
          <section class="library-summary-grid">
            <div class="summary-tile"><span>FOUND</span><strong>{scanResult.candidate_count}</strong><small>{recursive ? "Recursive scan" : "Top-level folder"}</small></div>
            <div class="summary-tile"><span>INPUT SIZE</span><strong>{bytes(scanResult.total_input_bytes)}</strong><small>Candidate media</small></div>
            <div class="summary-tile"><span>FREE ON VOLUME</span><strong>{bytes(scanResult.disk_free_bytes)}</strong><small>Output safety checks apply per file</small></div>
            <div class="summary-tile attention"><span>NEEDS REVIEW</span><strong>{analysisErrors.length + scanResult.entries.filter((entry) => entry.status === "Backed up").length}</strong><small>Analysis errors or existing backups</small></div>
          </section>
          {#if analysisErrors.length}
            <div class="inline-warning"><AlertTriangle size={17} /><div><strong>{analysisErrors.length} file{analysisErrors.length === 1 ? "" : "s"} could not be analyzed</strong><span>Open each row to inspect the error. These files will not be silently treated as ready.</span></div></div>
          {/if}
          {#if highMemoryEntries.length || constrainedMemoryEntries.length}
            <div class="inline-warning"><Gauge size={17} /><div><strong>{highMemoryEntries.length ? `${highMemoryEntries.length} file${highMemoryEntries.length === 1 ? "" : "s"} show high memory risk` : `${constrainedMemoryEntries.length} file${constrainedMemoryEntries.length === 1 ? "" : "s"} need reduced parallelism`}</strong><span>Predictions use current available RAM, effective memory limits, resolution, bit depth, and chroma. Open the row for the planned x265 worker/frame counts. The estimate is a scheduling heuristic, not a peak-RAM guarantee.</span></div></div>
          {/if}
          {#if forecastOutdated}<div class="inline-warning"><Info size={17} /><div><strong>The displayed thread plan predates your settings changes.</strong><span>The engine recalculates memory-aware worker and frame counts when each file starts. The scan forecast is informational only.</span></div></div>{/if}
          {#if !environment?.ready}
            <div class="inline-warning"><Wrench size={17} /><div><strong>Analysis is available, but runs are blocked until required tools are ready.</strong><span>Use Overview → Environment check to install the missing tools or select a compatible FFmpeg binary.</span></div><button class="text-button" onclick={() => (page = "overview")}>Open setup</button></div>
          {/if}
          <section class="table-card">
            <div class="table-header"><div><div class="eyebrow">ANALYSIS RESULTS</div><h2>{visibleEntries.length} files</h2></div><label class="search-box"><Search size={16} /><input aria-label="Search files" bind:value={search} placeholder="Search files or status" /></label></div>
            {@render MediaTable(visibleEntries, () => {})}
          </section>
          <div class="scan-next-step"><div class="next-icon"><CheckCircle2 size={19} /></div><div><strong>Review first. Choose the run when ready.</strong><span>Check only is a planning pass; short tests leave originals untouched; full rebuild validates before moving originals into <code>old/</code>.</span></div><button class="button primary" disabled={!canStart} onclick={() => (page = "run")}>Choose a run <ArrowRight size={16} /></button></div>
        {:else if !scanning}
          <div class="empty-state large"><div class="empty-icon"><Layers size={26} /></div><div><div class="eyebrow">START WITH A READ-ONLY SCAN</div><h2>Know what’s in the folder before changing anything.</h2><p>We’ll inventory supported media, inspect stream and HDR profiles, check disk space and identify files that already have backups.</p></div><button class="button primary" disabled={!desktopShell || !root || workflowBusy} onclick={analyzeFolder}><ScanSearch size={17} /> Analyze this folder</button></div>
        {/if}

      {:else if page === "run"}
        <section class="page-heading">
          <div><div class="eyebrow">CONTROLLED PROCESSING</div><h1>Run & <em>settings.</em></h1><p>Use the application’s effective defaults, then select a run mode for this library.</p></div>
          {#if scanResult}<div class="library-chip"><FolderOpen size={15} /><span title={root}>{fileName(root)}</span><span class="chip-divider"></span>{scanResult.candidate_count} files</div>{/if}
        </section>

        {#if !scanResult}<div class="inline-warning"><Info size={17} /><div><strong>Analyze a folder before selecting a run.</strong><span>The scan becomes the run plan and helps surface file-specific warnings first.</span></div><button class="button secondary" onclick={() => (page = "library")}>Open media library</button></div>{/if}

        <section class="mode-section"><div class="section-title-row"><div><div class="eyebrow">THREE SAFE STAGES</div><h2>Choose your run</h2></div><span class="mode-guidance"><ShieldCheck size={15} /> Start with Check only</span></div>
          <div class="mode-grid">
            <button class="mode-card" class:selected={mode === "check"} disabled={workflowBusy} onclick={() => (mode = "check")}><span class="mode-number">01</span><span class="mode-icon mint"><ScanSearch size={19} /></span><span class="mode-title">Check only</span><span class="mode-description">Review planned actions and preflight each file. No encoding.</span><span class="mode-tag">READ-ONLY</span></button>
            <button class="mode-card" class:selected={mode === "test"} disabled={workflowBusy} onclick={() => (mode = "test")}><span class="mode-number">02</span><span class="mode-icon lavender"><Play size={18} /></span><span class="mode-title">Short test</span><span class="mode-description">Create short test samples. Originals stay exactly where they are.</span><span class="mode-tag">5 SECONDS · TEST CRF 30</span></button>
            <button class="mode-card" class:selected={mode === "full"} disabled={workflowBusy} onclick={() => (mode = "full")}><span class="mode-number">03</span><span class="mode-icon amber"><Zap size={18} /></span><span class="mode-title">Full rebuild</span><span class="mode-description">Process the real files. Original moves only after output validates.</span><span class="mode-tag warning-tag">CONFIRM TO START</span></button>
          </div>
        </section>

        {#if settings}
          <section class="settings-intro"><div class="section-title-row"><div><div class="eyebrow">EFFECTIVE CONFIGURATION</div><h2>Rebuild parameters</h2></div><div class="settings-intro-actions"><span class="edit-badge"><Settings2 size={14} /> {settingsChanged ? "Unsaved changes" : appPreferences.settings ? "Saved app profile" : "Library values"}</span><button class="text-button" disabled={(!settingsChanged && !appPreferences.settings) || workflowBusy} onclick={resetSettings}>Reset to library values</button><button class="text-button" disabled={!settingsChanged || workflowBusy} onclick={saveSettings}>Save as app defaults</button></div></div><p>Application, user, and library values load first. Save your edits as app defaults to keep them across launches; unsaved edits apply to the next run only.</p>{#if forecastOutdated}<div class="inline-warning"><Info size={17} /><div><strong>Resource settings changed after the folder scan.</strong><span>The per-file forecast on the library page is from that scan. Actual worker/frame counts are recomputed immediately before encoding each file.</span></div></div>{/if}</section>
          <fieldset class="settings-grid" aria-label="Rebuild settings" disabled={workflowBusy}>
            <article class="settings-card"><div class="settings-card-heading"><div class="settings-icon purple"><Film size={18} /></div><div><h3>Picture encoding</h3><p>CPU libx265 · video stream only</p></div></div>
              <label class="form-row range-row"><span><strong>Quality</strong><small>CRF {settings.video.crf} · lower means larger, higher-quality files</small></span><input aria-label="CRF quality" type="range" min="0" max="51" step="1" bind:value={settings.video.crf} /><b>{settings.video.crf}</b></label>
              <label class="form-row"><span><strong>Preset</strong><small>Compression effort and time</small></span><select bind:value={settings.video.preset}><option value="ultrafast">ultrafast</option><option value="superfast">superfast</option><option value="veryfast">veryfast</option><option value="faster">faster</option><option value="fast">fast</option><option value="medium">medium</option><option value="slow">slow · recommended</option><option value="slower">slower</option><option value="veryslow">veryslow</option></select></label>
              <label class="toggle-row"><span><strong>Promote 8-bit sources to 10-bit</strong><small>Off by default; does not add missing color detail</small></span><input type="checkbox" bind:checked={settings.video.promote_8bit_to_10bit} /></label>
              <label class="form-row"><span><strong>Content tune</strong><small>Auto uses reliable animation metadata</small></span><select bind:value={settings.content.tune}><option value="auto">Automatic</option><option value="animation">Animation</option><option value="grain">Film grain</option><option value="none">Normal</option></select></label>
              <label class="toggle-row"><span><strong>Detect animation</strong><small>Only when content tune is automatic</small></span><input type="checkbox" bind:checked={settings.content.detect_animation} /></label>
            </article>

            <article class="settings-card"><div class="settings-card-heading"><div class="settings-icon blue"><ShieldCheck size={18} /></div><div><h3>HDR & stream safety</h3><p>Preserve by default. Never transcode audio or subtitles.</p></div></div>
              <label class="toggle-row"><span><strong>Preserve HDR10+ dynamic metadata</strong><small>Verify dynamic metadata after encoding</small></span><input type="checkbox" bind:checked={settings.hdr.preserve_hdr10plus} /></label>
              <label class="form-row"><span><strong>Dolby Vision policy</strong><small>Safe skips unsupported profiles instead of damaging them</small></span><select bind:value={settings.hdr.dolby_vision}><option value="safe">Safe preservation</option><option value="skip">Skip all Dolby Vision</option><option value="hdr10-fallback">Verified HDR10 fallback</option></select></label>
              <label class="form-row"><span><strong>Secondary video tracks</strong><small>Highest-resolution real video is the primary encode</small></span><select bind:value={settings.streams.secondary_video}><option value="copy">Copy</option><option value="drop">Drop</option></select></label>
              <label class="form-row"><span><strong>Data streams</strong><small>Preserve Matroska-compatible data</small></span><select bind:value={settings.streams.data_streams}><option value="preserve">Preserve</option><option value="drop">Drop</option></select></label>
              <label class="toggle-row"><span><strong>SHA-256 verify copied streams</strong><small>Audio and subtitle packets are copied, never encoded</small></span><input type="checkbox" bind:checked={settings.safety.verify_copy_hashes} /></label>
              <label class="text-setting"><span><strong>Matroska COMMENT</strong><small>Written into rebuilt output metadata</small></span><input maxlength="500" bind:value={settings.metadata.comment} /></label>
            </article>

            <article class="settings-card wide-settings"><div class="settings-card-heading"><div class="settings-icon green-bg"><Gauge size={18} /></div><div><h3>Resource guardrails</h3><p>One file at a time · automatic memory and disk protection</p></div></div>
              <div class="resource-grid">
                <label><span>CPU policy</span><select bind:value={settings.resources.threading}><option value="auto">Automatic</option><option value="unlimited">Use all CPU threads · memory-limited</option></select></label>
                <label><span>Leave CPU threads free</span><input type="number" min="0" max="64" bind:value={settings.resources.reserved_cpu_threads} /></label>
                <label><span>Worker thread cap · 0 = auto</span><input type="number" min="0" max="64" bind:value={settings.resources.max_worker_threads} /></label>
                <label><span>Frame thread cap · 0 = auto</span><input type="number" min="0" max="16" bind:value={settings.resources.frame_threads} /></label>
                <label><span>Minimum free memory · GiB</span><input type="number" min="0" step="0.25" bind:value={settings.resources.min_free_memory_gib} /></label>
                <label><span>Memory reserve · percent</span><input type="number" min="0" max="50" step="0.5" bind:value={settings.resources.memory_reserve_percent} /></label>
                <label><span>Minimum free disk · GiB</span><input type="number" min="0" step="0.5" bind:value={settings.resources.min_free_disk_gib} /></label>
                <label><span>Temporary output estimate · ratio</span><input type="number" min="0.25" max="4" step="0.05" bind:value={settings.resources.preflight_output_ratio} /></label>
                <label><span>Live size projection margin · percent</span><input type="number" min="0" max="100" step="1" bind:value={settings.resources.disk_projection_margin_percent} /></label>
                <label><span>Hardware calibration</span><select bind:value={settings.calibration.mode}><option value="auto">Automatic · 4K/large files</option><option value="ask">Ask in interactive CLI when uncached</option><option value="always">Every file · benchmark each time</option><option value="off">Disabled</option></select></label>
                <label><span>Calibration cache · hours</span><input type="number" min="0" max="720" step="1" bind:value={settings.calibration.cache_hours} /></label>
                <label><span>Large-file threshold · GiB</span><input type="number" min="0" max="1000" step="1" bind:value={settings.calibration.large_file_gib} /></label>
                <label><span>Sample duration · seconds</span><input type="number" min="1" max="15" step="1" bind:value={settings.calibration.sample_seconds} /></label>
                <label><span>Maximum layouts to compare</span><input type="number" min="1" max="4" step="1" bind:value={settings.calibration.max_candidates} /></label>
                <label><span>Throughput tie margin · percent</span><input type="number" min="0" max="20" step="0.5" bind:value={settings.calibration.throughput_tolerance_percent} /></label>
                <div class="hardware-note"><Cpu size={17} /><span>Calibration reads a short representative section and writes no media output. Automatic mode runs for 4K or files above the size threshold, then caches matching CPU, memory, FFmpeg/x265, profile, and encode-setting results. “Every file” bypasses the cache and can add repeated overhead. Calibration changes only x265 worker/frame counts; live RAM, swap-pressure, and safe-retry guards remain active. The first calibration can add time before a large file starts.</span></div>
                <div class="hardware-note"><Cpu size={17} /><span>Detected: <strong>{environment?.hardware.cpu_model ?? "CPU details unavailable"}</strong>. CPU libx265 parallelism adapts to current memory and each file's profile. This is a scheduling heuristic, not an exact peak-RAM forecast. Live monitoring tracks RAM, swap use, swap-in/out, and memory stalls; sustained pressure can trigger a lower-thread retry. Swap is not counted as fast RAM or resized automatically.</span></div>
              </div>
            </article>
          </fieldset>
          <div class="config-source"><Info size={14} /><span>Configuration source: {settingsSources.length ? settingsSources.join(" · ") : settingsPath || "application defaults"}{preferenceStorage ? ` · App settings: ${preferenceStorage.settingsFile}` : ""}</span><button class="text-button" disabled={workflowBusy} onclick={() => loadSettings(root)}>Reload effective values</button></div>
        {/if}

        <section class="run-launch card-surface"><div class="launch-left"><div class="launch-icon"><TerminalSquare size={19} /></div><div><strong>{activeRunId ? "A workflow is running" : mode === "full" ? "Full rebuild requires confirmation" : mode === "test" ? "Short test run" : "Ready for a check-only pass"}</strong><span>{scanResult ? `${scanResult.candidate_count} files · ${bytes(scanResult.total_input_bytes)} total` : "Analyze a folder before starting"}{environment?.hardware.gpu_devices.length ? ` · ${environment.hardware.gpu_devices.length} GPU detected, CPU encoder selected` : ""}</span></div></div>
          <label class="check-label stop-error"><input type="checkbox" bind:checked={stopOnError} onchange={() => void persistPreferences({ stopOnError })} /> Stop queue on first error</label>
          {#if activeRunId}<button class="button danger-outline" onclick={stopAfterCurrent}><Square size={15} /> Stop after current file</button>{:else}<button class="button primary" disabled={!canStart} onclick={beginRun}>{#if mode === "full"}<ShieldCheck size={17} />{:else if mode === "test"}<Play size={17} />{:else}<ScanSearch size={17} />{/if}{mode === "full" ? "Review & start full rebuild" : mode === "test" ? "Start short tests" : "Start check-only pass"}</button>{/if}
        </section>

        {#if runMessage || runLines.length}
          <section class="run-console-card"><div class="run-console-heading"><div><span class="console-indicator" class:running={Boolean(activeRunId)}></span><strong>Run activity</strong><span>{activeRunId ? "LIVE" : "LATEST"}</span></div>{#if activeRunId}<button class="text-button" onclick={stopAfterCurrent}>Stop safely</button>{/if}</div>{#if runMessage}<p class="run-message">{runMessage}</p>{/if}<pre class="run-console">{runLines.length ? runLines.join("\n") : "Waiting for engine output…"}</pre></section>
        {/if}

      {:else}
        <section class="page-heading">
          <div><div class="eyebrow">AUDIT TRAIL</div><h1>Reports & <em>errors.</em></h1><p>Review completed runs, per-file outcomes, and diagnostic logs from the media engine.</p></div>
          <button class="button secondary" disabled={Boolean(repairId) || hdrInstalling} onclick={loadReports}><RefreshCw size={16} /> Refresh history</button>
        </section>
        {#if failedRows.length}
          <div class="error-summary"><div class="error-summary-icon"><AlertTriangle size={20} /></div><div><strong>{failedRows.length} skipped or failed result{failedRows.length === 1 ? "" : "s"} in recent history</strong><span>Open a report and its companion log to see the exact phase, cause, and resource snapshot.</span></div><span class="error-summary-count">{failedRows.length}</span></div>
        {/if}
        {#if reports.length}
          <section class="reports-layout">
            <aside class="report-list card-surface"><div class="report-list-heading"><div class="eyebrow">RECENT RUNS</div><span>{reports.length} reports</span></div>
              {#each reports as report}
                {@const reportName = fileName(report.csv_path).replace(/\.csv$/i, "")}
                <button class="report-item" class:active={activeReport?.csv_path === report.csv_path} onclick={() => chooseReport(report.csv_path)}><span class="report-item-icon" class:failed={Object.entries(report.counts).some(([status, count]) => count > 0 && isFailure(status))}><FileText size={17} /></span><span class="report-item-copy"><strong>{reportName}</strong><small>{new Date(report.created * 1000).toLocaleString()}</small></span><ChevronRight size={15} /></button>
              {/each}
            </aside>
            {#if activeReport}
              <section class="report-detail">
                <div class="report-heading card-surface"><div><div class="eyebrow">RUN REPORT</div><h2>{fileName(activeReport.csv_path).replace(/\.csv$/i, "")}</h2><p>{new Date(activeReport.created * 1000).toLocaleString()}</p></div><button class="button secondary" onclick={() => openLog(activeReport.log_path)}><TerminalSquare size={16} /> {logLoading ? "Loading…" : "Open diagnostics"}</button></div>
                <div class="report-counts">{#each Object.entries(activeReport.counts) as [status, count]}<div class="report-count"><span class:bad-count={isFailure(status)}>{status}</span><strong>{count}</strong></div>{/each}</div>
                <div class="table-card report-table"><div class="table-header"><div><div class="eyebrow">FILE OUTCOMES</div><h2>{activeReport.rows.length} entries</h2></div></div>
                  <div class="table-scroll"><table><thead><tr><th>INPUT</th><th>RESULT</th><th>DETAIL</th><th>SIZE</th></tr></thead><tbody>
                    {#each activeReport.rows as row}<tr><td><strong class="file-cell-title">{fileName(row.input)}</strong><small class="file-cell-sub">{row.resolution ?? row.primary_video ?? "Media file"}</small></td><td><span class="status-tag" class:status-error={isFailure(row.status)} class:status-ready={!isFailure(row.status)}>{row.status}</span></td><td class="detail-cell" title={row.memory_forecast_message ?? ""}>{row.message || row.dolby_vision_action || "—"}{#if row.calibration_source}<small class="file-cell-sub">Calibration: {row.calibration_source}{row.calibration_workers ? ` · ${row.calibration_workers} workers / ${row.calibration_frame_threads ?? "—"} frame threads` : ""}{Number(row.calibration_candidate_count) > 0 ? ` · ${row.calibration_candidate_count} candidate(s)` : ""}{row.calibration_note ? ` · ${row.calibration_note}` : ""}</small>{/if}{#if row.memory_forecast}<small class="file-cell-sub">Memory: {row.memory_forecast.toUpperCase()} · {row.planned_workers ?? "—"} workers · {row.planned_frame_threads ?? "—"} frame threads</small>{/if}{#if row.min_memory_available_gib}<small class="file-cell-sub">Min available RAM: {Number(row.min_memory_available_gib).toFixed(1)} GiB</small>{/if}{#if row.max_encoder_rss_gib}<small class="file-cell-sub">Peak encoder RSS: {Number(row.max_encoder_rss_gib).toFixed(2)} GiB</small>{/if}{#if row.max_swap_used_percent || row.max_encoder_swap_gib || row.max_swap_in_mib_s || row.max_swap_out_mib_s}<small class="file-cell-sub">Peak swap: {row.max_swap_used_percent ? `${Number(row.max_swap_used_percent).toFixed(0)}% system used` : "usage unknown"}{row.max_encoder_swap_gib ? ` · x265 ${Number(row.max_encoder_swap_gib).toFixed(2)} GiB` : ""} · in {row.max_swap_in_mib_s ? `${Number(row.max_swap_in_mib_s).toFixed(1)} MiB/s` : "—"} · out {row.max_swap_out_mib_s ? `${Number(row.max_swap_out_mib_s).toFixed(1)} MiB/s` : "—"}{row.max_memory_full_psi_percent ? ` · memory stalls ${Number(row.max_memory_full_psi_percent).toFixed(1)}%` : ""}</small>{/if}</td><td>{row.input_bytes ? bytes(Number(row.input_bytes)) : "—"}</td></tr>{/each}
                  </tbody></table></div>
                  {#if activeReport.rows.length === 0}<div class="empty-state compact"><div><strong>This report has no readable rows.</strong><p>The CSV may be incomplete; inspect the companion log for context.</p></div></div>{/if}
                </div>
                <section class="diagnostic-card"><div class="diagnostic-heading"><div><div class="eyebrow">DIAGNOSTIC LOG</div><h3>Engine output</h3></div><button class="text-button" disabled={!selectedLog} onclick={copyLog}>Copy excerpt</button></div><pre>{selectedLog || "Select Open diagnostics to load the companion log. The viewer is limited to the latest 1 MB."}</pre></section>
              </section>
            {/if}
          </section>
        {:else}
          <div class="empty-state large"><div class="empty-icon"><History size={26} /></div><div><div class="eyebrow">NO RUN HISTORY YET</div><h2>Reports appear here after your first analysis or rebuild.</h2><p>File-level statuses and diagnostic logs are stored locally in your user state directory.</p></div><button class="button primary" onclick={() => (page = scanResult ? "run" : "library")}>Go to workflow <ArrowRight size={16} /></button></div>
        {/if}
      {/if}

      <footer class="page-footer"><span><ShieldCheck size={14} /> Local processing · Originals protected</span><span>HEVC Rebuild Studio <i>•</i> Ubuntu-first preview</span></footer>
    </div>
  </main>
</div>

{#if fullConfirmOpen}
  <div class="modal-backdrop" role="presentation">
    <div class="confirm-modal" role="dialog" aria-modal="true" aria-labelledby="confirm-title" tabindex="-1">
      <div class="confirm-symbol"><AlertTriangle size={22} /></div><button class="icon-button modal-close" aria-label="Close confirmation" onclick={() => (fullConfirmOpen = false)}><X size={18} /></button>
      <div class="eyebrow">FINAL SAFETY CHECK</div><h2 id="confirm-title">Start the full rebuild?</h2><p>This will encode every eligible file in <strong>{root}</strong>. Each original remains untouched until its rebuilt output passes validation, then moves into that folder’s <code>old/</code> directory.</p>
      <div class="confirm-facts"><span><Film size={16} /> {scanResult?.candidate_count ?? 0} candidate files</span><span><HardDrive size={16} /> {bytes(scanResult?.disk_free_bytes)} currently free</span><span><Cpu size={16} /> CPU libx265</span></div>
      <label class="confirm-check"><input type="checkbox" bind:checked={fullAck} /> I reviewed the analysis and understand this starts real encodes.</label>
      <div class="modal-actions"><button class="button secondary" onclick={() => (fullConfirmOpen = false)}>Go back</button><button class="button primary" disabled={!fullAck || !canStart} onclick={() => launchRun(true)}><ShieldCheck size={16} /> Start full rebuild</button></div>
    </div>
  </div>
{/if}

{#if repairConfirmOpen && environment}
  <div class="modal-backdrop" role="presentation">
    <div class="confirm-modal" role="dialog" aria-modal="true" aria-labelledby="repair-title" tabindex="-1">
      <div class="confirm-symbol repair-symbol"><Wrench size={21} /></div><button class="icon-button modal-close" aria-label="Close confirmation" onclick={() => (repairConfirmOpen = false)}><X size={18} /></button>
      <div class="eyebrow">SYSTEM REPAIR</div><h2 id="repair-title">Install missing Ubuntu tools?</h2><p>HEVC Rebuild Studio will refresh the APT package index and install only the missing packages listed below. Ubuntu will request administrator authentication. No package upgrades are requested.</p>
      <div class="repair-package-list">{#each environment.missing_apt_packages as pkg}<code>{pkg}</code>{/each}</div>
      <div class="modal-actions"><button class="button secondary" onclick={() => (repairConfirmOpen = false)}>Cancel</button><button class="button primary" disabled={Boolean(activeRunId) || hdrInstalling} onclick={confirmAptRepair}><Wrench size={16} /> Continue to authentication</button></div>
    </div>
  </div>
{/if}

{#snippet MediaTable(entries: MediaEntry[], onOpenLibrary: () => void)}
  {#if entries.length}
    <div class="table-scroll"><table><thead><tr><th>MEDIA FILE</th><th>VIDEO PROFILE</th><th>STREAMS</th><th>MEMORY PLAN</th><th>STATUS</th></tr></thead><tbody>
      {#each entries as entry}
        <tr><td><div class="file-cell"><div class="file-icon"><Video size={17} /></div><div class="file-cell-copy"><strong class="file-cell-title" title={entry.relative_path}>{entry.name}</strong><small class="file-cell-sub" title={entry.relative_path}>{entry.relative_path}</small>{#if entry.error}<small class="row-error">{entry.error}</small>{/if}</div></div></td>
          <td><span class="profile-primary">{profileSummary(entry)}</span><small class="file-cell-sub">{entry.profile?.video.codec ?? "—"}{entry.profile?.interlaced ? " · Interlaced" : ""}</small></td>
          <td><span class="stream-count"><Layers size={14} /> {entry.profile?.audio_count ?? "—"} audio <i>·</i> {entry.profile?.subtitle_count ?? "—"} subs</span>{#if entry.profile?.hdr.dolby_vision}<small class="file-cell-sub">Profile {entry.profile.hdr.dolby_vision_profile ?? "?"} · Dolby Vision</small>{/if}</td>
          <td>{#if entry.resource_forecast}<span class="status-tag" class:status-ready={entry.resource_forecast.level === "guarded"} class:status-warning={entry.resource_forecast.level === "constrained" || entry.resource_forecast.level === "unknown"} class:status-error={entry.resource_forecast.level === "high" || entry.resource_forecast.level === "blocked"}>{entry.resource_forecast.level.toUpperCase()}</span><small class="file-cell-sub">{entry.resource_forecast.planned_workers ?? "—"} workers · {entry.resource_forecast.planned_frame_threads ?? "—"} frame threads</small><small class="file-cell-sub" title={entry.resource_forecast.messages.join(" ")}>{entry.resource_forecast.planning_budget_gib !== null && entry.resource_forecast.planning_budget_gib !== undefined ? `${entry.resource_forecast.planning_budget_gib.toFixed(1)} GiB planning budget` : "Memory budget unavailable"}{entry.resource_forecast.swap_used_percent !== null && entry.resource_forecast.swap_used_percent !== undefined ? ` · swap ${entry.resource_forecast.swap_used_percent.toFixed(0)}% used` : ""}</small>{:else}<small class="file-cell-sub">No forecast</small>{/if}</td>
          <td><span class="status-tag" class:status-ready={entry.status === "Ready"} class:status-warning={entry.status === "Backed up"} class:status-error={entry.status === "Analysis failed" || entry.status === "Tools required"}>{statusLabel(entry.status)}</span><small class="file-cell-sub">{bytes(entry.size_bytes)}</small></td></tr>
      {/each}
    </tbody></table></div>
  {:else}
    <div class="empty-state compact"><div class="empty-icon"><Search size={19} /></div><div><strong>{search ? "No matching files" : "No media candidates found"}</strong><p>{search ? "Try another filename or status." : "Try enabling subfolder scanning or lowering the minimum file size."}</p></div>{#if !search}<button class="text-button" onclick={onOpenLibrary}>Adjust scan options <ArrowRight size={14} /></button>{/if}</div>
  {/if}
{/snippet}

{#snippet EnvironmentPanel(environment: Environment | null)}
  <section class="environment-section"><div class="section-title-row"><div><div class="eyebrow">MACHINE AWARENESS</div><h2>Environment & capabilities</h2></div><button class="text-button" onclick={loadEnvironment} disabled={environmentLoading}><RefreshCw size={14} class={environmentLoading ? "spin" : ""} /> Recheck</button></div>
    {#if environment}
      <div class="environment-layout"><div class="hardware-card"><div class="hardware-topline"><span class="hardware-avatar"><Monitor size={18} /></span><span class="hardware-os">{environment.os.name} {environment.os.version}</span><span class="status-tag" class:status-ready={environment.os.supported} class:status-error={!environment.os.supported}>{environment.os.supported ? "SUPPORTED" : "UNSUPPORTED"}</span></div><h3>{environment.hardware.cpu_model}</h3><div class="hardware-specs"><span><Cpu size={15} /> {environment.hardware.logical_cpus} logical CPUs</span><span><MemoryStick size={15} /> {environment.hardware.memory_total_gib?.toFixed(1) ?? "—"} GiB effective RAM</span><span><HardDrive size={15} /> {environment.hardware.memory_available_gib?.toFixed(1) ?? "—"} GiB available</span><span><Database size={15} /> Swap {environment.hardware.swap_used_gib?.toFixed(1) ?? "—"}/{environment.hardware.swap_total_gib?.toFixed(1) ?? "—"} GiB used{environment.hardware.swap_used_percent !== null ? ` (${environment.hardware.swap_used_percent.toFixed(0)}%)` : ""} · {environment.hardware.swap_free_gib?.toFixed(1) ?? "—"} free</span></div>
          <div class="gpu-strip"><Zap size={15} /><span>{environment.hardware.gpu_devices.length ? environment.hardware.gpu_devices.join(" · ") : "No GPU device detected"}</span><small>DETECTED · NOT USED FOR ENCODING</small></div>
        </div><div class="tool-check-card"><div class="tool-check-heading"><strong>Toolchain health</strong><span>{environment.checks.filter((item) => item.available).length}/{environment.checks.length} ready</span></div><div class="tool-check-list">{#each environment.checks as check}<div class="tool-check-row"><span class="check-mark" class:check-bad={!check.available}>{#if check.available}<Check size={13} />{:else}<X size={13} />{/if}</span><span class="tool-check-copy"><strong>{check.label}</strong><small>{check.path ?? check.detail}</small></span><span class="tool-version">{check.available ? check.version?.replace(/^.*?\s/, "") ?? "Ready" : check.required ? "Required" : "Optional"}</span></div>{/each}</div></div></div>
      <div class="encoder-note"><Info size={15} /><span>Hardware inventory is used for resource planning and diagnostics. The current rebuild engine uses <strong>CPU libx265</strong>; GPU encoding is not enabled.</span><button class="text-button" onclick={() => (page = "run")}>Resource settings <ArrowRight size={14} /></button></div>
    {:else}<div class="empty-state compact"><LoaderCircle class="spin" size={18} /><div><strong>{environmentLoading || booting ? "Checking this computer…" : "Could not read the environment"}</strong><p>{environmentError || "The desktop backend may be unavailable. Recheck after setup."}</p></div>{#if !environmentLoading}<button class="button secondary" onclick={loadEnvironment}>Retry check</button>{/if}</div>{/if}
  </section>
{/snippet}
