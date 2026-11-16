// ./desktop/src/lib/state/workbench-lookup-state.svelte.ts
// Metadata workbench automatic match lookup: saved movie and series suggestions, open editors and the lookup progress and message.
// A plain rune class: the workbench component creates one instance, reads and writes its fields directly and
// passes the instance to the controllers extracted from the component as their typed host.
import type { MatchSuggestion } from "../metadata-workbench-types.js";

export class WorkbenchLookupState {
  movieSuggestions = $state<Record<string, MatchSuggestion>>({});
  seriesSuggestions = $state<Record<string, MatchSuggestion>>({});
  movieEditorOpen = $state<Record<string, boolean>>({});
  lookupBusy = $state(false);
  lookupProgress = $state<{ done: number; total: number; label: string } | null>(null);
  lookupMessage = $state("");
  lookupMessageScope = $state<"analyzed films" | "series folders" | null>(null);
  // Not reactive: read by the lookup loop, never by the template.
  cancelLookupRequested = false;
  lastLookupStopped = false;
}
