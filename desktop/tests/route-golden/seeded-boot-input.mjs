// ./desktop/tests/route-golden/seeded-boot-input.mjs
// Route golden section (T10.3): Boot and input: keyboard shortcuts, the agent-control channel, drag-and-drop and external events.
// Serialized into the browser contract by route-golden-contract.mjs; it must stay self-contained.

export async function bootInput(ctx) {
  const { page, settle, fingerprint, snapshots, theme, width, keys } = ctx;
  // Boot and input: keyboard shortcuts, the native agent-control channel, drag-and-drop and the external-link handler.
  if (theme === 'light') {
    const emit = (event, payload) => page.evaluate(([event, payload]) => window.__goldenEmit(event, payload), [event, payload]);
    const inputGroups = [
      ['input-keys', 'Alt+1', [
        ['f8-help', async () => { await page.keyboard.press('F8'); }],
        ['f8-back', async () => { await page.keyboard.press('F8'); }],
        ['alt-m-metadata', async () => { await page.keyboard.press('Alt+m'); }],
        ['alt-5-reports', async () => { await page.keyboard.press('Alt+5'); }],
        ['alt-2-run', async () => { await page.keyboard.press('Alt+2'); }],
        ['ctrl-enter-start', async () => { await page.keyboard.press('Control+Enter'); await page.waitForTimeout(800); }],
      ]],
      ['input-agent', 'Alt+1', [
        ['agent-press', async () => { await page.evaluate(() => window.__hevcAgentControl({ id: 'a1', action: 'press', key: 'Alt+3' })); await page.waitForTimeout(500); }],
        ['agent-state', async () => { await page.evaluate(() => window.__hevcAgentControl({ id: 'a2', action: 'state' })); await page.waitForTimeout(500); }],
        ['agent-screenshot', async () => { await page.evaluate(() => window.__hevcAgentControl({ id: 'a3', action: 'screenshot' })); await page.waitForTimeout(500); }],
      ]],
      ['boot-drop', 'Alt+1', [
        ['drag-enter', async () => { await emit('tauri://drag-enter', { paths: [], position: { x: 1, y: 1 } }); }],
        ['drag-leave', async () => { await emit('tauri://drag-leave', {}); }],
        ['drag-drop', async () => { await emit('tauri://drag-drop', { paths: ['/fixture/library/Dropped Film (2005).mkv'], position: { x: 1, y: 1 } }); await page.waitForTimeout(1200); }],
      ]],
      ['input-link', 'Alt+4', [
        ['open-external-link', async () => { await page.getByRole('link', { name: /Get a TMDB API token/ }).click(); await page.waitForTimeout(300); }],
      ]],
    ];
    for (const [group, startKey, steps] of inputGroups) {
      await page.reload();
      await settle();
      await page.keyboard.press(startKey);
      await settle();
      await page.evaluate(() => { window.__goldenRepairCalls.splice(0); window.__goldenRunCalls.splice(0); window.__goldenQueueWrites.splice(0); });
      for (const [label, act] of steps) {
        await act();
        const aria = await settle();
        snapshots[`seeded:${group}-${label}|${theme}|${width}`] = { dom: await fingerprint(), aria, calls: await page.evaluate(() => window.__goldenRepairCalls.splice(0)), runCalls: await page.evaluate(() => window.__goldenRunCalls.splice(0)), writes: await page.evaluate(() => window.__goldenQueueWrites.splice(0)) };
      }
    }
  }
  // end-boot-input
}
