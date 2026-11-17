// ./desktop/src/lib/backdrop/scenes.ts
// The animated backdrop scenes, one per themed skin: matrix rain (Phosphor), an ion plasma field
// (Ion), rising sparks with eased glitch slices (Breach) and a projector beam with dust motes
// (Cinema). Each scene keeps plain, inspectable state and draws with a small canvas subset, so the
// motion rules (bounds, caps, wrapping, flicker band) are unit-tested in node without a canvas
// (pnpm test:backdrop). Nothing here touches the DOM; the engine owns timing and the canvas.

export interface DrawContext {
  clearRect(x: number, y: number, w: number, h: number): void;
  fillRect(x: number, y: number, w: number, h: number): void;
  strokeRect(x: number, y: number, w: number, h: number): void;
  beginPath(): void;
  moveTo(x: number, y: number): void;
  lineTo(x: number, y: number): void;
  closePath(): void;
  arc(x: number, y: number, r: number, a0: number, a1: number): void;
  fill(): void;
  stroke(): void;
  fillText(text: string, x: number, y: number): void;
  save(): void;
  restore(): void;
  translate(x: number, y: number): void;
  scale(x: number, y: number): void;
  rotate(angle: number): void;
  setLineDash(segments: number[]): void;
  lineDashOffset: number;
  lineCap: string;
  createLinearGradient(x0: number, y0: number, x1: number, y1: number): { addColorStop(offset: number, color: string): void };
  createRadialGradient(x0: number, y0: number, r0: number, x1: number, y1: number, r1: number): { addColorStop(offset: number, color: string): void };
  globalAlpha: number;
  globalCompositeOperation: string;
  fillStyle: unknown;
  strokeStyle: unknown;
  lineWidth: number;
  font: string;
  textAlign: string;
  textBaseline: string;
}

export type SceneId = "matrix" | "plasma" | "embers" | "projector";
export type Rand = () => number;

export interface Scene {
  id: SceneId;
  resize(width: number, height: number): void;
  step(dt: number, time: number): void;
  /** Pointer position in canvas pixels, or null when it leaves (scenes that follow the cursor). */
  pointer?(x: number | null, y: number | null): void;
  draw(ctx: DrawContext, width: number, height: number): void;
  /** Inspectable copy-free view of the scene state (tests and fixtures). */
  state(): any;
}

const clamp = (value: number, min: number, max: number) => Math.min(max, Math.max(min, value));
const between = (rand: Rand, min: number, max: number) => min + rand() * (max - min);

export function sceneIdFor(theme: string): SceneId | null {
  if (theme === "phosphor") return "matrix";
  if (theme === "ion") return "plasma";
  if (theme === "breach") return "embers";
  if (theme === "cinema") return "projector";
  return null;
}

/** The lamp's slow exposure breathing, always within 0.9 to 1. */
export function exposure(time: number): number {
  return 0.96 + 0.04 * Math.sin(time * 0.35);
}

/** The lamp flicker: a gentle band, never a blink. Always within 0.85 to 1. */
export function flicker(time: number): number {
  return 0.935 + 0.045 * Math.sin(time * 1.9) * Math.sin(time * 0.73) + 0.015 * Math.sin(time * 7.1);
}

// ------------------------------------------------------------------ matrix rain (Phosphor)
const GLYPHS = "ｱｲｳｴｵｶｷｸｹｺｻｼｽｾｿﾀﾁﾂﾃﾅﾆﾇﾈﾊﾋﾎﾏﾐﾑﾒﾓﾔﾕﾗﾘﾜ0123456789ABCDEF<>/\\|=+*:;#$%&";
interface Column { x: number; head: number; speed: number; trail: number; glyphs: string[]; acid: boolean; mutate: number }

