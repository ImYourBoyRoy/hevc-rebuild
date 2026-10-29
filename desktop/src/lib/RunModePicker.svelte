<!-- ./desktop/src/lib/RunModePicker.svelte
     Compact processing-task tabs for Run setup. Selected copy sits under the
     strip so Check/Integrity pages stay short. -->
<script lang="ts">
  import { FileSearch, FileText, Play, ScanSearch, Zap } from "@lucide/svelte";
  import type { RunMode } from "./types.js";
  import "../styles/components/run-mode.css";

  type PickerMode = Exclude<RunMode, "youtube">;
  interface Props {
    mode: RunMode;
    disabled: boolean;
    toolsReady: boolean;
    onSelect: (mode: PickerMode) => void;
  }
  let { mode, disabled, toolsReady, onSelect }: Props = $props();

  const options: {
    id: PickerMode;
    icon: typeof ScanSearch;
    title: string;
    description: string;
    tag: string;
  }[] = [
    { id: "check", icon: ScanSearch, title: "Check", description: "Preview streams, HDR, and the memory plan. Nothing is written.", tag: "Read-only · native" },
    { id: "integrity", icon: FileSearch, title: "Integrity", description: "Find damaged containers or bitstreams. Quick inspects structure; Full decodes every frame. Nothing is written.", tag: "Read-only · native" },
    { id: "metadata", icon: FileText, title: "Metadata", description: "Copy every track into a new MKV with your reviewed title and poster. No re-encode.", tag: "New file · source kept" },
    { id: "test", icon: Play, title: "Short test", description: "Encode a 5-second sample of each file to confirm tracks, HDR, and timing survive.", tag: "Sample · source kept" },
    { id: "full", icon: Zap, title: "Rebuild", description: "Re-encode the real files to HEVC. A file is replaced only after its new copy passes validation.", tag: "Review before start" },
  ];

  let selected = $derived(options.find((option) => option.id === mode) ?? options[0]);

  function onTabKeydown(event: KeyboardEvent, index: number) {
    if (disabled) return;
    const last = options.length - 1;
    let next = index;
    if (event.key === "ArrowRight" || event.key === "ArrowDown") next = index === last ? 0 : index + 1;
    else if (event.key === "ArrowLeft" || event.key === "ArrowUp") next = index === 0 ? last : index - 1;
    else if (event.key === "Home") next = 0;
    else if (event.key === "End") next = last;
    else return;
    event.preventDefault();
    onSelect(options[next].id);
    const tabs = (event.currentTarget as HTMLElement).parentElement?.querySelectorAll<HTMLButtonElement>("[role='radio']");
    tabs?.[next]?.focus();
  }
</script>

<section class="mode-section run-mode-picker">
  <div class="section-title-row">
    <div>
      <div class="eyebrow">PROCESSING TASK</div>
      <h2>Choose what to do</h2>
    </div>
    <span class="mode-guidance">{toolsReady ? "Required tools ready" : "Some required tools are missing"}</span>
  </div>
  <div class="run-mode-tabs" role="radiogroup" aria-label="Processing task">
    {#each options as option, index (option.id)}
      {@const Icon = option.icon}
      <button
        type="button"
        role="radio"
        class={["run-mode-tab", { selected: mode === option.id }]}
        aria-checked={mode === option.id}
        tabindex={mode === option.id ? 0 : -1}
        {disabled}
        onclick={() => onSelect(option.id)}
        onkeydown={(event) => onTabKeydown(event, index)}
      >
        <span class="run-mode-tab-icon" aria-hidden="true"><Icon size={16} /></span>
        <span class="run-mode-tab-title">{option.title}</span>
      </button>
    {/each}
  </div>
  <div class="run-mode-copy">
    <p>{selected.description}</p>
    <small>{selected.tag}</small>
  </div>
</section>
