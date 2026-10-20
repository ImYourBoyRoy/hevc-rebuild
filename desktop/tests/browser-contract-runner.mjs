// Real-browser acceptance; tool-only Playwright dependency stays outside runtime.
import { spawn, spawnSync } from 'node:child_process';
import { closeSync, mkdtempSync, openSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
export async function runBrowserContract(fixture, contract) {
const desktop = fileURLToPath(new URL('../', import.meta.url));
const evidence = mkdtempSync(join(tmpdir(), 'hevc-modal-acceptance-'));
const port = 5181;
const playwrightCommand = process.platform === 'win32' ? 'pnpm.exe' : 'npx';
const playwrightPrefix = process.platform === 'win32'
  ? ['dlx']
  : ['--yes'];
const server = spawn(process.execPath, ['tests/browser-server.mjs'], {
  cwd: desktop, env: { ...process.env, HEVC_UI_FIXTURE_PORT: String(port), HEVC_UI_FIXTURE_CACHE: join(evidence, 'cache') },
  stdio: ['ignore', 'pipe', 'pipe'],
});
let serverLog = '';
server.stdout.on('data', data => { serverLog += data; });
server.stderr.on('data', data => { serverLog += data; });
let index = 0;
function cli(...args) {
  const evidenceIndex = ++index;
  const commandArgs = [...playwrightPrefix, '--package', '@playwright/cli@0.1.22', 'playwright-cli', '--session', `hevc-modal-${process.pid}`, ...args];
  let result;
  let stdout = '';
  let stderr = '';
  if (process.platform === 'win32') {
    const stdoutPath = join(evidence, `${evidenceIndex}-${args[0]}.stdout.txt`);
    const stderrPath = join(evidence, `${evidenceIndex}-${args[0]}.stderr.txt`);
    const stdoutFd = openSync(stdoutPath, 'w');
    const stderrFd = openSync(stderrPath, 'w');
    try {
      result = spawnSync(playwrightCommand, commandArgs, {
        cwd: evidence, timeout: args[0] === 'run-code' ? 300000 : 60000,
        stdio: ['ignore', stdoutFd, stderrFd],
      });
    } finally {
      closeSync(stdoutFd);
      closeSync(stderrFd);
    }
    stdout = readFileSync(stdoutPath, 'utf8');
    stderr = readFileSync(stderrPath, 'utf8');
  } else {
    result = spawnSync(playwrightCommand, commandArgs, {
      cwd: evidence, encoding: 'utf8', timeout: args[0] === 'run-code' ? 300000 : 60000,
    });
    stdout = result.stdout || '';
    stderr = result.stderr || '';
  }
  writeFileSync(join(evidence, `${evidenceIndex}-${args[0]}.txt`), stdout + stderr);
  if (result.error || result.status !== 0 || stdout.includes('### Error')) throw new Error(result.error?.message || stdout || stderr);
  return stdout;
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
  const contractPath = join(evidence, 'contract.js');
  writeFileSync(contractPath, contract.toString().replaceAll('__EVIDENCE__', evidence));
  console.log(cli('run-code', '--filename', contractPath));
  console.log(`Browser evidence: ${evidence}`);
} finally {
  console.log(`Browser evidence retained: ${evidence}`);
  try { cli('close'); } catch {}
  server.kill('SIGTERM');
  writeFileSync(join(evidence, 'server.txt'), serverLog);
}

}
