<script lang="ts">
  import { ArrowRight, Database, FileText, Film, FolderOpen, Info, LoaderCircle, RefreshCw, Search, Settings2, Trash2, X } from '@lucide/svelte';
  import type { Snippet } from 'svelte';
  import type { ArtworkCacheInfo, Settings } from './types.js';
  import type { AppPreferences, PreferencesPayload, MetadataSources, TmdbCredentialStatus, TvdbCredentialStatus } from './ui-contracts.js';
  interface Props {
    preferenceStorage: PreferencesPayload | null;
    settings: Settings | null;
    settingsChanged: boolean;
    workflowBusy: boolean;
    appPreferences: AppPreferences;
    metadataLanguageDraft: string;
    tmdbCredentialStatus: TmdbCredentialStatus;
    tvdbCredentialStatus: TvdbCredentialStatus;
    desktopShell: boolean;
    tmdbTokenDraft: string;
    tmdbTokenVisible: boolean;
    tmdbCredentialBusy: boolean;
    tvdbApiKeyDraft: string;
    tvdbCredentialsVisible: boolean;
    tvdbCredentialBusy: boolean;
    tvdbSubscriberPinDraft: string;
    tvdbAccessConfirmed: boolean;
    artworkCacheLoading: boolean;
    artworkCachePurging: boolean;
    artworkCacheInfo: ArtworkCacheInfo | null;
    artworkCacheError: string;
    artworkCacheConfirm: boolean;
    newFilenameIncludePattern: string;
    newFilenameExcludePattern: string;
    filenameFilterError: string;
    newMetadataBlacklistPattern: string;
    metadataBlacklistError: string;
    environmentPanel: Snippet;
    saveSettings: () => unknown;
    persistPreferences: (patch: Partial<AppPreferences>) => unknown;
    saveMetadataLanguageDraft: () => unknown;
    updateMetadataSources: (patch: Partial<MetadataSources>) => unknown;
    openMetadataProviderSettings: (provider?: "tmdb" | "tvdb") => unknown;
    toggleTmdbCredentialVisibility: () => unknown;
    saveTmdbCredential: () => unknown;
    removeTmdbCredential: () => unknown;
    toggleTvdbCredentialVisibility: () => unknown;
    saveTvdbCredential: () => unknown;
    removeTvdbCredential: () => unknown;
    refreshArtworkCache: () => unknown;
    bytes: (value: number | null | undefined) => string;
    purgeUnusedArtworkCache: () => unknown;
    addFilenamePattern: (kind: "include" | "exclude") => unknown;
    clearFilenamePatterns: (kind: "include" | "exclude") => unknown;
    removeFilenamePattern: (kind: "include" | "exclude", index: number) => unknown;
    addMetadataBlacklistPattern: () => unknown;
    clearMetadataBlacklist: () => unknown;
    removeMetadataBlacklistPattern: (index: number) => unknown;
  }
  let {
    preferenceStorage,
    settings = $bindable(),
    settingsChanged,
    workflowBusy,
    appPreferences,
    metadataLanguageDraft = $bindable(),
    tmdbCredentialStatus,
    tvdbCredentialStatus,
    desktopShell,
    tmdbTokenDraft = $bindable(),
    tmdbTokenVisible = $bindable(),
    tmdbCredentialBusy,
    tvdbApiKeyDraft = $bindable(),
    tvdbCredentialsVisible = $bindable(),
    tvdbCredentialBusy,
    tvdbSubscriberPinDraft = $bindable(),
    tvdbAccessConfirmed = $bindable(),
    artworkCacheLoading,
    artworkCachePurging,
    artworkCacheInfo,
    artworkCacheError,
    artworkCacheConfirm = $bindable(),
    newFilenameIncludePattern = $bindable(),
    newFilenameExcludePattern = $bindable(),
    filenameFilterError,
    newMetadataBlacklistPattern = $bindable(),
    metadataBlacklistError,
    environmentPanel,
    saveSettings,
    persistPreferences,
    saveMetadataLanguageDraft,
    updateMetadataSources,
    openMetadataProviderSettings,
    toggleTmdbCredentialVisibility,
    saveTmdbCredential,
    removeTmdbCredential,
    toggleTvdbCredentialVisibility,
    saveTvdbCredential,
    removeTvdbCredential,
    refreshArtworkCache,
    bytes,
    purgeUnusedArtworkCache,
    addFilenamePattern,
    clearFilenamePatterns,
    removeFilenamePattern,
    addMetadataBlacklistPattern,
    clearMetadataBlacklist,
    removeMetadataBlacklistPattern
  }: Props = $props();
