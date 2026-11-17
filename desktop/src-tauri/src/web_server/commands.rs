//! Command routing for the headless server: maps a Tauri command name and its JSON arguments onto the
//! same `pub(crate)` functions the desktop window invokes. Every served command is classified `Read`
//! or `Write` here, so `--read-only` is enforced in one place; commands that need the desktop, carry
//! secrets, or are not wired to the event bus yet are listed with the reason they are refused.

use super::fs_browse;
use super::AppContext;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Access {
    Read,
    Write,
}

#[derive(Debug, PartialEq)]
pub(crate) enum CommandError {
    /// No such command (404).
    Unknown(String),
    /// Refused by server policy: read-only mode or a secret-bearing command (403).
    Forbidden(String),
    /// Valid command, but not available in server mode (501).
    Unavailable(String),
    /// Arguments did not match the command's parameters (400).
    BadArguments(String),
    /// The command ran and returned its error string (422).
    Failed(String),
}

impl CommandError {
    pub(crate) fn message(&self) -> &str {
        match self {
            CommandError::Unknown(message)
            | CommandError::Forbidden(message)
            | CommandError::Unavailable(message)
            | CommandError::BadArguments(message)
            | CommandError::Failed(message) => message,
        }
    }
}

/// Accepted but inert: these only make sense next to a native window.
const NO_OPERATION: &[&str] = &["frontend_ready", "reply"];

/// Refused in server mode, with the reason shown to the caller.
const REFUSED: &[(&str, &str)] = &[
    ("reveal_tmdb_read_token", SECRET_REASON),
    ("save_tmdb_read_token", SECRET_REASON),
    ("remove_tmdb_read_token", SECRET_REASON),
    ("reveal_tvdb_credentials", SECRET_REASON),
    ("save_tvdb_credentials", SECRET_REASON),
    ("remove_tvdb_credentials", SECRET_REASON),
    (
        "open_file_location",
        "There is no desktop file manager on the server; use the path shown instead.",
    ),
];

const SECRET_REASON: &str = "Credentials are never sent over the server API. Set them with the desktop app or the server's environment variables.";

fn camel_case(snake: &str) -> String {
    let mut out = String::with_capacity(snake.len());
    let mut upper_next = false;
    for ch in snake.chars() {
        if ch == '_' {
            upper_next = true;
        } else if upper_next {
            out.extend(ch.to_uppercase());
            upper_next = false;
        } else {
            out.push(ch);
        }
    }
    out
}

/// One argument by its Rust name; Tauri's wire names are camelCase. A missing key deserializes from
/// `null`, so `Option` parameters are optional and required ones report which argument is wrong.
fn arg<T: DeserializeOwned>(args: &Value, snake: &str) -> Result<T, CommandError> {
    let key = camel_case(snake);
    let value = args.get(&key).cloned().unwrap_or(Value::Null);
    serde_json::from_value(value)
        .map_err(|error| CommandError::BadArguments(format!("Invalid argument `{key}`: {error}")))
}

fn respond<T: Serialize>(result: Result<T, String>) -> Result<Value, CommandError> {
    match result {
        Ok(value) => serde_json::to_value(value)
            .map_err(|error| CommandError::Failed(format!("Could not encode the result: {error}"))),
        Err(message) => Err(CommandError::Failed(message)),
    }
}

async fn probe_route(ctx: &AppContext) -> Result<crate::provision::HeadlessRoute, CommandError> {
    let probe = ctx.privilege_route.clone();
    crate::runtime::spawn_blocking(move || probe())
        .await
        .map_err(|error| CommandError::Failed(format!("The privilege check failed: {error}")))
}

macro_rules! run_async {
    ($args:ident, $function:path $(, $name:ident)*) => {{
        $( let $name = arg(&$args, stringify!($name))?; )*
        respond($function($($name),*).await)
    }};
}

macro_rules! run_blocking {
    ($args:ident, $function:path $(, $name:ident)*) => {{
        $( let $name = arg(&$args, stringify!($name))?; )*
        let result = crate::runtime::spawn_blocking(move || $function($($name),*))
            .await
            .map_err(|error| CommandError::Failed(format!("The command task failed: {error}")))?;
        respond(result)
    }};
}

