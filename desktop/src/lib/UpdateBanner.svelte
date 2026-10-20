<script lang="ts">
  import { onMount } from "svelte";
  import { Download, Info, LoaderCircle, X } from "@lucide/svelte";
  import type { Update } from "@tauri-apps/plugin-updater";
  import {
    checkForAppUpdate,
    formatUpdateNotes,
    formatVersionLabel,
    getDismissedUpdateVersion,
    installAppUpdate,
    isTauri,
    setDismissedUpdateVersion,
    shouldShowUpdateBanner,
    updaterErrorMessage,
    type UpdateUiState,
  } from "./updater.js";

  type Props = { blocked?: boolean };
  let { blocked = false }: Props = $props();

  let desktopShell = $state(false);
  let updateState = $state<UpdateUiState>("idle");
  let update = $state<Update | undefined>();
  let message = $state("");
  let error = $state("");
  let notes = $state<string | undefined>();
  let progress = $state(0);
  let showBanner = $state(false);
  let restartRequired = $state(false);

  async function checkForUpdates(quiet = true): Promise<void> {
    if (!desktopShell) return;
    updateState = "checking";
    if (!quiet) error = "";
    const previous = update;
    const result = await checkForAppUpdate({ quiet, previous });
    if (result.state === "error") {
      // Keep an already discovered update visible when a later background
      // check is offline or temporarily unavailable.
      updateState = previous ? "available" : "error";
      if (!quiet) {
        error = result.message;
        message = "The update check could not complete.";
        showBanner = Boolean(previous);
      }
      return;
    }
    updateState = result.state;
    error = "";
    message = result.message;
    update = result.update;
    notes = formatUpdateNotes(update?.body);
    if (result.state === "available" && update) {
      showBanner = shouldShowUpdateBanner(update.version, getDismissedUpdateVersion());
    } else {
      showBanner = false;
    }
  }

  function dismiss(): void {
    setDismissedUpdateVersion(update?.version);
    showBanner = false;
  }

  async function install(): Promise<void> {
    if (!update || blocked || updateState === "installing") return;
    if (!globalThis.confirm("Download and install this signed update, then restart HEVC Rebuild Studio?")) return;
    updateState = "installing";
    error = "";
    progress = 0;
    try {
      const result = await installAppUpdate(update, {
        confirmed: true,
        onProgress: (value) => (progress = value),
      });
      restartRequired = !result.relaunched;
      message = result.relaunched
        ? "Restarting HEVC Rebuild Studio…"
        : "The update installed. Close and reopen HEVC Rebuild Studio to finish.";
      updateState = "available";
    } catch (installError) {
      error = updaterErrorMessage(installError);
      message = "The update could not be installed.";
      updateState = "error";
      showBanner = true;
    }
  }

  onMount(() => {
    desktopShell = isTauri();
    if (!desktopShell) return;

    const initialCheck = window.setTimeout(() => void checkForUpdates(), 5_000);
    const periodicCheck = window.setInterval(() => void checkForUpdates(), 24 * 60 * 60 * 1_000);
    return () => {
      window.clearTimeout(initialCheck);
      window.clearInterval(periodicCheck);
      if (update && updateState !== "installing") void update.close().catch(() => undefined);
    };
  });
</script>

{#if showBanner && update}
  <div class="notice update-notice" role="region" aria-label={`Update ${formatVersionLabel(update.version)} available`} aria-live="polite">
    <Info size={17} aria-hidden="true" />
    <div class="update-notice-copy">
      <strong>{restartRequired ? "Update installed" : `Update available: ${formatVersionLabel(update.version)}`}</strong>
      <span>{blocked && !restartRequired ? "Finish the current media operation before installing." : message}</span>
      {#if notes}
        <details>
          <summary>Release notes</summary>
          <p>{notes}</p>
        </details>
      {/if}
      {#if error}<span class="update-notice-error">{error}</span>{/if}
    </div>
    <div class="update-notice-actions">
      {#if updateState === "installing"}
        <span class="update-progress"><LoaderCircle size={15} class="spin" /> {progress ? `${progress}%` : "Downloading…"}</span>
      {:else if !restartRequired}
        <button type="button" class="button secondary" onclick={dismiss}>Later</button>
        <button type="button" class="button primary" disabled={blocked} onclick={install}>
          {#if updateState === "error"}<X size={15} /> Try again{:else}<Download size={15} /> Install & restart{/if}
        </button>
      {:else}
        <button type="button" class="button secondary" onclick={dismiss}>Dismiss</button>
      {/if}
    </div>
  </div>
{/if}
