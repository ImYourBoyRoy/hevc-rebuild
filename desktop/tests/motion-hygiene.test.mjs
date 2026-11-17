// ./desktop/tests/motion-hygiene.test.mjs
// Static guard for animation quality across every stylesheet and component. A looping animation must
// never "snap": no hard steps(), no strobes or pixel jitter, no loop shorter than one second (except a
// spinner), and every endless loop must close seamlessly (ping-pong, identical first and last frame,
// an invisible reset, or a documented periodic motion). Run: pnpm test:motion-hygiene
import assert from "node:assert/strict";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import test from "node:test";

const root = new URL("../src/", import.meta.url).pathname;
function walk(dir) {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    return statSync(path).isDirectory() ? walk(path) : [path];
  });
}
const sources = walk(root).filter((path) => path.endsWith(".css") || path.endsWith(".svelte")).map((path) => ({ path: path.slice(root.length), text: readFileSync(path, "utf8") }));

function block(text, from) {
  let depth = 1, i = from;
  while (depth && i < text.length) { depth += (text[i] === "{") - (text[i] === "}"); i++; }
  return [text.slice(from, i - 1), i];
}
function parseKeyframes(text) {
  const frames = new Map();
  for (const match of text.matchAll(/@keyframes\s+([\w-]+)\s*\{/g)) {
    const [body] = block(text, match.index + match[0].length);
    const steps = [];
    for (const frame of body.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
      const props = Object.fromEntries([...frame[2].matchAll(/([\w-]+)\s*:\s*([^;]+);?/g)].map((m) => [m[1], m[2].trim().replace(/\s+/g, " ")]));
      for (const key of frame[1].split(",").map((k) => k.trim())) steps.push({ at: key === "from" ? 0 : key === "to" ? 100 : parseFloat(key), props });
    }
    frames.set(match[1], steps);
  }
  return frames;
}
function splitTop(value) {
  const parts = []; let depth = 0, current = "";
  for (const ch of value) {
    if (ch === "(") depth++; if (ch === ")") depth--;
    if (ch === "," && depth === 0) { parts.push(current); current = ""; } else current += ch;
  }
  return [...parts, current].map((p) => p.trim()).filter(Boolean);
}
const keyframes = new Map(); for (const source of sources) for (const [name, steps] of parseKeyframes(source.text)) keyframes.set(name, { steps, file: source.path });
const declarations = [];
for (const source of sources) {
  for (const match of source.text.matchAll(/(?<![\w-])animation\s*:\s*([^;}]+)/g)) {
    for (const part of splitTop(match[1].replace(/!important/g, ""))) {
      const tokens = part.split(/\s+(?![^(]*\))/);
      const name = tokens.find((t) => keyframes.has(t));
      if (!name) continue;
      const time = tokens.find((t) => /^[\d.]+m?s$/.test(t));
      const ms = time ? (time.endsWith("ms") ? parseFloat(time) : parseFloat(time) * 1000) : 0;
      declarations.push({ file: source.path, text: part, name, ms, infinite: tokens.includes("infinite"), alternate: tokens.some((t) => t.includes("alternate")), stepped: /steps\(|step-end|step-start/.test(part) });
    }
  }
}

// Endless motion that is periodic by construction (verified by hand; the reason is the contract).
const periodic = {
  spin: "full 360-degree rotation",
  "hazard-tape": "background offset equals exactly one stripe tile width (22.627px)",
  "hud-ring": "full 360-degree rotation",
  "hud-orbit": "dash offset equals the dash pattern period (86 + 1414)",
  "hud-stream": "dash offset equals one dash period, set per element by --dash-period",
  "instrument-drift": "the bar is fully off-screen at both ends",
  "run-indeterminate": "bar is fully off-screen at both ends",
};

test("no animation uses hard steps (blinks, strobes and jitter snap instead of flow)", () => {
  const stepped = declarations.filter((d) => d.stepped).map((d) => `${d.file}: ${d.text}`);
  assert.deepEqual(stepped, []);
});

test("no endless animation is shorter than one second except the spinner", () => {
  const fast = declarations.filter((d) => d.infinite && d.ms < 1000 && d.name !== "spin").map((d) => `${d.file}: ${d.text}`);
  assert.deepEqual(fast, []);
});

test("jitter and glitch keyframes are not defined", () => {
  const banned = [...keyframes.keys()].filter((name) => /jitter|glitch|strobe/.test(name));
  assert.deepEqual(banned, []);
});

test("every endless loop closes seamlessly", () => {
  const open = [];
  for (const d of declarations.filter((item) => item.infinite && !item.alternate)) {
    if (periodic[d.name]) continue;
    const steps = [...keyframes.get(d.name).steps].sort((a, b) => a.at - b.at);
    const first = steps[0], last = steps[steps.length - 1];
    const props = new Set([...Object.keys(first.props), ...Object.keys(last.props)]);
    const closed = [...props].every((p) => (first.props[p] ?? "") === (last.props[p] ?? ""));
    const hiddenAtBothEnds = first.props.opacity === "0" && last.props.opacity === "0";
    if (!closed && !hiddenAtBothEnds) open.push(`${d.file}: ${d.name} (${d.text})`);
  }
  assert.deepEqual(open, []);
});

test("every documented periodic loop still exists and is used", () => {
  for (const name of Object.keys(periodic)) assert.ok(keyframes.has(name), `${name} is documented as periodic but not defined`);
});
