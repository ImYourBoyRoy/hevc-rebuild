// Serves the REAL SvelteKit route (src/routes/+page.svelte) in browser-preview mode.
// No Tauri bridge exists here, so the route takes its documented non-desktop path:
// preferences live in localStorage only and no native command is invoked.
import { createServer } from 'vite';
import { fileURLToPath } from 'node:url';
import { join } from 'node:path';
const desktop = fileURLToPath(new URL('../', import.meta.url));
const server = await createServer({
  root: desktop,
  configFile: join(desktop, 'vite.config.js'),
  cacheDir: process.env.HEVC_UI_FIXTURE_CACHE || join(desktop, '.native-smoke', 'route-cache'),
  clearScreen: false,
  server: { host: '127.0.0.1', port: Number(process.env.HEVC_UI_FIXTURE_PORT || 5179), strictPort: true, hmr: false, watch: null },
});
await server.listen();
server.printUrls();
const close = async () => { await server.close(); process.exit(0); };
process.on('SIGTERM', close); process.on('SIGINT', close);
