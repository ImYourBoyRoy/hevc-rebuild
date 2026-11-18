// ./desktop/tests/route-golden/bridge.mjs
// Route golden section (T10.3): the fake Tauri bridge: blocks external requests and answers every native command from the fixtures.
// Serialized into the browser contract by route-golden-contract.mjs; it must stay self-contained.

export async function installBridge(page, FIXTURES, tinyPng) {
  // Nothing may leave the machine: fixture images get a pixel, every other external request is refused.
  await page.route((url) => url.hostname !== '127.0.0.1', (route) => {
    const url = new URL(route.request().url());
    if (url.hostname === 'fixture.invalid') return route.fulfill({ status: 200, contentType: 'image/png', body: tinyPng });
    return route.abort();
  });

  await page.addInitScript((fixtures) => {
    if (sessionStorage.getItem('goldenMode') !== 'seeded') return;
    const unknown = [];
    window.__goldenUnknownCommands = unknown;
    window.__goldenQueueWrites = [];
    const summarize = (command, args) => {
      if (command === 'save_library_queue') {
        const queue = args.queue;
        return { command, sources: queue.sources.map((source) => ({ root: source.root, status: source.status, entries: source.scanResult?.entries.map((entry) => entry.name) ?? null, excluded: source.excludedPaths.map((path) => path.split('/').at(-1)), order: source.entryOrder.map((path) => path.split('/').at(-1)) })), selected: queue.selectedMediaPaths.map((path) => path.split('/').at(-1)), movieReviews: Object.keys(queue.movieReviews).map((path) => path.split('/').at(-1)), seriesReviews: Object.keys(queue.seriesReviews).map((path) => path.split('/').at(-1)), runStates: Object.fromEntries(Object.entries(queue.runStates).map(([path, state]) => [path.split('/').at(-1), [state.status, state.mode, state.message, state.reportStatus]])), activeRoot: queue.activeRoot };
      }
      return { command, args: JSON.parse(JSON.stringify(args, (key, value) => (key === 'updatedAtMs' ? 0 : value))) };
    };
    const callbacks = new Map();
    const listeners = {};
    let nextId = 1;
    let runCounter = 0;
    window.__goldenRunCalls = [];
    window.__goldenWindowCalls = [];
    window.__goldenRepairCalls = [];
    window.__goldenPrefWrites = [];
    window.__goldenWindow = { scale: 2, maximized: false, inner: { width: 2800, height: 1700 } };
    window.__goldenReports = [];
    window.__goldenDialogPicks = [];
    window.__goldenPrefExtra = [];
    try { Object.defineProperty(navigator, 'clipboard', { value: { writeText: async (value) => { window.__goldenRepairCalls.push({ command: 'clipboard_write', value }); } }, configurable: true }); } catch { /* the golden then reports the native clipboard error */ }
    // The contract plays the native engine: it emits Tauri events to whatever the route registered.
    const settingsSetup = JSON.parse(sessionStorage.getItem('goldenSettingsSetup') || 'null') ?? {};
    window.__goldenCredentials = { tmdb: settingsSetup.tmdb ? { configured: true, token: 'fixture-tmdb-token' } : { configured: false }, tvdb: settingsSetup.tvdb ? { configured: true, apiKey: 'fixture-tvdb-key', pin: '1234' } : { configured: false } };
    window.__goldenArtworkCache = settingsSetup.cachePurgeable === undefined
      ? { available: false, imageCount: 0, totalFileCount: 0, totalBytes: 0, purgeableImageCount: 0, purgeableBytes: 0, protectedImageCount: 0, protectedBytes: 0 }
      : { available: true, imageCount: 6, totalFileCount: 12, totalBytes: 6144, purgeableImageCount: settingsSetup.cachePurgeable, purgeableBytes: settingsSetup.cachePurgeable * 1024, protectedImageCount: 4, protectedBytes: 4096, unmanagedEntryCount: 0, previewId: 'preview-1' };
    window.__goldenBaseEnvironment = () => clone(fixtures.scan.environment);
    window.__goldenEmit = (event, payload) => { for (const listener of listeners[event] ?? []) callbacks.get(listener.handler)?.({ event, id: listener.id, payload }); };
    const png = 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==';
    const clone = (value) => JSON.parse(JSON.stringify(value));
    const repairSetup = JSON.parse(sessionStorage.getItem('goldenRepairSetup') || 'null');
    if (repairSetup) {
      window.__goldenAptStatus = repairSetup.apt;
      window.__goldenSetupLogs = repairSetup.logs ? [{ jobId: 'job-toolchain', modifiedUnix: 1700000000, sizeBytes: 2048, truncated: false }] : [];
      if (repairSetup.helperMissing) {
        const environment = clone(fixtures.scan.environment);
        environment.checks.find((check) => check.key === 'hdr10plus_tool').available = false;
        window.__goldenEnvironment = environment;
      }
    }
    const clue = (filename) => {
      const stem = filename.replace(/\.[^.]+$/, "");
      const year = /\((\d{4})\)/.exec(stem)?.[1] ?? null;
      return { title: stem.replace(/\s*\(\d{4}\)/, "").replace(/[._]+/g, " ").trim(), year, yearEnd: null, certification: null };
    };
    const handlers = {
      'plugin:event|listen': (args) => { const id = nextId++; (listeners[args.event] ??= []).push({ id, handler: args.handler }); return id; },
      'plugin:event|unlisten': () => null,
      // A 2x display with a 1400x850 logical work area, so a saved 1600x1000 window is clamped on restore.
      'plugin:window|current_monitor': () => ({ name: 'fixture', position: { x: 0, y: 0 }, size: { width: 3000, height: 1800 }, workArea: { position: { x: 0, y: 0 }, size: { width: 2800, height: 1700 } }, scaleFactor: 2 }),
      'plugin:window|scale_factor': () => window.__goldenWindow.scale,
      'plugin:window|set_size': (args) => { window.__goldenWindowCalls.push({ command: 'set_size', value: args.value }); return null; },
      'plugin:window|maximize': () => { window.__goldenWindowCalls.push({ command: 'maximize' }); return null; },
      'plugin:window|is_maximized': () => window.__goldenWindow.maximized,
      'plugin:window|inner_size': () => window.__goldenWindow.inner,
      'plugin:resources|close': () => null,
      'plugin:updater|check': () => null,
      'plugin:app|version': () => '0.0.0-fixture',
      frontend_ready: () => null,
      active_job: () => null,
      load_app_preferences: () => {
        const payload = clone(fixtures.preferences);
        payload.preferences.theme = sessionStorage.getItem('goldenTheme') || 'light';
        return payload;
      },
      save_app_preferences: (args) => ({ ...clone(fixtures.preferences), preferences: args.preferences }),
      default_media_root: () => fixtures.root,
      load_library_queue: () => clone(fixtures.queueLoad),
      save_library_queue: () => null,
      save_library_queue_selection: () => null,
      save_library_queue_movie_reviews: () => null,
      save_library_queue_series_reviews: () => null,
      save_library_queue_metadata_suggestions: () => null,
      inspect_environment: () => clone(window.__goldenEnvironment ?? fixtures.scan.environment),
      load_settings: () => clone(fixtures.settingsLoad),
      resolve_work_directory_command: (args) => { window.__goldenRepairCalls.push({ command: 'resolve_work_directory_command', root: args.root, workRoot: args.workRoot }); return clone(fixtures.scan.work_directory); },
      list_reports: () => ({ reports: clone(window.__goldenReports) }),
      start_run: (args) => {
        const r = args.request;
        const byName = (record) => Object.fromEntries(Object.entries(record).map(([path, value]) => [path.split('/').at(-1), value]));
        window.__goldenRunCalls.push({
          command: 'start_run', mode: r.mode, root: r.root, selectedFiles: r.selectedFiles.map((path) => path.split('/').at(-1)), stopOnError: r.stopOnError,
          integrityLevel: r.integrityLevel, fullRebuildConfirmed: r.fullRebuildConfirmed, rebuildOutputDir: r.rebuildOutputDir, youtubeOutputDir: r.youtubeOutputDir,
          fileTuneOverrides: byName(r.fileTuneOverrides), filePosterOverrides: byName(r.filePosterOverrides), fileMetadataOverrides: byName(r.fileMetadataOverrides),
          fileSettingsOverrides: byName(r.fileSettingsOverrides), fileStreamOverrides: byName(r.fileStreamOverrides),
        });
        return `run-${++runCounter}`;
      },
      preview_queue_output_targets: (args) => {
        const request = args.request;
        const files = request.sources.flatMap((source) => source.files);
        window.__goldenRunCalls.push({ command: 'preview_queue_output_targets', mode: request.mode, files: files.map((path) => path.split('/').at(-1)), outputDir: request.outputDir });
        const collisions = window.__goldenCollisions ?? [];
        return {
          mode: request.mode, sourceCount: request.sources.length, fileCount: files.length, eligible: collisions.length === 0,
          targets: files.map((path) => ({ source: path, target: path.replace(/\.[^./]+$/, '') + (request.mode === 'test' ? '.test.mkv' : '.mkv'), destination: 'source_folder', replacesSource: request.mode === 'full', note: null })),
          omittedTargetCount: 0, collisions, collisionCount: collisions.length, omittedCollisionCount: 0, testOutputsMayReceiveUniqueSuffixes: request.mode === 'test',
        };
      },
      request_stop: (args) => { window.__goldenRunCalls.push({ command: 'request_stop', runId: args.runId }); return null; },
      request_abort: (args) => { window.__goldenRunCalls.push({ command: 'request_abort', runId: args.runId }); return null; },
      read_log: () => 'fixture log line',
      'plugin:dialog|open': (args) => {
        const o = args.options ?? args;
        window.__goldenRepairCalls.push({ command: 'dialog_open', title: o.title ?? null, directory: Boolean(o.directory), multiple: Boolean(o.multiple), defaultPath: o.defaultPath ?? null, filters: o.filters ?? null });
        const pick = window.__goldenDialogPicks.shift();
        return pick === undefined ? null : pick;
      },
      delete_reports: () => { const removed = window.__goldenReports.length * 2; window.__goldenRepairCalls.push({ command: 'delete_reports' }); window.__goldenReports = []; return removed; },
      open_file_location: (args) => { window.__goldenRepairCalls.push({ command: 'open_file_location', path: args.path }); return null; },
      cancel_scan: () => { window.__goldenRepairCalls.push({ command: 'cancel_scan' }); return null; },
      list_tool_setup_logs: () => clone(window.__goldenSetupLogs ?? []),
      read_tool_setup_log: (args) => 'fixture setup log for ' + args.jobId,
      export_tool_setup_log: (args) => { window.__goldenRepairCalls.push({ command: 'export_tool_setup_log', jobId: args.jobId, destination: args.destination }); return args.destination; },
      'plugin:dialog|save': () => '/tmp/hevc-setup.log',
      provision_plan: () => {
        window.__goldenRepairCalls.push({ command: 'provision_plan' });
        const apt = window.__goldenAptStatus ?? { packages: [], catalog_stale: false };
        const environment = window.__goldenEnvironment ?? fixtures.scan.environment;
        const helperMissing = !environment.checks.find((check) => check.key === 'hdr10plus_tool')?.available;
        const upgrade = apt.packages.map((item) => item.name);
        const refresh = Boolean(apt.catalog_stale);
        const privileged = refresh || upgrade.length > 0;
        return { manager: 'apt', blockers: [], advisories: [], refresh_catalog: refresh, install_packages: [], upgrade_packages: upgrade, helper: { action: helperMissing ? 'install' : 'current', installed: helperMissing ? null : '1.7.2', latest: null }, needs_password: privileged, password_prompts: privileged ? 1 : 0, nothing_to_do: !privileged && !helperMissing, steps: [], release_check_error: null };
      },
      provision_toolchain: (args) => { window.__goldenRepairCalls.push({ command: 'provision_toolchain', toolPaths: args.toolPaths }); return 'job-toolchain'; },
      tmdb_credential_status: () => ({ configured: window.__goldenCredentials.tmdb.configured, secureStoreAvailable: true, message: null }),
      save_tmdb_read_token: (args) => { window.__goldenRepairCalls.push({ command: 'save_tmdb_read_token', token: args.token }); window.__goldenCredentials.tmdb = { configured: true, token: args.token }; return null; },
      reveal_tmdb_read_token: () => { window.__goldenRepairCalls.push({ command: 'reveal_tmdb_read_token' }); return window.__goldenCredentials.tmdb.token ?? ''; },
      remove_tmdb_read_token: () => { window.__goldenRepairCalls.push({ command: 'remove_tmdb_read_token' }); window.__goldenCredentials.tmdb = { configured: false }; return null; },
      tvdb_credential_status: () => ({ configured: window.__goldenCredentials.tvdb.configured, subscriberPinConfigured: Boolean(window.__goldenCredentials.tvdb.pin), secureStoreAvailable: true, message: null }),
      save_tvdb_credentials: (args) => { window.__goldenRepairCalls.push({ command: 'save_tvdb_credentials', apiKey: args.apiKey, subscriberPin: args.subscriberPin, accessConfirmed: args.accessConfirmed }); window.__goldenCredentials.tvdb = { configured: true, apiKey: args.apiKey, pin: args.subscriberPin }; return null; },
      reveal_tvdb_credentials: () => { window.__goldenRepairCalls.push({ command: 'reveal_tvdb_credentials' }); return { apiKey: window.__goldenCredentials.tvdb.apiKey ?? '', subscriberPin: window.__goldenCredentials.tvdb.pin ?? null }; },
      remove_tvdb_credentials: () => { window.__goldenRepairCalls.push({ command: 'remove_tvdb_credentials' }); window.__goldenCredentials.tvdb = { configured: false }; return null; },
      check_ubuntu_package_updates: () => { window.__goldenRepairCalls.push({ command: 'check_ubuntu_package_updates' }); return clone(window.__goldenAptStatus ?? { packages: [], catalog_updated_at: 1700000000, catalog_stale: false, source: 'fixture', checked_at: 1700000000 }); },
      preview_library_rename: (args) => {
        window.__goldenRepairCalls.push({ command: 'preview_library_rename', root: args.root, items: args.items.map((item) => ({ source: item.sourcePath, kind: item.kind, template: item.template, values: item.values })), textBlacklistPatterns: args.textBlacklistPatterns });
        if (window.__goldenRenameConflict) return { previewId: 'pv-conflict', canApply: false, conflicts: [{ reason: 'A file named Renamed already exists.' }], items: [] };
        return { previewId: 'pv-1', canApply: true, conflicts: [], items: args.items.map((item) => ({ sourcePath: item.sourcePath, targetPath: item.sourcePath.replace(/[^/]+$/, 'Renamed ' + item.sourcePath.split('/').at(-1)), unchanged: false })) };
      },
      apply_library_rename: (args) => { window.__goldenRepairCalls.push({ command: 'apply_library_rename', previewId: args.previewId, confirmed: args.confirmed, root: args.root }); return null; },
      reply: (args) => { const r = args.response; window.__goldenRepairCalls.push({ command: 'reply', id: args.id, response: { handled: r.handled, supported: r.supported, page: r.page, page_label: r.page_label, run_mode: r.run_mode, booting: r.booting, scanning: r.scanning } }); return null; },
      'plugin:opener|open_url': (args) => { window.__goldenRepairCalls.push({ command: 'open_url', url: args.url }); return null; },
      inspect_app_artwork_cache: () => { window.__goldenRepairCalls.push({ command: 'inspect_app_artwork_cache' }); return clone(window.__goldenArtworkCache); },
      purge_unused_app_artwork_cache: (args) => {
        window.__goldenRepairCalls.push({ command: 'purge_unused_app_artwork_cache', previewId: args.previewId, confirmed: args.confirmed });
        window.__goldenArtworkCache = { ...window.__goldenArtworkCache, purgeableImageCount: 0, purgeableBytes: 0, imageCount: 4, previewId: 'preview-2' };
        return { deletedImageCount: 2, deletedBytes: 2048, failedFileCount: 0, message: 'Removed 2 unused posters.', remaining: clone(window.__goldenArtworkCache) };
      },
      scan_local_library_assets: (args) => ({
        directories: args.request.directoryPaths
          .filter((path) => path.endsWith('/Show Name') || path.endsWith('/Season 1'))
          .map((path) => ({ path, artwork: [{ name: 'poster.jpg', path: `${path}/poster.jpg`, role: 'poster' }] })),
        files: [],
      }),
      read_local_library_artwork_preview: () => ({ dataUrl: png }),
      scan_folder: () => clone(fixtures.scan),
      metadata_filename_clue: (args) => clue(args.filename),
      metadata_filename_clues: (args) => args.filenames.map(clue),
    };
    window.isTauri = true;
    window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
    window.__TAURI_INTERNALS__ = {
      metadata: { currentWindow: { label: 'main' }, currentWebview: { windowLabel: 'main', label: 'main' } },
      transformCallback(callback) { const id = nextId++; callbacks.set(id, callback); return id; },
      unregisterCallback(id) { callbacks.delete(id); },
      convertFileSrc: (path) => path,
      invoke: async (command, args) => {
        if (command.startsWith('save_library_queue')) window.__goldenQueueWrites.push(summarize(command, args ?? {}));
        if (command === 'save_app_preferences') { const p = args.preferences; window.__goldenPrefExtra.push({ toolPaths: p.toolPaths, youtubeOutputDir: p.youtubeOutputDir }); window.__goldenPrefWrites.push({ theme: p.theme, lastPage: p.lastPage, root: p.root, windowState: p.windowState, recursive: p.recursive, minimumSizeMib: p.minimumSizeMib, stopOnError: p.stopOnError, metadataLanguage: p.metadataLanguage, youtubeResolution: p.youtubeResolution, rebuildOutputDir: p.rebuildOutputDir, fileOutputDirs: p.fileOutputDirs }); }
        const handler = handlers[command];
        if (!handler) { unknown.push(command); return null; }
        return handler(args ?? {});
      },
    };
  }, FIXTURES);
}
