<script lang="ts">
  import { Activity } from "@lucide/svelte";
  import type { RuntimeResourceSnapshot } from "#lib/types.js";

  let { snapshot }: { snapshot: RuntimeResourceSnapshot | null } = $props();

  import { finiteNumber, gib, gibBytes, percent, ramUsed, byteRate } from "./telemetry-format.js";
</script>

{#if snapshot}
  <section class="resource-telemetry" aria-label="Live resource telemetry">
    <header>
      <div class="telemetry-title"><Activity size={16} /><strong>Live resource telemetry</strong></div>
      <span>Rust observer · {finiteNumber(snapshot.sampled_at_unix_ms) ? new Date(snapshot.sampled_at_unix_ms).toLocaleTimeString() : "Sample time unavailable"}</span>
    </header>
    <div class="telemetry-grid">
      <div><small>AVAILABLE RAM</small><strong>{gib(snapshot.memory_available_gib)} / {gib(snapshot.memory_total_gib)}</strong></div>
      <div><small>SWAP IN USE</small><strong>{gib(snapshot.swap_used_gib)} · {percent(snapshot.swap_used_percent)}</strong><span>In {byteRate(snapshot.swap_in_bytes_per_second)} · Out {byteRate(snapshot.swap_out_bytes_per_second)}</span></div>
      <div><small>FFMPEG RSS</small><strong>{gib(snapshot.ffmpeg_process_rss_gib)}</strong><span>Process swap {gib(snapshot.ffmpeg_process_swap_gib)}</span></div>
      <div><small>SYSTEM PRESSURE</small><strong>RAM {ramUsed(snapshot.memory_total_gib, snapshot.memory_available_gib)}</strong><span>Memory stall {percent(snapshot.memory_full_avg10)} · CPU stall {percent(snapshot.cpu_some_avg10)} · CPU {percent(snapshot.cpu_percent)} · Load {finiteNumber(snapshot.load1) ? snapshot.load1.toFixed(2) : "—"}</span></div>
      <div><small>RUN VOLUME FREE / TOTAL</small><strong>{gibBytes(snapshot.disk_free_bytes)} / {gibBytes(snapshot.disk_total_bytes)}</strong><span>{percent(snapshot.disk_free_percent)} free · {gibBytes(snapshot.disk_used_bytes)} used · {!finiteNumber(snapshot.temperature_c) ? "Temperature unavailable" : `Sensor ${snapshot.temperature_c.toFixed(0)} °C`}</span></div>
    </div>
    <p>RAM used is how full memory is. Memory stall is how long tasks waited for RAM. Swap already on disk stays on the swap card. A stall near 0% means the encode is moving, and the memory and disk safeguards still apply.</p>
  </section>
{/if}

<style>
  .resource-telemetry { margin: 10px; padding: 13px 15px; border: 1px solid var(--theme-line); border-radius: 11px; color: var(--theme-text); background: var(--theme-raised); }
  header { display: flex; justify-content: space-between; align-items: center; gap: 12px; margin-bottom: 11px; }
  .telemetry-title { display: flex; align-items: center; gap: 8px; }
  .telemetry-title :global(svg) { color: var(--theme-accent); }
  header strong { font-size: 13px; }
  header > span { color: var(--theme-muted); font-size: 12px; }
  .telemetry-grid { display: grid; grid-template-columns: repeat(5, minmax(0, 1fr)); gap: 8px; }
  .telemetry-grid > div { min-width: 0; display: grid; align-content: start; gap: 4px; padding: 9px; border: 1px solid var(--theme-line); border-radius: 8px; background: var(--theme-surface); }
  .telemetry-grid small { color: var(--theme-muted); font-size: 12px; font-weight: 700; letter-spacing: .06em; }
  .telemetry-grid strong { overflow-wrap: anywhere; font-size: 14px; }
  .telemetry-grid span { color: var(--theme-muted); font-size: 12px; line-height: 1.4; }
  .resource-telemetry p { margin: 10px 0 0; color: var(--theme-muted); font-size: 12px; line-height: 1.45; }
  @media (max-width: 1100px) { .telemetry-grid { grid-template-columns: repeat(3, minmax(0, 1fr)); } }
  @media (max-width: 680px) { header { align-items: flex-start; flex-direction: column; } .telemetry-grid { grid-template-columns: repeat(2, minmax(0, 1fr)); } }
</style>
