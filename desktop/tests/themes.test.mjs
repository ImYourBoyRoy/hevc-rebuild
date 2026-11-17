// ./desktop/tests/themes.test.mjs
// Contract for the theme registry (src/lib/themes.ts): every theme, its family, the DOM attributes
// it publishes and the order the picker shows. Run: pnpm test:themes
import assert from "node:assert/strict";
import test from "node:test";
import { themeAttributes, themeOptions, themes } from "../src/lib/themes.ts";

const ids = ["light", "dark", "cinema", "oled", "dos", "phosphor", "ion", "breach"];

test("the registry lists the eight themes in picker order with unique ids and labels", () => {
  assert.deepEqual(themes.map((theme) => theme.id), ids);
  assert.equal(new Set(themes.map((theme) => theme.label)).size, ids.length);
  assert.deepEqual(themeOptions.map((option) => option.value), ids);
  for (const theme of themes) assert.ok(theme.label.length > 0 && theme.description.length > 0, theme.id);
});

test("palette themes publish their base family so existing light and dark rules apply", () => {
  assert.deepEqual(themeAttributes("light"), { theme: "light", palette: "light", instrument: undefined });
  assert.deepEqual(themeAttributes("dark"), { theme: "dark", palette: "dark", instrument: undefined });
  assert.deepEqual(themeAttributes("cinema"), { theme: "dark", palette: "cinema", instrument: undefined });
  assert.deepEqual(themeAttributes("oled"), { theme: "dark", palette: "oled", instrument: undefined });
  assert.deepEqual(themeAttributes("dos"), { theme: "dark", palette: "dos", instrument: undefined });
});

test("instrument skins publish themselves as theme, palette and instrument", () => {
  for (const skin of ["phosphor", "ion", "breach"]) {
    assert.deepEqual(themeAttributes(skin), { theme: skin, palette: skin, instrument: skin });
  }
});

test("each theme declares whether it is dark or light for the browser colour scheme", () => {
  const scheme = Object.fromEntries(themes.map((theme) => [theme.id, theme.scheme]));
  assert.deepEqual(scheme, { light: "light", dark: "dark", cinema: "dark", oled: "dark", dos: "dark", phosphor: "dark", ion: "dark", breach: "dark" });
});
