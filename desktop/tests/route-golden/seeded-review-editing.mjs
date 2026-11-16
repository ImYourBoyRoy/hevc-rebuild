// ./desktop/tests/route-golden/seeded-review-editing.mjs
// Route golden section (T10.3): Review editing: series, season, episode and film dialogs, with the recorded save_library_queue_* payloads.
// Serialized into the browser contract by route-golden-contract.mjs; it must stay self-contained.

export async function reviewEditing(ctx) {
  const { page, settle, fingerprint, snapshots, theme, width } = ctx;
  // Review editing on a fresh load: series, season, episode and film dialogs. Edits are saved as the
  // user types, so the recorded save_library_queue_* payloads show exactly what each edit persists.
  await page.reload();
  await settle();
  await page.keyboard.press('Alt+1');
  await settle();
  const fill = async (label, value) => { await page.getByRole('textbox', { name: label, exact: true }).first().fill(value, { timeout: 5000 }); };
  const button = (name) => page.getByRole('button', { name, exact: true }).first();
  const reviewSteps = [
    ['review-series-edit', async () => {
      await button('Review series').click({ timeout: 5000 });
      await fill('Title', 'Edited Show'); await fill('Year', '2021'); await fill('Certification', 'TV-MA'); await fill('Plot', 'Edited plot');
      await fill('Genres', 'Drama, Comedy'); await fill('Audience rating', '9.1'); await fill('Comment', 'Series comment');
      await page.getByRole('checkbox', { name: 'Animated encode for every episode here', exact: true }).check({ timeout: 5000 });
      await page.getByRole('listbox', { name: 'Poster choices' }).getByRole('option').nth(1).click({ timeout: 5000 });
    }],
    ['review-series-done', async () => { await button('Done').click({ timeout: 5000 }); }],
    ['review-season-edit', async () => {
      await button('Open Edited Show').click({ timeout: 5000 }).catch(async () => { await button('Open Fixture Show').click({ timeout: 5000 }); });
      await button('Review season').click({ timeout: 5000 });
      await fill('Title', 'Season Edited'); await fill('Certification', 'TV-14'); await fill('Plot', 'Season plot'); await fill('Comment', 'Season comment');
      await page.getByRole('listbox', { name: 'Poster choices' }).getByRole('option').nth(0).click({ timeout: 5000 });
    }],
    ['review-season-done', async () => { await button('Done').click({ timeout: 5000 }); }],
    ['review-episode-edit', async () => {
      await page.getByRole('button', { name: /^Open Season/ }).first().click({ timeout: 5000 });
      await button('Review title and poster for S01E02.Second.mkv').click({ timeout: 5000 });
      await fill('Season', 'x'); await fill('Season', '2'); await fill('Episode', '0'); await fill('Episode', '3');
      await fill('Title', 'Edited Second'); await fill('Plot', 'Episode plot'); await fill('Air date', '2020-02-02'); await fill('Certification', 'TV-14'); await fill('Comment', 'Episode note');
      await page.getByRole('checkbox', { name: 'Animated encode for this episode', exact: true }).check({ timeout: 5000 });
    }],
    ['review-episode-close', async () => { await button('Close media file review').click({ timeout: 5000 }); }],
    ['review-movie-edit', async () => {
      await page.reload(); await settle(); await page.keyboard.press('Alt+1'); await settle();
      await button('Review title and poster for Film (2001).mkv').click({ timeout: 5000 });
      await fill('Title', 'Edited Film'); await fill('Year', '2002'); await fill('Certification', 'R'); await fill('Comment', 'Film comment');
    }],
  ];
  for (const [label, act] of reviewSteps) {
    await act();
    const aria = await settle();
    snapshots[`seeded:${label}|${theme}|${width}`] = { dom: await fingerprint(), aria, writes: await page.evaluate(() => (window.__goldenQueueWrites ?? []).splice(0)) };
  }
}
