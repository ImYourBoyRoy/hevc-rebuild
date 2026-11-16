// ./desktop/src/lib/state/workbench-derived.svelte.ts
// Derived values of the metadata workbench: whether the desktop shell is present, the series groups of the analyzed scan
// (all and the ones visible for the focused file) and the organizer files visible for the focused file. State and props are
// injected; each field is computed lazily.

import { isTauri } from "@tauri-apps/api/core";
import { buildAnalysisSeriesGroups, normalizedPath } from "../organizer-analysis.js";
import type { WorkbenchOrganizerState } from "./workbench-organizer-state.svelte.js";
import type { WorkbenchPropsView } from "./workbench-props.js";

export class WorkbenchDerived {
  desktop = $derived.by(() => (isTauri()));
  analysisSeriesGroups = $derived.by(() => (this.p.scanResult ? buildAnalysisSeriesGroups(this.p.scanResult) : []));
  visibleAnalysisSeriesGroups = $derived.by(() => {
    const focus = this.p.focusMediaPath;
    return this.p.compactItemMode && focus
      ? this.analysisSeriesGroups.filter((group) => group.files.some((file) => normalizedPath(file.path) === normalizedPath(focus)))
      : this.analysisSeriesGroups;
  });
  visibleOrganizerFiles = $derived.by(() => {
    const focus = this.p.focusMediaPath;
    return this.p.compactItemMode && focus && this.organizerState.organizerScan
      ? this.organizerState.organizerScan.files.filter((file) => normalizedPath(file.path) === normalizedPath(focus))
      : this.organizerState.organizerScan?.files ?? [];
  });

  constructor(private readonly p: WorkbenchPropsView, private readonly organizerState: WorkbenchOrganizerState) {}
}