</script>
<!-- app.css explicitly scopes settings/provider/credential/cache/filter families; draft bindings are owned by the route, which owns validation, secure storage and native effects. -->
        <section class="page-heading">
          <div><div class="eyebrow">APP-WIDE PREFERENCES</div><h1>Settings</h1><p>Choose the services and defaults the library tools use across the app.</p></div>
          <div class="heading-actions">
            {#if preferenceStorage}<div class="library-chip"><Settings2 size={15} /><span title={preferenceStorage.settingsFile}>{preferenceStorage.portable ? "Portable profile" : "User profile"}</span></div>{/if}
            {#if settings}<button class="button primary" type="button" disabled={!settingsChanged || workflowBusy} onclick={saveSettings}>{settingsChanged ? "Save app defaults" : "Defaults saved"}</button>{/if}
          </div>
        </section>

        <section class="settings-card metadata-provider-settings" aria-labelledby="metadata-provider-title">
          <div class="settings-card-heading">
            <div class="settings-icon purple"><Database size={18} /></div>
            <div>
              <h2 id="metadata-provider-title">Metadata sources</h2>
              <p>Choose where film and series details come from. Results stay attached to a scanned record until you review them.</p>
            </div>
          </div>

          <label class="toggle-row metadata-auto-lookup">
            <span><strong>Find titles after a scan</strong><small>Searches enabled public sources by default when no Kodi NFO is present. Exact title-and-year matches with no provider conflict are associated automatically; uncertain matches wait for review. NFO metadata remains a local fallback.</small></span>
            <input type="checkbox" checked={appPreferences.metadataAutoLookup} onchange={(event) => void persistPreferences({ metadataAutoLookup: event.currentTarget.checked })} />
          </label>

          <label class="form-row metadata-language-setting">
            <span><strong>Metadata language and region</strong><small>TMDB and Wikidata use localized fields when available. TheTVDB supports selected languages. TVmaze text follows its contributors.</small></span>
            <input aria-label="Metadata language and region" list="metadata-language-options" maxlength="5" pattern="[A-Za-z]{2}(-[A-Za-z]{2})?" value={metadataLanguageDraft} placeholder="en-US" oninput={(event) => metadataLanguageDraft = event.currentTarget.value} onblur={saveMetadataLanguageDraft} onkeydown={(event) => { if (event.key === "Enter") { event.preventDefault(); saveMetadataLanguageDraft(); } }} />
            <datalist id="metadata-language-options">
              <option value="en-US">English · United States</option><option value="en-GB">English · United Kingdom</option><option value="es-ES">Spanish · Spain</option><option value="es-MX">Spanish · Mexico</option><option value="fr-FR">French · France</option><option value="fr-CA">French · Canada</option><option value="de-DE">German · Germany</option><option value="it-IT">Italian · Italy</option><option value="pt-BR">Portuguese · Brazil</option><option value="pt-PT">Portuguese · Portugal</option><option value="ja-JP">Japanese · Japan</option><option value="ko-KR">Korean · South Korea</option><option value="zh-CN">Chinese · Simplified</option><option value="zh-TW">Chinese · Taiwan</option><option value="ru-RU">Russian · Russia</option><option value="nl-NL">Dutch · Netherlands</option><option value="ar-SA">Arabic · Saudi Arabia</option><option value="hi-IN">Hindi · India</option><option value="tr-TR">Turkish · Türkiye</option><option value="pl-PL">Polish · Poland</option><option value="sv-SE">Swedish · Sweden</option><option value="da-DK">Danish · Denmark</option><option value="fi-FI">Finnish · Finland</option><option value="no-NO">Norwegian · Norway</option><option value="cs-CZ">Czech · Czechia</option><option value="el-GR">Greek · Greece</option>
            </datalist>
          </label>

          <div class="metadata-source-grid" aria-label="Available metadata providers">
            <article class="metadata-provider-card" class:provider-selected={appPreferences.metadataSources.tmdb}>
              <div class="metadata-provider-card-top">
                <label class="metadata-provider-choice"><input type="checkbox" checked={appPreferences.metadataSources.tmdb} onchange={(event) => updateMetadataSources({ tmdb: event.currentTarget.checked })} /><strong>TMDB</strong></label>
                <span class="provider-state" class:provider-state-ready={tmdbCredentialStatus.configured} class:provider-state-needed={!tmdbCredentialStatus.configured}>{tmdbCredentialStatus.configured ? "Token saved" : "Token required"}</span>
              </div>
              <p>Movies and series, localized details, episode guides, ratings, and artwork.</p>
              {#if !tmdbCredentialStatus.configured}<button class="provider-setup-link" type="button" onclick={() => void openMetadataProviderSettings("tmdb")}>Add TMDB token <ArrowRight size={14} /></button>{/if}
            </article>

            <article class="metadata-provider-card" class:provider-selected={appPreferences.metadataSources.wikidata}>
              <div class="metadata-provider-card-top">
                <label class="metadata-provider-choice"><input type="checkbox" checked={appPreferences.metadataSources.wikidata} onchange={(event) => updateMetadataSources({ wikidata: event.currentTarget.checked })} /><strong>Wikidata</strong></label>
                <span class="provider-state provider-state-open">Open catalog</span>
              </div>
              <p>Movie titles, release years, language-aware labels, and links to related public records. Coverage varies by film.</p>
            </article>

            <article class="metadata-provider-card" class:provider-selected={appPreferences.metadataSources.tvmaze}>
              <div class="metadata-provider-card-top">
                <label class="metadata-provider-choice"><input type="checkbox" checked={appPreferences.metadataSources.tvmaze} onchange={(event) => updateMetadataSources({ tvmaze: event.currentTarget.checked })} /><strong>TVmaze</strong></label>
                <span class="provider-state provider-state-open">No key needed</span>
              </div>
              <p>TV series, seasons, episodes, audience ratings, and artwork. Descriptions use the language supplied by contributors.</p>
            </article>

            <article class="metadata-provider-card" class:provider-selected={appPreferences.metadataSources.tvdb}>
              <div class="metadata-provider-card-top">
                <label class="metadata-provider-choice"><input type="checkbox" checked={appPreferences.metadataSources.tvdb} onchange={(event) => updateMetadataSources({ tvdb: event.currentTarget.checked })} /><strong>TheTVDB</strong></label>
                <span class="provider-state" class:provider-state-ready={tvdbCredentialStatus.configured} class:provider-state-needed={!tvdbCredentialStatus.configured}>{tvdbCredentialStatus.configured ? "Key saved" : "Project key required"}</span>
              </div>
              <p>TV and movie records, posters, and the provider’s default season order. Anime may use different aired, DVD, or absolute episode numbering.</p>
              {#if !tvdbCredentialStatus.configured}<button class="provider-setup-link" type="button" onclick={() => void openMetadataProviderSettings("tvdb")}>Set up TheTVDB access <ArrowRight size={14} /></button>{/if}
            </article>
          </div>

          <section class="metadata-credential-card" id="tmdb-credential-settings" aria-labelledby="tmdb-credential-title">
            <div class="metadata-credential-heading">
              <div><strong id="tmdb-credential-title">TMDB API Read Access Token</strong><small>{tmdbCredentialStatus.configured ? "Saved in this user’s operating-system keyring." : desktopShell ? "Add a token to include TMDB results in searches." : "Open the installed desktop app to save this token securely."}</small></div>
              <span class="status-tag" class:status-ready={tmdbCredentialStatus.configured} class:status-warning={!desktopShell} class:status-error={desktopShell && !tmdbCredentialStatus.secureStoreAvailable}>{tmdbCredentialStatus.configured ? "TOKEN SAVED" : desktopShell ? tmdbCredentialStatus.secureStoreAvailable ? "SETUP NEEDED" : "KEYRING UNAVAILABLE" : "DESKTOP APP"}</span>
            </div>
            {#if tmdbCredentialStatus.message}<p class="metadata-credential-message" role="status">{tmdbCredentialStatus.message}</p>{/if}
            <label class="metadata-token" for="tmdb-api-token">{tmdbCredentialStatus.configured ? "Saved token or replacement" : "API Read Access Token"}
              <input id="tmdb-api-token" aria-label="TMDB API Read Access Token" bind:value={tmdbTokenDraft} type={tmdbTokenVisible ? "text" : "password"} autocomplete="new-password" autocapitalize="off" spellcheck="false" placeholder={tmdbCredentialStatus.configured ? "Use Show saved token to view the keyring value" : "Paste the TMDB API Read Access Token"} disabled={!desktopShell || !tmdbCredentialStatus.secureStoreAvailable || tmdbCredentialBusy} />
            </label>
            <div class="metadata-credential-actions">
              <a class="text-button" href="https://www.themoviedb.org/settings/api" target="_blank" rel="noreferrer">Get a TMDB API token <ArrowRight size={14} /></a>
              <div>{#if tmdbCredentialStatus.configured || tmdbTokenDraft}<button class="button secondary" aria-pressed={tmdbTokenVisible} disabled={tmdbCredentialBusy} onclick={toggleTmdbCredentialVisibility}>{tmdbTokenDraft ? tmdbTokenVisible ? "Hide token" : "Show token" : "Show saved token"}</button>{/if}<button class="button primary" disabled={!desktopShell || !tmdbCredentialStatus.secureStoreAvailable || !tmdbTokenDraft.trim() || tmdbCredentialBusy} onclick={saveTmdbCredential}>{tmdbCredentialBusy ? "Saving…" : tmdbCredentialStatus.configured ? "Replace token" : "Save token"}</button>{#if tmdbCredentialStatus.configured}<button class="button secondary" disabled={tmdbCredentialBusy} onclick={removeTmdbCredential}>Remove</button>{/if}</div>
            </div>
            <small class="metadata-credential-note">The token stays in the operating-system keyring. Save keeps it visible in this window; after reopening the app, choose Show saved token to read it back. It is never written to portable settings, browser storage, logs, CLI output, or MCP.</small>
          </section>

          <section class="metadata-credential-card tvdb-credential-card" id="tvdb-credential-settings" aria-labelledby="tvdb-credential-title">
            <div class="metadata-credential-heading">
              <div><strong id="tvdb-credential-title">TheTVDB project access</strong><small>{tvdbCredentialStatus.configured ? `Project key saved${tvdbCredentialStatus.subscriberPinConfigured ? " with subscriber PIN" : ""} in this user’s operating-system keyring.` : desktopShell ? "Enter a project API key before TheTVDB can be searched." : "Open the installed desktop app to save credentials securely."}</small></div>
              <span class="status-tag" class:status-ready={tvdbCredentialStatus.configured} class:status-warning={!desktopShell} class:status-error={desktopShell && !tvdbCredentialStatus.secureStoreAvailable}>{tvdbCredentialStatus.configured ? "KEY SAVED" : desktopShell ? tvdbCredentialStatus.secureStoreAvailable ? "SETUP NEEDED" : "KEYRING UNAVAILABLE" : "DESKTOP APP"}</span>
            </div>
            {#if tvdbCredentialStatus.message}<p class="metadata-credential-message" role="status">{tvdbCredentialStatus.message}</p>{/if}
            <p class="provider-access-copy">The key belongs to a TheTVDB API project. Direct access requires an approved project access model; subscriber-supported keys may also require your personal PIN. Check current terms with TheTVDB before saving a key.</p>
            <div class="metadata-credential-fields">
              <label class="metadata-token" for="tvdb-project-key">{tvdbCredentialStatus.configured ? "Replace project API key" : "Project API key"}
                <input id="tvdb-project-key" aria-label="TheTVDB project API key" bind:value={tvdbApiKeyDraft} type={tvdbCredentialsVisible ? "text" : "password"} autocomplete="new-password" autocapitalize="off" spellcheck="false" placeholder="Paste your TheTVDB project API key" disabled={!desktopShell || !tvdbCredentialStatus.secureStoreAvailable || tvdbCredentialBusy} />
              </label>
              <label class="metadata-token" for="tvdb-subscriber-pin">Subscriber PIN <span>(if your key requires one)</span>
                <input id="tvdb-subscriber-pin" aria-label="TheTVDB subscriber PIN" bind:value={tvdbSubscriberPinDraft} type={tvdbCredentialsVisible ? "text" : "password"} autocomplete="new-password" autocapitalize="off" spellcheck="false" placeholder="Optional subscriber PIN" disabled={!desktopShell || !tvdbCredentialStatus.secureStoreAvailable || tvdbCredentialBusy} />
              </label>
            </div>
            <label class="provider-access-confirm"><input type="checkbox" bind:checked={tvdbAccessConfirmed} disabled={!desktopShell || !tvdbCredentialStatus.secureStoreAvailable || tvdbCredentialBusy} /><span>I have valid TheTVDB access for this project and any subscription required by its access model.</span></label>
            <div class="metadata-credential-actions">
              <div class="metadata-provider-links">
                <a href="https://www.thetvdb.com/api-information/signup" target="_blank" rel="noreferrer">Apply for API access <ArrowRight size={14} /></a>
                <a href="https://github.com/thetvdb/v4-api/blob/main/README.md" target="_blank" rel="noreferrer">Read API access terms <ArrowRight size={14} /></a>
              </div>
              <div>{#if tvdbCredentialStatus.configured || tvdbApiKeyDraft || tvdbSubscriberPinDraft}<button class="button secondary" aria-pressed={tvdbCredentialsVisible} disabled={tvdbCredentialBusy} onclick={toggleTvdbCredentialVisibility}>{tvdbApiKeyDraft || tvdbSubscriberPinDraft ? tvdbCredentialsVisible ? "Hide credentials" : "Show credentials" : "Show saved credentials"}</button>{/if}<button class="button primary" disabled={!desktopShell || !tvdbCredentialStatus.secureStoreAvailable || !tvdbApiKeyDraft.trim() || !tvdbAccessConfirmed || tvdbCredentialBusy} onclick={saveTvdbCredential}>{tvdbCredentialBusy ? "Saving…" : tvdbCredentialStatus.configured ? "Replace credentials" : "Save credentials"}</button>{#if tvdbCredentialStatus.configured}<button class="button secondary" disabled={tvdbCredentialBusy} onclick={removeTvdbCredential}>Remove</button>{/if}</div>
            </div>
            <small class="metadata-credential-note">The project key and optional PIN stay in the operating-system keyring. Save keeps them visible in this window; after reopening the app, choose Show saved credentials to read them back. They are never written to app preferences, logs, CLI output, or MCP.</small>
          </section>

          <details class="metadata-provider-notes">
            <summary>Provider coverage, anime order, and local use</summary>
            <p>TMDB and TheTVDB cover films and series. Wikidata adds public film records. TVmaze covers series and episodes and needs no key. Results stay suggestions until you review them.</p>
            <p>TheTVDB is read in its default season order. Check anime episode numbers against your files before you assign them. AniList is not used; its terms forbid storing its data as a backup.</p>
          </details>
        </section>

        <section class="settings-card poster-storage-settings" aria-labelledby="poster-storage-title">
          <div class="settings-card-heading"><div class="settings-icon green-bg"><Film size={18} /></div><div><h2 id="poster-storage-title">Poster storage</h2><p>Choose where a reviewed poster image is kept.</p></div></div>
          <label class="toggle-row"><span><strong>Save poster beside the media</strong><small>Uses <code>[media filename]-poster.[jpg|png|webp]</code> and stores source details beside it. Turn off to keep posters in the private app cache.</small></span><input type="checkbox" checked={appPreferences.savePosterBesideMedia} onchange={(event) => void persistPreferences({ savePosterBesideMedia: event.currentTarget.checked })} /></label>
          {#if settings}
            <label class="toggle-row"><span><strong>Embed poster in the new MKV</strong><small>Attach the selected image as cover art during a metadata-only copy or video rebuild. It does not change the original file.</small></span><input type="checkbox" checked={settings.metadata.embed_artwork} disabled={workflowBusy} onchange={(event) => settings!.metadata.embed_artwork = event.currentTarget.checked} /></label>
          {:else}
            <p class="filename-filter-saved">Load a library’s effective settings to choose whether its new MKV outputs include embedded cover art.</p>
          {/if}
          <div class="artwork-settings-save"><p class="filename-filter-saved">The two choices are independent. Save app defaults to keep embedding for new libraries.</p></div>
        </section>

        <section class="settings-card artwork-cache-settings" aria-labelledby="artwork-cache-title">
          <div class="settings-card-heading"><div class="settings-icon purple"><Database size={18} /></div><div><h2 id="artwork-cache-title">Saved poster artwork</h2><p>Check the private poster cache and remove unused images when you need the space.</p></div><button class="button secondary compact" type="button" disabled={!desktopShell || artworkCacheLoading || artworkCachePurging} onclick={() => void refreshArtworkCache()}><RefreshCw size={14} class={artworkCacheLoading ? "spin" : ""} /> Refresh</button></div>
          {#if !desktopShell}
            <div class="artwork-cache-note"><Info size={16} /><span>Cache details are available from the installed desktop app. The browser preview does not read local application files.</span></div>
          {:else if artworkCacheLoading && !artworkCacheInfo}
            <div class="artwork-cache-note" role="status"><LoaderCircle size={16} class="spin" /><span>Checking saved poster images…</span></div>
          {:else if artworkCacheInfo}
            <div class="artwork-cache-summary">
              <div><span>IN USE</span><strong>{bytes(artworkCacheInfo.totalBytes)}</strong><small>{artworkCacheInfo.totalFileCount} files · {artworkCacheInfo.imageCount} managed posters</small></div>
              <div><span>ASSIGNED TO QUEUE ITEMS</span><strong>{bytes(artworkCacheInfo.protectedBytes)}</strong><small>{artworkCacheInfo.protectedImageCount} posters kept with saved reviews</small></div>
              <div><span>UNUSED POSTERS</span><strong>{bytes(artworkCacheInfo.purgeableBytes)}</strong><small>{artworkCacheInfo.purgeableImageCount} managed posters can be removed</small></div>
            </div>
            {#if artworkCacheInfo.unmanagedEntryCount}<p class="artwork-cache-note"><Info size={16} /><span>{artworkCacheInfo.unmanagedEntryCount} unrecognized cache entr{artworkCacheInfo.unmanagedEntryCount === 1 ? "y is" : "ies are"} counted where possible and will be left untouched.</span></p>{/if}
            <p class="filename-filter-saved">Only unused posters and their credit files can be removed. Artwork assigned to the queue stays. This does not clear the x265 calibration cache; clearing that cache makes large-file jobs run a new short benchmark.</p>
            {#if artworkCacheError}<p class="blacklist-error" role="alert">Artwork cache: {artworkCacheError}</p>{/if}
            {#if artworkCacheInfo.purgeableImageCount && !artworkCacheConfirm}
              <button class="button secondary artwork-cache-review" type="button" disabled={artworkCachePurging || workflowBusy} onclick={() => artworkCacheConfirm = true}><Trash2 size={15} /> Review unused posters</button>
            {:else if artworkCacheConfirm}
              <div class="artwork-cache-confirm" role="group" aria-label="Confirm artwork cleanup">
                <div><strong>Remove {artworkCacheInfo.purgeableImageCount} unused poster{artworkCacheInfo.purgeableImageCount === 1 ? "" : "s"}?</strong><span>This permanently removes up to {bytes(artworkCacheInfo.purgeableBytes)} of app-managed artwork and its matching attribution files. Assigned posters and unrecognized files stay in place.</span></div>
                <button class="button danger-outline compact" type="button" disabled={artworkCachePurging || workflowBusy} onclick={() => void purgeUnusedArtworkCache()}>{#if artworkCachePurging}<LoaderCircle size={14} class="spin" /> Removing…{:else}<Trash2 size={14} /> Remove unused posters{/if}</button>
                <button class="button secondary compact" type="button" disabled={artworkCachePurging} onclick={() => artworkCacheConfirm = false}>Cancel</button>
              </div>
            {:else if artworkCacheInfo.purgeableImageCount === 0}
              <p class="blacklist-empty">No unused app-managed poster images to remove.</p>
            {/if}
          {:else if artworkCacheError}
            <p class="blacklist-error" role="alert">Could not read the artwork cache: {artworkCacheError}</p>
          {/if}
        </section>

        <section class="settings-card filename-filter-settings" aria-labelledby="filename-filter-title">
          <div class="settings-card-heading"><div class="settings-icon purple"><Search size={18} /></div><div><h2 id="filename-filter-title">Filename filters</h2><p>Limit scans and jobs using file and folder names.</p></div></div>
          <p class="filename-filter-help">Ignores case. <code>*</code> matches any length and <code>?</code> matches one character. With no include patterns, every supported name is scanned. Excludes override includes and match files and folders. The starting chips skip extras, samples, trailers, and hidden folders; remove a chip to scan that name. App state folders stay excluded.</p>
          <div class="filename-filter-columns">
            {#each [{ kind: "include", title: "Include names", hint: "Only names matching at least one pattern are scanned." }, { kind: "exclude", title: "Exclude names", hint: "Names matching any pattern are skipped." }] as filter (filter.kind)}
              {@const kind = filter.kind as "include" | "exclude"}
              {@const key = kind === "include" ? "filenameIncludePatterns" : "filenameExcludePatterns"}
              {@const draft = kind === "include" ? newFilenameIncludePattern : newFilenameExcludePattern}
              <div class="filename-filter-list">
                <div class="filename-filter-heading"><strong>{filter.title}</strong><span>{appPreferences[key].length}/100</span></div>
                <small>{filter.hint}</small>
                <div class="blacklist-add-row"><input aria-label={`New ${filter.title.toLowerCase()} pattern`} maxlength="128" placeholder={kind === "include" ? "*.mkv or *featurette*" : "*sample*"} value={draft} oninput={(event) => kind === "include" ? newFilenameIncludePattern = event.currentTarget.value : newFilenameExcludePattern = event.currentTarget.value} onkeydown={(event) => { if (event.key === "Enter") { event.preventDefault(); addFilenamePattern(kind); } }} /><button class="button secondary" type="button" disabled={appPreferences[key].length >= 100} onclick={() => addFilenamePattern(kind)}>Add</button></div>
                {#if appPreferences[key].length}
                  <div class="blacklist-list-actions"><span>{appPreferences[key].length} active rule{appPreferences[key].length === 1 ? "" : "s"}{appPreferences[key].length > 20 ? " · scroll to see all" : ""}</span><button class="text-button" type="button" onclick={() => clearFilenamePatterns(kind)}>Clear all</button></div>
                  <ul class="blacklist-pattern-list">{#each appPreferences[key] as pattern, patternIndex (`${kind}-${pattern}-${patternIndex}`)}<li><code>{pattern}</code><button class="blacklist-remove" type="button" aria-label={`Remove ${kind} pattern ${pattern}`} title="Remove pattern" onclick={() => removeFilenamePattern(kind, patternIndex)}><X size={16} /></button></li>{/each}</ul>
                {:else}<p class="blacklist-empty">No {kind} patterns configured.</p>{/if}
              </div>
            {/each}
          </div>
          {#if filenameFilterError}<p class="blacklist-error" role="status">{filenameFilterError}</p>{/if}
          <p class="filename-filter-saved">Saved with application preferences{preferenceStorage ? ` · ${preferenceStorage.settingsFile}` : !desktopShell ? " · browser preview preferences saved in this browser" : ""}. Changing a pattern requires a fresh scan.</p>
        </section>

        <section class="settings-card filename-filter-settings title-cleanup-settings" aria-labelledby="title-cleanup-title">
          <div class="settings-card-heading"><div class="settings-icon purple"><FileText size={18} /></div><div><h2 id="title-cleanup-title">Title cleanup</h2><p>Clean imported filename clues before title searches and rename previews.</p></div></div>
          {#if settings}
            <div class="metadata-blacklist">
              <div class="blacklist-heading"><div><strong>Blocked text patterns</strong><small>Removes matched text from title searches, rename previews, and output tags. The file on disk changes only when you confirm a rename.</small></div><span>{settings.metadata.text_blacklist.length}/100</span></div>
              <p class="blacklist-help"><code>*</code> matches any length, ignoring case. <code>Movie*Group</code> removes text such as <code>movieReleaseGroup</code> and leaves the rest of the title.</p>
              <div class="blacklist-add-row"><input aria-label="New blocked text pattern" maxlength="128" placeholder="Add a pattern, e.g. Movie*Group" bind:value={newMetadataBlacklistPattern} onkeydown={(event) => { if (event.key === "Enter") { event.preventDefault(); addMetadataBlacklistPattern(); } }} /><button class="button secondary" type="button" disabled={settings.metadata.text_blacklist.length >= 100 || workflowBusy} onclick={addMetadataBlacklistPattern}>Add pattern</button></div>
              {#if metadataBlacklistError}<p class="blacklist-error" role="status">{metadataBlacklistError}</p>{/if}
              {#if settings.metadata.text_blacklist.length}
                <div class="blacklist-list-actions"><span>{settings.metadata.text_blacklist.length} active rule{settings.metadata.text_blacklist.length === 1 ? "" : "s"}{settings.metadata.text_blacklist.length > 20 ? " · scroll to see all" : ""}</span><button class="text-button" type="button" disabled={workflowBusy} onclick={clearMetadataBlacklist}>Clear all</button></div>
                <ul class="blacklist-pattern-list">{#each settings.metadata.text_blacklist as pattern, patternIndex (`title-${pattern}-${patternIndex}`)}<li><code>{pattern}</code><button class="blacklist-remove" type="button" aria-label={`Remove ${pattern}`} title="Remove pattern" disabled={workflowBusy} onclick={() => removeMetadataBlacklistPattern(patternIndex)}><X size={15} /></button></li>{/each}</ul>
              {:else}
                <p class="blacklist-empty">No custom patterns yet. Built-in release-group cleanup still runs.</p>
              {/if}
              <p class="filename-filter-saved">Extra and trailer folders are controlled under Filename filters. <code>Movie*Group</code> is the default pattern here. Save app defaults to keep these edits.</p>
            </div>
          {:else}
            <p class="filename-filter-saved">Load application defaults to edit blocked text patterns.</p>
          {/if}
        </section>
        {@render environmentPanel()}
