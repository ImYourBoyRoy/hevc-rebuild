// ./desktop/src/lib/app-input.ts
// Global keyboard shortcuts, the native agent-control channel (key presses, text snapshots, replies)
// and the native context menus (editing menu, utility menu, clipboard copies), extracted from the
// route (boot slice, input module). The route's state is reached through a typed `AppInputHost`; the
// DOM, window, native menu API and the route's actions are injected so every branch is unit-tested
// in node (pnpm test:app-input). The code is the route's code moved verbatim with `h.` prefixes.

import type { Menu, MenuOptions } from '@tauri-apps/api/menu';
import type { Page, RunMode } from './types.js';
import type { AgentControlRequest } from './app-boot.ts';

export interface AppInputHost {
  page: Page;
  mode: RunMode;
  booting: boolean;
  scanning: boolean;
  workflowBusy: boolean;
  root: string;
  environmentLoading: boolean;
  selectedLog: string;
  mutatingRun: boolean;
  canStart: boolean;
  abortConfirmOpen: boolean;
  desktopShell: boolean;
  readonly runId: string;
  setNotice(message: string, kind?: "info" | "success" | "error"): void;
}

export interface AppInputActions {
  openPage(page: Page): void;
  stopAfterCurrent(): Promise<unknown> | unknown;
  beginRun(): Promise<unknown> | unknown;
  browseFolder(): Promise<unknown> | unknown;
  analyzeFolder(): Promise<unknown> | unknown;
  loadEnvironment(): Promise<unknown> | unknown;
  copyLog(): Promise<unknown> | unknown;
  tick(): Promise<void>;
}

export interface AppInputEnv {
  document: Document;
  window: Window;
  navigator: Pick<Navigator, "clipboard">;
  KeyboardEvent: typeof KeyboardEvent;
  Menu: { new: (options: MenuOptions) => Promise<Menu> };
  getCurrentWindow(): Parameters<Menu["popup"]>[1];
  invoke: <T>(command: string, args?: Record<string, unknown>) => Promise<T>;
}

export interface AppInputDeps {
  host: AppInputHost;
  actions: AppInputActions;
  env: AppInputEnv;
  pageLabel: (page: Page) => string;
}

