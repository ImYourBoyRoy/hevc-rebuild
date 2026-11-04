//! Opt-in local control channel for native Tauri walkthroughs.
//!
//! The channel is deliberately disabled unless `HEVC_REBUILD_AGENT_CONTROL=1` is
//! present in the launched application's environment. When enabled, the app
//! publishes a per-user local socket (Unix) or named pipe (Windows) and accepts
//! one JSON-lines request at a time. The webview performs key dispatch and
//! returns bounded state/text snapshots through the normal Tauri command path.
//! No network listener or general shell execution is exposed.

#[cfg(not(target_os = "linux"))]
use interprocess::local_socket::GenericFilePath;
#[cfg(target_os = "linux")]
use interprocess::local_socket::GenericNamespaced;
use interprocess::local_socket::{prelude::*, ListenerOptions, Stream};
#[cfg(unix)]
use interprocess::os::unix::local_socket::ListenerOptionsExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager, State};

const ENDPOINT_FILE: &str = "agent-control.json";
const MAX_REQUEST_BYTES: usize = 64 * 1024;
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Default)]
pub(crate) struct AgentControlState {
    pending: Arc<Mutex<BTreeMap<String, Sender<Value>>>>,
    frontend_ready: Arc<AtomicBool>,
    last_event_id: Arc<Mutex<Option<String>>>,
}

