// ./desktop/tests/theme-transition.test.mjs
// Contract for the theme-switch transition runner (src/lib/theme-transition.ts): a cross-fade through
// the View Transitions API when the webview has it, a CSS transition window otherwise, and a plain
// instant switch under reduced motion. Run: pnpm test:theme-transition
import assert from "node:assert/strict";
import test from "node:test";
import { createThemeTransition } from "../src/lib/theme-transition.ts";

function rig({ reduced = false, viewTransitions = true, throwOnStart = false, stall = false, fallbackMs = 700, guardMs = 1800 } = {}) {
  const classes = new Set();
  const events = [];
  const timers = [];
  let nextTimer = 1;
  const root = { classList: { add: (name) => { classes.add(name); events.push(["add", name]); }, remove: (name) => { classes.delete(name); events.push(["remove", name]); } } };
  const finished = { resolve: () => {}, reject: () => {} };
  const document = { documentElement: root };
  if (viewTransitions) {
    document.startViewTransition = (callback) => {
      if (throwOnStart) throw new Error("not allowed");
      events.push(["start"]);
      const done = new Promise((resolve, reject) => { finished.resolve = resolve; finished.reject = reject; });
      if (!stall) Promise.resolve().then(callback);
      return { finished: done, skipTransition: () => { events.push(["skip"]); } };
    };
  }
  const run = createThemeTransition({
    document, prefersReducedMotion: () => reduced, settle: async () => { events.push(["settle"]); }, fallbackMs, guardMs,
    setTimeout: (fn, ms) => { timers.push({ id: nextTimer, fn, ms, live: true }); return nextTimer++; },
    clearTimeout: (id) => { const timer = timers.find((item) => item.id === id); if (timer) timer.live = false; events.push(["clear", id]); },
  });
  return { run, classes, events, timers, finished };
}

test("reduced motion switches instantly with no class, no transition and no timer", () => {
  const r = rig({ reduced: true });
  let applied = 0;
  r.run(() => { applied++; });
  assert.equal(applied, 1);
  assert.deepEqual([...r.classes], []);
  assert.deepEqual(r.events, []);
  assert.equal(r.timers.length, 0);
});

test("with View Transitions the class brackets the transition and the update waits for the DOM to settle", async () => {
  const r = rig();
  const order = [];
  r.run(() => { order.push("update"); });
  assert.ok(r.classes.has("theme-vt"));
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.deepEqual(order, ["update"]);
  assert.deepEqual(r.events.map((event) => event[0]), ["add", "start", "settle"]);
  r.finished.resolve();
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(r.classes.has("theme-vt"), false);
});

test("a transition that ends in rejection still removes the class", async () => {
  const r = rig();
  r.run(() => {});
  r.finished.reject(new Error("skipped"));
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(r.classes.has("theme-vt"), false);
});

test("if starting a view transition throws, the CSS fallback runs and the update still happens exactly once", () => {
  const r = rig({ throwOnStart: true });
  let applied = 0;
  r.run(() => { applied++; });
  assert.equal(applied, 1);
  assert.deepEqual([...r.classes], ["theme-fade"]);
  assert.equal(r.timers.at(-1).ms, 700);
});

test("without the API the fade class is held for the fallback window, then removed", () => {
  const r = rig({ viewTransitions: false });
  let applied = 0;
  r.run(() => { applied++; });
  assert.equal(applied, 1);
  assert.deepEqual([...r.classes], ["theme-fade"]);
  assert.equal(r.timers.length, 1);
  r.timers[0].fn();
  assert.deepEqual([...r.classes], []);
});

test("a second switch inside the window extends it instead of dropping the class early", () => {
  const r = rig({ viewTransitions: false });
  r.run(() => {});
  r.run(() => {});
  assert.equal(r.timers.length, 2);
  assert.equal(r.timers[0].live, false);
  r.timers[0].fn?.call(null);
  r.timers[1].fn();
  assert.deepEqual([...r.classes], []);
});

test("a view transition that never renders (hidden window) still applies the theme and clears the class", () => {
  const r = rig({ stall: true });
  let applied = 0;
  r.run(() => { applied++; });
  assert.equal(applied, 0);
  assert.ok(r.classes.has("theme-vt"));
  const guard = r.timers.at(-1);
  assert.equal(guard.ms, 1800);
  guard.fn();
  assert.equal(applied, 1);
  assert.equal(r.classes.has("theme-vt"), false);
  assert.ok(r.events.some((event) => event[0] === "skip"));
});

test("when the transition completes normally the guard is cancelled and the update ran only once", async () => {
  const r = rig();
  let applied = 0;
  r.run(() => { applied++; });
  const guard = r.timers.at(-1);
  await new Promise((resolve) => setTimeout(resolve, 0));
  r.finished.resolve();
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(applied, 1);
  assert.equal(guard.live, false);
  guard.fn();
  assert.equal(applied, 1);
});