function matrixScene(rand: Rand): Scene {
  const cell = 18;
  let columns: Column[] = [];
  let height = 0;
  const glyph = () => GLYPHS[Math.floor(rand() * GLYPHS.length)];
  const spawn = (index: number, initial: boolean): Column => {
    const trail = 10 + Math.floor(rand() * 17);
    return {
      x: index * cell + cell / 2,
      head: initial ? between(rand, -trail * cell, Math.max(height, 1)) : -between(rand, 0, height * 0.6) - cell,
      speed: between(rand, 55, 150),
      trail, glyphs: Array.from({ length: trail }, glyph), acid: rand() < 0.2, mutate: 0,
    };
  };
  return {
    id: "matrix",
    resize(width, h) {
      height = h;
      const count = Math.floor(width / cell);
      if (count < columns.length) columns.length = count;
      while (columns.length < count) columns.push(spawn(columns.length, true));
    },
    step(dt) {
      for (let i = 0; i < columns.length; i++) {
        const column = columns[i];
        column.head += column.speed * dt;
        column.mutate += dt;
        if (column.mutate > 0.12) { column.mutate = 0; column.glyphs[Math.floor(rand() * column.trail)] = glyph(); }
        if (column.head - column.trail * cell > height) columns[i] = spawn(i, false);
      }
    },
    draw(ctx) {
      ctx.font = `700 15px "Ubuntu Mono", "DejaVu Sans Mono", ui-monospace, monospace`;
      ctx.textAlign = "center"; ctx.textBaseline = "middle";
      for (const column of columns) {
        for (let k = 0; k < column.trail; k++) {
          const y = column.head - k * cell;
          if (y < -cell || y > height + cell) continue;
          const fade = Math.pow(1 - k / column.trail, 1.7);
          ctx.globalAlpha = k === 0 ? 1 : 0.62 * fade;
          ctx.fillStyle = k === 0 ? "#f1ffd8" : k < 3 ? (column.acid ? "#d4ff4a" : "#9bff7a") : column.acid ? "#a6ff00" : "#13ff4d";
          ctx.fillText(column.glyphs[k], column.x, y);
        }
      }
      ctx.globalAlpha = 1;
    },
    state: () => ({ cell, columns }),
  };
}

// ------------------------------------------------------------------ ion plasma field (Ion)
interface Particle { x: number; y: number; speed: number; trail: { x: number; y: number }[]; tone: number }

function plasmaScene(rand: Rand): Scene {
  let width = 0, height = 0, clock = 0;
  const particles: Particle[] = [];
  const target = () => clamp(Math.round((width * height) / 5200), 36, 200);
  const angle = (x: number, y: number, t: number) => (Math.sin(x * 0.004 + t * 0.15) + Math.cos(y * 0.005 - t * 0.11) + Math.sin((x + y) * 0.003 + t * 0.07)) * 1.25;
  const make = (): Particle => ({ x: rand() * width, y: rand() * height, speed: between(rand, 26, 62), trail: [], tone: rand() });
  return {
    id: "plasma",
    resize(w, h) {
      width = w; height = h;
      const count = target();
      if (particles.length > count) particles.length = count;
      while (particles.length < count) particles.push(make());
      for (const p of particles) { p.x = clamp(p.x, 0, width); p.y = clamp(p.y, 0, height); }
    },
    step(dt, time) {
      clock = time;
      for (const p of particles) {
        const a = angle(p.x, p.y, time);
        p.x += Math.cos(a) * p.speed * dt;
        p.y += Math.sin(a) * p.speed * dt;
        let wrapped = false;
        if (p.x < 0) { p.x += width; wrapped = true; } else if (p.x > width) { p.x -= width; wrapped = true; }
        if (p.y < 0) { p.y += height; wrapped = true; } else if (p.y > height) { p.y -= height; wrapped = true; }
        if (wrapped) p.trail.length = 0;
        p.trail.push({ x: p.x, y: p.y });
        if (p.trail.length > 30) p.trail.shift();
      }
    },
    draw(ctx, w, h) {
      const glow = (cx: number, cy: number, radius: number, color: string) => {
        const g = ctx.createRadialGradient(cx, cy, 0, cx, cy, radius);
        g.addColorStop(0, color); g.addColorStop(1, "rgba(0,0,0,0)");
        ctx.fillStyle = g; ctx.fillRect(0, 0, w, h);
      };
      glow(w * (0.25 + 0.1 * Math.sin(clock * 0.07)), h * (0.7 + 0.08 * Math.cos(clock * 0.05)), Math.max(w, h) * 0.55, "rgba(120, 90, 255, .24)");
      glow(w * (0.8 + 0.08 * Math.cos(clock * 0.06)), h * (0.25 + 0.1 * Math.sin(clock * 0.08)), Math.max(w, h) * 0.5, "rgba(40, 220, 255, .2)");
      ctx.lineWidth = 1.7;
      for (const p of particles) {
        const color = p.tone < 0.35 ? "170, 120, 255" : p.tone < 0.75 ? "60, 225, 255" : "170, 255, 250";
        const n = p.trail.length;
        for (let i = 1; i < n; i++) {
          ctx.globalAlpha = Math.pow(i / n, 1.4) * 0.85;
          ctx.strokeStyle = `rgb(${color})`;
          ctx.beginPath(); ctx.moveTo(p.trail[i - 1].x, p.trail[i - 1].y); ctx.lineTo(p.trail[i].x, p.trail[i].y); ctx.stroke();
        }
        ctx.globalAlpha = 0.18; ctx.fillStyle = `rgb(${color})`;
        ctx.beginPath(); ctx.arc(p.x, p.y, 5, 0, Math.PI * 2); ctx.fill();
        ctx.globalAlpha = 0.95;
        ctx.beginPath(); ctx.arc(p.x, p.y, 1.9, 0, Math.PI * 2); ctx.fill();
      }
      ctx.globalAlpha = 1;
    },
    state: () => ({ particles }),
  };
}

