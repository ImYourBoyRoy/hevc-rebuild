// ./desktop/src/lib/reports-controller.ts
// Report history and diagnostic logs extracted from the route (helper slice): loading and permanently deleting
// report history, choosing a report, reading and copying its diagnostic log and recovering an interrupted session.
// The native bridge and clipboard are injected so every branch is unit-tested in node (pnpm test:reports-controller).
// The code is the route's code moved verbatim.

import type { InterruptedRecovery, MediaQueueRunState, RunReport } from "./types.js";


type ReportsPayload = { reports: RunReport[] };

export interface ReportsHost {
  reports: RunReport[];
  selectedReportPath: string;
  selectedLog: string;
  reportDeleteConfirm: boolean;
  reportDeleteBusy: boolean;
  logLoading: boolean;
  runStates: Record<string, MediaQueueRunState>;
  setNotice(message: string, kind?: "info" | "success" | "error"): void;
}

export interface ReportsDeps {
  invoke: <T>(command: string, args?: Record<string, unknown>) => Promise<T>;
  clipboard: { writeText(value: string): Promise<void> };
  host: ReportsHost;
}

export function createReportsController(deps: ReportsDeps) {
  const { host: h, invoke, clipboard } = deps;
  const setNotice = h.setNotice.bind(h);

  async function deleteReportHistory() {
    if (!h.reportDeleteConfirm) {
      h.reportDeleteConfirm = true;
      return;
    }
    h.reportDeleteBusy = true;
    try {
      const removed = await invoke<number>("delete_reports");
      h.reports = [];
      h.selectedReportPath = "";
      h.selectedLog = "";
      h.reportDeleteConfirm = false;
      await loadReports();
      setNotice(
        removed
          ? `Permanently deleted ${removed} report file${removed === 1 ? "" : "s"}.`
          : "No saved reports were found to delete.",
        "success",
      );
    } catch (error) {
      setNotice(`Could not delete the report history: ${String(error)}`, "error");
    } finally {
      h.reportDeleteBusy = false;
    }
  }

  async function loadReports() {
    try {
      const payload = await invoke<ReportsPayload>("list_reports", { limit: 20 });
      h.reports = payload.reports;
      if (!h.reports.some((report) => report.csv_path === h.selectedReportPath)) {
        h.selectedReportPath = h.reports[0]?.csv_path ?? "";
      }
    } catch (error) {
      setNotice(`Could not read report history: ${String(error)}`, "error");
    }
  }

  async function recoverInterruptedSession() {
    try {
      const recovery = await invoke<InterruptedRecovery>("reconcile_interrupted_runs");
      if (recovery.skipped) {
        h.runStates = recovery.runStates;
        return;
      }
      h.runStates = recovery.runStates;
      if (recovery.summary) setNotice(recovery.summary, recovery.held > 0 ? "info" : "success");
    } catch (error) {
      setNotice(`Could not recover the interrupted session: ${String(error)}`, "error");
    }
  }

  async function openLog(path: string | null | undefined) {
    if (!path) {
      h.selectedLog = "This report has no companion diagnostic log.";
      return;
    }
    h.logLoading = true;
    try {
      h.selectedLog = await invoke<string>("read_log", { path });
    } catch (error) {
      h.selectedLog = `Could not read diagnostic log: ${String(error)}`;
    } finally {
      h.logLoading = false;
    }
  }

  function chooseReport(path: string) {
    h.selectedReportPath = path;
    const report = h.reports.find((item) => item.csv_path === path);
    if (report) void openLog(report.log_path);
  }

  async function copyLog() {
    try {
      await clipboard.writeText(h.selectedLog);
      setNotice("Diagnostic excerpt copied to the clipboard.", "success");
    } catch (error) {
      setNotice(`Could not copy diagnostic text: ${String(error)}`, "error");
    }
  }

  return { deleteReportHistory, loadReports, recoverInterruptedSession, openLog, chooseReport, copyLog };
}