/// The served commands: name, access class, and the call. The name list is what `access_of` reports.
macro_rules! registry {
    (|$ctx:ident, $args:ident| $( $name:literal, $access:ident => $call:expr; )*) => {
        const SERVED: &[(&str, Access)] = &[ $( ($name, Access::$access) ),* ];

        async fn run($ctx: &AppContext, name: &str, $args: Value) -> Result<Value, CommandError> {
            let _ = &$ctx;
            let _ = &$args;
            match name {
                $( $name => $call, )*
                _ => Err(CommandError::Unknown(format!("Unknown command `{name}`."))),
            }
        }
    };
}

registry! { |ctx, args|
    // Server-only helpers.
    "server_info", Read => Ok(super::info_json(ctx));
    "list_directory", Read => {
        let path: Option<String> = arg(&args, "path")?;
        let show_hidden: Option<bool> = arg(&args, "show_hidden")?;
        let scope = ctx.scope.clone();
        let result = crate::runtime::spawn_blocking(move || fs_browse::list_directory(path.as_deref(), show_hidden.unwrap_or(false), &scope))
            .await
            .map_err(|error| CommandError::Failed(format!("The command task failed: {error}")))?;
        respond(result)
    };
    // Environment, settings, preferences.
    "inspect_environment", Read => run_async!(args, crate::commands_workspace::inspect_environment, tool_paths);
    "load_settings", Read => run_async!(args, crate::commands_workspace::load_settings, root);
    "resolve_work_directory_command", Read => run_blocking!(args, crate::commands_workspace::resolve_work_directory_command, root, work_root);
    "default_media_root", Read => match ctx.scope.first_root() {
        Some(root) => Ok(json!(root.to_string_lossy())),
        None => respond(crate::commands_workspace::default_media_root()),
    };
    "load_app_preferences", Read => run_blocking!(args, crate::app_preferences::load_app_preferences);
    "save_app_preferences", Write => run_blocking!(args, crate::app_preferences::save_app_preferences, preferences);
    "check_ubuntu_package_updates", Read => run_async!(args, crate::apt_commands::check_ubuntu_package_updates);
    "provision_plan", Read => {
        let tool_paths: crate::ToolPaths = arg(&args, "tool_paths")?;
        let route = probe_route(ctx).await?;
        respond(crate::apt_commands::provision_plan_for(tool_paths, crate::provision::Privilege::Headless(route)).await)
    };
    "provision_toolchain", Write => {
        let tool_paths: crate::ToolPaths = arg(&args, "tool_paths")?;
        let user_space_only: Option<bool> = arg(&args, "user_space_only")?;
        let route = probe_route(ctx).await?;
        respond(crate::apt_commands::provision_toolchain_with(ctx.events.clone(), &ctx.run_state, tool_paths, user_space_only, crate::provision::Privilege::Headless(route)).await)
    };
    // Metadata search and credential status (secrets themselves are refused above).
    "metadata_filename_clue", Read => run_blocking!(args, crate::commands_metadata::metadata_filename_clue, filename, patterns);
    "metadata_filename_clues", Read => run_blocking!(args, crate::commands_metadata::metadata_filename_clues, filenames, patterns);
    "search_public_films", Read => run_async!(args, crate::commands_metadata::search_public_films, query, language);
    "load_public_film_details", Read => run_async!(args, crate::commands_metadata::load_public_film_details, entity_id, language);
    "search_public_tv_shows", Read => run_async!(args, crate::commands_metadata::search_public_tv_shows, query);
    "load_public_tv_details", Read => run_async!(args, crate::commands_metadata::load_public_tv_details, show_id);
    "search_tmdb_movies", Read => run_async!(args, crate::commands_metadata::search_tmdb_movies, query, language);
    "search_tmdb_tv", Read => run_async!(args, crate::commands_metadata::search_tmdb_tv, query, language);
    "load_tmdb_movie_details", Read => run_async!(args, crate::commands_metadata::load_tmdb_movie_details, id, language);
    "load_tmdb_tv_details", Read => run_async!(args, crate::commands_metadata::load_tmdb_tv_details, id, language);
    "tmdb_tv_certification_for_tvdb", Read => run_async!(args, crate::commands_metadata::tmdb_tv_certification_for_tvdb, tvdb_id, language);
    "tmdb_credential_status", Read => run_async!(args, crate::commands_metadata::tmdb_credential_status);
    "tvdb_credential_status", Read => run_async!(args, crate::commands_metadata::tvdb_credential_status);
    "search_tvdb_titles", Read => run_async!(args, crate::commands_metadata::search_tvdb_titles, query, kind, language);
    "load_tvdb_details", Read => run_async!(args, crate::commands_metadata::load_tvdb_details, id, kind, language);
    // Library, artwork and renames.
    "save_library_artwork", Write => run_async!(args, crate::commands_library::save_library_artwork, request);
    "cache_library_artwork", Write => run_async!(args, crate::commands_library::cache_library_artwork, candidate);
    "save_local_library_artwork", Write => run_blocking!(args, crate::commands_library::save_local_library_artwork, request);
    "cache_local_library_artwork", Write => run_async!(args, crate::commands_library::cache_local_library_artwork, source_path);
    "inspect_app_artwork_cache", Read => run_async!(args, crate::commands_library::inspect_app_artwork_cache);
    "purge_unused_app_artwork_cache", Write => {
        let preview_id: String = arg(&args, "preview_id")?;
        let confirmed: bool = arg(&args, "confirmed")?;
        respond(crate::commands_library::purge_unused_app_artwork_cache_with(&ctx.run_state, preview_id, confirmed).await)
    };
    "scan_local_library_assets", Read => run_blocking!(args, crate::commands_library::scan_local_library_assets, request);
    "read_local_library_artwork_preview", Read => run_blocking!(args, crate::commands_library::read_local_library_artwork_preview, request);
    "scan_organizer_folder", Read => run_blocking!(args, crate::commands_library::scan_organizer_folder, root);
    "preview_library_rename", Read => run_blocking!(args, crate::commands_library::preview_library_rename, root, items, text_blacklist_patterns);
    "apply_library_rename", Write => run_blocking!(args, crate::commands_library::apply_library_rename, root, items, text_blacklist_patterns, preview_id, confirmed);
    // Queue.
    "load_library_queue", Read => run_async!(args, crate::commands_queue::load_library_queue);
    "save_library_queue", Write => run_async!(args, crate::commands_queue::save_library_queue, queue);
    "save_library_queue_selection", Write => run_async!(args, crate::commands_queue::save_library_queue_selection, selected_media_paths, active_root, file_tune_overrides, file_stream_overrides);
    "save_library_queue_movie_reviews", Write => run_async!(args, crate::commands_queue::save_library_queue_movie_reviews, movie_reviews);
    "save_library_queue_series_reviews", Write => run_async!(args, crate::commands_queue::save_library_queue_series_reviews, series_reviews);
    "save_library_queue_metadata_suggestions", Write => run_async!(args, crate::commands_queue::save_library_queue_metadata_suggestions, match_suggestions);
    "preview_queue_output_targets", Read => run_async!(args, crate::commands_queue::preview_queue_output_targets, request);
    "reconcile_interrupted_runs", Write => respond(crate::commands_workspace::reconcile_interrupted_runs_with(&ctx.run_state).await);
    // Reports and logs.
    "list_reports", Read => run_async!(args, crate::commands_logs::list_reports, limit);
    "delete_reports", Write => run_async!(args, crate::commands_logs::delete_reports);
    "list_tool_setup_logs", Read => run_blocking!(args, crate::commands_logs::list_tool_setup_logs);
    "read_tool_setup_log", Read => run_blocking!(args, crate::commands_logs::read_tool_setup_log, job_id);
    "export_tool_setup_log", Write => run_blocking!(args, crate::commands_logs::export_tool_setup_log, job_id, destination);
    "read_log", Read => run_blocking!(args, crate::commands_logs::read_log, path);
    // Jobs: the same runners as the desktop, publishing to the server's event bus.
    "start_run", Write => {
        let request: crate::run_commands::RunRequest = arg(&args, "request")?;
        let events = ctx.events.clone();
        let state = ctx.run_state.clone();
        let result = crate::runtime::spawn_blocking(move || crate::run_commands::start_run_with(events, &state, request))
            .await
            .map_err(|error| CommandError::Failed(format!("The command task failed: {error}")))?;
        respond(result)
    };
    "scan_folder", Write => {
        let options: crate::commands_workspace::ScanFolderOptions = arg(&args, "options")?;
        respond(crate::commands_workspace::scan_folder_with(ctx.events.clone(), &ctx.run_state, options).await)
    };
    // Job controls.
    "request_stop", Write => {
        let run_id: String = arg(&args, "run_id")?;
        respond(crate::run_commands::request_stop_with(&ctx.run_state, run_id))
    };
    "request_abort", Write => {
        let run_id: String = arg(&args, "run_id")?;
        respond(crate::run_commands::request_abort_with(&ctx.run_state, run_id))
    };
    "active_job", Read => respond(Ok::<_, String>(crate::active_job_info(&ctx.run_state)));
    "cancel_scan", Write => respond(crate::commands_workspace::cancel_scan_with(&ctx.run_state));
}

