// Isolated browser fixtures; native IPC, user profiles and media are never mounted.
import { createServer } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { fileURLToPath } from 'node:url';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
const desktop = fileURLToPath(new URL('../', import.meta.url));
const server = await createServer({
  configFile: false,
  root: join(desktop, 'tests/browser'),
  cacheDir: process.env.HEVC_UI_FIXTURE_CACHE || join(tmpdir(), 'hevc-ui-fixture-cache'),
  plugins: [svelte()],
  resolve: { alias: { '#lib': join(desktop, 'src/lib') } },
  server: { host: '127.0.0.1', port: Number(process.env.HEVC_UI_FIXTURE_PORT || 5178), strictPort: true, fs: { allow: [desktop] } },
});
await server.listen();
server.printUrls();
const close = async () => { await server.close(); process.exit(0); };
process.on('SIGTERM', close); process.on('SIGINT', close);