export function createAppInput(deps: AppInputDeps) {
  const { host: h, actions, env, pageLabel } = deps;
  const { document, window, navigator, KeyboardEvent, Menu, getCurrentWindow, invoke } = env;
  const { openPage, stopAfterCurrent, beginRun, browseFolder, analyzeFolder, loadEnvironment, copyLog, tick } = actions;
  const setNotice = h.setNotice.bind(h);
  let contextMenu: Menu | null = null;

  function handleGlobalKeydown(event: KeyboardEvent) {
    if (event.defaultPrevented) return;
    const target = event.target;
    const typing = target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement || target instanceof HTMLSelectElement || (target instanceof HTMLElement && target.isContentEditable);
    if (event.key === "F8") {
      event.preventDefault();
      openPage(h.page === "help" ? "library" : "help");
      return;
    }
    if (typing && !(event.altKey || event.ctrlKey || event.metaKey)) return;
    if (event.altKey && !event.ctrlKey && !event.metaKey && !event.shiftKey) {
      const pages: Record<string, Page> = { "1": "library", "2": "run", "3": "youtube", "4": "settings", "5": "reports", "6": "help" };
      const next = pages[event.key];
      if (next) {
        event.preventDefault();
        openPage(next);
        return;
      }
      if (event.key.toLowerCase() === "m") {
        event.preventDefault();
        openPage("metadata");
        return;
      }
      if (event.key.toLowerCase() === "s" && (h.runId || h.scanning)) {
        event.preventDefault();
        void stopAfterCurrent();
        return;
      }
    }
    if (event.altKey && event.shiftKey && !event.ctrlKey && !event.metaKey && event.key.toLowerCase() === "s" && h.mutatingRun) {
      event.preventDefault();
      h.abortConfirmOpen = true;
      return;
    }
    if (event.ctrlKey && !event.altKey && !event.metaKey && event.key === "Enter" && h.canStart) {
      event.preventDefault();
      void beginRun();
    }
  }

  function dispatchAgentKey(specification: string) {
    const parts = specification.split("+").map((part) => part.trim()).filter(Boolean);
    const requestedKey = parts.pop() ?? specification;
    const key = requestedKey === "Space" ? " " : requestedKey;
    const event = new KeyboardEvent("keydown", {
      key,
      code: key.length === 1 ? `Key${key.toUpperCase()}` : key,
      altKey: parts.some((part) => part.toLowerCase() === "alt"),
      ctrlKey: parts.some((part) => part.toLowerCase() === "ctrl" || part.toLowerCase() === "control"),
      metaKey: parts.some((part) => part.toLowerCase() === "meta" || part.toLowerCase() === "command"),
      shiftKey: parts.some((part) => part.toLowerCase() === "shift"),
      bubbles: true,
      cancelable: true,
    });
    window.dispatchEvent(event);
  }

  function agentControlSnapshot() {
    return {
      page: h.page,
      page_label: pageLabel(h.page),
      booting: h.booting,
      scanning: h.scanning,
      workflow_busy: h.workflowBusy,
      run_id: h.runId,
      run_mode: h.mode,
      active_element: document.activeElement instanceof HTMLElement ? document.activeElement.outerHTML.slice(0, 2000) : null,
      title: document.title,
      text: document.body.innerText.slice(0, 30000),
    };
  }

  async function handleAgentControlRequest(request: AgentControlRequest) {
    let response: Record<string, unknown>;
    try {
      if (request.action === "press") {
        dispatchAgentKey(request.key ?? "");
        await tick();
        response = { handled: true, ...agentControlSnapshot() };
      } else if (request.action === "screenshot") {
        response = { supported: false, message: "Use the native desktop capture surface for screenshots; the control channel intentionally returns text snapshots only.", ...agentControlSnapshot() };
      } else {
        response = agentControlSnapshot();
      }
    } catch (error) {
      response = { error: String(error), ...agentControlSnapshot() };
    }
    try {
      await invoke("reply", { id: request.id, response });
    } catch (error) {
      console.error("Could not answer the native agent-control request", error);
    }
  }

  async function copyUtilityText(value: string, description: string) {
    try {
      await navigator.clipboard.writeText(value);
      h.setNotice(`${description} copied to the clipboard.`, "success");
    } catch (error) {
      h.setNotice(`Could not copy ${description.toLowerCase()}: ${String(error)}`, "error");
    }
  }

  function handleUtilityContextMenu(event: MouseEvent) {
    const target = event.target instanceof Element ? event.target : null;
    if (!target?.closest(".app-shell")) return;
    const editingField = Boolean(target.closest("input:not([type]), input[type='text'], input[type='search'], input[type='number'], input[type='email'], input[type='url'], input[type='tel'], input[type='password'], textarea, [contenteditable]:not([contenteditable='false']), [role='textbox']"));

    if (!h.desktopShell && editingField) return;
    event.preventDefault();
    if (!h.desktopShell) return;
    if (editingField) {
      void showNativeEditingMenu();
      return;
    }
    void showUtilityContextMenu(target);
  }

  async function showNativeContextMenu(items: NonNullable<MenuOptions["items"]>) {
    try {
      if (contextMenu) {
        await contextMenu.close();
        contextMenu = null;
      }
      const menu = await Menu.new({ items });
      contextMenu = menu;
      await menu.popup(undefined, getCurrentWindow());
    } catch (error) {
      if (contextMenu) {
        void contextMenu.close().catch(() => {});
        contextMenu = null;
      }
      h.setNotice(`Could not open the native context menu: ${String(error)}`, "error");
    }
  }

  async function showNativeEditingMenu() {
    const items: NonNullable<MenuOptions["items"]> = [
      { item: "Cut" },
      { item: "Copy" },
      { item: "Paste" },
      { item: "Separator" },
      { item: "SelectAll" },
    ];
    await showNativeContextMenu(items);
  }

  async function showUtilityContextMenu(target: Element) {
    const mediaPath = target.closest<HTMLElement>("[data-media-path]")?.dataset.mediaPath;
    const logViewer = Boolean(target.closest(".diagnostic-card pre"));
    const hasTextSelection = Boolean(window.getSelection()?.toString().trim());
    const items: NonNullable<MenuOptions["items"]> = [
      { id: "choose-folder", text: "Choose media folder", enabled: !h.workflowBusy, action: () => { void browseFolder(); } },
      { id: "analyze-library", text: "Add current path to queue", enabled: Boolean(h.root.trim()) && !h.workflowBusy, action: () => { void analyzeFolder(); } },
      { item: "Separator" },
      { id: "library", text: "Library", enabled: h.page !== "library" && h.page !== "metadata", action: () => { openPage("library"); } },
      { id: "metadata", text: "Organize titles", enabled: h.page !== "metadata", action: () => { openPage("metadata"); } },
      { id: "youtube", text: "YouTube", enabled: h.page !== "youtube", action: () => { openPage("youtube"); } },
      { id: "run", text: "Run", enabled: h.page !== "run", action: () => { openPage("run"); } },
      { id: "settings", text: "Settings", enabled: h.page !== "settings", action: () => { openPage("settings"); } },
      { id: "reports", text: "Reports & errors", enabled: h.page !== "reports", action: () => { openPage("reports"); } },
      { item: "Separator" },
      { id: "refresh-environment", text: "Refresh environment", enabled: !h.environmentLoading, action: () => { void loadEnvironment(); } },
    ];
    if (mediaPath) {
      items.push({ item: "Separator" });
      items.push({ id: "copy-media-path", text: "Copy media file path", action: () => { void copyUtilityText(mediaPath, "Media file path"); } });
    } else if (h.root.trim()) {
      items.push({ item: "Separator" });
      items.push({ id: "copy-library-path", text: "Copy library path", action: () => { void copyUtilityText(h.root, "Library path"); } });
    }
    if (logViewer && h.selectedLog) {
      items.push({ item: "Separator" });
      items.push({ id: "copy-diagnostic-log", text: "Copy diagnostic log", action: () => { void copyLog(); } });
    }
    if (hasTextSelection) items.push({ item: "Copy" });

    await showNativeContextMenu(items);
  }

  function closeContextMenu() {
    if (contextMenu) void contextMenu.close().catch(() => {});
  }

  return { handleGlobalKeydown, dispatchAgentKey, agentControlSnapshot, handleAgentControlRequest, copyUtilityText, handleUtilityContextMenu, showNativeContextMenu, showNativeEditingMenu, showUtilityContextMenu, closeContextMenu };
}