pub(crate) fn access_of(name: &str) -> Option<Access> {
    SERVED
        .iter()
        .find(|(served, _)| *served == name)
        .map(|(_, access)| *access)
}

/// Every command name the server knows, in any state (served, refused, inert).
#[cfg(test)]
pub(crate) fn known_commands() -> Vec<&'static str> {
    SERVED
        .iter()
        .map(|(name, _)| *name)
        .chain(REFUSED.iter().map(|(name, _)| *name))
        .chain(NO_OPERATION.iter().copied())
        .collect()
}

pub(crate) async fn dispatch(
    ctx: &AppContext,
    name: &str,
    args: Value,
) -> Result<Value, CommandError> {
    if NO_OPERATION.contains(&name) {
        return Ok(Value::Null);
    }
    if let Some((_, reason)) = REFUSED.iter().find(|(refused, _)| *refused == name) {
        return Err(if reason == &SECRET_REASON {
            CommandError::Forbidden((*reason).into())
        } else {
            CommandError::Unavailable((*reason).into())
        });
    }
    let access = access_of(name)
        .ok_or_else(|| CommandError::Unknown(format!("Unknown command `{name}`.")))?;
    if access == Access::Write && ctx.read_only {
        return Err(CommandError::Forbidden(format!(
            "The server is read-only; `{name}` changes state."
        )));
    }
    // Paths are checked before the command runs, so a refused call never touches the job slot or the disk.
    super::scope::check_command(&ctx.scope, name, &args).map_err(CommandError::Forbidden)?;
    run(ctx, name, args).await
}

