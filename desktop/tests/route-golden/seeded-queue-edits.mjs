// ./desktop/tests/route-golden/seeded-queue-edits.mjs
// Route golden section (T10.3): Queue edits: reorder sources, remove a file, remove a failed source, with every queue write recorded; fails on unknown native commands.
// Serialized into the browser contract by route-golden-contract.mjs; it must stay self-contained.

export async function queueEdits(ctx) {
  const { page, settle, fingerprint, snapshots, theme, width } = ctx;
  // Queue edits on a fresh load: reorder sources, remove a file (excluded), remove a failed source.
  // The fake bridge records every queue write so persistence payloads are part of the golden.
  await page.reload();
  await settle();
  await page.keyboard.press('Alt+1');
  await settle();
  for (const [label, name] of [
    ['queue-move-later', 'Move library later'],
    ['queue-remove-file', 'Remove Fixture Film from queue'],
    ['queue-remove-source', 'Remove second from queue'],
    ['queue-refresh-source', 'Refresh'],
  ]) {
    await page.getByRole('button', { name, exact: true }).first().click({ timeout: 5000 });
    const aria = await settle();
    snapshots[`seeded:${label}|${theme}|${width}`] = { dom: await fingerprint(), aria, writes: await page.evaluate(() => (window.__goldenQueueWrites ?? []).splice(0)) };
  }
  const unknown = await page.evaluate(() => window.__goldenUnknownCommands ?? []);
  if (unknown.length) throw new Error(`Fake Tauri bridge does not know: ${[...new Set(unknown)].join(', ')}`);
}
