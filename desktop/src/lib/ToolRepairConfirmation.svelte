<script lang="ts">
  import { CheckCircle2, KeyRound, Wrench, X } from '@lucide/svelte';
  import Modal from './Modal.svelte';
  import type { ProvisionPlan } from './types.js';
  interface Props { open: boolean; plan: ProvisionPlan | null; workflowBusy: boolean; confirmSetup: () => unknown; onClose: () => void }
  let { open, plan, workflowBusy, confirmSetup, onClose }: Props = $props();
  const helperLine = $derived(!plan ? "" : plan.helper.action === "install" ? "Install the HDR10+ helper (hdr10plus_tool)" : plan.helper.action === "update" ? `Update the HDR10+ helper ${plan.helper.installed} → ${plan.helper.latest}` : "");
</script>
<!-- Shared confirm-modal/repair-plan selectors in the app stylesheets (src/styles/app) preserve plan review and theme overrides. -->
{#if open && plan}
  <Modal {open} class="confirm-modal" labelId="repair-title" onClose={onClose}>
      <div class="confirm-symbol repair-symbol"><Wrench size={21} /></div><button class="icon-button modal-close" aria-label="Close confirmation" onclick={onClose}><X size={18} /></button>
      <div class="eyebrow">SYSTEM TOOL SETUP</div><h2 id="repair-title">Install and update everything?</h2>
      <p>One pass sets up every tool HEVC Rebuild needs and brings it up to date. {#if plan.needs_password}You will be asked for your password <strong>once</strong>.{:else}No password is needed.{/if}</p>
      {#if plan.refresh_catalog || plan.install_packages.length || plan.upgrade_packages.length}
        <div class="repair-plan-group"><strong>System packages{plan.needs_password ? " (one password prompt)" : ""}</strong>
          {#if plan.refresh_catalog}<small>Refresh the Ubuntu package catalog</small>{/if}
          {#if plan.install_packages.length}<small>Install missing</small><div class="repair-package-list">{#each plan.install_packages as pkg (pkg)}<code>{pkg}</code>{/each}</div>{/if}
          {#if plan.upgrade_packages.length}<small>Update from your enabled Ubuntu sources</small><div class="repair-package-list">{#each plan.upgrade_packages as pkg (pkg)}<code>{pkg}</code>{/each}</div>{/if}
        </div>
      {/if}
      {#if helperLine}<div class="repair-helper-note"><CheckCircle2 size={16} /><span><strong>{helperLine}</strong> from its verified upstream release. No password is needed.</span></div>{/if}
      {#if plan.release_check_error}<small class="repair-plan-footnote">The HDR10+ helper release could not be checked ({plan.release_check_error}). Installed copies are left unchanged.</small>{/if}
      <small class="repair-plan-footnote">Only the application’s fixed package list is touched, and nothing is ever removed. Custom binary overrides are left untouched.</small>
      <div class="modal-actions"><button class="button secondary" onclick={onClose}>Cancel</button><button class="button primary" disabled={workflowBusy} onclick={confirmSetup}>{#if plan.needs_password}<KeyRound size={16} /> Authenticate once & install{:else}<Wrench size={16} /> Install & update{/if}</button></div>
  </Modal>
{/if}
