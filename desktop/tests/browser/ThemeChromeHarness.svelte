<script lang="ts">
  // Full shell chrome (real sidebar, header, Home dashboard) for one theme, chosen by ?theme=.
  // Instrument themes key off <html>, so the gallery loads one of these per iframe.
  import ShellSidebar from '../../src/lib/views/ShellSidebar.svelte';
  import ShellHeader from '../../src/lib/views/ShellHeader.svelte';
  import HomeDashboard from '../../src/lib/HomeDashboard.svelte';
  import LibraryPosterGrid from '../../src/lib/LibraryPosterGrid.svelte';
  import LibraryQueueStatus from '../../src/lib/LibraryQueueStatus.svelte';
  import ThemeBackdrop from '../../src/lib/ThemeBackdrop.svelte';
  import { selectQueuePaths } from '../../src/lib/workspace-state.js';
  import type { LibraryPosterCard } from '../../src/lib/types.js';
  import { tick } from 'svelte';
  import { createThemeTransition } from '../../src/lib/theme-transition.js';
  import { createWorkspaceState } from '../../src/lib/workspace.svelte.js';
  import type { Environment, RunReport } from '../../src/lib/types.js';
  import { themeAttributes, themes as registry } from '../../src/lib/themes.js';
  const themes: string[] = registry.map((item) => item.id);
  const workspace = createWorkspaceState();
  const runTransition = createThemeTransition({ document, prefersReducedMotion: () => matchMedia('(prefers-reduced-motion: reduce)').matches, settle: () => tick(), setTimeout: (callback, ms) => setTimeout(callback, ms), clearTimeout: (id) => clearTimeout(id) });
  const requested = new URLSearchParams(location.search).get('theme') ?? 'light';
  let theme = $state(themes.includes(requested) || requested.length ? requested : 'light');
  const instrument = (value: string) => ['phosphor', 'ion', 'breach'].includes(value) ? value : null;
  $effect(() => {
    const attributes = themeAttributes(theme as never);
    document.documentElement.dataset.theme = attributes.theme;
    document.documentElement.dataset.palette = attributes.palette;
    if (attributes.instrument) document.documentElement.dataset.instrument = attributes.instrument; else delete document.documentElement.dataset.instrument;
  });
  workspace.notice.message = 'Library scan finished: 3 files matched, 1 needs review.';
  workspace.notice.kind = 'success';
  const report: RunReport = { csv_path: '/synthetic/run.csv', log_path: null, created: 1, counts: { OK: 4, FAILED: 1 }, rows: [] };
  const environment: Environment = {
    os: { name: 'Synthetic Ubuntu', version: 'fixture', supported: true, note: '' },
    hardware: { cpu_model: 'fixture', logical_cpus: 8, memory_total_gib: 16, memory_available_gib: 8, swap_total_gib: 0, swap_free_gib: 0, swap_used_gib: 0, swap_used_percent: 0, gpu_devices: [], encoder: 'libx265' },
    checks: [
      { key: 'ffmpeg', label: 'FFmpeg', path: '/usr/bin/ffmpeg', version: '8.0.1', available: true, required: true, detail: 'ok' },
      { key: 'mkvtoolnix', label: 'MKVToolNix', path: null, version: null, available: false, required: false, detail: 'optional, not installed' },
    ],
    x265_pixel_formats: [], dolby_vision_x265: false, can_analyze: true, youtube_ready: true, ready: true, missing_apt_packages: [],
  };
  const params = new URLSearchParams(location.search);
  const view = params.get('page') === 'library' ? 'library' : params.get('page') === 'reports' ? 'reports' : 'home';
  let page = $state<'home' | 'library'>(view === 'library' ? 'library' : 'home');
  let backdrop = $state<{ advance(seconds: number): void } | null>(null);
  (window as unknown as { __backdrop?: unknown }).__backdrop = { advance: (seconds: number) => backdrop?.advance(seconds) };
  const poster = (from: string, to: string, label: string) => 'data:image/svg+xml;utf8,' + encodeURIComponent(`<svg xmlns="http://www.w3.org/2000/svg" width="200" height="300"><defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="${from}"/><stop offset="1" stop-color="${to}"/></linearGradient></defs><rect width="200" height="300" fill="url(#g)"/><text x="100" y="160" font-family="Georgia,serif" font-size="26" font-weight="700" fill="#fff" fill-opacity=".92" text-anchor="middle">${label}</text></svg>`);
  const base: LibraryPosterCard = { path: '', filename: '', relativePath: '', selected: false, title: '', year: '2008', proposedName: '', proposedNeedsReview: false, posterUrl: null, localArtworkPath: '', matchState: 'matched', matchLabel: 'TMDB', provider: 'TMDB', animated: false, statusLabel: 'Ready', sourceBytes: '2.4 GiB', profileLine: 'HEVC 10-bit', streamLine: '1 video, 2 audio', memoryLine: '', outputLine: 'In place', outputIsCustom: false, error: '', warning: '', stale: '', runLabel: '', progressLabel: '', progressPercent: null, kind: 'media', seasonNumber: null, mediaPaths: [], selection: 'none', animation: 'none', summary: '', reviewLabel: 'Review' } as LibraryPosterCard;
  const films: [string, string, string, string][] = [['Chaos Theory', '#b8222c', '#3a0d12', 'CHAOS THEORY'], ['Gnomeo & Juliet', '#2f8f4e', '#12301c', 'GNOMEO'], ['Spirited Away', '#1b2b4a', '#07101f', 'SPIRITED AWAY'], ['The Proposal', '#d8d4cf', '#6b625a', 'THE PROPOSAL']];
  const cards: LibraryPosterCard[] = films.map(([title, from, to, label], index) => ({ ...base, path: `/media/Movies/${title}.mkv`, filename: `${title}.mkv`, relativePath: `${title}.mkv`, title, mediaPaths: [`/media/Movies/${title}.mkv`], posterUrl: poster(from, to, label), selected: index === 0, selection: index === 0 ? 'all' : 'none', animated: index === 1 || index === 2 } as LibraryPosterCard));
  if (view === 'library') {
    const entries = films.map(([title]) => ({ path: `/media/Movies/${title}.mkv`, name: `${title} (2008).mkv` }));
    workspace.queue.sources = ['Movies', 'TV Shows', 'An extremely long media folder name for a remastered collection 2024'].map((name, index) => ({ root: `/media/${name}`, status: 'ready', scanResult: index === 0 ? { candidate_count: 4, entries } : { candidate_count: 26, entries: [] }, error: null, updatedAtMs: 1, changedFiles: [], missingFiles: [], excludedPaths: [], entryOrder: [] })) as never;
    workspace.queue.activeRoot = '/media/Movies';
    workspace.queue.selectedPaths = [cards[0].path];
  }
