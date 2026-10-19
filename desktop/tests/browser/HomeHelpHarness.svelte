<script lang="ts">
  import HomeDashboard from '../../src/lib/HomeDashboard.svelte';
  import HelpCenter from '../../src/lib/HelpCenter.svelte';
  import { createWorkspaceState } from '../../src/lib/workspace.svelte.js';
  import type { Environment, RunReport } from '../../src/lib/types.js';
  const workspace = createWorkspaceState();
  let theme = $state('light');
  $effect(() => { document.documentElement.dataset.theme = theme; if (['phosphor','ion','breach'].includes(theme)) document.documentElement.dataset.instrument = theme; else delete document.documentElement.dataset.instrument; });
  let toolState = $state('missing');
  let view = $state('home');
  let populated = $state(false);
  const report: RunReport = { csv_path: '/synthetic/run.csv', log_path: null, created: 1, counts: { OK: 1 }, rows: [] };
  let environment = $derived<Environment | null>(toolState === 'unknown' ? null : {
    os: { name: 'Synthetic Ubuntu', version: 'fixture', supported: true, note: '' },
    hardware: { cpu_model: 'fixture', logical_cpus: 4, memory_total_gib: 8, memory_available_gib: 4, swap_total_gib: 0, swap_free_gib: 0, swap_used_gib: 0, swap_used_percent: 0, gpu_devices: [], encoder: 'libx265' },
    checks: [{ key: 'ffmpeg', label: 'FFmpeg', path: null, version: '8.0.1', available: toolState === 'ready', required: true, detail: 'synthetic check' }],
    x265_pixel_formats: [], dolby_vision_x265: false, can_analyze: toolState === 'ready', youtube_ready: toolState === 'ready', ready: toolState === 'ready', missing_apt_packages: toolState === 'ready' ? [] : ['ffmpeg'],
  });
</script>
<div class="app-shell" data-theme={theme} data-instrument={['phosphor', 'ion', 'breach'].includes(theme) ? theme : undefined}>
  <main style="padding: 24px; width: 100%; min-width: 0">
    <label>Theme<select aria-label="Fixture theme" bind:value={theme}>{#each ['light','dark','phosphor','ion','breach'] as item}<option>{item}</option>{/each}</select></label>
    <label>Tools<select aria-label="Fixture tools" bind:value={toolState}>{#each ['unknown','loading','missing','ready'] as item}<option>{item}</option>{/each}</select></label>
    <label><input type="checkbox" bind:checked={populated} /> Populated queue</label>
    <button onclick={() => view = view === 'home' ? 'help' : 'home'}>Toggle Home/Help</button>
    <output aria-label="Last callback">{workspace.notice.message}</output>
    {#if view === 'home'}
      <HomeDashboard
        skin={theme === 'phosphor' || theme === 'ion' || theme === 'breach' ? theme : null}
        {environment} environmentLoading={toolState === 'loading'} sourceCount={populated ? 1 : 0} fileCount={populated ? 2 : 0}
        activeFolder={populated ? 'Synthetic library' : ''} statusCounts={populated ? [{ key: 'ready', label: 'Ready', count: 2 }] : []}
        activityLabel={workspace.run.message} reports={populated ? [report] : []} setupLogs={populated ? [{ jobId: 'fixture', modifiedUnix: 1, sizeBytes: 1024 }] : []}
        onOpen={(page) => workspace.notice.message = page}
        onOpenReport={(path) => workspace.notice.message = path}
        onAddFolder={() => workspace.notice.message = 'folder'} onAddFiles={() => workspace.notice.message = 'files'}
        onRefreshChecks={() => workspace.notice.message = 'refresh'}
      />
    {:else}<HelpCenter />{/if}
  </main>
</div>