/// Convenience used by the HTTP layer for the JSON error body.
pub(crate) fn error_body(error: &CommandError) -> Value {
    json!({ "error": error.message() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::web_server::test_context;

    #[test]
    fn camel_case_matches_tauri_wire_names() {
        assert_eq!(camel_case("tool_paths"), "toolPaths");
        assert_eq!(
            camel_case("text_blacklist_patterns"),
            "textBlacklistPatterns"
        );
        assert_eq!(camel_case("root"), "root");
    }

    /// Every `#[tauri::command]` registered for the desktop window must be served, refused with a
    /// reason, or inert here, so a new command cannot silently go missing from (or leak into) the server.
    #[test]
    fn every_desktop_command_is_classified_for_the_server() {
        let source = include_str!("../lib.rs");
        let start = source.find("generate_handler![").expect("handler list");
        let end = source[start..].find("])").expect("end of handler list") + start;
        let registered: Vec<&str> = source[start..end]
            .lines()
            .skip(1)
            .filter_map(|line| line.trim().trim_end_matches(',').rsplit("::").next())
            .filter(|name| !name.is_empty())
            .collect();
        assert!(
            registered.len() > 50,
            "parsed {} handlers",
            registered.len()
        );
        let known = known_commands();
        for name in registered {
            // `resolve_work_directory_command` keeps its full name on both sides.
            assert!(
                known.contains(&name),
                "command `{name}` is not classified in web_server/commands.rs"
            );
        }
    }

    #[test]
    fn served_names_are_unique() {
        let mut names = known_commands();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(before, names.len(), "a command is classified twice");
    }

    #[tokio::test]
    async fn unknown_commands_are_not_found() {
        let ctx = test_context(false);
        let error = dispatch(&ctx, "format_the_disk", json!({}))
            .await
            .unwrap_err();
        assert!(matches!(error, CommandError::Unknown(_)));
    }

    #[tokio::test]
    async fn secret_commands_are_forbidden_even_when_writable() {
        let ctx = test_context(false);
        for name in [
            "reveal_tmdb_read_token",
            "save_tmdb_read_token",
            "save_tvdb_credentials",
            "reveal_tvdb_credentials",
            "remove_tmdb_read_token",
            "remove_tvdb_credentials",
        ] {
            let error = dispatch(&ctx, name, json!({"token": "x"}))
                .await
                .unwrap_err();
            assert!(
                matches!(error, CommandError::Forbidden(_)),
                "{name}: {error:?}"
            );
        }
    }

    #[tokio::test]
    async fn desktop_only_commands_are_unavailable_not_silent() {
        let ctx = test_context(false);
        let error = dispatch(&ctx, "open_file_location", json!({"path": "/"}))
            .await
            .unwrap_err();
        assert!(matches!(error, CommandError::Unavailable(_)), "{error:?}");
        assert_eq!(
            dispatch(&ctx, "frontend_ready", json!({})).await,
            Ok(Value::Null)
        );
    }

    #[tokio::test]
    async fn job_commands_are_writes_and_validate_before_starting_anything() {
        for name in ["start_run", "scan_folder", "provision_toolchain"] {
            assert_eq!(access_of(name), Some(Access::Write), "{name}");
        }
        let ctx = test_context(false);
        let too_big = run_request("/", "check", &[], 5000);
        let error = dispatch(&ctx, "start_run", json!({ "request": too_big }))
            .await
            .unwrap_err();
        assert_eq!(
            error,
            CommandError::Failed("Minimum input size must be 4096 MiB or less.".into())
        );
        let error = dispatch(&ctx, "start_run", json!({})).await.unwrap_err();
        assert!(matches!(error, CommandError::BadArguments(_)), "{error:?}");
        assert!(
            ctx.run_state.0.lock().unwrap().is_none(),
            "a rejected request must not hold the job slot"
        );
    }

    fn run_request(root: &str, mode: &str, selected: &[&str], minimum_size_mib: u32) -> Value {
        json!({
            "root": root, "recursive": false, "minimumSizeMib": minimum_size_mib, "ffmpegPath": null,
            "mode": mode, "settings": {}, "stopOnError": false, "fullRebuildConfirmed": false,
            "selectedFiles": selected, "integrityLevel": "quick",
        })
    }

    async fn next_event_named(
        receiver: &mut tokio::sync::broadcast::Receiver<super::super::events::ServerEvent>,
        name: &str,
    ) -> super::super::events::ServerEvent {
        loop {
            let event = tokio::time::timeout(std::time::Duration::from_secs(90), receiver.recv())
                .await
                .expect("the job event arrives")
                .expect("bus open");
            if event.event == name {
                return event;
            }
        }
    }

    /// The same run engine the desktop uses, started through the server: the browser and an agent both see
    /// its events on the bus, and the shared job slot is claimed then released.
    #[test]
    fn a_run_started_through_the_server_publishes_its_events_and_releases_the_job_slot() {
        let _state = crate::TEST_STATE_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let state_dir = crate::test_dir("web-run-state");
                std::env::set_var("HEVC_REBUILD_PORTABLE_DATA_DIR", &state_dir);
                let media = crate::test_dir("web-run-media");
                let broken = media.join("broken.mkv");
                std::fs::write(&broken, b"not really matroska").unwrap();
                let ctx = test_context(false);
                let (_, mut receiver) = ctx.bus.subscribe_since(None);
                let request = run_request(
                    media.to_str().unwrap(),
                    "integrity",
                    &[broken.to_str().unwrap()],
                    0,
                );
                let started =
                    dispatch(&ctx, "start_run", json!({ "request": request.clone() })).await;
                let run_id = started
                    .expect("the run starts")
                    .as_str()
                    .expect("the run id")
                    .to_string();
                let busy = dispatch(&ctx, "start_run", json!({ "request": request })).await;
                let finished = next_event_named(&mut receiver, "run-finished").await;
                assert_eq!(finished.payload["runId"], run_id.as_str());
                if let Err(error) = busy {
                    assert_eq!(
                        error,
                        CommandError::Failed(
                            "Another run, scan, or repair is already active.".into()
                        )
                    );
                }
                for _ in 0..50 {
                    if ctx.run_state.0.lock().unwrap().is_none() {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
                assert!(
                    ctx.run_state.0.lock().unwrap().is_none(),
                    "the job slot is released when the run finishes"
                );
                std::env::remove_var("HEVC_REBUILD_PORTABLE_DATA_DIR");
                std::fs::remove_dir_all(state_dir).ok();
                std::fs::remove_dir_all(media).ok();
            });
    }

    #[test]
    fn a_folder_scan_through_the_server_returns_its_result_and_releases_the_job_slot() {
        let _state = crate::TEST_STATE_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
        let state_dir = crate::test_dir("web-scan-state");
        std::env::set_var("HEVC_REBUILD_PORTABLE_DATA_DIR", &state_dir);
        let media = crate::test_dir("web-scan-media");
        let ctx = test_context(false);
        let options = json!({"root": media.to_str().unwrap(), "recursive": false, "minimumSizeMib": 0, "filenameIncludePatterns": [], "filenameExcludePatterns": []});
        let scanned = dispatch(&ctx, "scan_folder", json!({ "options": options }))
            .await
            .expect("the scan completes");
        assert!(scanned.is_object() || scanned.is_array(), "{scanned}");
        assert!(ctx.run_state.0.lock().unwrap().is_none());
        std::env::remove_var("HEVC_REBUILD_PORTABLE_DATA_DIR");
        std::fs::remove_dir_all(state_dir).ok();
        std::fs::remove_dir_all(media).ok();
        });
    }

    #[tokio::test]
    async fn the_plan_command_reports_the_probed_privilege_route() {
        let mut ctx = test_context(true);
        ctx.privilege_route = std::sync::Arc::new(|| crate::provision::HeadlessRoute::Unavailable);
        // Only the route label is asserted: the plan itself depends on this machine's packages.
        match dispatch(&ctx, "provision_plan", json!({"toolPaths": {}})).await {
            Ok(plan) => {
                assert_eq!(plan["elevation"], "none");
                assert_eq!(plan["password_prompts"], 0);
            }
            Err(error) => assert!(matches!(error, CommandError::Failed(_)), "{error:?}"),
        }
    }

    #[tokio::test]
    async fn read_only_mode_refuses_every_write_command_before_running_it() {
        let ctx = test_context(true);
        for (name, access) in SERVED {
            if *access != Access::Write {
                continue;
            }
            // Empty arguments would be a bad-arguments error if the command were reached.
            let error = dispatch(&ctx, name, json!({})).await.unwrap_err();
            assert!(
                matches!(error, CommandError::Forbidden(_)),
                "{name}: {error:?}"
            );
        }
    }

    #[tokio::test]
    async fn bad_arguments_name_the_offending_parameter() {
        let ctx = test_context(false);
        let error = dispatch(&ctx, "load_settings", json!({}))
            .await
            .unwrap_err();
        match error {
            CommandError::BadArguments(message) => assert!(message.contains("`root`"), "{message}"),
            other => panic!("{other:?}"),
        }
        let error = dispatch(&ctx, "request_stop", json!({"runId": 7}))
            .await
            .unwrap_err();
        assert!(matches!(error, CommandError::BadArguments(_)));
    }

    #[tokio::test]
    async fn job_controls_report_the_desktop_messages_when_nothing_runs() {
        let ctx = test_context(false);
        let stop = dispatch(&ctx, "request_stop", json!({"runId": "none"}))
            .await
            .unwrap_err();
        assert_eq!(stop, CommandError::Failed("There is no active run.".into()));
        let cancel = dispatch(&ctx, "cancel_scan", json!({})).await.unwrap_err();
        assert_eq!(
            cancel,
            CommandError::Failed("There is no active scan.".into())
        );
    }

    #[tokio::test]
    async fn wire_arguments_use_camel_case_and_results_serialize() {
        let ctx = test_context(false);
        let root = crate::test_dir("web-resolve");
        let value = dispatch(
            &ctx,
            "resolve_work_directory_command",
            json!({"root": root.to_str().unwrap(), "workRoot": null}),
        )
        .await
        .expect("resolve the work directory");
        assert!(value.is_object(), "{value}");
        std::fs::remove_dir_all(root).ok();
        let media = dispatch(&ctx, "default_media_root", json!({}))
            .await
            .unwrap();
        assert!(media.is_string());
    }

    #[tokio::test]
    async fn environment_check_runs_through_the_server_router() {
        let ctx = test_context(true);
        let environment = dispatch(&ctx, "inspect_environment", json!({}))
            .await
            .expect("environment check");
        assert!(environment.pointer("/os").is_some(), "{environment}");
    }

    #[tokio::test]
    async fn with_a_root_the_browser_api_refuses_paths_outside_it_before_anything_runs() {
        let base = crate::test_dir("web-scope-dispatch");
        let media = base.join("media");
        std::fs::create_dir_all(&media).unwrap();
        std::fs::create_dir_all(base.join("private")).unwrap();
        std::fs::write(base.join("private/x.mkv"), b"x").unwrap();
        let mut ctx = test_context(false);
        ctx.scope = crate::web_server::scope::PathScope::new(std::slice::from_ref(&media)).unwrap();

        let outside = dispatch(
            &ctx,
            "list_directory",
            json!({"path": base.join("private").to_str().unwrap()}),
        )
        .await
        .unwrap_err();
        assert!(matches!(outside, CommandError::Forbidden(_)), "{outside:?}");
        let start = dispatch(&ctx, "list_directory", json!({}))
            .await
            .expect("the default is the root");
        assert_eq!(start["parent"], Value::Null);

        let scan = dispatch(&ctx, "scan_folder", json!({"options": {"root": base.join("private").to_str().unwrap(), "recursive": false, "minimumSizeMib": 0, "filenameIncludePatterns": [], "filenameExcludePatterns": []}})).await.unwrap_err();
        assert!(matches!(scan, CommandError::Forbidden(_)), "{scan:?}");
        let run = dispatch(&ctx, "start_run", json!({"request": run_request(media.to_str().unwrap(), "integrity", &[base.join("private/x.mkv").to_str().unwrap()], 0)})).await.unwrap_err();
        assert!(matches!(run, CommandError::Forbidden(_)), "{run:?}");
        assert!(
            ctx.run_state.0.lock().unwrap().is_none(),
            "a refused run never takes the job slot"
        );
        // Calls that name no paths still work, and the GUI's default media folder is the first root, so its
        // first settings load at boot is not refused.
        let default = dispatch(&ctx, "default_media_root", json!({}))
            .await
            .unwrap();
        assert_eq!(
            default,
            json!(media.canonicalize().unwrap().to_string_lossy())
        );
        assert!(dispatch(&ctx, "load_settings", json!({"root": default}))
            .await
            .is_ok());
        std::fs::remove_dir_all(base).ok();
    }

    #[tokio::test]
    async fn directory_listing_is_available_in_read_only_mode() {
        let ctx = test_context(true);
        let root = crate::test_dir("web-listing");
        std::fs::write(root.join("a.mkv"), b"1").unwrap();
        let value = dispatch(
            &ctx,
            "list_directory",
            json!({"path": root.to_str().unwrap()}),
        )
        .await
        .unwrap();
        assert_eq!(value["entries"][0]["name"], "a.mkv");
        std::fs::remove_dir_all(root).ok();
    }
}
