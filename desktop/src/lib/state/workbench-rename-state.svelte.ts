// ./desktop/src/lib/state/workbench-rename-state.svelte.ts
// Metadata workbench rename: the preview, error and message, the busy flag and the confirmation step.
// A plain rune class: the workbench component creates one instance, reads and writes its fields directly and
// passes the instance to the controllers extracted from the component as their typed host.
import type { RenamePreview } from "../metadata-workbench-types.js";

export class WorkbenchRenameState {
  renamePreview = $state<RenamePreview | null>(null);
  renameError = $state("");
  renameMessage = $state("");
  renameBusy = $state(false);
  confirmRename = $state(false);
}
