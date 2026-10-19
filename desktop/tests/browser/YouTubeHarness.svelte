<script lang="ts">
  import YouTubePreparation from '../../src/lib/YouTubePreparation.svelte';
  import {createWorkspaceState} from '../../src/lib/workspace.svelte.js';
  const workspace=createWorkspaceState();
  let root=$state('/synthetic/source'),recursive=$state(true),minimumSizeMib=$state('5'),youtubeResolution=$state<'source'|'1440p'|'4k'>('1440p'),youtubeQualityMode=$state<'balanced'|'recommended'>('balanced'),youtubeSourceMultiplier=$state(2),youtubeOutputDir=$state('/synthetic/output'),search=$state(''),mediaPage=$state(1),busy=$state(false),tools=$state(false),active=$state(false),callback=$state('');
  $effect(()=>{workspace.run.id=active?'fixture-run':'';workspace.run.message=active?'Encoding':'Export failed: synthetic invalid HDR';workspace.run.lines=['Synthetic failure: source unchanged'];});
</script>
<div class="app-shell" data-theme="light"><main style="width:100%;min-width:0;padding:24px">
  <label><input type="checkbox" bind:checked={busy}/>YouTube busy</label><label><input type="checkbox" bind:checked={tools}/>YouTube tools ready</label><label><input type="checkbox" bind:checked={active}/>YouTube active</label>
  <output aria-label="YouTube callback">{callback}</output>
  <YouTubePreparation queue={workspace.queue} run={workspace.run} bind:root bind:recursive bind:minimumSizeMib bind:youtubeResolution bind:youtubeQualityMode bind:youtubeSourceMultiplier bind:youtubeOutputDir bind:search bind:mediaPage scanResult={null} scanProgress={null} scanning={false} workflowBusy={busy} desktopShell={true} youtubeEstimateSummary={null} analysisErrors={[]} toolsRequiredEntries={[]} selectedRunEntries={[]} visibleEntries={[]} selectedVisibleCount={0} selectedFilteredCount={0} mediaPageSize={50} selectedRunInputBytes={0} selectedMediaPathsForRun={[]} modeToolsReady={tools} canStart={false} progressElapsedLabel="0s" bytes={(value)=>`${value} B`} fileName={(value)=>value.split('/').at(-1)??value} persistPreferences={(patch)=>{callback=JSON.stringify(patch);}} browseFolder={()=>callback='folder'} browseFile={()=>callback='files'} analyzeFolder={()=>callback='scan'} chooseYoutubeOutput={()=>callback='output'} openPage={(value)=>callback=value} setFilteredSelection={()=>callback='selection'} stopAfterCurrent={()=>callback='stop'} beginRun={()=>callback='start'}>
    {#snippet mediaTable()}<p>Isolated media table boundary</p>{/snippet}
  </YouTubePreparation>
</main></div>
