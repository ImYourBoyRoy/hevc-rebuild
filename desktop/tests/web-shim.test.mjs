// ./desktop/tests/web-shim.test.mjs
// Contract tests for the headless-server web shim (src-tauri/src/web_server/web_shim.js). The shim is
// evaluated against a fake window, fetch and EventSource, and driven through the real
// `@tauri-apps/api` and plugin modules the GUI imports, so a Tauri API change that the shim no longer
// satisfies fails here. Run: pnpm test:web-shim

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const shimSource = readFileSync(new URL("../src-tauri/src/web_server/web_shim.js", import.meta.url), "utf8");

const encoder = new TextEncoder();

/** A fetch response whose body is a stream the test can push chunks into, like a Server-Sent Events reply. */
function streamResponse() {
  const queue = [];
  let wake = null;
  let ended = false;
  const reader = {
    read() {
      return new Promise((resolve) => {
        const next = () => {
          if (queue.length) resolve({ done: false, value: queue.shift() });
          else if (ended) resolve({ done: true, value: undefined });
          else wake = next;
        };
        next();
      });
    },
  };
  return {
    ok: true,
    status: 200,
    body: { getReader: () => reader },
    push(text) { queue.push(encoder.encode(text)); const w = wake; wake = null; w?.(); },
    end() { ended = true; const w = wake; wake = null; w?.(); },
  };
}

function frame(event, payload, id) {
  return `${id ? `id: ${id}\n` : ""}data: ${JSON.stringify({ event, payload })}\n\n`;
}

const tick = () => new Promise((resolve) => setTimeout(resolve, 5));

function install({ routes = {}, hash = "", search = "", stored = null } = {}) {
  const calls = [];
  const streams = [];
  const fetchMock = async (url, init = {}) => {
    calls.push({ url, init });
    if (url === "/api/events") {
      const stream = streamResponse();
      stream.request = init;
      streams.push(stream);
      return stream;
    }
    const handler = routes[url];
    const reply = handler ? await handler(init) : { status: 404, body: { error: "no route" } };
    return {
      ok: reply.status >= 200 && reply.status < 300,
      status: reply.status,
      text: async () => (reply.body === undefined ? "" : JSON.stringify(reply.body)),
      json: async () => reply.body,
    };
  };
  const store = new Map(stored ? Object.entries(stored) : []);
  const history = { replaced: [], replaceState: (...args) => history.replaced.push(args) };
  const win = {
    innerWidth: 1400,
    innerHeight: 900,
    opened: [],
    location: { hash, search, pathname: "/library" },
    history,
    sessionStorage: { getItem: (k) => (store.has(k) ? store.get(k) : null), setItem: (k, v) => store.set(k, String(v)), removeItem: (k) => store.delete(k) },
    __HEVC_SSE_RETRY_MS__: 5,
  };
  win.open = (...args) => win.opened.push(args);
  globalThis.window = win;
  new Function("window", "fetch", shimSource)(win, fetchMock);
  return { win, calls, streams, store, history };
}

test("the shim presents itself as the Tauri runtime the GUI already detects", () => {
  const { win } = install();
  assert.equal(win.isTauri, true);
  assert.equal(typeof win.__TAURI_INTERNALS__.invoke, "function");
  assert.deepEqual(win.__TAURI_INTERNALS__.metadata.currentWindow, { label: "main" });
});

test("invoke posts camelCase arguments as JSON and resolves the result", async () => {
  const { calls } = install({ routes: { "/api/invoke/load_settings": () => ({ status: 200, body: { root: "/m" } }) } });
  const { invoke } = await import("@tauri-apps/api/core");
  const result = await invoke("load_settings", { root: "/m" });
  assert.deepEqual(result, { root: "/m" });
  assert.equal(calls[0].init.method, "POST");
  assert.equal(calls[0].init.headers["Content-Type"], "application/json");
  assert.deepEqual(JSON.parse(calls[0].init.body), { root: "/m" });
  assert.equal(calls[0].init.credentials, "omit");
});

