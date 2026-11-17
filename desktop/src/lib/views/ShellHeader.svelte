<!-- ./desktop/src/lib/views/ShellHeader.svelte
     Breadcrumb, appearance picker, engine status, refresh/help buttons and the
     dismissible notice. Pure view; every action is a route callback. -->
<script lang="ts">
  import { AlertTriangle, CheckCircle2, ChevronRight, HelpCircle, Info, LoaderCircle, Monitor, Moon, RefreshCw, Sun, Terminal, X } from "@lucide/svelte";
  import type { Environment, Page } from "../types.js";
  import type { Theme } from "../ui-contracts.js";
  import { themeOptions, themes } from "../themes.js";
  import type { createWorkspaceState } from "../workspace.svelte.js";
  import { pageLabel } from "../normalizers.js";

  interface Props {
    page: Page;
    theme: Theme;
    environment: Environment | null;
    environmentLoading: boolean;
    booting: boolean;
    setupGapDetail: string;
    setupGapLabel: string;
    notice: ReturnType<typeof createWorkspaceState>["notice"];
    applyTheme: (theme: string) => unknown;
    openPage: (page: Page) => void;
    openToolchainSettings: () => unknown;
    loadEnvironment: () => unknown;
    dismissNotice: () => void;
    /** Header notice for missing or outdated tools; empty when everything is current. */
    updateLabel?: string;
    updateDetail?: string;
    reviewUpdates?: () => unknown;
  }
  let { page, theme, environment, environmentLoading, booting, setupGapDetail, setupGapLabel, notice, applyTheme, openPage, openToolchainSettings, loadEnvironment, dismissNotice, updateLabel = "", updateDetail = "", reviewUpdates = () => {} }: Props = $props();
  const current = $derived(themes.find((item) => item.id === theme));
</script>

<header class="topbar">
  <div class="breadcrumbs"><span>HEVC REBUILD</span><ChevronRight size={14} /><strong>{pageLabel(page)}</strong></div>
  <div class="top-actions">
    <label class="theme-control" title="Choose the application appearance">
      {#if theme === "dos"}<Monitor size={15} aria-hidden="true" />{:else if current?.family === "instrument"}<Terminal size={15} aria-hidden="true" />{:else if current?.scheme === "dark"}<Moon size={15} aria-hidden="true" />{:else}<Sun size={15} aria-hidden="true" />{/if}
      <span class="appearance-label">Appearance</span>
      <select aria-label="Appearance" value={theme} onchange={(event) => applyTheme(event.currentTarget.value)}>
        {#each themeOptions as option (option.value)}<option value={option.value}>{option.label}</option>{/each}
      </select>
    </label>
    {#if environment?.ready}
      <span class="status-pill good"><span class="status-dot"></span> SYSTEM READY</span>
    {:else if environmentLoading || booting}
      <span class="status-pill neutral"><LoaderCircle size={14} class="spin" /> CHECKING</span>
    {:else}
      <button class="status-pill warning" aria-label={setupGapDetail} title={setupGapDetail} onclick={openToolchainSettings}><AlertTriangle size={14} /><span>{setupGapLabel}</span></button>
    {/if}
    {#if updateLabel}
      <button class="status-pill warning" aria-label={updateDetail} title={updateDetail} onclick={reviewUpdates}><AlertTriangle size={14} /><span>{updateLabel}</span></button>
    {/if}
    <button class="icon-button" aria-label="Refresh environment" title="Refresh environment" disabled={environmentLoading} onclick={loadEnvironment}><RefreshCw size={17} class={environmentLoading ? "spin" : ""} /></button>
    <button class="icon-button" class:active-help={page === "help"} aria-label="Help and commands" title="Help and commands" aria-current={page === "help" ? "page" : undefined} onclick={() => openPage("help")}><HelpCircle size={17} /></button>
  </div>
</header>

{#if notice.message}
  <div class="notice" class:error={notice.kind === "error"} class:success={notice.kind === "success"} role="status">
    {#if notice.kind === "error"}<AlertTriangle size={16} />{:else if notice.kind === "success"}<CheckCircle2 size={16} />{:else}<Info size={16} />{/if}
    <span>{notice.message}</span><button class="notice-close" aria-label="Dismiss notification" onclick={dismissNotice}><X size={15} /></button>
  </div>
{/if}
