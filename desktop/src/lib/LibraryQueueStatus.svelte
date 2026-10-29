<!-- ./desktop/src/lib/LibraryQueueStatus.svelte
     Persistent library queue manager: add/remove sources and files, drag or
     keyboard reorder, and cancel an in-flight folder scan. -->
<script lang="ts">
  import { AlertTriangle, ChevronDown, ChevronUp, FolderOpen, GripVertical, Info, Video } from "@lucide/svelte";
  import ScanProgressBar from "./ScanProgressBar.svelte";
  import type { QueueState } from "./workspace-state.js";
  import type { ScanProgress } from "./types.js";
  import "../styles/components/library-poster-grid.css";

  interface Props {
    queue: QueueState;
    busy: boolean;
    scanning: boolean;
    scanProgress: ScanProgress | null;
    fileName: (path: string) => string;
    onActivate: (path: string) => void;
    onRefresh: (path: string) => unknown;
    onRemove: (path: string) => void;
    onCancelScan?: () => unknown;
    onAddFolder?: () => unknown;
    onAddFiles?: () => unknown;
    onReorderSources?: (fromRoot: string, toRoot: string) => void;
    onMoveSource?: (root: string, delta: number) => void;
    onRemoveFile?: (path: string) => void;
    onReorderFiles?: (root: string, fromPath: string, toPath: string) => void;
    onMoveFile?: (root: string, path: string, delta: number) => void;
    onRestoreExcluded?: (root: string) => void;
  }

  let {
    queue,
    busy,
    scanning,
    scanProgress,
    fileName,
    onActivate,
    onRefresh,
    onRemove,
    onCancelScan,
    onAddFolder,
    onAddFiles,
    onReorderSources,
    onMoveSource,
    onRemoveFile,
    onReorderFiles,
    onMoveFile,
    onRestoreExcluded,
  }: Props = $props();

  let draggingSource = $state("");
  let dropSource = $state("");
  let draggingFile = $state("");
  let dropFile = $state("");

  let activeSource = $derived(queue.sources.find((source) => source.root === queue.activeRoot) ?? null);
  let activeFiles = $derived(activeSource?.scanResult?.entries ?? []);
  let hiddenCount = $derived(activeSource?.excludedPaths.length ?? 0);
  let sourceLocked = $derived(busy && !scanning);

  function sourcePayload(root: string): string {
    return `hevc-source:${root}`;
  }

  function filePayload(root: string, path: string): string {
    return `hevc-file:${root}\t${path}`;
  }

  function readDrag(event: DragEvent): string {
    return event.dataTransfer?.getData("text/plain") ?? "";
  }

  function beginSourceDrag(event: DragEvent, root: string) {
    if (sourceLocked || !onReorderSources) return;
    event.dataTransfer?.setData("text/plain", sourcePayload(root));
    if (event.dataTransfer) event.dataTransfer.effectAllowed = "move";
    draggingSource = root;
  }

  function beginFileDrag(event: DragEvent, root: string, path: string) {
    if (sourceLocked || !onReorderFiles) return;
    event.dataTransfer?.setData("text/plain", filePayload(root, path));
    if (event.dataTransfer) event.dataTransfer.effectAllowed = "move";
    draggingFile = path;
  }

  function clearDrag() {
    draggingSource = "";
    dropSource = "";
    draggingFile = "";
    dropFile = "";
  }

  function dropOnSource(event: DragEvent, root: string) {
    event.preventDefault();
    const payload = readDrag(event);
    if (payload.startsWith("hevc-source:") && onReorderSources) {
      onReorderSources(payload.slice("hevc-source:".length), root);
    }
    clearDrag();
  }

  function dropOnFile(event: DragEvent, root: string, path: string) {
    event.preventDefault();
    const payload = readDrag(event);
    const prefix = `hevc-file:${root}\t`;
    if (payload.startsWith(prefix) && onReorderFiles) {
      onReorderFiles(root, payload.slice(prefix.length), path);
    }
    clearDrag();
  }
</script>

