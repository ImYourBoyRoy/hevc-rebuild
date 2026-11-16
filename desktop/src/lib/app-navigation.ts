// ./desktop/src/lib/app-navigation.ts
// Page navigation extracted from the route (helper slice): which sidebar item is active, switching pages together
// with the YouTube run mode, jumping to the run progress details and opening Settings at a metadata provider
// credential field. DOM access is injected so every branch is unit-tested in node (pnpm test:app-navigation).
// The code is the route's code moved verbatim.

import type { Page, RunMode } from "./types.js";
import type { AppPreferences } from "./ui-contracts.js";


export interface AppNavigationHost {
  page: Page;
  mode: RunMode;
  appPreferences: AppPreferences;
  readonly runMode: RunMode;
}

export interface AppNavigationActions {
  persistPreferences(patch: Partial<AppPreferences>, message?: string): Promise<boolean>;
  tick(): Promise<void>;
}

export interface AppNavigationDeps {
  env: { getElementById(id: string): (HTMLElement | null) | { scrollIntoView(options?: ScrollIntoViewOptions): void; focus(options?: FocusOptions): void } | null; requestAnimationFrame(callback: FrameRequestCallback): number | void };
  host: AppNavigationHost;
  actions: AppNavigationActions;
}

export function createAppNavigation(deps: AppNavigationDeps) {
  const { host: h, actions, env } = deps;
  const { persistPreferences, tick } = actions;

  function navItemActive(id: Page): boolean {
    if (id === "library") return h.page === "library" || h.page === "metadata" || h.page === "home";
    return h.page === id;
  }

  function openPage(next: Page) {
    if (next === "home") next = "library";
    h.page = next;
    if (next === "youtube") h.mode = "youtube";
    else if (h.mode === "youtube") h.mode = "check";
    if (h.appPreferences.lastPage !== next) void persistPreferences({ lastPage: next });
  }

  function openProgressDetails() {
    openPage(h.runMode === "youtube" ? "youtube" : "run");
    env.requestAnimationFrame(() => env.requestAnimationFrame(() => {
      env.getElementById("run-activity")?.scrollIntoView({ behavior: "smooth", block: "start" });
    }));
  }

  async function openMetadataProviderSettings(provider?: "tmdb" | "tvdb") {
    openPage("settings");
    await tick();
    const target = provider ? env.getElementById(`${provider}-credential-settings`) : null;
    target?.scrollIntoView({ behavior: "smooth", block: "center" });
    const inputId = provider === "tmdb" ? "tmdb-api-token" : provider === "tvdb" ? "tvdb-project-key" : null;
    if (inputId) env.getElementById(inputId)?.focus({ preventScroll: true });
  }

  return { navItemActive, openPage, openProgressDetails, openMetadataProviderSettings };
}