test("command failures reject with the server's message string, as Tauri does", async () => {
  install({
    routes: {
      "/api/invoke/cancel_scan": () => ({ status: 422, body: { error: "There is no active scan." } }),
      "/api/invoke/start_run": () => ({ status: 501, body: { error: "not wired" } }),
      "/api/invoke/load_settings": () => ({ status: 401 }),
    },
  });
  const { invoke } = await import("@tauri-apps/api/core");
  await assert.rejects(() => invoke("cancel_scan"), (error) => error === "There is no active scan.");
  await assert.rejects(() => invoke("start_run", {}), (error) => error === "not wired");
  await assert.rejects(() => invoke("load_settings", { root: "/" }), (error) => /login link/.test(error));
});

test("command names are URL-encoded", async () => {
  const { calls } = install({ routes: { "/api/invoke/a%2Fb": () => ({ status: 200, body: null }) } });
  const { invoke } = await import("@tauri-apps/api/core");
  await invoke("a/b");
  assert.equal(calls[0].url, "/api/invoke/a%2Fb");
});

test("listen shares one authenticated event stream, delivers payloads, and unlisten stops delivery", async () => {
  const { streams } = install({ hash: "#token=abcdefabcdefabcdefabcdef" });
  const { listen } = await import("@tauri-apps/api/event");
  const lines = [];
  const finished = [];
  const stopLines = await listen("run-line", (event) => lines.push(event.payload));
  await listen("run-finished", (event) => finished.push(event.payload));
  await tick();
  assert.equal(streams.length, 1, "one stream serves every listener");
  assert.equal(streams[0].request.headers.Authorization, "Bearer abcdefabcdefabcdefabcdef", "the token goes in a header, not a cookie");
  streams[0].push(frame("run-line", { runId: "a", line: "frame=1" }, "7"));
  streams[0].push(frame("run-finished", { runId: "a", exitCode: 0 }, "8"));
  streams[0].push(frame("scan-progress", { current: 1 })); // nobody listens: ignored
  await tick();
  assert.deepEqual(lines, [{ runId: "a", line: "frame=1" }]);
  assert.deepEqual(finished, [{ runId: "a", exitCode: 0 }]);
  await stopLines();
  streams[0].push(frame("run-line", { runId: "a", line: "frame=2" }));
  await tick();
  assert.equal(lines.length, 1);
});

test("the event parser copes with keep-alives, split chunks and several frames in one chunk", async () => {
  const { streams } = install({ hash: "#token=abcdefabcdefabcdefabcdef" });
  const { listen } = await import("@tauri-apps/api/event");
  const seen = [];
  await listen("run-line", (event) => seen.push(event.payload.line));
  await tick();
  const first = frame("run-line", { line: "one" }, "1");
  streams[0].push(": keep-alive\n\n");
  streams[0].push(first.slice(0, 11));
  streams[0].push(first.slice(11) + frame("run-line", { line: "two" }, "2") + frame("run-line", { line: "three" }, "3"));
  await tick();
  assert.deepEqual(seen, ["one", "two", "three"]);
});

test("a dropped stream reconnects with the last event id so nothing is lost or repeated", async () => {
  const { streams } = install({ hash: "#token=abcdefabcdefabcdefabcdef" });
  const { listen } = await import("@tauri-apps/api/event");
  const seen = [];
  await listen("run-line", (event) => seen.push(event.payload.line));
  await tick();
  streams[0].push(frame("run-line", { line: "one" }, "41"));
  await tick();
  streams[0].end();
  await new Promise((resolve) => setTimeout(resolve, 40));
  assert.equal(streams.length, 2, "it reconnected");
  assert.equal(streams[1].request.headers["Last-Event-ID"], "41");
  streams[1].push(frame("run-line", { line: "two" }, "42"));
  await tick();
  assert.deepEqual(seen, ["one", "two"]);
});

