<script lang="ts">
  import Modal from '../../src/lib/Modal.svelte';
  let open = $state(false);
  let chainedOpen = $state(false);
  let opener = $state<HTMLButtonElement | null>(null);
  let acknowledgment = $state(false);
  let busy = $state(false);
  let error = $state('');
  let closes = $state(0);
  let backgroundActions = $state(0);
  let backdrop = $state(false);
  let surface = $state('Full rebuild');
  const classes: Record<string, string> = {
    'Metadata review': 'metadata-review-dialog',
    'Library rename': 'metadata-review-dialog library-rename-dialog',
    'Full rebuild': 'confirm-modal',
    'Output preview': 'confirm-modal queue-output-modal',
    'Recovery': 'confirm-modal queue-recovery-modal',
    'Tool repair': 'confirm-modal',
  };
</script>
<div class="app-shell" data-theme="light">
  <main style="padding: 32px">
    <h1>Isolated modal contract fixture</h1>
    <label>Surface<select aria-label="Surface" bind:value={surface} onchange={() => backdrop = false}>{#each Object.keys(classes) as kind}<option>{kind}</option>{/each}</select></label>
    <button bind:this={opener} id="opener" onclick={() => { open = true; acknowledgment = false; busy = false; error = ''; }}>Open review</button>
    <button id="background" onclick={() => backgroundActions++}>Background action</button>
    <label><input type="checkbox" bind:checked={backdrop} /> Allow backdrop dismissal</label>
    <output id="counts">closes={closes}; background={backgroundActions}</output>
    {#if open}
      <Modal {open} class={classes[surface]} labelId="fixture-title" descriptionId="fixture-description" closeOnBackdrop={surface === 'Metadata review' || backdrop} onClose={() => { open = false; closes++; }}>
        <h2 id="fixture-title">Review synthetic operation</h2>
        <p id="fixture-description">No native operation or media is used by this fixture.</p>
        <label><input type="checkbox" bind:checked={acknowledgment} /> I reviewed the operation</label>
        {#if error}<p role="alert">{error}</p>{/if}
        {#if busy}<p role="status">Waiting for synthetic operation</p>{/if}
        <div class="modal-actions">
          <button class="button secondary" onclick={() => open = false}>Cancel</button>
          <button class="button primary" disabled={!acknowledgment || busy} onclick={() => { error = 'Synthetic validation failure'; }}>Confirm</button>
          <button onclick={() => busy = !busy}>Toggle loading</button>
          <button onclick={() => {open = false; setTimeout(() => chainedOpen = true, 0);}}>Continue to next review</button>
        </div>
      </Modal>
    {/if}
    {#if chainedOpen}
      <Modal open={chainedOpen} returnFocus={opener} labelId="chained-title" onClose={() => chainedOpen = false}>
        <h2 id="chained-title">Chained synthetic review</h2>
        <div class="modal-actions"><button class="button secondary" onclick={() => chainedOpen = false}>Back</button></div>
      </Modal>
    {/if}
  </main>
</div>
