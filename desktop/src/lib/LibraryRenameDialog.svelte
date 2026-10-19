<script lang="ts">
  import { ArrowRight, X } from '@lucide/svelte';
  import Modal from './Modal.svelte';
  import type { MediaEntry, OrganizerTemplates } from './types.js';
  import '../styles/components/metadata-review.css';
  import '../styles/components/library-poster-grid.css';
  interface Props {
    libraryRenameOpen: boolean; libraryRenameDialog: HTMLDialogElement | null;
    renameDraft: OrganizerTemplates; moviePatternField: HTMLInputElement | null; episodePatternField: HTMLInputElement | null;
    libraryRenameError: string; libraryRenameBusy: boolean;
    libraryRenameEntries: () => MediaEntry[]; libraryRenameShape: () => 'movie' | 'episode' | 'both';
    libraryRenameNote: (entry: MediaEntry, templates: OrganizerTemplates) => { next: string | null; note: string };
    insertRenameToken: (kind: 'movie' | 'episode', token: string, field: HTMLInputElement | null) => void;
    resetRenamePattern: (kind: 'movie' | 'episode') => void;
    applyLibraryRename: () => unknown; onClose: () => void;
  }
  let {
    libraryRenameOpen, libraryRenameDialog = $bindable(null), renameDraft = $bindable(),
    moviePatternField = $bindable(null), episodePatternField = $bindable(null), libraryRenameError = $bindable(), libraryRenameBusy,
    libraryRenameEntries, libraryRenameShape, libraryRenameNote, insertRenameToken, resetRenamePattern, applyLibraryRename, onClose,
  }: Props = $props();
</script>
<!-- Route owns exact preview/apply IPC and persisted templates; this view owns only the existing dialog rendering. -->
          {#if libraryRenameOpen}
            <Modal open={libraryRenameOpen} class="metadata-review-dialog library-rename-dialog" bind:element={libraryRenameDialog} labelId="library-rename-title" onClose={onClose} initialFocus=".metadata-review-dialog-header button">
              <header class="metadata-review-dialog-header">
                <div class="metadata-review-dialog-title">
                  <span class="eyebrow">RENAME</span>
                  <h2 id="library-rename-title">{libraryRenameEntries().length === 1 ? "Name this file" : `Name ${libraryRenameEntries().length} files`}</h2>
                </div>
                <button class="icon-button" type="button" aria-label="Close rename preview" onclick={() => libraryRenameDialog?.close()}><X size={16} /></button>
              </header>
              <div class="metadata-review-dialog-content">
                {#if !libraryRenameEntries().length}
                  <p>Select the files to rename.</p>
                {:else}
                  {@const shape = libraryRenameShape()}
                  <div class="library-rename-builder">
                    <p>{libraryRenameEntries().length === 1 ? "The file stays in its folder. Only the name changes." : "Every selected file stays in its folder. The same pattern names the batch."}</p>
                    {#if shape !== "episode"}
                      <label class="library-rename-pattern">Film pattern
                        <input bind:this={moviePatternField} bind:value={renameDraft.movie} spellcheck="false" aria-label="Film filename pattern" oninput={() => libraryRenameError = ""} />
                        <span class="library-rename-tokens">
                          <button type="button" class="library-rename-token" onclick={() => insertRenameToken("movie", "{title}", moviePatternField)}>Title</button>
                          <button type="button" class="library-rename-token" onclick={() => insertRenameToken("movie", "{year}", moviePatternField)}>Year</button>
                          <button type="button" class="library-rename-token" onclick={() => insertRenameToken("movie", "{certification}", moviePatternField)}>Certificate</button>
                          <button type="button" class="library-rename-token" onclick={() => insertRenameToken("movie", "{ext}", moviePatternField)}>Extension</button>
                          <button type="button" class="library-rename-token" onclick={() => insertRenameToken("movie", " [[({year} {certification})]]", moviePatternField)}>Year and certificate, when present</button>
                          <button type="button" class="library-rename-token" onclick={() => resetRenamePattern("movie")}>Standard film name</button>
                        </span>
                      </label>
                    {/if}
                    {#if shape !== "movie"}
                      <label class="library-rename-pattern">Episode pattern
                        <input bind:this={episodePatternField} bind:value={renameDraft.episode} spellcheck="false" aria-label="Episode filename pattern" oninput={() => libraryRenameError = ""} />
                        <span class="library-rename-tokens">
                          <button type="button" class="library-rename-token" onclick={() => insertRenameToken("episode", "S{season:02}E{episode:02}", episodePatternField)}>Season and episode</button>
                          <button type="button" class="library-rename-token" onclick={() => insertRenameToken("episode", "{episode_title}", episodePatternField)}>Episode title</button>
                          <button type="button" class="library-rename-token" onclick={() => insertRenameToken("episode", "{series_title}", episodePatternField)}>Series title</button>
                          <button type="button" class="library-rename-token" onclick={() => insertRenameToken("episode", "{ext}", episodePatternField)}>Extension</button>
                          <button type="button" class="library-rename-token" onclick={() => resetRenamePattern("episode")}>Standard episode name</button>
                        </span>
                      </label>
                    {/if}
                  </div>
                  <div class="library-rename-list">
                    {#each libraryRenameEntries() as entry (entry.path)}
                      {@const rename = libraryRenameNote(entry, renameDraft)}
                      <div class="library-rename-row">
                        <div><span>Original name</span><strong>{entry.name}</strong></div>
                        {#if rename.next}
                          <ArrowRight size={16} />
                          <div><span>{rename.note}</span><strong>{rename.next}</strong></div>
                        {:else}
                          <span class="library-rename-same">{rename.note}</span>
                        {/if}
                      </div>
                    {/each}
                  </div>
                {/if}
                {#if libraryRenameError}<p class="library-poster-note">{libraryRenameError}</p>{/if}
              </div>
              <footer class="metadata-review-dialog-footer">
                <span>The video is not re-encoded. A file that already has this name stays where it is.</span>
                <div>
                  <button class="button secondary" type="button" disabled={libraryRenameBusy} onclick={() => libraryRenameDialog?.close()}>Close</button>
                  <button class="button primary" type="button" disabled={libraryRenameBusy || !libraryRenameEntries().length} onclick={() => void applyLibraryRename()}>{libraryRenameBusy ? "Renaming…" : libraryRenameEntries().length === 1 ? "Rename this file" : `Rename ${libraryRenameEntries().length} files`}</button>
                </div>
              </footer>
            </Modal>
          {/if}
