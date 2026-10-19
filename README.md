# HEVC Rebuild Studio for Ubuntu and Linux Mint

The HEVC rebuild workflow compresses the **video** to H.265/HEVC with CPU x265 while protecting the rest of the media file. The desktop app also has a **Metadata-only MKV** task that remuxes every source track into a new MKV, applies reviewed tags and selected local poster art, and validates the copy without encoding video.

In a rebuild, **audio and subtitles are never transcoded**: selected tracks are copied and verified. By default, video handling follows the saved secondary-video policy. In the Media library, you can customize included FFprobe streams for each file: included real-video tracks are encoded to HEVC, and selected audio, subtitle, attachment, and data tracks are copied. This supports keeping and encoding both 4K and 1080p video tracks in one MKV. The GUI also lets you override the x265 content tune per file, so an animated source can use x265's Animation tune while live-action files keep the global setting.

The safe defaults are **CRF 21 + preset slow**. The program keeps the source bit depth by default: 8-bit stays 8-bit, 10-bit stays 10-bit, and 12-bit stays 12-bit. It can optionally promote 8-bit video to 10-bit if you turn that setting on.

Your original file stays untouched until the rebuilt MKV passes validation. After success, the original is moved into an `old/` folder. If encoding finishes and a later metadata, validation, or install step fails, the completed output is retained and the run remains failed for review. When the intended destination is free it keeps the exact `<name>.mkv` path; an in-place rebuild keeps the source-safe sibling `<name>.hevc-rebuild.kept.mkv` because it must not overwrite the original before validation. In both cases the adjacent diagnostic is `<name>+error.log` (with a numbered suffix only if that log already exists). The retained media is never reported as a successful rebuild.

If the app reopens while files were still active, it settles them without a manual check. A saved successful result is marked complete. An unfinished file whose original is still present returns to the normal queue, and only an unopened temporary encode created by this app is removed. A partial encode is not resumed, and an existing output or `old/` backup is not overwritten. A missing file, or a successful report whose output is gone, stays marked for review; output collisions still block the next run.

## Desktop GUI (v1.3.0 preview)

### Current acceptance evidence (2026-10-12)

The final acceptance artifacts were built inside an Ubuntu 24.04.5/glibc 2.39 userspace and passed isolated `.deb` metadata/install/environment/scan/engine smoke. Packaged WebDriver covered Home, Media Library, Run Setup, the final safety dialog, and focus return after Escape. The ordinary host-built AppImage guidance below still applies to artifacts built outside that baseline; repeat the Ubuntu 24.04 build for any release artifact. Screen-reader announcements beyond startup/frame announcements, provider/account flows, and Windows/macOS remain unverified or unsupported.

**Release readiness (2026-10-12): not ready for publication.** Source and Ubuntu-baseline package gates pass, but the acceptance record still has a specific screen-reader announcement blocker, no Windows/macOS runtime proof, no account upload/playback proof, and host encoder limitations for HDR10+ dynamic-metadata encoding and Dolby Vision full re-encode. The local safety policies fail closed or require explicit fallback consent in those cases.

### Windows CLI build lane

The direct Python engine now has a Windows dependency path for the agent-facing
build lane. On the configured Windows Server guest, `python -m hevc_rebuild
--setup-only --assume-yes` installs the explicit winget packages
`Gyan.FFmpeg`, `MediaArea.MediaInfo`, and `MoritzBunkus.MKVToolNix`, then
validates FFmpeg/FFprobe, MediaInfo, and MKVToolNix capabilities. A synthetic
10-bit HEVC short encode has passed this lane. This is not yet proof of the
Tauri desktop UI, native accessibility, signed packaging, real-media
compatibility, or macOS support.

This lane is narrower than the desktop target statement below: it proves the
direct engine's Windows dependency/build path and synthetic encode only. The
Tauri Windows runtime remains an unverified future target.

The Ubuntu-first desktop interface is built with **Tauri 2 + Svelte 5**. Its first window opens at 1600 × 1000 logical pixels when the monitor permits, then restores the last usable size and maximized state from the app’s native settings profile. Environment checks, effective-settings reads, read-only media discovery/profiling/resource forecasts, live Linux resource observations, starting/retry thread plans, and live memory/swap/disk guard decisions run through native Rust services across the GUI, CLI, and MCP. App-launched jobs also use Rust to discover input batches, resolve configured/PATH tool binaries, check versions and encoder capabilities, run bounded encoder smoke checks, and build source plus selected-video-track profiles from one base FFprobe/MediaInfo result, sampling frame details for selected additional video tracks. The Python adapter converts those typed Rust profiles for the engine stages not yet ported; direct Python CLI runs keep their profiler. Rust builds the validated FFmpeg/x265 argument vector and owns FFmpeg child start, progress-pipe relay, exit handling, cancellation, and the validated-output commit/rollback step for Rust-launched rebuilds, including multi-video mappings and source-aware HDR/deinterlace flags. Rust's `hdr_metadata.rs` streams HDR10+ verification/extraction and evaluates Dolby Vision preservation/fallback eligibility for Rust-launched runs. Rust `validation.rs` checks selected stream identity/count/order, video codec/pixel format/resolution, copied-stream hashes/dispositions, track parameters/timing, chapters, duration, and each encoded video track's color/HDR/Dolby Vision signaling; it also verifies requested HDR10+ metadata on the corresponding output track. Python still interprets progress, applies tune overrides and retry/report orchestration, prepares per-file report values, performs Matroska metadata changes, and keeps its full direct-run validation fallback. Rust appends fixed-schema CSV rows for Rust-launched jobs; direct Python retains its report-writer fallback. Rust computes calibration eligibility and machine/workload fingerprints, constructs bounded null-output commands, checks/writes the cache, monitors candidate processes/resources, rejects unsafe candidates, and selects the layout for Rust-launched jobs; direct Python retains the reference implementation. Direct Python runs retain reference implementations, including the transaction fallback. The engine and `.deb` target Ubuntu 24.04+ and Ubuntu-based Linux Mint 22+; Windows and macOS are future targets, not supported builds yet. The current AppImage was built on Ubuntu 26.04.1 and bundles GTK/WebKit libraries requiring glibc 2.43, so this artifact is not compatible with Ubuntu 24.04's glibc 2.39.

The desktop app opens on Home. Home leads with the next action: add media, review the library, confirm the match, write metadata or rebuild, or prepare a YouTube copy. Tool checks, recent runs, and setup logs stay under those actions. An empty queue always returns to Home. A queue that already has media reopens on the last tab. Movies are poster cards. A TV folder opens as the series, then its seasons, then its episodes; each episode shows the poster from its season. The path above those cards goes back to the series or to all media. Review series, Review season, and an episode open a poster-first review with the title, plot, poster choices, and a Media info section for the files in that review. A season with no plot or score of its own shows the series description and series score, and its year comes from the first episode air date in that season. When the matched show publishes an episode title, plot, or air date, that text is saved on the matching file when the field is still empty. A new episode MKV writes that episode plot into both the summary and the synopsis. The series description is not written as the episode summary, and the series score is not written as the episode rating or still only the filename or series text. A season plot and audience score are saved the same way when that season has them. A series score is not copied onto a season, and a season with no published plot stays empty. A season or episode with no certificate of its own shows the series certificate. Open file location on a card or in that review opens the series folder, season folder, or the episode’s folder. A poster that is no longer in the folder is left off the card; a matched provider image is used instead. The Animated box on a series or season sets every episode inside it, and shows a dash when only some episodes are marked. Click a movie to open the same kind of review, including year, certification, and audience rating. Rename files opens a name preview on the library. The setup badge names the missing tool. Rename, metadata-only MKV, and full HEVC rebuild apply to the selected cards and still use the organizer and run safety checks; Check only and short tests remain in Run setup. The Active Library control returns to that page, while folder selection is a separate action that starts from the current or last-used media location. Select-all in the table applies to visible (filtered) rows. YouTube readiness and upload-specific estimates stay on the YouTube prep page. The persistent queue saves multiple source paths, scan snapshots, global selection, active source, per-file animation/stream choices, reviewed movie and TV-series metadata, provider-artwork references, episode-to-file assignments, selected local movie/show/season poster paths, and bounded per-file run results. Selected provider or local posters and reviewed titles appear on each file's poster card. A provider thumbnail shows as soon as the match arrives. A matched series poster, and each season poster found for that series, is saved as `poster.jpg` (or the provider image’s PNG or WebP extension) in the series or season folder when Save poster beside the media is on. An image already in that folder is left in place. Movie posters stay a preview until they are saved. Embedding a saved poster still happens when metadata-only output or a rebuild runs. Local previews load only as their cards approach view and only from within the analyzed source. Confirmed renames update cached queue paths, selections, assignments, artwork references, and output overrides. Explicit local poster selections are passed into metadata-only, short-test, and full-rebuild outputs after Rust validates their path and image format. Provider art must first be saved locally; YouTube output does not consume poster overrides. Changed or missing files are flagged when the queue is restored. A run includes selected files, or new and queued files for the chosen task when nothing is selected; completed, failed, skipped, interrupted, and needs-review results stay out of that task's default run until selected. The GUI processes one source at a time using its saved scan settings. Stop-after-current and stop-on-error apply to the whole sequence. Per-file outcomes are saved from the report; an active batch becomes interrupted after restart while untouched batches remain queued. Each result row can expand the saved message, output path, and report status, load a bounded 1 MB diagnostic excerpt, open the report, select a file for retry, or mark it skipped. The memory column keeps risk and worker count visible while its budget, swap, and pressure-retry explanation are expandable. Interrupted/uncertain files still require report/output review before a mutating retry. A Rust planner previews destinations across all selected queue sources and blocks existing-file or duplicate-target collisions before start; the engine repeats each source check before writing. Native output-review and close/reopen verification remain open.

Per-file queue status now shows and persists active scanning, renaming, remuxing, rebuilding, and validation stages, plus terminal outcomes. Active work is settled on the next open: a saved success is marked complete, and an unfinished original returns to the queue after its temporary encode is removed. A selected local movie poster can be assigned to selected movie rows in the current organizer after the exact affected files are shown. This changes saved artwork choices only; it does not copy or replace image files. The shared output plan still shows and checks each destination before creating tagged MKVs.

