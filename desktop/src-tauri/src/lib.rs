//! HEVC Rebuild Studio library crate: module declarations, the Tauri command registry (`run`), and
//! the re-exports other modules and the binary rely on. Behavior lives in the focused modules.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, State};

mod agent_control;
mod app_paths;
mod app_preferences;
mod apt_commands;
mod apt_policy;
#[cfg(test)]
mod argument_tests;
mod backup_folder;
mod batch_plan;
mod calibration;
mod cli_apt;
mod cli_json;
mod cli_modes;
mod commands_library;
mod commands_logs;
mod commands_metadata;
mod commands_queue;
mod commands_workspace;
mod content;
mod dependencies;
mod encode_plan;
mod encode_run;
mod encode_supervisor;
mod environment;
mod executable_lookup;
mod hdr_metadata;
mod integrity;
mod job_state;
mod library_artwork;
mod library_assets;
mod library_metadata;
mod library_organizer;
mod matroska_apply;
mod matroska_poster;
mod matroska_scrub;
mod matroska_tags;
mod matroska_verify;
mod mcp_server;
mod media_queue;
mod media_scan;
mod metadata_credentials;
mod metadata_run;
mod metadata_text;
mod output_targets;
mod packet_verify;
mod process_output;
mod profile_cache;
mod quality_gate;
mod rebuild_run;
mod reports;
mod resource_monitor;
mod resource_plan;
mod run_arguments;
mod run_checks;
mod run_commands;
mod run_coordinator;
mod run_native;
mod runtime_guard;
mod settings;
mod settings_arguments;
mod stop_control;
mod tool_diagnostics;
mod transactions;
mod validation;
mod work_directory;
mod youtube;
mod youtube_arguments;
mod youtube_optimize;
mod youtube_run;

pub(crate) use run_arguments::{
    append_filename_filter_arguments, append_selected_file_arguments, append_tool_path_arguments,
    file_settings_override_arguments, metadata_override_arguments, poster_override_arguments,
    rebuild_output_arguments, stream_override_arguments, youtube_hdr10plus_loss_arguments,
    ToolPaths,
};
#[cfg(test)]
use settings_arguments::metadata_text_blacklist_arguments;
use settings_arguments::settings_arguments;

pub use app_paths::initialize_portable_storage;
pub(crate) use app_paths::{
    app_cache_directory, app_config_directory, app_state_directory, media_queue_file,
    state_log_directory, tool_setup_log_directory, user_home_directory,
};
pub(crate) use app_preferences::app_preferences_file;
pub(crate) use apt_commands::libc_geteuid;
pub(crate) use apt_policy::{
    validate_apt_toolchain_confirmation, validate_apt_update_confirmation,
    validate_apt_upgrade_plan, APT_ALLOWLIST,
};
pub(crate) use cli_apt::{run_json_cli_apt_refresh, run_json_cli_apt_upgrade};
pub(crate) use cli_json::run_json_cli;
pub use cli_modes::{cli_mode, internal_cli_mode, mcp_mode, metadata_cli_mode, organizer_cli_mode};
pub(crate) use executable_lookup::executable_in_path;
pub(crate) use job_state::{
    child_tracker, claim_job, new_active_job, new_job_id, release_job, RunFinished, RunLine,
    RunProgress, RunResourceEvent, RunState,
};
pub(crate) use run_checks::{start_check_run, start_integrity_run};
pub(crate) use run_commands::RunRequest;
pub(crate) use run_native::{
    run_native_metadata_job, run_native_rebuild_job, run_native_youtube_job,
};
pub(crate) use work_directory::{
    media_work_directory, preview_work_directory, resolve_work_directory, WorkDirectoryPreview,
};
pub(crate) use youtube_arguments::{youtube_mode_arguments, YOUTUBE_CHOICE_PREFERENCES};

/// Integration tests repoint the process-wide state directory, so they all take turns here.
#[cfg(test)]
pub(crate) static TEST_STATE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Gate for integration tests that drive the real media tools. Returns `true`
/// when every tool runs. A missing tool normally skips the test with a note, but
/// with `HEVC_REQUIRE_MEDIA_TOOLS=1` (set in CI) it panics so the suite cannot
/// pass while silently skipping its end-to-end coverage.
#[cfg(test)]
pub(crate) fn media_tools_ready(test: &str, tools: &[&str]) -> bool {
    for tool in tools {
        let available = std::process::Command::new(tool)
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok();
        if available {
            continue;
        }
        assert!(
            std::env::var_os("HEVC_REQUIRE_MEDIA_TOOLS").is_none(),
            "{test}: required media tool `{tool}` is unavailable (HEVC_REQUIRE_MEDIA_TOOLS is set)"
        );
        eprintln!("skipping {test}: {tool} is unavailable");
        return false;
    }
    true
}

