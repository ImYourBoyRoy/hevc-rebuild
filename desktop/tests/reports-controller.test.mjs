// ./desktop/tests/reports-controller.test.mjs
// Characterization tests for the reports/logs module extracted from the route (helper slice): report
// history load and delete confirmation, report selection, diagnostic log viewing and copying, and the
// interrupted-session recovery. The native bridge and clipboard are injected fakes.
// Run: pnpm test:reports-controller

import assert from "node:assert/strict";
import test from "node:test";
import { createReportsController } from "../src/lib/reports-controller.ts";

function harness({ invokeImpl = {}, hostOver = {}, clipboardFail = false } = {}) {
  const log = [];
  const invokes = [];
  const written = [];
  const state = { reports: [], selectedReportPath: "", selectedLog: "", reportDeleteConfirm: false, reportDeleteBusy: false, logLoading: false, runStates: {}, ...hostOver };
  const host = state;
  host.setNotice = (message, kind = "info") => log.push([kind, message]);
  const invoke = async (command, args) => {
    invokes.push({ command, args });
    const impl = invokeImpl[command];
    if (impl === undefined) throw new Error(`unexpected ${command}`);
    return typeof impl === "function" ? impl(args) : impl;
  };
  const clipboard = { writeText: async (value) => { if (clipboardFail) throw new Error("denied"); written.push(value); } };
  return { ctl: createReportsController({ invoke, host, clipboard }), host, log, invokes, written };
}

const report = (csv, logPath = `${csv}.log`) => ({ csv_path: csv, log_path: logPath });

test("loadReports stores the history and keeps the selection only while it still exists", async () => {
  const reports = [report("/r/2.csv"), report("/r/1.csv")];
  const h = harness({ invokeImpl: { list_reports: { reports } } });
  await h.ctl.loadReports();
  assert.deepEqual(h.invokes[0], { command: "list_reports", args: { limit: 20 } });
  assert.deepEqual([h.host.reports, h.host.selectedReportPath], [reports, "/r/2.csv"]);
  h.host.selectedReportPath = "/r/1.csv";
  await h.ctl.loadReports();
  assert.equal(h.host.selectedReportPath, "/r/1.csv");
  h.host.selectedReportPath = "/gone.csv";
  await h.ctl.loadReports();
  assert.equal(h.host.selectedReportPath, "/r/2.csv");
  const empty = harness({ invokeImpl: { list_reports: { reports: [] } }, hostOver: { selectedReportPath: "/old.csv" } });
  await empty.ctl.loadReports();
  assert.equal(empty.host.selectedReportPath, "");
  const failed = harness({ invokeImpl: { list_reports: () => { throw new Error("io"); } } });
  await failed.ctl.loadReports();
  assert.deepEqual(failed.log, [["error", "Could not read report history: Error: io"]]);
});

test("deleteReportHistory needs a confirming second call, then deletes and refreshes", async () => {
  const h = harness({ invokeImpl: { delete_reports: 3, list_reports: { reports: [] } }, hostOver: { reports: [report("/a")], selectedReportPath: "/a", selectedLog: "log" } });
  await h.ctl.deleteReportHistory();
  assert.deepEqual([h.host.reportDeleteConfirm, h.invokes.length], [true, 0]);
  const pending = h.ctl.deleteReportHistory();
  assert.equal(h.host.reportDeleteBusy, true);
  await pending;
  assert.deepEqual(h.invokes.map((call) => call.command), ["delete_reports", "list_reports"]);
  assert.deepEqual([h.host.reports, h.host.selectedReportPath, h.host.selectedLog, h.host.reportDeleteConfirm, h.host.reportDeleteBusy], [[], "", "", false, false]);
  assert.deepEqual(h.log, [["success", "Permanently deleted 3 report files."]]);
  const one = harness({ invokeImpl: { delete_reports: 1, list_reports: { reports: [] } }, hostOver: { reportDeleteConfirm: true } });
  await one.ctl.deleteReportHistory();
  assert.deepEqual(one.log, [["success", "Permanently deleted 1 report file."]]);
  const none = harness({ invokeImpl: { delete_reports: 0, list_reports: { reports: [] } }, hostOver: { reportDeleteConfirm: true } });
  await none.ctl.deleteReportHistory();
  assert.deepEqual(none.log, [["success", "No saved reports were found to delete."]]);
});

