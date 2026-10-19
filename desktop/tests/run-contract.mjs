import {runBrowserContract} from './browser-contract-runner.mjs';
async function contract(page){
  const callback=page.getByRole('status',{name:'Run callback'});
  const progress=page.getByRole('progressbar',{name:'Selected queue progress'});
  if(await progress.getAttribute('aria-valuenow')!=='25')throw new Error('Queue progress lost');
  if(!(await page.locator('#run-activity').textContent()).includes('Calibration: synthetic layout'))throw new Error('Calibration log lost');
  if(!(await page.locator('#run-activity').textContent()).includes('/synthetic/run.csv'))throw new Error('Report association log lost');
  await page.getByRole('button',{name:'Open progress details'}).click();if(await callback.textContent()!=='details')throw new Error('Details action');
  await page.getByRole('button',{name:'Stop after this file'}).click();if(await callback.textContent()!=='stop'||!await page.getByRole('button',{name:'Stopping safely…'}).isDisabled())throw new Error('Stop state');
  await page.getByRole('button',{name:'Stop safely',exact:true}).click();
  for(const value of ['unknown','nonfinite']){await page.getByRole('combobox',{name:'Fixture telemetry'}).selectOption(value);const text=await page.getByRole('region',{name:'Live resource telemetry'}).textContent();if(text.includes('NaN')||text.includes('Infinity')||text.includes('0% used'))throw new Error('Unknown telemetry misrepresented');}
  for(const state of ['cancelled','failed','recovered','stale']){await page.getByRole('combobox',{name:'Fixture run state'}).selectOption(state);if(await progress.count()||await page.getByRole('button',{name:'Stop safely',exact:true}).count())throw new Error('Terminal controls stale');if(!(await page.locator('#run-activity').textContent()).includes(`Outcome: ${state}`))throw new Error('Terminal updates lost');}
  return {progressEta:true,logsCalibrationReport:true,stopControls:true,unknownFinite:true,terminalStates:4};
}
await runBrowserContract('run',contract);
