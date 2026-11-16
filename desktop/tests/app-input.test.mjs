// ./desktop/tests/app-input.test.mjs
// Characterization tests for the input handlers extracted from the route (boot slice, input module):
// global keyboard shortcuts, the native agent-control channel (key presses, snapshots, replies) and
// the native context menus (editing menu, utility menu items, clipboard copies). The DOM, window,
// native menu API and the route's actions are injected fakes. Run: pnpm test:app-input

import assert from "node:assert/strict";
import test from "node:test";
import { createAppInput } from "../src/lib/app-input.ts";

globalThis.Element ??= class Element {};
class FakeHTMLElement extends globalThis.Element { constructor(props = {}) { super(); Object.assign(this, props); } }
class FakeInput extends FakeHTMLElement {}
class FakeTextArea extends FakeHTMLElement {}
class FakeSelect extends FakeHTMLElement {}
globalThis.HTMLElement = FakeHTMLElement;
globalThis.HTMLInputElement = FakeInput;
globalThis.HTMLTextAreaElement = FakeTextArea;
globalThis.HTMLSelectElement = FakeSelect;

function harness({ hostOver = {}, menuFail = false, selection = "", clipboardFail = false, invokeFail = false } = {}) {
  const log = [];
  const calls = [];
  const dispatched = [];
  const invokes = [];
  const menus = [];
  const state = {
    page: "library", mode: "check", booting: false, scanning: false, workflowBusy: false, runId: "", root: "/lib", environmentLoading: false, selectedLog: "",
    mutatingRun: false, canStart: false, abortConfirmOpen: false, desktopShell: true, ...hostOver,
  };
  const host = state;
  host.setNotice = (message, kind = "info") => log.push([kind, message]);
  const actions = {
    openPage: (page) => calls.push(["openPage", page]), stopAfterCurrent: async () => { calls.push(["stopAfterCurrent"]); }, beginRun: async () => { calls.push(["beginRun"]); },
    browseFolder: async () => { calls.push(["browseFolder"]); }, analyzeFolder: async () => { calls.push(["analyzeFolder"]); }, loadEnvironment: async () => { calls.push(["loadEnvironment"]); },
    copyLog: async () => { calls.push(["copyLog"]); }, tick: async () => { calls.push(["tick"]); },
  };
  class FakeKeyboardEvent { constructor(type, init) { this.type = type; Object.assign(this, init); } }
  const env = {
    document: { activeElement: new FakeHTMLElement({ outerHTML: "<button>x</button>" }), title: "HEVC", body: { innerText: "page text" } },
    window: { dispatchEvent: (event) => dispatched.push(event), getSelection: () => ({ toString: () => selection }) },
    navigator: { clipboard: { writeText: async (value) => { if (clipboardFail) throw new Error("denied"); calls.push(["clipboard", value]); } } },
    KeyboardEvent: FakeKeyboardEvent,
    Menu: { new: async (options) => { if (menuFail) throw new Error("no menu"); const menu = { options, closed: 0, popup: async () => { calls.push(["popup"]); }, close: async function () { this.closed += 1; } }; menus.push(menu); return menu; } },
    getCurrentWindow: () => ({ label: "main" }),
    invoke: async (command, args) => { invokes.push({ command, args }); if (invokeFail) throw new Error("channel closed"); return null; },
  };
  const input = createAppInput({ host, actions, env, pageLabel: (page) => `Label ${page}` });
  return { input, host, log, calls, dispatched, invokes, menus };
}

const key = (over = {}) => {
  const event = { key: "", altKey: false, ctrlKey: false, metaKey: false, shiftKey: false, defaultPrevented: false, target: null, prevented: false, preventDefault() { this.prevented = true; }, ...over };
  return event;
};

test("F8 toggles between help and the library, even from a text field", () => {
  const h = harness({ hostOver: { page: "library" } });
  const press = key({ key: "F8" });
  h.input.handleGlobalKeydown(press);
  assert.deepEqual([press.prevented, h.calls[0]], [true, ["openPage", "help"]]);
  h.host.page = "help";
  h.input.handleGlobalKeydown(key({ key: "F8", target: new FakeInput() }));
  assert.deepEqual(h.calls[1], ["openPage", "library"]);
});

