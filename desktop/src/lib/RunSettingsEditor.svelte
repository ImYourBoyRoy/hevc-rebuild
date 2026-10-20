<script lang="ts">
  import { Film, Info, Layers, ShieldCheck, SlidersHorizontal, X } from "@lucide/svelte";
  import Modal from "./Modal.svelte";
  import type { FileRunSettingsOverride, MediaEntry, Settings } from "./types.js";

  interface Props {
    open: boolean;
    entries: MediaEntry[];
    overrides: Record<string, FileRunSettingsOverride>;
    settings: Settings | null;
    busy: boolean;
    fileName: (path: string) => string;
    selectedStreams: (entry: MediaEntry) => number[];
    streamIndicesForMode: (entry: MediaEntry, mode: "default" | "all" | "primary" | "no-data") => number[];
    onApply: (paths: string[], override: FileRunSettingsOverride, streamMode: "keep" | "default" | "all" | "primary" | "no-data" | "exact") => void;
    onClear: (paths: string[]) => void;
    onClose: () => void;
  }

  let {
    open,
    entries,
    overrides,
    settings,
    busy,
    fileName,
    selectedStreams,
    streamIndicesForMode,
    onApply,
    onClear,
    onClose,
  }: Props = $props();

  type TuneChoice = "inherit" | "auto" | "animation" | "grain" | "none";
  type DolbyChoice = "inherit" | "safe" | "skip" | "hdr10-fallback";
  type StreamMode = "keep" | "default" | "all" | "primary" | "no-data" | "exact";

  let crfSet = $state(false);
  let crf = $state(21);
  let presetSet = $state(false);
  let preset = $state("slow");
  let promoteSet = $state(false);
  let promote = $state(false);
  let tune = $state<TuneChoice>("inherit");
  let hdrSet = $state(false);
  let preserveHdr10plus = $state(true);
  let dolby = $state<DolbyChoice>("inherit");
  let streamMode = $state<StreamMode>("keep");
  let exactStreams = $state<number[]>([]);
  let initializedKey = $state("");

  let firstEntry = $derived(entries[0] ?? null);
  let singleFile = $derived(entries.length === 1);
  let currentOverrideCount = $derived(entries.filter((entry) => Boolean(overrides[entry.path] && Object.keys(overrides[entry.path]).length)).length);

  function commonValue<K extends keyof FileRunSettingsOverride>(key: K): FileRunSettingsOverride[K] | undefined {
    const values = entries.map((entry) => overrides[entry.path]?.[key]).filter((value) => value !== undefined) as FileRunSettingsOverride[K][];
    if (!values.length || values.some((value) => value !== values[0])) return undefined;
    return values[0];
  }

  function initialize() {
    const first = firstEntry ? overrides[firstEntry.path] ?? {} : {};
    const commonCrf = commonValue("crf");
    crfSet = commonCrf !== undefined;
    crf = Number(commonCrf ?? settings?.video.crf ?? 21);
    const commonPreset = commonValue("preset");
    presetSet = commonPreset !== undefined;
    preset = String(commonPreset ?? settings?.video.preset ?? "slow");
    const commonPromote = commonValue("promote_8bit_to_10bit");
    promoteSet = commonPromote !== undefined;
    promote = Boolean(commonPromote ?? settings?.video.promote_8bit_to_10bit ?? false);
    const commonTune = commonValue("tune");
    tune = commonTune === undefined ? "inherit" : commonTune;
    const commonHdr = commonValue("preserve_hdr10plus");
    hdrSet = commonHdr !== undefined;
    preserveHdr10plus = Boolean(commonHdr ?? settings?.hdr.preserve_hdr10plus ?? true);
    const commonDolby = commonValue("dolby_vision");
    dolby = commonDolby === undefined ? "inherit" : commonDolby;
    if (singleFile && firstEntry) {
      const savedStreams = first?.stream_indices;
      exactStreams = [...(savedStreams ?? selectedStreams(firstEntry))];
      streamMode = savedStreams
        ? savedStreams.join(",") === streamIndicesForMode(firstEntry, "default").join(",") ? "default" : "exact"
        : "keep";
    } else {
      streamMode = "keep";
      exactStreams = [];
    }
  }

  $effect(() => {
    const key = `${open ? "open" : "closed"}:${entries.map((entry) => entry.path).join("\u0000")}`;
    if (open && key !== initializedKey) {
      initialize();
      initializedKey = key;
    }
    if (!open) initializedKey = "";
  });

  function toggleExactStream(index: number, checked: boolean) {
    if (!firstEntry) return;
    const next = checked ? [...new Set([...exactStreams, index])] : exactStreams.filter((value) => value !== index);
    const realVideoCount = (firstEntry.profile?.tracks ?? []).filter((track) =>
      track.type === "video" && !track.attached_picture && (track.width ?? 0) > 0 && (track.height ?? 0) > 0 && next.includes(track.index),
    ).length;
    if (!checked && realVideoCount === 0) return;
    exactStreams = next.sort((a, b) => a - b);
  }

  function apply() {
    if (!entries.length || busy) return;
    const next: FileRunSettingsOverride = {};
    if (crfSet) next.crf = Math.max(0, Math.min(51, Math.round(Number(crf) || 0)));
    if (presetSet) next.preset = preset;
    if (promoteSet) next.promote_8bit_to_10bit = promote;
    if (tune !== "inherit") next.tune = tune;
    if (hdrSet) next.preserve_hdr10plus = preserveHdr10plus;
    if (dolby !== "inherit") next.dolby_vision = dolby;
    if (streamMode === "exact" && exactStreams.length) next.stream_indices = exactStreams;
    onApply(entries.map((entry) => entry.path), next, streamMode);
    onClose();
  }

  function clear() {
    if (!entries.length || busy) return;
    onClear(entries.map((entry) => entry.path));
    onClose();
  }

  function trackLabel(type: string): string {
    if (type === "subtitle") return "Subtitle";
    if (type === "attachment") return "Attachment";
    if (type === "data") return "Data";
    if (type === "audio") return "Audio";
    return "Video";
  }
