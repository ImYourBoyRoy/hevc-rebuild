<script lang="ts">
  import RunConsole from '../../src/lib/RunConsole.svelte';
  import RunProgress from '../../src/lib/RunProgress.svelte';
  import { createWorkspaceState } from '../../src/lib/workspace.svelte.js';
  import type { RuntimeResourceSnapshot } from '../../src/lib/types.js';
  const workspace=createWorkspaceState();
  let state=$state('active'), telemetry=$state('known'), callback=$state('');
  const unknown: RuntimeResourceSnapshot={sampled_at_unix_ms:1,cpu_percent:null,load1:null,memory_total_gib:null,memory_available_gib:null,swap_total_gib:null,swap_free_gib:null,swap_used_gib:null,swap_used_percent:null,swap_in_bytes_per_second:null,swap_out_bytes_per_second:null,memory_some_avg10:null,memory_full_avg10:null,io_some_avg10:null,io_full_avg10:null,cpu_some_avg10:null,disk_total_bytes:null,disk_used_bytes:null,disk_free_bytes:null,disk_free_percent:null,engine_process_rss_gib:null,engine_process_swap_gib:null,ffmpeg_process_rss_gib:null,ffmpeg_process_swap_gib:null,temperature_c:null};
  $effect(()=>{workspace.run.id=state==='active'?'fixture-run':'';workspace.run.message=state==='active'?'Encoding':state;workspace.run.lines=['Calibration: synthetic layout','Report: /synthetic/run.csv',`Outcome: ${state}`];workspace.run.progress=state==='active'?{runId:'fixture-run',fileName:'Synthetic media',phase:'Encoding',batchNumber:1,batchTotal:2,filePercent:50,queuePercent:25,speed:1.2,etaSeconds:20,elapsedSeconds:10,inputBytes:100,outputBytes:40,estimatedOutputBytes:80}:null;});
  $effect(()=>{workspace.run.resources=telemetry==='unknown'?{...unknown}:telemetry==='nonfinite'?{...unknown,cpu_percent:NaN,load1:Infinity,memory_total_gib:NaN,memory_available_gib:Infinity,disk_free_bytes:NaN,temperature_c:Infinity}:{...unknown,cpu_percent:25,load1:1.5,memory_total_gib:8,memory_available_gib:4,swap_used_gib:0,swap_used_percent:0,ffmpeg_process_rss_gib:1,disk_free_bytes:4*1024**3,disk_total_bytes:8*1024**3,disk_free_percent:50,temperature_c:42};});
  function stop(){workspace.run.stopAfterCurrent=true;callback='stop';}
</script>
<div class="app-shell" data-theme="light"><main style="width:100%;min-width:0;padding:24px">
  <label>Run state<select aria-label="Fixture run state" bind:value={state}>{#each ['active','cancelled','failed','recovered','stale'] as value}<option>{value}</option>{/each}</select></label>
  <label>Telemetry<select aria-label="Fixture telemetry" bind:value={telemetry}>{#each ['known','unknown','nonfinite'] as value}<option>{value}</option>{/each}</select></label>
  <output aria-label="Run callback">{callback}</output>
  <RunProgress run={workspace.run} encodeMeasured={Boolean(workspace.run.progress)} overallRunPercent={25} progressElapsedLabel="10s" batchCount={2} batchIndex={0} bytes={(value)=>value===null||value===undefined?'—':`${value} B`} duration={(value)=>value===undefined?'—':`${value}s`} openProgressDetails={()=>callback='details'} stopAfterCurrent={stop}/>
  <RunConsole run={workspace.run} title="Run activity" elapsedLabel="10s" onStop={stop}/>
</main></div>
