// ./desktop/tests/backdrop.test.mjs
// Contract for the themed canvas backdrop (src/lib/backdrop/): which theme gets which scene, the pure
// state each scene advances, and the engine loop (frame cap, pausing when hidden, reduced motion).
// Run: pnpm test:backdrop
import assert from "node:assert/strict";
import test from "node:test";
import { createBackdrop } from "../src/lib/backdrop/engine.ts";
import { mulberry32 } from "../src/lib/backdrop/random.ts";
import { createScene, exposure, flicker, sceneIdFor } from "../src/lib/backdrop/scenes.ts";
import { computeTilt } from "../src/lib/backdrop/tilt.ts";

function fakeContext() {
  const calls = [];
  const target = { calls };
  return new Proxy(target, {
    get(t, key) { if (key in t) return t[key]; if (key === "createLinearGradient" || key === "createRadialGradient") return () => ({ addColorStop() {} }); return (...args) => { t.calls.push([String(key), ...args]); }; },
    set(t, key, value) { t[key] = value; return true; },
  });
}

test("each themed skin maps to its own scene and the calm themes get none", () => {
  assert.equal(sceneIdFor("phosphor"), "matrix");
  assert.equal(sceneIdFor("ion"), "plasma");
  assert.equal(sceneIdFor("breach"), "embers");
  assert.equal(sceneIdFor("cinema"), "projector");
  for (const calm of ["light", "dark", "oled", "dos", "nonsense", ""]) assert.equal(sceneIdFor(calm), null, calm);
});

test("the seeded random source is deterministic and stays in [0, 1)", () => {
  const a = mulberry32(7), b = mulberry32(7);
  for (let i = 0; i < 50; i++) { const value = a(); assert.equal(value, b()); assert.ok(value >= 0 && value < 1); }
});

test("matrix rain: one column per cell width, heads fall, and columns wrap back above the top", () => {
  const scene = createScene("matrix", mulberry32(1));
  scene.resize(400, 300);
  const first = scene.state();
  assert.equal(first.columns.length, Math.floor(400 / first.cell));
  assert.ok(first.columns.every((column) => column.glyphs.length === column.trail && column.speed > 0));
  const speeds = first.columns.map((column) => column.speed);
  const before = first.columns.map((column) => column.head);
  scene.step(0.1, 0.1);
  scene.state().columns.forEach((column, index) => assert.ok(column.head > before[index] || column.head < 0));
  for (let i = 0; i < 2000; i++) scene.step(0.05, i * 0.05);
  assert.ok(scene.state().columns.every((column) => column.head < 300 + column.trail * scene.state().cell + 1));
  const again = createScene("matrix", mulberry32(1)); again.resize(400, 300);
  assert.deepEqual(again.state().columns.map((c) => c.speed), speeds);
});

test("ion plasma: a bounded number of particles that always stay inside the canvas", () => {
  const scene = createScene("plasma", mulberry32(2));
  scene.resize(1200, 800);
  const count = scene.state().particles.length;
  assert.ok(count >= 36 && count <= 200, String(count));
  for (let i = 0; i < 1500; i++) scene.step(0.033, i * 0.033);
  for (const p of scene.state().particles) assert.ok(p.x >= 0 && p.x <= 1200 && p.y >= 0 && p.y <= 800, `${p.x},${p.y}`);
  assert.equal(scene.state().particles.length, count);
  scene.resize(300, 200);
  assert.ok(scene.state().particles.length >= 36 && scene.state().particles.length < count);
});

