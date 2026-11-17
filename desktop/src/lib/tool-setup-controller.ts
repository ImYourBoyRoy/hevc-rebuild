// ./desktop/src/lib/tool-setup-controller.ts
// Environment inspection, APT update/catalog/toolchain jobs, the HDR10+ helper install, saved setup
// logs and the three repair-* event handlers, extracted from the route (repair slice). The route's
// state is reached through a typed `ToolSetupHost`; the native bridge and the save dialog are
// injected so every branch is unit-tested in node (pnpm test:tool-setup). The code is the route's
// code moved verbatim with `h.` prefixes. No direct Tauri imports.

import type { AptUpdateStatus, CommandCheck, Environment, ProvisionPlan, RunLineEvent, ToolPaths } from './types.js';
import type { RepairToolOutcome, ToolSetupLogSummary } from './ui-contracts.js';

export type PackageJobKind = "repair" | "catalog-refresh" | "upgrade" | "toolchain" | "provision" | null;
export type RepairFinished = { jobId: string; success: boolean; error: string | null; logWarning?: string | null };

export interface ToolSetupHost {
  environment: Environment | null;
  environmentLoading: boolean;
  environmentError: string;
  aptUpdates: AptUpdateStatus | null;
  aptChecking: boolean;
  repairConfirmOpen: boolean;
  setupConfirmOpen: boolean;
  setupPlan: ProvisionPlan | null;
  setupUpdates: ProvisionPlan | null;
  repairId: string;
  activeToolSetupLogId: string;
  packageJobKind: PackageJobKind;
  repairLines: string[];
  repairSummary: string;
  repairSummaryIsError: boolean;
  repairLogWarning: string;
  toolSetupLogs: ToolSetupLogSummary[];
  selectedToolSetupLog: string;
  viewedToolSetupLog: string;
  toolSetupLogLoading: boolean;
  toolSetupLogError: string;
  toolSetupLogExporting: boolean;
  repairToolChecksBefore: CommandCheck[] | null;
  repairAffectedToolKeys: string[];
  repairToolOutcomes: RepairToolOutcome[];
  hdrInstalling: boolean;
  continueToolchainSetupAfterRefresh: boolean;
  installHdrHelperAfterPackages: boolean;
  readonly aptHost: boolean;
  readonly workflowBusy: boolean;
  readonly scanning: boolean;
  readonly runId: string;
  readonly runStarting: boolean;
  setNotice(message: string, kind?: "info" | "success" | "error"): void;
  currentToolPaths(): ToolPaths;
}

export interface ToolSetupDeps {
  invoke: <T>(command: string, args?: Record<string, unknown>) => Promise<T>;
  isDesktop: () => boolean;
  host: ToolSetupHost;
  dialogs: { save(options: { title: string; defaultPath: string; filters: { name: string; extensions: string[] }[] }): Promise<string | null> };
}

