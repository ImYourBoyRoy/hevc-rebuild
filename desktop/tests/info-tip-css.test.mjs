// ./desktop/tests/info-tip-css.test.mjs
// The help popover sits inside <span> elements in settings rows, and `.form-row span` (and similar)
// set `display: grid`, which out-ranks a plain `.info-tip-popover { display: none }`. That left every
// popover permanently open. The popover must therefore be hidden with `visibility`, which no row rule
// touches, and shown only on hover, keyboard focus or when pinned. Run: pnpm test:info-tip-css

import assert from "node:assert/strict";
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import test from "node:test";

const css = readFileSync(new URL("../src/styles/components/info-tip.css", import.meta.url), "utf8");
const rules = [...css.matchAll(/([^{}]+)\{([^{}]*)\}/g)].map((match) => ({ selector: match[1].trim(), body: match[2] }));
const popover = rules.find((rule) => rule.selector === ".info-tip-popover");
const reveal = rules.find((rule) => rule.selector.includes(".info-tip:hover .info-tip-popover"));

test("the popover is hidden with visibility, not only with display", () => {
  assert.ok(popover, "base popover rule");
  assert.match(popover.body, /visibility:\s*hidden/);
  assert.match(popover.body, /pointer-events:\s*none/);
  assert.match(popover.body, /position:\s*absolute/, "it never takes up space in the row");
});

test("it opens on hover, keyboard focus and when pinned, and only then", () => {
  assert.ok(reveal, "reveal rule");
  for (const selector of [".info-tip:hover .info-tip-popover", ".info-tip:focus-within .info-tip-popover", ".info-tip.pinned .info-tip-popover"]) {
    assert.ok(reveal.selector.includes(selector), selector);
  }
  assert.match(reveal.body, /visibility:\s*visible/);
  assert.match(reveal.body, /pointer-events:\s*auto/);
});

test("no other stylesheet rule overrides the popover's visibility", () => {
  const stack = [join(new URL("../src/styles", import.meta.url).pathname)];
  const offenders = [];
  while (stack.length) {
    const path = stack.pop();
    if (statSync(path).isDirectory()) { for (const name of readdirSync(path)) stack.push(join(path, name)); continue; }
    if (!path.endsWith(".css") || path.endsWith("info-tip.css")) continue;
    const text = readFileSync(path, "utf8");
    for (const match of text.matchAll(/([^{}]*info-tip-popover[^{}]*)\{([^{}]*)\}/g)) {
      if (/visibility/.test(match[2])) offenders.push(`${path}: ${match[1].trim()}`);
    }
  }
  assert.deepEqual(offenders, []);
});
