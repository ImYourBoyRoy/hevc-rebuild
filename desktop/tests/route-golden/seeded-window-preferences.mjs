// ./desktop/tests/route-golden/seeded-window-preferences.mjs
// Route golden section (T10.3): Window and preferences: restore clamp, resize/maximize saving, no-op saves, themes, YouTube rows.
// Serialized into the browser contract by route-golden-contract.mjs; it must stay self-contained.

export async function windowPreferences(ctx) {
  const { page, settle, fingerprint, snapshots, theme, width } = ctx;
  // Window and preferences on a fresh load: the restore clamp, resize/maximize saving, no-op saves, themes.
  await page.reload();
  await settle();
  const drainWindow = async () => ({
    windowCalls: await page.evaluate(() => window.__goldenWindowCalls.splice(0)),
    prefWrites: await page.evaluate(() => window.__goldenPrefWrites.splice(0)),
  });
  const resizeTo = async (width, height, maximized) => {
    await page.evaluate(([width, height, maximized]) => {
      window.__goldenWindow.inner = { width, height };
      window.__goldenWindow.maximized = maximized;
      window.__goldenEmit('tauri://resize', { width, height });
    }, [width, height, maximized]);
    await page.waitForTimeout(1200);
  };
  const windowSteps = [
    ['window-restored', async () => { await page.waitForTimeout(1000); }],
    ['window-resized', async () => { await resizeTo(2400, 1500, false); }],
    ['window-resized-again-unchanged', async () => { await resizeTo(2400, 1500, false); }],
    ['window-maximized', async () => { await resizeTo(3000, 1800, true); }],
    ['window-theme-dark', async () => { await page.getByRole('combobox', { name: 'Appearance', exact: true }).selectOption('dark'); }],
    ['window-theme-phosphor', async () => { await page.getByRole('combobox', { name: 'Appearance', exact: true }).selectOption('phosphor'); }],
    ['window-theme-light', async () => { await page.getByRole('combobox', { name: 'Appearance', exact: true }).selectOption('light'); }],
    ['window-page-run', async () => { await page.keyboard.press('Alt+2'); }],
    ['shell-help-button', async () => { await page.getByRole('button', { name: 'Help and commands', exact: true }).click(); }],
    ['shell-refresh-environment', async () => { await page.getByRole('button', { name: 'Refresh environment', exact: true }).click(); await page.waitForTimeout(800); }],
    ['youtube-page', async () => { await page.keyboard.press('Alt+3'); }],
    ['youtube-row-unselect', async () => { await page.getByRole('checkbox', { name: 'Select Film (2001).mkv', exact: true }).uncheck(); }],
    ['youtube-row-reselect', async () => { await page.getByRole('checkbox', { name: 'Select Film (2001).mkv', exact: true }).check(); }],
  ];
  for (const [label, act] of windowSteps) {
    await act();
    const aria = await settle();
    snapshots[`seeded:${label}|${theme}|${width}`] = { dom: await fingerprint(), aria, ...(await drainWindow()) };
  }
}