test("handled events are ignored and typing only passes shortcut modifiers", () => {
  const h = harness();
  h.input.handleGlobalKeydown(key({ key: "F8", defaultPrevented: true }));
  for (const target of [new FakeInput(), new FakeTextArea(), new FakeSelect(), new FakeHTMLElement({ isContentEditable: true })]) {
    const plain = key({ key: "1", target });
    h.input.handleGlobalKeydown(plain);
    assert.equal(plain.prevented, false);
  }
  assert.equal(h.calls.length, 0);
  const withAlt = key({ key: "2", altKey: true, target: new FakeInput() });
  h.input.handleGlobalKeydown(withAlt);
  assert.deepEqual([withAlt.prevented, h.calls[0]], [true, ["openPage", "run"]]);
});

test("Alt+number opens its page and Alt+M the metadata organizer", () => {
  const h = harness();
  for (const [digit, page] of [["1", "library"], ["2", "run"], ["3", "youtube"], ["4", "settings"], ["5", "reports"], ["6", "help"]]) h.input.handleGlobalKeydown(key({ key: digit, altKey: true }));
  assert.deepEqual(h.calls.map((call) => call[1]), ["library", "run", "youtube", "settings", "reports", "help"]);
  h.input.handleGlobalKeydown(key({ key: "M", altKey: true }));
  assert.deepEqual(h.calls.at(-1), ["openPage", "metadata"]);
  const unknown = key({ key: "9", altKey: true });
  h.input.handleGlobalKeydown(unknown);
  assert.equal(unknown.prevented, false);
  const withShift = key({ key: "1", altKey: true, shiftKey: true });
  h.input.handleGlobalKeydown(withShift);
  assert.equal(withShift.prevented, false);
});

test("Alt+S stops after the current file only while a run or scan is active", () => {
  const idle = harness();
  const none = key({ key: "s", altKey: true });
  idle.input.handleGlobalKeydown(none);
  assert.deepEqual([none.prevented, idle.calls.length], [false, 0]);
  const running = harness({ hostOver: { runId: "run-1" } });
  const stop = key({ key: "S", altKey: true });
  running.input.handleGlobalKeydown(stop);
  assert.deepEqual([stop.prevented, running.calls[0]], [true, ["stopAfterCurrent"]]);
  const scanning = harness({ hostOver: { scanning: true } });
  scanning.input.handleGlobalKeydown(key({ key: "s", altKey: true }));
  assert.deepEqual(scanning.calls[0], ["stopAfterCurrent"]);
});

test("Alt+Shift+S asks to confirm stop-now only for a mutating run", () => {
  const h = harness({ hostOver: { mutatingRun: true } });
  const press = key({ key: "S", altKey: true, shiftKey: true });
  h.input.handleGlobalKeydown(press);
  assert.deepEqual([press.prevented, h.host.abortConfirmOpen], [true, true]);
  const readOnly = harness({ hostOver: { mutatingRun: false } });
  readOnly.input.handleGlobalKeydown(key({ key: "S", altKey: true, shiftKey: true }));
  assert.equal(readOnly.host.abortConfirmOpen, false);
});

test("Ctrl+Enter starts the run only when it can start and no other modifier is held", () => {
  const ready = harness({ hostOver: { canStart: true } });
  const press = key({ key: "Enter", ctrlKey: true });
  ready.input.handleGlobalKeydown(press);
  assert.deepEqual([press.prevented, ready.calls[0]], [true, ["beginRun"]]);
  const blocked = harness({ hostOver: { canStart: false } });
  const refused = key({ key: "Enter", ctrlKey: true });
  blocked.input.handleGlobalKeydown(refused);
  assert.deepEqual([refused.prevented, blocked.calls.length], [false, 0]);
  const withAlt = harness({ hostOver: { canStart: true } });
  withAlt.input.handleGlobalKeydown(key({ key: "Enter", ctrlKey: true, altKey: true }));
  assert.equal(withAlt.calls.length, 0);
});

