//! Headless server mode: `hevc-rebuild-studio serve` serves the built GUI over HTTP and exposes the
//! desktop's commands (`POST /api/invoke/{command}`) and events (`GET /api/events`, Server-Sent
//! Events) to a browser, without a desktop session. Design and safety model:
//! docs/maintainer/SERVE_MODE.md. Behaviour lives in the submodules; this file is the router.

mod assets;
mod auth;
mod commands;
mod config;
mod events;
mod fs_browse;
#[cfg(test)]
mod mcp_tests;
#[cfg(test)]
mod router_tests;

use crate::mcp_server::McpServer;
use crate::{Events, RunState};
use assets::AssetSource;
use auth::Token;
use axum::body::Bytes;
use axum::extract::{Path as UrlPath, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use commands::CommandError;
use events::EventBus;
use serde_json::{json, Value};
use std::convert::Infallible;
use std::sync::Arc;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

pub use config::serve_mode;

/// Everything a request handler shares. The `run_state` is the same single-job registry the desktop
/// window uses; the job runners publish to `events` (the bus behind the SSE stream), and stage 3 attaches
/// the MCP endpoint to the same two.
#[derive(Clone)]
pub(crate) struct AppContext {
    pub(crate) run_state: RunState,
    pub(crate) bus: EventBus,
    /// What job runners emit into: the same bus the SSE stream reads, so every viewer sees every job.
    pub(crate) events: Events,
    /// The MCP server behind `/mcp`: one instance for every session, sharing `run_state` and `events`.
    pub(crate) mcp: McpServer,
    /// How the privileged setup step is authorised on this host; probed per call, replaceable in tests.
    pub(crate) privilege_route: PrivilegeProbe,
    pub(crate) read_only: bool,
    pub(crate) token: Arc<Token>,
    pub(crate) allowed_hosts: Arc<Vec<String>>,
    pub(crate) assets: Arc<AssetSource>,
}

pub(crate) type PrivilegeProbe = Arc<dyn Fn() -> crate::provision::HeadlessRoute + Send + Sync>;

/// The MCP server for a context: its jobs take the same slot and publish to the same bus as the browser's.
pub(crate) fn mcp_for(
    run_state: &RunState,
    events: &Events,
    privilege_route: &PrivilegeProbe,
) -> McpServer {
    let probe = privilege_route.clone();
    McpServer::new_shared(crate::mcp_server::shared::Shared {
        run_state: run_state.clone(),
        events: events.clone(),
        privilege: Arc::new(move || crate::provision::Privilege::Headless(probe())),
    })
}

/// An `Events` sink that publishes into the server's bus.
pub(crate) fn bus_events(bus: &EventBus) -> Events {
    let bus = bus.clone();
    Events::from_fn(move |event, payload| {
        bus.publish(event, payload);
    })
}

pub(crate) fn info_json(ctx: &AppContext) -> Value {
    json!({
        "name": "HEVC Rebuild Studio",
        "version": env!("CARGO_PKG_VERSION"),
        "readOnly": ctx.read_only,
        "server": true,
    })
}

#[cfg(test)]
pub(crate) fn test_context(read_only: bool) -> AppContext {
    let mut files = std::collections::HashMap::new();
    files.insert(
        "index.html".to_string(),
        br#"<!doctype html><html><head><meta http-equiv="Content-Security-Policy" content="default-src 'self'"></head><body><script>window.boot = 1;</script></body></html>"#.to_vec(),
    );
    files.insert("_app/app.js".to_string(), b"export {};".to_vec());
    let bus = EventBus::default();
    let run_state = RunState::default();
    let events = bus_events(&bus);
    let privilege_route: PrivilegeProbe = Arc::new(|| crate::provision::HeadlessRoute::Unavailable);
    AppContext {
        mcp: mcp_for(&run_state, &events, &privilege_route),
        run_state,
        events,
        bus,
        privilege_route,
        read_only,
        token: Arc::new(Token::from_secret("test-token-for-unit-tests").expect("test token")),
        allowed_hosts: Arc::new(Vec::new()),
        assets: Arc::new(AssetSource::Memory(files)),
    }
}

/// Streamable-HTTP MCP at `/mcp`. Hosts the server will answer for follow `--allowed-host`; read-only
/// servers do not offer MCP at all (its tools start runs, renames and setup).
fn mcp_routes(router: Router<AppContext>, ctx: &AppContext) -> Router<AppContext> {
    if ctx.read_only {
        return router.route(
            "/mcp",
            axum::routing::any(|| async {
                plain(
                    StatusCode::FORBIDDEN,
                    "The server is read-only; the MCP endpoint is not offered.",
                )
            }),
        );
    }
    use rmcp::transport::streamable_http_server::{
        session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
    };
    let mut hosts = vec![
        "localhost".to_string(),
        "127.0.0.1".to_string(),
        "::1".to_string(),
    ];
    hosts.extend(ctx.allowed_hosts.iter().cloned());
    let server = ctx.mcp.clone();
    let service = StreamableHttpService::new(
        move || Ok(server.clone()),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default()
            .with_allowed_hosts(hosts)
            .with_json_response(true),
    );
    router.nest_service("/mcp", service)
}

pub(crate) fn router(ctx: AppContext) -> Router {
    mcp_routes(Router::<AppContext>::new(), &ctx)
        .route("/api/info", get(info))
        .route("/api/invoke/{command}", post(invoke))
        .route("/api/events", get(events_stream))
        .route(assets::SHIM_PATH, get(shim))
        .fallback(get(static_file))
        .layer(middleware::from_fn_with_state(ctx.clone(), guard))
        .with_state(ctx)
}

fn plain(status: StatusCode, message: &str) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        message.to_string(),
    )
        .into_response()
}

