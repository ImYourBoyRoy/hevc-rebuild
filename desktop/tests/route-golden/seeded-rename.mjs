// ./desktop/tests/route-golden/seeded-rename.mjs
// Route golden section (T10.3): Library rename dialog: open, token insert, reset, conflict, apply.
// Serialized into the browser contract by route-golden-contract.mjs; it must stay self-contained.

export async function renameDialog(ctx) {
  const { page, settle, fingerprint, snapshots, theme, width } = ctx;
  // Library rename dialog: open, token insert, reset, conflict, apply (queue state is remapped to the new names).
  if (theme === 'light') {
    const renameSteps = [
      ['open-dialog', async () => { await page.getByRole('button', { name: 'Rename files', exact: true }).click(); }],
      ['insert-token', async () => { await page.getByRole('button', { name: 'Year', exact: true }).click(); }],
      ['reset-pattern', async () => { await page.getByRole('button', { name: 'Standard film name', exact: true }).click(); }],
      ['apply-conflict', async () => { await page.evaluate(() => { window.__goldenRenameConflict = true; }); await page.getByRole('button', { name: /^Rename (this file|\d+ files)$/ }).click(); await page.waitForTimeout(500); }],
      ['apply', async () => { await page.evaluate(() => { window.__goldenRenameConflict = false; }); await page.getByRole('button', { name: /^Rename (this file|\d+ files)$/ }).click(); await page.waitForTimeout(800); }],
    ];
    await page.reload();
    await settle();
    await page.keyboard.press('Alt+1');
    await settle();
    await page.evaluate(() => { window.__goldenRepairCalls.splice(0); window.__goldenQueueWrites.splice(0); window.__goldenPrefWrites.splice(0); });
    for (const [label, act] of renameSteps) {
      await act();
      const aria = await settle();
      snapshots[`seeded:rename-${label}|${theme}|${width}`] = { dom: await fingerprint(), aria, calls: await page.evaluate(() => window.__goldenRepairCalls.splice(0)), writes: await page.evaluate(() => window.__goldenQueueWrites.splice(0)), prefWrites: await page.evaluate(() => window.__goldenPrefWrites.splice(0)) };
    }
  }
  // end-rename
}
