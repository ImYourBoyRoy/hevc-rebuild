<!-- ./desktop/src/lib/views/StopNowDialog.svelte
     Confirmation for stopping the active encode immediately. Pure view; the
     route owns the open flag and the stop action. -->
<script lang="ts">
  import { OctagonX, X } from "@lucide/svelte";
  import Modal from "../Modal.svelte";
  import type { createWorkspaceState } from "../workspace.svelte.js";

  interface Props {
    open: boolean;
    workspace: ReturnType<typeof createWorkspaceState>;
    onClose: () => void;
    onConfirm: () => unknown;
  }
  let { open, workspace, onClose, onConfirm }: Props = $props();
</script>

<Modal open={open} class="confirm-modal stop-now-modal" labelId="stop-now-title" descriptionId="stop-now-copy" closeOnBackdrop={true} onClose={onClose}>
  <button class="icon-button modal-close" aria-label="Cancel stop now" onclick={onClose}><X size={18} /></button>
  <div class="confirm-symbol stop-now-symbol"><OctagonX size={22} /></div>
  <div class="eyebrow">STOP NOW</div>
  <h2 id="stop-now-title">Stop this encode immediately?</h2>
  <p id="stop-now-copy">The original file stays where it is. This file is marked interrupted and is not resumed. A partial output is discarded or kept as an unvalidated <code>.error.mkv</code> for review. Use <strong>Stop after this file</strong> if you want the current encode to finish cleanly.</p>
  <div class="modal-actions">
    <button class="button secondary" type="button" onclick={onClose}>Keep running</button>
    <button class="button danger" type="button" disabled={!workspace.run.id || workspace.run.aborting} onclick={onConfirm}>Stop now</button>
  </div>
</Modal>
