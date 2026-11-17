// ./desktop/src/lib/theme-transition.ts
// Runs a theme switch as a smooth cross-fade instead of a hard cut. With the View Transitions API
// (Chromium-based WebView2, recent WebKit) the whole page cross-fades, gradients and shadows
// included; without it a short CSS window (`html.theme-fade`) transitions colours, borders and
// shadows. Under reduced motion the switch is instant. Pure: the document, timers, reduced-motion
// query and the "wait for the DOM to settle" hook are all injected, so every branch is unit-tested in
// node (pnpm test:theme-transition). The timing and easing live in styles/themes/theme-motion.css.

type ClassList = { add(name: string): void; remove(name: string): void };

export interface ThemeTransitionDocument {
  documentElement: { classList: ClassList };
  startViewTransition?: (update: () => Promise<void> | void) => { finished: Promise<unknown>; skipTransition?: () => void };
}

export interface ThemeTransitionDeps {
  document: ThemeTransitionDocument;
  prefersReducedMotion: () => boolean;
  /** Resolves once the framework has flushed the DOM changes made by the update (Svelte `tick`). */
  settle?: () => Promise<void>;
  setTimeout: (callback: () => void, ms: number) => number;
  clearTimeout: (id: number) => void;
  /** How long the CSS fallback keeps `theme-fade` on the root. Must cover the CSS duration. */
  fallbackMs?: number;
  /** If a view transition has not finished by then (hidden or minimized window), apply the theme anyway. */
  guardMs?: number;
}

export function createThemeTransition(deps: ThemeTransitionDeps) {
  const { document, prefersReducedMotion, setTimeout, clearTimeout } = deps;
  const settle = deps.settle ?? (async () => {});
  const fallbackMs = deps.fallbackMs ?? 700;
  const guardMs = deps.guardMs ?? 1800;
  const root = document.documentElement;
  let fadeTimer: number | null = null;

  function fade(update: () => void) {
    if (fadeTimer !== null) clearTimeout(fadeTimer);
    root.classList.add("theme-fade");
    update();
    fadeTimer = setTimeout(() => {
      root.classList.remove("theme-fade");
      fadeTimer = null;
    }, fallbackMs);
  }

  return function runThemeTransition(update: () => void): void {
    if (prefersReducedMotion()) {
      update();
      return;
    }
    if (typeof document.startViewTransition !== "function") {
      fade(update);
      return;
    }
    let applied = false;
    let guard: number | null = null;
    const apply = () => {
      if (applied) return;
      applied = true;
      update();
    };
    const cleanup = () => {
      root.classList.remove("theme-vt");
      if (guard !== null) clearTimeout(guard);
      guard = null;
    };
    root.classList.add("theme-vt");
    try {
      const transition = document.startViewTransition(async () => {
        apply();
        await settle();
      });
      // A window that is hidden or minimized never renders a frame, so the transition can stall.
      guard = setTimeout(() => {
        guard = null;
        apply();
        cleanup();
        transition.skipTransition?.();
      }, guardMs);
      transition.finished.then(cleanup, cleanup);
    } catch {
      cleanup();
      fade(apply);
    }
  };
}
