import { runBrowserContract } from './browser-contract-runner.mjs';
async function contract(page) {
  const last = page.getByRole('status', { name: 'Last callback' });
  for (const [name, action] of [['Add folder', 'folder'], ['Add files', 'files'], ['Open organizer', 'metadata'], ['Open run setup', 'run'], ['Open YouTube prep', 'youtube'], ['Refresh', 'refresh']]) {
    await page.getByRole('button', { name, exact: true }).click();
    if (await last.textContent() !== action) throw new Error(`${name} callback lost`);
  }
  if (!await page.getByText('1 required tool missing', { exact: true }).isVisible()) throw new Error('Missing tools not visible');
  await page.getByRole('combobox', { name: 'Fixture tools' }).selectOption('ready');
  if (!await page.getByText('1 tools ready', { exact: true }).isVisible()) throw new Error('Ready tools not visible');
  await page.getByRole('checkbox', { name: 'Populated queue' }).check();
  await page.getByRole('button', { name: 'Open library', exact: true }).click();
  if (await last.textContent() !== 'library') throw new Error('Populated library callback lost');
  await page.getByRole('button', { name: /run.*1 OK/ }).click();
  if (await last.textContent() !== '/synthetic/run.csv') throw new Error('Exact report reference lost');
  if (!await page.getByText('Package or helper setup', { exact: true }).isVisible()) throw new Error('Setup log lost');
  for (const theme of ['light','dark','phosphor','ion','breach']) {
    await page.getByRole('combobox', { name: 'Fixture theme' }).selectOption(theme);
    const illustration = page.getByRole('figure', { name: 'Local rebuild workflow illustration' });
    if (!await illustration.isVisible() || await illustration.locator('svg').count() !== 1) throw new Error(`${theme} illustration missing`);
  }
  await page.getByRole('button', { name: 'Toggle Home/Help' }).click();
  for (const phrase of ['metadata-only', 'HDR10+', '--mcp', 'Stop safely']) {
    if (!(await page.locator('.help-page').textContent()).includes(phrase)) throw new Error(`Help lost ${phrase}`);
  }
  return { emptyAndPopulated: true, toolsMissingReady: true, actions: 8, recentReport: true, setupLog: true, skins: 5, helpModes: true };
}
await runBrowserContract('home', contract);
