// ./desktop/tests/route-golden/seeded-review-flows.mjs
// Route golden section (T10.3): Review dialogs (film, series, season) opened and closed on the seeded library.
// Serialized into the browser contract by route-golden-contract.mjs; it must stay self-contained.

export async function reviewFlows(ctx) {
  const { page, settle, fingerprint, snapshots, theme, width } = ctx;
  // Review flows exercise poster resolvers, review factories and filename templating with real data.
  await page.keyboard.press('Alt+1');
  await settle();
  const flow = [
    ['film-review', 'Review title and poster for Film (2001).mkv', 'Close media file review'],
    ['series-review', 'Review series', 'Done'],
    ['series-open', 'Open Fixture Show', null],
    ['season-review', 'Review season', 'Done'],
    ['season-open', 'Open Season 01', null],
  ];
  for (const [label, open, close] of flow) {
    await page.getByRole('button', { name: open, exact: true }).first().click({ timeout: 5000 });
    const aria = await settle();
    snapshots[`seeded:${label}|${theme}|${width}`] = { dom: await fingerprint(), aria };
    if (close) {
      await page.getByRole('button', { name: close, exact: true }).first().click({ timeout: 5000 });
      await settle();
    }
  }
}