When automatic metadata suggestions are enabled, each scan checks local NFO data before querying enabled public sources. The library shows the top title/poster suggestion without assigning it. Compact, bounded suggestions persist in the Rust queue; incomplete lookups can resume after restart, and opening a suggested row fetches detailed candidates for that item only. These flows have passed source-level Rust persistence and frontend checks; native provider, restart, and large-library verification remain open.

The desktop workflow includes:

- Rust HDR profiling considers stream and sampled-frame side data and falls back to MediaInfo mastering-display and chroma-position fields when FFprobe omits them; unknown color labels stay visibly unavailable.
- native Rust read-only CPU, memory, GPU, operating-system, FFmpeg/libx265, and required-tool inventory across the GUI, JSON environment command, and MCP;
- native Rust library folder/file discovery, recursive analysis, minimum-size filtering, per-file media/HDR/stream summaries with expandable raw FFprobe, MediaInfo, container, and chapter details, track selectors, per-file Animated/Normal tune overrides, NFO/MediaInfo genre evidence and automatic x265 tune decisions, disk-space visibility, scan warnings/errors, and a file-by-file memory forecast;
- a versioned Rust-owned local scan queue that accepts multiple files/folders from native pickers or drag-and-drop, retains completed scans, global selection, active source, per-file animation/stream choices, and per-file run results using bounded atomic writes beside the portable or installed app profile; library tables paginate results (50/100/200 rows), the header checkbox selects the current page, and a separate action selects every search match; selections persist across pages and workflow tabs; GUI runs schedule selected or all-ready files sequentially, and the next open settles interrupted files without resuming an encode; native restart verification remains open;
- FFprobe-only scans remain available when MediaInfo is missing; those source profiles are marked partial, and processing jobs remain gated on their own required toolchain;
- rebuild-time metadata text cleanup for release-group suffixes and user-managed blacklist patterns across Matroska tags, track names, and chapter titles; reports show each exact before/after change, including matches removed from encoder/statistics text while retaining all other technical text and stream payloads;
- effective built-in, user, app-profile, and library settings resolved by the shared Rust settings module and parity-checked against Python without duplicating defaults in the UI;
- cached, short x265 hardware calibration for demanding 4K/large sources, with candidate results and the selected thread layout recorded in reports;
- Check only, metadata-only MKV remux, 5-second short tests (test CRF 30), and confirmed Full rebuild modes;
- a saved default rebuild destination and per-file destination overrides across the GUI, CLI, and MCP; default destinations mirror folder structure, separate outputs preserve source files, and a queue-wide preview blocks existing or duplicate targets before encoding;
- a source-aware YouTube export profile for one file or a folder, defaulting to a 1440p output target for smaller sources, with a size-aware bitrate mode, per-file/aggregate size estimates, separate destinations, per-file errors, and Rust live-resource sampling and guard decisions; Python still adapts the sample for display and supervises the encode during migration;
- a dedicated **YouTube prep** sidebar workflow with editable output tier, bitrate profile, balanced-size multiplier, and destination, using the same analyzer and report pipeline;
- a safe stop request that finishes the active file before stopping the queue;
- a reviewed Ubuntu tool setup that installs missing allowlisted packages, updates only the listed outdated packages, simulates both APT plans, refuses removals, and installs a missing HDR10+ helper from its checksum-verified upstream release after APT succeeds;
- startup checks for detected tool versions and a timestamped comparison against the local Ubuntu APT catalog; catalog refresh and package changes require an explicit user action, and repository candidates may lag upstream;
- Per-tool executable overrides for FFmpeg, FFprobe, MediaInfo, and all three MKVToolNix utilities, with version/readiness rechecks; report history, per-file outcomes, and persistent setup diagnostics that can be reopened and exported.
- a dedicated **Identify & organize** workbench connected to the current scan: each Media Library file has a **Title and poster** action that keeps it selected and opens a focused review dialog for that film or series. The dialog shows the current and proposed filename, title matches, source details, editable fields, and artwork, with a direct route to the full batch organizer. Candidate lists start with four results and expand on request; year conflicts require review. Independent metadata per film/episode/series, naming patterns, collision-checked rename previews, and attributed poster sidecars remain available;
- a dedicated in-app **Help** section with step-by-step workflows, Ubuntu media-tool commands and official download links, CLI examples, and MCP stdio connection instructions;
- one shared file selection across media analysis, YouTube preparation, and metadata editing; run controls apply to selected files across ready sources, or every eligible queued file when the selection is empty;
- editable filename include/exclude patterns in Settings, seeded with removable extra-folder, hidden-name, sample, and trailer rules from RenameTheThings; users can remove one chip or clear the list. The archive has no separate word-blacklist list, so the example-based `Movie*Group` metadata rule is editable independently. Text rules clean imported title clues before public search, rename proposals, and output tags without changing source filenames during import;
- a cross-page run status bar and matching queue-row progress for the active file, with the current preparation step named as it starts (reading tracks, an HDR10+ check only when the file signals HDR10+, Dolby Vision check, disk space, HDR10+ extraction, encode planning, and calibration when it runs), then encode percent, elapsed time, and an encode ETA in hours and minutes from the remaining duration divided by the current speed, plus input/output size, a changing output-size estimate, and a visible validation/result state; the activity log keeps a line at each 5% while the bar and resource cards carry the live numbers; the final report records actual output size;
- Light and Dark everyday appearances use coordinated sage and graphite surfaces with theme-aware workflow artwork and status icons; Phosphor, Ion, and Breach remain distinct instrument skins (legacy Hacker maps to Phosphor). Library/app preferences persist in native settings, while the browser-only preview stores its UI preferences separately in that browser;
- a queue-first Media Library where every folder and file is scanned, reviewed, renamed, remuxed, or rebuilt from the same persistent list;
- the Python compatibility media engine and legacy JSON bridge remain available during migration; native Rust now owns environment/settings/report reads, cached APT checks, app-launched input discovery and toolchain resolution, folder analysis and tune previews, run-time global/per-file tune selection, live Linux resource observation, host/run sampling and memory/swap/disk guard decisions, FFmpeg/x265 argument planning, HDR10+ streaming verification/extraction, Dolby Vision eligibility checks, child-process supervision, per-track stream/HDR output validation, YouTube output validation/install, and validated-output commit/rollback for app-launched rebuilds, plus the public metadata/organizer/NFO CLI and typed MCP tools. Python still interprets progress, adapts native snapshots, passes validated selections, prepares report values, edits Matroska metadata, coordinates run/retry/error workflows, and retains direct-engine fallbacks; expected Matroska tag/poster verification and end-to-end run/report orchestration remain Python-owned. Rust owns eligibility, workload fingerprints, command construction, cache/benchmark policy, and selection for Rust-launched calibration; direct Python keeps compatibility fallbacks. See the [Python-to-Rust migration map](RUST_MIGRATION.md) for exact parity boundaries.

The application routes all documented JSON actions to Rust; it keeps the standalone Python bridge only as a migration/reference entry point.