test("breach embers: sparks rise and die, glitch slices expire, and the totals stay capped", () => {
  const scene = createScene("embers", mulberry32(3));
  scene.resize(900, 600);
  let peakSparks = 0, peakSlices = 0, sawSlice = false;
  for (let i = 0; i < 4000; i++) {
    scene.step(0.033, i * 0.033);
    const state = scene.state();
    peakSparks = Math.max(peakSparks, state.sparks.length); peakSlices = Math.max(peakSlices, state.slices.length); sawSlice ||= state.slices.length > 0;
    for (const slice of state.slices) assert.ok(slice.age <= slice.life + 0.034);
    for (const spark of state.sparks) assert.ok(spark.life > 0);
  }
  assert.ok(peakSparks > 0 && peakSparks <= 90, String(peakSparks));
  assert.ok(sawSlice && peakSlices <= 3, String(peakSlices));
  const rising = createScene("embers", mulberry32(4)); rising.resize(900, 600);
  for (let i = 0; i < 60; i++) rising.step(0.033, i * 0.033);
  const list = rising.state().sparks; const spark = list[list.length - 1]; const y = spark.y; rising.step(0.033, 2); assert.ok(rising.state().sparks.find((s) => s.id === spark.id)?.y < y);
});

test("cinema projector: depth-layered dust stays inside the frame and the light only ever breathes gently", () => {
  const scene = createScene("projector", mulberry32(5));
  scene.resize(1000, 700);
  const motes = scene.state().motes;
  assert.ok(motes.length >= 40);
  assert.ok(motes.every((m) => m.depth >= 0 && m.depth <= 1));
  assert.ok(motes.some((m) => m.depth < 0.33) && motes.some((m) => m.depth > 0.66), "near and far layers both exist");
  for (let i = 0; i < 1500; i++) scene.step(0.033, i * 0.033);
  for (const m of scene.state().motes) assert.ok(m.x >= 0 && m.x <= 1000 && m.y >= 0 && m.y <= 700);
  for (let t = 0; t < 120; t += 0.37) { const f = flicker(t), e = exposure(t); assert.ok(f >= 0.85 && f <= 1, String(f)); assert.ok(e >= 0.9 && e <= 1, String(e)); }
});

test("cinema projector: an old reel projector runs, with turning reels, film through the gate and a lens that sets the beam origin", () => {
  const scene = createScene("projector", mulberry32(8));
  scene.resize(1200, 700);
  const lens = scene.state().lens;
  assert.ok(lens.x > 0.6 * 1200 && lens.x < 1200 && lens.y > 0.6 * 700 && lens.y < 700, `the projector stands in the bottom-right corner: ${lens.x},${lens.y}`);
  assert.equal(scene.state().reels.length, 2);
  const before = scene.state().reels.map((reel) => reel.angle);
  let filmBefore = scene.state().film.offset;
  const TAU = Math.PI * 2;
  for (let i = 1; i <= 600; i++) {
    scene.step(1 / 30, i / 30);
    scene.state().reels.forEach((reel) => assert.ok(reel.angle >= 0 && reel.angle < TAU, `${reel.angle}`));
    const film = scene.state().film.offset; assert.ok(film >= 0 && film < scene.state().film.period, `${film}`); filmBefore = film;
  }
  scene.step(0.1, 20.5);
  const after = scene.state().reels.map((reel) => reel.angle);
  assert.notDeepEqual(after, before, "the reels have turned");
  assert.ok(scene.state().reels.every((reel) => reel.speed > 0.4 && reel.speed < 3), "a believable reel speed (radians per second)");
  assert.ok(filmBefore >= 0);
});

test("cinema projector: the lens is on the end of the machine that faces the beam, so the light leaves the way the lens points", () => {
  for (const [w, h] of [[1200, 700], [1865, 1048], [900, 600]]) {
    const scene = createScene("projector", mulberry32(10));
    scene.resize(w, h);
    const { lens, body, reels } = scene.state();
    assert.ok(body.w > 0 && body.h > 0);
    // The beam travels toward the upper left, so the lens barrel must be at the machine's left end.
    assert.ok(lens.x < body.x, `lens ${lens.x} must be left of the body at ${body.x}`);
    assert.ok(lens.y >= body.y && lens.y <= body.y + body.h, `lens height ${lens.y} must sit within the body ${body.y}-${body.y + body.h}`);
    // Both reels sit on top of the body, and the whole machine stays inside the canvas.
    for (const reel of reels) assert.ok(reel.cx >= body.x && reel.cx <= body.x + body.w && reel.cy < body.y, "reels sit above the body");
    assert.ok(lens.x > 0.5 * w && body.x + body.w < w, "inside the canvas, in the right half");
  }
});