// ------------------------------------------------------------------ breach sparks and slices (Breach)
interface Spark { id: number; x: number; y: number; vx: number; vy: number; life: number; maxLife: number; size: number; wobble: number }
interface Slice { y: number; h: number; dx: number; age: number; life: number }

function embersScene(rand: Rand): Scene {
  let width = 0, height = 0, nextId = 1, untilSlice = 5;
  let carry = 0;
  const sparks: Spark[] = [];
  const slices: Slice[] = [];
  return {
    id: "embers",
    resize(w, h) { width = w; height = h; },
    step(dt) {
      carry += (width / 1000) * 26 * dt;
      while (carry >= 1 && sparks.length < 90) {
        carry -= 1;
        const maxLife = between(rand, 2.5, 5);
        sparks.push({ id: nextId++, x: rand() * width, y: height + 4, vx: between(rand, -22, 22), vy: between(rand, 60, 220), life: maxLife, maxLife, size: between(rand, 1.4, 3.4), wobble: rand() * 6.28 });
      }
      carry = Math.min(carry, 1);
      for (let i = sparks.length - 1; i >= 0; i--) {
        const s = sparks[i];
        s.life -= dt; s.y -= s.vy * dt; s.wobble += dt * 2.2; s.x += (s.vx + Math.sin(s.wobble) * 14) * dt;
        if (s.life <= 0 || s.y < -10) sparks.splice(i, 1);
      }
      untilSlice -= dt;
      if (untilSlice <= 0 && slices.length < 3) {
        untilSlice = between(rand, 5, 9);
        slices.push({ y: rand() * Math.max(height - 30, 1), h: between(rand, 6, 28), dx: between(rand, 10, 40) * (rand() < 0.5 ? -1 : 1), age: 0, life: between(rand, 0.35, 0.7) });
      }
      for (let i = slices.length - 1; i >= 0; i--) { slices[i].age += dt; if (slices[i].age >= slices[i].life) slices.splice(i, 1); }
    },
    draw(ctx, w) {
      for (const s of sparks) {
        const k = clamp(s.life / s.maxLife, 0, 1);
        ctx.globalAlpha = Math.min(1, k * 1.6);
        ctx.fillStyle = k > 0.66 ? "#ffd27a" : k > 0.33 ? "#ff8a3a" : "#ff2d4a";
        ctx.beginPath(); ctx.arc(s.x, s.y, s.size * (0.6 + k * 0.6), 0, Math.PI * 2); ctx.fill();
      }
      for (const slice of slices) {
        const envelope = Math.sin(Math.PI * (slice.age / slice.life));
        ctx.globalAlpha = 0.16 * envelope; ctx.fillStyle = "#ff2d4a"; ctx.fillRect(slice.dx, slice.y, w, slice.h);
        ctx.globalAlpha = 0.1 * envelope; ctx.fillStyle = "#3ce6ff"; ctx.fillRect(-slice.dx, slice.y + slice.h * 0.45, w, Math.max(2, slice.h * 0.2));
      }
      ctx.globalAlpha = 1;
    },
    state: () => ({ sparks, slices }),
  };
}

