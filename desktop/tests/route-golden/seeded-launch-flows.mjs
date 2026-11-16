// ./desktop/tests/route-golden/seeded-launch-flows.mjs
// Route golden section (T10.3): Launch flows: output-preview dialogs, per-file overrides sent to the engine, an interrupted full rebuild and its recovery review.
// Serialized into the browser contract by route-golden-contract.mjs; it must stay self-contained.

export async function launchFlows(ctx) {
  const { page, settle, fingerprint, snapshots, theme, width } = ctx;
  // Launch flows on a fresh load: output-preview dialogs, per-file overrides sent to the engine,
  // a full rebuild that is interrupted, and the recovery review that follows it.
  await page.reload();
  await settle();
  await page.keyboard.press('Alt+1');
  await settle();
  await page.getByRole('button', { name: 'Refresh', exact: true }).first().click({ timeout: 5000 });
  await settle();
  await page.getByRole('checkbox', { name: 'Select Fixture Show', exact: true }).check({ timeout: 5000 });
  await settle();
  const names = ['Film (2001).mkv', 'Season 1/S01E01.Pilot.mkv', 'Season 1/S01E02.Second.mkv'];
  const paths = ['/fixture/library/Movies/Film (2001).mkv', '/fixture/library/Show Name/Season 1/S01E01.Pilot.mkv', '/fixture/library/Show Name/Season 1/S01E02.Second.mkv'];
  void names;
  const finish = (runId, extra, rows, csv) => page.evaluate(([runId, extra, rows, csv]) => {
    window.__goldenReports = rows.length ? [{ csv_path: csv, log_path: null, created: 1700000000, counts: {}, rows }] : [];
    window.__goldenEmit('run-finished', { runId, exitCode: 0, stopRequested: false, logPath: null, reportPath: rows.length ? csv : null, error: null, ...extra });
  }, [runId, extra, rows, csv]);
  const launchSteps = [
    ['launch-metadata-preview', async () => { await page.getByRole('button', { name: 'Metadata-only MKV', exact: true }).first().click({ timeout: 5000 }); }],
    ['launch-metadata-started', async () => { await page.getByRole('button', { name: 'Confirm destinations and start', exact: true }).first().click({ timeout: 5000 }); }],
    ['launch-metadata-finished', async () => { await finish('run-1', {}, paths.map((path) => ({ input: path, status: 'METADATA_OK', message: 'ok', output: path.replace(/\.mkv$/, '.metadata.mkv') })), '/fixture/reports/meta.csv'); }],
    ['launch-full-preview', async () => { await page.keyboard.press('Alt+1'); await settle(); await page.getByRole('button', { name: 'Rebuild video', exact: true }).first().click({ timeout: 5000 }); }],
    ['launch-full-started', async () => { await page.getByRole('checkbox', { name: /I reviewed the destinations/ }).check({ timeout: 5000 }); await page.getByRole('button', { name: 'Start full rebuild', exact: true }).first().click({ timeout: 5000 }); }],
    ['launch-stop-now-dialog', async () => { await page.getByRole('button', { name: 'Stop now', exact: true }).first().click({ timeout: 5000 }); }],
    ['launch-stop-now-requested', async () => { await page.getByRole('dialog', { name: 'Stop this encode immediately?' }).getByRole('button', { name: 'Stop now', exact: true }).click({ timeout: 5000 }); }],
    ['launch-full-interrupted', async () => { await finish('run-2', { exitCode: 1, error: 'Fixture engine stopped' }, [], ''); }],
    ['launch-recovery-open', async () => { await page.keyboard.press('Alt+1'); await settle(); await page.getByRole('button', { name: 'Rebuild video', exact: true }).first().click({ timeout: 5000 }); }],
  ];
  const reviewFile = async (advance) => {
    await page.getByRole('button', { name: 'Inspect recorded target', exact: true }).first().click({ timeout: 5000 });
    await settle();
    await page.getByRole('checkbox', { name: /I reviewed the report and file check/ }).check({ timeout: 5000 });
    await settle();
    await page.getByRole('button', { name: advance, exact: true }).first().click({ timeout: 5000 });
  };
  launchSteps.push(
    ['launch-recovery-probed', async () => { await page.getByRole('button', { name: 'Inspect recorded target', exact: true }).first().click({ timeout: 5000 }); }],
    ['launch-recovery-acknowledged', async () => { await page.getByRole('checkbox', { name: /I reviewed the report and file check/ }).check({ timeout: 5000 }); }],
    ['launch-recovery-next', async () => { await page.getByRole('button', { name: 'Review next file', exact: true }).first().click({ timeout: 5000 }); }],
    ['launch-recovery-third', async () => { await reviewFile('Review next file'); }],
    ['launch-recovery-finished', async () => { await reviewFile('Continue to output review'); }],
    ['launch-recovery-preview-back', async () => { await page.getByRole('button', { name: 'Back', exact: true }).first().click({ timeout: 5000 }); }],
    // A blocked destination: the preview lists the collision and offers to adjust output folders.
    ['launch-collision', async () => {
      await page.reload(); await settle(); await page.keyboard.press('Alt+1'); await settle();
      await page.evaluate(() => { window.__goldenCollisions = [{ kind: 'existing_output', source: '/fixture/library/Movies/Film (2001).mkv', target: '/fixture/library/Movies/Film (2001).metadata.mkv', conflictsWith: null }]; });
      await page.getByRole('button', { name: 'Metadata-only MKV', exact: true }).first().click({ timeout: 5000 });
    }],
    ['launch-collision-adjust', async () => { await page.getByRole('button', { name: 'Adjust output folders', exact: true }).first().click({ timeout: 5000 }); }],
  );
  for (const [label, act] of launchSteps) {
    await act();
    const aria = await settle();
    snapshots[`seeded:${label}|${theme}|${width}`] = { dom: await fingerprint(), aria, writes: await page.evaluate(() => (window.__goldenQueueWrites ?? []).splice(0)), runCalls: await page.evaluate(() => window.__goldenRunCalls.splice(0)) };
  }
}
