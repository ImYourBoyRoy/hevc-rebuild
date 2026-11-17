// ./desktop/src/lib/backdrop/engine.ts
// The themed canvas backdrop loop. Owns timing and the canvas; the scenes own the motion. Frames are
// capped to about 30 per second, a long gap (a background tab, a debugger) never makes a huge step,
// the loop stops while the window is hidden, and reduced motion paints one settled still frame and
// never schedules animation. Every dependency (canvas, frame scheduler, clock, reduced-motion query,
// random source) is injected so the loop is unit-tested in node (pnpm test:backdrop).

import { mulberry32 } from "./random.ts";
import { createScene, sceneIdFor, type DrawContext, type Rand, type Scene, type SceneId } from "./scenes.ts";

export interface BackdropDeps {
  canvas: { width: number; height: number; getContext(kind: "2d"): any };
  requestFrame: (callback: (time: number) => void) => number;
  cancelFrame: (id: number) => void;
  now: () => number;
  prefersReducedMotion: () => boolean;
  random?: Rand;
}

const FRAME_MS = 1000 / 30;
const MAX_STEP = 0.1;

export function createBackdrop(deps: BackdropDeps) {
  const ctx = deps.canvas.getContext("2d") as DrawContext & { setTransform(a: number, b: number, c: number, d: number, e: number, f: number): void };
  const rand = deps.random ?? mulberry32(Date.now() & 0xffff);
  let scene: Scene | null = null;
  let sceneId: SceneId | null = null;
  let width = 0, height = 0, ratio = 1;
  let frame: number | null = null;
  let last = 0;
  let clock = 0;
  let paused = false;
  let destroyed = false;
  let pointer: { x: number; y: number } | null = null;

  const sized = () => width > 0 && height > 0;
  const paint = () => {
    ctx.clearRect(0, 0, width, height);
    scene?.draw(ctx, width, height);
  };
  const cancel = () => { if (frame !== null) { deps.cancelFrame(frame); frame = null; } };
  const schedule = () => { if (frame === null) frame = deps.requestFrame(loop); };
  function loop(time: number) {
    frame = null;
    if (!scene || paused || destroyed) return;
    const elapsed = time - last;
    if (elapsed >= FRAME_MS) {
      const dt = Math.min(elapsed / 1000, MAX_STEP);
      last = time;
      clock += dt;
      scene.step(dt, clock);
      paint();
    }
    schedule();
  }
  function settle(seconds: number) {
    if (!scene) return;
    for (let t = 0; t < seconds; t += 1 / 30) { clock += 1 / 30; scene.step(1 / 30, clock); }
  }
  function run() {
    if (!scene || destroyed || !sized()) return;
    if (deps.prefersReducedMotion()) { settle(1.5); paint(); return; }
    paint();
    if (!paused) { last = deps.now(); schedule(); }
  }

  return {
    setTheme(theme: string) {
      if (destroyed) return;
      const id = sceneIdFor(theme);
      if (id === sceneId) return;
      cancel();
      sceneId = id;
      scene = id ? createScene(id, rand) : null;
      if (!scene) { ctx.clearRect(0, 0, width, height); return; }
      if (sized()) scene.resize(width, height);
      scene.pointer?.(pointer?.x ?? null, pointer?.y ?? null);
      run();
    },
    resize(cssWidth: number, cssHeight: number, devicePixelRatio: number) {
      width = Math.max(0, Math.floor(cssWidth)); height = Math.max(0, Math.floor(cssHeight));
      ratio = Math.min(2, Math.max(1, devicePixelRatio || 1));
      deps.canvas.width = Math.round(width * ratio); deps.canvas.height = Math.round(height * ratio);
      ctx.setTransform(ratio, 0, 0, ratio, 0, 0);
      if (!scene || !sized()) return;
      scene.resize(width, height);
      if (deps.prefersReducedMotion()) { settle(1.5); paint(); return; }
      paint();
      if (!paused && frame === null) { last = deps.now(); schedule(); }
    },
    setPaused(value: boolean) {
      if (destroyed || paused === value) return;
      paused = value;
      if (paused) cancel();
      else if (scene && sized() && !deps.prefersReducedMotion()) { last = deps.now(); schedule(); }
    },
    /** The pointer in canvas pixels (null when it leaves); only scenes that follow the cursor react. */
    setPointer(x: number | null, y: number | null) {
      pointer = x === null || y === null ? null : { x, y };
      scene?.pointer?.(x, y);
    },
    /** Step and paint by hand (fixtures and tests; the browser pane stops animation frames while hidden). */
    advance(seconds: number) {
      if (!scene) return;
      settle(seconds);
      paint();
    },
    active: (): SceneId | null => sceneId,
    snapshot: () => scene?.state() ?? null,
    destroy() { destroyed = true; cancel(); scene = null; sceneId = null; },
    get ratio() { return ratio; },
  };
}
