// Rebuilds tests/golden/route-fixtures.json from a REAL `hevc-rebuild-studio json scan` of a
// synthetic three-file library (generated with ffmpeg lavfi; no user media). Run once, review the diff:
//   node tests/golden/make-route-fixtures.mjs <scan.json> <library-root>
// The scan output is sanitized (paths, timestamps, host data) so the stored fixture is deterministic.
import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
const [scanFile, libraryRoot] = process.argv.slice(2);
if (!scanFile || !libraryRoot) throw new Error('usage: make-route-fixtures.mjs <scan.json> <library-root>');
const ROOT = '/fixture/library';
let text = readFileSync(scanFile, 'utf8').replaceAll(libraryRoot, ROOT).replaceAll('/home/v1x0r', '/home/user');
const scan = JSON.parse(text);
const volatile = new Set(['modified_unix_ms', 'updated_at', 'disk_free_bytes']);
(function clean(node) {
  if (Array.isArray(node)) return node.forEach(clean);
  if (node && typeof node === 'object') {
    for (const key of Object.keys(node)) {
      if (volatile.has(key)) node[key] = key === 'disk_free_bytes' ? 500 * 1024 ** 3 : 1700000000000;
      else if (/(unique_?id|encoded_date|tagged_date)/i.test(key)) node[key] = 'fixture';
      else clean(node[key]);
    }
  }
})(scan);
// Entry sizes vary with the encoder build; pin them.
scan.settings.metadata.comment = 'Fixture comment';
scan.entries.forEach((entry, index) => { entry.size_bytes = (index + 1) * 1_500_000_000; });
scan.total_input_bytes = scan.entries.reduce((sum, entry) => sum + entry.size_bytes, 0);
const art = (id, host, name) => ({ id, provider: 'tmdb', title: name, imageUrl: `https://${host}/${id}.jpg`, thumbnailUrl: `https://${host}/${id}-thumb.jpg`, sourceUrl: `https://${host}/title/${id}`, creator: null, license: null, licenseUrl: null, attribution: null });
const film = scan.entries.find((entry) => entry.name.startsWith('Film'));
const episodes = scan.entries.filter((entry) => entry.name.startsWith('S01'));
const episode = (entry, number, title) => ({ season: 1, episode: number, title, airdate: `2020-01-0${number}`, summary: `Fixture summary ${number}`, sourceUrl: '' });
const seriesRoot = `${ROOT}/Show Name`;
const queue = {
  version: 1,
  sources: [
    { root: ROOT, status: 'ready', scanResult: scan, error: null, updatedAtMs: 1700000000000, changedFiles: [episodes[1].path], missingFiles: [], excludedPaths: [], entryOrder: [] },
    { root: '/fixture/second', status: 'failed', scanResult: null, error: 'Fixture scan failure: folder not readable', updatedAtMs: 1700000000000, changedFiles: [], missingFiles: [], excludedPaths: [], entryOrder: [] },
  ],
  selectedMediaPaths: [film.path],
  fileTuneOverrides: {}, fileStreamOverrides: {},
  movieReviews: { [film.path]: { title: 'Fixture Film', year: '2001', certification: 'PG', recordId: 'tmdb:1', shortDescription: 'A fixture film.', plot: 'A fixture film plot.', genresText: 'Drama', ratingText: '7.1', sourceUrl: 'https://fixture.invalid/title/film', artworkCandidates: [art('film', 'fixture.invalid', 'Film poster')], posterId: 'film', localPosterPath: `${ROOT}/Movies/poster.jpg` } },
  seriesReviews: { [seriesRoot]: { title: 'Fixture Show', year: '2020', certification: 'TV-PG', recordId: 'tvmaze:2', shortDescription: 'A fixture show.', plot: 'A fixture show plot.', genresText: 'Comedy', ratingText: '8.0', sourceUrl: 'https://fixture.invalid/title/show', artworkCandidates: [art('show', 'fixture.invalid', 'Show poster')], posterId: 'show', localPosterPath: '', localSeasonPosterPaths: {}, seasonRecords: {}, episodes: [episode(null, 1, 'The Pilot'), episode(null, 2, 'The Second')], episodeAssignments: { [episodes[0].path]: episode(null, 1, 'The Pilot') } } },
  matchSuggestions: { [episodes[1].path]: { kind: 'tv', status: 'ready', title: 'Fixture Show', year: '2020', provider: 'tvmaze', recordId: 'tvmaze:2', sourceUrl: 'https://fixture.invalid/title/show', description: 'A fixture show.', autoAssociated: true, artwork: art('show', 'fixture.invalid', 'Show poster'), error: '' } },
  runStates: { [episodes[1].path]: { status: 'failed', mode: 'full', message: 'Fixture failure: output did not validate', outputPath: null, reportPath: '/fixture/reports/run.csv', reportStatus: 'FAILED', updatedAtMs: 1700000000000 } }, activeRoot: ROOT, updatedAtMs: 1700000000000,
};
const prefs = {
  theme: 'light', windowState: { width: 1600, height: 1000, maximized: false }, lastPage: 'library', root: ROOT,
  toolPaths: { ffmpeg: null, ffprobe: null, mediainfo: null, mkvpropedit: null, mkvmerge: null, mkvextract: null },
  recursive: true, minimumSizeMib: '0', stopOnError: false, youtubeResolution: '1440p', youtubeQualityMode: 'balanced', youtubeSourceMultiplier: 2,
  youtubeMode: 'upscale', youtubeSdrCodec: 'h264', youtubePreset: 'slow', youtubeShrink: 'auto', youtubeQualityCheck: 'standard', youtubeOutputDir: null,
  rebuildOutputDir: null, fileOutputDirs: {}, filenameIncludePatterns: [], filenameExcludePatterns: ['old', '.*'], filenameExcludeDefaultsVersion: 2,
  metadataAutoLookup: false, metadataLanguage: 'en-US', metadataSources: { tmdb: true, wikidata: true, tvmaze: true, tvdb: false },
  organizerTemplates: { movie: '{title} [[({year} {certification})]]{ext}', series: '{series_title}', season: 'Season {season:02}', episode: 'S{season:02}E{episode:02}.{episode_title}{ext}' },
  savePosterBesideMedia: false, settings: null,
};
const fixtures = {
  root: ROOT, seriesRoot, scan,
  settingsLoad: { settings: scan.settings, sources: scan.settings_sources, user_config: '/fixture/config/settings.toml', work_directory: scan.work_directory },
  preferences: { preferences: prefs, settingsFile: '/fixture/config/app.json', hasSavedSettings: true, portable: false, storageWarning: null },
  queueLoad: { queue, queueFile: '/fixture/config/queue.json', warning: null },
};
writeFileSync(fileURLToPath(new URL('./route-fixtures.json', import.meta.url)), JSON.stringify(fixtures, null, 1) + '\n');
console.log(`fixtures: ${scan.entries.length} entries, queue sources ${queue.sources.length}`);