test("dispatchAgentKey builds a keydown with the named modifiers", () => {
  const h = harness();
  h.input.dispatchAgentKey("Alt+1");
  h.input.dispatchAgentKey("Ctrl+Shift+Enter");
  h.input.dispatchAgentKey("Command+k");
  h.input.dispatchAgentKey("Space");
  const [alt, ctrl, meta, space] = h.dispatched;
  assert.deepEqual([alt.type, alt.key, alt.code, alt.altKey, alt.ctrlKey, alt.bubbles, alt.cancelable], ["keydown", "1", "Key1", true, false, true, true]);
  assert.deepEqual([ctrl.key, ctrl.code, ctrl.ctrlKey, ctrl.shiftKey, ctrl.altKey], ["Enter", "Enter", true, true, false]);
  assert.deepEqual([meta.key, meta.code, meta.metaKey], ["k", "KeyK", true]);
  assert.deepEqual([space.key, space.code], [" ", "Key "]);
});

test("agentControlSnapshot describes the page, activity and visible text", () => {
  const h = harness({ hostOver: { page: "run", booting: true, scanning: true, workflowBusy: true, runId: "r1", mode: "full" } });
  const snapshot = h.input.agentControlSnapshot();
  assert.deepEqual(snapshot, { page: "run", page_label: "Label run", booting: true, scanning: true, workflow_busy: true, run_id: "r1", run_mode: "full", active_element: "<button>x</button>", title: "HEVC", text: "page text" });
});

test("handleAgentControlRequest presses keys, snapshots, refuses screenshots and replies", async () => {
  const h = harness();
  await h.input.handleAgentControlRequest({ id: "a", action: "press", key: "Alt+2" });
  assert.equal(h.dispatched.length, 1);
  assert.deepEqual(h.calls.map((call) => call[0]), ["tick"]);
  assert.deepEqual([h.invokes[0].command, h.invokes[0].args.id, h.invokes[0].args.response.handled, h.invokes[0].args.response.page], ["reply", "a", true, "library"]);
  await h.input.handleAgentControlRequest({ id: "b", action: "screenshot" });
  assert.equal(h.invokes[1].args.response.supported, false);
  assert.match(h.invokes[1].args.response.message, /text snapshots only/);
  await h.input.handleAgentControlRequest({ id: "c", action: "state" });
  assert.equal(h.invokes[2].args.response.run_mode, "check");
  await h.input.handleAgentControlRequest({ id: "d", action: "snapshot" });
  assert.equal("handled" in h.invokes[3].args.response, false);
});

test("handleAgentControlRequest survives a closed reply channel and tolerates a missing key", async () => {
  const h = harness({ invokeFail: true });
  const original = console.error;
  const errors = [];
  console.error = (...args) => errors.push(args[0]);
  try {
    await h.input.handleAgentControlRequest({ id: "x", action: "state" });
  } finally { console.error = original; }
  assert.equal(errors[0], "Could not answer the native agent-control request");
  const press = harness();
  await press.input.handleAgentControlRequest({ id: "e", action: "press", key: undefined });
  assert.equal(press.invokes[0].args.response.handled, true);
  assert.equal(press.dispatched[0].key, "");
});

const targetFor = (matches = {}) => Object.assign(new globalThis.Element(), { closest: (selector) => matches[selector] ?? (selector.includes("input") && matches.editing ? {} : null) });

test("copyUtilityText copies and confirms, or explains the failure", async () => {
  const h = harness();
  await h.input.copyUtilityText("/lib", "Library path");
  assert.deepEqual([h.calls.at(-1), h.log.at(-1)], [["clipboard", "/lib"], ["success", "Library path copied to the clipboard."]]);
  const failed = harness({ clipboardFail: true });
  await failed.input.copyUtilityText("/lib", "Library path");
  assert.deepEqual(failed.log, [["error", "Could not copy library path: Error: denied"]]);
});

