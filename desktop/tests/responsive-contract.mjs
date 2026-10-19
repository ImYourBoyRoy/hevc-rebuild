import {runBrowserContract} from './browser-contract-runner.mjs';
async function contract(page){
  const widths=[360,390,620,800,1024,1440],themes=['light','dark','phosphor','ion','breach'];
  const results=[];
  for(const fixture of ['home','help','library','run','youtube','support','settings','repair','recovery','rename']){
    await page.goto(`http://127.0.0.1:5181/?fixture=${['settings','repair','recovery'].includes(fixture)?'support':fixture==='help'?'home':fixture}`);await page.locator('main').waitFor();
    if(fixture==='home')await page.getByRole('checkbox',{name:'Populated queue'}).check();
    if(fixture==='help')await page.getByRole('button',{name:'Toggle Home/Help'}).click();
    if(['settings','repair','recovery'].includes(fixture)){await page.getByRole('combobox',{name:'Support view'}).selectOption(fixture);if(fixture!=='settings')await page.getByRole('button',{name:'Open support review'}).click();}
    if(fixture==='rename')await page.getByRole('button',{name:'Open rename fixture'}).click();
    if(fixture==='library')await page.getByRole('checkbox',{name:'Populated library'}).check();
    for(const width of widths){
      await page.setViewportSize({width,height:1000});
      for(const theme of themes){
        if(fixture==='home')await page.getByRole('combobox',{name:'Fixture theme'}).selectOption(theme);
        await page.evaluate(theme=>{for(const e of [document.documentElement,document.querySelector('.app-shell')]){e.dataset.theme=theme;if(['phosphor','ion','breach'].includes(theme))e.dataset.instrument=theme;else delete e.dataset.instrument;}const m=document.querySelector('main');m.style.overflow='auto';m.style.maxHeight='100vh';},theme);
        await page.evaluate(()=>document.fonts.ready);
        const metrics=await page.evaluate(()=>({viewport:innerWidth,document:document.documentElement.scrollWidth,main:document.querySelector('main').scrollWidth,mainClient:document.querySelector('main').clientWidth,tokens:getComputedStyle(document.querySelector('.app-shell')).getPropertyValue('--theme-text').trim(),floor:getComputedStyle(document.body).minWidth}));
        results.push({fixture,width,theme,...metrics});
        await page.screenshot({path:`__EVIDENCE__/${fixture}-${theme}-${width}.png`,fullPage:true});
      }
    }
  }
  await page.setViewportSize({width:720,height:500});
  await page.evaluate(()=>{document.documentElement.style.zoom='1';document.querySelector('main').style.maxHeight='100vh';});
  const zoom=await page.evaluate(()=>({main:document.querySelector('main').scrollWidth,client:document.querySelector('main').clientWidth}));
  await page.screenshot({path:'__EVIDENCE__/rename-200-percent-layout.png',fullPage:true});
  if(zoom.main>zoom.client+1)throw new Error(JSON.stringify({zoom}));
  const failures=results.filter(result=>result.document>result.viewport+1||result.main>result.mainClient+1||!result.tokens||result.floor!=='360px');
  if(failures.length)throw new Error(JSON.stringify({overflow:failures,checked:results.length}));
  return {zoom:'720 CSS-pixel effective-viewport proxy for 1440px at 200%; actual browser/native zoom unverified',screenshots:results.length+1,widths,themes,overflow:false,canonicalTokens:true};
}
await runBrowserContract('home',contract);
