// ./desktop/src/lib/latest-outcomes.ts
// Which failures still count. A file is failed only when its most recent decisive outcome is a failure:
// an older failure is superseded by a later success, repeated failures of one file count once, and a
// read-only check (INFO), a skip or an interruption never clears a failure. An intact integrity check
// (INTEGRITY_OK) clears a damaged or unreadable file but not a failed rebuild. Pure: no IPC, no DOM, no
// state. Tested by pnpm test:latest-outcomes.

import type { ReportRow, RunReport } from './types.js';
import { isFailure } from './outcomes.ts';

type Kind = "failure" | "success" | "integrity-ok" | "neutral";

function classify(status: string | undefined): Kind {
  if (isFailure(status)) return "failure";
  const normalized = (status ?? "").trim().toUpperCase();
  if (normalized === "OK" || normalized === "METADATA_OK" || normalized === "TEST_OK") return "success";
  if (normalized === "INTEGRITY_OK") return "integrity-ok";
  return "neutral";
}

function isIntegrityFailure(status: string | undefined): boolean {
  const normalized = (status ?? "").trim().toUpperCase();
  return normalized === "CORRUPT" || normalized === "UNREADABLE";
}

/**
 * The failure rows that are each file's current state, newest report first (original row order within a
 * report). Reports are ordered by `created`, so the order they are listed in does not matter; within one
 * report the later row for a file wins. Rows without a path are never merged with each other.
 */
export function latestFailureRows(reports: readonly RunReport[]): ReportRow[] {
  const ordered = reports
    .map((report, index) => ({ report, index }))
    .sort((a, b) => (b.report.created - a.report.created) || (a.index - b.index));
  const seen = new Map<string, { done: boolean; integrityOk: boolean }>();
  const kept = new Set<ReportRow>();
  for (const { report } of ordered) {
    for (let i = report.rows.length - 1; i >= 0; i--) {
      const row = report.rows[i];
      if (!row.input) { if (isFailure(row.status)) kept.add(row); continue; }
      let state = seen.get(row.input);
      if (!state) { state = { done: false, integrityOk: false }; seen.set(row.input, state); }
      if (state.done) continue;
      const kind = classify(row.status);
      if (kind === "neutral") continue;
      if (kind === "integrity-ok") { state.integrityOk = true; continue; }
      state.done = true;
      if (kind === "failure" && !(state.integrityOk && isIntegrityFailure(row.status))) kept.add(row);
    }
  }
  const result: ReportRow[] = [];
  for (const { report } of ordered) for (const row of report.rows) if (kept.has(row)) result.push(row);
  return result;
}
