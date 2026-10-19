// Real-browser acceptance; tool-only Playwright dependency stays outside runtime.
import { spawn, spawnSync } from 'node:child_process';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
export async function runBrowserContract(fixture, contract) {
const desktop = fileURLToPath(new URL('../', import.meta.url));
const evidence = mkdtempSync(join(tmpdir(), 'hevc-modal-acceptance-'));
const port = 5181;
const server = spawn(process.execPath, ['tests/browser-server.mjs'], {
  cwd: desktop, env: { ...process.env, HEVC_UI_FIXTURE_PORT: String(port), HEVC_UI_FIXTURE_CACHE: join(evidence, 'cache') },
  stdio: ['ignore', 'pipe', 'pipe'],
});
let serverLog = '';
server.stdout.on('data', data => { serverLog += data; });
server.stderr.on('data', data => { serverLog += data; });
let index = 0;
function cli(...args) {
  const result = spawnSync('npx', ['--yes', '--package', '@playwright/cli@0.1.22', 'playwright-cli', '--session', `hevc-modal-${process.pid}`, ...args], {
    cwd: evidence, encoding: 'utf8', timeout: 60000,
  });
  writeFileSync(join(evidence, `${++index}-${args[0]}.txt`), (result.stdout || '') + (result.stderr || ''));
  if (result.error || result.status !== 0 || result.stdout?.includes('### Error')) throw new Error(result.error?.message || result.stdout || result.stderr);
  return result.stdout;
}

try {
  let ready = false;
  for (let attempt = 0; attempt < 100; attempt++) {
    if (server.exitCode !== null) throw new Error(serverLog);
    try { ready = (await fetch(`http://127.0.0.1:${port}`)).ok; } catch {}
    if (ready) break;
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  if (!ready) throw new Error('Isolated browser fixture server did not become ready');
  cli('open', `http://127.0.0.1:${port}?fixture=${fixture}`);
  cli('snapshot');
  console.log(cli('run-code', contract.toString().replaceAll('__EVIDENCE__', evidence)));
  console.log(`Browser evidence: ${evidence}`);
} finally {
  console.log(`Browser evidence retained: ${evidence}`);
  try { cli('close'); } catch {}
  server.kill('SIGTERM');
  writeFileSync(join(evidence, 'server.txt'), serverLog);
}

}