fn header_text(headers: &HeaderMap, name: header::HeaderName) -> Option<&str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

fn is_json_request(headers: &HeaderMap) -> bool {
    header_text(headers, header::CONTENT_TYPE)
        .map(|value| {
            value
                .split(';')
                .next()
                .unwrap_or("")
                .trim()
                .eq_ignore_ascii_case("application/json")
        })
        .unwrap_or(false)
}

fn token_in_query(query: &str) -> Option<String> {
    query
        .split('&')
        .find_map(|pair| pair.strip_prefix("token="))
        .map(str::to_string)
}

/// Host check, authentication, cross-site checks and security headers for every request.
async fn guard(State(ctx): State<AppContext>, request: Request, next: Next) -> Response {
    let headers = request.headers();
    let host = header_text(headers, header::HOST).unwrap_or("");
    if !auth::host_allowed(host, &ctx.allowed_hosts) {
        return plain(
            StatusCode::FORBIDDEN,
            "This host name is not allowed. Start the server with --allowed-host for it.",
        );
    }
    let secure = header_text(
        headers,
        header::HeaderName::from_static("x-forwarded-proto"),
    ) == Some("https");

    // A login link: verify the token, set the session cookie, and drop the token from the address bar.
    if request.method() == Method::GET && request.uri().path() == "/" {
        if let Some(candidate) = request.uri().query().and_then(token_in_query) {
            if !ctx.token.verify(&candidate) {
                return plain(StatusCode::UNAUTHORIZED, "That token is not valid.");
            }
            let mut response = Response::builder()
                .status(StatusCode::SEE_OTHER)
                .header(header::LOCATION, "/")
                .header(header::CACHE_CONTROL, "no-store")
                .header(header::REFERRER_POLICY, "no-referrer")
                .body(axum::body::Body::empty())
                .expect("redirect");
            if let Ok(cookie) = HeaderValue::from_str(&auth::session_cookie(&ctx.token, secure)) {
                response.headers_mut().insert(header::SET_COOKIE, cookie);
            }
            return response;
        }
    }

    let authorized = auth::credential_from_headers(headers)
        .is_some_and(|candidate| ctx.token.verify(&candidate));
    if !authorized {
        return plain(
            StatusCode::UNAUTHORIZED,
            "Authentication required. Open the login link printed when the server started, or send `Authorization: Bearer <token>`.",
        );
    }

    if !matches!(*request.method(), Method::GET | Method::HEAD) {
        if !is_json_request(headers) {
            return plain(StatusCode::UNSUPPORTED_MEDIA_TYPE, "Send application/json.");
        }
        if let Some(origin) = header_text(headers, header::ORIGIN) {
            if !auth::origin_matches_host(origin, host) {
                return plain(StatusCode::FORBIDDEN, "Cross-site request refused.");
            }
        }
    }

    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    response
}

