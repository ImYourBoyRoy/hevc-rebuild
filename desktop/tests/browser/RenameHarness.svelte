<script lang="ts">
  import LibraryRenameDialog from '../../src/lib/LibraryRenameDialog.svelte';
  import MetadataWorkbench from '../../src/lib/MetadataWorkbench.svelte';
  import { insertRenameTemplateToken } from '../../src/lib/rename-template.js';
  import type { MediaEntry, OrganizerTemplates } from '../../src/lib/types.js';
  let open = $state(false), busy = $state(false), error = $state('');
  let dialog = $state<HTMLDialogElement|null>(null), movieField=$state<HTMLInputElement|null>(null), episodeField=$state<HTMLInputElement|null>(null);
  let shape = $state<'movie'|'episode'|'both'>('both');
  let multiple=$state(false), applied=$state(false);
  const defaults: OrganizerTemplates = {movie:'{title}{ext}',series:'{series_title}',season:'Season {season:02}',episode:'S{season:02}E{episode:02} {episode_title}{ext}'};
  let draft=$state({...defaults});
  // Rendering only uses path/name; no media or backend operation is reachable from this fixture.
  const entries = $derived<MediaEntry[]>([{path:'/synthetic/one.mkv',name:'one.mkv'},...(multiple?[{path:'/synthetic/two.mkv',name:'two.mkv'}]:[])] as MediaEntry[]);
  function insert(kind:'movie'|'episode',token:string,field:HTMLInputElement|null){const value=insertRenameTemplateToken(draft[kind],token,field?.selectionStart??null,field?.selectionEnd??null);draft={...draft,[kind]:value.value};error='';queueMicrotask(()=>{field?.focus();field?.setSelectionRange(value.caret,value.caret);});}
</script>
<div class="app-shell" data-theme="light"><main style="width:100%;min-width:0;padding:24px">
  <button onclick={()=>{open=true;applied=false;}}>Open rename fixture</button>
  <label><input type="checkbox" bind:checked={multiple}/>Whole selection</label><label><input type="checkbox" bind:checked={busy}/>Rename busy</label>
  <label>Shape<select aria-label="Rename shape" bind:value={shape}><option>movie</option><option>episode</option><option>both</option></select></label>
  <output aria-label="Rename applied">{String(applied)}</output>
  <LibraryRenameDialog libraryRenameOpen={open} bind:libraryRenameDialog={dialog} bind:renameDraft={draft} bind:moviePatternField={movieField} bind:episodePatternField={episodeField} bind:libraryRenameError={error} libraryRenameBusy={busy} libraryRenameEntries={()=>entries} libraryRenameShape={()=>shape} libraryRenameNote={(entry)=>({next:entry.name.replace('.mkv','.reviewed.mkv'),note:'Synthetic preview'})} insertRenameToken={insert} resetRenamePattern={(kind)=>{draft={...draft,[kind]:defaults[kind]};error='';}} applyLibraryRename={()=>{error='Synthetic collision: target exists';applied=false;}} onClose={()=>open=false}/>
  <MetadataWorkbench metadataAutoLookup={false} workbenchActive={false} onOpenSettings={()=>error='settings callback'}/>
</main></div>