test("handleUtilityContextMenu only acts inside the app shell and respects editing fields", async () => {
  const h = harness();
  const outside = { target: targetFor({}), prevented: false, preventDefault() { this.prevented = true; } };
  h.input.handleUtilityContextMenu(outside);
  assert.equal(outside.prevented, false);
  const shellOnly = { target: targetFor({ ".app-shell": {} }), prevented: false, preventDefault() { this.prevented = true; } };
  h.input.handleUtilityContextMenu(shellOnly);
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(shellOnly.prevented, true);
  assert.equal(h.menus.length, 1, "the utility menu opened on desktop");
  const preview = harness({ hostOver: { desktopShell: false } });
  const editing = { target: targetFor({ ".app-shell": {}, editing: true }), prevented: false, preventDefault() { this.prevented = true; } };
  preview.input.handleUtilityContextMenu(editing);
  assert.equal(editing.prevented, false);
  const previewPlain = { target: targetFor({ ".app-shell": {} }), prevented: false, preventDefault() { this.prevented = true; } };
  preview.input.handleUtilityContextMenu(previewPlain);
  assert.deepEqual([previewPlain.prevented, preview.menus.length], [true, 0]);
  const nativeEdit = harness();
  nativeEdit.input.handleUtilityContextMenu({ target: targetFor({ ".app-shell": {}, editing: true }), preventDefault() {} });
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(nativeEdit.menus[0].options.items.map((item) => item.item), ["Cut", "Copy", "Paste", "Separator", "SelectAll"]);
});

test("showUtilityContextMenu lists navigation with enabled state and runs the actions", async () => {
  const h = harness({ hostOver: { page: "run", root: "/lib", workflowBusy: false } });
  await h.input.showUtilityContextMenu(targetFor({}));
  const items = h.menus[0].options.items;
  const byId = Object.fromEntries(items.filter((item) => item.id).map((item) => [item.id, item]));
  assert.deepEqual(Object.keys(byId), ["choose-folder", "analyze-library", "library", "metadata", "youtube", "run", "settings", "reports", "refresh-environment", "copy-library-path"]);
  assert.deepEqual([byId.library.enabled, byId.metadata.enabled, byId.youtube.enabled, byId.run.enabled, byId.settings.enabled, byId.reports.enabled], [true, true, true, false, true, true]);
  byId["choose-folder"].action();
  byId["analyze-library"].action();
  byId.metadata.action();
  byId["refresh-environment"].action();
  byId["copy-library-path"].action();
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(h.calls.map((call) => call[0]).filter((name) => name !== "popup"), ["browseFolder", "analyzeFolder", "openPage", "loadEnvironment", "clipboard"]);
});

test("showUtilityContextMenu disables busy actions and adds media, log and selection items", async () => {
  const busy = harness({ hostOver: { page: "library", workflowBusy: true, environmentLoading: true, root: " " }, selection: "" });
  await busy.input.showUtilityContextMenu(targetFor({}));
  const byId = Object.fromEntries(busy.menus[0].options.items.filter((item) => item.id).map((item) => [item.id, item]));
  assert.deepEqual([byId["choose-folder"].enabled, byId["analyze-library"].enabled, byId["refresh-environment"].enabled, byId.library.enabled], [false, false, false, false]);
  assert.equal("copy-library-path" in byId, false);
  const rich = harness({ hostOver: { selectedLog: "log text", root: "/lib" }, selection: "chosen" });
  await rich.input.showUtilityContextMenu(targetFor({ "[data-media-path]": { dataset: { mediaPath: "/lib/a.mkv" } }, ".diagnostic-card pre": {} }));
  const items = rich.menus[0].options.items;
  assert.equal(items.some((item) => item.id === "copy-media-path"), true);
  assert.equal(items.some((item) => item.id === "copy-library-path"), false);
  assert.equal(items.some((item) => item.id === "copy-diagnostic-log"), true);
  assert.deepEqual(items.at(-1), { item: "Copy" });
  items.find((item) => item.id === "copy-diagnostic-log").action();
  items.find((item) => item.id === "copy-media-path").action();
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(rich.calls.filter((call) => call[0] === "copyLog" || call[0] === "clipboard"), [["copyLog"], ["clipboard", "/lib/a.mkv"]]);
});

test("showNativeContextMenu replaces an open menu and reports a failure", async () => {
  const h = harness();
  await h.input.showNativeContextMenu([{ item: "Copy" }]);
  await h.input.showNativeContextMenu([{ item: "Paste" }]);
  assert.deepEqual([h.menus.length, h.menus[0].closed, h.menus[1].closed], [2, 1, 0]);
  h.input.closeContextMenu();
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(h.menus[1].closed, 1);
  h.input.closeContextMenu();
  const failing = harness({ menuFail: true });
  await failing.input.showNativeContextMenu([{ item: "Copy" }]);
  assert.deepEqual(failing.log, [["error", "Could not open the native context menu: Error: no menu"]]);
});