#[derive(Debug, Deserialize)]
struct ControlRequest {
    id: String,
    action: String,
    #[serde(default)]
    key: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct EndpointDescription {
    protocol: u8,
    transport: String,
    name: String,
    pid: u32,
}

fn enabled() -> bool {
    std::env::var("HEVC_REBUILD_AGENT_CONTROL")
        .ok()
        .is_some_and(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
}

fn endpoint_file() -> Result<PathBuf, String> {
    Ok(crate::app_state_directory()?.join(ENDPOINT_FILE))
}

fn endpoint_name() -> Result<String, String> {
    #[cfg(windows)]
    {
        Ok(format!(
            r"\\.\pipe\hevc-rebuild-studio-agent-control-{}",
            std::process::id()
        ))
    }
    #[cfg(target_os = "linux")]
    {
        // Linux abstract namespace sockets have no filesystem path, so a
        // project-local portable profile can remain fully local without
        // exceeding the host's Unix socket path limit.
        Ok(format!(
            "hevc-rebuild-studio-agent-control-{}",
            std::process::id()
        ))
    }
    #[cfg(all(unix, not(target_os = "linux")))]
    {
        let path = crate::app_state_directory()?.join("agent-control.sock");
        let name = path
            .to_str()
            .ok_or_else(|| "The agent-control socket path is not valid UTF-8.".to_string())?;
        if name.len() > 100 {
            return Err(
                "The agent-control socket path is too long for the host Unix socket limit.".into(),
            );
        }
        Ok(name.to_owned())
    }
}

fn write_endpoint(name: &str) -> Result<(), String> {
    let path = endpoint_file()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create the agent-control directory: {error}"))?;
    }
    let payload = serde_json::to_vec_pretty(&EndpointDescription {
        protocol: 1,
        transport: "local-socket".into(),
        name: name.to_owned(),
        pid: std::process::id(),
    })
    .map_err(|error| format!("Could not serialize the agent-control endpoint: {error}"))?;
    fs::write(&path, payload)
        .map_err(|error| format!("Could not publish the agent-control endpoint: {error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("Could not protect the agent-control endpoint: {error}"))?;
    }
    Ok(())
}

fn remove_endpoint() {
    if let Ok(path) = endpoint_file() {
        let owned_by_current_process = fs::read_to_string(&path)
            .ok()
            .and_then(|payload| serde_json::from_str::<EndpointDescription>(&payload).ok())
            .is_some_and(|endpoint| endpoint.pid == std::process::id());
        if owned_by_current_process {
            let _ = fs::remove_file(path);
        }
    }
}

pub(crate) fn cleanup() {
    remove_endpoint();
}

fn validate_request(request: &ControlRequest) -> Result<(), String> {
    if request.id.is_empty() || request.id.len() > 128 || request.id.chars().any(char::is_control) {
        return Err("The agent-control request id is invalid.".into());
    }
    if !matches!(
        request.action.as_str(),
        "press" | "snapshot" | "state" | "screenshot"
    ) {
        return Err(
            "The agent-control action must be press, snapshot, state, or screenshot.".into(),
        );
    }
    if request.action == "press" {
        let key = request
            .key
            .as_deref()
            .ok_or("The press action requires a key.")?;
        if key.is_empty() || key.len() > 64 || key.chars().any(char::is_control) {
            return Err("The agent-control key is invalid.".into());
        }
    }
    Ok(())
}

fn error_response(id: Option<&str>, message: impl Into<String>) -> Value {
    json!({
        "id": id,
        "ok": false,
        "error": message.into(),
    })
}

fn state_response(state: &AgentControlState, app: &AppHandle) -> Value {
    let pending = state
        .pending
        .lock()
        .map(|pending| pending.len())
        .unwrap_or(0);
    let last_event_id = state
        .last_event_id
        .lock()
        .ok()
        .and_then(|event_id| event_id.clone());
    let webview_url = app
        .get_webview_window("main")
        .and_then(|webview| webview.url().ok())
        .map(|url| url.to_string());
    json!({
        "frontend_ready": state.frontend_ready.load(Ordering::Acquire),
        "pending_requests": pending,
        "last_event_id": last_event_id,
        "webview_available": app.get_webview_window("main").is_some(),
        "webview_url": webview_url,
    })
}

fn dispatch(request: ControlRequest, app: &AppHandle, state: &AgentControlState) -> Value {
    if let Err(error) = validate_request(&request) {
        return error_response(Some(&request.id), error);
    }
    if request.action == "state" {
        return json!({ "id": request.id, "ok": true, "result": state_response(state, app) });
    }
    let (sender, receiver) = mpsc::channel();
    if state
        .pending
        .lock()
        .map_err(|_| "Agent-control state is unavailable.")
        .and_then(|mut pending| {
            if pending.contains_key(&request.id) {
                return Err("The agent-control request id is already pending.");
            }
            pending.insert(request.id.clone(), sender);
            Ok(())
        })
        .is_err()
    {
        return error_response(
            Some(&request.id),
            "Could not register the agent-control request.",
        );
    }

    let id = match serde_json::to_string(&request.id) {
        Ok(value) => value,
        Err(error) => {
            let _ = state
                .pending
                .lock()
                .map(|mut pending| pending.remove(&request.id));
            return error_response(
                Some(&request.id),
                format!("Could not serialize the agent-control request id: {error}"),
            );
        }
    };
    let action = serde_json::to_string(&request.action).unwrap_or_else(|_| "null".into());
    let key = serde_json::to_string(&request.key).unwrap_or_else(|_| "null".into());
    let script = format!(
        r#"(() => {{
          const internals = window.__TAURI_INTERNALS__;
          const id = {id};
          const action = {action};
          const key = {key};
          const send = (response) => {{
            if (internals && typeof internals.invoke === "function") {{
              void internals.invoke("reply", {{ id, response }}).catch(() => {{}});
            }}
          }};
          const snapshot = () => {{
            const active = document.activeElement;
            const current = document.querySelector('[aria-current="page"]');
            const label = document.querySelector(".breadcrumbs strong");
            return {{
              page: current?.getAttribute("aria-label")?.toLowerCase() ?? null,
              page_label: label?.textContent?.trim() ?? null,
              booting: (document.body?.innerText ?? "").includes("Checking this computer"),
              workflow_busy: (document.body?.innerText ?? "").includes("Running"),
              active_element: active && "outerHTML" in active ? String(active.outerHTML).slice(0, 2000) : null,
              title: document.title,
              location: window.location.href,
              text: String(document.body?.innerText ?? "").slice(0, 30000)
            }};
          }};
          if (!internals || typeof internals.invoke !== "function") {{
            return;
          }}
          if (action === "press") {{
            const parts = String(key ?? "").split("+").map((part) => part.trim()).filter(Boolean);
            const requested = parts.pop() ?? String(key ?? "");
            const eventKey = requested === "Space" ? " " : requested;
            window.dispatchEvent(new KeyboardEvent("keydown", {{
              key: eventKey,
              code: eventKey.length === 1 ? `Key${{eventKey.toUpperCase()}}` : eventKey,
              altKey: parts.some((part) => part.toLowerCase() === "alt"),
              ctrlKey: parts.some((part) => ["ctrl", "control"].includes(part.toLowerCase())),
              metaKey: parts.some((part) => ["meta", "command"].includes(part.toLowerCase())),
              shiftKey: parts.some((part) => part.toLowerCase() === "shift"),
              bubbles: true,
              cancelable: true
            }}));
            window.setTimeout(() => send({{ handled: true, ...snapshot() }}), 50);
          }} else if (action === "screenshot") {{
            send({{ supported: false, message: "Use the native desktop capture surface for screenshots; the control channel returns text snapshots only.", ...snapshot() }});
          }} else {{
            send(snapshot());
          }}
        }})();"#
    );
    let eval_result = app
        .get_webview_window("main")
        .ok_or_else(|| "The main native webview is not available.".to_string())
        .and_then(|webview| {
            webview.eval(script).map_err(|error| {
                format!("Could not dispatch the request to the native webview: {error}")
            })
        });
    if let Err(error) = eval_result {
        let _ = state
            .pending
            .lock()
            .map(|mut pending| pending.remove(&request.id));
        return error_response(Some(&request.id), error);
    }
    if let Ok(mut last_event_id) = state.last_event_id.lock() {
        *last_event_id = Some(request.id.clone());
    }

    let response = match receiver.recv_timeout(RESPONSE_TIMEOUT) {
        Ok(value) => json!({ "id": request.id, "ok": true, "result": value }),
        Err(error) => error_response(
            Some(&request.id),
            format!("The webview did not answer the agent-control request: {error}"),
        ),
    };
    let _ = state
        .pending
        .lock()
        .map(|mut pending| pending.remove(&request.id));
    response
}

