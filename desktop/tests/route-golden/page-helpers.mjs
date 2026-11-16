// ./desktop/tests/route-golden/page-helpers.mjs
// Route golden section (T10.3): DOM fingerprint and the settle wait used by every snapshot.
// Serialized into the browser contract by route-golden-contract.mjs; it must stay self-contained.

export function createPageHelpers(page) {
  // Structure the accessibility tree does not show: tag counts and every image source (poster choices).
  async function fingerprint() {
    return page.evaluate(() => {
      const counts = {};
      for (const element of document.body.querySelectorAll('*')) {
        if (element.closest('svg')) continue;
        const tag = element.tagName.toLowerCase();
        counts[tag] = (counts[tag] ?? 0) + 1;
      }
      const images = [...document.body.querySelectorAll('img')].map((image) => image.getAttribute('src') ?? '').map((src) => (src.length > 80 ? src.slice(0, 40) + '…' + src.length : src));
      return { theme: document.documentElement.dataset.theme ?? null, instrument: document.documentElement.dataset.instrument ?? null, counts, images, title: document.title };
    });
  }

  // Wait for a stable tree: three identical reads in a row. Lazy chunks (the metadata workbench)
  // can arrive late on a cold dev server, so a single repeat is not enough.
  async function settle() {
    let previous = '';
    let repeats = 0;
    for (let attempt = 0; attempt < 240; attempt++) {
      await page.waitForTimeout(250);
      const current = await page.locator('body').ariaSnapshot();
      repeats = current === previous ? repeats + 1 : 0;
      // A loaded machine can leave the first reads identical while startup is still running.
      // The app shell must also be mounted: two identical blank reads would otherwise end the wait early.
      if (repeats >= 2 && !current.includes('Checking engine') && current.includes('navigation "Main navigation"')) return current;
      previous = current;
    }
    throw new Error('Route never settled');
  }

  return { fingerprint, settle };
}
