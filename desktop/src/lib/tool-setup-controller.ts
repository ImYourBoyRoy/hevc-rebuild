// ./desktop/src/lib/tool-setup-controller.ts
// Environment inspection, APT update/catalog/toolchain jobs, the HDR10+ helper install, saved setup
// logs and the three repair-* event handlers, extracted from the route (repair slice). The route's
// state is reached through a typed `ToolSetupHost`; the native bridge and the save dialog are
// injected so every branch is unit-tested in node (pnpm test:tool-setup). The code is the route's
// code moved verbatim with `h.` prefixes. No direct Tauri imports.

import type { AptUpdateStatus, CommandCheck, Environment, ProvisionPlan, RunLineEvent, ToolPaths } from './types.js';
import type { RepairToolOutcome, ToolSetupLogSummary } from './ui-contracts.js';

export type PackageJobKind = "provision" | null;
export type RepairFinished = { jobId: string; success: boolean; error: string | null; logWarning?: string | null };

export interface ToolSetupHost {
  environment: Environment | null;
  environmentLoading: boolean;
  environmentError: string;
  aptUpdates: AptUpdateStatus | null;
  aptChecking: boolean;
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
  readonly aptHost: boolean;
  readonly autoUpdateTools: boolean;
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

  /** The part of a plan that needs no password: the HDR10+ helper, and Homebrew (which runs as the user). */
  function passwordFreeWork(plan: ProvisionPlan): boolean {
    const helper = plan.helper.action === "install" || plan.helper.action === "update";
    const brew = plan.manager === "brew" && !plan.blockers.length && (plan.install_packages.length > 0 || plan.upgrade_packages.length > 0);
    return helper || brew;
  }

  let lastAutomaticPlan = "";

  /** Silent background check (launch and every few hours): remembers what is missing or outdated for
   *  the header notice, and, when the owner switched on "keep tools up to date", applies only the
   *  password-free part while the app is idle. It never opens a dialog or asks for a password. */
  async function checkSetupUpdates() {
    if (!isDesktop() || h.runId || h.runStarting || h.repairId || h.scanning) return;
    try {
      h.setupUpdates = await invoke<ProvisionPlan>("provision_plan", { toolPaths: h.currentToolPaths() });
    } catch {
      /* Offline or unsupported hosts keep the last known result; the manual button reports errors. */
      return;
    }
    await applyAutomaticUpdates();
  }

  async function applyAutomaticUpdates() {
    const plan = h.setupUpdates;
    if (!h.autoUpdateTools || !plan || !passwordFreeWork(plan)) return;
    if (h.runId || h.runStarting || h.repairId || h.scanning) return;
    // Never retry the same plan in a loop after a failure; a different plan or a restart tries again.
    const key = JSON.stringify([plan.manager, plan.install_packages, plan.upgrade_packages, plan.helper]);
    if (key === lastAutomaticPlan) return;
    lastAutomaticPlan = key;
    h.repairLines = [];
    h.repairLogWarning = "";
    h.repairSummary = "Updating tools in the background…";
    h.repairSummaryIsError = false;
    h.repairToolChecksBefore = h.environment?.checks.map((check) => ({ ...check })) ?? null;
    h.repairAffectedToolKeys = plan.helper.action === "install" || plan.helper.action === "update" ? ["hdr10plus_tool"] : [];
    h.repairToolOutcomes = [];
    try {
      h.repairId = await invoke<string>("provision_toolchain", { toolPaths: h.currentToolPaths(), userSpaceOnly: true });
      h.packageJobKind = "provision";
    } catch (error) {
      h.repairId = "";
      h.repairSummary = `Automatic update could not start: ${String(error)}`;
      h.repairSummaryIsError = true;
    }
  }

  /** One pass: fetch the plan (packages, catalog, helper) and ask once; nothing runs until confirmSetup. */
  async function reviewSetup() {
    if (!isDesktop() || h.runId || h.runStarting || h.repairId || h.scanning) return;
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
    if (!h.setupPlan || h.runId || h.runStarting || h.repairId || h.scanning) return;
    const plan = h.setupPlan;
    h.setupConfirmOpen = false;
    h.repairLines = [];
    h.repairLogWarning = "";
    h.repairSummary = plan.needs_password ? "Waiting for administrator authentication (one prompt)…" : "Installing and updating…";
    h.repairSummaryIsError = false;
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

  function handleRepairLine(payload: RunLineEvent) {
    if (payload.runId === h.repairId) h.repairLines = [...h.repairLines, payload.line].slice(-120);
  }

  function handleRepairLogWarning(payload: RunLineEvent) {
    if (payload.runId === h.activeToolSetupLogId) h.repairLogWarning = payload.line;
  }

  async function handleRepairFinished(payload: RepairFinished) {
    if (payload.jobId !== h.repairId) return;
    const checksBefore = h.repairToolChecksBefore;
    const affectedToolKeys = h.repairAffectedToolKeys;
    h.repairLogWarning = payload.logWarning ?? "";
    h.repairId = "";
    h.packageJobKind = null;
    h.repairSummary = payload.success ? "Setup and updates finished; checking installed versions…" : payload.error ?? "The setup did not finish successfully.";
    h.repairSummaryIsError = !payload.success;
    h.setNotice(payload.success ? "Setup and updates completed. Rechecking installed tools." : payload.error ?? "The setup did not finish successfully.", payload.success ? "success" : "error");
    await refreshToolSetupLogs(payload.jobId);
    const environmentLoaded = await loadEnvironment();
    if (environmentLoaded && affectedToolKeys.length) recordRepairToolOutcomes(checksBefore, h.environment?.checks ?? []);
    h.repairToolChecksBefore = null;
    h.repairAffectedToolKeys = [];
    if (h.aptHost) await checkUbuntuPackageUpdates(true);
    await checkSetupUpdates();
    if (payload.success) {
      h.repairSummary = h.environment?.ready
        ? "Setup complete. All required media tools are ready."
        : "Setup finished. One or more required tools still need attention; check their status in Settings.";
    }
  }

  return { loadEnvironment, aptPackageForTool, packageUpdateForTool, toolCheckReadout, recordRepairToolOutcomes, toolKeysForPackages, reviewSetup, confirmSetup, checkSetupUpdates, checkUbuntuPackageUpdates, refreshToolSetupLogs, openToolSetupLog, exportToolSetupLog, handleRepairLine, handleRepairLogWarning, handleRepairFinished };
}
