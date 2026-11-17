<!-- ./desktop/src/lib/ThemeBackdrop.svelte
     The animated canvas behind the work area for the themed skins (matrix rain, ion plasma, breach
     sparks, cinema projector). Purely decorative (aria-hidden, pointer-events none). It sizes itself
     to the main panel, pauses while the window is hidden or the user prefers reduced motion, and fades
     in and out when the theme changes. The Cinema skin also gets a cursor spotlight and a gentle 3D lean
     on cards (backdrop/tilt.ts). The loop lives in backdrop/engine.ts (tested in node). -->
<script lang="ts">
  import { onMount } from "svelte";
  import { createBackdrop } from "./backdrop/engine.js";
  import { sceneIdFor } from "./backdrop/scenes.js";
  import { computeTilt } from "./backdrop/tilt.js";

  let { theme }: { theme: string } = $props();
  let canvas = $state<HTMLCanvasElement | null>(null);
  let engine = $state<ReturnType<typeof createBackdrop> | null>(null);
  let currentTheme = "";
  const shown = $derived(engine !== null && sceneIdFor(theme) !== null);

  /** Step and paint by hand (fixtures; the browser pane stops animation frames while hidden). */
  export function advance(seconds: number) { engine?.advance(seconds); }

  onMount(() => {
    if (!canvas) return;
    const element = canvas;
    const reduce = window.matchMedia("(prefers-reduced-motion: reduce)");
    const created = createBackdrop({
      canvas: element,
      requestFrame: (callback) => window.requestAnimationFrame(callback),
      cancelFrame: (id) => window.cancelAnimationFrame(id),
      now: () => performance.now(),
      prefersReducedMotion: () => reduce.matches,
    });
    const size = () => {
      const box = element.parentElement?.getBoundingClientRect();
      if (box) created.resize(box.width, box.height, window.devicePixelRatio);
    };
    const observer = new ResizeObserver(size);
    if (element.parentElement) observer.observe(element.parentElement);
    const syncPause = () => created.setPaused(document.hidden || reduce.matches);
    document.addEventListener("visibilitychange", syncPause);
    reduce.addEventListener("change", syncPause);
    // The pointer drives the spotlight; Cinema cards also lean toward it. Both are skipped under reduced motion.
    const panel = element.parentElement;
    let leaning: HTMLElement | null = null;
    const release = () => {
      if (!leaning) return;
      leaning.style.removeProperty("--tilt-x"); leaning.style.removeProperty("--tilt-y");
      leaning.style.removeProperty("--sheen-x"); leaning.style.removeProperty("--sheen-y");
      leaning.removeAttribute("data-leaning");
      leaning = null;
    };
    const move = (event: PointerEvent) => {
      if (reduce.matches || !panel) return;
      const box = panel.getBoundingClientRect();
      created.setPointer(event.clientX - box.left, event.clientY - box.top);
      if (currentTheme !== "cinema") return;
      const card = (event.target as Element | null)?.closest?.<HTMLElement>(".library-poster-card, .home-action") ?? null;
      if (card !== leaning) release();
      if (!card) return;
      const tilt = computeTilt(card.getBoundingClientRect(), event.clientX, event.clientY, 6);
      leaning = card;
      card.setAttribute("data-leaning", "");
      card.style.setProperty("--tilt-x", `${tilt.rx.toFixed(2)}deg`); card.style.setProperty("--tilt-y", `${tilt.ry.toFixed(2)}deg`);
      card.style.setProperty("--sheen-x", `${tilt.px.toFixed(1)}%`); card.style.setProperty("--sheen-y", `${tilt.py.toFixed(1)}%`);
    };
    const leave = () => { created.setPointer(null, null); release(); };
    panel?.addEventListener("pointermove", move);
    panel?.addEventListener("pointerleave", leave);
    size();
    engine = created;
    return () => {
      observer.disconnect();
      document.removeEventListener("visibilitychange", syncPause);
      reduce.removeEventListener("change", syncPause);
      panel?.removeEventListener("pointermove", move);
      panel?.removeEventListener("pointerleave", leave);
      release();
      created.destroy();
    };
  });

  $effect(() => {
    currentTheme = theme;
    engine?.setTheme(theme);
  });
</script>

<canvas bind:this={canvas} class="theme-backdrop" class:shown aria-hidden="true"></canvas>

<style>
  .theme-backdrop {
    position: absolute;
    inset: 0;
    z-index: -1;
    width: 100%;
    height: 100%;
    pointer-events: none;
    opacity: 0;
    transition: opacity 900ms cubic-bezier(.4, 0, .2, 1);
  }
  .theme-backdrop.shown { opacity: var(--backdrop-opacity, .4); }
  @media (prefers-reduced-motion: reduce) { .theme-backdrop { transition: none; } }
</style>
