//! HTTP-level tests for the headless server: authentication, host/origin checks, command routing
//! status codes, the event stream and static serving. They call the router in-process (no sockets).

use super::{router, test_context, AppContext};
use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use axum::response::Response;
use serde_json::{json, Value};
use std::time::Duration;
use tokio_stream::StreamExt;
use tower::ServiceExt;

const TOKEN: &str = "test-token-for-unit-tests";

fn request(method: Method, uri: &str) -> axum::http::request::Builder {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(header::HOST, "localhost:8437")
}

fn authed(method: Method, uri: &str) -> axum::http::request::Builder {
    request(method, uri).header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
}

async fn send(ctx: &AppContext, builder: axum::http::request::Builder, body: Body) -> Response {
    router(ctx.clone())
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap()
}

async fn json_body(response: Response) -> Value {
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap_or_else(|_| json!(String::from_utf8_lossy(&bytes)))
}

async fn invoke(ctx: &AppContext, command: &str, args: Value) -> (StatusCode, Value) {
    let response = send(
        ctx,
        authed(Method::POST, &format!("/api/invoke/{command}"))
            .header(header::CONTENT_TYPE, "application/json"),
        Body::from(args.to_string()),
    )
    .await;
    (response.status(), json_body(response).await)
}

#[tokio::test]
async fn the_api_and_mcp_need_the_token_but_the_public_gui_files_do_not() {
    let ctx = test_context(false);
    for (method, uri) in [
        (Method::GET, "/api/info"),
        (Method::GET, "/api/events"),
        (Method::GET, "/api/nothing"),
        (Method::GET, "/mcp"),
    ] {
        let response = send(&ctx, request(method, uri), Body::empty()).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{uri}");
    }
    let response = send(
        &ctx,
        request(Method::POST, "/api/invoke/default_media_root")
            .header(header::CONTENT_TYPE, "application/json"),
        Body::from("{}"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let wrong = send(
        &ctx,
        request(Method::GET, "/api/info").header(header::AUTHORIZATION, "Bearer nope"),
        Body::empty(),
    )
    .await;
    assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);
    // The GUI files hold nothing secret; the page signs itself in with the token it is given.
    for uri in ["/", "/library", "/_app/app.js", "/__hevc/web-shim.js"] {
        let response = send(&ctx, request(Method::GET, uri), Body::empty()).await;
        assert_eq!(response.status(), StatusCode::OK, "{uri}");
        assert!(
            response.headers().get(header::SET_COOKIE).is_none(),
            "{uri} never sets a cookie"
        );
    }
}

#[tokio::test]
async fn only_the_bearer_header_authenticates_so_a_cookie_cannot_leak_the_token_to_other_ports() {
    let ctx = test_context(false);
    let bearer = send(&ctx, authed(Method::GET, "/api/info"), Body::empty()).await;
    assert_eq!(bearer.status(), StatusCode::OK);
    let info = json_body(bearer).await;
    assert_eq!(info["server"], true);
    assert_eq!(info["readOnly"], false);
    // A cookie carrying the token is not accepted at all: cookies are sent to every port on localhost.
    let cookie = send(
        &ctx,
        request(Method::GET, "/api/info").header(header::COOKIE, format!("hevc_session={TOKEN}")),
        Body::empty(),
    )
    .await;
    assert_eq!(cookie.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn the_login_link_is_handled_in_the_browser_and_the_server_never_sees_or_stores_the_token() {
    let ctx = test_context(false);
    // The token travels in the URL fragment, which browsers never send. A query token is only a fallback
    // the page reads itself; the server ignores it and sets nothing.
    for uri in [format!("/?token={TOKEN}"), "/?token=wrong".to_string()] {
        let response = send(&ctx, request(Method::GET, &uri), Body::empty()).await;
        assert_eq!(response.status(), StatusCode::OK, "{uri}");
        assert!(response.headers().get(header::SET_COOKIE).is_none());
        assert!(response.headers().get(header::LOCATION).is_none());
    }
    let elsewhere = send(
        &ctx,
        request(Method::GET, &format!("/api/info?token={TOKEN}")),
        Body::empty(),
    )
    .await;
    assert_eq!(
        elsewhere.status(),
        StatusCode::UNAUTHORIZED,
        "a query token never authenticates an API call"
    );
}

#[tokio::test]
async fn a_foreign_host_header_is_refused_for_public_files_too() {
    let ctx = test_context(false);
    for uri in ["/", "/_app/app.js"] {
        let mut builder = request(Method::GET, uri);
        builder
            .headers_mut()
            .unwrap()
            .insert(header::HOST, "attacker.example:8437".parse().unwrap());
        assert_eq!(
            send(&ctx, builder, Body::empty()).await.status(),
            StatusCode::FORBIDDEN,
            "{uri}"
        );
    }
}

#[tokio::test]
async fn a_foreign_host_header_is_refused_even_with_a_valid_token() {
    let ctx = test_context(false);
    let mut builder = authed(Method::GET, "/api/info");
    builder
        .headers_mut()
        .unwrap()
        .insert(header::HOST, "attacker.example:8437".parse().unwrap());
    let rebound = send(&ctx, builder, Body::empty()).await;
    assert_eq!(rebound.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn an_allowed_host_is_accepted() {
    let mut ctx = test_context(false);
    ctx.allowed_hosts = std::sync::Arc::new(vec!["studio.example.com".to_string()]);
    let mut builder = authed(Method::GET, "/api/info");
    builder
        .headers_mut()
        .unwrap()
        .insert(header::HOST, "studio.example.com".parse().unwrap());
    let response = send(&ctx, builder, Body::empty()).await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn state_changing_requests_must_be_json_and_same_origin() {
    let ctx = test_context(false);
    let form = send(
        &ctx,
        authed(Method::POST, "/api/invoke/default_media_root")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded"),
        Body::from("a=b"),
    )
    .await;
    assert_eq!(form.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    let cross = send(
        &ctx,
        authed(Method::POST, "/api/invoke/default_media_root")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ORIGIN, "http://evil.example"),
        Body::from("{}"),
    )
    .await;
    assert_eq!(cross.status(), StatusCode::FORBIDDEN);
    let same = send(
        &ctx,
        authed(Method::POST, "/api/invoke/default_media_root")
            .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
            .header(header::ORIGIN, "http://localhost:8437"),
        Body::from("{}"),
    )
    .await;
    assert_eq!(same.status(), StatusCode::OK);
}

#[tokio::test]
async fn command_outcomes_map_to_distinct_status_codes() {
    let ctx = test_context(false);
    let (status, body) = invoke(&ctx, "default_media_root", json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.is_string());
    assert_eq!(
        invoke(&ctx, "format_the_disk", json!({})).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        invoke(&ctx, "reveal_tmdb_read_token", json!({})).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        invoke(&ctx, "open_file_location", json!({"path": "/"}))
            .await
            .0,
        StatusCode::NOT_IMPLEMENTED
    );
    assert_eq!(
        invoke(&ctx, "load_settings", json!({})).await.0,
        StatusCode::BAD_REQUEST
    );
    let (status, body) = invoke(&ctx, "cancel_scan", json!({})).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"], "There is no active scan.");
}

#[tokio::test]
async fn malformed_json_is_a_bad_request() {
    let ctx = test_context(false);
    let response = send(
        &ctx,
        authed(Method::POST, "/api/invoke/load_settings")
            .header(header::CONTENT_TYPE, "application/json"),
        Body::from("{not json"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn read_only_server_serves_reads_and_refuses_writes() {
    let ctx = test_context(true);
    assert_eq!(
        invoke(&ctx, "default_media_root", json!({})).await.0,
        StatusCode::OK
    );
    let (status, body) = invoke(&ctx, "save_app_preferences", json!({"preferences": {}})).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(body["error"].as_str().unwrap().contains("read-only"));
    let info = json_body(send(&ctx, authed(Method::GET, "/api/info"), Body::empty()).await).await;
    assert_eq!(info["readOnly"], true);
}

#[tokio::test]
async fn gui_pages_carry_the_shim_and_a_strict_csp() {
    let ctx = test_context(false);
    for path in ["/", "/library"] {
        let response = send(&ctx, authed(Method::GET, path), Body::empty()).await;
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        let policy = response.headers()[header::CONTENT_SECURITY_POLICY]
            .to_str()
            .unwrap()
            .to_string();
        assert!(policy.contains("connect-src 'self'") && policy.contains("'sha256-"));
        assert_eq!(
            response.headers()[header::X_CONTENT_TYPE_OPTIONS],
            "nosniff"
        );
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .unwrap();
        let page = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(page.contains("/__hevc/web-shim.js") && !page.contains("Content-Security-Policy"));
    }
}

#[tokio::test]
async fn static_files_the_shim_and_missing_paths() {
    let ctx = test_context(false);
    let script = send(&ctx, authed(Method::GET, "/_app/app.js"), Body::empty()).await;
    assert_eq!(script.status(), StatusCode::OK);
    assert_eq!(
        script.headers()[header::CONTENT_TYPE],
        "text/javascript; charset=utf-8"
    );
    let shim = send(
        &ctx,
        authed(Method::GET, "/__hevc/web-shim.js"),
        Body::empty(),
    )
    .await;
    assert_eq!(shim.status(), StatusCode::OK);
    let text = String::from_utf8(
        axum::body::to_bytes(shim.into_body(), 1 << 20)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(text.contains("__TAURI_INTERNALS__"));
    assert_eq!(
        send(&ctx, authed(Method::GET, "/missing.png"), Body::empty())
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        send(&ctx, authed(Method::GET, "/api/nothing"), Body::empty())
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
}

async fn next_frame(
    stream: &mut (impl tokio_stream::Stream<Item = Result<axum::body::Bytes, axum::Error>> + Unpin),
) -> String {
    let chunk = tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .expect("an event frame")
        .expect("stream open")
        .expect("frame");
    String::from_utf8(chunk.to_vec()).unwrap()
}

#[tokio::test]
async fn events_stream_delivers_published_events_to_every_listener() {
    let ctx = test_context(false);
    let first = send(&ctx, authed(Method::GET, "/api/events"), Body::empty()).await;
    let second = send(&ctx, authed(Method::GET, "/api/events"), Body::empty()).await;
    assert_eq!(first.status(), StatusCode::OK);
    assert_eq!(first.headers()[header::CONTENT_TYPE], "text/event-stream");
    ctx.bus
        .publish("run-line", json!({"runId": "job-1", "line": "frame=1"}));
    for response in [first, second] {
        let mut stream = response.into_body().into_data_stream();
        let frame = next_frame(&mut stream).await;
        assert!(
            frame.contains("data: ")
                && frame.contains(r#""event":"run-line""#)
                && frame.contains("frame=1"),
            "{frame}"
        );
        assert!(frame.contains("id: 1"), "{frame}");
    }
}

#[tokio::test]
async fn a_tab_that_opens_during_a_run_is_told_about_it_and_gets_its_recent_output() {
    let ctx = test_context(false);
    let mut job = crate::new_active_job("run-9".into(), None);
    job.kind = "media_run".into();
    crate::claim_job(&ctx.run_state, job).unwrap();
    ctx.bus.publish(
        "run-line",
        json!({"runId": "run-9", "line": "Checking a.mkv"}),
    );
    ctx.bus.publish(
        "run-progress",
        json!({"runId": "run-9", "filePercent": 40.0}),
    );
    let response = send(&ctx, authed(Method::GET, "/api/events"), Body::empty()).await;
    let mut stream = response.into_body().into_data_stream();
    let mut seen = String::new();
    while !(seen.contains("Checking a.mkv") && seen.contains("filePercent")) {
        seen.push_str(&next_frame(&mut stream).await);
    }
    let started = seen.find("job-started").expect("job-started is sent first");
    assert!(started < seen.find("Checking a.mkv").unwrap(), "{seen}");
    assert!(seen.contains(r#""kind":"media_run""#), "{seen}");
}

#[tokio::test]
async fn an_idle_server_sends_a_new_tab_nothing_to_replay() {
    let ctx = test_context(false);
    ctx.bus
        .publish("run-line", json!({"runId": "old", "line": "stale"}));
    let response = send(&ctx, authed(Method::GET, "/api/events"), Body::empty()).await;
    let mut stream = response.into_body().into_data_stream();
    ctx.bus
        .publish("run-line", json!({"runId": "new", "line": "fresh"}));
    let frame = next_frame(&mut stream).await;
    assert!(
        frame.contains("fresh") && !frame.contains("stale"),
        "{frame}"
    );
}

#[tokio::test]
async fn reconnecting_with_last_event_id_replays_what_was_missed() {
    let ctx = test_context(false);
    let one = ctx.bus.publish("run-line", json!({"line": "one"}));
    ctx.bus.publish("run-finished", json!({"runId": "x"}));
    let response = send(
        &ctx,
        authed(Method::GET, "/api/events").header("last-event-id", one.to_string()),
        Body::empty(),
    )
    .await;
    let mut stream = response.into_body().into_data_stream();
    let frame = next_frame(&mut stream).await;
    assert!(
        frame.contains("run-finished") && !frame.contains("one"),
        "{frame}"
    );
}

#[tokio::test]
async fn a_path_outside_the_root_is_a_403() {
    let base = crate::test_dir("web-scope-http");
    std::fs::create_dir_all(base.join("media")).unwrap();
    let mut ctx = test_context(false);
    ctx.scope = super::scope::PathScope::new(&[base.join("media")]).unwrap();
    std::fs::create_dir_all(base.join("elsewhere")).unwrap();
    let outside = base.join("elsewhere");
    let (status, body) = invoke(
        &ctx,
        "list_directory",
        json!({"path": outside.to_str().unwrap()}),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(body["error"].as_str().unwrap().contains("--root"));
    let (status, _) = invoke(&ctx, "list_directory", json!({})).await;
    assert_eq!(status, StatusCode::OK);
    std::fs::remove_dir_all(base).ok();
}

#[tokio::test]
async fn too_many_open_event_streams_are_refused_and_a_closed_one_frees_its_place() {
    let mut ctx = test_context(false);
    ctx.bus = super::events::EventBus::with_listener_limit(2);
    let first = send(&ctx, authed(Method::GET, "/api/events"), Body::empty()).await;
    let _second = send(&ctx, authed(Method::GET, "/api/events"), Body::empty()).await;
    assert_eq!(first.status(), StatusCode::OK);
    let third = send(&ctx, authed(Method::GET, "/api/events"), Body::empty()).await;
    assert_eq!(third.status(), StatusCode::SERVICE_UNAVAILABLE);
    drop(first);
    let again = send(&ctx, authed(Method::GET, "/api/events"), Body::empty()).await;
    assert_eq!(
        again.status(),
        StatusCode::OK,
        "closing a stream frees its place"
    );
}