async fn info(State(ctx): State<AppContext>) -> Json<Value> {
    Json(info_json(&ctx))
}

fn command_response(result: Result<Value, CommandError>) -> Response {
    match result {
        Ok(value) => (StatusCode::OK, Json(value)).into_response(),
        Err(error) => {
            let status = match &error {
                CommandError::Unknown(_) => StatusCode::NOT_FOUND,
                CommandError::Forbidden(_) => StatusCode::FORBIDDEN,
                CommandError::Unavailable(_) => StatusCode::NOT_IMPLEMENTED,
                CommandError::BadArguments(_) => StatusCode::BAD_REQUEST,
                CommandError::Failed(_) => StatusCode::UNPROCESSABLE_ENTITY,
            };
            (status, Json(commands::error_body(&error))).into_response()
        }
    }
}

async fn invoke(
    State(ctx): State<AppContext>,
    UrlPath(command): UrlPath<String>,
    body: Bytes,
) -> Response {
    let args: Value = if body.is_empty() {
        json!({})
    } else {
        match serde_json::from_slice(&body) {
            Ok(value) => value,
            Err(error) => {
                return command_response(Err(CommandError::BadArguments(format!(
                    "The request body is not valid JSON: {error}"
                ))))
            }
        }
    };
    command_response(commands::dispatch(&ctx, &command, args).await)
}

fn sse_event(event: &events::ServerEvent) -> Event {
    Event::default()
        .id(event.id.to_string())
        .data(json!({ "event": event.event, "payload": event.payload }).to_string())
}

async fn events_stream(State(ctx): State<AppContext>, headers: HeaderMap) -> Response {
    let last_id = header_text(&headers, header::HeaderName::from_static("last-event-id"))
        .and_then(|value| value.parse::<u64>().ok());
    // A reconnecting window resumes from its last id. A new window catches up on the active run instead.
    let (replay, receiver): (Vec<Event>, _) = match last_id {
        Some(_) => {
            let (missed, receiver) = ctx.bus.subscribe_since(last_id);
            (missed.iter().map(sse_event).collect(), receiver)
        }
        None => {
            let active = crate::active_job_info(&ctx.run_state).map(|job| (job.id, job.kind));
            let (events, receiver) = ctx.bus.subscribe_active(active);
            (
                events
                    .into_iter()
                    .map(|(name, payload)| {
                        Event::default()
                            .data(json!({ "event": name, "payload": payload }).to_string())
                    })
                    .collect(),
                receiver,
            )
        }
    };
    let replay = tokio_stream::iter(replay.into_iter().map(Ok::<Event, Infallible>));
    let live = BroadcastStream::new(receiver).filter_map(|item| {
        item.ok()
            .map(|event| Ok::<Event, Infallible>(sse_event(&event)))
    });
    let mut response = Sse::new(replay.chain(live))
        .keep_alive(KeepAlive::default())
        .into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

async fn shim() -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        assets::SHIM_SOURCE,
    )
        .into_response()
}

async fn static_file(State(ctx): State<AppContext>, request: Request) -> Response {
    let path = request.uri().path().to_string();
    if path.starts_with("/api/") {
        return plain(StatusCode::NOT_FOUND, "No such API route.");
    }
    let Some(asset) = ctx.assets.resolve(&path) else {
        return plain(StatusCode::NOT_FOUND, "Not found.");
    };
    let is_page = asset.content_type.starts_with("text/html");
    let mut response = (
        [(header::CONTENT_TYPE, asset.content_type)],
        asset.bytes.clone(),
    )
        .into_response();
    let headers = response.headers_mut();
    if is_page {
        let hashes = assets::inline_script_hashes(&String::from_utf8_lossy(&asset.bytes));
        if let Ok(value) = HeaderValue::from_str(&assets::content_security_policy(&hashes)) {
            headers.insert(header::CONTENT_SECURITY_POLICY, value);
        }
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    } else {
        headers.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("private, max-age=3600"),
        );
    }
    response
}