export function createToolSetupController(deps: ToolSetupDeps) {
  const { invoke, isDesktop, host: h, dialogs } = deps;

  async function loadEnvironment(): Promise<boolean> {
    h.environmentLoading = true;
    h.environmentError = "";
    try {
      h.environment = await invoke<Environment>("inspect_environment", { toolPaths: h.currentToolPaths() });
      return true;
    } catch (error) {
      h.environmentError = String(error);
      h.environment = null;
      h.setNotice(`Could not check the selected executables: ${h.environmentError}`, "error");
      return false;
    } finally {
      h.environmentLoading = false;
    }
  }

  function aptPackageForTool(checkKey: string): string | null {
    if (checkKey === "ffmpeg" || checkKey === "ffprobe") return "ffmpeg";
    if (["mkvpropedit", "mkvmerge", "mkvextract"].includes(checkKey)) return "mkvtoolnix";
    if (["mediainfo", "fontconfig", "ca-certificates"].includes(checkKey)) return checkKey;
    return null;
  }

  function packageUpdateForTool(checkKey: string) {
    const packageName = aptPackageForTool(checkKey);
    return packageName ? h.aptUpdates?.packages.find((item) => item.name === packageName) : undefined;
  }

  function toolCheckReadout(check: CommandCheck | undefined): string {
    if (!check) return "Not checked";
    if (!check.available) return `Missing · ${check.path ?? check.detail}`;
    return `${check.version ?? "Available"}${check.path ? ` · ${check.path}` : ""}`;
  }

  function recordRepairToolOutcomes(before: CommandCheck[] | null, after: CommandCheck[]) {
    if (!before || !h.repairAffectedToolKeys.length) return;
    const beforeByKey = new Map(before.map((check) => [check.key, check]));
    const afterByKey = new Map(after.map((check) => [check.key, check]));
    const latest = new Map(h.repairToolOutcomes.map((outcome) => [outcome.key, outcome]));
    for (const key of h.repairAffectedToolKeys) {
      const oldCheck = beforeByKey.get(key);
      const newCheck = afterByKey.get(key);
      if (!newCheck) continue;
      const status: RepairToolOutcome["status"] = !newCheck.available
        ? "Still missing"
        : !oldCheck?.available
          ? "Installed"
          : oldCheck.version !== newCheck.version || oldCheck.path !== newCheck.path
            ? "Updated or re-resolved"
            : "Ready · unchanged";
      latest.set(key, {
        key,
        label: newCheck.label,
        before: toolCheckReadout(oldCheck),
        after: toolCheckReadout(newCheck),
        status,
      });
    }
    h.repairToolOutcomes = [...latest.values()];
  }

  function toolKeysForPackages(packages: string[]): string[] {
    const selectedPackages = new Set(packages);
    return [...new Set((h.environment?.checks ?? [])
      .filter((check) => {
        const packageName = aptPackageForTool(check.key);
        return packageName !== null && selectedPackages.has(packageName);
      })
      .map((check) => check.key))];
  }

  async function repairApt() {
    if (h.runId || h.runStarting || h.repairId || h.hdrInstalling || h.scanning) return;
    if (!h.environment?.os.supported || !h.aptHost) {
      h.setNotice("Automatic APT repair is available only on Linux hosts using APT. Use the platform setup documented in Help & commands.", "error");
      return;
    }
    if (!h.aptUpdates) {
      const status = await checkUbuntuPackageUpdates(true);
      if (!status) return;
    }
    if (h.aptUpdates?.catalog_stale) {
      h.continueToolchainSetupAfterRefresh = true;
      h.setNotice("The Ubuntu package catalog is stale. Refreshing it first; the package list will be shown for review before anything is installed.", "info");
      await refreshUbuntuPackageCatalog(true);
      return;
    }
    const missing = h.environment.missing_apt_packages;
    const helperMissing = h.environment.checks.some((check) => check.key === "hdr10plus_tool" && !check.available);
    if (!missing.length && !h.aptUpdates?.packages.length) {
      if (helperMissing) {
        await installHdrHelper();
        return;
      }
      h.repairSummary = "The checked Ubuntu packages are installed and current in the enabled repositories.";
      h.repairSummaryIsError = false;
      h.setNotice("The required system tools are installed and current in the enabled Ubuntu package sources.", "success");
      return;
    }
    if (!h.environment.checks.some((check) => check.key === "pkexec" && check.available)) {
      h.repairSummary = "Desktop administrator authentication (pkexec) is missing. No system packages were changed.";
      h.repairSummaryIsError = true;
      h.setNotice("Desktop administrator authentication (pkexec) is missing. Install the listed system packages through Ubuntu Software or a terminal.", "error");
      return;
    }
    h.repairConfirmOpen = true;
  }

  /** Silent background check (launch and every few hours): remembers what is missing or outdated for the header notice. */
  async function checkSetupUpdates() {
    if (!isDesktop() || h.runId || h.runStarting || h.repairId || h.hdrInstalling || h.scanning) return;
    try {
      h.setupUpdates = await invoke<ProvisionPlan>("provision_plan", { toolPaths: h.currentToolPaths() });
    } catch {
      /* Offline or unsupported hosts keep the last known result; the manual button reports errors. */
    }
  }

  /** One pass: fetch the plan (packages, catalog, helper) and ask once; nothing runs until confirmSetup. */
  async function reviewSetup() {
    if (!isDesktop() || h.runId || h.runStarting || h.repairId || h.hdrInstalling || h.scanning) return;
    try {
      const plan = await invoke<ProvisionPlan>("provision_plan", { toolPaths: h.currentToolPaths() });
      h.setupPlan = plan;
      if (plan.nothing_to_do) {
        h.setupConfirmOpen = false;
        h.repairSummary = "Everything is installed and up to date.";
        h.repairSummaryIsError = false;
        h.setNotice("Everything is installed and up to date.", "success");
        return;
      }
      h.setupConfirmOpen = true;
    } catch (error) {
      h.setupConfirmOpen = false;
      h.setNotice(`Could not prepare the setup plan: ${String(error)}`, "error");
    }
  }

  /** Apply the reviewed plan. The backend recomputes it and asks for the password at most once. */
  async function confirmSetup() {
    if (!h.setupPlan || h.runId || h.runStarting || h.repairId || h.hdrInstalling || h.scanning) return;
    const plan = h.setupPlan;
    h.setupConfirmOpen = false;
    h.repairLines = [];
    h.repairLogWarning = "";
    h.repairSummary = plan.needs_password ? "Waiting for administrator authentication (one prompt)…" : "Installing and updating…";
    h.repairSummaryIsError = false;
    h.installHdrHelperAfterPackages = false;
    h.repairToolChecksBefore = h.environment?.checks.map((check) => ({ ...check })) ?? null;
    const helperWork = plan.helper.action === "install" || plan.helper.action === "update";
    h.repairAffectedToolKeys = [...new Set([...toolKeysForPackages([...plan.install_packages, ...plan.upgrade_packages]), ...(helperWork ? ["hdr10plus_tool"] : [])])];
    h.repairToolOutcomes = [];
    try {
      h.repairId = await invoke<string>("provision_toolchain", { toolPaths: h.currentToolPaths() });
      h.packageJobKind = "provision";
      h.setNotice(plan.needs_password ? "Setup started. You will be asked for your password once." : "Setup started. No password is needed.", "info");
    } catch (error) {
      h.repairId = "";
      h.repairSummary = `Setup could not start: ${String(error)}`;
      h.repairSummaryIsError = true;
      h.setNotice(`Could not start setup: ${String(error)}`, "error");
    }
  }

  async function checkUbuntuPackageUpdates(silent = false): Promise<AptUpdateStatus | null> {
    if (!isDesktop() || h.workflowBusy || h.aptChecking || !h.aptHost) return h.aptUpdates;
    h.aptChecking = true;
    try {
      h.aptUpdates = await invoke<AptUpdateStatus>("check_ubuntu_package_updates");
      const count = h.aptUpdates.packages.length;
      if (!silent) {
        h.setNotice(
          h.aptUpdates.catalog_stale
            ? `Found ${count} update${count === 1 ? "" : "s"} in a stale local APT catalog. Refresh the catalog before installing.`
            : count
              ? `${count} allowlisted Ubuntu package update${count === 1 ? "" : "s"} found.`
              : "No updates are available for the checked Ubuntu packages.",
          h.aptUpdates.catalog_stale ? "info" : "success",
        );
      }
      return h.aptUpdates;
    } catch (error) {
      if (!silent) h.setNotice(`Could not check Ubuntu package updates: ${String(error)}`, "error");
      return null;
    } finally {
      h.aptChecking = false;
    }
  }

  async function refreshUbuntuPackageCatalog(continueSetup = false) {
    if (!isDesktop() || h.workflowBusy || h.aptChecking || !h.aptHost) return;
    h.repairLines = [];
    h.repairLogWarning = "";
    h.repairSummary = "Refreshing Ubuntu package information…";
    h.repairSummaryIsError = false;
    try {
      h.repairId = await invoke<string>("refresh_ubuntu_package_catalog");
      h.packageJobKind = "catalog-refresh";
      h.continueToolchainSetupAfterRefresh = continueSetup;
      h.setNotice("Refreshing the configured Ubuntu package catalog. No software is being installed.", "info");
    } catch (error) {
      h.continueToolchainSetupAfterRefresh = false;
      h.repairSummary = `Package information could not be refreshed: ${String(error)}`;
      h.repairSummaryIsError = true;
      h.setNotice(`Could not refresh the Ubuntu package catalog: ${String(error)}`, "error");
    }
  }

  async function refreshToolSetupLogs(preferredJobId = "") {
    if (!isDesktop()) return;
    try {
      h.toolSetupLogs = await invoke<ToolSetupLogSummary[]>("list_tool_setup_logs");
      if (preferredJobId && h.toolSetupLogs.some((log) => log.jobId === preferredJobId)) {
        h.selectedToolSetupLog = preferredJobId;
        await openToolSetupLog(preferredJobId);
      } else if (!h.toolSetupLogs.some((log) => log.jobId === h.selectedToolSetupLog)) {
        h.selectedToolSetupLog = h.toolSetupLogs[0]?.jobId ?? "";
        h.viewedToolSetupLog = "";
      }
      h.toolSetupLogError = "";
    } catch (error) {
      h.toolSetupLogError = `Could not load saved setup logs: ${String(error)}`;
    }
  }

  async function openToolSetupLog(jobId = h.selectedToolSetupLog) {
    if (!isDesktop() || !jobId || h.toolSetupLogLoading) return;
    h.selectedToolSetupLog = jobId;
    h.toolSetupLogLoading = true;
    h.toolSetupLogError = "";
    try {
      h.viewedToolSetupLog = await invoke<string>("read_tool_setup_log", { jobId });
    } catch (error) {
      h.viewedToolSetupLog = "";
      h.toolSetupLogError = `Could not read this setup log: ${String(error)}`;
    } finally {
      h.toolSetupLogLoading = false;
    }
  }

  async function exportToolSetupLog() {
    if (!isDesktop() || !h.selectedToolSetupLog || h.toolSetupLogExporting) return;
    h.toolSetupLogExporting = true;
    try {
      const destination = await dialogs.save({
        title: "Export tool setup log",
        defaultPath: `hevc-tool-setup-${h.selectedToolSetupLog}.log`,
        filters: [{ name: "Setup log", extensions: ["log", "txt"] }],
      });
      if (!destination) return;
      const savedPath = await invoke<string>("export_tool_setup_log", {
        jobId: h.selectedToolSetupLog,
        destination,
      });
      h.setNotice(`Saved the complete setup log to ${savedPath}.`, "success");
    } catch (error) {
      h.setNotice(`Could not export the setup log: ${String(error)}`, "error");
    } finally {
      h.toolSetupLogExporting = false;
    }
  }

  async function confirmAptRepair() {
    if (h.runId || h.runStarting || h.repairId || h.hdrInstalling || h.scanning) return;
    if (!h.environment || !h.aptUpdates || h.aptUpdates.catalog_stale) return;
    const missingPackages = [...h.environment.missing_apt_packages];
    const updatePackages = h.aptUpdates.packages.map((item) => item.name);
    if (!missingPackages.length && !updatePackages.length) return;
    h.repairConfirmOpen = false;
    h.repairLines = [];
    h.repairLogWarning = "";
    const actions = [
      ...(missingPackages.length ? [`installing ${missingPackages.length} missing package${missingPackages.length === 1 ? "" : "s"}`] : []),
      ...(updatePackages.length ? [`updating ${updatePackages.length} package${updatePackages.length === 1 ? "" : "s"}`] : []),
    ];
    h.repairSummary = `${actions.join(" and ").replace(/^./, (first) => first.toUpperCase())}…`;
    h.repairSummaryIsError = false;
    h.installHdrHelperAfterPackages = h.environment.checks.some((check) => check.key === "hdr10plus_tool" && !check.available);
    h.repairToolChecksBefore = h.environment.checks.map((check) => ({ ...check }));
    h.repairAffectedToolKeys = toolKeysForPackages([...missingPackages, ...updatePackages]);
    if (h.installHdrHelperAfterPackages) h.repairAffectedToolKeys = [...new Set([...h.repairAffectedToolKeys, "hdr10plus_tool"])];
    h.repairToolOutcomes = [];
    try {
      h.repairId = await invoke<string>("install_ubuntu_toolchain", {
        toolPaths: h.currentToolPaths(),
        confirmedMissingPackages: missingPackages,
        confirmedUpdatePackages: updatePackages,
      });
      h.packageJobKind = "toolchain";
      h.setNotice("System tool setup started. APT will install the reviewed missing packages, then apply only the listed updates.", "info");
    } catch (error) {
      h.repairId = "";
      h.installHdrHelperAfterPackages = false;
      h.repairSummary = `Tool setup could not start: ${String(error)}`;
      h.repairSummaryIsError = true;
      h.setNotice(`Could not start system tool setup: ${String(error)}`, "error");
    }
  }

  async function installHdrHelper(preservePreviousResults = false) {
    if (h.runId || h.runStarting || h.repairId || h.hdrInstalling || h.scanning) return;
    h.hdrInstalling = true;
    h.repairToolChecksBefore = h.environment?.checks.map((check) => ({ ...check })) ?? null;
    h.repairAffectedToolKeys = ["hdr10plus_tool"];
    if (!preservePreviousResults) h.repairToolOutcomes = [];
    h.repairLogWarning = "";
    const setupLogId = `${Date.now()}-${Math.floor(Math.random() * 1_000_000_000)}`;
    h.activeToolSetupLogId = setupLogId;
    h.repairSummary = "Downloading and verifying the latest HDR10+ helper…";
    h.repairSummaryIsError = false;
    h.setNotice("Installing the HDR10+ helper from its verified upstream release…", "info");
    try {
      const path = await invoke<string>("install_hdr10plus", { jobId: setupLogId });
      h.repairSummary = `HDR10+ helper installed at ${path}; checking its version…`;
      h.setNotice(`HDR10+ helper installed at ${path}. Rechecking the environment.`, "success");
      if (await loadEnvironment()) recordRepairToolOutcomes(h.repairToolChecksBefore, h.environment?.checks ?? []);
      h.repairSummary = h.environment?.checks.find((check) => check.key === "hdr10plus_tool")?.available
        ? "HDR10+ helper installed and detected."
        : "The helper installer finished, but the environment check still cannot detect HDR10+ helper.";
      h.repairSummaryIsError = !h.environment?.checks.find((check) => check.key === "hdr10plus_tool")?.available;
      await refreshToolSetupLogs(setupLogId);
    } catch (error) {
      h.repairSummary = `HDR10+ helper setup failed: ${String(error)}`;
      h.repairSummaryIsError = true;
      if (await loadEnvironment()) recordRepairToolOutcomes(h.repairToolChecksBefore, h.environment?.checks ?? []);
      await refreshToolSetupLogs(setupLogId);
      h.setNotice(`HDR10+ helper setup failed: ${String(error)}`, "error");
    } finally {
      h.hdrInstalling = false;
      h.activeToolSetupLogId = "";
      h.repairToolChecksBefore = null;
      h.repairAffectedToolKeys = [];
    }
  }

  function handleRepairLine(payload: RunLineEvent) {
    if (payload.runId === h.repairId) h.repairLines = [...h.repairLines, payload.line].slice(-120);
  }

  function handleRepairLogWarning(payload: RunLineEvent) {
    if (payload.runId === h.activeToolSetupLogId) h.repairLogWarning = payload.line;
  }

  async function handleRepairFinished(payload: RepairFinished) {
    if (payload.jobId !== h.repairId) return;
    const completedKind = h.packageJobKind;
    const checksBefore = h.repairToolChecksBefore;
    const affectedToolKeys = h.repairAffectedToolKeys;
    let helperStarted = false;
    h.repairLogWarning = payload.logWarning ?? "";
    h.repairId = "";
    h.packageJobKind = null;
    h.repairSummary = payload.success
      ? completedKind === "catalog-refresh" ? "Package information refreshed; checking installed versions…"
        : completedKind === "provision" ? "Setup and updates finished; checking installed versions…"
        : completedKind === "toolchain" ? "Reviewed package setup finished; checking installed versions…"
          : completedKind === "upgrade" ? "Package updates finished; checking installed versions…"
            : "Package repair finished; checking installed versions…"
      : payload.error ?? "The package operation did not finish successfully.";
    h.repairSummaryIsError = !payload.success;
    if (payload.success) h.setNotice(completedKind === "catalog-refresh" ? "Ubuntu package catalog refreshed." : completedKind === "provision" ? "Setup and updates completed. Rechecking installed tools." : completedKind === "toolchain" ? "Reviewed Ubuntu package setup completed. Rechecking installed tools." : completedKind === "upgrade" ? "Ubuntu package upgrades completed." : "Package repair completed. Rechecking the installed tools.", "success");
    else h.setNotice(payload.error ?? "The package operation did not finish successfully.", "error");
    await refreshToolSetupLogs(payload.jobId);
    const environmentLoaded = await loadEnvironment();
    if (environmentLoaded && affectedToolKeys.length) recordRepairToolOutcomes(checksBefore, h.environment?.checks ?? []);
    h.repairToolChecksBefore = null;
    h.repairAffectedToolKeys = [];
    if (h.aptHost && (h.aptUpdates || completedKind === "catalog-refresh" || completedKind === "upgrade" || completedKind === "toolchain" || completedKind === "provision")) await checkUbuntuPackageUpdates(true);
    if (completedKind === "catalog-refresh" && h.continueToolchainSetupAfterRefresh) {
      h.continueToolchainSetupAfterRefresh = false;
      if (payload.success) await repairApt();
    }
    if (completedKind === "toolchain" && payload.success && h.installHdrHelperAfterPackages) {
      h.installHdrHelperAfterPackages = false;
      if (h.environment?.checks.some((check) => check.key === "hdr10plus_tool" && !check.available)) {
        helperStarted = true;
        await installHdrHelper(true);
      }
    }
    if (completedKind === "toolchain" && !payload.success) h.installHdrHelperAfterPackages = false;
    if (completedKind === "catalog-refresh" && !payload.success) h.continueToolchainSetupAfterRefresh = false;
    if (completedKind === "provision") await checkSetupUpdates();
    if (payload.success && !helperStarted) {
      h.repairSummary = h.environment?.ready
        ? "Setup complete. All required media tools are ready."
        : "Package work completed. One or more required tools still need attention; check their status in Settings.";
    }
  }

  return { loadEnvironment, aptPackageForTool, packageUpdateForTool, toolCheckReadout, recordRepairToolOutcomes, toolKeysForPackages, repairApt, reviewSetup, confirmSetup, checkSetupUpdates, checkUbuntuPackageUpdates, refreshUbuntuPackageCatalog, refreshToolSetupLogs, openToolSetupLog, exportToolSetupLog, confirmAptRepair, installHdrHelper, handleRepairLine, handleRepairLogWarning, handleRepairFinished };
}
