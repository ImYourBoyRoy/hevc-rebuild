<script lang="ts">
  import { AlertTriangle, CheckCircle2, KeyRound, Wrench, X } from '@lucide/svelte';
  import Modal from './Modal.svelte';
  import type { ProvisionPlan } from './types.js';
  interface Props { open: boolean; plan: ProvisionPlan | null; workflowBusy: boolean; confirmSetup: () => unknown; onClose: () => void }
  let { open, plan, workflowBusy, confirmSetup, onClose }: Props = $props();
  const helperLine = $derived(!plan ? "" : plan.helper.action === "install" ? "Install the HDR10+ helper (hdr10plus_tool)" : plan.helper.action === "update" ? `Update the HDR10+ helper ${plan.helper.installed} → ${plan.helper.latest}` : "");
  const hasPackageWork = $derived(Boolean(plan && (plan.refresh_catalog || plan.install_packages.length || plan.upgrade_packages.length)));
  const canApply = $derived(hasPackageWork || Boolean(helperLine));
  const managerName = $derived(!plan ? "" : plan.manager === "brew" ? "Homebrew" : plan.manager === "winget" ? "Windows Package Manager" : "Ubuntu");
  const approval = $derived(!plan ? "" : plan.manager === "winget" ? "Windows will ask you to approve once." : "You will be asked for your password");
</script>
<!-- Shared confirm-modal/repair-plan selectors in the app stylesheets (src/styles/app) preserve plan review and theme overrides. -->
{#if open && plan}
  <Modal {open} class="confirm-modal" labelId="repair-title" onClose={onClose}>
      <div class="confirm-symbol repair-symbol"><Wrench size={21} /></div><button class="icon-button modal-close" aria-label="Close confirmation" onclick={onClose}><X size={18} /></button>
      <div class="eyebrow">SYSTEM TOOL SETUP</div><h2 id="repair-title">{plan.blockers.length ? "A few things are needed first" : "Install and update everything?"}</h2>
      {#if plan.blockers.length}
        <p>{managerName} cannot install the media tools until these are in place.</p>
        {#each plan.blockers as blocker (blocker.key)}
          <div class="repair-plan-group"><strong>{blocker.label}</strong><small>{blocker.remedy}</small>{#if blocker.command}<code class="repair-command">{blocker.command}</code>{/if}</div>
        {/each}
      {:else}
        <p>One pass sets up every tool HEVC Rebuild needs and brings it up to date. {#if plan.needs_password}{approval}{#if plan.manager !== "winget"}{" "}<strong>once</strong>{/if}.{:else}No password is needed.{/if}</p>
      {/if}
      {#if hasPackageWork}
        <div class="repair-plan-group"><strong>System packages ({managerName}){plan.needs_password ? " · one approval" : ""}</strong>
          {#if plan.refresh_catalog}<small>Refresh the package list</small>{/if}
          {#if plan.install_packages.length}<small>Install missing</small><div class="repair-package-list">{#each plan.install_packages as pkg (pkg)}<code>{pkg}</code>{/each}</div>{/if}
          {#if plan.upgrade_packages.length}<small>Update</small><div class="repair-package-list">{#each plan.upgrade_packages as pkg (pkg)}<code>{pkg}</code>{/each}</div>{/if}
        </div>
      {/if}
      {#if helperLine}<div class="repair-helper-note"><CheckCircle2 size={16} /><span><strong>{helperLine}</strong> from its verified upstream release. No password is needed.</span></div>{/if}
      {#each plan.advisories as advisory (advisory)}<div class="encoder-advisory" data-level="warning" role="note"><AlertTriangle size={16} aria-hidden="true" /><span>{advisory}</span></div>{/each}
      {#if plan.release_check_error}<small class="repair-plan-footnote">The HDR10+ helper release could not be checked ({plan.release_check_error}). Installed copies are left unchanged.</small>{/if}
      <small class="repair-plan-footnote">Only the application’s fixed package list is touched, and nothing is ever removed. Custom binary overrides are left untouched.</small>
      <div class="modal-actions"><button class="button secondary" onclick={onClose}>{canApply ? "Cancel" : "Close"}</button>{#if canApply}<button class="button primary" disabled={workflowBusy} onclick={confirmSetup}>{#if plan.needs_password}<KeyRound size={16} /> {plan.manager === "winget" ? "Approve once & install" : "Authenticate once & install"}{:else}<Wrench size={16} /> Install & update{/if}</button>{/if}</div>
  </Modal>
{/if}
