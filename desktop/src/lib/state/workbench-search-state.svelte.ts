// ./desktop/src/lib/state/workbench-search-state.svelte.ts
// Metadata workbench title search: provider, media kind, query, results, warnings, the open search target and the selected result with its detail.
// A plain rune class: the workbench component creates one instance, reads and writes its fields directly and
// passes the instance to the controllers extracted from the component as their typed host.
import type { Provider, MediaKind, Result, SearchTarget, Detail, FilenameClue } from "../metadata-workbench-types.js";

export class WorkbenchSearchState {
  provider = $state<Provider>("wikidata");
  mediaKind = $state<MediaKind>("movie");
  query = $state("");
  searching = $state(false);
  loadingDetail = $state(false);
  error = $state("");
  reviewDetailError = $state("");
  results = $state<Result[]>([]);
  searchWarnings = $state<string[]>([]);
  searchTarget = $state<SearchTarget | null>(null);
  titleSearchInput = $state<HTMLInputElement | null>(null);
  selected = $state<Result | null>(null);
  detail = $state<Detail | null>(null);
  manualSearchClue = $state<FilenameClue | null>(null);
  expandedMatchLists = $state<Record<string, boolean>>({});
}