fn handle_connection(mut stream: Stream, app: AppHandle, state: AgentControlState) {
    let mut line = String::new();
    let read_result = {
        let mut reader = BufReader::new(&mut stream);
        reader.read_line(&mut line)
    };
    let response = match read_result {
        Ok(0) => error_response(None, "The agent-control request was empty."),
        Ok(size) if size > MAX_REQUEST_BYTES => {
            error_response(None, "The agent-control request is too large.")
        }
        Ok(_) => match serde_json::from_str::<ControlRequest>(line.trim_end()) {
            Ok(request) => dispatch(request, &app, &state),
            Err(error) => error_response(
                None,
                format!("The agent-control request is not valid JSON: {error}"),
            ),
        },
        Err(error) => error_response(
            None,
            format!("Could not read the agent-control request: {error}"),
        ),
    };
    if let Ok(payload) = serde_json::to_vec(&response) {
        let _ = stream.write_all(&payload);
        let _ = stream.write_all(b"\n");
        let _ = stream.flush();
    }
}

/// Start the opt-in local server used by native walkthroughs.
pub(crate) fn start_server(app: AppHandle, state: AgentControlState) -> Result<(), String> {
    if !enabled() {
        return Ok(());
    }
    let name_text = endpoint_name()?;
    #[cfg(target_os = "linux")]
    let name = name_text
        .as_str()
        .to_ns_name::<GenericNamespaced>()
        .map_err(|error| format!("Could not create the agent-control socket name: {error}"))?;
    #[cfg(not(target_os = "linux"))]
    let name = name_text
        .as_str()
        .to_fs_name::<GenericFilePath>()
        .map_err(|error| format!("Could not create the agent-control socket name: {error}"))?;
    let mut options = ListenerOptions::new()
        .name(name)
        .reclaim_name(true)
        .try_overwrite(false);
    #[cfg(unix)]
    {
        options = options.mode(0o600);
    }
    let listener = options
        .create_sync()
        .map_err(|error| format!("Could not start the agent-control listener: {error}"))?;
    write_endpoint(&name_text)?;
    let server_state = state.clone();
    thread::Builder::new()
        .name("hevc-agent-control".into())
        .spawn(move || {
            for incoming in listener.incoming() {
                match incoming {
                    Ok(stream) => {
                        let app = app.clone();
                        let state = server_state.clone();
                        let _ = thread::Builder::new()
                            .name("hevc-agent-control-request".into())
                            .spawn(move || handle_connection(stream, app, state));
                    }
                    Err(error) => eprintln!("agent-control listener error: {error}"),
                }
            }
            remove_endpoint();
        })
        .map_err(|error| format!("Could not start the agent-control thread: {error}"))?;
    Ok(())
}

#[tauri::command]
pub(crate) fn reply(
    state: State<'_, AgentControlState>,
    id: String,
    response: Value,
) -> Result<(), String> {
    let sender = state
        .pending
        .lock()
        .map_err(|_| "Agent-control state is unavailable.".to_string())?
        .get(&id)
        .cloned()
        .ok_or_else(|| "The agent-control request is no longer pending.".to_string())?;
    sender
        .send(response)
        .map_err(|_| "The agent-control client disconnected before receiving the response.".into())
}

