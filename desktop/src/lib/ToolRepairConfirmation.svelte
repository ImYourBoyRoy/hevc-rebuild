<script lang="ts">
  import { ArrowRight, CheckCircle2, Wrench, X } from '@lucide/svelte';
  import Modal from './Modal.svelte';
  import type { Environment, AptUpdateStatus } from './types.js';
  interface Props { repairConfirmOpen: boolean; environment: Environment | null; aptUpdates: AptUpdateStatus | null; workflowBusy: boolean; confirmAptRepair: () => unknown; onClose: () => void }
  let { repairConfirmOpen, environment, aptUpdates, workflowBusy, confirmAptRepair, onClose }: Props = $props();
</script>
<!-- Shared confirm-modal/repair-plan selectors in app.css preserve exact-plan review and theme overrides. -->
{#if repairConfirmOpen && environment && aptUpdates}
  <Modal open={repairConfirmOpen} class="confirm-modal" labelId="repair-title" onClose={onClose}>
      <div class="confirm-symbol repair-symbol"><Wrench size={21} /></div><button class="icon-button modal-close" aria-label="Close confirmation" onclick={onClose}><X size={18} /></button>
      <div class="eyebrow">SYSTEM TOOL SETUP</div><h2 id="repair-title">Install or update these tools?</h2><p>The app will install missing packages, then update only the listed packages from your enabled Ubuntu sources. Each APT plan is simulated first and stops if it would remove packages or change unlisted updates. Ubuntu will request administrator authentication.</p>
      {#if environment.missing_apt_packages.length}<div class="repair-plan-group"><strong>Install missing</strong><div class="repair-package-list">{#each environment.missing_apt_packages as pkg (pkg)}<code>{pkg}</code>{/each}</div></div>{/if}
      {#if aptUpdates.packages.length}<div class="repair-plan-group"><strong>Update from {aptUpdates.source}</strong><div class="apt-update-list modal-apt-update-list">{#each aptUpdates.packages as update (update.name)}<div><code>{update.name}</code><span>{update.installed_version}</span><ArrowRight size={13} /><strong>{update.candidate_version}</strong></div>{/each}</div></div>{/if}
      {#if environment.checks.some((check) => check.key === "hdr10plus_tool" && !check.available)}<div class="repair-helper-note"><CheckCircle2 size={16} /><span><strong>HDR10+ helper</strong> will also be installed from its verified upstream release after the package setup succeeds.</span></div>{/if}
      <small class="repair-plan-footnote">Ubuntu package versions come from the enabled APT repositories and may lag upstream releases. Custom binary overrides are left untouched.</small>
      <div class="modal-actions"><button class="button secondary" onclick={onClose}>Cancel</button><button class="button primary" disabled={workflowBusy || aptUpdates.catalog_stale} onclick={confirmAptRepair}><Wrench size={16} /> Continue to authentication</button></div>
  </Modal>
{/if}