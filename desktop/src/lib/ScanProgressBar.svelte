<!-- ./desktop/src/lib/ScanProgressBar.svelte
     Shared read-only scan progress with a cooperative stop control.
     Cancel asks the native scanner to finish the current file, then return. -->
<script lang="ts">
  import { LoaderCircle } from "@lucide/svelte";
  import type { ScanProgress } from "./types.js";

  interface Props {
    scanProgress: ScanProgress | null;
    onCancel?: () => void;
  }

  let { scanProgress, onCancel }: Props = $props();
</script>

<div class="scan-progress card-surface">
  <LoaderCircle class="spin" size={19} />
  <div>
    <strong>{scanProgress ? `Reading ${scanProgress.current} of ${scanProgress.total}` : "Preparing the analysis…"}</strong>
    <span>{scanProgress?.file ?? "Finding media files and checking stream metadata"}</span>
  </div>
  <div class="progress-track"><div style={`width:${scanProgress?.total ? Math.max(4, scanProgress.current / scanProgress.total * 100) : 9}%`}></div></div>
  {#if onCancel}
    <button class="text-button scan-cancel" type="button" onclick={onCancel}>Stop scan</button>
  {/if}
</div>