test("the token comes from the URL fragment or query, is kept for this tab only and is removed from the address", async () => {
  const fromHash = install({ hash: "#token=hashtoken-0123456789abcdef", routes: { "/api/invoke/ping": () => ({ status: 200, body: "pong" }) } });
  const { invoke } = await import("@tauri-apps/api/core");
  await invoke("ping");
  assert.equal(fromHash.calls.at(-1).init.headers.Authorization, "Bearer hashtoken-0123456789abcdef");
  assert.equal(fromHash.store.get("hevc.token"), "hashtoken-0123456789abcdef", "kept in sessionStorage, which ends with the tab");
  assert.deepEqual(fromHash.history.replaced.at(-1).slice(2), ["/library"], "the token is replaced out of the address bar, leaving only the path");

  const fromQuery = install({ search: "?token=querytoken-0123456789abcdef&x=1", routes: { "/api/invoke/ping": () => ({ status: 200, body: "pong" }) } });
  await invoke("ping");
  assert.equal(fromQuery.calls.at(-1).init.headers.Authorization, "Bearer querytoken-0123456789abcdef");
  assert.equal(fromQuery.history.replaced.length, 1);

  const reloaded = install({ stored: { "hevc.token": "kept-token-0123456789abcdef0" }, routes: { "/api/invoke/ping": () => ({ status: 200, body: "pong" }) } });
  await invoke("ping");
  assert.equal(reloaded.calls.at(-1).init.headers.Authorization, "Bearer kept-token-0123456789abcdef0", "a reload in the same tab stays signed in");
  assert.equal(reloaded.history.replaced.length, 0, "nothing to strip");

  const none = install({ routes: { "/api/invoke/ping": () => ({ status: 401, body: { error: "x" } }) } });
  await assert.rejects(() => invoke("ping"), (error) => /login link/.test(error), "without a token the user is told to open the login link");
  assert.equal(none.calls.at(-1).init.headers.Authorization, undefined);
});

test("no cookie is ever involved: requests omit credentials", async () => {
  const { calls } = install({ hash: "#token=abcdefabcdefabcdefabcdef", routes: { "/api/invoke/ping": () => ({ status: 200, body: 1 }) } });
  const { invoke } = await import("@tauri-apps/api/core");
  await invoke("ping");
  assert.equal(calls.at(-1).init.credentials, "omit");
});

test("two listeners on the same event both receive it", async () => {
  const { streams } = install({ hash: "#token=abcdefabcdefabcdefabcdef" });
  const { listen } = await import("@tauri-apps/api/event");
  let a = 0;
  let b = 0;
  await listen("run-progress", () => a++);
  await listen("run-progress", () => b++);
  await tick();
  streams[0].push(frame("run-progress", {}));
  await tick();
  assert.deepEqual([a, b], [1, 1]);
});

test("a context menu built with the real Menu API opens in the page and runs the chosen action", async () => {
  const { win } = install();
  const executed = [];
  win.document = { execCommand: (command) => { executed.push(command); return true; } };
  const { Menu } = await import("@tauri-apps/api/menu");
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  const picked = [];
  const menu = await Menu.new({
    items: [
      { id: "choose-folder", text: "Choose media folder", enabled: true, action: (id) => picked.push(["choose-folder", id]) },
      { item: "Separator" },
      { id: "library", text: "Library", enabled: false, action: () => picked.push(["library"]) },
      { id: "copy-path", text: "Copy library path", action: (id) => picked.push(["copy-path", id]) },
      { item: "Copy" },
    ],
  });
  await menu.popup(undefined, getCurrentWindow());
  const shown = win.__HEVC_WEB_SERVER__.menu.current();
  assert.deepEqual(shown.map((entry) => [entry.type, entry.text ?? entry.name ?? null, entry.enabled ?? true]), [
    ["item", "Choose media folder", true], ["separator", null, true], ["item", "Library", false], ["item", "Copy library path", true], ["builtin", "Copy", true],
  ]);
  assert.equal(win.__HEVC_WEB_SERVER__.menu.activate("choose-folder"), true);
  assert.deepEqual(win.__HEVC_WEB_SERVER__.menu.current(), [], "choosing an item closes the menu");
  await menu.popup();
  assert.equal(win.__HEVC_WEB_SERVER__.menu.activate("copy-path"), true);
  await menu.popup();
  assert.deepEqual(picked, [["choose-folder", "choose-folder"], ["copy-path", "copy-path"]], "each action receives its item id once");
  assert.equal(win.__HEVC_WEB_SERVER__.menu.activate("library"), false, "a disabled item does nothing");
  assert.equal(win.__HEVC_WEB_SERVER__.menu.activate("nope"), false);
  assert.equal(picked.length, 2);
  win.__HEVC_WEB_SERVER__.menu.activateBuiltin("Copy");
  win.__HEVC_WEB_SERVER__.menu.activateBuiltin("SelectAll");
  win.__HEVC_WEB_SERVER__.menu.activateBuiltin("Cut");
  assert.deepEqual(executed, ["copy", "selectAll", "cut"]);
  await menu.popup();
  await menu.close();
  assert.deepEqual(win.__HEVC_WEB_SERVER__.menu.current(), [], "closing the menu resource forgets it");
});

