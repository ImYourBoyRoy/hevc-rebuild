// ./desktop/src/lib/state/library-ui-state.svelte.ts
// Library page state: search and paging, browse position, drop highlight, local posters, run logs and the rename dialog.
// A plain rune class: its instance is the typed host the controller works on, so the route needs no
// per-field accessors. Members marked `declare` are supplied by the route (see withGetters).
import type { OrganizerTemplates } from "../types.js";
import type { LocalLibraryRecord } from "../review-notes.ts";
import { defaultOrganizerTemplates } from "./defaults.ts";

export class LibraryUiState {
  search = $state("");
  mediaPage = $state(1);
  mediaPageSize = $state(50);
  visibleSelectAllCheckbox = $state<HTMLInputElement | null>(null);
  libraryBrowse = $state<{ seriesRoot: string; season: number | null } | null>(null);
  libraryDropActive = $state(false);
  libraryLocalPosters = $state<Record<string, string>>({});
  libraryLocalRecords = $state<Record<string, LocalLibraryRecord>>({});
  queueRunLogs = $state<Record<string, { loading: boolean; text: string; error: string }>>({});
  libraryRenameDialog = $state<HTMLDialogElement | null>(null);
  libraryRenameOpen = $state(false);
  renameDraft = $state<OrganizerTemplates>({ ...defaultOrganizerTemplates });
  moviePatternField = $state<HTMLInputElement | null>(null);
  episodePatternField = $state<HTMLInputElement | null>(null);
  libraryRenameBusy = $state(false);
  libraryRenameError = $state("");
}
