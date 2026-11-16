// ./desktop/src/lib/state/scan-state.svelte.ts
// Scan activity, progress and the visible scan result.
// A plain rune class: its instance is the typed host the controller works on, so the route needs no
// per-field accessors. Members marked `declare` are supplied by the route (see withGetters).
import type { ScanProgress, ScanResult } from "../types.js";

export class ScanState {
  scanning = $state(false);
  scanProgress = $state<ScanProgress | null>(null);
  scanResult = $state<ScanResult | null>(null);
}
