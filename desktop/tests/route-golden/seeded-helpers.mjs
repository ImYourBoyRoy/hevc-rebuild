// ./desktop/tests/route-golden/seeded-helpers.mjs
// Route golden section (T10.3): Helpers: native pickers, output folders, tool paths, reports and logs, navigation.
// Serialized into the browser contract by route-golden-contract.mjs; it must stay self-contained.

export async function helperFlows(ctx) {
  const { page, settle, fingerprint, snapshots, theme, width } = ctx;
  // Helpers (helper slice): native pickers, output folders, tool paths, reports and logs, navigation shortcuts.
  if (theme === 'light') {
    const picks = (...values) => page.evaluate((values) => { window.__goldenDialogPicks = values; }, values);
    const rep = (name) => ({ csv_path: '/fixture/reports/' + name + '.csv', log_path: name === 'second' ? null : '/fixture/reports/' + name + '.log', created: 1700000000, counts: { INTEGRITY_OK: 1 }, rows: [{ input: '/fixture/library/Movies/Film (2001).mkv', status: 'INTEGRITY_OK', message: 'Fixture ' + name, output: '' }] });
    const helperGroups = [
      ['pickers', 'Alt+1', null, [
        ['add-folder', async () => { await picks(['/fixture/library/Picked Folder']); await page.getByRole('button', { name: 'Add folder', exact: true }).first().click(); await page.waitForTimeout(800); }],
        ['add-folder-cancelled', async () => { await picks(null); await page.getByRole('button', { name: 'Add folder', exact: true }).first().click(); await page.waitForTimeout(300); }],
        ['add-files', async () => { await picks(['/fixture/library/Movies/Extra Film (2010).mkv']); await page.getByRole('button', { name: 'Add files', exact: true }).first().click(); await page.waitForTimeout(800); }],
      ]],
      ['rebuild-output', 'Alt+2', null, [
        ['choose-folder', async () => { await page.getByRole('radio', { name: /Rebuild/ }).click(); await picks('/fixture/out/rebuilds'); await page.getByRole('button', { name: 'Choose folder', exact: true }).first().click(); await page.waitForTimeout(500); }],
        ['choose-cancelled', async () => { await picks(null); await page.getByRole('button', { name: 'Choose folder', exact: true }).first().click(); await page.waitForTimeout(300); }],
      ]],
      ['youtube-output', 'Alt+3', null, [
        ['choose-folder', async () => { await picks('/fixture/out/youtube'); await page.getByRole('button', { name: 'Choose folder', exact: true }).first().click(); await page.waitForTimeout(500); }],
      ]],
      ['reports', 'Alt+5', async () => { await page.evaluate((rows) => { window.__goldenReports = rows; }, [rep('first'), rep('second')]); }, [
        ['refresh', async () => { await page.getByRole('button', { name: 'Refresh history', exact: true }).click(); await page.waitForTimeout(500); }],
        ['choose-second', async () => { await page.locator('button.report-item').nth(1).click(); await page.waitForTimeout(300); }],
        ['choose-first', async () => { await page.locator('button.report-item').nth(0).click(); await page.waitForTimeout(300); }],
        ['copy-excerpt', async () => { await page.getByRole('button', { name: 'Copy excerpt', exact: true }).click(); await page.waitForTimeout(300); }],
        ['delete-arm', async () => { await page.getByRole('button', { name: 'Delete history', exact: true }).click(); await page.waitForTimeout(300); }],
        ['delete-cancel', async () => { await page.getByRole('button', { name: 'Cancel', exact: true }).click(); await page.waitForTimeout(300); }],
        ['delete-rearm', async () => { await page.getByRole('button', { name: 'Delete history', exact: true }).click(); await page.waitForTimeout(300); }],
        ['delete-confirm', async () => { await page.getByRole('button', { name: 'Delete permanently', exact: true }).click(); await page.waitForTimeout(500); }],
      ]],
      ['tool-paths', 'Alt+4', null, [
        ['browse-first', async () => { await picks('/opt/fixture/bin/ffmpeg'); await page.getByRole('button', { name: 'Browse', exact: true }).first().click(); await page.waitForTimeout(800); }],
        ['browse-cancelled', async () => { await picks(null); await page.getByRole('button', { name: 'Browse', exact: true }).first().click(); await page.waitForTimeout(300); }],
        ['use-path', async () => { await page.getByRole('button', { name: 'PATH', exact: true }).first().click(); await page.waitForTimeout(800); }],
      ]],
      ['provider-links', 'Alt+4', null, [
        ['tmdb-setup', async () => { await page.getByRole('button', { name: /^Add TMDB token/ }).first().click(); await page.waitForTimeout(500); }],
        ['tvdb-setup', async () => { await page.getByRole('button', { name: /^Set up TheTVDB access/ }).first().click(); await page.waitForTimeout(500); }],
      ]],
      ['run-details', 'Alt+1', null, [
        ['start', async () => { await page.getByRole('button', { name: 'Check integrity', exact: true }).first().click({ timeout: 5000 }); await page.waitForTimeout(500); }],
        ['open-details', async () => { await page.getByRole('button', { name: 'Open progress details', exact: true }).first().click({ timeout: 5000 }); await page.waitForTimeout(800); }],
      ]],
    ];
    for (const [group, startKey, prepare, steps] of helperGroups) {
      await page.reload();
      await settle();
      await page.keyboard.press(startKey);
      await settle();
      if (prepare) await prepare();
      await page.evaluate(() => { window.__goldenRepairCalls.splice(0); window.__goldenPrefWrites.splice(0); window.__goldenPrefExtra.splice(0); window.__goldenQueueWrites.splice(0); window.__goldenRunCalls.splice(0); });
      for (const [label, act] of steps) {
        await act();
        const aria = await settle();
        snapshots[`seeded:helpers-${group}-${label}|${theme}|${width}`] = {
          dom: await fingerprint(), aria, focus: await page.evaluate(() => document.activeElement?.id ?? null),
          calls: await page.evaluate(() => window.__goldenRepairCalls.splice(0)), writes: await page.evaluate(() => window.__goldenQueueWrites.splice(0)),
          prefWrites: await page.evaluate(() => window.__goldenPrefWrites.splice(0)), prefExtra: await page.evaluate(() => window.__goldenPrefExtra.splice(0)),
          runCalls: await page.evaluate(() => window.__goldenRunCalls.splice(0)),
        };
      }
    }
  }
  // end-helpers
}
