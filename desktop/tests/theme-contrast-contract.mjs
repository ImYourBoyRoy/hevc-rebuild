// ./desktop/tests/theme-contrast-contract.mjs
// Browser contract: every visible piece of text on the Home, Library (with its queue, toolbar, file list
// and search expanded) and Reports table screens meets WCAG AA contrast
// (4.5:1, or 3:1 for large text) in all eight themes. A gradient fill is judged by the worst stop of
// its bottom-most layer (upper layers are decoration such as reticle ticks or hazard stripes).
// Run: pnpm test:theme-contrast
import { runBrowserContract } from './browser-contract-runner.mjs';
async function contract(page) {
  const themes = ['light', 'dark', 'cinema', 'oled', 'dos', 'phosphor', 'ion', 'breach'];
  const results = {};
  for (const screen of ['home', 'library', 'reports']) {
  const target = new URL(page.url()); target.searchParams.set('page', screen);
  await page.goto(target.toString());
  await page.addStyleTag({ content: '*,*::before,*::after{transition:none !important;animation:none !important}' });
  await page.evaluate(() => document.querySelectorAll('details').forEach((d) => { d.open = true; }));
  for (const theme of themes) {
    const key = `${screen}:${theme}`;
    await page.getByRole('combobox', { name: 'Appearance' }).selectOption(theme);
    await page.waitForFunction((id) => document.querySelector('.app-shell')?.dataset.palette === id, theme);
    results[key] = await page.evaluate(() => {
      const canvas = document.createElement('canvas'); canvas.width = canvas.height = 1;
      const context = canvas.getContext('2d', { willReadFrequently: true });
      const rgba = (css) => { context.clearRect(0, 0, 1, 1); context.fillStyle = '#000'; context.fillStyle = css; context.fillRect(0, 0, 1, 1); const d = context.getImageData(0, 0, 1, 1).data; return [d[0], d[1], d[2], d[3] / 255]; };
      const over = (f, b) => [0, 1, 2].map((i) => f[i] * f[3] + b[i] * (1 - f[3]));
      const lum = (c) => { const t = c.map((v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4); }); return 0.2126 * t[0] + 0.7152 * t[1] + 0.0722 * t[2]; };
      const ratio = (a, b) => { const [hi, lo] = [lum(a), lum(b)].sort((p, q) => q - p); return (hi + 0.05) / (lo + 0.05); };
      const bottomLayer = (image) => { let depth = 0, start = 0, last = image; for (let i = 0; i < image.length; i++) { const ch = image[i]; if (ch === '(') depth++; else if (ch === ')') depth--; else if (ch === ',' && depth === 0) { last = image.slice(i + 1); start = i + 1; } } return start ? last : image; };
      const stops = (image) => (bottomLayer(image).match(/(?:rgba?|color|oklab|oklch|lab|lch|hsl)\([^)]*\)|#[0-9a-fA-F]{3,8}\b/g) || []).map(rgba).filter((c) => c[3] > 0.5);
      const backgrounds = (element) => {
        const layers = []; let solid = null;
        for (let node = element; node; node = node.parentElement) {
          const style = getComputedStyle(node); const color = rgba(style.backgroundColor);
          if (!layers.length && style.backgroundImage.includes('gradient')) { const own = stops(style.backgroundImage); if (own.length) return own.map((c) => c.slice(0, 3)); }
          if (color[3] > 0) layers.push(color);
          if (color[3] >= 0.999) { solid = color; break; }
        }
        let base = solid ? [255, 255, 255] : [255, 255, 255];
        for (let i = layers.length - 1; i >= 0; i--) base = over(layers[i], base);
        return [base];
      };
      let checked = 0; const failures = [];
      for (const element of document.querySelectorAll('.app-shell *')) {
        if (![...element.childNodes].some((n) => n.nodeType === 3 && n.textContent.trim())) continue;
        const style = getComputedStyle(element);
        if (style.visibility === 'hidden' || style.display === 'none' || Number(style.opacity) === 0) continue;
        const box = element.getBoundingClientRect(); if (!box.width || !box.height) continue;
        const size = parseFloat(style.fontSize); const large = size >= 24 || (size >= 18.66 && Number(style.fontWeight) >= 700);
        const fg = rgba(style.color); checked++;
        const worst = Math.min(...backgrounds(element).map((bg) => ratio(fg[3] < 1 ? over(fg, bg) : fg.slice(0, 3), bg)));
        if (worst < (large ? 3 : 4.5)) failures.push(`${element.textContent.trim().slice(0, 24)} (${element.getAttribute('class') || element.tagName}) ${worst.toFixed(2)}:1`);
      }
      return { checked, failures };
    });
    if (results[key].checked < 30) throw new Error(`${key}: only ${results[key].checked} text elements were audited`);
    if (results[key].failures.length) throw new Error(`${key}: ${results[key].failures.join('; ')}`);
  }
  }
  return { themes: themes.length, screens: 3, audited: Object.fromEntries(Object.entries(results).map(([k, v]) => [k, v.checked])) };
}
await runBrowserContract('chrome', contract);