The library rebuild encoder is **CPU libx265** even when a GPU is detected. YouTube export selects H.264/AAC for SDR or 10-bit HEVC/AAC for source files that meet the HDR color-tag requirements. The HDR path requires a 10-bit-capable libx265 build; Dolby Vision, incomplete HDR tags, and files below the 720p HDR floor are skipped with per-file reasons. The transcode preserves base HDR10/PQ or HLG signaling, but does not carry HDR10+ dynamic metadata; affected sources are flagged in the library and report. YouTube accepts HDR uploads with Rec. 2020 primaries/matrix and PQ or HLG at 10/12-bit; see its [HDR upload requirements](https://support.google.com/youtube/answer/7126552?hl=en-GB) and [recommended upload settings](https://support.google.com/youtube/answer/1722171?hl=en-GB). Export creates new MP4 files in a `YouTube Uploads` folder by default, keeps sources in place, skips existing outputs, and records each result; it prepares files but does not upload them. The default **Balanced size** mode targets total output bitrate near twice the source's average overall bitrate, subject to YouTube's video-rate recommendation. **YouTube bitrate guidance** mode targets YouTube's recommended video rate and can produce much larger files. The preflight estimate is not a hard size limit; actual size varies with content and audio tracks. 1440p is the default for smaller sources, and the 1440p/4K choices upscale smaller video while never downscaling a larger source. Upscaling adds no source detail and does not guarantee a YouTube codec or playback tier. YouTube may initially show lower quality while higher-resolution versions finish processing. The toolchain needs Python 3, FFmpeg/FFprobe, MediaInfo, libx264, and AAC; x265 10-bit is checked for HDR files. Full rebuilds retain their broader tool requirements. Opening the environment screen and scanning do not install packages or write the user config. System repairs run only after you choose them; APT repair shows the package list and asks for confirmation. Rebuild edits remain run-only until you choose **Save as app defaults**. Those saved values are shared with the CLI and MCP server, layer after the user config, and remain below a library-specific `.hevc-rebuild.toml` override.

The Ubuntu AppImage stores `settings.json`, `library-queue.json`, its engine config, and logs in a writable sibling directory named `<AppImage filename>.data/`. Move that folder with the AppImage to keep the portable profile. The HDR10+ helper is installed once under `~/.local/share/hevc-rebuild/bin` and copied into that sidecar when the sidecar can be written. The AppImage and a local build both look in those two folders, then on `PATH`, so the same install is found after a rebuild. If the folder beside the AppImage cannot be written, the app reports that and falls back to the user's XDG config location. The `.deb` installer stores application preferences and `library-queue.json` under `$XDG_CONFIG_HOME/hevc-rebuild-studio/` (normally `~/.config/hevc-rebuild-studio/`); it does not attempt to write beside the system executable under `/usr/bin`.

In the non-native browser preview, appearance and other UI preferences are saved to browser-local storage; this preview cannot connect to system tools, inspect hardware, scan folders, or run jobs. In the desktop app, right-clicking the application opens a native utility menu with folder analysis, section navigation, environment refresh, and context-specific path/log copy actions. Text-editing fields retain their normal copy/paste menus.

Use the app with local media you own or are authorized to process, such as home media-server libraries and lawful personal backups. HEVC Rebuild does not download media, remove DRM, or distribute content.

The YouTube prep tab lets you set the output tier, bitrate profile, balanced-size multiplier, and destination. Balanced mode defaults to `2.0×` the source's average overall bitrate; adjust it from `0.5×` to `4.0×` to tune the size/quality tradeoff. The video target has a 250 kb/s floor and remains capped by YouTube's published bitrate guidance. The estimate updates from the current scan and is not a strict file-size ceiling. These preferences are saved with the app and can also be set through CLI and MCP.

### Metadata lookup and organization

The **Identify & organize** page is tied to the current scan. It checks for existing Kodi NFO data and nearby art as compatibility inputs, then searches enabled public providers for records without a usable local or reviewed match. When automatic lookup is enabled, a background worker starts after a scan and reports progress and per-file failures in the library queue. It keeps a small top result and poster reference with each file. A unique exact normalized title-and-year match with no same-title year conflict, duplicate same-provider result, or disagreement between providers' top results is associated automatically; other files keep the highest-ranked result as a suggestion for review. This is a deterministic safeguard, not a measured 99% precision claim. Opening an auto-associated item fetches its full provider details for the editor. You can disable automatic lookup, compare providers, or open **Search another title** on a specific record. Title clues are cleaned locally and year hints only affect local ranking. Providers receive the cleaned title, not filenames, paths, NFO text, ratings, artwork bytes, or video contents.

From the Media Library, clicking a poster card adds that file to the shared selection and opens a focused review dialog for its film or associated series. A movie opens as a large poster with clickable alternate artwork beside the title, year, certification, and plot; genres, rating, short description, and source URL stay under More metadata. The dialog also shows the current filename and proposed name. An auto-associated record loads its full provider details when opened. Choose **Rename files** on the library to name one selected file or every selected file with the same pattern. The original name and the new name update while the pattern is edited, and nothing moves until that rename is confirmed. **Open full organizer** still renames a series folder and its season folders. Four alternative candidates are shown first; **Show all** expands the list. A year mismatch is labeled with both years and cannot be auto-associated. Match evidence is shown as text, not as a probability. For Wikidata/Commons, artwork appears only when the image file title contains every meaningful word in the candidate title; weak matches are left out. Reviewed movie and series fields, provider/local poster choices, season art paths, episode lists, and episode-to-file assignments persist in the local queue and follow confirmed renames. The library row previews the assigned local image or selected provider poster. A remote poster is associated and previewed, but its bytes are not fetched without confirmation. In **Settings → Poster storage**, independently choose whether confirmed artwork is saved beside the media as `[media filename]-poster.<ext>` or in the private app artwork cache, and whether the selected poster is embedded in metadata-only and rebuilt MKV outputs. Enable both to keep a sidecar and embed a cover. The **Saved poster artwork** panel reports total, assigned, and unused cache size and offers a preview-ID-protected purge for unused app-managed images; assigned posters and unrecognized files are left untouched. This purge does not touch x265 calibration data, which stores thread-layout benchmark results and makes eligible large-file jobs benchmark again if cleared.

The main **Settings** page controls automatic lookup, the TMDB/Wikidata/TVmaze/TheTVDB sources, and the metadata language/region. Provider cards show key requirements and jump directly to credential setup. TMDB and Wikidata use the selected locale where available; TVmaze returns fields in the language its contributors supplied. TMDB is enabled by default but is skipped until you save your personal API Read Access Token. Settings links to [TMDB's API registration page](https://www.themoviedb.org/settings/api). The token is stored in the operating-system credential manager and read by Rust only for provider searches. The input stays readable while editing and after saving in the current window; after reopening, **Show saved token** explicitly reads it from the keyring. Configured status remains visible without exposing the value. The token does not travel with a portable profile and is never written to browser storage, reports, CLI output, or MCP. Public Wikidata/Wikipedia/Commons and TVmaze lookup is also available through the Rust metadata CLI and four typed MCP tools; TMDB lookup results and its saved token remain in the GUI workflow.

TheTVDB v4 can be enabled with a user-supplied project API key and optional subscriber PIN. Both are saved in the operating-system credential manager and never enter the portable profile, browser storage, CLI, MCP, or logs. Entries stay visible while editing and after saving in the current window; after reopening, **Show saved credentials** explicitly reads them from the keyring. The user must confirm valid project access before saving. TheTVDB documents direct-access licensing and user-supported models; review its [API access page](https://www.thetvdb.com/api-information/signup) and [v4 API guide](https://github.com/thetvdb/v4-api/blob/main/README.md). The workbench currently loads TheTVDB’s default season order; alternate aired/DVD/absolute episode orders still need explicit selection and review for anime. AniList is not used for locally persisted library metadata because its published API terms prohibit using the API as backup or data storage.

Search candidates show a ranking hint, not an automatic match. Select one to inspect dates, descriptions, certifications or audience ratings, season/episode data, posters, attribution, and available license information. TMDB use displays its logo and non-endorsement notice; the logo is loaded from TMDB's official static asset URL, so it may not appear offline. Review the [TMDB API FAQ](https://developer.themoviedb.org/docs/faq), [application authentication guide](https://developer.themoviedb.org/docs/authentication-application), and applicable [API terms](https://www.themoviedb.org/api-terms-of-use) before reuse. Commercial use requires a separate license from TMDB.

The Rust organizer scans names and common `SxxEyy` codes locally without probing media contents. Kodi NFO files remain optional compatibility inputs: existing movie, episode, and nearest show NFO details can seed editable fields, and an episode NFO's `<showtitle>` can identify local show metadata when `tvshow.nfo` is absent. For TV folders, the local `tvshow.nfo` is the primary local series record; conflicting episode `<showtitle>` values or unreadable episode NFOs block local episode metadata from being applied and identify the files to review. A malformed NFO remains visible as an error. Conflicting scalar fields inside or across same-kind NFO sidecars are reported with their filenames and neither record is applied. The workbench also finds nearby movie/show/season images and previews them inside the matching assignment or series record. This is an artwork preview, not a media player.

For a movie folder, assign a reviewed match to each selected file; matches in one batch may refer to different films. Each assignment can independently edit title, year, certification, short and long descriptions, genres, rating, and poster choice. Copying one reviewed match to several files is an explicit shortcut for alternate encodes of the same film. Then inspect the complete before/after preview. For one TV-series folder, the workbench maps recognizable episode IDs to provider episodes and requires a reviewed assignment for each selected episode. It flags possible gaps in the observed `SxxEyy` filename sequence as a clue to check, not proof that an episode is missing; multi-episode seasons are excluded from this heuristic. A filename/provider year mismatch stays visible and requires an explicit acknowledgment before a TV rename. Full-series rename is blocked when the scan is truncated or unreadable, or when a supported episode was left unselected. Multi-episode files need manual handling and are not included in a TV rename batch yet. The defaults follow the supplied examples: `Film Title (Year Certification).mkv` and `Series/Season 01/S01E01.Episode Title.mkv`; a series year is not added by default. Naming patterns are editable and saved with application settings; conditional `[[ ... ]]` blocks omit optional parts when their shortcodes have no value. Extensions are preserved. Unicode-normalized, case-insensitive duplicate targets and occupied destinations block the entire batch. Applying requires a second explicit confirmation, rechecks the preview, stages same-directory moves without overwriting, and attempts rollback if an operation fails. Video contents are unchanged. Matching NFOs, file-specific local artwork (`Movie.jpg`, `Movie-poster.jpg`, `Movie-cover.jpg`, and recognized fanart/thumb names), artwork attribution files, common external subtitle files, and external audio files follow the renamed video with their suffixes intact; these moves appear in the same collision-checked preview and transaction.

Poster candidates can be saved after reviewing their source and license. A film poster is saved beside each explicitly assigned movie as `<filename>-poster.jpg` (or the detected PNG/WebP extension). A series poster is saved as both `folder.<ext>` and `poster.<ext>` at the selected root. Season artwork is saved as `poster.<ext>` and `SeasonXX.<ext>` in the season folder, plus `season<num>-poster.<ext>` at the series root; for specials, the season-folder and show-root copies use `season-specials-poster.<ext>`. These filenames follow local-art conventions documented by [Jellyfin](https://jellyfin.org/docs/general/server/media/shows/), [Plex](https://support.plex.tv/articles/200220717-local-media-assets-tv-shows/), and [Kodi](https://kodi.wiki/view/Artwork/Season). Plex documents JPG, JPEG, PNG, and TBN image files, so choose JPEG or PNG artwork when Plex must read it; the app preserves the provider image's detected format. The utility also saves a `.attribution.json` file for each image. Downloads are limited to HTTPS image hosts owned by the selected provider, checked against their file signatures, limited to 16 MiB per image, and written only into existing folders under the selected root. Existing files are never overwritten. The selected image path is associated with its reviewed queue item; Rust checks that the image is a supported file contained in the analyzed library before attaching it to the new Matroska output, with available source/license credits in the attachment description. Existing local poster choices follow the same validation and output path.

The full organizer also has a direct **Create a tagged MKV copy** path. Select the files there, review title/tags/art, and optionally confirm the filename plan first. **Review MKV output plan** then carries only those organizer selections into Metadata-only MKV mode. Run setup shows the destination and whole-queue collision check before starting. The remux makes a separate MKV without video encoding, applies reviewed metadata and selected local art, validates the copy, and leaves the source in place. Kodi NFO sidecars remain an optional, separate write action.

Reviewed movie, series, and episode metadata can be saved as Kodi-format NFO sidecars from the workbench. Movie documents carry each file's own reviewed and edited values, including short/long text, genres, and rating; TV episode edits include title, air date, and plot. The UI previews every source and destination first; a second explicit action writes the files. Existing user NFOs are preserved: when one exists, HEVC Rebuild uses a separate `.hevc-rebuild.nfo` sidecar and replaces only its own marked files after a fresh preview. A metadata-only remux or full rebuild reads the nearest TV-show NFO plus adjacent NFOs, applies supported title/show/episode numbering, release date, certification, genres, summaries, source URL, and ratings to the new Matroska global tags, and validates those tags before installing output. Kodi's `userrating` is on a 0–10 scale; Matroska `RATING` is 0–5, so rebuilds divide the rating by two. Saving metadata sidecars never changes video bytes. When a user-owned standard NFO exists, other applications that only inspect canonical NFO filenames may not discover the separate app-managed sidecar.

The Rust-owned organizer is also available to CLI and MCP automation. CLI takes a JSON plan for preview and apply so automation can show the same before/after result before confirmation:

```bash
hevc-rebuild-studio organize scan "/path/to/TV/Show"
hevc-rebuild-studio organize preview rename-plan.json
hevc-rebuild-studio organize apply rename-plan.json --preview-id "<copy-previewId-from-preview-output>" --confirm
```

MCP exposes `scan_library_names`, `inspect_local_library_assets`, `preview_library_rename`, and `apply_library_rename`; apply requires `confirmRename=true` and the current `previewId`. `inspect_local_library_assets` reads local NFO fields and returns paths for supported adjacent artwork under the supplied root, without contacting providers. Organizer parameter keys use camelCase (`sourcePath`, for example). `save_library_artwork` writes a confirmed provider image to reviewed sidecar targets; `cache_library_artwork` saves a confirmed provider image to the private app cache. These tools fetch or copy image bytes only when explicitly called. The Rust CLI accepts the sidecar artwork request in a JSON file:

```bash
hevc-rebuild-studio metadata artwork save artwork-request.json
```

The JSON request contains the selected library `root`, an `ArtworkCandidate` returned by a metadata lookup, and `targets` entries with an existing `directory` and safe filename `basename`. To inspect local metadata/artwork from CLI or MCP, pass the selected `root`, `mediaPaths`, and `directoryPaths` to `metadata assets inspect REQUEST.json` or `inspect_local_library_assets`. The read-only result includes parsed local NFO fields and matching image paths, not image bytes. Explicitly saved sidecars and private-cache images can be selected for MKV output; attachment depends on `metadata.embed_artwork`. Alternate titles, multi-episode/special/date-based matching, full Kodi field coverage, and complete provider/embedded-tag parity remain in progress.

The workbench and automation interfaces can preview and save metadata documents with the same request. CLI: `hevc-rebuild-studio metadata write-preview metadata-request.json`, then `hevc-rebuild-studio metadata write-apply metadata-request.json --preview-id ID --confirm`. MCP provides `preview_library_metadata_write` and `apply_library_metadata_write`; apply needs `confirmWrite=true` and the exact returned `previewId`. Request JSON has a selected `root` and a `documents` array. Each document includes `sourcePath`, `kind` (`movie`, `tv_show`, or `tv_episode`), `title`, `year`, `certification`, `shortDescription`, `plot`, `genres`, `rating`, `sourceUrl`, `showTitle`, `season`, `episode`, and `airdate`; unused values are `null`, and `genres` is an array. A TV request contains a show document plus one document for each selected, reviewed episode.

MCP artwork operations are `save_library_artwork` (confirmed sidecars), `cache_library_artwork` (confirmed provider image in the private cache), and `cache_local_library_artwork` (confirmed local image in the private cache; input is an object with `source_path`). `inspect_artwork_cache` reports cache usage, and `purge_unused_artwork_cache` requires its current `preview_id` plus `confirm_purge=true`. The run tool accepts returned cache paths in `file_poster_overrides`; `metadata.embed_artwork` controls attachment to the output MKV.

A CLI rename plan is a JSON object with `root` and an `items` list. Each item has `sourcePath`, `kind`, `template`, and `values`:

```json
{
  "root": "/media/Movies",
  "items": [
    {
      "sourcePath": "/media/Movies/old-name.mkv",
      "kind": "movie",
      "template": "{title} [[({year} {certification})]]{ext}",
      "values": {"title": "Example Film", "year": "2024", "certification": "PG-13"}
    }
  ]
}
```

### Command line and automation

The packaged desktop executable also exposes the existing engine CLI and a JSON bridge:

```bash
hevc-rebuild-studio cli --help
hevc-rebuild-studio cli "/path/to/Media" --recursive --info-only
hevc-rebuild-studio json environment
hevc-rebuild-studio json defaults --root "/path/to/Media"
hevc-rebuild-studio json scan --root "/path/to/Media" --recursive --min-size-mib 5
hevc-rebuild-studio json reports --limit 20
hevc-rebuild-studio json apt-updates
hevc-rebuild-studio json apt-refresh --confirm
hevc-rebuild-studio json apt-upgrade --confirm --package ffmpeg --package mediainfo
```

### Create short test clips

`tools/extract_clip.py` is a standalone Python 3 utility for making bounded
fixtures on Linux, macOS, and Windows. It never writes the source file,
refuses to overwrite an existing destination, writes a temporary file beside
the requested output, and publishes it only after FFmpeg succeeds. The
default `copy` mode is fast, preserves all mapped streams that the output
container supports, and drops chapters whose timestamps would no longer be
trustworthy. Stream-copy seeking starts at a nearby keyframe, so the requested
start and length are bounds rather than frame-accurate cuts. Use `transcode`
mode for an accurate start; it creates a short H.264/AAC fixture and may
change HDR, codec, or other encoding characteristics.

```bash
# Linux/macOS
python3 tools/extract_clip.py "/path/to/input.mkv" --duration 30s
python3 tools/extract_clip.py "/path/to/input.mkv" --start 2m --duration 45s \
  --output ./fixtures/input-45s.mkv
python3 tools/extract_clip.py "/path/to/input.mkv" --start 12s --duration 8s \
  --mode transcode --output ./fixtures/input-accurate.mkv --json

# Windows PowerShell or cmd.exe (single line)
py -3 tools\extract_clip.py "D:\Media\input.mkv" --duration 30s --output .\fixtures\input-30s.mkv
```

Without `--output`, the utility writes a derived `.mkv` name in the current
directory. `--output-dir` selects an existing directory, `--ffmpeg` selects an
explicit executable, and `HEVC_REBUILD_FFMPEG` is the optional environment
override. The same command can be run after a project-scoped copy to the
macOS VM or Windows test environment; do not sync private media or rely on the
Remote_Build macOS adapter to imply Windows support. The Remote_Build safety
contract requires a separate authorization before touching its `WinServer`
domain.

`json environment`, `json defaults`, `json scan`, the read-only `json apt-updates` check, `json reports`, `json apt-refresh`, and `json apt-upgrade` run entirely in Rust. They inspect hardware/tool capabilities, resolve effective settings, discover and profile media with FFprobe plus optional MediaInfo, produce memory/YouTube forecasts, compare installed packages against the local APT index, read local CSV report history, explicitly refresh the catalog, and simulate/upgrade the exact confirmed package list. Rust-launched rebuilds now use the same Rust FFprobe/MediaInfo source profiler and build profile details for only the selected video tracks; Python adapts these results for engine stages still in migration. Rust-launched GUI, CLI, and MCP media runs also use native helpers for bounded-memory stream fingerprints, per-track static HDR and Dolby Vision checks, HDR10+ verification/extraction, fixed-schema CSV appends, and no-overwrite commit/rollback. Python still prepares report values, edits Matroska metadata, and verifies expected tags/posters; GUI tool setup installs the managed HDR10+ helper through the native Rust path, while direct Python engine runs retain the full compatibility path. The Rust JSON APT actions share the GUI/MCP package policy; catalog refresh and upgrades remain explicit operations.

In the desktop app, reviewed package setup rechecks detected executables after changes and reports affected tools with before/after versions and paths. Live output is bounded to the latest 120 lines. Per-operation APT and HDR10+ helper logs are stored with application state, can be reopened from **Settings → Environment & capabilities**, and can be exported as `.log` or `.txt`. On Linux they are private to the current user; AppImage logs stay in the writable sibling `.data/logs/tool-setup/` folder, while installed builds use the user's state directory. A log marks any line clipped at 32 KiB and is marked if it reaches the 16 MiB per-operation limit. **Reports & errors → Delete history** permanently removes saved run reports and their companion diagnostic logs from this app’s log folder. It leaves package and HDR10+ setup logs in place. The button asks for a second confirmation before anything is deleted.

Use `--metadata-text-blacklist` more than once to add patterns for a rebuild. Patterns are case-insensitive substrings; `*` matches any sequence. For example, `Movie*Group` matches `movieReleaseGroup` and `Movieshowgroup` wherever that text appears, removing only the matched portion. In **Settings → Title cleanup**, the GUI keeps one pattern per removable chip, supports removing an individual rule or clearing all, and applies the active rules to imported title clues and provider search phrases as well as rename proposals and output metadata. Original source filenames are never changed during import. Save **app defaults** to keep edits across launches; MCP accepts the list in `settings_json.metadata.text_blacklist`. Do not enter a comma-separated list.

```bash
hevc-rebuild-studio cli "/path/to/Media" --recursive \
  --metadata-text-blacklist 'Movie*Group' \
  --metadata-text-blacklist 'ExplicitWord'
```

The equivalent TOML setting is one pattern per array entry:

```toml
[metadata]
text_blacklist = ["Movie*Group", "ExplicitWord"]
embed_artwork = true
```

`embed_artwork` controls attaching the selected local poster to metadata-only and rebuilt MKV outputs. Poster sidecar/cache storage is a separate application preference, so enabling both stores an external poster and embeds it.

The Rust metadata CLI searches only public providers and prints one JSON document. Search phrases can contain spaces; the detail commands take a reviewed provider ID:

```bash
hevc-rebuild-studio metadata movie search "Gnomeo & Juliet 2011"
hevc-rebuild-studio metadata movie details Q123
hevc-rebuild-studio metadata tv search The Expanse
hevc-rebuild-studio metadata tv details 123
hevc-rebuild-studio metadata write-preview metadata-request.json
hevc-rebuild-studio metadata write-apply metadata-request.json --preview-id ID --confirm
```

If Wikidata returns no film candidates for a query ending in a year, the CLI and MCP search retry with that trailing year removed. The returned matches remain suggestions for manual review.

The upgrade command is an example: use it only if both names appear in the current `apt-updates` result; otherwise repeat `--package` with the exact package names reported on this system.

The YouTube profile is available through the same engine CLI for a single file or a folder:

```bash
hevc-rebuild-studio cli "/path/to/film.mkv" --youtube-upload --upload-resolution 4k
hevc-rebuild-studio cli "/path/to/TV" --recursive --youtube-upload --upload-resolution 1440p
hevc-rebuild-studio cli "/path/to/film.mkv" --youtube-upload --upload-resolution 1440p --upload-quality-mode recommended
hevc-rebuild-studio cli "/path/to/film.mkv" --youtube-upload --upload-resolution 1440p --upload-source-multiplier 1.5
hevc-rebuild-studio cli "/path/to/film.mkv" --youtube-upload --upload-output-dir "/path/to/Upload Queue"
# Only after reviewing this specific analyzed file: HDR10+ dynamic metadata will be dropped.
hevc-rebuild-studio cli "/path/to/hdr10plus-film.mkv" --youtube-upload --allow-youtube-hdr10plus-loss "/path/to/hdr10plus-film.mkv"
```

For **Balanced size**, `--upload-source-multiplier` accepts `0.5`–`4.0` (default `2.0`) and adjusts the video bitrate target in proportion to the source's average total bitrate. YouTube's published video bitrate remains the ceiling. The setting is ignored by `--upload-quality-mode recommended`; estimates are still forecasts, not hard size limits.

`source` keeps input dimensions. The default `1440p` and optional `4k` profiles upscale smaller frames until the short side reaches the selected tier (height for landscape, width for portrait), while preserving aspect ratio; wider-than-16:9 sources can therefore have a wider output frame. A source already at or above the selected tier is kept at its current dimensions rather than downscaled. Balanced size mode is the default and estimates output size during analysis; `--upload-quality-mode recommended` uses YouTube's published bitrate guidance and may create much larger files. That guidance is not a required bitrate limit, and the estimate is not a hard file-size cap. YouTube recommends 16 Mb/s SDR or 20 Mb/s HDR for 1440p at 24–30 fps, and 35–45 Mb/s SDR or 44–56 Mb/s HDR for 4K; 48–60 fps recommendations are higher. This app deliberately does not force those rates on every small source. YouTube's help page notes that high-resolution versions can take longer to process, and that 1440p/4K dimensions do not guarantee a specific codec, playback tier, or quality improvement. HDR remains HDR when source tags are complete: PQ/HDR10 and HLG use 10-bit HEVC Main 10, while SDR uses H.264. Dolby Vision, unsupported HDR tags, or missing 10-bit x265 support produce a per-file skip. HDR10+ dynamic metadata is optional to YouTube, but this exporter cannot preserve it: HDR10+ sources are skipped unless the user explicitly approves that loss for each individual file in the YouTube prep tab. CLI users repeat `--allow-youtube-hdr10plus-loss` with the reviewed absolute source path, and MCP callers pass the reviewed paths in `allow_youtube_hdr10plus_loss_paths`. Consent is run-scoped; approved copies retain static HDR10/PQ signaling. Review YouTube's [HDR upload requirements](https://support.google.com/youtube/answer/7126552?hl=en-GB), [recommended upload encoding settings](https://support.google.com/youtube/answer/1722171?hl=en-GB), [resolution guidance](https://support.google.com/youtube/answer/6375112?co=GENIE.Platform%3DDesktop&hl=en), and [processing-quality guidance](https://support.google.com/youtube/answer/71674?hl=en-GB) before export.

Source subtitle streams are deliberately left out of the MP4 upload copy; add YouTube-supported captions separately. The app records this omission per file. HDR10+ dynamic metadata is optional to YouTube but this exporter cannot retain it: HDR10+ sources are skipped by default. In the YouTube prep tab, approve the loss on each affected file before exporting it; the copy retains static HDR10/PQ signaling. CLI users must repeat `--allow-youtube-hdr10plus-loss` with that file's absolute path. MCP callers pass only reviewed paths in `allow_youtube_hdr10plus_loss_paths`. Consent is run-scoped and never saved in preferences. PGS-to-SRT OCR remains a separate planned workflow because OCR can introduce text errors.

Existing preference files that contain the previous default `source` choice but no YouTube size profile are migrated once to the new 1440p/Balanced defaults. Select **Keep source size** again to retain the original dimensions.

In the Media library, each analyzed file has a **Content tune** control beside its filename: **Automatic** follows that source's saved tune and metadata detector, **Animated** forces x265's Animation tune for that file, and **Normal** disables the content tune for that file. When automatic animation detection is enabled, a reviewed provider genre of Animation or Anime is passed as per-file evidence alongside source metadata. Explicit file choices take precedence. These choices apply to check, test, and rebuild runs and are recorded in the per-file report. The CLI/MCP accepts equivalent per-file overrides as a map from absolute source paths to `animation` or `none`.

Expand **Tracks & stream metadata** to choose which streams each rebuild includes. The initial selection follows the saved secondary-video/data policies and includes the normal audio, subtitle, and attachment tracks. Each track exposes its parsed summary and expandable raw FFprobe fields; container format, chapter data, and MediaInfo enrichment are shown when available. If MediaInfo is unavailable, the scan labels the profile partial and the toolchain gate prevents a run that requires it. Once a file is customized, all included real-video tracks are encoded to HEVC; selected non-video tracks are copied. Keep both 4K and 1080p selected to include both versions in one MKV. At least one real video track is required. A fresh analysis clears the run-scoped stream maps because stream indices may have changed.

The Environment & capabilities page lets you set a path for FFmpeg, FFprobe, MediaInfo, `mkvpropedit`, `mkvmerge`, and `mkvextract`. Blank fields use PATH. Native Rust checks these binaries and encoder capabilities for the GUI, MCP `inspect_environment`, and `json environment`; the CLI accepts `--ffmpeg-path`, `--ffprobe-path`, `--mediainfo-path`, `--mkvpropedit-path`, `--mkvmerge-path`, and `--mkvextract-path`. Paths are persisted in application settings and version-checked when you recheck the environment.

`cli` forwards the engine's normal options, including setup and rebuild controls. `--metadata-only` remuxes all streams into a new MKV, applies reviewed metadata/poster overrides, and validates copied payloads without video encoding; existing destinations are rejected and the original stays in place. For app-launched runs, Rust supplies the discovered input batch and resolves/smoke-checks the selected toolchain through private helper actions; these helpers do not install or update packages. Rust discovery stops at 10,000 files and refuses to process a partial batch if that limit is reached. Direct Python invocations retain their compatibility discovery and toolchain paths. Rust's `cli` launcher passes its own executable path to the engine as the bounded native helper; the user does not need to invoke helper actions directly. `--info-only` prints each file's resource forecast and planned worker/frame counts without encoding. `json scan` includes a `resource_forecast` for rebuilds plus YouTube size and output-dimension memory forecasts for each analyzed file, target tier, and size mode. MCP analysis returns the same per-file data in pages. `json` supports `environment`, `scan`, `defaults`, `reports`, `apt-updates`, `apt-refresh --confirm`, `apt-upgrade`, and the explicit `install-hdr10plus` helper action. `reports` reads existing local CSV history natively; Python still creates run reports. `apt-updates` compares installed and candidate versions from the local package index; it does not contact repositories or install packages. `apt-refresh --confirm` explicitly refreshes that index and does not install packages. `apt-upgrade --confirm --package NAME` upgrades only the exact unique package list currently reported by `apt-updates`; repeat `--package` for every listed name, and the command fails if the catalog is stale, the list changed, the simulation includes additional packages/removals, or APT fails. It does not refresh the index automatically. JSON results go to stdout; scan progress and diagnostics go to stderr. Successful JSON actions return one JSON document. Environment inspection, defaults, reports, scanning, and package version comparison are read-only. APT setup, catalog refresh, package upgrades, and HDR helper installation are deliberate commands.

HDR10+ helper installation runs natively in Rust for the GUI, JSON CLI, and MCP after an explicit user action. It fetches the official latest GitHub release, requires a SHA-256 digest or checksum sidecar, caps the archive size, reads only the named executable from the tarball, checks the staged binary's version, and atomically replaces the managed helper. Automatic helper update selection and Python engine-start dependency resolution remain part of the migration backlog.

### MCP server

HEVC Rebuild Studio includes a local MCP server over stdio in the same executable. Add it to an MCP host configuration using the installed command on `PATH`:

```json
{
  "mcpServers": {
    "hevc-rebuild": {
      "command": "hevc-rebuild-studio",
      "args": ["--mcp"]
    }
  }
}
```

The server exposes typed tools for environment/settings inspection, folder analysis, media runs and safe stop, reports/logs, dependency repair, HDR10+ helper installation, and the Ubuntu package-update workflow. The helper installer requires `confirm_installation=true` and uses the native Rust checksum-verified path. `check_ubuntu_package_updates` compares installed/candidate versions using the local APT index and reports index staleness; it does not refresh or install anything. `refresh_ubuntu_package_catalog` requires `confirm_refresh=true` and only updates local package metadata. `upgrade_ubuntu_packages` requires `confirmed_package_names` to exactly match the current checked updates; it simulates the transaction and refuses extra package changes or removals before administrator authentication. No package update runs at launch. Folder analysis is asynchronous and returns a job ID; retrieve per-file results in pages of up to 250 entries, including probe errors, track-level metadata, and per-file YouTube size estimates. Runs default to check-only. Metadata-only mode remuxes all source tracks and accepts reviewed metadata/poster overrides without a video encoder; short tests and full rebuilds require explicit confirmation parameters. YouTube exports require `confirm_youtube_upload=true`, accept `youtube_resolution`, `youtube_quality_mode`, and an optional absolute `upload_output_dir`, and always write separate copies. The balanced size profile is the default. HDR10+ files are skipped unless the caller explicitly supplies each reviewed path in `allow_youtube_hdr10plus_loss_paths`; this approves loss of dynamic metadata for those files only. Rebuild runs accept `file_tune_overrides` (absolute paths to `animation`/`none`) and `file_stream_overrides` (absolute analyzed paths to unique FFprobe stream indices); metadata/test/full accept `file_metadata_overrides` and `file_poster_overrides` for reviewed per-file tags and local artwork. Environment inspection, analysis, and runs accept `tool_paths` for FFmpeg, FFprobe, MediaInfo, and MKVToolNix executables. Safe stop finishes the current file. Package repair is limited to the exact missing-package list reported by environment inspection. Environment output distinguishes full rebuild readiness from YouTube export readiness.

`start_media_run` also accepts optional `youtube_source_multiplier` from `0.5` through `4.0` (default `2.0`) for Balanced size. It is capped by YouTube's published video-rate guidance and does not impose a hard output byte ceiling.

Public metadata tools are also available: `search_public_films`, `get_public_film_metadata`, `search_public_tv_series`, and `get_public_tv_series_metadata`. They send only the requested title phrase or a reviewed provider ID and do not read media files. TMDB lookup stays in the GUI, where suggestions follow the scanned movie or series record and can also be started from that record; its user token is never passed to an MCP host.

The MCP transport uses stdin/stdout rather than opening a network listener. Keep MCP enabled only in the local AI host where you want to grant access to your machine's media-analysis and run controls. The CLI and MCP server are available from both Ubuntu desktop outputs; Windows and macOS remain future targets.

### Develop or build on Ubuntu

Install Tauri's Linux build prerequisites first. Follow the official [Tauri Linux prerequisites](https://v2.tauri.app/start/prerequisites/) for `libwebkit2gtk-4.1-dev`, GTK, Rust build tools, OpenSSL, and related system libraries. Python 3 is still required by the current media engine and its compatibility stages; native Rust environment/resource checks do not require Python.

For the frontend, Node 24 LTS is the recommended toolchain (`.nvmrc`); the project supports Node 22.17+, 24, and 26. `desktop/package.json` pins pnpm 12.10.1 and declares compatible-major dependency ranges, while the committed pnpm lockfile fixes the exact resolved dependency versions. SvelteKit 3 is configured in `desktop/vite.config.js`; there is no `svelte.config.js`. `pnpm check` uses TypeScript 6; `pnpm check:tsgo` runs the additional TypeScript 7.0.2 native compiler path.

From the extracted project directory:

```bash
cd desktop
pnpm install --frozen-lockfile
pnpm tauri dev
```

Build the Ubuntu installer and portable AppImage:

```bash
pnpm tauri build
```

The configured outputs are under `desktop/src-tauri/target/release/bundle/deb/` and `desktop/src-tauri/target/release/bundle/appimage/`. After a successful Linux release build, copy the `.deb` and AppImage into the repository-root `App/` folder for easy retrieval. The `.deb` declares Python 3 and the normal GTK/WebKit runtime dependencies. The AppImage is a single-file GUI distribution but still requires system Python 3 and the external media tools that the engine inspects/repairs; it does not bundle FFmpeg, MediaInfo, or MKVToolNix. Its Rust launcher removes linuxdeploy's inherited `PYTHONHOME` before starting host Python, so the host standard library remains discoverable. The current AppImage is for Ubuntu releases with glibc 2.43 or newer; build on Ubuntu 24.04 before advertising AppImage support for Ubuntu 24.04. Make it executable and run it directly:

```bash
chmod +x "./<generated-file>.AppImage"
"./<generated-file>.AppImage"
```

The adjacent `<generated-file>.AppImage.data/` folder is the portable profile and should travel with the AppImage. Install the `.deb` with Ubuntu's package manager, for example `sudo apt install ./<generated-package>.deb`.

For source checks and a frontend build:

```bash
pnpm check
pnpm build
```

These commands run from `desktop/`. From the repository root, use `pnpm ui:check`, `pnpm ui:check:tsgo`, `pnpm ui:build`, or `pnpm ui:dev`; these shortcuts delegate to the desktop package and add no root JavaScript dependencies.

The app shell does not fetch remote fonts or scripts. After an explicit metadata lookup, the workbench may load posters from the selected provider's approved image hosts. The Rust bridge exposes fixed application operations and does not give the frontend a general shell command API. For platform packaging and backend boundaries, see `TECHNICAL.md`.

### Current desktop limitations

- Ubuntu `.deb` and x86-64 AppImage are configured as distributable targets. The current AppImage includes libraries requiring glibc 2.43 because the build host is Ubuntu 26.04.1; a build from an Ubuntu 24.04 baseline is still needed for AppImage users on 24.04. The `.deb` executable requires glibc 2.39. A built package and a Linux development run establish no Windows or macOS support.
- The engine and automated repair logic remain Ubuntu/Linux Mint-specific. Cross-platform packaging needs an engine/runtime plan and platform-specific dependency repair before those targets can be claimed.
- GPU detection currently reports Linux DRM vendor/device IDs; GPU encoding is intentionally not offered.
- Rebuilds now scrub clear release-group suffixes from output segment titles, global/track-targeted text tags, track names, and chapter titles before validation. The cleanup is conservative, runs on the temporary output, and records exact before/after values in each CSV row and the diagnostic log. A pre-run metadata preview is still needed; a copied or edited source is never changed by this stage.
- The metadata workbench stays associated with the current Media Library scan. It reads supported local movie/show/episode NFO fields and recognized sibling artwork as optional compatibility data. After a scan, enabled public sources are queried for items without usable local/reviewed metadata; exact normalized title/year matches without year/provider conflicts are automatically associated, and other top results stay suggestions. Opening an auto-associated item fetches full provider details. Match safety still needs a labeled real-library precision benchmark and native/high-volume acceptance. Remote movie artwork remains a preview until it is explicitly saved. A matched TV series poster, and each season poster returned for that series, is written beside the media when Save poster beside the media is on; an existing image is not replaced. A matched episode title, plot, and air date are saved on that file when those fields are still empty or still only the filename or series text. A season plot and audience score are saved when the provider publishes them for that season; a series score is not copied onto a season. Automatic association does not embed poster bytes. Broader embedded-tag matching, alternate titles, complex TV episode matching, full Kodi coverage, and complete Rust-engine parity remain open. Full-series rename still requires a complete local folder scan because analysis may filter smaller files.
- Rebuilds can use the validated in-place replacement flow or a configured default/per-file destination. Separate destinations retain originals, mirror the source folder structure for default output, and reject existing/colliding targets before encoding. The YouTube preparation tab has its own output destination.
- The MCP server currently exposes the application's operational surface as typed tools over local stdio; it does not run an HTTP endpoint.
- Short tests use the engine's fixed five-second duration and CRF 30; full rebuild uses the effective configured CRF (default 21).

### Preview output paths across a queue

The desktop review checks the selected files across all queued sources before a metadata-only copy, short test, or full rebuild. The same read-only planner is available through the Rust JSON CLI and MCP. Make `plan.json` with absolute source paths; destination folders must already exist:

```json
{
  "mode": "full",
  "sources": [
    { "root": "/media/Films", "files": ["/media/Films/Film A.mp4"] },
    { "root": "/media/TV", "files": ["/media/TV/Show/Season 01/Episode.mkv"] }
  ],
  "outputDir": "/media/Rebuilt",
  "fileOutputDirs": {}
}
```

Run `hevc-rebuild-studio json output-plan --request plan.json`. The JSON result lists the planned targets, whether the set is eligible, and existing-output or duplicate-target collisions. The response caps displayed targets at 250 and collision details at 1,000; all selected paths are still checked. MCP clients can pass the same fields to `preview_media_outputs`. Previewing does not read or modify media content. The engine repeats its per-source output check immediately before writing.

### Inspect and select saved queue files

`hevc-rebuild-studio json queue --offset 0 --limit 100` returns a compact page of the local queue, including scan status, selected state, short video/HDR summaries, and per-file run outcomes. The page size is limited to 200; raw scan payloads and poster data are omitted. To update the shared selection, write `selection.json` as `{"selectedMediaPaths":["/media/Films/Film.mkv"]}` and run `hevc-rebuild-studio json queue-select --request selection.json`. Every selected path must already exist in the saved scan queue; the operation changes selection only and preserves scan results and metadata decisions. MCP offers the same operations as `inspect_media_queue` and `set_media_selection`.

Inspect private poster-cache totals with `hevc-rebuild-studio json artwork-cache inspect`. The response separates queue-assigned posters, eligible unused app-managed images, and unrecognized entries, and includes a `previewId`. After reviewing it, run `hevc-rebuild-studio json artwork-cache purge --preview-id PREVIEW_ID --confirm`. A changed cache or queue assignment invalidates the preview. The purge removes only eligible poster files and their matching attribution files. MCP offers `inspect_artwork_cache` and `purge_unused_artwork_cache` with the same preview/confirmation boundary. This operation does not clear x265 calibration results; clearing those results would make eligible large-file jobs benchmark thread layouts again.

For automations that need one ordered batch across several analyzed roots, use `json queue-run` or MCP `start_media_queue_run`. Each source request uses the same fields as `start_media_run`, but must include its analyzed `root` and a non-empty explicit `selected_files` array. All source requests in one batch use the same mode; selected paths must be unique across roots. The complete batch is validated before its first source starts, and metadata/test/full destinations are checked together. A batch defaults to stopping before the next source after an error. Matching saved queue rows are synchronized from each source's own CSV report; missing rows remain `needs_review`, and later unstarted sources stay queued.

Example read-only two-source request (`queue-run.json`):

```json
{
  "runs": [
    {
      "root": "/media/Films",
      "mode": "check",
      "selected_files": ["/media/Films/Film A.mkv"]
    },
    {
      "root": "/media/TV",
      "mode": "check",
      "selected_files": ["/media/TV/Show/Season 01/Episode 01.mkv"]
    }
  ],
  "stop_on_error": true
}
```

Run `hevc-rebuild-studio json queue-run --request queue-run.json`. For `test` or `full`, put the matching `confirm_test_output: true` or `confirm_full_rebuild: true` on every run entry. Interrupted or `needs_review` files require report/output review and their exact paths in top-level `acknowledge_review_paths` before retry. MCP returns a job ID for `get_job` polling and supports `request_safe_stop`; the JSON CLI runs synchronously, writes progress to stderr, and leaves the final JSON summary on stdout.

## Install once

1. Extract the ZIP.
2. Open the extracted folder.
3. Right-click an empty area and choose **Open in Terminal**.
4. Copy/paste this:

```bash
chmod +x install.sh && ./install.sh
```

If Linux asks for your password, type your normal login password and press **Enter**. Nothing appears while typing the password. That is normal.

If required software is missing or has an update, the installer explains it. Press **Enter** to let it fix the software.

## Start the program

```bash
hevc-rebuild
```

If that says `command not found`:

```bash
~/.local/bin/hevc-rebuild
```

The guided mode asks for the media folder, what kind of run you want, and how to handle Dolby Vision files that cannot be preserved safely.

1. **Check only** - safest first step; nothing is changed.
2. **Short test videos** - creates short samples; originals stay untouched.
3. **Full rebuild** - processes the real files.

For Dolby Vision, Guided Mode now offers **Safe mode** or an explicit **HDR10 fallback**. The fallback is only allowed when FFprobe verifies a Dolby Vision base layer and the selected picture independently identifies as HDR10. It intentionally removes Dolby Vision from the rebuilt video; the untouched original is still retained after a successful full rebuild.

Use **Check only → Short test → Full rebuild** the first time.

**Check only** is read-only: it uses FFmpeg and FFprobe for inspection, does not repair or install tools, and does not require encoding helpers such as `hdr10plus_tool`. If an HDR10+ helper is missing, the report explains that dynamic-metadata verification was unavailable; the check can still complete.

## Settings

The first run automatically creates:

```text
~/.config/hevc-rebuild/config.toml
```

The defaults are recommended. You do not need to edit the file unless you want to change quality/speed behavior.

Open the settings file with:

```bash
xdg-open ~/.config/hevc-rebuild/config.toml
```

The important settings are explained directly inside the file. The main ones are:

```text
CRF 21           = recommended high quality
preset slow      = recommended compression/time balance
preset slower    = smaller files, MUCH slower
8 -> 10 bit      = optional; OFF by default
animation tune   = automatic when reliable metadata says Animation
grain tune       = manual option for real film grain
Dolby Vision     = safe preservation by default; Guided Mode can explicitly choose verified HDR10 fallback
```

You can see the active settings without processing anything:

```bash
hevc-rebuild --show-config
```

A specific media library can override the normal settings by putting a file named:

```text
.hevc-rebuild.toml
```

inside that library's root folder. Normal users do not need this.


### CPU, memory, storage, and the live display

#### Quick x265 hardware calibration

The default `[calibration].mode = "auto"` benchmarks only 4K-or-larger sources or files at least 20 GiB, and only when a matching result is not already cached. It encodes a short representative section to FFmpeg's null output, comparing up to two x265 worker/frame layouts by default. It reads the source but creates no media output. The calibration cache is machine- and workload-specific and expires after 24 hours; portable AppImage runs keep it in the writable sidecar cache folder.

Calibration changes only x265 worker/frame concurrency. It does not change CRF, preset, tune, bit depth, HDR behavior, or copied streams. The current profile-aware memory plan caps every candidate, and the same live RAM reserve, swap-pressure, and quality-neutral retry remain active during real encoding. A calibration candidate that crosses the live memory reserve or causes sustained swap-in plus full-memory stalls is stopped and rejected. Calibration is skipped if usable-memory telemetry is unavailable or already below the configured reserve. The first uncached large file may take extra time before its actual encode starts; later matching files reuse the measurement. Select `ask` to prompt in an interactive CLI when an eligible file has no fresh cached result. Select `always` to benchmark every processed file while CPU threading is `auto`; this bypasses the cache and can add repeated overhead. Select `off` to use the profile-aware plan without benchmarking.

The default `auto` mode starts from **all available logical CPU threads except one**, then may lower worker and frame parallelism for the file's resolution, target bit depth, chroma, selected x265 preset, available RAM, memory limit, and current memory pressure. The folder scan and CLI check-only output show the starting plan and, when parallelism can be reduced, the first quality-neutral retry plan for each file. If a file is already at one worker and one frame thread, they say that a pressure retry cannot lower parallelism further. `unlimited` skips the reserved-CPU policy but still uses the memory-aware cap. During a desktop/MCP run, the Rust observer publishes live Linux CPU, RAM, swap/PSI, process memory, temperature, and run-volume storage snapshots; the media engine still applies the active scheduling and safety decisions. High CPU use by itself is expected and is not treated as a failure.

Memory protection uses both current available RAM and total effective RAM, including finite cgroup-v2 limits where available. It keeps the larger of **3 GiB** or **8% of usable RAM** available by default, then uses a profile-scaled allowance to choose initial x265 concurrency. This forecast is a conservative scheduling heuristic, not an exact peak-memory prediction: x265 memory use also varies by build, content, and encoding details. If the live reserve is crossed, only the temporary encode is stopped and retried with lower x265 parallelism when a lower plan is available; at one worker and one frame thread, the run fails rather than repeating the same plan. The live monitor also shows swap occupancy and system-wide swap-in/out rates. Sustained swap-in traffic alongside full-memory stalls triggers the same quality-neutral retry; allocated swap alone does not, because old pages can remain in swap after pressure has passed. **CRF, preset, HDR handling, bit depth, and picture quality never change during a resource retry.** Swap is not counted as fast RAM or resized automatically; heavy paging can make 4K encoding extremely slow.

Storage is checked before **every file** and continuously while encoding. The program keeps an 8 GiB disk reserve by default, estimates a conservative temporary-output budget, and watches the projected final output size so it can stop safely before `ENOSPC`. Remember: the untouched original is kept under `old/`, so a successful rebuild initially uses *more* disk space until you later decide to remove the backup.

The generated config contains short explanations:

```toml
[calibration]
mode = "auto"
cache_hours = 24.0
large_file_gib = 20.0
sample_seconds = 3.0
max_candidates = 2
throughput_tolerance_percent = 3.0

[resources]
threading = "auto"
reserved_cpu_threads = 1
max_worker_threads = 0
frame_threads = 0

min_free_memory_gib = 3.0
memory_reserve_percent = 8.0

min_free_disk_gib = 0.0 # optional extra reserve; 0 uses media-derived working-space checks
preflight_output_ratio = 1.30
disk_projection_margin_percent = 15.0
```

Normal users should leave these values alone. `0` for the advanced x265 thread caps means automatic.

The CLI also accepts `--calibration-mode`, `--calibration-cache-hours`, `--calibration-large-file-gib`, `--calibration-sample-seconds`, `--calibration-max-candidates`, and `--calibration-tolerance-percent`; these same effective settings are accepted through the GUI profile and typed MCP run settings.

For a batch or TV series, files are processed **one at a time**. The terminal shows a queue summary, then a compact per-file card and a two-line live HUD that updates in place: file/queue progress, speed and ETA on the first line; CPU load, RAM, swap used and active swap traffic, encoder memory, free disk space, temperature, and meaningful I/O pressure on the second. Detailed FFmpeg/x265 output remains in the log instead of scrolling across the terminal.

The utility also records Linux PSI (Pressure Stall Information) for CPU, memory, and I/O in the diagnostic log. Per-file reports retain minimum available RAM, peak encoder RSS, peak system swap use, peak FFmpeg/x265 `VmSwap`, peak swap-in/out rates, and peak full-memory PSI across successful and memory-retried attempts. If FFmpeg receives `SIGKILL`, the utility distinguishes **confirmed kernel OOM evidence** from an **unexplained external kill** instead of guessing. Every failed file gets a concise `FAILURE SUMMARY` block in the log with the phase, exception, and resource snapshot.


## What Full Rebuild does

For each media file it:

- selects the highest-resolution real video track;
- encodes only that primary video with **CPU libx265**;
- defaults to **CRF 21, preset slow**;
- keeps the **source bit depth and chroma format** whenever x265 supports them exactly;
- refuses unusual bit depths it cannot reproduce exactly rather than silently changing precision;
- automatically uses x265's **animation tune** when trustworthy Kodi/embedded genre metadata identifies Animation;
- allows manual `animation`, `grain`, normal, or automatic tuning in the config;
- **copies every audio stream without transcoding**;
- **copies every subtitle stream without transcoding**;
- SHA-256 verifies copied compressed audio/subtitle packet data and codec-private data;
- encodes every selected real-video track to HEVC by default, preserving selected 4K and 1080p versions together in one MKV; users can choose to copy secondary tracks or keep only the primary track;
- preserves chapters, attachments, dispositions, language/title metadata, and timing;
- preserves existing Matroska global metadata and overlays Kodi `.nfo` metadata where available;
- uses poster artwork only when a matching local `-poster`/season poster exists;
- never invents generic poster artwork;
- preserves HDR10 mastering metadata, MaxCLL/MaxFALL, color primaries, transfer function, matrix, range, and chroma location;
- preserves **HDR10+ dynamic scene/frame metadata**, then verifies it after encoding;
- preserves HLG signaling;
- uses modern FFmpeg/libx265 native Dolby Vision RPU transcoding for **verified single-layer Profile 8** sources when the installed FFmpeg supports it;
- skips unsafe/unsupported Dolby Vision profiles in safe mode instead of silently damaging them;
- can explicitly rebuild verified Profile 7/8 HDR10-compatible base layers as HDR10 while stripping Dolby Vision;
- validates that HDR10 fallback outputs remain HDR10 and no longer carry Dolby Vision signaling;
- validates the completed MKV before touching the original.

Example after success:

```text
Movies/
├── Movie.mkv          <- rebuilt HEVC version
└── old/
    └── Movie.mkv      <- untouched original
```

**Do not delete `old/` until you have personally checked the rebuilt files.**

## Easy copy/paste examples

Check a folder only:

```bash
hevc-rebuild "/home/$USER/Videos" --recursive --info-only
```

Make short test encodes:

```bash
hevc-rebuild "/home/$USER/Videos" --recursive --test
```

Full rebuild:

```bash
hevc-rebuild "/home/$USER/Videos" --recursive
```

Create a metadata-only MKV copy with every track preserved and no video re-encoding:

```bash
hevc-rebuild "/home/$USER/Videos" --recursive --metadata-only
```

Force animation tuning for one run:

```bash
hevc-rebuild "/home/$USER/Videos" --recursive --tune animation
```

Use the slower preset for one run:

```bash
hevc-rebuild "/home/$USER/Videos" --recursive --preset slower
```

Temporarily promote 8-bit video to 10-bit:

```bash
hevc-rebuild "/home/$USER/Videos" --recursive --promote-8bit-to-10bit
```

Explicitly use a verified HDR10 base layer when unsupported Dolby Vision would otherwise be skipped:

```bash
hevc-rebuild "/home/$USER/Videos" --recursive --dolby-vision-policy hdr10-fallback
```

## Poster rules

For a movie:

```text
Movies/
├── Dune Part Two.mkv
└── Dune Part Two-poster.jpg
```

`.jpg`, `.jpeg`, `.png`, and `.webp` are accepted.

Series-root `folder.*` art and season-folder `poster.*` art are recognized. For TV seasons, season poster layouts from the original utility remain supported, for example:

```text
Show Name/
├── season01-poster.jpg
└── Season 01/
    └── S01E01.mkv
```

The rebuild recognizes matching local movie, show, and season poster names and attaches the selected image to the new Matroska output when `metadata.embed_artwork` is enabled, retaining source/license details in the attachment description when an adjacent `.attribution.json` file exists. An episode in a season folder uses that season's poster when one is on disk, including when the saved review still holds the provider image URL. The series poster is used for that episode only when the season has no local poster. Poster downloads and renames create sidecars only; they do not edit video bytes. If no matching local poster exists, the tool does nothing to invent artwork. Existing embedded artwork copied from the source remains intact.

In the desktop queue, an explicitly selected local movie/series/season image takes precedence for that media item during metadata-only, short-test, and full-rebuild output. Provider artwork remains a preview until you confirm saving it beside the media or into the private application artwork cache; the returned path is then associated with that exact review. The independent `metadata.embed_artwork` setting decides whether a selected poster is attached to the MKV. Rust requires the selected image to be a valid JPEG, PNG, or WebP no larger than 16 MiB and contained in either the analyzed library or the canonical private app cache. The local CLI accepts the equivalent `--file-poster-overrides-json` mapping of absolute media paths to image paths; reviewed tag fields use `--file-metadata-overrides-json`. MCP `start_media_run` accepts these as `file_poster_overrides` and `file_metadata_overrides`. Metadata-only output creates a new validated MKV and retains the source. YouTube output remains a separate workflow and does not consume these overrides.

## Dolby Vision behavior

The default setting is:

```toml
dolby_vision = "safe"
```

This means:

- verified **single-layer Profile 8 + RPU** can be preserved when the installed FFmpeg/libx265 exposes native Dolby Vision RPU encoding;
- Profile 5, Profile 7/FEL/MEL, enhancement-layer sources, unknown DV layouts, or systems with older FFmpeg are **skipped**;
- the tool does not pretend that throwing away an enhancement layer is lossless;
- an expert can explicitly choose `hdr10-fallback`, and Guided Mode exposes the same choice;
- fallback requires FFprobe-verified Dolby Vision base-layer presence plus independent HDR10 identification;
- on FFmpeg builds that expose `-dolbyvision`, fallback forces `-dolbyvision 0` so the encoder's `auto` mode cannot accidentally carry RPU metadata into the HDR10 output;
- validation rejects fallback output if Dolby Vision signaling remains.

Safe mode remains the default. The fallback is intentional metadata reduction, not a lossless Dolby Vision conversion.

## If something fails

The program is designed to fail safely. If encoding, HDR/HDR10+/Dolby Vision preservation, stream-copy verification, metadata verification, or final validation fails, the original is not replaced.

At the end it prints the log and report locations. Keep those files if you need help diagnosing a problem.

Each run's log and CSV report share a unique identifier, so runs started within one second stay separate. New engine logs/reports are private (`0600` on Linux), and newly created log directories use `0700`; existing artifact permissions stay intact. Direct Python and Rust writers serialize CSV appends and reject incompatible headers. Successful report writes sync the file and, on creation, its directory; a sync failure explicitly reports that the row was written but durability is unconfirmed.

External inspection helpers normally have a five-minute timeout; full-media MKV/HDR helpers may run up to 24 hours. Timeout/cancellation terminates the owned process group and retains diagnostics. This helper limit does not cap active encodes, which retain their live memory/disk guards. Retained streaming diagnostics are bounded and label truncation. Malformed numeric metadata is shown as unknown or rejected; invalid external JSON reports the failing boundary.

## Supported systems

- The standalone `tools/extract_clip.py` fixture utility runs on Linux, macOS,
  and Windows when Python 3 and FFmpeg are available. That utility's portability
  does not extend the desktop application's supported-target statement.
- Ubuntu 24.04 LTS or newer
- Ubuntu-based Linux Mint 22 or newer

For deeper implementation details, see `TECHNICAL.md` and `REVIEW.md`.

## License and project ownership

The supplied source archive did not include a license file or maintainer/repository contact. The desktop package is marked private and is not intended for publication; confirm the project's license and release ownership before redistributing it.

### Transaction and safe-stop guarantees (2026-10-11)

Direct Python and native Linux validated-output commits use atomic no-overwrite moves, including the original backup and rollback. A failed final directory sync reports that installation occurred and retains the new output and original backup. Organizer rename success now requires directory sync; a failure reports applied paths retained and unconfirmed durability. Organizer rollback covers operation-time failures only. A persistent journal, undo, and power-loss recovery remain a separate design requirement; this maintenance does not claim crash recovery. Non-Unix directory sync remains an unverified no-op on unsupported targets.

Desktop and MCP safe-stop requests use unique private per-job control directories (0700 on Unix) and exclusively created markers (0600), refusing symlinks. Direct CLI `--stop-request-file` retains its path interface and accepts an owned regular marker created or changed after that run starts. Stale markers and writable-by-others markers are ignored. Safe stop still finishes the current file and leaves later files untouched.

### Reproducible maintenance gate

From the repository root, run `python3 -m compileall -q hevc_rebuild`, `python3 -m unittest discover -s tests -q`, and `ruff check --isolated --select F401,F841 hevc_rebuild tests` (verified with Ruff 0.16.8). Optional native integration tests require `HEVC_REBUILD_NATIVE_HELPER` pointing to the locally built executable and use generated fixtures. No runtime Python package is required. From `desktop/src-tauri`, run `cargo fmt --all -- --check`, `cargo test --locked --lib`, `cargo check --locked --bin hevc-rebuild-studio`, and `cargo clippy --all-targets --locked`. From `desktop`, run `pnpm check`, `pnpm check:tsgo`, `pnpm test:metadata-matching`, and `pnpm build`.

Formatting, compiler checks, focused unused-symbol lint, and tests are required source gates. Clippy must execute successfully; its remaining advisories and broader Ruff style findings are tracked in `2026-10-11_warnings.md` rather than suppressed globally. Stronger warning gates require a reviewed parity decision. These checks do not establish native keyboard/accessibility, packaged Ubuntu-baseline, representative HDR media, upload, or unsupported-platform acceptance.

The desktop component acceptance commands `pnpm test:home-help`, `pnpm test:library`, `pnpm test:rename`, `pnpm test:run`, `pnpm test:youtube`, and `pnpm test:support` run isolated synthetic browser fixtures using the pinned Playwright CLI test tool. They require a locally available Chrome browser and may download that test tool through npx. Their callback checks simulate native boundaries; use the later native/package acceptance gate for real desktop behavior. See `desktop/UI_OWNERSHIP.md`. YouTube preparation creates normal-upload copies; [Live HLS ingestion](https://developers.google.com/youtube/v3/live/guides/hls-ingestion) has separate transport and encoder requirements. Local export policy checks do not prove actual upload processing or playback.

### Backend request boundaries

Rust domain modules retain scan, queue, report, transaction, library, and YouTube ownership. `run_arguments.rs` validates job paths and per-file overrides; `settings_arguments.rs` translates validated settings for both GUI and MCP. Python orchestration remains supported. Run shared synthetic parity fixtures with `HEVC_REBUILD_NATIVE_HELPER="$PWD/desktop/src-tauri/target/debug/hevc-rebuild-studio" python3 -m unittest discover -s tests -p test_cross_path_parity.py -v` after rebuilding the helper.

Python validated-output transactions live in `hevc_rebuild/transactions.py`; existing imports from `hevc_rebuild.app` remain supported. CLI orchestration still performs validation before commit, and direct runs retain their no-overwrite fallback.

### Synthetic scan performance

Set `HEVC_REBUILD_SCAN_TIMING=1` to include `scan_timings` in native scan JSON. These diagnostics cover backend progress handoff, not rendered frames. `python3 tools/benchmark_scan.py --helper desktop/src-tauri/target/debug/hevc-rebuild-studio --output /tmp/hevc-scan-benchmark.json` generates its own media and removes it afterward. The process-local profile cache retains at most 128 successful profiles and 16 MiB of serialized keys/values. Keys include canonical source identity, size, nanosecond mtime, Unix inode/ctime, tool identity/version, effective settings and policy revision. Changes invalidate reuse; errors and partial MediaInfo results are not cached. Live host/resource and YouTube forecasts are recomputed. Nothing is persisted.

Measured on this development host: 20 synthetic files took 4.335 s cold and 0.560 s warm in the same process (20 cache hits). This is fixture evidence, not a real-library or packaged performance guarantee. Run the explicit cache benchmark with `HEVC_REBUILD_SCAN_TIMING=1 cargo test --locked --lib measure_repeated_scan_profile_cache -- --ignored --nocapture` from `desktop/src-tauri`, using isolated XDG directories.