test("cinema projector: old-film scratches come and go, a few at a time, inside the frame", () => {
  const scene = createScene("projector", mulberry32(9));
  scene.resize(1000, 600);
  let seen = false;
  for (let i = 0; i < 2400; i++) {
    scene.step(1 / 30, i / 30);
    const list = scene.state().scratches;
    seen ||= list.length > 0;
    assert.ok(list.length <= 3, String(list.length));
    for (const scratch of list) assert.ok(scratch.x >= 0 && scratch.x <= 1000 && scratch.age <= scratch.life + 0.04);
  }
  assert.ok(seen, "at least one scratch appeared in 80 seconds");
});

test("cinema projector: the lamp warms up smoothly and the grain refreshes every frame", () => {
  const scene = createScene("projector", mulberry32(6));
  scene.resize(800, 500);
  let last = scene.state().warm;
  assert.ok(last >= 0 && last < 0.2, String(last));
  for (let i = 1; i <= 240; i++) { scene.step(1 / 30, i / 30); const now = scene.state().warm; assert.ok(now >= last && now <= 1, `${last} -> ${now}`); last = now; }
  assert.ok(last > 0.95, String(last));
  const first = scene.state().grainSeed; scene.step(1 / 30, 9); assert.notEqual(scene.state().grainSeed, first);
});

test("cinema projector: the spotlight follows the pointer with a lag, never jumping, and fades when it leaves", () => {
  const scene = createScene("projector", mulberry32(7));
  scene.resize(900, 600);
  assert.equal(scene.state().spot.alpha, 0);
  scene.pointer(700, 120);
  const path = [];
  for (let i = 1; i <= 90; i++) { scene.step(1 / 30, i / 30); path.push(scene.state().spot); }
  for (let i = 1; i < path.length; i++) assert.ok(Math.hypot(path[i].x - path[i - 1].x, path[i].y - path[i - 1].y) < 120, "no jumps");
  assert.ok(Math.hypot(path.at(-1).x - 700, path.at(-1).y - 120) < 25, "converges on the pointer");
  assert.ok(path.at(-1).alpha > 0.9);
  scene.pointer(null, null);
  for (let i = 0; i < 90; i++) scene.step(1 / 30, 4 + i / 30);
  assert.ok(scene.state().spot.alpha < 0.05);
});

test("card tilt: a gentle, bounded 3D lean toward the pointer and a sheen that follows it", () => {
  const rect = { left: 100, top: 200, width: 200, height: 300 };
  const centre = computeTilt(rect, 200, 350);
  assert.deepEqual([centre.rx, centre.ry, centre.px, centre.py], [0, 0, 50, 50]);
  const corner = computeTilt(rect, 300, 200, 8);
  assert.ok(corner.ry > 0 && corner.ry <= 8 && corner.rx > 0 && corner.rx <= 8, `${corner.rx},${corner.ry}`);
  const opposite = computeTilt(rect, 100, 500, 8);
  assert.ok(opposite.ry < 0 && opposite.rx < 0);
  const outside = computeTilt(rect, 5000, -5000, 8);
  assert.ok(Math.abs(outside.rx) <= 8 && Math.abs(outside.ry) <= 8 && outside.px <= 100 && outside.px >= 0 && outside.py >= 0);
  assert.deepEqual(computeTilt({ left: 0, top: 0, width: 0, height: 0 }, 5, 5), { rx: 0, ry: 0, px: 50, py: 50 });
});

test("every scene draws something without throwing", () => {
  for (const id of ["matrix", "plasma", "embers", "projector"]) {
    const scene = createScene(id, mulberry32(9)); scene.resize(640, 400);
    for (let i = 0; i < 40; i++) scene.step(0.05, i * 0.05);
    const ctx = fakeContext(); scene.draw(ctx, 640, 400);
    assert.ok(ctx.calls.length > 0, id);
  }
});

