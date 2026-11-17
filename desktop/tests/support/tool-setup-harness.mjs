// ./desktop/tests/support/tool-setup-harness.mjs
// Shared fixtures and the fake-host harness for the tool-setup tests (not a test file).

import { createToolSetupController } from "../../src/lib/tool-setup-controller.ts";

export const check = (key, over = {}) => ({ key, label: key.toUpperCase(), available: true, version: "1.0", path: `/usr/bin/${key}`, detail: "", ...over });

export const aptEnvironment = (over = {}) => ({
  ready: false,
  os: { supported: true, package_manager: "apt", package_manager_available: true, name: "Ubuntu", version: "24.04" },
  missing_apt_packages: [],
  checks: [check("ffmpeg"), check("pkexec"), check("hdr10plus_tool")],
  ...over,
});

export function harness({ desktop = true, invokeImpl = {}, hostOver = {}, saveResult = "/tmp/exported.log" } = {}) {
  const log = [];
  const invokes = [];
  const state = {
    environment: aptEnvironment(), environmentLoading: false, environmentError: "",
    aptUpdates: null, aptChecking: false, setupConfirmOpen: false, setupPlan: null, setupUpdates: null, repairId: "", activeToolSetupLogId: "", packageJobKind: null,
    repairLines: [], repairSummary: "", repairSummaryIsError: false, repairLogWarning: "",
    toolSetupLogs: [], selectedToolSetupLog: "", viewedToolSetupLog: "", toolSetupLogLoading: false, toolSetupLogError: "", toolSetupLogExporting: false,
    repairToolChecksBefore: null, repairAffectedToolKeys: [], repairToolOutcomes: [],
    scanning: false, runId: "", runStarting: false, autoUpdateTools: false,
    ...hostOver,
  };
  const host = state;
  Object.defineProperty(host, "aptHost", { get: () => state.environment?.os.supported === true && state.environment?.os.package_manager === "apt" && state.environment?.os.package_manager_available !== false });
  Object.defineProperty(host, "workflowBusy", { get: () => Boolean(state.scanning || state.runStarting || state.runId || state.repairId) });
  host.setNotice = (message, kind = "info") => log.push(["notice", kind, message]);
  host.currentToolPaths = () => ({ ffmpeg: "/opt/ffmpeg", ffprobe: null });
  const invoke = async (command, args) => {
    invokes.push({ command, args });
    const impl = invokeImpl[command];
    if (impl === undefined) throw new Error(`unexpected command ${command}`);
    return typeof impl === "function" ? impl(args) : impl;
  };
  const controller = createToolSetupController({
    invoke, isDesktop: () => desktop, host,
    dialogs: { save: async (options) => { log.push(["save", options]); return saveResult; } },
  });
  return { controller, host, log, invokes };
}

export const notices = (h) => h.log.filter((entry) => entry[0] === "notice").map((entry) => entry.slice(1));

export const finishHarness = (hostOver = {}, invokeImpl = {}) => harness({
  hostOver: { repairId: "job", ...hostOver },
  invokeImpl: { inspect_environment: aptEnvironment({ ready: true }), list_tool_setup_logs: [], check_ubuntu_package_updates: { packages: [], catalog_stale: false }, provision_plan: { nothing_to_do: true, install_packages: [], upgrade_packages: [], helper: { action: "current" } }, ...invokeImpl },
});
