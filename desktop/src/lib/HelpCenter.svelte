<script lang="ts">
  import { BookOpen, Check, Copy, ExternalLink, FolderOpen, HelpCircle, Layers, Play, Settings2, ShieldCheck, Video } from "@lucide/svelte";

  let copied = $state("");

  const mcpConfig = JSON.stringify({
    mcpServers: {
      "hevc-rebuild-studio": {
        command: "/absolute/path/to/hevc-rebuild-studio",
        args: ["--mcp"],
      },
    },
  }, null, 2);

  async function copyText(value: string, label: string) {
    try {
      await navigator.clipboard.writeText(value);
      copied = `${label} copied`;
    } catch {
      copied = "Clipboard access is unavailable. Select and copy the text instead.";
    }
  }
</script>

<section class="help-page">
  <header class="page-heading">
    <div>
      <div class="eyebrow">HEVC REBUILD STUDIO</div>
      <h1>Help & <em>commands.</em></h1>
      <p>Instructions for scanning, editing, renaming, rebuilding, and using the command-line and MCP interfaces.</p>
    </div>
  </header>

  {#if copied}<div class="help-copy-status" role="status">{copied}</div>{/if}

  <section class="help-grid" aria-label="Application guide">
    <article class="help-card">
      <div class="help-card-icon"><FolderOpen size={18} /></div>
      <div class="eyebrow">START HERE</div>
      <h2>Build your media queue</h2>
      <ol>
        <li>Start on <strong>Library</strong>. Drop files or folders, or use Add folder / Add files. Each source is scanned into the same queue. Toolchain status lives in the top bar and in Settings.</li>
        <li>Scanning starts as each source is added. The app reads tracks, tags, HDR data, and resource estimates; it does not change the media. <strong>Stop scan</strong> finishes the file currently being read, then returns. Files already analyzed stay in the queue.</li>
        <li>Use the <strong>Queue</strong> manager to add or remove folders and files. Drag a handle, or use the arrow buttons, to change source and file order. Removing a file hides it on refresh until you restore hidden items.</li>
        <li>If FFprobe is installed but MediaInfo is missing, scans can still run with partial track details. Rebuild and YouTube jobs stay blocked until their required tools are ready.</li>
        <li>Search narrows the active source. Large results are shown in pages of 50, 100, or 200 rows; the header checkbox selects the current page, while <strong>Select all matches</strong> includes every filtered result. Your selection remains shared across the queue.</li>
        <li>Use the checkboxes to select files from different folders. The selection follows you between the media list and metadata workbench. Jobs use selected files, or new and queued files for the chosen task when nothing is selected. Completed, failed, skipped, interrupted, or needs-review files stay out of that task’s default run until you select them.</li>
        <li>Open a file’s track details to choose video, audio, subtitle, and attachment streams.</li>
        <li>The queue status strip counts files that are ready, queued, scanning, renaming, copying streams, rebuilding, validating, complete, skipped, failed, or waiting for review.</li>
        <li>The file row keeps its memory-risk level and planned worker count visible. Open <strong>Memory and retry details</strong> for the budget, swap pressure, and lower-thread retry plan.</li>
        <li>After scanning, choose <strong>Rename files</strong>, <strong>Organize titles</strong>, <strong>Metadata-only MKV</strong>, or <strong>Rebuild video</strong> from Library. Rename files names one selected file or the whole selection from a pattern, and shows the original name beside the new name before anything moves. The organizer still renames series and season folders. Processing keeps the shared selection across queued folders.</li>
        <li>Check only and integrity checks run in the native pipeline and write no media. Short tests and rebuilds are on <strong>Run</strong>. Before metadata-only output, a short test, or a full rebuild, review destinations, collisions, and disk space in one dialog. Existing outputs or duplicate destinations block the run.</li>
        <li>After a restart, the app settles files that were still active. A saved successful result is marked complete. An unfinished file whose original is still in place returns to the normal queue, and only an unopened temporary encode from this app is removed. A partial encode is not resumed, and an existing output is not overwritten. A file that is missing, or whose saved success has no output, stays for review. Select failed or skipped items to prepare a retry; mark queued or failed items skipped to keep them out of the default run.</li>
      </ol>
      <p class="help-note"><ShieldCheck size={15} /><span>A full rebuild keeps the source until its replacement passes validation. If compression finishes but a later check fails, the completed file is retained as an unvalidated <code>.error.mkv</code> review artifact beside a matching <code>.error.log</code>; an in-place rebuild never overwrites the source early. Stop safely finishes the current file and then stops the full multi-folder queue.</span></p>
    </article>

    <article class="help-card">
      <div class="help-card-icon"><Layers size={18} /></div>
      <div class="eyebrow">WHERE THINGS LIVE</div>
      <h2>Find the right screen</h2>
      <dl class="help-map">
        <div><dt>Library</dt><dd>The start screen. Add folders or files to the persistent queue, inspect posters and tracks, and start rename, metadata-only, integrity, or rebuild work. An empty queue opens here.</dd></div>
        <div><dt>Organize titles</dt><dd>A library view for batch title matches, film or series details, artwork, and filename previews. Open it from Organize titles on the poster bar.</dd></div>
        <div><dt>YouTube</dt><dd>Choose a YouTube profile and separate output folder; review HDR and size warnings.</dd></div>
        <div><dt>Run</dt><dd>Choose a job, including native Check only and integrity checks. Encoder settings appear only for short tests and full rebuilds. Review destinations before anything is written.</dd></div>
        <div><dt>Settings</dt><dd>Tool paths, metadata sources, filename filters, and title cleanup.</dd></div>
        <div><dt>Reports & errors</dt><dd>See each file’s success or failure, with the matching log.</dd></div>
        <div><dt>Help & commands</dt><dd>Opened from the question mark in the top bar. Same safety rules as the GUI, plus CLI and <code>--mcp</code>.</dd></div>
      </dl>
    </article>

    <article class="help-card help-card-wide">
      <div class="help-card-icon"><Layers size={18} /></div>
      <div class="eyebrow">RENAME WITHOUT REBUILDING</div>
      <h2>Preview names, then apply them</h2>
      <ol>
        <li>In <strong>Library</strong>, review the proposed filename under each matched movie or episode. Public title suggestions stay marked as candidates until you review them.</li>
        <li>Open <strong>Title and poster</strong> on the file row to review that exact film or its associated TV series. The review dialog keeps the match and poster attached to the queue item.</li>
        <li>For a single file, use <strong>Preview filename change</strong> in that dialog. Check the exact path and confirm the rename; a focused TV review can rename only the selected episode.</li>
        <li>For batch rename, choose <strong>Open full organizer</strong>. It reads names and nearby metadata; it does not encode video. A complete folder scan is needed before renaming a whole series.</li>
        <li>For each film or episode, review the title and any match. Edit the name or metadata fields when needed.</li>
        <li>Choose <strong>Preview rename</strong>. Check every destination and resolve any collision before continuing.</li>
        <li>Apply the current preview only after the paths look right. Matching NFO and poster attribution files move with their video.</li>
      </ol>
      <p>Rename is a separate task from rebuilding. The current organizer does not rename unmatched files from guesses, and its error rollback does not yet provide a persistent undo history.</p>
    </article>

    <article class="help-card help-card-wide">
      <div class="help-card-icon"><Video size={18} /></div>
      <div class="eyebrow">MOVIES, SHOWS, AND ARTWORK</div>
      <h2>Edit details on the scanned item</h2>
      <ol>
        <li>In <strong>Library</strong>, open the title and poster review on the film or an episode from the show. This opens the focused review for that record.</li>
        <li>The app checks nearby poster artwork first. With automatic suggestions enabled, it searches the enabled public sources, sending only the cleaned title phrase; the year is used locally to rank results. Existing NFO files are ignored.</li>
        <li>Edit the title, year, certification, descriptions, genres, rating, and poster choice for the specific film or show.</li>
        <li>Reviewed movie and series details, provider poster choices, and episode-to-file matches stay with the saved queue. The dialog's <strong>Open full organizer</strong> button opens batch naming tools.</li>
        <li>Preview rename targets. Confirm each write only after checking the paths and values.</li>
      </ol>
      <p>Open <strong>Local artwork</strong> on a film or show to preview images already beside it. This is a metadata preview, not media playback. Choose a poster for an individual film or series, or for a TV season; the selected path is saved with the queue and shown on library cards. In a movie folder, one existing local image can also be applied to the selected, already-assigned films after reviewing the affected filenames. That only changes the saved poster choices; it does not copy or replace image files. Metadata-only, short-test, and full-rebuild outputs use the reviewed image for the output cover. The image must remain inside the scanned library. A provider poster must first be saved locally; check its source and reuse terms before choosing it.</p>
    </article>

    <article class="help-card help-card-wide">
      <div class="help-card-icon"><ShieldCheck size={18} /></div>
      <div class="eyebrow">NO VIDEO RE-ENCODING</div>
      <h2>Update tags and artwork in a new MKV</h2>
      <ol>
        <li>Review the title, tags, and poster on each movie or show from its card in Library.</li>
        <li>Open <strong>Organize titles</strong> from Library, select the intended files, and apply any confirmed rename first. Rename changes paths only; it does not alter media contents.</li>
        <li>Choose <strong>Review MKV output plan</strong> in Organize titles. This carries only the selected organizer files into Run with Metadata-only MKV selected. You can also start this mode from the Library task bar.</li>
        <li>Review the queue-wide destination and collision list. Choose an output folder if needed; otherwise, the app creates a sibling file ending in <code>.metadata.mkv</code>.</li>
        <li>Start the task. The app remuxes every source track, updates the reviewed tags and selected cover, checks the output, and leaves the original untouched.</li>
      </ol>
      <p>The operation does not run a video encoder. Existing output files are never replaced. Unsupported streams or failed validation stop that file and leave its source in place. The tagged MKV copy embeds the reviewed fields in the output itself; no NFO sidecars are read or written.</p>
    </article>

    <article class="help-card help-card-wide">
      <div class="help-card-icon"><Settings2 size={18} /></div>
      <div class="eyebrow">SETTINGS · NAME RULES</div>
      <h2>Choose what to skip and what to clean</h2>
      <p>In <strong>Settings → Filename filters</strong>, add one include or exclude pattern per row. Matching ignores case. <code>*</code> matches any length and <code>?</code> matches one character. With no include patterns, every supported name is scanned. Excludes override includes. The starting rules skip extras, samples, trailers, and hidden folders; remove a rule or clear the list to scan those names again.</p>
      <p>In <strong>Settings → Title cleanup</strong>, add blocked-text patterns such as <code>Movie*Group</code>. They run automatically when imported filenames are parsed for title matching, before public lookup. They also clean rename previews and readable output tags, track names, and chapter titles. Only matching text is removed. Remove a rule with its own button or use <strong>Clear all</strong> for the custom list. The original media filename remains unchanged until you confirm a rename.</p>
    </article>

    <article class="help-card help-card-wide">
      <div class="help-card-icon"><Play size={18} /></div>
      <div class="eyebrow">LIVE RUN STATUS</div>
      <h2>Follow each file while it runs</h2>
      <p>The queue status strip and each file row show the current operation stage: scanning, renaming, copying streams, rebuilding, or validating. Run activity reports the current file, percent complete, encoding speed, and estimated time remaining. It also shows source size, bytes written so far, and a live output-size estimate once enough frames have been processed. That estimate can move as the video changes; the finished report records the actual output size.</p>
      <p>Resource telemetry updates separately with memory, swap, CPU, and disk readings. Use <strong>Stop after this file</strong> to finish the current encode, then stop before the next one. That is the same as <strong>Stop safely</strong>: the current file completes. <strong>Stop now</strong> kills the engine immediately, keeps the source, and does not resume that encode.</p>
    </article>

    <article class="help-card help-card-wide">
      <div class="help-card-icon"><FolderOpen size={18} /></div>
      <div class="eyebrow">OUTPUT DESTINATIONS</div>
      <h2>Choose where rebuilt files go</h2>
      <ol>
        <li>Set a default destination in <strong>Run → Output folders</strong>. The app mirrors the input folder structure there.</li>
        <li>To send one file somewhere else, open <strong>Library</strong> and choose <strong>Set output folder</strong> on its card. That choice overrides the default for that file.</li>
        <li>Before starting a metadata copy, short test, or full rebuild, review the queue-wide destination list. The app checks selected files across every queued source for existing files and duplicate targets.</li>
      </ol>
      <p>If no separate folder is set, a full rebuild replaces the source only after validation and moves the original into <code>old/</code>. With a separate folder, the source stays where it is. Existing destination files are never overwritten.</p>
    </article>

    <article class="help-card help-card-wide">
      <div class="help-card-icon"><Settings2 size={18} /></div>
      <div class="eyebrow">REQUIRED MEDIA TOOLS · ALL PLATFORMS</div>
      <h2>Check or set up the tools</h2>
      <p>Open <strong>Settings → Environment & capabilities</strong> to see detected paths, tool versions, and capability status. The GUI, queue, drag-and-drop, metadata, reports, CLI, and MCP workflows are shared across Linux, Windows, and macOS. Use the host package manager for setup, or use each executable’s <strong>Browse</strong> button when a tool is installed in a nonstandard location. Custom binary overrides are never replaced by setup.</p>
      <pre><code>sudo apt update
sudo apt install ffmpeg mediainfo mkvtoolnix</code><button type="button" class="help-copy" aria-label="Copy Linux install command" onclick={() => copyText("sudo apt update\nsudo apt install ffmpeg mediainfo mkvtoolnix", "Linux command")}><Copy size={15} /> Copy</button></pre>
      <pre><code>winget install --exact --id Gyan.FFmpeg --accept-source-agreements --accept-package-agreements
winget install --exact --id MediaArea.MediaInfo --accept-source-agreements --accept-package-agreements
winget install --exact --id MoritzBunkus.MKVToolNix --accept-source-agreements --accept-package-agreements</code><button type="button" class="help-copy" aria-label="Copy Windows install command" onclick={() => copyText("winget install --exact --id Gyan.FFmpeg --accept-source-agreements --accept-package-agreements\nwinget install --exact --id MediaArea.MediaInfo --accept-source-agreements --accept-package-agreements\nwinget install --exact --id MoritzBunkus.MKVToolNix --accept-source-agreements --accept-package-agreements", "Windows command")}><Copy size={15} /> Copy</button></pre>
      <pre><code>brew install ffmpeg mediainfo mkvtoolnix fontconfig</code><button type="button" class="help-copy" aria-label="Copy macOS install command" onclick={() => copyText("brew install ffmpeg mediainfo mkvtoolnix fontconfig", "macOS command")}><Copy size={15} /> Copy</button></pre>
      <p class="help-note"><span>Integrity checks, scans, metadata-only MKV, short tests, full rebuilds, and YouTube copies use the shared native Rust coordinators exposed by the GUI, CLI, and MCP server. The HDR10+ helper is downloaded from its official release and checksum-verified on all three desktop hosts.</span></p>
      <div class="help-links">
        <a href="https://www.ffmpeg.org/download.html" target="_blank" rel="noreferrer">FFmpeg downloads <ExternalLink size={13} /></a>
        <a href="https://mediaarea.net/en/MediaInfo/Download" target="_blank" rel="noreferrer">MediaInfo downloads <ExternalLink size={13} /></a>
        <a href="https://mediaarea.net/en/Repos" target="_blank" rel="noreferrer">MediaArea package repository <ExternalLink size={13} /></a>
        <a href="https://mkvtoolnix.download/downloads.html" target="_blank" rel="noreferrer">MKVToolNix downloads <ExternalLink size={13} /></a>
        <a href="https://github.com/quietvoid/hdr10plus_tool/releases/latest" target="_blank" rel="noreferrer">HDR10+ helper <ExternalLink size={13} /></a>
      </div>
      <p class="help-note">The Linux command uses enabled repositories, which can lag upstream releases. FFprobe normally comes with FFmpeg; this app needs MediaInfo’s command-line tool plus MKVToolNix for remux and metadata work. The app checks the installed version and required encoder/features before a run.</p>
    </article>

    <article class="help-card help-card-wide">
      <div class="help-card-icon"><BookOpen size={18} /></div>
      <div class="eyebrow">CLI</div>
      <h2>Use it from a terminal</h2>
      <p>Run a read-only tool check or inspect a media input. A confirmed run can be limited to absolute paths from that analysis.</p>
      <pre><code>hevc-rebuild-studio json environment
hevc-rebuild-studio json integrity --file "/path/to/Film.mkv" --level quick
hevc-rebuild-studio json scan --root "/path/to/Media" --recursive
hevc-rebuild-studio cli "/path/to/Media" --recursive --info-only
hevc-rebuild-studio cli "/path/to/Media" --selected-file "/path/to/Media/Film.mkv" --info-only
hevc-rebuild-studio cli "/path/to/Media" --recursive --output-dir "/path/to/Rebuilt"</code><button type="button" class="help-copy" aria-label="Copy CLI examples" onclick={() => copyText("hevc-rebuild-studio json environment\nhevc-rebuild-studio json integrity --file \"/path/to/Film.mkv\" --level quick\nhevc-rebuild-studio json scan --root \"/path/to/Media\" --recursive\nhevc-rebuild-studio cli \"/path/to/Media\" --recursive --info-only\nhevc-rebuild-studio cli \"/path/to/Media\" --selected-file \"/path/to/Media/Film.mkv\" --info-only\nhevc-rebuild-studio cli \"/path/to/Media\" --recursive --output-dir \"/path/to/Rebuilt\"", "CLI examples")}><Copy size={15} /> Copy</button></pre>
      <p>The JSON commands are suitable for scripts. Start with environment inspection and a scan; use a confirmed run command only when you intend to create outputs. <code>json queue</code> returns a compact page of the saved library, and <code>json queue-select --request selection.json</code> saves a selection containing paths already in that queue.</p>
      <p>For a multi-folder output check, create a JSON request file and run <code>hevc-rebuild-studio json output-plan --request plan.json</code>. The same read-only planner is available to MCP clients as <code>preview_media_outputs</code>.</p>
    </article>

    <article class="help-card help-card-wide">
      <div class="help-card-icon"><HelpCircle size={18} /></div>
      <div class="eyebrow">MCP · LOCAL STDIO CONNECTION</div>
      <h2>Connect an MCP client</h2>
      <ol>
        <li>Install HEVC Rebuild Studio and note the absolute path to its executable. On Ubuntu, <code>command -v hevc-rebuild-studio</code> prints the PATH location when installed that way.</li>
        <li>In your MCP client’s server configuration, add this entry and replace the example executable path.</li>
        <li>Restart or reload the client, then call <code>inspect_environment</code> and <code>start_folder_analysis</code>.</li>
        <li>Poll the returned job with <code>get_job</code>. Start with the default check mode; output-producing runs require their explicit confirmation fields.</li>
      </ol>
      <pre><code>{mcpConfig}</code><button type="button" class="help-copy" aria-label="Copy MCP configuration" onclick={() => copyText(mcpConfig, "MCP configuration")}><Copy size={15} /> Copy</button></pre>
        <p>Available MCP tools include environment/settings inspection, <code>inspect_work_directory</code> for dynamic source-specific scratch-path discovery, folder analysis, <code>start_integrity_check</code> for native Quick/Full corruption checks, metadata and artwork lookup, <code>inspect_media_queue</code> for paged saved-library state, <code>set_media_selection</code> for the shared global selection, <code>inspect_local_library_assets</code> for read-only artwork details, <code>preview_media_outputs</code> for queue-wide destination/collision review, safe rename previews, job status, stop-after-current, and reports. Pass analyzed absolute paths in <code>selected_files</code> to limit a run; omit it to include every eligible file. <code>start_media_run.work_root</code> accepts the same <code>auto</code> or absolute-folder policy as the GUI and CLI. For test or full rebuilds, <code>output_dir</code> sets the default destination and <code>file_output_dirs</code> overrides it for individual files. <code>file_poster_overrides</code> maps media paths to selected local image paths within the same library. The server runs as a local process over stdio.</p>
      <a class="help-doc-link" href="https://modelcontextprotocol.io/docs/develop/build-server" target="_blank" rel="noreferrer">MCP server and stdio overview <ExternalLink size={13} /></a>
    </article>

    <article class="help-card help-card-wide">
      <div class="help-card-icon"><Play size={18} /></div>
      <div class="eyebrow">SUPPORTED DESKTOPS</div>
      <h2>One workflow across Linux, Windows, and macOS</h2>
      <p>The product contract is cross-platform: the GUI, drag-and-drop queue, Library organize view, reports, CLI, and local MCP server use the same behavior on all three desktop hosts. Package managers, app-data paths, process telemetry, and installers are host adapters. Platform builds and native runtime acceptance must still be performed on each operating system before a release is called verified.</p>
    </article>
  </section>
</section>

<style>
  .help-page { display: grid; gap: 22px; }
  .help-page .page-heading { margin: 0; }
  .help-page .page-heading p { max-width: 780px; }
  .help-copy-status { position: sticky; top: 8px; z-index: 3; width: fit-content; margin-left: auto; padding: 8px 12px; border: 1px solid var(--theme-line); border-radius: 8px; background: var(--theme-surface); color: var(--theme-text); }
  .help-grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 14px; }
  .help-card { display: grid; grid-template-columns: minmax(0, 1fr); align-content: start; gap: 10px; min-width: 0; padding: 20px; border: 1px solid var(--theme-line); border-radius: 14px; background: var(--theme-surface); box-shadow: var(--theme-shadow); }
  .help-card-wide { grid-column: 1 / -1; }
  .help-card-icon { display: grid; place-items: center; width: 38px; height: 38px; border-radius: 10px; color: var(--theme-accent-strong); background: var(--theme-raised); }
  .help-card .eyebrow { color: var(--theme-muted); font-size: 12px; font-weight: 760; letter-spacing: .08em; }
  .help-card h2 { margin: 0; color: var(--theme-text); font-size: 20px; line-height: 1.2; }
  .help-card p, .help-card li, .help-card dd { color: var(--theme-muted); font-size: 14px; line-height: 1.6; }
  .help-card p { margin: 0; }
  .help-card ol { display: grid; gap: 8px; min-width: 0; margin: 0; padding-left: 22px; }
  .help-card li { min-width: 0; overflow-wrap: anywhere; }
  .help-card strong, .help-card dt { color: var(--theme-text); }
  .help-card code { color: var(--theme-accent-strong); font-family: var(--font-mono, monospace); font-size: 13px; }
  .help-card pre { position: relative; overflow: auto; margin: 0; padding: 15px 90px 15px 15px; border: 1px solid var(--theme-line); border-radius: 9px; background: var(--theme-raised); color: var(--theme-text); font: 13px/1.65 var(--font-mono, monospace); }
  .help-card pre code { color: inherit; font-size: inherit; white-space: pre; }
  .help-copy { position: absolute; top: 8px; right: 8px; display: flex; align-items: center; gap: 5px; min-height: 30px; padding: 0 8px; border: 1px solid var(--theme-line); border-radius: 7px; background: var(--theme-surface); color: var(--theme-text); cursor: pointer; font: inherit; }
  .help-note { display: flex; align-items: flex-start; gap: 8px; padding: 10px 12px; border: 1px solid var(--theme-line); border-radius: 8px; background: var(--theme-raised); }
  .help-note :global(svg) { flex: 0 0 auto; margin-top: 2px; color: var(--theme-accent-strong); }
  .help-note span { min-width: 0; overflow-wrap: anywhere; }
  .help-map { display: grid; gap: 10px; margin: 0; }
  .help-map div { display: grid; grid-template-columns: minmax(150px, .35fr) 1fr; gap: 12px; padding-bottom: 9px; border-bottom: 1px solid var(--theme-line); }
  .help-map dt { font-size: 13px; font-weight: 700; }
  .help-map dd { margin: 0; }
  .help-links { display: flex; flex-wrap: wrap; gap: 9px; }
  .help-links a, .help-doc-link { display: inline-flex; align-items: center; gap: 6px; width: fit-content; padding: 8px 10px; border: 1px solid var(--theme-line); border-radius: 7px; color: var(--theme-accent-strong); text-decoration: none; font-size: 13px; }
  .help-links a:hover, .help-doc-link:hover { border-color: var(--theme-accent); }
  @media (max-width: 780px) {
    .help-grid { grid-template-columns: 1fr; }
    .help-card-wide { grid-column: auto; }
    .help-map div { grid-template-columns: 1fr; gap: 2px; }
    .help-card { padding: 16px; }
  }
</style>