test("opening a second menu replaces the first and an editing menu offers only the built-in items", async () => {
  const { win } = install();
  win.document = { execCommand: () => true };
  const { Menu } = await import("@tauri-apps/api/menu");
  const first = await Menu.new({ items: [{ id: "a", text: "A", action: () => {} }] });
  await first.popup();
  const editing = await Menu.new({ items: [{ item: "Cut" }, { item: "Copy" }, { item: "Paste" }, { item: "Separator" }, { item: "SelectAll" }] });
  await editing.popup();
  assert.deepEqual(win.__HEVC_WEB_SERVER__.menu.current().map((entry) => entry.type === "builtin" ? entry.name : entry.type), ["Cut", "Copy", "Paste", "separator", "SelectAll"]);
});

test("window, webview and menu calls the GUI makes at boot are inert but do not fail", async () => {
  install();
  const { getCurrentWindow, currentMonitor } = await import("@tauri-apps/api/window");
  const { Menu } = await import("@tauri-apps/api/menu");
  assert.equal(await currentMonitor(), null);
  const current = getCurrentWindow();
  assert.equal(await current.scaleFactor(), 1);
  assert.equal(await current.isMaximized(), false);
  const unlistenResize = await current.onResized(() => {});
  const unlistenDrop = await current.onDragDropEvent(() => {});
  assert.equal(typeof unlistenResize, "function");
  assert.equal(typeof unlistenDrop, "function");
  const menu = await Menu.new({ items: [{ item: "Copy" }] });
  await menu.popup(undefined, current);
});

test("app version comes from the server; the updater reports up to date; opener opens a tab", async () => {
  const { win } = install({ routes: { "/api/info": () => ({ status: 200, body: { version: "9.9.9", readOnly: false } }) } });
  const { getVersion } = await import("@tauri-apps/api/app");
  assert.equal(await getVersion(), "9.9.9");
  const { check } = await import("@tauri-apps/plugin-updater");
  assert.equal(await check(), null);
  const { openUrl } = await import("@tauri-apps/plugin-opener");
  await openUrl("https://www.themoviedb.org/movie/1");
  assert.deepEqual(win.opened[0].slice(0, 2), ["https://www.themoviedb.org/movie/1", "_blank"]);
});

test("restart and unknown plugins fail loudly instead of pretending to work", async () => {
  install();
  const { relaunch } = await import("@tauri-apps/plugin-process");
  await assert.rejects(() => relaunch());
  const { invoke } = await import("@tauri-apps/api/core");
  await assert.rejects(() => invoke("plugin:made-up|thing"), /Not available in server mode/);
});

test("file pickers are answered by the server-side path browser, not the browser's filesystem", async () => {
  // No document: the browser cannot draw, so the call fails with a clear message rather than hanging.
  install();
  const { open } = await import("@tauri-apps/plugin-dialog");
  await assert.rejects(() => open({ directory: true }), /needs a page/);
});

test("path helpers handle both separators and filters", () => {
  const { win } = install();
  const { joinPath, extensionAllowed } = win.__HEVC_WEB_SERVER__.helpers;
  assert.equal(joinPath("/srv/media/", "a.mkv"), "/srv/media/a.mkv");
  assert.equal(joinPath("C:\\Media", "a.mkv"), "C:\\Media\\a.mkv");
  assert.equal(extensionAllowed("Poster.JPG", [{ name: "Images", extensions: ["jpg", "png"] }]), true);
  assert.equal(extensionAllowed("clip.mkv", [{ name: "Images", extensions: ["jpg", "png"] }]), false);
  assert.equal(extensionAllowed("anything.bin", undefined), true);
});
