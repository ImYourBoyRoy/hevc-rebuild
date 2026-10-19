import { runBrowserContract } from './browser-contract-runner.mjs';
async function contract(page) {
  const open = page.getByRole('button', { name: 'Open review', exact: true });
  const dialog = page.getByRole('dialog');
  await open.click();
  if (await dialog.getAttribute('aria-labelledby') !== 'fixture-title' || await dialog.getAttribute('aria-describedby') !== 'fixture-description') throw new Error('Accessible labels missing');
  if (await page.evaluate(() => document.activeElement?.textContent) !== 'Cancel') throw new Error('Initial focus must choose the safe action');
  for (const key of ['Tab', 'Shift+Tab']) {
    for (let index = 0; index < 12; index++) {
      await page.keyboard.press(key);
      if (!await page.evaluate(() => document.activeElement?.closest('dialog') !== null)) throw new Error(`${key} focus left dialog`);
    }
  }
  const bounds = await dialog.boundingBox();
  await page.mouse.click(Math.max(0, bounds.x - 2), Math.max(0, bounds.y - 2));
  if (!await dialog.isVisible()) throw new Error('Default backdrop dismissed safety review');
  let blocked = false;
  try { await page.getByRole('button', { name: 'Background action' }).click({ timeout: 300 }); } catch { blocked = true; }
  if (!blocked) throw new Error('Background interaction not inert');
  if (await page.getByRole('button', { name: 'Confirm', exact: true }).isEnabled()) throw new Error('Acknowledgment gate lost');
  await page.getByRole('checkbox', { name: 'I reviewed the operation' }).check();
  await page.getByRole('button', { name: 'Confirm', exact: true }).click();
  if (!await page.getByRole('alert').isVisible() || !await dialog.isVisible()) throw new Error('Validation error state lost');
  await page.getByRole('button', { name: 'Toggle loading' }).click();
  if (await page.getByRole('button', { name: 'Confirm', exact: true }).isEnabled()) throw new Error('Loading gate lost');
  await page.keyboard.press('Escape');
  if (await page.evaluate(() => document.activeElement?.id) !== 'opener') throw new Error('Escape/loading focus return failed');
  await open.click(); await page.getByRole('button', { name: 'Cancel', exact: true }).click();
  if (await page.evaluate(() => document.activeElement?.id) !== 'opener') throw new Error('Cancel focus return failed');
  await page.getByRole('checkbox', { name: 'Allow backdrop dismissal' }).check();
  await open.click(); const box = await dialog.boundingBox();
  await page.mouse.click(Math.max(0, box.x - 2), Math.max(0, box.y - 2));
  if (await dialog.count()) throw new Error('Opt-in backdrop dismissal failed');
  for (const surface of ['Metadata review', 'Library rename', 'Full rebuild', 'Output preview', 'Recovery', 'Tool repair']) {
    await page.getByRole('combobox', { name: 'Surface' }).selectOption(surface);
    await open.click();
    for (let index = 0; index < 8; index++) {
      await page.keyboard.press('Tab');
      if (!await page.evaluate(() => document.activeElement?.closest('dialog') !== null)) throw new Error(`${surface} lost focus containment`);
    }
    await page.keyboard.press('Escape');
    if (await page.evaluate(() => document.activeElement?.id) !== 'opener') throw new Error(`${surface} lost focus return`);
  }
  await open.click();
  await page.getByRole("button", {name:"Continue to next review"}).click();
  await page.getByRole("heading", {name:"Chained synthetic review"}).waitFor();
  await page.keyboard.press("Escape");
  if (await page.evaluate(() => document.activeElement?.id) !== "opener") throw new Error("Chained review lost persistent launch focus");
  return { chainedFocusReturn:true, keyboardPasses: 24, callerSurfaces: 6, initialFocus: 'Cancel', focusReturn: true, inert: true, validation: true, loading: true, backdropPolicy: true };
}
await runBrowserContract('modal', contract);