</script>
<div class="app-shell" data-theme={themeAttributes(theme as never).theme} data-palette={theme} data-instrument={instrument(theme) ?? undefined}>
  <ShellSidebar {environment} environmentLoading={false} booting={false} root="/media/Movies" failedCount={1} navItemActive={(id) => id === 'library' && page === 'home'} openPage={() => {}} browseFolder={() => {}} fileName={(p) => p.split('/').pop() ?? p} />
  <main class="main-panel">
    <ThemeBackdrop bind:this={backdrop} {theme} />
    <ShellHeader page="home" {theme} {environment} environmentLoading={false} booting={false} setupGapDetail="" setupGapLabel="" notice={workspace.notice} applyTheme={(value) => runTransition(() => { theme = value; })} openPage={() => {}} openToolchainSettings={() => {}} loadEnvironment={() => {}} dismissNotice={() => {}} />
    <div class="page-wrap" class:page-wrap-library={view === 'library' || view === 'home'}>
      {#if view === 'library'}
        <section class="library-toolbar library-toolbar-idle">
          <details class="library-scan-options">
            <summary>Scan options</summary>
            <div class="folder-field"><input aria-label="Path to add to queue" placeholder="Paste a media file or folder path to add" /></div>
            <div class="toolbar-options"><label class="check-label"><input type="checkbox" checked /> Include subfolders</label></div>
          </details>
        </section>
        <LibraryQueueStatus queue={workspace.queue} busy={false} scanning={false} scanProgress={null} fileName={(path) => path.split('/').at(-1) ?? path} onActivate={() => {}} onRefresh={() => {}} onRemove={() => {}} onAddFolder={() => {}} onAddFiles={() => {}} onReorderSources={() => {}} onMoveSource={() => {}} onRemoveFile={() => {}} onReorderFiles={() => {}} onMoveFile={() => {}} />
        <div class="table-header-tools library-poster-search"><label class="search-box"><input aria-label="Search files" placeholder="Search files or status" /></label></div>
        <LibraryPosterGrid {cards} busy={false} selectedOnPage={1} selectedTotal={1} readyCount={4} lookupMessage="" lookupFailed={false} crumbs={[]} reviewCurrentLabel="" onReviewCurrent={() => {}} canRename={true} canProcess={true} observeArtwork={() => {}} onToggleSelect={(values, selected) => { workspace.queue.selectedPaths = selectQueuePaths(workspace.queue.selectedPaths, values, selected); }} onToggleAnimated={() => {}} onOpenReview={() => {}} onOpenLocation={() => {}} onReview={() => {}} onRename={() => {}} onMetadata={() => {}} onIntegrity={() => {}} onRebuild={() => {}} onSetOutput={() => {}} onClearOutput={() => {}} onSelectPage={() => {}} />
      {:else if view === 'reports'}
        <section class="report-failures" aria-label="Failed results">
          <h2>1 failed result</h2>
          <p class="report-failures-note">Each file shows its most recent outcome. A later successful run clears an earlier failure.</p>
          <ul><li><strong>Spirited Away (2001).mkv</strong><span>Failed</span><p>Output is missing or does not match the source after validation.</p></li></ul>
        </section>
        <section class="table-card">
          <div class="table-header"><h2>Run results</h2></div>
          <div class="table-scroll"><table class="report-table"><thead><tr><th>File</th><th>Status</th><th>Size</th><th>Quality</th></tr></thead><tbody>
            {#each films as film, i (film[0])}<tr><td><div class="file-cell-title">{film[0]} (2008).mkv</div><div class="file-cell-sub">/media/Movies</div></td><td><span class="status-tag">{i === 2 ? 'Failed' : 'OK'}</span></td><td>{(2.4 - i * 0.3).toFixed(1)} GiB</td><td>SSIM 0.9{6 - i}</td></tr>{/each}
          </tbody></table></div>
        </section>
      {:else}
      <HomeDashboard skin={instrument(theme) as 'phosphor' | 'ion' | 'breach' | null} {environment} environmentLoading={false} sourceCount={2} fileCount={14}
        activeFolder="Movies" statusCounts={[{ key: 'ready', label: 'Ready', count: 9 }, { key: 'review', label: 'Needs review', count: 3 }, { key: 'failed', label: 'Failed', count: 2 }]}
        activityLabel="Idle" reports={[report]} setupLogs={[]} onOpen={() => {}} onOpenReport={() => {}} onAddFolder={() => {}} onAddFiles={() => {}} onRefreshChecks={() => {}} />
      {/if}
      <section class="theme-fixture-controls" style="display:flex;gap:10px;align-items:center;flex-wrap:wrap;margin-top:16px">
        <button class="button primary">Primary</button><button class="button secondary">Secondary</button><button class="button danger-outline">Danger</button>
        <button class="text-button">Text action</button>
        <span class="status-pill good"><span class="status-dot"></span> GOOD</span><span class="status-pill warning">WARNING</span><span class="status-pill neutral">NEUTRAL</span>
        <input aria-label="Sample input" placeholder="Search titles" /><select aria-label="Sample select"><option>HEVC 10-bit</option></select>
      </section>
    </div>
  </main>
</div>
