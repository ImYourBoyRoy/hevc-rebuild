// ./desktop/src/lib/backdrop/tilt.ts
// Pure maths for the cinema skin's card lean: how far a card tips toward the pointer and where the
// light sheen sits on it. Bounded and never NaN, so a fast pointer or an empty box cannot break the
// card. Tested in node (pnpm test:backdrop).

export interface TiltRect { left: number; top: number; width: number; height: number }
export interface Tilt { rx: number; ry: number; px: number; py: number }

const clamp = (value: number, min: number, max: number) => Math.min(max, Math.max(min, value));

/** `rx`/`ry` are degrees (bounded by `max`); `px`/`py` are the pointer position in percent of the box. */
export function computeTilt(rect: TiltRect, clientX: number, clientY: number, max = 7): Tilt {
  if (!(rect.width > 0) || !(rect.height > 0)) return { rx: 0, ry: 0, px: 50, py: 50 };
  const px = clamp(((clientX - rect.left) / rect.width) * 100, 0, 100);
  const py = clamp(((clientY - rect.top) / rect.height) * 100, 0, 100);
  return { rx: -((py - 50) / 50) * max + 0, ry: ((px - 50) / 50) * max + 0, px, py };
}
