<script lang="ts">
  import ResourceTelemetry from './ResourceTelemetry.svelte';
  import type { RunState } from './workspace-state.js';
  interface Props { run: RunState; title: string; elapsedLabel: string; onStop: () => unknown }
  let { run, title, elapsedLabel, onStop }: Props = $props();
</script>
<!-- app.css explicitly scopes console layout and theme overrides to .run-console-* and .run-message. -->
<section id="run-activity" class="run-console-card">
  <div class="run-console-heading"><div><span class="console-indicator" class:running={Boolean(run.id)}></span><strong>{title}</strong><span>{run.id ? "LIVE" : "LATEST"}</span></div>{#if run.id}<button class="text-button" onclick={onStop}>Stop safely</button>{/if}</div>
  {#if run.id && run.progress}<p class="run-message">{run.progress.phase} · {elapsedLabel} elapsed</p>{:else if run.message}<p class="run-message">{run.message}</p>{/if}
  <ResourceTelemetry snapshot={run.resources} />
  <pre class="run-console">{run.lines.length ? run.lines.join("\n") : "Waiting for engine output…"}</pre>
</section>