// ------------------------------------------------------------------ projector (Cinema)
// An old reel projector in the top-right corner: two turning reels, film marching through the gate and a
// lens that sets the origin of a flickering, volumetric beam with dust drifting through it. On top of
// that: an anamorphic lens streak, bokeh, old-film scratches, live film grain, slow exposure breathing
// and a warm spotlight that follows the cursor. The lamp warms up over the first moments so nothing pops on.
interface Mote { x: number; y: number; vx: number; vy: number; r: number; phase: number; depth: number }
interface Reel { cx: number; cy: number; r: number; angle: number; speed: number }
interface Scratch { x: number; w: number; age: number; life: number }
const TAU = Math.PI * 2;

function projectorScene(rand: Rand): Scene {
  let width = 0, height = 0, clock = 0, age = 0, warm = 0, grainSeed = 0, untilScratch = 3;
  const spot = { x: 0, y: 0, alpha: 0 };
  let target: { x: number; y: number } | null = null;
  const motes: Mote[] = [];
  const scratches: Scratch[] = [];
  const reels: Reel[] = [{ cx: 0, cy: 0, r: 0, angle: 0, speed: 0.8 }, { cx: 0, cy: 0, r: 0, angle: 1.1, speed: 0.8 }];
  const lens = { x: 0, y: 0 };
  const body = { x: 0, y: 0, w: 0, h: 0 };
  const film = { offset: 0, period: 16 };
  let unit = 1;
  const count = () => clamp(Math.round((width * height) / 11000), 40, 110);
  const make = (): Mote => {
    const depth = rand();
    return { x: rand() * width, y: rand() * height, vx: between(rand, -5, 5) * (0.4 + depth), vy: -between(rand, 1, 9) * (0.4 + depth * 1.1), r: 0.5 + depth * 2.2, phase: rand() * 6.28, depth };
  };
  // The machine stands in the bottom-right corner with its lens barrel on the LEFT end, facing the room: the beam
  // leaves the lens toward the upper left, the way the lens points.
  const layout = () => {
    unit = clamp(width / 1300, 0.75, 1.25);
    body.w = 250 * unit; body.h = 68 * unit; body.x = width - 48 * unit - body.w; body.y = height - 150 * unit;
    lens.x = body.x - 34 * unit; lens.y = body.y + 30 * unit;
    reels[0].cx = body.x + 70 * unit; reels[1].cx = body.x + 190 * unit;
    for (const reel of reels) { reel.cy = body.y - 74 * unit; reel.r = 52 * unit; }
  };
  // The beam leaves the lens and widens toward the upper left.
  const inBeam = (x: number, y: number) => {
    const mirrored = height - y;
    const apex = height - lens.y;
    const t = (lens.x - x) / Math.max(lens.x + width * 0.05, 1);
    const centre = apex + t * (height * 0.72 - apex);
    return t >= 0 && Math.abs(mirrored - centre) < 24 + t * height * 0.34;
  };
  const metal = (ctx: DrawContext, x0: number, y0: number, x1: number, y1: number, light: string, dark: string) => {
    const g = ctx.createLinearGradient(x0, y0, x1, y1); g.addColorStop(0, light); g.addColorStop(1, dark); return g;
  };
  const drawReel = (ctx: DrawContext, reel: Reel, lamp: number) => {
    const fade = Math.min(1, lamp + 0.15);
    ctx.save(); ctx.translate(reel.cx, reel.cy); ctx.rotate(reel.angle);
    // The reel body: a lit metal disc, with film wound in concentric turns near the rim.
    const disc = ctx.createRadialGradient(-reel.r * 0.3, -reel.r * 0.35, reel.r * 0.1, 0, 0, reel.r);
    disc.addColorStop(0, "#4a2229"); disc.addColorStop(0.7, "#22090f"); disc.addColorStop(1, "#12050a");
    ctx.globalAlpha = 0.95 * fade; ctx.fillStyle = disc; ctx.beginPath(); ctx.arc(0, 0, reel.r, 0, TAU); ctx.fill();
    ctx.strokeStyle = "#0a0305"; ctx.lineWidth = reel.r * 0.05; ctx.globalAlpha = 0.9;
    for (const k of [0.66, 0.72, 0.78]) { ctx.beginPath(); ctx.arc(0, 0, reel.r * k, 0, TAU); ctx.stroke(); }
    ctx.strokeStyle = "#c9a45c"; ctx.lineWidth = 2.5 * unit; ctx.globalAlpha = 0.65 * fade;
    ctx.beginPath(); ctx.arc(0, 0, reel.r, 0, TAU); ctx.stroke();
    ctx.lineWidth = unit; ctx.globalAlpha = 0.3 * fade; ctx.beginPath(); ctx.arc(0, 0, reel.r * 0.58, 0, TAU); ctx.stroke();
    // Six cut-outs with a fine rim, and the spokes between them.
    for (let k = 0; k < 6; k++) {
      const a = (k * TAU) / 6; const hx = Math.cos(a) * reel.r * 0.4, hy = Math.sin(a) * reel.r * 0.4;
      ctx.fillStyle = "#040102"; ctx.globalAlpha = 0.97; ctx.beginPath(); ctx.arc(hx, hy, reel.r * 0.16, 0, TAU); ctx.fill();
      ctx.strokeStyle = "#c9a45c"; ctx.lineWidth = unit; ctx.globalAlpha = 0.4 * fade; ctx.beginPath(); ctx.arc(hx, hy, reel.r * 0.16, 0, TAU); ctx.stroke();
    }
    const hub = ctx.createRadialGradient(-reel.r * 0.03, -reel.r * 0.04, 0, 0, 0, reel.r * 0.13);
    hub.addColorStop(0, "#f2d98a"); hub.addColorStop(1, "#8f6e2c");
    ctx.fillStyle = hub; ctx.globalAlpha = 0.95 * fade; ctx.beginPath(); ctx.arc(0, 0, reel.r * 0.13, 0, TAU); ctx.fill();
    ctx.restore();
    // A fixed highlight on the rim (it does not turn with the reel).
    ctx.save(); ctx.translate(reel.cx, reel.cy); ctx.strokeStyle = "#ffe9c2"; ctx.lineWidth = 3 * unit; ctx.globalAlpha = 0.22 * fade;
    ctx.beginPath(); ctx.arc(0, 0, reel.r * 0.93, TAU * 0.6, TAU * 0.78); ctx.stroke(); ctx.restore();
  };
  const drawProjector = (ctx: DrawContext, lamp: number) => {
    const bx = body.x, by = body.y, bw = body.w, bh = body.h;
    const fade = Math.min(1, lamp + 0.15);
    // A soft contact shadow under the machine.
    ctx.save(); ctx.translate(bx + bw / 2, by + bh + 18 * unit); ctx.scale(1, 0.1);
    const floor = ctx.createRadialGradient(0, 0, 0, 0, 0, bw * 0.66);
    floor.addColorStop(0, "rgba(0, 0, 0, 0.6)"); floor.addColorStop(1, "rgba(0, 0, 0, 0)");
    ctx.fillStyle = floor; ctx.beginPath(); ctx.arc(0, 0, bw * 0.66, 0, TAU); ctx.fill(); ctx.restore();
    // Film strands between the reels and the gate (which is near the lens), sprocket holes marching along them.
    ctx.strokeStyle = "#e6d3a8"; ctx.lineWidth = 6 * unit; ctx.lineCap = "butt"; ctx.globalAlpha = 0.34 * fade;
    ctx.setLineDash([6 * unit * 0.9, 10 * unit * 0.9]); ctx.lineDashOffset = -film.offset * unit * 0.9;
    ctx.beginPath(); ctx.moveTo(reels[1].cx - reels[1].r * 0.3, reels[1].cy + reels[1].r * 0.95); ctx.lineTo(bx + 214 * unit, by); ctx.stroke();
    ctx.beginPath(); ctx.moveTo(bx + 100 * unit, by); ctx.lineTo(reels[0].cx + reels[0].r * 0.3, reels[0].cy + reels[0].r * 0.95); ctx.stroke();
    ctx.setLineDash([]);
    drawReel(ctx, reels[0], lamp); drawReel(ctx, reels[1], lamp);
    // Feet, body (lit metal with a gold rim along the top), vents, gate and the lens barrel on the left end.
    ctx.globalAlpha = 0.9 * fade; ctx.fillStyle = "#0c0407";
    ctx.fillRect(bx + 30 * unit, by + bh, 22 * unit, 9 * unit); ctx.fillRect(bx + bw - 60 * unit, by + bh, 22 * unit, 9 * unit);
    ctx.globalAlpha = 0.96 * fade; ctx.fillStyle = metal(ctx, 0, by, 0, by + bh, "#33141c", "#12060a");
    ctx.beginPath(); ctx.moveTo(bx, by); ctx.lineTo(bx + bw - 8 * unit, by); ctx.lineTo(bx + bw, by + 8 * unit); ctx.lineTo(bx + bw, by + bh); ctx.lineTo(bx, by + bh); ctx.closePath(); ctx.fill();
    ctx.strokeStyle = "#e8cf8c"; ctx.lineWidth = 1.5 * unit; ctx.globalAlpha = 0.55 * fade;
    ctx.beginPath(); ctx.moveTo(bx, by); ctx.lineTo(bx + bw - 8 * unit, by); ctx.lineTo(bx + bw, by + 8 * unit); ctx.stroke();
    ctx.strokeStyle = "#c9a45c"; ctx.lineWidth = unit; ctx.globalAlpha = 0.28 * fade; ctx.strokeRect(bx, by, bw, bh);
    ctx.strokeStyle = "#050203"; ctx.lineWidth = 2 * unit; ctx.globalAlpha = 0.7 * fade;
    for (let k = 0; k < 5; k++) { ctx.beginPath(); ctx.moveTo(bx + (80 - k * 11) * unit, by + 18 * unit); ctx.lineTo(bx + (80 - k * 11) * unit, by + 50 * unit); ctx.stroke(); }
    ctx.globalAlpha = 0.94 * fade; ctx.fillStyle = "#070203"; ctx.fillRect(bx + 114 * unit, by + 12 * unit, 98 * unit, bh - 24 * unit);
    ctx.globalAlpha = 0.5 * fade; ctx.strokeStyle = "#c9a45c"; ctx.lineWidth = unit; ctx.strokeRect(bx + 114 * unit, by + 12 * unit, 98 * unit, bh - 24 * unit);
    const barrelX = bx - 34 * unit;
    ctx.globalAlpha = 0.97 * fade; ctx.fillStyle = metal(ctx, 0, lens.y - 20 * unit, 0, lens.y + 20 * unit, "#40202a", "#14070b");
    ctx.fillRect(barrelX, lens.y - 20 * unit, 34 * unit, 40 * unit);
    ctx.strokeStyle = "#e8cf8c"; ctx.lineWidth = unit; ctx.globalAlpha = 0.5 * fade;
    ctx.beginPath(); ctx.moveTo(barrelX + 11 * unit, lens.y - 20 * unit); ctx.lineTo(barrelX + 11 * unit, lens.y + 20 * unit); ctx.moveTo(barrelX + 22 * unit, lens.y - 20 * unit); ctx.lineTo(barrelX + 22 * unit, lens.y + 20 * unit); ctx.stroke();
    ctx.strokeRect(barrelX, lens.y - 20 * unit, 34 * unit, 40 * unit);
    const glass = ctx.createRadialGradient(lens.x, lens.y, 0, lens.x, lens.y, 22 * unit);
    glass.addColorStop(0, `rgba(255, 244, 214, ${0.95 * lamp})`); glass.addColorStop(1, "rgba(255, 214, 150, 0)");
    ctx.globalAlpha = 1; ctx.fillStyle = glass; ctx.beginPath(); ctx.arc(lens.x, lens.y, 22 * unit, 0, TAU); ctx.fill();
    ctx.globalAlpha = 1;
  };
  return {
    id: "projector",
    resize(w, h) {
      width = w; height = h; layout();
      const n = count();
      if (motes.length > n) motes.length = n;
      while (motes.length < n) motes.push(make());
      for (const m of motes) { m.x = clamp(m.x, 0, width); m.y = clamp(m.y, 0, height); }
    },
    pointer(x, y) { target = x === null || y === null ? null : { x, y }; },
    step(dt, time) {
      clock = time; age += dt; grainSeed = (grainSeed + 1) % 1000000;
      warm = 1 - Math.exp(-age / 1.1);
      for (const reel of reels) reel.angle = (reel.angle + reel.speed * dt) % TAU;
      film.offset = (film.offset + 38 * dt) % film.period;
      for (const m of motes) {
        m.x += (m.vx + Math.sin(time * 0.35 + m.phase) * (2 + m.depth * 5)) * dt;
        m.y += (m.vy + Math.cos(time * 0.27 + m.phase) * 2) * dt;
        if (m.x < 0) m.x += width; else if (m.x > width) m.x -= width;
        if (m.y < 0) m.y += height; else if (m.y > height) m.y -= height;
      }
      untilScratch -= dt;
      if (untilScratch <= 0 && scratches.length < 3) { untilScratch = between(rand, 3, 6); scratches.push({ x: rand() * width, w: between(rand, 0.8, 1.6), age: 0, life: between(rand, 0.5, 1.1) }); }
      for (let i = scratches.length - 1; i >= 0; i--) { scratches[i].age += dt; if (scratches[i].age >= scratches[i].life) scratches.splice(i, 1); }
      if (target) {
        if (spot.alpha < 0.02) { spot.x = target.x; spot.y = target.y; }
        const follow = 1 - Math.exp(-dt * 5);
        spot.x += (target.x - spot.x) * follow; spot.y += (target.y - spot.y) * follow;
      }
      spot.alpha += ((target ? 1 : 0) - spot.alpha) * (1 - Math.exp(-dt * 3));
    },
    draw(ctx, w, h) {
      const lamp = flicker(clock) * exposure(clock) * warm;
      // Volumetric beam from the lens, drawn in a vertically mirrored space so it sweeps up and to the left:
      // twelve nested cones, each faint, so the edges dissolve into a soft shaft.
      ctx.save(); ctx.translate(0, h); ctx.scale(1, -1);
      const apexY = h - lens.y;
      for (let i = 0; i < 12; i++) {
        const topEdge = h * (0.3 + i * 0.045), bottomEdge = h * (0.86 + i * 0.022);
        const shimmer = 0.92 + 0.08 * Math.sin(clock * 0.7 + i * 1.3);
        const a = (0.04 - i * 0.0021) * lamp * shimmer;
        const g = ctx.createLinearGradient(lens.x, apexY, -w * 0.05, h);
        g.addColorStop(0, `rgba(255, 232, 186, ${a * 2.4})`); g.addColorStop(0.45, `rgba(255, 214, 150, ${a})`); g.addColorStop(1, "rgba(255, 204, 140, 0)");
        ctx.fillStyle = g;
        ctx.beginPath(); ctx.moveTo(lens.x, apexY - 6 * unit); ctx.lineTo(-w * 0.05, bottomEdge); ctx.lineTo(-w * 0.05, topEdge); ctx.lineTo(lens.x, apexY + 6 * unit); ctx.closePath(); ctx.fill();
      }
      ctx.restore();
      // The lamp glow and its anamorphic streak at the lens.
      const core = ctx.createRadialGradient(lens.x, lens.y, 0, lens.x, lens.y, 150 * unit);
      core.addColorStop(0, `rgba(255, 242, 210, ${0.5 * lamp})`); core.addColorStop(1, "rgba(255, 220, 160, 0)");
      ctx.fillStyle = core; ctx.fillRect(Math.max(0, lens.x - 160 * unit), Math.max(0, lens.y - 160 * unit), 320 * unit, 320 * unit);
      ctx.save(); ctx.translate(lens.x, lens.y); ctx.scale(1, 0.014);
      const streak = ctx.createRadialGradient(0, 0, 0, 0, 0, w * 0.55);
      streak.addColorStop(0, `rgba(170, 205, 255, ${0.5 * lamp})`); streak.addColorStop(0.5, `rgba(150, 190, 255, ${0.14 * lamp})`); streak.addColorStop(1, "rgba(150, 190, 255, 0)");
      ctx.fillStyle = streak; ctx.beginPath(); ctx.arc(0, 0, w * 0.55, 0, TAU); ctx.fill(); ctx.restore();
      drawProjector(ctx, lamp);
      // Soft bokeh drifting in the far corner.
      for (let i = 0; i < 6; i++) {
        const bx = w * (0.08 + i * 0.07) + Math.sin(clock * 0.09 + i) * 18, by = h * (0.82 - (i % 3) * 0.07) + Math.cos(clock * 0.07 + i) * 12, br = 36 + (i * 17) % 70;
        const orb = ctx.createRadialGradient(bx, by, br * 0.2, bx, by, br);
        orb.addColorStop(0, `rgba(255, 214, 150, ${0.05 * lamp})`); orb.addColorStop(1, "rgba(255, 214, 150, 0)");
        ctx.fillStyle = orb; ctx.beginPath(); ctx.arc(bx, by, br, 0, TAU); ctx.fill();
      }
      // Dust in three depth layers: far specks are crisp and dim, near ones are large, soft and bright in the beam.
      for (const m of motes) {
        const twinkle = 0.6 + 0.4 * Math.sin(clock * 1.1 + m.phase);
        const lit = inBeam(m.x, m.y) ? 1 : 0.22;
        const alpha = (0.08 + m.depth * 0.55) * lit * twinkle * lamp;
        if (m.depth > 0.66) {
          const bokeh = ctx.createRadialGradient(m.x, m.y, 0, m.x, m.y, m.r * 3.4);
          bokeh.addColorStop(0, `rgba(255, 232, 190, ${alpha * 0.55})`); bokeh.addColorStop(1, "rgba(255, 232, 190, 0)");
          ctx.fillStyle = bokeh; ctx.beginPath(); ctx.arc(m.x, m.y, m.r * 3.4, 0, TAU); ctx.fill();
        } else {
          ctx.globalAlpha = alpha; ctx.fillStyle = "#ffe6bd";
          ctx.beginPath(); ctx.arc(m.x, m.y, m.r, 0, TAU); ctx.fill(); ctx.globalAlpha = 1;
        }
      }
      // The cursor's warm spotlight.
      if (spot.alpha > 0.01) {
        const light = ctx.createRadialGradient(spot.x, spot.y, 0, spot.x, spot.y, 340);
        light.addColorStop(0, `rgba(255, 238, 206, ${0.16 * spot.alpha * warm})`); light.addColorStop(1, "rgba(255, 220, 170, 0)");
        ctx.fillStyle = light; ctx.fillRect(0, 0, w, h);
      }
      // Old-film scratches fade in and out; live grain is redrawn every frame from the frame counter.
      ctx.fillStyle = "#fff2dc";
      for (const scratch of scratches) { ctx.globalAlpha = 0.18 * Math.sin(Math.PI * (scratch.age / scratch.life)) * warm; ctx.fillRect(scratch.x, 0, scratch.w, h); }
      const grain = mulberry32Local(grainSeed * 7919 + 13);
      const specks = clamp(Math.round((w * h) / 2600), 160, 700);
      for (let i = 0; i < specks; i++) { ctx.globalAlpha = 0.05 + grain() * 0.1; ctx.fillRect(grain() * w, grain() * h, 1.2, 1.2); }
      ctx.globalAlpha = 1;
    },
    state: () => ({ motes, warm, grainSeed, spot, lens, body, reels, film, scratches }),
  };
}

function mulberry32Local(seed: number): Rand {
  let state = seed >>> 0;
  return () => {
    state = (state + 0x6d2b79f5) >>> 0;
    let t = state;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

export function createScene(id: SceneId, rand: Rand): Scene {
  if (id === "matrix") return matrixScene(rand);
  if (id === "plasma") return plasmaScene(rand);
  if (id === "embers") return embersScene(rand);
  return projectorScene(rand);
}