/// Write an executable fake tool for a test and make it safe to run at once.
///
/// Executing a file that was just written can fail with `ETXTBSY` ("Text file
/// busy") when another test thread forks in that instant: the forked child
/// briefly holds a copy of the still-open write handle until it execs. The
/// end-to-end tests spawn many processes, so the window is real. A short pause
/// lets any such child finish its exec and release the handle.
#[cfg(all(test, unix))]
pub(crate) fn write_test_script(path: &std::path::Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, body).expect("write the fake tool script");
    let mut permissions = std::fs::metadata(path)
        .expect("inspect the fake tool script")
        .permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(path, permissions).expect("make the fake tool executable");
    std::thread::sleep(std::time::Duration::from_millis(100));
}

/// The temp directory with symlinks resolved. On macOS `$TMPDIR` lives under
/// `/var`, a symlink to `/private/var`; the engine canonicalizes every path it
/// accepts, so tests that compare against paths they built by hand must start
/// from the resolved form or they fail on a correct implementation.
#[cfg(test)]
pub(crate) fn test_temp_base() -> PathBuf {
    let base = std::env::temp_dir();
    base.canonicalize().unwrap_or(base)
}

#[cfg(test)]
pub(crate) fn test_dir(label: &str) -> PathBuf {
    let path = test_temp_base().join(format!("hevc-rebuild-{label}-{}", new_job_id()));
    fs::create_dir_all(&path).expect("create isolated test directory");
    path
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let agent_control_state = agent_control::AgentControlState::default();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(agent_control_state.clone())
        .manage(RunState::default())
        .setup(move |app| {
            agent_control::start_server(app.handle().clone(), agent_control_state.clone())
                .map_err(std::io::Error::other)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            agent_control::reply,
            agent_control::frontend_ready,
            app_preferences::load_app_preferences,
            app_preferences::save_app_preferences,
            commands_workspace::inspect_environment,
            commands_library::open_file_location,
            commands_metadata::metadata_filename_clue,
            commands_metadata::metadata_filename_clues,
            commands_metadata::search_public_films,
            commands_metadata::load_public_film_details,
            commands_metadata::search_public_tv_shows,
            commands_metadata::load_public_tv_details,
            commands_metadata::search_tmdb_movies,
            commands_metadata::search_tmdb_tv,
            commands_metadata::load_tmdb_movie_details,
            commands_metadata::load_tmdb_tv_details,
            commands_metadata::tmdb_tv_certification_for_tvdb,
            commands_metadata::tmdb_credential_status,
            commands_metadata::save_tmdb_read_token,
            commands_metadata::reveal_tmdb_read_token,
            commands_metadata::remove_tmdb_read_token,
            commands_metadata::tvdb_credential_status,
            commands_metadata::save_tvdb_credentials,
            commands_metadata::reveal_tvdb_credentials,
            commands_metadata::remove_tvdb_credentials,
            commands_metadata::search_tvdb_titles,
            commands_metadata::load_tvdb_details,
            commands_library::save_library_artwork,
            commands_library::cache_library_artwork,
            commands_library::save_local_library_artwork,
            commands_library::cache_local_library_artwork,
            commands_library::inspect_app_artwork_cache,
            commands_library::purge_unused_app_artwork_cache,
            commands_library::scan_local_library_assets,
            commands_library::read_local_library_artwork_preview,
            commands_library::scan_organizer_folder,
            commands_library::preview_library_rename,
            commands_library::apply_library_rename,
            commands_workspace::default_media_root,
            commands_workspace::load_settings,
            commands_workspace::resolve_work_directory_command,
            commands_workspace::scan_folder,
            commands_queue::load_library_queue,
            commands_workspace::reconcile_interrupted_runs,
            commands_queue::save_library_queue,
            commands_queue::save_library_queue_selection,
            commands_queue::save_library_queue_movie_reviews,
            commands_queue::save_library_queue_series_reviews,
            commands_queue::save_library_queue_metadata_suggestions,
            commands_queue::preview_queue_output_targets,
            commands_logs::list_reports,
            commands_logs::delete_reports,
            commands_logs::list_tool_setup_logs,
            commands_logs::read_tool_setup_log,
            commands_logs::export_tool_setup_log,
            commands_logs::read_log,
            run_commands::start_run,
            run_commands::request_stop,
            run_commands::request_abort,
            commands_workspace::cancel_scan,
            apt_commands::check_ubuntu_package_updates,
            apt_commands::refresh_ubuntu_package_catalog,
            apt_commands::upgrade_ubuntu_packages,
            apt_commands::install_ubuntu_toolchain,
            apt_commands::install_apt_tools,
            apt_commands::install_hdr10plus,
        ])
        .build(tauri::generate_context!())
        .expect("error while building HEVC Rebuild Studio")
        .run(|_, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                agent_control::cleanup();
            }
        });
}

pub fn agent_control_mode(args: Vec<std::ffi::OsString>) -> i32 {
    agent_control::client_mode(args)
}
