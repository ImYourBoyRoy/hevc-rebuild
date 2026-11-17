// ./desktop/tests/route-golden/seeded-tool-setup.mjs
// Route golden section (T10.3): Tool setup (package repair) on fresh loads of the Settings page: update check, reviewed toolchain, live output, outcomes.
// Serialized into the browser contract by route-golden-contract.mjs; it must stay self-contained.

export async function toolSetup(ctx) {
  const { page, settle, fingerprint, snapshots, theme, width } = ctx;
  // Tool setup (package repair) on fresh loads of the Settings page: update check, reviewed toolchain job,
  // live output, finish with tool-by-tool outcomes, stale-catalog refresh, failure, HDR10+ helper, saved logs.
  if (theme === 'light') {
    const aptStatus = (over = {}) => ({ packages: [{ name: 'ffmpeg', installed_version: '7.0-1', candidate_version: '7.1-1' }], catalog_updated_at: 1700000000, catalog_stale: false, source: 'fixture-apt', checked_at: 1700000000, ...over });
    const repairGroups = [
      ['repair-updates', { apt: aptStatus() }, [
        ['check-versions', async () => { await page.getByRole('button', { name: 'Check versions', exact: true }).click(); }],
        ['review-setup', async () => { await page.getByRole('button', { name: 'Install & update everything', exact: true }).click(); }],
        ['confirm-toolchain', async () => { await page.getByRole('button', { name: 'Authenticate once & install', exact: true }).click(); }],
        ['live-output', async () => { await page.evaluate(() => { window.__goldenEmit('repair-line', { runId: 'job-toolchain', line: 'Reading package lists…' }); window.__goldenEmit('repair-line', { runId: 'other-job', line: 'ignored line' }); window.__goldenEmit('repair-line', { runId: 'job-toolchain', line: 'Setting up ffmpeg (7.1-1)…' }); }); }],
        ['finished-ok', async () => { await page.evaluate(() => { const environment = window.__goldenBaseEnvironment(); environment.checks.find((check) => check.key === 'ffmpeg').version = '7.1-1'; window.__goldenEnvironment = environment; window.__goldenAptStatus = { packages: [], catalog_updated_at: 1700000000, catalog_stale: false, source: 'fixture-apt', checked_at: 1700000001 }; window.__goldenEmit('repair-finished', { jobId: 'job-toolchain', success: true, error: null, logWarning: null }); }); await page.waitForTimeout(800); }],
      ]],
      ['repair-stale', { apt: aptStatus({ catalog_stale: true }) }, [
        ['review-includes-catalog-refresh', async () => { await page.getByRole('button', { name: 'Install & update everything', exact: true }).click(); }],
        ['cancel-review', async () => { await page.getByRole('dialog').getByRole('button', { name: 'Cancel', exact: true }).click(); }],
      ]],
      ['repair-failure', { apt: aptStatus() }, [
        ['review-setup', async () => { await page.getByRole('button', { name: 'Install & update everything', exact: true }).click(); }],
        ['confirm-toolchain', async () => { await page.getByRole('button', { name: 'Authenticate once & install', exact: true }).click(); }],
        ['finished-failed', async () => { await page.evaluate(() => { window.__goldenEmit('repair-finished', { jobId: 'job-toolchain', success: false, error: 'apt exited with status 100', logWarning: 'Log was truncated' }); }); await page.waitForTimeout(800); }],
      ]],
      ['repair-helper', { apt: aptStatus({ packages: [] }), helperMissing: true }, [
        ['review-helper-only', async () => { await page.getByRole('button', { name: 'Install & update everything', exact: true }).click(); }],
        ['install-helper', async () => { await page.getByRole('dialog').getByRole('button', { name: 'Install & update', exact: true }).click(); await page.waitForTimeout(800); }],
      ]],
      ['repair-logs', { apt: aptStatus({ packages: [] }), logs: true }, [
        ['open-history', async () => { await page.locator('summary', { hasText: 'Saved setup logs' }).click(); }],
        ['view-log', async () => { await page.getByRole('button', { name: 'View complete log', exact: true }).click(); }],
        ['export-log', async () => { await page.getByRole('button', { name: 'Export log…', exact: true }).click(); await page.waitForTimeout(500); }],
      ]],
    ];
    for (const [group, setup, steps] of repairGroups) {
      await page.evaluate((setup) => sessionStorage.setItem('goldenRepairSetup', JSON.stringify(setup)), setup);
      await page.reload();
      await settle();
      await page.keyboard.press('Alt+4');
      await settle();
      await page.evaluate(() => window.__goldenRepairCalls.splice(0));
      for (const [label, act] of steps) {
        await act();
        const aria = await settle();
        snapshots[`seeded:${group}-${label}|${theme}|${width}`] = { dom: await fingerprint(), aria, repairCalls: await page.evaluate(() => window.__goldenRepairCalls.splice(0)) };
      }
    }
    await page.evaluate(() => sessionStorage.removeItem('goldenRepairSetup'));
  }
}
