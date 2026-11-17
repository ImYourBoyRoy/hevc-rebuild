// HEVC Rebuild Studio web shim: lets the unmodified desktop GUI run in an ordinary browser tab against
// the headless server (`hevc-rebuild-studio serve`). It installs the object `@tauri-apps/api` calls
// (`window.__TAURI_INTERNALS__`) and translates:
//   invoke("command", args)        -> POST /api/invoke/command
//   listen("event", handler)       -> one Server-Sent Events stream, GET /api/events
//   dialog open/save               -> a folder browser backed by the server's filesystem
//   window/menu/updater/process    -> inert answers (a browser tab has no native window)
// Served first in <head> by the server; see docs/maintainer/SERVE_MODE.md.
(function (win, fetchImpl, EventSourceImpl) {
  "use strict";
  if (win.__TAURI_INTERNALS__) return;

  var callbacks = new Map();
  var nextCallbackId = 1;
  var listeners = new Map(); // event name -> Map(eventId -> callback id)
  var nextEventId = 1;
  var nextResourceId = 1;
  var info = null;
  var source = null;

  function transformCallback(callback, once) {
    var id = nextCallbackId++;
    callbacks.set(id, function (data) {
      if (once) callbacks.delete(id);
      return callback && callback(data);
    });
    return id;
  }
  function unregisterCallback(id) { callbacks.delete(id); }
  function runCallback(id, data) {
    var callback = callbacks.get(id);
    if (callback) callback(data);
  }

  function failure(message) { return Promise.reject(typeof message === "string" ? message : String(message)); }

  function loadInfo() {
    if (!info) {
      info = fetchImpl("/api/info", { credentials: "same-origin" })
        .then(function (response) { return response.ok ? response.json() : { version: "0.0.0", readOnly: false }; })
        .catch(function () { return { version: "0.0.0", readOnly: false }; });
    }
    return info;
  }

  function post(command, args) {
    return fetchImpl("/api/invoke/" + encodeURIComponent(command), {
      method: "POST",
      credentials: "same-origin",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(args === undefined || args === null ? {} : args),
    }).then(function (response) {
      return response.text().then(function (text) {
        var body = null;
        try { body = text ? JSON.parse(text) : null; } catch (error) { body = null; }
        if (response.ok) return body;
        if (response.status === 401) return failure("The server session ended. Open the login link printed by the server again.");
        return failure(body && body.error ? body.error : "The server answered " + response.status + ".");
      });
    }, function (error) { return failure("Could not reach the server: " + (error && error.message ? error.message : error)); });
  }

  // ---- events --------------------------------------------------------------------------------
  function deliver(event, payload) {
    var registered = listeners.get(event);
    if (!registered) return;
    registered.forEach(function (callbackId, eventId) {
      runCallback(callbackId, { event: event, id: eventId, payload: payload });
    });
  }
  function ensureEventStream() {
    if (source || !EventSourceImpl) return;
    source = new EventSourceImpl("/api/events", { withCredentials: true });
    source.onmessage = function (message) {
      try {
        var parsed = JSON.parse(message.data);
        deliver(parsed.event, parsed.payload);
      } catch (error) { /* ignore a malformed frame */ }
    };
  }
  function addListener(args) {
    ensureEventStream();
    var eventId = nextEventId++;
    if (!listeners.has(args.event)) listeners.set(args.event, new Map());
    listeners.get(args.event).set(eventId, args.handler);
    return eventId;
  }
  function removeListener(event, eventId) {
    var registered = listeners.get(event);
    if (registered) registered.delete(eventId);
  }

  // ---- path browser (replaces the native pickers) ---------------------------------------------
  function joinPath(directory, name) {
    var separator = directory.indexOf("\\") >= 0 && directory.indexOf("/") < 0 ? "\\" : "/";
    return directory.replace(/[\\/]+$/, "") + separator + name;
  }
  function extensionAllowed(name, filters) {
    if (!filters || !filters.length) return true;
    var lower = name.toLowerCase();
    return filters.some(function (filter) {
      return !filter.extensions || !filter.extensions.length || filter.extensions.some(function (ext) {
        return ext === "*" || lower.endsWith("." + String(ext).toLowerCase());
      });
    });
  }

  var lastDirectory = null;
  function browse(mode, options) {
    options = options || {};
    var doc = win.document;
    if (!doc) return failure("A folder browser needs a page.");
    return new Promise(function (resolve) {
      var directoryMode = mode === "open" && Boolean(options.directory);
      var multiple = mode === "open" && Boolean(options.multiple);
      var chosen = [];
      var current = null;

      var backdrop = doc.createElement("div");
      backdrop.setAttribute("data-hevc-web-browser", "");
      backdrop.setAttribute("role", "dialog");
      backdrop.setAttribute("aria-modal", "true");
      backdrop.style.cssText = "position:fixed;inset:0;z-index:2147483000;background:rgba(0,0,0,.55);display:flex;align-items:center;justify-content:center;font:14px system-ui,sans-serif";
      var panel = doc.createElement("div");
      panel.style.cssText = "background:var(--surface,#1f2330);color:var(--text,#e8eaf2);border:1px solid var(--border,#3a4052);border-radius:10px;width:min(720px,94vw);max-height:84vh;display:flex;flex-direction:column;gap:8px;padding:14px;box-shadow:0 12px 40px rgba(0,0,0,.5)";
      var title = doc.createElement("strong");
      title.textContent = options.title || (mode === "save" ? "Choose where to save (on the server)" : directoryMode ? "Choose a folder on the server" : "Choose a file on the server");
      var pathInput = doc.createElement("input");
      pathInput.type = "text";
      pathInput.setAttribute("aria-label", "Folder path on the server");
      pathInput.style.cssText = "padding:6px 8px;border-radius:6px;border:1px solid var(--border,#3a4052);background:var(--bg,#14161e);color:inherit;font:inherit";
      var list = doc.createElement("div");
      list.style.cssText = "overflow:auto;min-height:200px;border:1px solid var(--border,#3a4052);border-radius:6px";
      var status = doc.createElement("div");
      status.style.cssText = "opacity:.8;min-height:1.2em";
      var nameInput = doc.createElement("input");
      nameInput.type = "text";
      nameInput.setAttribute("aria-label", "File name");
      nameInput.placeholder = "File name";
      nameInput.style.cssText = pathInput.style.cssText + (mode === "save" ? "" : ";display:none");
      var row = doc.createElement("div");
      row.style.cssText = "display:flex;gap:8px;justify-content:flex-end;flex-wrap:wrap";

      function button(label, onClick) {
        var element = doc.createElement("button");
        element.type = "button";
        element.textContent = label;
        element.style.cssText = "padding:6px 12px;border-radius:6px;border:1px solid var(--border,#3a4052);background:var(--surface-2,#2a3042);color:inherit;font:inherit;cursor:pointer";
        element.addEventListener("click", onClick);
        return element;
      }
      function finish(value) {
        doc.removeEventListener("keydown", onKey, true);
        backdrop.remove();
        resolve(value);
      }
      function onKey(event) { if (event.key === "Escape") { event.stopPropagation(); finish(null); } }
      doc.addEventListener("keydown", onKey, true);

      var confirm = button("Choose", function () {
        if (mode === "save") {
          var name = nameInput.value.trim();
          if (!name) { status.textContent = "Enter a file name."; return; }
          finish(joinPath(current || pathInput.value, name));
        } else if (directoryMode && !chosen.length) {
          finish(multiple ? [current] : current);
        } else {
          finish(multiple ? chosen.slice() : chosen[0] || null);
        }
      });
      function refreshConfirm() {
        confirm.textContent = multiple && chosen.length ? "Choose " + chosen.length : directoryMode && !chosen.length ? "Choose this folder" : "Choose";
        confirm.disabled = !directoryMode && mode === "open" && !chosen.length;
        confirm.style.opacity = confirm.disabled ? ".5" : "1";
      }
      function toggle(path) {
        var at = chosen.indexOf(path);
        if (at >= 0) chosen.splice(at, 1);
        else if (multiple) chosen.push(path);
        else chosen = [path];
        render(lastListing);
      }

      var lastListing = null;
      function render(listing) {
        lastListing = listing;
        list.textContent = "";
        if (!listing) return;
        if (listing.parent) {
          var up = button(".. (up)", function () { go(listing.parent); });
          up.style.cssText += ";display:block;width:100%;text-align:left;border:0;border-radius:0";
          list.appendChild(up);
        }
        listing.entries.forEach(function (entry) {
          if (!entry.isDir && (directoryMode || mode === "save" || !extensionAllowed(entry.name, options.filters))) return;
          var item = button((entry.isDir ? "📁 " : "") + entry.name, function () {
            if (entry.isDir) go(entry.path);
            else if (mode === "save") nameInput.value = entry.name;
            else toggle(entry.path);
          });
          item.style.cssText += ";display:block;width:100%;text-align:left;border:0;border-radius:0" + (chosen.indexOf(entry.path) >= 0 ? ";outline:2px solid var(--accent,#6aa7ff)" : "");
          list.appendChild(item);
          if (entry.isDir && directoryMode) {
            var pick = button(chosen.indexOf(entry.path) >= 0 ? "Selected ✓" : "Select", function () { toggle(entry.path); });
            pick.style.cssText += ";margin-left:8px";
            item.appendChild(pick);
          }
        });
        status.textContent = (listing.truncated ? "Showing the first entries only. " : "") + (listing.entries.length ? "" : "This folder is empty.");
        refreshConfirm();
      }
      function go(path) {
        status.textContent = "Loading…";
        post("list_directory", { path: path }).then(function (listing) {
          current = listing.path;
          lastDirectory = listing.path;
          pathInput.value = listing.path;
          render(listing);
        }, function (error) { status.textContent = String(error); });
      }
      pathInput.addEventListener("keydown", function (event) { if (event.key === "Enter") go(pathInput.value); });

      var cancel = button("Cancel", function () { finish(null); });
      row.appendChild(cancel);
      row.appendChild(confirm);
      panel.appendChild(title);
      panel.appendChild(pathInput);
      panel.appendChild(list);
      panel.appendChild(nameInput);
      panel.appendChild(status);
      panel.appendChild(row);
      backdrop.appendChild(panel);
      doc.body.appendChild(backdrop);
      refreshConfirm();
      pathInput.focus();
      go(options.defaultPath || lastDirectory || undefined);
    });
  }

  // ---- plugin commands -------------------------------------------------------------------------
  function windowCommand(name) {
    switch (name) {
      case "scale_factor": return Promise.resolve(1);
      case "inner_size": case "outer_size": return Promise.resolve({ width: win.innerWidth || 1280, height: win.innerHeight || 800 });
      case "inner_position": case "outer_position": return Promise.resolve({ x: 0, y: 0 });
      case "is_maximized": case "is_fullscreen": case "is_minimized": case "is_decorated": return Promise.resolve(false);
      case "is_visible": case "is_focused": case "is_resizable": return Promise.resolve(true);
      case "get_all_windows": return Promise.resolve(["main"]);
      case "current_monitor": case "primary_monitor": case "monitor_from_point": return Promise.resolve(null);
      case "available_monitors": return Promise.resolve([]);
      case "title": return Promise.resolve(win.document ? win.document.title : "");
      default: return Promise.resolve(null);
    }
  }

  function pluginInvoke(command, args) {
    var parts = command.slice("plugin:".length).split("|");
    var plugin = parts[0];
    var name = parts[1];
    args = args || {};
    switch (plugin) {
      case "event":
        if (name === "listen") return Promise.resolve(addListener(args));
        if (name === "unlisten") { removeListener(args.event, args.eventId); return Promise.resolve(null); }
        return Promise.resolve(null); // emit / emit_to: nothing else listens in a browser
      case "app":
        return loadInfo().then(function (data) {
          if (name === "version") return data.version;
          if (name === "name") return "HEVC Rebuild Studio";
          if (name === "tauri_version") return data.version;
          return null;
        });
      case "dialog":
        if (name === "open") return browse("open", args.options || args);
        if (name === "save") return browse("save", args.options || args);
        if (name === "ask" || name === "confirm") return Promise.resolve(win.confirm ? win.confirm(args.message) : false);
        if (name === "message") { if (win.alert) win.alert(args.message); return Promise.resolve("Ok"); }
        return failure("Unsupported dialog command: " + name);
      case "opener":
        if (name === "open_url") { win.open(args.url, "_blank", "noopener,noreferrer"); return Promise.resolve(null); }
        return failure("There is no desktop file manager on the server.");
      case "updater":
        if (name === "check") return Promise.resolve(null); // a headless install updates through its package manager
        return failure("Updates are not installed from the browser.");
      case "process":
        return failure("The server process cannot be restarted from the browser.");
      case "window": case "webview":
        return windowCommand(name);
      case "menu": case "resources":
        if (name === "new") { var rid = nextResourceId++; return Promise.resolve([rid, "menu-" + rid]); }
        return Promise.resolve(null); // popup/close: a browser keeps its own context menu behaviour
      default:
        return failure("Not available in server mode: " + command);
    }
  }

  function invoke(command, args) {
    if (typeof command === "string" && command.indexOf("plugin:") === 0) return pluginInvoke(command, args);
    return post(command, args);
  }

  win.__TAURI_INTERNALS__ = {
    invoke: invoke,
    transformCallback: transformCallback,
    unregisterCallback: unregisterCallback,
    runCallback: runCallback,
    callbacks: callbacks,
    convertFileSrc: function (path) { return path; },
    metadata: {
      currentWindow: { label: "main" },
      currentWebview: { windowLabel: "main", label: "main" },
    },
  };
  win.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: removeListener };
  win.isTauri = true;
  win.__HEVC_WEB_SERVER__ = { loadInfo: loadInfo, helpers: { joinPath: joinPath, extensionAllowed: extensionAllowed } };
})(window, typeof fetch === "function" ? fetch.bind(window) : undefined, typeof EventSource === "function" ? EventSource : undefined);