#[tauri::command]
pub(crate) fn frontend_ready(state: State<'_, AgentControlState>) {
    state.frontend_ready.store(true, Ordering::Release);
}

fn endpoint_from_file(path: Option<PathBuf>) -> Result<String, String> {
    let path = match path {
        Some(path) => path,
        None => endpoint_file()?,
    };
    let payload = fs::read_to_string(&path).map_err(|error| {
        format!(
            "Could not read agent-control endpoint {}: {error}",
            path.display()
        )
    })?;
    let endpoint: EndpointDescription = serde_json::from_str(&payload)
        .map_err(|error| format!("Could not parse agent-control endpoint: {error}"))?;
    if endpoint.protocol != 1 || endpoint.transport != "local-socket" {
        return Err("The agent-control endpoint protocol is unsupported.".into());
    }
    Ok(endpoint.name)
}

fn request_id() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("{}-{millis}", std::process::id())
}

/// CLI client for the opt-in local control channel.
pub fn client_mode(args: Vec<std::ffi::OsString>) -> i32 {
    let mut args = args.into_iter();
    let mut endpoint = None;
    let mut action = None;
    let mut key = None;
    while let Some(argument) = args.next() {
        let argument = argument.to_string_lossy().into_owned();
        match argument.as_str() {
            "--endpoint" => endpoint = args.next().map(PathBuf::from),
            "--help" | "-h" => {
                println!("Usage: hevc-rebuild-studio agent-control [--endpoint PATH] <state|snapshot|screenshot|press KEY>");
                return 0;
            }
            value if action.is_none() => action = Some(value.to_owned()),
            value if key.is_none() => key = Some(value.to_owned()),
            _ => {
                eprintln!("agent-control accepts one action and, for press, one key.");
                return 2;
            }
        }
    }
    let action = match action {
        Some(action)
            if matches!(
                action.as_str(),
                "state" | "snapshot" | "screenshot" | "press"
            ) =>
        {
            action
        }
        _ => {
            eprintln!("Usage: hevc-rebuild-studio agent-control [--endpoint PATH] <state|snapshot|screenshot|press KEY>");
            return 2;
        }
    };
    if action == "press" && key.is_none() {
        eprintln!("agent-control press requires a key.");
        return 2;
    }
    let name_text = match endpoint_from_file(endpoint) {
        Ok(name) => name,
        Err(error) => {
            eprintln!("{error}");
            return 1;
        }
    };
    #[cfg(target_os = "linux")]
    let name = match name_text.as_str().to_ns_name::<GenericNamespaced>() {
        Ok(name) => name,
        Err(error) => {
            eprintln!("Could not create the agent-control socket name: {error}");
            return 1;
        }
    };
    #[cfg(not(target_os = "linux"))]
    let name = match name_text.as_str().to_fs_name::<GenericFilePath>() {
        Ok(name) => name,
        Err(error) => {
            eprintln!("Could not create the agent-control socket name: {error}");
            return 1;
        }
    };
    let mut stream = match Stream::connect(name) {
        Ok(stream) => stream,
        Err(error) => {
            eprintln!("Could not connect to the launched application: {error}");
            return 1;
        }
    };
    let request = json!({
        "id": request_id(),
        "action": action,
        "key": key,
    });
    if let Err(error) = writeln!(stream, "{request}") {
        eprintln!("Could not send the agent-control request: {error}");
        return 1;
    }
    let mut response = String::new();
    if let Err(error) = BufReader::new(stream).read_line(&mut response) {
        eprintln!("Could not read the agent-control response: {error}");
        return 1;
    }
    println!("{}", response.trim_end());
    match serde_json::from_str::<Value>(response.trim_end()) {
        Ok(value) if value.get("ok") == Some(&Value::Bool(true)) => 0,
        Ok(_) => 1,
        Err(_) => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_validation_rejects_unknown_actions_and_accepts_snapshots() {
        assert!(validate_request(&ControlRequest {
            id: "one".into(),
            action: "snapshot".into(),
            key: None,
        })
        .is_ok());
        assert!(validate_request(&ControlRequest {
            id: "one".into(),
            action: "shell".into(),
            key: None,
        })
        .is_err());
    }

    #[test]
    fn request_validation_requires_a_bounded_key_for_press() {
        assert!(validate_request(&ControlRequest {
            id: "one".into(),
            action: "press".into(),
            key: None,
        })
        .is_err());
        assert!(validate_request(&ControlRequest {
            id: "one".into(),
            action: "press".into(),
            key: Some("Alt+1".into()),
        })
        .is_ok());
    }
}
