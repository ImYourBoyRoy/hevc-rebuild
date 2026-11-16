<!-- ./desktop/src/lib/views/ShellHeader.svelte
     Breadcrumb, appearance picker, engine status, refresh/help buttons and the
     dismissible notice. Pure view; every action is a route callback. -->
<script lang="ts">
  import { AlertTriangle, CheckCircle2, ChevronRight, HelpCircle, Info, LoaderCircle, Moon, RefreshCw, Sun, Terminal, X } from "@lucide/svelte";
  import type { Environment, Page } from "../types.js";
  import type { Theme } from "../ui-contracts.js";
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
  }
  let { page, theme, environment, environmentLoading, booting, setupGapDetail, setupGapLabel, notice, applyTheme, openPage, openToolchainSettings, loadEnvironment, dismissNotice }: Props = $props();
</script>

<header class="topbar">
  <div class="breadcrumbs"><span>HEVC REBUILD</span><ChevronRight size={14} /><strong>{pageLabel(page)}</strong></div>
  <div class="top-actions">
    <label class="theme-control" title="Choose the application appearance">
      {#if theme === "light"}<Sun size={15} aria-hidden="true" />{:else if theme === "dark"}<Moon size={15} aria-hidden="true" />{:else}<Terminal size={15} aria-hidden="true" />{/if}
      <span class="appearance-label">Appearance</span>
      <select aria-label="Appearance" value={theme} onchange={(event) => applyTheme(event.currentTarget.value)}>
        <option value="light">Light</option>
        <option value="dark">Dark</option>
        <option value="phosphor">Phosphor</option>
        <option value="ion">Ion</option>
        <option value="breach">Breach</option>
      </select>
    </label>
    {#if environment?.ready}
      <span class="status-pill good"><span class="status-dot"></span> SYSTEM READY</span>
    {:else if environmentLoading || booting}
      <span class="status-pill neutral"><LoaderCircle size={14} class="spin" /> CHECKING</span>
    {:else}
      <button class="status-pill warning" aria-label={setupGapDetail} title={setupGapDetail} onclick={openToolchainSettings}><AlertTriangle size={14} /><span>{setupGapLabel}</span></button>
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
