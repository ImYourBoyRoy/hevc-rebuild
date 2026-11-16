// ./desktop/tests/route-golden/seeded-run-lifecycle.mjs
// Route golden section (T10.3): Run lifecycle: start an integrity check, feed engine events, request a safe stop, finish, then a second run that fails.
// Serialized into the browser contract by route-golden-contract.mjs; it must stay self-contained.

export async function runLifecycle(ctx) {
  const { page, settle, fingerprint, snapshots, theme, width } = ctx;
  // Run lifecycle on a fresh load: start an integrity check, feed engine events, request a safe stop,
  // finish; then a second run that fails. The contract plays the native engine through the fake bridge.
  await page.reload();
  await settle();
  await page.keyboard.press('Alt+1');
  await settle();
  const film = '/fixture/library/Movies/Film (2001).mkv';
  const progress = (runId) => ({ runId, fileName: 'Film (2001).mkv', filePath: film, phase: 'Checking', batchNumber: 1, batchTotal: 1, filePercent: 40, queuePercent: 40, speed: 1.5, etaSeconds: 30, elapsedSeconds: 20, inputBytes: 1500000000, outputBytes: null, estimatedOutputBytes: null });
  const report = (status, message) => ({ csv_path: '/fixture/reports/run.csv', log_path: null, created: 1700000000, counts: { [status]: 1 }, rows: [{ input: film, status, message, output: '' }] });
  const runSteps = [
    ['run-started', async () => { await page.getByRole('button', { name: 'Check integrity', exact: true }).first().click({ timeout: 5000 }); }],
    ['run-progress', async () => { await page.evaluate(([event, progress]) => { window.__goldenEmit('run-line', { runId: 'run-1', line: 'fixture engine line' }); window.__goldenEmit(event, progress); }, ['run-progress', progress('run-1')]); }],
    ['run-validating', async () => { await page.evaluate(([event, progress]) => window.__goldenEmit(event, progress), ['run-progress', { ...progress('run-1'), phase: 'Validating output', filePercent: 90, queuePercent: 90 }]); }],
    ['run-stop-requested', async () => { await page.getByRole('button', { name: 'Stop safely', exact: true }).first().click({ timeout: 5000 }); }],
    ['run-finished-stopped', async () => { await page.evaluate(([rep]) => { window.__goldenReports = [rep]; window.__goldenEmit('run-finished', { runId: 'run-1', exitCode: 0, stopRequested: true, logPath: null, reportPath: '/fixture/reports/run.csv', error: null }); }, [report('INTEGRITY_OK', 'Fixture ok')]); }],
    ['run2-started', async () => { await page.getByRole('button', { name: 'Start quick integrity check', exact: true }).first().click({ timeout: 5000 }); }],
    ['run2-failed', async () => { await page.evaluate(([rep]) => { window.__goldenReports = [rep]; window.__goldenEmit('run-finished', { runId: 'run-2', exitCode: 1, stopRequested: false, logPath: '/fixture/reports/run.log', reportPath: '/fixture/reports/run.csv', error: null }); }, [report('CORRUPT', 'Fixture corrupt stream')]); }],
  ];
  for (const [label, act] of runSteps) {
    await act();
    const aria = await settle();
    snapshots[`seeded:${label}|${theme}|${width}`] = { dom: await fingerprint(), aria, writes: await page.evaluate(() => (window.__goldenQueueWrites ?? []).splice(0)), runCalls: await page.evaluate(() => window.__goldenRunCalls.splice(0)) };
  }
}