test("deleteReportHistory failure reports the error and clears busy", async () => {
  const h = harness({ invokeImpl: { delete_reports: () => { throw new Error("locked"); } }, hostOver: { reportDeleteConfirm: true, reports: [report("/a")] } });
  await h.ctl.deleteReportHistory();
  assert.deepEqual(h.log, [["error", "Could not delete the report history: Error: locked"]]);
  assert.deepEqual([h.host.reportDeleteBusy, h.host.reports.length], [false, 1]);
});

test("openLog reads a log, explains a missing one and reports a read failure", async () => {
  const h = harness({ invokeImpl: { read_log: (args) => `log of ${args.path}` } });
  const pending = h.ctl.openLog("/r/1.log");
  assert.equal(h.host.logLoading, true);
  await pending;
  assert.deepEqual([h.host.selectedLog, h.host.logLoading], ["log of /r/1.log", false]);
  await h.ctl.openLog(null);
  assert.equal(h.host.selectedLog, "This report has no companion diagnostic log.");
  await h.ctl.openLog(undefined);
  assert.equal(h.invokes.length, 1);
  const failed = harness({ invokeImpl: { read_log: () => { throw new Error("gone"); } } });
  await failed.ctl.openLog("/x.log");
  assert.deepEqual([failed.host.selectedLog, failed.host.logLoading], ["Could not read diagnostic log: Error: gone", false]);
});

test("chooseReport selects the report and opens its log when it exists", async () => {
  const h = harness({ invokeImpl: { read_log: "text" }, hostOver: { reports: [report("/a.csv", "/a.log"), report("/b.csv", null)] } });
  h.ctl.chooseReport("/a.csv");
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual([h.host.selectedReportPath, h.host.selectedLog], ["/a.csv", "text"]);
  h.ctl.chooseReport("/b.csv");
  assert.equal(h.host.selectedLog, "This report has no companion diagnostic log.");
  h.ctl.chooseReport("/unknown.csv");
  assert.deepEqual([h.host.selectedReportPath, h.invokes.length], ["/unknown.csv", 1]);
});

test("copyLog copies the excerpt and confirms, or explains the failure", async () => {
  const h = harness({ hostOver: { selectedLog: "diagnostics" } });
  await h.ctl.copyLog();
  assert.deepEqual([h.written, h.log], [["diagnostics"], [["success", "Diagnostic excerpt copied to the clipboard."]]]);
  const failed = harness({ clipboardFail: true });
  await failed.ctl.copyLog();
  assert.deepEqual(failed.log, [["error", "Could not copy diagnostic text: Error: denied"]]);
});

test("recoverInterruptedSession stores the run states and only announces a real summary", async () => {
  const states = { "/a.mkv": { status: "interrupted" } };
  const held = harness({ invokeImpl: { reconcile_interrupted_runs: { skipped: false, runStates: states, summary: "2 files held for review.", held: 2 } } });
  await held.ctl.recoverInterruptedSession();
  assert.deepEqual([held.host.runStates, held.log], [states, [["info", "2 files held for review."]]]);
  const done = harness({ invokeImpl: { reconcile_interrupted_runs: { skipped: false, runStates: states, summary: "Nothing to recover.", held: 0 } } });
  await done.ctl.recoverInterruptedSession();
  assert.deepEqual(done.log, [["success", "Nothing to recover."]]);
  const skipped = harness({ invokeImpl: { reconcile_interrupted_runs: { skipped: true, runStates: states, summary: "ignored", held: 1 } } });
  await skipped.ctl.recoverInterruptedSession();
  assert.deepEqual([skipped.host.runStates, skipped.log], [states, []]);
  const quiet = harness({ invokeImpl: { reconcile_interrupted_runs: { skipped: false, runStates: {}, summary: "", held: 0 } } });
  await quiet.ctl.recoverInterruptedSession();
  assert.deepEqual(quiet.log, []);
  const failed = harness({ invokeImpl: { reconcile_interrupted_runs: () => { throw new Error("db"); } } });
  await failed.ctl.recoverInterruptedSession();
  assert.deepEqual(failed.log, [["error", "Could not recover the interrupted session: Error: db"]]);
});
