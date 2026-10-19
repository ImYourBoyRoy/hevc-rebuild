<script lang="ts">
  import { AlertTriangle, Info, LoaderCircle } from '@lucide/svelte';
  import type { QueueState } from './workspace-state.js';
  import type { ScanProgress } from './types.js';
  import '../styles/components/library-poster-grid.css';
  interface Props {
    queue: QueueState; busy: boolean; scanning: boolean; scanProgress: ScanProgress | null;
    fileName: (path: string) => string;
    onActivate: (path: string) => void; onRefresh: (path: string) => unknown; onRemove: (path: string) => void;
  }
  let { queue, busy, scanning, scanProgress, fileName, onActivate, onRefresh, onRemove }: Props = $props();
</script>
<!-- Library chrome retains shared app.css scan/token rules and explicitly imports source-strip CSS. -->
        {#if queue.sources.length}
          <section class="library-source-strip" aria-label="Queued media sources">
            {#if queue.persistenceWarning}<div class="queue-persistence-warning"><Info size={15} /><span>{queue.persistenceWarning}</span></div>{/if}
            {#if queue.persistenceError}<div class="queue-persistence-warning error"><AlertTriangle size={15} /><span>Queue changes could not be saved: {queue.persistenceError}</span></div>{/if}
            {#each queue.sources as source (source.root)}
              <div class="library-source-chip" class:active={source.root === queue.activeRoot}>
                <button class="library-source-name" type="button" title={source.root} aria-current={source.root === queue.activeRoot ? "true" : undefined} onclick={() => onActivate(source.root)}>{fileName(source.root)} · {source.scanResult?.candidate_count ?? 0}</button>
                <button class="text-button" type="button" disabled={busy} onclick={() => void onRefresh(source.root)}>Refresh</button>
                <button class="text-button" type="button" disabled={busy} aria-label={`Remove ${fileName(source.root)} from queue`} onclick={() => onRemove(source.root)}>Remove</button>
              </div>
            {/each}
          </section>
        {/if}

        {#if scanning}
          <div class="scan-progress card-surface"><LoaderCircle class="spin" size={19} /><div><strong>{scanProgress ? `Reading ${scanProgress.current} of ${scanProgress.total}` : "Preparing the analysis…"}</strong><span>{scanProgress?.file ?? "Finding media files and checking stream metadata"}</span></div><div class="progress-track"><div style={`width:${scanProgress?.total ? Math.max(4, scanProgress.current / scanProgress.total * 100) : 9}%`}></div></div></div>
        {/if}