</script>

<Modal open={open} class="run-settings-modal" labelId="run-settings-title" onClose={onClose}>
  <button class="icon-button modal-close" type="button" aria-label="Close current-run settings" onclick={onClose}><X size={18} /></button>
  <div class="eyebrow">CURRENT RUN ONLY</div>
  <h2 id="run-settings-title">Adjust this run</h2>
  <p class="run-settings-lead">
    {#if singleFile}Editing <strong>{fileName(firstEntry?.path ?? "")}</strong>.{:else}Editing {entries.length} selected files.{/if}
    These choices are sent to the encoder for this run and do not change saved defaults.
  </p>

  <div class="run-settings-targets">
    {#each entries.slice(0, 5) as entry (entry.path)}<span title={entry.path}><Film size={13} />{fileName(entry.path)}</span>{/each}
    {#if entries.length > 5}<span>+{entries.length - 5} more</span>{/if}
  </div>

  <section class="run-settings-group">
    <div class="run-settings-group-heading"><SlidersHorizontal size={17} /><div><h3>Video encoding</h3><p>Only fields you switch on override the run settings.</p></div></div>
    <div class="run-settings-fields">
      <label class="run-override-field" title="CRF controls quality and size. Lower values retain more detail and create larger files; higher values compress more and create smaller files.">
        <span><input type="checkbox" bind:checked={crfSet} /> <strong>CRF</strong><small>lower = larger, higher-quality output</small></span>
        <input type="number" min="0" max="51" step="1" bind:value={crf} disabled={!crfSet} aria-label="Per-file CRF" />
      </label>
      <label class="run-override-field" title="Preset changes compression effort, not target quality. Slower presets take longer but can reduce size at the same CRF; faster presets finish sooner.">
        <span><input type="checkbox" bind:checked={presetSet} /> <strong>Encoder effort</strong><small>slower takes longer; quality target stays CRF-based</small></span>
        <select bind:value={preset} disabled={!presetSet} aria-label="Per-file encoder effort"><option value="ultrafast">ultrafast</option><option value="superfast">superfast</option><option value="veryfast">veryfast</option><option value="faster">faster</option><option value="fast">fast</option><option value="medium">medium</option><option value="slow">slow</option><option value="slower">slower</option><option value="veryslow">veryslow</option></select>
      </label>
      <label class="run-override-field run-override-toggle" title="Promoting an 8-bit source changes the output pixel format to 10-bit when supported. It does not restore color detail that was not present in the source.">
        <span><input type="checkbox" bind:checked={promoteSet} /> <strong>8-bit promotion</strong><small>encode this file as 10-bit when supported</small></span>
        <input type="checkbox" bind:checked={promote} disabled={!promoteSet} aria-label="Promote this file to 10-bit" />
      </label>
      <label class="run-override-field" title="Film grain preserves grain structure more deliberately and can increase output size. Animation favors clean edges and flat areas. Automatic uses the saved detection rules.">
        <span><strong>Content tune</strong><small>film grain is available per file here</small></span>
        <select bind:value={tune} aria-label="Per-file content tune"><option value="inherit">Use current setting</option><option value="auto">Automatic</option><option value="animation">Animation</option><option value="grain">Film grain</option><option value="none">Normal</option></select>
      </label>
    </div>
  </section>

  <section class="run-settings-group">
    <div class="run-settings-group-heading"><ShieldCheck size={17} /><div><h3>HDR handling</h3><p>Static HDR signaling stays source-aware; these controls cover dynamic metadata and Dolby Vision policy.</p></div></div>
    <div class="run-settings-fields">
      <label class="run-override-field run-override-toggle" title="HDR10+ is dynamic scene metadata. Turning this off deliberately omits that dynamic metadata from the new encode; static HDR signaling remains source-aware.">
        <span><input type="checkbox" bind:checked={hdrSet} /> <strong>HDR10+ dynamic metadata</strong><small>preserve and verify it when available</small></span>
        <input type="checkbox" bind:checked={preserveHdr10plus} disabled={!hdrSet} aria-label="Preserve HDR10+ for this file" />
      </label>
      <label class="run-override-field" title="Safe preservation keeps verified Dolby Vision. Verified HDR10 fallback removes Dolby Vision only when the source has a safe HDR10 base. Skip file leaves the source untouched and marks it skipped.">
        <span><strong>Dolby Vision</strong><small>choose preservation or a verified fallback</small></span>
        <select bind:value={dolby} aria-label="Per-file Dolby Vision policy"><option value="inherit">Use current setting</option><option value="safe">Safe preservation</option><option value="hdr10-fallback">Verified HDR10 fallback</option><option value="skip">Skip this file</option></select>
      </label>
    </div>
  </section>

  <section class="run-settings-group">
    <div class="run-settings-group-heading"><Layers size={17} /><div><h3>Tracks in the output</h3><p>Video tracks are encoded when selected; audio, subtitles, attachments, and data are copied when selected.</p></div></div>
    {#if singleFile && firstEntry?.profile?.tracks.length}
      <label class="run-override-field" title="Choose exactly which FFprobe streams are mapped into this file's MKV output.">
        <span><strong>Track selection</strong><small>exact selection for this file</small></span>
        <select bind:value={streamMode} aria-label="Track selection mode"><option value="keep">Keep current selection</option><option value="default">Use run defaults</option><option value="exact">Choose tracks below</option></select>
      </label>
      {#if streamMode === "exact"}
        <div class="run-track-list">
          {#each firstEntry.profile.tracks as track (track.index)}
            <label title={`${trackLabel(track.type)} stream #${track.index}${track.title ? ` · ${track.title}` : ""}`}>
              <input type="checkbox" checked={exactStreams.includes(track.index)} onchange={(event) => toggleExactStream(track.index, event.currentTarget.checked)} />
              <span><strong>{trackLabel(track.type)} {track.ordinal + 1}</strong><small>#{track.index}{track.codec ? ` · ${track.codec}` : ""}{track.language ? ` · ${track.language}` : ""}{track.title ? ` · ${track.title}` : ""}</small></span>
            </label>
          {/each}
        </div>
      {/if}
    {:else if entries.length > 1}
      <label class="run-override-field" title="Batch choices are translated to each file's own FFprobe indices, so a folder can be edited safely even when its files have different track layouts.">
        <span><strong>Track selection for this batch</strong><small>translated per file from each file's scan</small></span>
        <select bind:value={streamMode} aria-label="Batch track selection"><option value="keep">Keep each file's current selection</option><option value="default">Use run defaults</option><option value="all">Keep all supported tracks</option><option value="primary">Primary video + audio, subtitles, attachments</option><option value="no-data">Keep all except data streams</option></select>
      </label>
    {:else}
      <p class="run-settings-empty"><Info size={15} />Finish the media scan before changing tracks.</p>
    {/if}
  </section>

  {#if currentOverrideCount}<p class="run-settings-existing"><Info size={14} />{currentOverrideCount} of these files already have current-run choices. Applying only changes the fields you enable; reset clears all current-run choices for this scope.</p>{/if}
  <div class="modal-actions">
    <button class="button secondary" type="button" disabled={busy} onclick={clear}>Clear current-run choices</button>
    <button class="button secondary" type="button" disabled={busy} onclick={onClose}>Cancel</button>
    <button class="button primary" type="button" data-modal-initial-focus disabled={busy || !entries.length} onclick={apply}>Apply to {entries.length} file{entries.length === 1 ? "" : "s"}</button>
  </div>
</Modal>
