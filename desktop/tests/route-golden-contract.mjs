// Route-level golden snapshot (tasklist T10.3). Mounts the REAL +page.svelte and compares its
// accessibility tree and structural fingerprint with tests/golden/*.json in two modes:
//   preview: no Tauri bridge (browser-preview path: shell, navigation, empty states);
//   seeded:  a FAKE Tauri bridge answers every native command from tests/golden/route-fixtures.json
//            (a real `json scan` of a synthetic library, sanitized), so the route takes its desktop path with
//            a populated queue, reviews, posters, settings and environment.
// The seeded mode fails on any native command the fake bridge does not know, so new IPC cannot slip in unnoticed.
// Run: pnpm test:route-golden        Refresh after an intended UI change: pnpm test:route-golden -- --update
// This is browser evidence only: native dialogs, drag/drop, real IPC, screen readers and live runs are NOT covered.
import { readFileSync, writeFileSync, existsSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { runBrowserContract } from './browser-contract-runner.mjs';
import { installBridge } from './route-golden/bridge.mjs';
import { createPageHelpers } from './route-golden/page-helpers.mjs';
import { reviewFlows } from './route-golden/seeded-review-flows.mjs';
import { runLifecycle } from './route-golden/seeded-run-lifecycle.mjs';
import { launchFlows } from './route-golden/seeded-launch-flows.mjs';
import { reviewEditing } from './route-golden/seeded-review-editing.mjs';
import { windowPreferences } from './route-golden/seeded-window-preferences.mjs';
import { toolSetup } from './route-golden/seeded-tool-setup.mjs';
import { settingsFlows } from './route-golden/seeded-settings.mjs';
import { bootInput } from './route-golden/seeded-boot-input.mjs';
import { renameDialog } from './route-golden/seeded-rename.mjs';
import { helperFlows } from './route-golden/seeded-helpers.mjs';
import { queueEdits } from './route-golden/seeded-queue-edits.mjs';
import { previewPreferences } from './route-golden/preview-preferences.mjs';
const fixtures = readFileSync(fileURLToPath(new URL('./golden/route-fixtures.json', import.meta.url)), 'utf8');

async function contract(page) {
  const FIXTURES = __FIXTURES__;
  const keys = { library: 'Alt+1', run: 'Alt+2', youtube: 'Alt+3', settings: 'Alt+4', reports: 'Alt+5', help: 'Alt+6', metadata: 'Alt+m' };
  const storageKey = 'hevc-rebuild-studio.preview-preferences.v1';
  const tinyPng = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==', 'base64');
  const snapshots = {};

  await installBridge(page, FIXTURES, tinyPng);
  const { fingerprint, settle } = createPageHelpers(page);

  for (const mode of ['preview', 'seeded']) {
    for (const theme of ['light', 'dark']) {
      for (const width of [360, 1024]) {
        await page.setViewportSize({ width, height: 900 });
        await page.evaluate(([key, value, mode]) => {
          sessionStorage.setItem('goldenMode', mode);
          sessionStorage.setItem('goldenTheme', value);
          localStorage.setItem(key, JSON.stringify({ theme: value }));
        }, [storageKey, theme, mode]);
        await page.reload();
        await page.waitForSelector('body');
        await settle();
        for (const [name, key] of Object.entries(keys)) {
          await page.keyboard.press(key);
          if (name === 'metadata') await page.getByRole('heading', { level: 1 }).first().waitFor({ timeout: 20000 });
          const aria = await settle();
          const dom = await fingerprint();
          snapshots[`${mode === 'preview' ? '' : 'seeded:'}${name}|${theme}|${width}`] = { dom, aria };
        }
        if (mode === 'seeded') {
          const ctx = { page, settle, fingerprint, snapshots, theme, width, mode, keys, storageKey, tinyPng, FIXTURES };
          await reviewFlows(ctx);
          await runLifecycle(ctx);
          await launchFlows(ctx);
          await reviewEditing(ctx);
          await windowPreferences(ctx);
          await toolSetup(ctx);
          await settingsFlows(ctx);
          await bootInput(ctx);
          await renameDialog(ctx);
          await helperFlows(ctx);
          await queueEdits(ctx);
        }
      }
    }
  }
  await previewPreferences({ page, settle, fingerprint, snapshots, storageKey });
  return snapshots;
}

const goldenPath = fileURLToPath(new URL('./golden/route-golden.json', import.meta.url));
const update = process.argv.includes('--update');
// The browser contract is one serialized function, so the section functions are embedded in its body.
const sections = [installBridge, createPageHelpers, reviewFlows, runLifecycle, launchFlows, reviewEditing, windowPreferences, toolSetup, settingsFlows, bootInput, renameDialog, helperFlows, queueEdits, previewPreferences];
const prelude = sections.map((section) => section.toString()).join('\n\n');
const source = contract.toString().replace('{\n', () => `{\n${prelude}\n`).replace('__FIXTURES__', () => fixtures.trim());
const output = await runBrowserContract('route', { toString: () => source }, {
  server: 'tests/route-server.mjs',
  url: (port) => `http://127.0.0.1:${port}/`,
});
const match = output.match(/### Result\n([\s\S]*?)\n### /);
if (!match) throw new Error('Route golden contract returned no result');
const actual = JSON.parse(match[1]);
const keys = Object.keys(actual);
if (keys.length !== 448) throw new Error(`Expected 448 route snapshots, captured ${keys.length}`);
if (update || !existsSync(goldenPath)) {
  writeFileSync(goldenPath, JSON.stringify(actual, null, 1) + '\n');
  console.log(`Route golden ${update ? 'updated' : 'created'}: ${keys.length} snapshots`);
} else {
  const expected = JSON.parse(readFileSync(goldenPath, 'utf8'));
  // The progress label counts wall-clock seconds, so a loaded machine would drift it; everything else is exact.
  const normalize = (value) => JSON.stringify(value).replace(/\d+s elapsed/g, 'Ns elapsed');
  const drift = keys.filter((key) => normalize(actual[key]) !== normalize(expected[key]));
  const missing = Object.keys(expected).filter((key) => !(key in actual));
  if (drift.length || missing.length) {
    console.error(`Route golden drift in ${drift.length} snapshots: ${drift.join(', ')}${missing.length ? `; missing: ${missing.join(', ')}` : ''}`);
    process.exit(1);
  }
  console.log(`Route golden matches ${keys.length} snapshots`);
}
