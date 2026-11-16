// ./desktop/src/lib/state/workbench-artwork-save-state.svelte.ts
// Metadata workbench poster saving: the pending provider poster save, the pending local and batch poster copies, and their busy, error and message fields.
// A plain rune class: the workbench component creates one instance, reads and writes its fields directly and
// passes the instance to the controllers extracted from the component as their typed host.
import type { PendingArtworkSave, PendingLocalPoster } from "../metadata-workbench-types.js";

export class WorkbenchArtworkSaveState {
  pendingArtworkSave = $state<PendingArtworkSave | null>(null);
  artworkBusy = $state(false);
  artworkError = $state("");
  artworkMessage = $state("");
  pendingLocalPoster = $state<PendingLocalPoster | null>(null);
  pendingBatchLocalPoster = $state<{ sourcePath: string; artworkPath: string; mediaPaths: string[] } | null>(null);
  localPosterBusy = $state(false);
  localPosterError = $state("");
  localPosterMessage = $state("");
}