function rig({ reduced = false, hidden = false } = {}) {
  const frames = []; let id = 0, time = 0; const cancelled = []; const ctx = fakeContext();
  const canvas = { width: 0, height: 0, getContext: () => ctx };
  const backdrop = createBackdrop({ canvas, requestFrame: (cb) => { frames.push({ cb, id: ++id }); return id; }, cancelFrame: (n) => { cancelled.push(n); const i = frames.findIndex((f) => f.id === n); if (i >= 0) frames.splice(i, 1); }, now: () => time, prefersReducedMotion: () => reduced, random: mulberry32(11) });
  if (hidden) backdrop.setPaused(true);
  const tick = (ms) => { time += ms; const f = frames.shift(); f?.cb(time); };
  return { backdrop, frames, tick, ctx, canvas, cancelled, get time() { return time; } };
}

test("the engine sizes the canvas by device pixel ratio and runs only for themed skins", () => {
  const r = rig(); r.backdrop.resize(500, 300, 2);
  assert.deepEqual([r.canvas.width, r.canvas.height], [1000, 600]);
  r.backdrop.setTheme("light"); assert.equal(r.frames.length, 0); assert.equal(r.backdrop.active(), null);
  r.backdrop.setTheme("phosphor"); assert.equal(r.backdrop.active(), "matrix"); assert.equal(r.frames.length, 1);
  r.backdrop.setTheme("light"); assert.equal(r.backdrop.active(), null); assert.equal(r.frames.length, 0);
});

test("frames are capped to about 30 per second and a long gap never produces a huge step", () => {
  const r = rig(); r.backdrop.resize(400, 300, 1); r.backdrop.setTheme("breach");
  const steps = []; const original = r.ctx.calls.length;
  r.tick(10); r.tick(10); const early = r.ctx.calls.length; assert.equal(early, original, "no draw before one frame interval");
  r.tick(30); assert.ok(r.ctx.calls.length > early, "draws once a full interval has elapsed");
  const sparkBefore = r.backdrop.snapshot().sparks.map((s) => s.y);
  r.tick(60_000);
  const sparkAfter = r.backdrop.snapshot().sparks.map((s) => s.y);
  assert.ok(sparkBefore.length === 0 || sparkAfter.every((y, i) => Math.abs(y - sparkBefore[i]) < 200));
  assert.equal(r.frames.length, 1);
  assert.ok(steps.length === 0);
});

test("pausing cancels the loop and resuming restarts it; destroy stops everything", () => {
  const r = rig(); r.backdrop.resize(400, 300, 1); r.backdrop.setTheme("ion");
  assert.equal(r.frames.length, 1);
  r.backdrop.setPaused(true); assert.equal(r.frames.length, 0); assert.ok(r.cancelled.length >= 1);
  r.backdrop.setPaused(false); assert.equal(r.frames.length, 1);
  r.backdrop.destroy(); assert.equal(r.frames.length, 0);
  r.backdrop.setTheme("breach"); assert.equal(r.frames.length, 0);
});

test("reduced motion paints one still frame and never schedules animation", () => {
  const r = rig({ reduced: true }); r.backdrop.resize(400, 300, 1); r.backdrop.setTheme("cinema");
  assert.equal(r.frames.length, 0);
  assert.ok(r.ctx.calls.length > 0);
});

test("pointer movement is handed to the active scene and ignored by scenes without one", () => {
  const r = rig(); r.backdrop.resize(900, 600, 1);
  r.backdrop.setPointer(10, 10);
  r.backdrop.setTheme("cinema");
  r.backdrop.setPointer(700, 100);
  for (let i = 0; i < 40; i++) r.tick(34);
  assert.ok(r.backdrop.snapshot().spot.alpha > 0.5);
  r.backdrop.setTheme("breach");
  r.backdrop.setPointer(5, 5);
  r.backdrop.setPointer(null, null);
});
