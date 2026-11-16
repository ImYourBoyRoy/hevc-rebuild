// ./desktop/tests/route-golden/preview-preferences.mjs
// Route golden section (T10.3): browser-preview preferences: older saved shapes, legacy keys, corrupt storage and what a theme change writes.
// Serialized into the browser contract by route-golden-contract.mjs; it must stay self-contained.

export async function previewPreferences(ctx) {
  const { page, settle, fingerprint, snapshots, theme, width, keys, storageKey } = ctx;
  // Browser-preview preferences: older saved shapes, legacy keys, corrupt storage, and what a theme change writes.
  const legacyExclude = ['old', 'My Extra Folder'];
  const previewScenarios = {
    'prefs-legacy-object': { storage: { [storageKey]: JSON.stringify({ theme: 'hacker', lastPage: 'home', root: '/legacy/root', ffmpegPath: ' /legacy/ffmpeg ', recursive: false, minimumSizeMib: '9', youtubeResolution: 'source', youtubeSourceMultiplier: 9, youtubeMode: 'optimize', filenameExcludePatterns: legacyExclude, filenameIncludePatterns: ['keep'], metadataLanguage: 'fr-CA', metadataSources: { tmdb: false, tvdb: true }, organizerTemplates: { movie: '{title}{ext}', series: '{series_title} [[({year_range})]]' }, fileOutputDirs: { '/a.mkv': '/out', '/b.mkv': 5 }, bogus: true }) } },
    'prefs-legacy-keys': { storage: { 'hevc-rebuild-theme': 'hacker', 'hevc-rebuild.root': '/old/root', 'hevc-rebuild.ffmpeg': '/old/ffmpeg', 'hevc-rebuild.recursive': 'false', 'hevc-rebuild.minimumSizeMib': '12' } },
    'prefs-corrupt': { storage: { [storageKey]: '{not json' } },
    'prefs-valid-modern': { storage: { [storageKey]: JSON.stringify({ theme: 'ion', lastPage: 'settings', youtubeResolution: '4k', youtubeQualityMode: 'recommended', youtubeSdrCodec: 'hevc', filenameExcludePatterns: ['x'], filenameExcludeDefaultsVersion: 2 }) } },
  };
  for (const [name, scenario] of Object.entries(previewScenarios)) {
    await page.setViewportSize({ width: 1024, height: 900 });
    await page.evaluate((storage) => { sessionStorage.setItem('goldenMode', 'preview'); localStorage.clear(); for (const [key, value] of Object.entries(storage)) localStorage.setItem(key, value); }, scenario.storage);
    await page.reload();
    await settle();
    const stored = async () => page.evaluate(() => Object.fromEntries(Object.entries(localStorage).map(([key, value]) => [key, (() => { try { return JSON.parse(value); } catch { return value; } })()]).sort(([a], [b]) => (a < b ? -1 : 1))));
    for (const [label, key] of [['settings', 'Alt+4'], ['youtube', 'Alt+3']]) {
      await page.keyboard.press(key);
      const aria = await settle();
      snapshots[`preview:${name}:${label}`] = { dom: await fingerprint(), aria, storage: await stored() };
    }
    await page.getByRole('combobox', { name: 'Appearance', exact: true }).selectOption('breach');
    const aria = await settle();
    snapshots[`preview:${name}:theme-breach`] = { dom: await fingerprint(), aria, storage: await stored() };
  }
}
