import { runBrowserContract } from './browser-contract-runner.mjs';
async function contract(page) {
  const callback=page.getByRole('status',{name:'Library callback'});
  if (!await page.getByRole('button',{name:'Rebuild video',exact:true}).isDisabled()) throw new Error('Empty library runnable');
  if (!await page.getByText('Reading 1 of 2',{exact:true}).isVisible()) throw new Error('Scan progress lost');
  await page.getByRole('checkbox',{name:'Populated library'}).check();
  for (const [name,value] of [['series · 0','activate:/synthetic/series'],['Refresh','refresh:/synthetic/series'],['Remove series from queue','remove:/synthetic/series']]) {
    await page.getByRole('button',{name,exact:true}).click(); if(await callback.textContent()!==value) throw new Error(name);
  }
  await page.getByRole('checkbox',{name:'Select Synthetic series',exact:true}).check();
  const selection=page.getByRole('status',{name:'Selected paths'});
  if(await selection.textContent()!=='/synthetic/series/S01E01.mkv|/synthetic/series/S01E02.mkv') throw new Error('Grouped selection contract');
  for (const [name,value] of [['Rename files','rename'],['Metadata-only MKV','metadata'],['Rebuild video','rebuild'],['Review series','review-series'],['Open file location','location:/synthetic/series/S01E01.mkv']]) {
    await page.getByRole('button',{name,exact:true}).click(); if(await callback.textContent()!==value) throw new Error(name);
  }
  await page.getByText('Streams and output',{exact:true}).click();
  for(const [name,value] of [['Set output folder','output:/synthetic/series/S01E01.mkv'],['Use default','clear:/synthetic/series/S01E01.mkv']]) {await page.getByRole('button',{name,exact:true}).click();if(await callback.textContent()!==value)throw new Error(name);}
  await page.getByRole('checkbox',{name:'Active job',exact:true}).check();
  if(!await page.getByRole('button',{name:'Refresh',exact:true}).isDisabled()||!await page.getByRole('button',{name:'Rebuild video',exact:true}).isDisabled())throw new Error('Busy gates lost');
  await page.getByRole('checkbox',{name:'Active job',exact:true}).uncheck();
  for(const level of ['series','season','episode']) {
    await page.getByRole('combobox',{name:'Review level'}).selectOption(level);
    await page.getByRole('textbox',{name:'Title',exact:true}).fill(`Reviewed ${level}`);
    if(await page.getByRole('textbox',{name:'Title',exact:true}).inputValue()!==`Reviewed ${level}`) throw new Error('Review field callback');
    await page.getByRole('button',{name:'Use the shared certificate',exact:true}).click(); if(await callback.textContent()!=='shared-certificate') throw new Error('Inheritance lost');
    await page.getByRole('button',{name:'Use the shared comment',exact:true}).click(); if(await callback.textContent()!=='shared-comment') throw new Error('Comment inheritance lost');
    if(level==='episode'){await page.getByRole('textbox',{name:'Episode',exact:true}).fill('3');if(await callback.textContent()!=='episode:3')throw new Error('Episode identity lost');}
  }
  await page.getByRole('option',{name:'Poster',exact:true}).click(); if(await callback.textContent()!=='poster:fixture-poster')throw new Error('Poster boundary lost');
  await page.getByRole('button',{name:'Use your own image',exact:true}).click();if(await callback.textContent()!=='choose-image')throw new Error('Image callback lost');
  return {emptyPopulated:true,scan:true,sourceActions:3,selection:true,output:true,busyGates:true,reviewLevels:3,inheritance:true,posters:true};
}
await runBrowserContract('library',contract);
