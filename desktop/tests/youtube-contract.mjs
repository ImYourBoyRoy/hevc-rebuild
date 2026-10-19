import {runBrowserContract} from './browser-contract-runner.mjs';
async function contract(page){
  const callback=page.getByRole('status',{name:'YouTube callback'});
  await page.getByRole('heading',{name:'YouTube prep.',exact:true}).waitFor();
  const text=await page.locator('main').textContent();
  for(const policy of ['normal YouTube uploads','YouTube Live','Dolby Vision files are skipped','HDR10+ files are skipped','individual file','Source unchanged']){if(!text.toLowerCase().includes(policy.toLowerCase()))throw new Error(`Policy ${policy} lost`);}
  if(!await page.getByRole('button',{name:'Create upload copies',exact:true}).isDisabled())throw new Error('No-source gate lost');
  for(const [name,value]of[['Add folder','folder'],['Add files','files'],['Open Settings','settings'],['Choose folder','output']]){await page.getByRole('button',{name,exact:true}).click();if(await callback.textContent()!==value)throw new Error(name);}
  await page.getByRole('combobox',{name:'Minimum output tier'}).selectOption('4k');if(!(await callback.textContent()).includes('4k'))throw new Error('Resolution persistence');
  await page.getByRole('combobox',{name:'Bitrate profile'}).selectOption('recommended');if(!await page.getByRole('slider',{name:'Balanced source size target multiplier'}).isDisabled())throw new Error('Profile multiplier gate');
  await page.getByRole('button',{name:'Use default',exact:true}).click();if(!(await callback.textContent()).includes('null'))throw new Error('Output default contract');
  await page.getByRole('checkbox',{name:'YouTube active',exact:true}).check();await page.getByRole('button',{name:'Stop after current file',exact:true}).click();if(await callback.textContent()!=='stop')throw new Error('Stop contract');
  await page.getByRole('checkbox',{name:'YouTube active',exact:true}).uncheck();await page.getByRole('checkbox',{name:'YouTube busy',exact:true}).check();if(!await page.getByRole('button',{name:'Add folder',exact:true}).isDisabled())throw new Error('Busy gate');
  return {independentOpen:true,normalLiveGuidance:true,hdrPolicy:true,noSource:true,preferences:true,outputBoundary:true,failure:true,stop:true,busy:true};
}
await runBrowserContract('youtube',contract);
