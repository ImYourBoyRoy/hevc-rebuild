// ./desktop/src/lib/state/workbench-organizer-state.svelte.ts
// Metadata workbench organizer: the scanned folder or analysis, its source, the chosen series root, rename scope, episode assignments and selected artwork.
// A plain rune class: the workbench component creates one instance, reads and writes its fields directly and
// passes the instance to the controllers extracted from the component as their typed host.
import type { OrganizerScan, TvRenameScope } from "../metadata-workbench-types.js";

export class WorkbenchOrganizerState {
  organizerScan = $state<OrganizerScan | null>(null);
  organizerSource = $state<"analysis" | "folder" | null>(null);
  selectedAnalysisSeriesRoot = $state("");
  tvRenameScope = $state<TvRenameScope>("series-tree");
  seriesYearConflictConfirmed = $state(false);
  episodeAssignments = $state<Record<string, string>>({});
  selectedArtwork = $state<Record<string, string>>({});
}
