// ./desktop/tests/route-golden/seeded-settings.mjs
// Route golden section (T10.3): Credentials, artwork cache and settings: TMDB/TVDB, cache purge, language, filename filters, work root, Run-page settings.
// Serialized into the browser contract by route-golden-contract.mjs; it must stay self-contained.

export async function settingsFlows(ctx) {
  const { page, settle, fingerprint, snapshots, theme, width, mode } = ctx;
  // Credentials, artwork cache and settings on fresh loads: TMDB/TVDB save, show, hide, remove; saved-poster cache
  // review and purge; language, filename filters, title cleanup, work root; Run-page settings edit/save/reset.
  if (theme === 'light') {
    const fillBlur = async (name, value, role = 'textbox') => { const input = page.getByRole(role, { name, exact: true }); await input.fill(value); await input.blur(); };
    const settingsGroups = [
      ['cred-tmdb', {}, 'Alt+4', [
        ['fill-token', async () => { await page.getByRole('textbox', { name: 'TMDB API Read Access Token', exact: true }).fill('fixture-tmdb-token'); }],
        ['save-token', async () => { await page.getByRole('button', { name: 'Save token', exact: true }).click(); await page.waitForTimeout(500); }],
        ['hide-token', async () => { await page.getByRole('button', { name: 'Hide token', exact: true }).click(); }],
        ['remove-token', async () => { await page.locator('#tmdb-credential-settings').getByRole('button', { name: /^Remove/ }).click(); await page.waitForTimeout(500); }],
      ]],
      ['cred-tmdb-saved', { tmdb: true }, 'Alt+4', [
        ['show-saved-token', async () => { await page.getByRole('button', { name: 'Show saved token', exact: true }).click(); await page.waitForTimeout(500); }],
      ]],
      ['cred-tvdb', {}, 'Alt+4', [
        ['fill-credentials', async () => { await page.getByRole('textbox', { name: 'TheTVDB project API key', exact: true }).fill('fixture-tvdb-key'); await page.getByRole('textbox', { name: 'TheTVDB subscriber PIN', exact: true }).fill('4321'); }],
        ['confirm-access', async () => { await page.locator('#tvdb-credential-settings').getByRole('checkbox').check(); }],
        ['save-credentials', async () => { await page.getByRole('button', { name: 'Save credentials', exact: true }).click(); await page.waitForTimeout(500); }],
        ['hide-credentials', async () => { await page.getByRole('button', { name: 'Hide credentials', exact: true }).click(); }],
        ['remove-credentials', async () => { await page.locator('#tvdb-credential-settings').getByRole('button', { name: /^Remove/ }).click(); await page.waitForTimeout(500); }],
      ]],
      ['cred-tvdb-saved', { tvdb: true }, 'Alt+4', [
        ['show-saved-credentials', async () => { await page.getByRole('button', { name: 'Show saved credentials', exact: true }).click(); await page.waitForTimeout(500); }],
      ]],
      ['artwork-cache', { cachePurgeable: 2 }, 'Alt+4', [
        ['review-cleanup', async () => { await page.locator('button.artwork-cache-review').click(); }],
        ['cancel-cleanup', async () => { await page.locator('section[aria-labelledby="artwork-cache-title"]').getByRole('button', { name: 'Cancel', exact: true }).click(); }],
        ['purge', async () => { await page.locator('button.artwork-cache-review').click(); await page.locator('section[aria-labelledby="artwork-cache-title"] button.danger-outline').click(); await page.waitForTimeout(500); }],
        ['refresh-cache', async () => { await page.locator('section[aria-labelledby="artwork-cache-title"] button.compact').first().click(); await page.waitForTimeout(500); }],
      ]],
      ['settings-prefs', {}, 'Alt+4', [
        ['language-invalid', async () => { await fillBlur('Metadata language and region', 'x', 'combobox'); }],
        ['language-valid', async () => { await fillBlur('Metadata language and region', 'fr-fr', 'combobox'); await page.waitForTimeout(500); }],
        ['include-add', async () => { await page.getByRole('textbox', { name: 'New include names pattern', exact: true }).fill('*.mkv'); await page.locator('.filename-filter-list').nth(0).getByRole('button', { name: 'Add', exact: true }).click(); await page.waitForTimeout(500); }],
        ['include-duplicate', async () => { await page.getByRole('textbox', { name: 'New include names pattern', exact: true }).fill('*.MKV'); await page.locator('.filename-filter-list').nth(0).getByRole('button', { name: 'Add', exact: true }).click(); }],
        ['exclude-invalid', async () => { await page.getByRole('textbox', { name: 'New exclude names pattern', exact: true }).fill('***'); await page.locator('.filename-filter-list').nth(1).getByRole('button', { name: 'Add', exact: true }).click(); }],
        ['include-remove', async () => { await page.getByRole('button', { name: 'Remove include pattern *.mkv', exact: true }).click(); await page.waitForTimeout(500); }],
        ['exclude-clear', async () => { await page.locator('.filename-filter-list').nth(1).getByRole('button', { name: 'Clear all', exact: true }).click(); await page.waitForTimeout(500); }],
        ['blacklist-add', async () => { await page.getByRole('textbox', { name: 'New blocked text pattern', exact: true }).fill('Rip*Group'); await page.getByRole('button', { name: 'Add pattern', exact: true }).click(); await page.waitForTimeout(500); }],
        ['blacklist-wildcard', async () => { await page.getByRole('textbox', { name: 'New blocked text pattern', exact: true }).fill('***'); await page.getByRole('button', { name: 'Add pattern', exact: true }).click(); }],
        ['blacklist-remove', async () => { await page.locator('.metadata-blacklist .blacklist-remove').first().click(); await page.waitForTimeout(500); }],
        ['work-root-change', async () => { await fillBlur('Temporary work root', '/work/root'); await page.waitForTimeout(500); }],
      ]],
      ['settings-run', {}, 'Alt+2', [
        ['rebuild-mode', async () => { await page.getByRole('radio', { name: /Rebuild/ }).click(); }],
        ['crf-edit', async () => { await page.getByRole('slider', { name: 'CRF quality', exact: true }).fill('18'); }],
        ['save-defaults', async () => { await page.getByRole('button', { name: 'Save as app defaults', exact: true }).click(); await page.waitForTimeout(500); }],
        ['reset-saved', async () => { await page.getByRole('button', { name: 'Reset to library values', exact: true }).click(); await page.waitForTimeout(500); }],
      ]],
    ];
    for (const [group, setup, startKey, steps] of settingsGroups) {
      await page.evaluate((setup) => sessionStorage.setItem('goldenSettingsSetup', JSON.stringify(setup)), setup);
      await page.reload();
      await settle();
      await page.keyboard.press(startKey);
      await settle();
      await page.evaluate(() => { window.__goldenRepairCalls.splice(0); window.__goldenPrefWrites.splice(0); });
      for (const [label, act] of steps) {
        await act();
        const aria = await settle();
        snapshots[`seeded:settings-${group}-${label}|${theme}|${width}`] = { dom: await fingerprint(), aria, calls: await page.evaluate(() => window.__goldenRepairCalls.splice(0)), prefWrites: await page.evaluate(() => window.__goldenPrefWrites.splice(0)) };
      }
    }
    await page.evaluate(() => sessionStorage.removeItem('goldenSettingsSetup'));
  }
}
