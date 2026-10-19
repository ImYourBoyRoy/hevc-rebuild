import { runBrowserContract } from './browser-contract-runner.mjs';
async function contract(page) {
  const navigation = await page.evaluate(() => {
    const entry = performance.getEntriesByType('navigation')[0];
    return { durationMs: entry.duration, domContentLoadedMs: entry.domContentLoadedEventEnd, heapBytes: performance.memory?.usedJSHeapSize ?? null };
  });
  await page.getByRole('checkbox', {name:'Populated library', exact:true}).check();
  const rows = [];
  for (const count of [20, 200]) {
    const start = await page.evaluate(() => performance.now());
    await page.getByRole('spinbutton', {name:'Synthetic card count'}).fill(String(count));
    await page.waitForFunction(expected => document.querySelectorAll('.library-poster-card').length === expected, count);
    const finish = await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve({now:performance.now(),heapBytes:performance.memory?.usedJSHeapSize ?? null})))));
    rows.push({count, interactionAndTwoFramesMs:finish.now-start,heapBytes:finish.heapBytes});
  }
  const queueStart = await page.evaluate(() => performance.now());
  await page.getByRole('button', {name:'Large queue update',exact:true}).click();
  await page.waitForFunction(() => document.querySelector('output[aria-label="Selected paths"]').textContent.split('|').length === 10000);
  const queueMs = await page.evaluate(start => performance.now()-start, queueStart);
  await page.goto('http://127.0.0.1:5181');
  const modal = [];
  for (let index=0;index<10;index++) {
    const start = await page.evaluate(() => performance.now());
    await page.getByRole('button',{name:'Open review',exact:true}).click();
    await page.getByRole('dialog').waitFor();
    await page.keyboard.press('Escape');
    if (await page.evaluate(()=>document.activeElement?.id)!=='opener') throw new Error('Modal performance lost focus return');
    modal.push(await page.evaluate(start=>performance.now()-start,start));
  }
  return {environment:'Chrome isolated Vite component fixtures; production bundle measured separately',navigation,library:rows,queue10000Ms:queueMs,modalOpenEscapeMs:modal};
}
await runBrowserContract('library',contract);