{#if queue.sources.length || scanning}
  <section class="library-queue-manager" aria-label="Media queue">
    <div class="library-queue-heading">
      <div>
        <div class="eyebrow">QUEUE</div>
        <h2>Sources</h2>
      </div>
      <div class="library-queue-heading-actions">
        {#if onAddFolder}<button class="button secondary" type="button" disabled={busy} onclick={onAddFolder}><FolderOpen size={15} /> Add folder</button>{/if}
        {#if onAddFiles}<button class="button secondary" type="button" disabled={busy} onclick={onAddFiles}><Video size={15} /> Add files</button>{/if}
      </div>
    </div>

    {#if queue.persistenceWarning}<div class="queue-persistence-warning"><Info size={15} /><span>{queue.persistenceWarning}</span></div>{/if}
    {#if queue.persistenceError}<div class="queue-persistence-warning error"><AlertTriangle size={15} /><span>Queue changes could not be saved: {queue.persistenceError}</span></div>{/if}

    {#if queue.sources.length}
      <div class="library-source-strip" role="list" aria-label="Queued media sources">
        {#each queue.sources as source, index (source.root)}
          <div
            class={["library-source-chip", source.root === queue.activeRoot && "active", draggingSource === source.root && "dragging", dropSource === source.root && "drag-over"]}
            role="listitem"
            ondragover={(event) => { if (!onReorderSources || sourceLocked) return; event.preventDefault(); dropSource = source.root; }}
            ondragleave={() => { if (dropSource === source.root) dropSource = ""; }}
            ondrop={(event) => dropOnSource(event, source.root)}
          >
            {#if onReorderSources}
              <button class="queue-drag-handle" type="button" draggable="true" aria-label={`Drag ${fileName(source.root)} to reorder`} disabled={sourceLocked} ondragstart={(event) => beginSourceDrag(event, source.root)} ondragend={clearDrag}><GripVertical size={14} /></button>
              <button class="icon-button queue-move" type="button" aria-label={`Move ${fileName(source.root)} earlier`} disabled={sourceLocked || index === 0} onclick={() => onMoveSource?.(source.root, -1)}><ChevronUp size={14} /></button>
              <button class="icon-button queue-move" type="button" aria-label={`Move ${fileName(source.root)} later`} disabled={sourceLocked || index === queue.sources.length - 1} onclick={() => onMoveSource?.(source.root, 1)}><ChevronDown size={14} /></button>
            {/if}
            <button class="library-source-name" type="button" title={source.root} aria-current={source.root === queue.activeRoot ? "true" : undefined} onclick={() => onActivate(source.root)}>{fileName(source.root)} · {source.scanResult?.candidate_count ?? 0}</button>
            <button class="text-button" type="button" disabled={busy} onclick={() => void onRefresh(source.root)}>Refresh</button>
            {#if (source.excludedPaths?.length ?? 0) > 0 && onRestoreExcluded}
              <button class="text-button" type="button" disabled={busy} onclick={() => onRestoreExcluded(source.root)}>Restore {source.excludedPaths.length} hidden</button>
            {/if}
            <button class="text-button" type="button" disabled={busy} aria-label={`Remove ${fileName(source.root)} from queue`} onclick={() => onRemove(source.root)}>Remove</button>
          </div>
        {/each}
      </div>
    {/if}

    {#if activeSource?.scanResult && onRemoveFile}
      <details class="library-queue-files">
        <summary>Manage queue · {activeFiles.length} file{activeFiles.length === 1 ? "" : "s"}{hiddenCount ? ` · ${hiddenCount} hidden` : ""}</summary>
        {#if activeFiles.length}
          <ul class="library-queue-file-list" aria-label="Queued files in the active source">
            {#each activeFiles as entry, index (entry.path)}
              <li
                class={["library-queue-file", draggingFile === entry.path && "dragging", dropFile === entry.path && "drag-over"]}
                ondragover={(event) => { if (!onReorderFiles || sourceLocked) return; event.preventDefault(); dropFile = entry.path; }}
                ondragleave={() => { if (dropFile === entry.path) dropFile = ""; }}
                ondrop={(event) => dropOnFile(event, activeSource.root, entry.path)}
              >
                {#if onReorderFiles}
                  <button class="queue-drag-handle" type="button" draggable="true" aria-label={`Drag ${entry.name} to reorder`} disabled={sourceLocked} ondragstart={(event) => beginFileDrag(event, activeSource.root, entry.path)} ondragend={clearDrag}><GripVertical size={14} /></button>
                  <button class="icon-button queue-move" type="button" aria-label={`Move ${entry.name} earlier`} disabled={sourceLocked || index === 0} onclick={() => onMoveFile?.(activeSource.root, entry.path, -1)}><ChevronUp size={14} /></button>
                  <button class="icon-button queue-move" type="button" aria-label={`Move ${entry.name} later`} disabled={sourceLocked || index === activeFiles.length - 1} onclick={() => onMoveFile?.(activeSource.root, entry.path, 1)}><ChevronDown size={14} /></button>
                {/if}
                <span class="library-queue-file-name" title={entry.path}>{entry.name}</span>
                <button class="text-button" type="button" disabled={busy} aria-label={`Remove ${entry.name} from queue`} onclick={() => onRemoveFile(entry.path)}>Remove</button>
              </li>
            {/each}
          </ul>
        {:else}
          <p class="library-queue-empty">No files remain in this source. Restore hidden files or refresh after adding media.</p>
        {/if}
      </details>
    {/if}
  </section>
{/if}

{#if scanning}
  <ScanProgressBar {scanProgress} onCancel={onCancelScan} />
{/if}
