//! The MCP endpoint (`/mcp`, streamable HTTP) inside the web server: same token as the rest of the API,
//! refused in read-only mode, and sharing the job slot and event bus with the browser.

use super::{router, test_context, AppContext};
use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use axum::response::Response;
use axum::Router;
use serde_json::{json, Value};
use std::time::Duration;
use tower::ServiceExt;

const TOKEN: &str = "test-token-for-unit-tests";

fn post(session: Option<&str>, body: Value) -> Request<Body> {
    let mut builder = Request::builder()
        .method(Method::POST)
        .uri("/mcp")
        .header(header::HOST, "localhost:8437")
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, "application/json, text/event-stream");
    if let Some(session) = session {
        builder = builder.header("mcp-session-id", session);
    }
    builder.body(Body::from(body.to_string())).unwrap()
}

async fn text(response: Response) -> String {
    String::from_utf8(
        axum::body::to_bytes(response.into_body(), 1 << 22)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap()
}

/// The JSON-RPC message in a reply, whether the server answered with plain JSON or one SSE frame.
fn rpc(body: &str) -> Value {
    let payload = body
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(str::trim)
        .find(|data| data.starts_with('{'))
        .unwrap_or(body.trim());
    serde_json::from_str(payload).unwrap_or_else(|error| panic!("not JSON-RPC ({error}): {body}"))
}

async fn initialize(app: &Router) -> String {
    let response = app
        .clone()
        .oneshot(post(
            None,
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test-agent", "version": "0"}}}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let session = response
        .headers()
        .get("mcp-session-id")
        .expect("a session id")
        .to_str()
        .unwrap()
        .to_string();
    let reply = rpc(&text(response).await);
    assert_eq!(
        reply["result"]["serverInfo"]["name"], "hevc-rebuild-studio",
        "{reply}"
    );
    let ready = app
        .clone()
        .oneshot(post(
            Some(&session),
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        ))
        .await
        .unwrap();
    assert!(ready.status().is_success(), "{}", ready.status());
    session
}

async fn call_tool(app: &Router, session: &str, id: u64, name: &str, arguments: Value) -> Value {
    let response = app
        .clone()
        .oneshot(post(Some(session), json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": name, "arguments": arguments}})))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    rpc(&text(response).await)
}

fn app(ctx: &AppContext) -> Router {
    router(ctx.clone())
}

#[tokio::test]
async fn the_mcp_endpoint_needs_the_same_token_as_the_api() {
    let ctx = test_context(false);
    let mut request = post(
        None,
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize"}),
    );
    request.headers_mut().remove(header::AUTHORIZATION);
    let response = app(&ctx).oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let mut wrong = post(None, json!({}));
    wrong
        .headers_mut()
        .insert(header::AUTHORIZATION, "Bearer nope".parse().unwrap());
    assert_eq!(
        app(&ctx).oneshot(wrong).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn read_only_servers_do_not_offer_mcp() {
    let ctx = test_context(true);
    let response = app(&ctx)
        .oneshot(post(
            None,
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize"}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(text(response).await.contains("read-only"));
}

#[tokio::test]
async fn an_agent_can_initialize_list_tools_and_call_a_read_only_tool() {
    let ctx = test_context(false);
    let app = app(&ctx);
    let session = initialize(&app).await;
    let listed = app
        .clone()
        .oneshot(post(
            Some(&session),
            json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
        ))
        .await
        .unwrap();
    let tools = rpc(&text(listed).await);
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    for expected in [
        "inspect_environment",
        "start_integrity_check",
        "get_job",
        "list_jobs",
        "plan_system_setup",
    ] {
        assert!(
            names.contains(&expected),
            "missing tool {expected}: {names:?}"
        );
    }
    let jobs = call_tool(&app, &session, 3, "list_jobs", json!({})).await;
    assert!(jobs["result"].is_object(), "{jobs}");
}

#[tokio::test]
async fn the_headless_plan_tool_reports_the_server_privilege_route() {
    let ctx = test_context(false);
    let app = app(&ctx);
    let session = initialize(&app).await;
    let reply = call_tool(&app, &session, 2, "plan_system_setup", json!({})).await;
    // The plan depends on this machine's packages; the route label does not.
    let content = reply["result"]["structuredContent"].clone();
    if content.is_object() {
        assert_eq!(content["elevation"], "none");
        assert_eq!(content["password_prompts"], 0);
    } else {
        assert!(
            reply["result"]["isError"] == true || reply["error"].is_object(),
            "{reply}"
        );
    }
}

/// An agent starts a job over MCP; the browser's view of it (the bus) gets the desktop events, the shared
/// slot is held while it runs, and the GUI's `active_job` sees it.
#[test]
fn a_job_an_agent_starts_over_mcp_is_visible_on_the_shared_bus_and_slot() {
    let _state = crate::TEST_STATE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let state_dir = crate::test_dir("web-mcp-state");
    std::env::set_var("HEVC_REBUILD_PORTABLE_DATA_DIR", &state_dir);
    let media = crate::test_dir("web-mcp-media");
    let broken = media.join("broken.mkv");
    std::fs::write(&broken, b"not really matroska").unwrap();
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let ctx = test_context(false);
            ctx.mcp.start_event_bridge();
            let (_, mut receiver) = ctx.bus.subscribe_since(None);
            let app = app(&ctx);
            let session = initialize(&app).await;
            let started = call_tool(
                &app,
                &session,
                2,
                "start_integrity_check",
                json!({"files": [broken.to_str().unwrap()], "level": "quick"}),
            )
            .await;
            let job_id = started["result"]["structuredContent"]["job_id"]
                .as_str()
                .unwrap_or_else(|| panic!("no job id: {started}"))
                .to_string();

            let mut seen = Vec::new();
            let finished = loop {
                let event = tokio::time::timeout(Duration::from_secs(90), receiver.recv())
                    .await
                    .expect("events arrive")
                    .expect("bus open");
                let name = event.event.clone();
                let done = name == "run-finished";
                seen.push(event.clone());
                if done {
                    break event;
                }
            };
            assert_eq!(finished.payload["runId"], job_id.as_str());
            let started_event = seen
                .iter()
                .find(|event| event.event == "job-started")
                .expect("job-started precedes the run events");
            assert_eq!(
                started_event.payload,
                json!({"runId": job_id, "kind": "integrity_check"})
            );
            assert!(
                seen.iter()
                    .any(|event| event.event == "run-line"
                        && event.payload["runId"] == job_id.as_str()),
                "the run's lines reach the browser"
            );

            for _ in 0..50 {
                if crate::active_job_info(&ctx.run_state).is_none() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            assert_eq!(
                crate::active_job_info(&ctx.run_state),
                None,
                "the slot is free once the job ends"
            );
        });
    std::env::remove_var("HEVC_REBUILD_PORTABLE_DATA_DIR");
    std::fs::remove_dir_all(state_dir).ok();
    std::fs::remove_dir_all(media).ok();
}

#[tokio::test]
async fn while_the_browser_runs_a_job_an_agents_job_is_refused() {
    let ctx = test_context(false);
    crate::claim_job(
        &ctx.run_state,
        crate::new_active_job("browser-run".into(), None),
    )
    .unwrap();
    let app = app(&ctx);
    let session = initialize(&app).await;
    let media = crate::test_dir("web-mcp-busy");
    let clip = media.join("clip.mkv");
    std::fs::write(&clip, b"x").unwrap();
    let reply = call_tool(
        &app,
        &session,
        2,
        "start_integrity_check",
        json!({"files": [clip.to_str().unwrap()]}),
    )
    .await;
    let message = reply.to_string();
    assert!(message.contains("already active"), "{message}");
    std::fs::remove_dir_all(media).ok();
}

#[tokio::test]
async fn mcp_is_not_offered_with_a_root_unless_the_operator_asks_for_it() {
    let base = crate::test_dir("web-scope-mcp");
    let mut ctx = test_context(false);
    ctx.scope = super::scope::PathScope::new(std::slice::from_ref(&base)).unwrap();
    let response = app(&ctx)
        .oneshot(post(
            None,
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize"}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let body = text(response).await;
    assert!(
        body.contains("--root") && body.contains("--allow-mcp-with-root"),
        "{body}"
    );
    ctx.mcp_with_root = true;
    let session = initialize(&app(&ctx)).await;
    assert!(
        !session.is_empty(),
        "offered when the operator accepts that MCP paths are not confined"
    );
    std::fs::remove_dir_all(base).ok();
}
