// ./desktop/src/lib/state/reports-state.svelte.ts
// Report history, selection, diagnostic log and delete confirmation.
// A plain rune class: its instance is the typed host the controller works on, so the route needs no
// per-field accessors. Members marked `declare` are supplied by the route (see withGetters).
import type { RunReport } from "../types.js";

export class ReportsState {
  reports = $state<RunReport[]>([]);
  selectedReportPath = $state("");
  selectedLog = $state("");
  reportDeleteConfirm = $state(false);
  reportDeleteBusy = $state(false);
  logLoading = $state(false);
}
