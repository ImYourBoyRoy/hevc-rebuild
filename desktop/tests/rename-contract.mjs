import { runBrowserContract } from './browser-contract-runner.mjs';
async function contract(page) {
  const opener=page.getByRole('button',{name:'Open rename fixture'});
  await opener.click();
  const dialog=page.getByRole('dialog',{name:'Name this file',exact:true});
  await dialog.waitFor();
  const pattern=page.getByRole('textbox',{name:'Film filename pattern'});
  await pattern.fill('abc'); await pattern.focus(); await pattern.press('Home');
  await page.getByRole('button',{name:'Title',exact:true}).click();
  if(await pattern.inputValue()!=='{title}abc'||!await pattern.evaluate(el=>el===document.activeElement))throw new Error('Token/caret focus lost');
  await page.getByRole('button',{name:'Standard film name',exact:true}).click();
  if(await pattern.inputValue()!=='{title}{ext}')throw new Error('Reset changed');
  await page.getByRole('button',{name:'Rename this file',exact:true}).click();
  if(!await page.getByText('Synthetic collision: target exists',{exact:true}).isVisible())throw new Error('Collision error hidden');
  await page.keyboard.press('Escape'); if(!await opener.evaluate(el=>el===document.activeElement))throw new Error('Focus not returned');
  await page.getByRole('checkbox',{name:'Whole selection',exact:true}).check();
  await opener.click(); await page.getByRole('dialog',{name:'Name 2 files',exact:true}).waitFor();
  if(await pattern.inputValue()!=='{title}{ext}')throw new Error('Reopen changed draft');
  await page.getByRole('button',{name:'Close rename preview',exact:true}).click();
  await page.getByRole('checkbox',{name:'Rename busy',exact:true}).check(); await opener.click();
  if(!await page.getByRole('button',{name:'Renaming…',exact:true}).isDisabled())throw new Error('Busy gate lost');
  await page.keyboard.press('Escape');
  for(const shape of ['movie','episode']) {await page.getByRole('combobox',{name:'Rename shape'}).selectOption(shape);await opener.click();if(await page.getByRole('textbox',{name:shape==='movie'?'Episode filename pattern':'Film filename pattern'}).count())throw new Error('Wrong shape fields');await page.keyboard.press('Escape');}
  await page.getByRole('heading',{name:'Metadata workbench',exact:true}).waitFor();
  if(!(await page.locator('main').textContent()).includes('separate previews and confirmations'))throw new Error('Metadata safety guidance lost');
  await page.getByRole('button',{name:'TV series',exact:true}).click();
  if(!await page.getByRole('button',{name:'TV series',exact:true}).getAttribute('aria-pressed'))throw new Error('TV mode lost');
  return {tokenCaretFocus:true,reset:true,collision:true,singleWhole:true,reopen:true,busy:true,shapes:true,metadataBrowserBoundary:true};
}
await runBrowserContract('rename',contract);
