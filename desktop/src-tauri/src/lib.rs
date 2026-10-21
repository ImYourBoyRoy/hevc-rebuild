use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager, State};

mod calibration;
mod content;
mod dependencies;
mod encode_plan;
mod encode_supervisor;
mod environment;
mod hdr_metadata;
mod library_artwork;
mod library_assets;
mod library_metadata;
mod library_nfo;
mod library_organizer;
mod mcp_server;
mod media_queue;
mod media_scan;
mod metadata_credentials;
mod metadata_text;
mod output_targets;
mod packet_verify;
mod process_output;
mod profile_cache;
mod reports;
mod resource_monitor;
mod resource_plan;
mod run_arguments;
mod runtime_guard;
pub(crate) use run_arguments::{
    append_filename_filter_arguments, append_selected_file_arguments, append_tool_path_arguments,
    file_settings_override_arguments, metadata_override_arguments, poster_override_arguments,
    rebuild_output_arguments, stream_override_arguments, youtube_hdr10plus_loss_arguments,
    ToolPaths,
};
mod settings;
mod settings_arguments;
#[cfg(test)]
use settings_arguments::metadata_text_blacklist_arguments;
use settings_arguments::settings_arguments;
mod stop_control;
mod tool_diagnostics;
mod transactions;
mod validation;
mod youtube;

pub(crate) const APT_ALLOWLIST: &[&str] = &[
    "python3",
    "ffmpeg",
    "mediainfo",
    "mkvtoolnix",
    "fontconfig",
    "ca-certificates",
];

pub(crate) fn validate_apt_update_confirmation(
    status: &Value,
    mut confirmed: Vec<String>,
) -> Result<Vec<String>, String> {
    if status.get("catalog_stale").and_then(Value::as_bool) == Some(true) {
        return Err("The local APT package catalog is stale. Refresh it explicitly, then check updates again.".into());
    }
    let mut available = status
        .get("packages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|package| package.get("name").and_then(Value::as_str))
        .map(str::to_string)
        .collect::<Vec<_>>();
    available.sort();
    available.dedup();
    if available
        .iter()
        .any(|package| !APT_ALLOWLIST.contains(&package.as_str()))
    {
        return Err(
            "The package update report contained a package outside the fixed allowlist.".into(),
        );
    }
    let provided_count = confirmed.len();
    confirmed.sort();
    confirmed.dedup();
    if confirmed.len() != provided_count {
        return Err("Each package can appear only once in the confirmation list.".into());
    }
    if available.is_empty() {
        return Err("The checked Ubuntu package catalog reports no available updates.".into());
    }
    if confirmed != available {
        return Err(format!(
            "Package update confirmation must exactly match the current checked list: {}.",
            available.join(", ")
        ));
    }
    Ok(available)
}

pub(crate) fn validate_apt_upgrade_plan(packages: &[String]) -> Result<(), String> {
    if !cfg!(target_os = "linux") {
        return Err("APT package operations are available only on Linux hosts.".into());
    }
    if packages.is_empty()
        || packages
            .iter()
            .any(|package| !APT_ALLOWLIST.contains(&package.as_str()))
    {
        return Err("APT upgrades accept only the fixed Ubuntu package allowlist.".into());
    }
    let output = Command::new("apt-get")
        .args([
            "-s",
            "install",
            "--only-upgrade",
            "--no-install-recommends",
            "--no-remove",
            "--",
        ])
        .args(packages)
        .output()
        .map_err(|error| format!("Could not simulate the APT upgrade plan: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "APT could not simulate this update plan: {}",
            String::from_utf8_lossy(&output.stderr)
                .chars()
                .take(2000)
                .collect::<String>()
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut planned = BTreeSet::new();
    let mut removals = Vec::new();
    for line in stdout.lines() {
        if let Some(package) = line
            .strip_prefix("Inst ")
            .and_then(|rest| rest.split_whitespace().next())
        {
            planned.insert(package.split(':').next().unwrap_or(package).to_string());
        }
        if let Some(package) = line
            .strip_prefix("Rem ")
            .and_then(|rest| rest.split_whitespace().next())
        {
            removals.push(package.to_string());
        }
    }
    let requested = packages.iter().cloned().collect::<BTreeSet<_>>();
    if planned != requested || !removals.is_empty() {
        let planned = planned.into_iter().collect::<Vec<_>>().join(", ");
        let removed = removals.join(", ");
        return Err(format!(
            "APT's simulated plan includes changes beyond the confirmed package list (upgrade plan: [{}]; removals: [{}]). No package changes were made.",
            planned, removed
        ));
    }
    Ok(())
}

pub(crate) fn validate_apt_install_plan(packages: &[String]) -> Result<(), String> {
    if !cfg!(target_os = "linux") {
        return Err("APT package operations are available only on Linux hosts.".into());
    }
    if packages.is_empty()
        || packages
            .iter()
            .any(|package| !APT_ALLOWLIST.contains(&package.as_str()))
    {
        return Err("APT installs accept only the fixed Ubuntu package allowlist.".into());
    }
    let output = Command::new("apt-get")
        .args([
            "-s",
            "install",
            "--no-upgrade",
            "--no-install-recommends",
            "--no-remove",
            "--",
        ])
        .args(packages)
        .output()
        .map_err(|error| format!("Could not simulate the APT install plan: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "APT could not simulate this install plan: {}",
            String::from_utf8_lossy(&output.stderr)
                .chars()
                .take(2000)
                .collect::<String>()
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    if stdout.lines().any(|line| line.starts_with("Rem ")) {
        return Err(
            "APT's simulated install plan would remove a package. No package changes were made."
                .into(),
        );
    }
    Ok(())
}

fn validate_apt_missing_confirmation(
    environment: &Value,
    mut confirmed: Vec<String>,
) -> Result<Vec<String>, String> {
    let mut missing = environment
        .get("missing_apt_packages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect::<Vec<_>>();
    missing.sort();
    missing.dedup();
    if missing
        .iter()
        .any(|package| !APT_ALLOWLIST.contains(&package.as_str()))
    {
        return Err(
            "The environment report included a package outside the fixed APT allowlist.".into(),
        );
    }
    let provided_count = confirmed.len();
    confirmed.sort();
    confirmed.dedup();
    if confirmed.len() != provided_count || confirmed != missing {
        return Err(format!(
            "Missing-package confirmation must exactly match the current environment report: {}.",
            missing.join(", ")
        ));
    }
    Ok(missing)
}

fn validate_apt_toolchain_confirmation(
    environment: &Value,
    update_status: &Value,
    confirmed_missing: Vec<String>,
    confirmed_updates: Vec<String>,
) -> Result<(Vec<String>, Vec<String>), String> {
    let missing = validate_apt_missing_confirmation(environment, confirmed_missing)?;
    let updates = if update_status
        .get("packages")
        .and_then(Value::as_array)
        .is_some_and(|packages| !packages.is_empty())
    {
        validate_apt_update_confirmation(update_status, confirmed_updates)?
    } else {
        if update_status.get("catalog_stale").and_then(Value::as_bool) == Some(true) {
            return Err("The local APT package catalog is stale. Refresh it, then review the tool setup again.".into());
        }
        if !confirmed_updates.is_empty() {
            return Err("The checked Ubuntu package catalog reports no available updates.".into());
        }
        Vec::new()
    };
    if missing.is_empty() && updates.is_empty() {
        return Err("No missing or outdated allowlisted Ubuntu packages need changes.".into());
    }
    if missing.iter().any(|package| updates.contains(package)) {
        return Err("A package cannot be confirmed as both missing and outdated.".into());
    }
    if !missing.is_empty() {
        validate_apt_install_plan(&missing)?;
    }
    if !updates.is_empty() {
        validate_apt_upgrade_plan(&updates)?;
    }
    Ok((missing, updates))
}

#[derive(Clone, Default)]
struct RunState(Arc<Mutex<Option<ActiveJob>>>);

#[derive(Clone)]
struct ActiveJob {
    id: String,
    stop_file: Option<PathBuf>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RunLine {
    run_id: String,
    line: String,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EngineProgress {
    file_name: String,
    #[serde(default)]
    file_path: Option<String>,
    phase: String,
    batch_number: u32,
    batch_total: u32,
    file_percent: Option<f64>,
    queue_percent: Option<f64>,
    speed: Option<f64>,
    eta_seconds: Option<f64>,
    elapsed_seconds: Option<f64>,
    input_bytes: Option<u64>,
    output_bytes: Option<u64>,
    estimated_output_bytes: Option<u64>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RunProgress {
    run_id: String,
    file_name: String,
    file_path: Option<String>,
    phase: String,
    batch_number: u32,
    batch_total: u32,
    file_percent: Option<f64>,
    queue_percent: Option<f64>,
    speed: Option<f64>,
    eta_seconds: Option<f64>,
    elapsed_seconds: Option<f64>,
    input_bytes: Option<u64>,
    output_bytes: Option<u64>,
    estimated_output_bytes: Option<u64>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RunResourceEvent {
    run_id: String,
    snapshot: resource_monitor::RuntimeResourceSnapshot,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RunFinished {
    run_id: String,
    exit_code: Option<i32>,
    stop_requested: bool,
    log_path: Option<String>,
    report_path: Option<String>,
    error: Option<String>,
}

fn engine_root(app: &AppHandle) -> Result<PathBuf, String> {
    #[cfg(debug_assertions)]
    {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .map_err(|error| format!("Could not locate the Python media engine: {error}"))?;
        if root.join("hevc_rebuild").is_dir() {
            return Ok(root);
        }
    }

    app.path()
        .resolve("engine", tauri::path::BaseDirectory::Resource)
        .map_err(|error| format!("Could not locate the bundled Python media engine: {error}"))
}

fn python_program() -> std::ffi::OsString {
    if let Some(value) = std::env::var_os("HEVC_REBUILD_PYTHON") {
        return value;
    }
    #[cfg(target_os = "windows")]
    {
        return "python".into();
    }
    #[cfg(not(target_os = "windows"))]
    {
        "python3".into()
    }
}

fn python_module_command(module: &str) -> Command {
    let mut command = Command::new(python_program());
    command
        .arg("-m")
        .arg(module)
        .env("PYTHONUNBUFFERED", "1")
        // linuxdeploy's AppRun exports PYTHONHOME to its AppDir even though
        // this package relies on the system Python runtime and stdlib.
        .env_remove("PYTHONHOME");
    command
}

fn standalone_engine_root() -> Result<PathBuf, String> {
    if let Some(configured) = std::env::var_os("HEVC_REBUILD_ENGINE_ROOT") {
        let root = PathBuf::from(configured).canonicalize().map_err(|error| {
            format!("Configured HEVC_REBUILD_ENGINE_ROOT is unavailable: {error}")
        })?;
        if root.join("hevc_rebuild").is_dir() {
            return Ok(root);
        }
        return Err(
            "HEVC_REBUILD_ENGINE_ROOT must contain the hevc_rebuild Python package.".into(),
        );
    }

    #[cfg(debug_assertions)]
    {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .map_err(|error| format!("Could not locate the Python media engine: {error}"))?;
        if root.join("hevc_rebuild").is_dir() {
            return Ok(root);
        }
    }

    #[cfg(target_os = "linux")]
    if let Some(app_dir) = std::env::var_os("APPDIR") {
        let candidate = PathBuf::from(app_dir).join("usr/lib/HEVC Rebuild Studio/engine");
        if candidate.join("hevc_rebuild").is_dir() {
            return Ok(candidate);
        }
    }

    #[cfg(target_os = "linux")]
    for candidate in [
        PathBuf::from("/usr/lib/HEVC Rebuild Studio/engine"),
        PathBuf::from("/usr/lib/hevc-rebuild-studio/engine"),
        PathBuf::from("/usr/share/hevc-rebuild-studio/engine"),
    ] {
        if candidate.join("hevc_rebuild").is_dir() {
            return Ok(candidate);
        }
    }

    // Tauri's installed Windows and macOS layouts keep bundled resources
    // beside the executable rather than in the Linux system directories above.
    // Keep the standalone CLI and MCP entry points on the same bundled engine
    // as the GUI without requiring a manually configured environment variable.
    if let Ok(executable) = std::env::current_exe() {
        if let Some(executable_directory) = executable.parent() {
            for candidate in [
                executable_directory.join("resources/engine"),
                executable_directory.join("../Resources/engine"),
                executable_directory.join("../resources/engine"),
                executable_directory.join("engine"),
            ] {
                if candidate.join("hevc_rebuild").is_dir() {
                    return Ok(candidate.canonicalize().unwrap_or(candidate));
                }
            }
        }
    }
    Err(
        "Could not locate the Python media engine. Set HEVC_REBUILD_ENGINE_ROOT to its folder."
            .into(),
    )
}

fn standalone_command(module: &str, args: &[String]) -> Result<Command, String> {
    let root = standalone_engine_root()?;
    let mut command = python_module_command(module);
    command.args(args);
    let mut python_paths = vec![root];
    if let Some(existing) = std::env::var_os("PYTHONPATH") {
        python_paths.extend(std::env::split_paths(&existing));
    }
    let joined = std::env::join_paths(python_paths)
        .map_err(|error| format!("Could not configure Python's module path: {error}"))?;
    command.env("PYTHONPATH", joined);
    if let Ok(executable) = std::env::current_exe() {
        command.env("HEVC_REBUILD_NATIVE_HELPER", executable);
    }
    Ok(command)
}

fn run_json_cli(args: &[String]) -> i32 {
    if args.is_empty() {
        eprintln!("Choose a JSON action: environment, scan, defaults, queue, queue-select, queue-run, output-plan, reports, apt-updates, apt-refresh, apt-upgrade, install-hdr10plus.");
        return 2;
    }
    if args[0] == "output-plan" {
        return output_targets::cli_mode(&args[1..]);
    }
    if args[0] == "scan" {
        return run_json_scan_cli(args);
    }
    if args[0] == "environment" {
        return run_environment_json_cli(args);
    }
    if args[0] == "queue" {
        return run_json_queue_cli(&args[1..]);
    }
    if args[0] == "queue-select" {
        return run_json_queue_select_cli(&args[1..]);
    }
    if args[0] == "queue-run" {
        return mcp_server::queue_run_json_cli(&args[1..]);
    }
    if args[0] == "artwork-cache" {
        return run_json_artwork_cache_cli(&args[1..]);
    }
    if args[0] == "apt-updates" {
        if args.len() != 1 {
            eprintln!("The apt-updates action does not accept additional options.");
            return 2;
        }
        return match dependencies::apt_update_status() {
            Ok(payload) => match serde_json::to_string(&payload) {
                Ok(json) => {
                    println!("{json}");
                    0
                }
                Err(error) => {
                    eprintln!("Could not serialize Ubuntu package update status: {error}");
                    2
                }
            },
            Err(error) => {
                eprintln!("{error}");
                1
            }
        };
    }
    if args[0] == "reports" {
        return run_reports_json_cli(args);
    }
    if args[0] == "apt-refresh" {
        return run_json_cli_apt_refresh(&args[1..]);
    }
    if args[0] == "apt-upgrade" {
        return run_json_cli_apt_upgrade(&args[1..]);
    }
    if args[0] == "install-hdr10plus" {
        if args.len() != 1 {
            eprintln!("install-hdr10plus does not accept additional options.");
            return 2;
        }
        return match dependencies::install_latest_hdr10plus_tool() {
            Ok(path) => match serde_json::to_string(&json!({ "installed": path })) {
                Ok(payload) => {
                    println!("{payload}");
                    0
                }
                Err(error) => {
                    eprintln!("Could not serialize the HDR10+ helper result: {error}");
                    2
                }
            },
            Err(error) => {
                eprintln!("{error}");
                1
            }
        };
    }
    if args[0] == "defaults" {
        let mut root = None;
        let mut index = 1;
        while index < args.len() {
            if args[index] != "--root" {
                eprintln!("Unknown defaults option {}.", args[index]);
                return 2;
            }
            let Some(value) = args.get(index + 1) else {
                eprintln!("Defaults option --root requires a path.");
                return 2;
            };
            if root.replace(value.clone()).is_some() {
                eprintln!("Defaults option --root can be specified only once.");
                return 2;
            }
            index += 2;
        }
        let Some(root) = root else {
            eprintln!("Defaults require --root PATH.");
            return 2;
        };
        return match settings::load_effective_settings(&root) {
            Ok(payload) => match serde_json::to_string(&payload) {
                Ok(json) => {
                    println!("{json}");
                    0
                }
                Err(error) => {
                    eprintln!("Could not serialize effective settings: {error}");
                    2
                }
            },
            Err(error) => {
                eprintln!("{error}");
                1
            }
        };
    }
    eprintln!(
        "Unknown JSON action {:?}. Available actions: environment, scan, defaults, queue, queue-select, queue-run, output-plan, reports, apt-updates, apt-refresh, apt-upgrade, install-hdr10plus, artwork-cache.",
        args[0]
    );
    2
}

fn run_json_artwork_cache_cli(args: &[String]) -> i32 {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("Could not start artwork-cache operation runtime: {error}");
            return 2;
        }
    };
    if args == ["inspect"] {
        return match runtime
            .block_on(library_artwork::inspect_app_artwork_cache())
            .and_then(|info| serde_json::to_string(&info).map_err(|error| error.to_string()))
        {
            Ok(payload) => {
                println!("{payload}");
                0
            }
            Err(error) => {
                eprintln!("Could not inspect the artwork cache: {error}");
                1
            }
        };
    }
    if args.len() == 4 && args[0] == "purge" && args[1] == "--preview-id" && args[3] == "--confirm"
    {
        return match runtime
            .block_on(library_artwork::purge_unused_app_artwork_cache(
                &args[2], true,
            ))
            .and_then(|result| serde_json::to_string(&result).map_err(|error| error.to_string()))
        {
            Ok(payload) => {
                println!("{payload}");
                0
            }
            Err(error) => {
                eprintln!("Could not purge unused artwork: {error}");
                1
            }
        };
    }
    eprintln!("Usage: hevc-rebuild-studio json artwork-cache inspect | artwork-cache purge --preview-id ID --confirm");
    2
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct QueueSelectionRequest {
    selected_media_paths: Vec<String>,
}

fn run_json_queue_cli(args: &[String]) -> i32 {
    let mut offset = 0usize;
    let mut limit = 100usize;
    let mut index = 0;
    while index < args.len() {
        let (target, label) = match args[index].as_str() {
            "--offset" => (&mut offset, "offset"),
            "--limit" => (&mut limit, "limit"),
            option => {
                eprintln!("Unknown queue option {option}.");
                return 2;
            }
        };
        let Some(value) = args.get(index + 1) else {
            eprintln!("Queue option --{label} requires an integer.");
            return 2;
        };
        let Ok(parsed) = value.parse::<usize>() else {
            eprintln!("Queue option --{label} requires an integer.");
            return 2;
        };
        *target = parsed;
        index += 2;
    }
    let path = match media_queue_file() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("Could not locate the saved media queue: {error}");
            return 1;
        }
    };
    match media_queue::summarize(&path, offset, limit)
        .and_then(|payload| serde_json::to_string(&payload).map_err(|error| error.to_string()))
    {
        Ok(payload) => {
            println!("{payload}");
            0
        }
        Err(error) => {
            eprintln!("Could not read the saved media queue: {error}");
            1
        }
    }
}

fn run_json_queue_select_cli(args: &[String]) -> i32 {
    if args.len() != 2 || args[0] != "--request" {
        eprintln!("Usage: hevc-rebuild-studio json queue-select --request SELECTION.json");
        return 2;
    }
    let request_path = PathBuf::from(&args[1]);
    let Ok(metadata) = request_path.metadata() else {
        eprintln!("Queue selection request file is unavailable.");
        return 2;
    };
    if !metadata.is_file() || metadata.len() > 1024 * 1024 {
        eprintln!("Queue selection request must be a regular file no larger than 1 MiB.");
        return 2;
    }
    let request_bytes = match fs::read(request_path) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("Could not read queue selection request: {error}");
            return 2;
        }
    };
    let request: QueueSelectionRequest = match serde_json::from_slice(&request_bytes) {
        Ok(request) => request,
        Err(error) => {
            eprintln!("Invalid queue selection JSON: {error}");
            return 2;
        }
    };
    let path = match media_queue_file() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("Could not locate the saved media queue: {error}");
            return 1;
        }
    };
    match media_queue::save_selected_paths(&path, request.selected_media_paths)
        .and_then(|()| media_queue::summarize(&path, 0, 100))
        .and_then(|mut payload| {
            payload
                .as_object_mut()
                .ok_or_else(|| "Queue summary had an invalid response shape.".to_string())?
                .insert("selectionSaved".into(), Value::Bool(true));
            serde_json::to_string(&payload).map_err(|error| error.to_string())
        }) {
        Ok(payload) => {
            println!("{payload}");
            0
        }
        Err(error) => {
            eprintln!("Could not save queue selection: {error}");
            1
        }
    }
}

fn run_json_scan_cli(args: &[String]) -> i32 {
    let mut root = None;
    let mut recursive = false;
    let mut recursive_seen = false;
    let mut minimum_size_mib = 5_u32;
    let mut minimum_seen = false;
    let mut tool_paths = ToolPaths::default();
    let mut filename_include_patterns = Vec::new();
    let mut filename_exclude_patterns = Vec::new();
    let mut index = 1;
    while index < args.len() {
        let name = args[index].as_str();
        match name {
            "--recursive" if !recursive_seen => {
                recursive = true;
                recursive_seen = true;
                index += 1;
            }
            "--recursive" => {
                eprintln!("Scan option --recursive can be specified only once.");
                return 2;
            }
            "--root" => {
                let Some(value) = args.get(index + 1) else {
                    eprintln!("Scan option --root requires a path.");
                    return 2;
                };
                if root.replace(value.clone()).is_some() {
                    eprintln!("Scan option --root can be specified only once.");
                    return 2;
                }
                index += 2;
            }
            "--min-size-mib" => {
                let Some(value) = args.get(index + 1) else {
                    eprintln!("Scan option --min-size-mib requires a whole number.");
                    return 2;
                };
                if minimum_seen {
                    eprintln!("Scan option --min-size-mib can be specified only once.");
                    return 2;
                }
                minimum_size_mib = match value.parse::<u32>() {
                    Ok(value) if value <= 4096 => value,
                    _ => {
                        eprintln!("Minimum input size must be between 0 and 4096 MiB.");
                        return 2;
                    }
                };
                minimum_seen = true;
                index += 2;
            }
            "--include-filename-pattern" | "--exclude-filename-pattern" => {
                let Some(value) = args.get(index + 1) else {
                    eprintln!("Scan option {name} requires a filename pattern.");
                    return 2;
                };
                if name == "--include-filename-pattern" {
                    filename_include_patterns.push(value.clone());
                } else {
                    filename_exclude_patterns.push(value.clone());
                }
                index += 2;
            }
            "--ffmpeg-path" | "--ffprobe-path" | "--mediainfo-path" | "--mkvpropedit-path"
            | "--mkvmerge-path" | "--mkvextract-path" => {
                let Some(value) = args.get(index + 1) else {
                    eprintln!("Scan option {name} requires an executable path.");
                    return 2;
                };
                let target = match name {
                    "--ffmpeg-path" => &mut tool_paths.ffmpeg,
                    "--ffprobe-path" => &mut tool_paths.ffprobe,
                    "--mediainfo-path" => &mut tool_paths.mediainfo,
                    "--mkvpropedit-path" => &mut tool_paths.mkvpropedit,
                    "--mkvmerge-path" => &mut tool_paths.mkvmerge,
                    _ => &mut tool_paths.mkvextract,
                };
                if target.replace(value.clone()).is_some() {
                    eprintln!("Scan option {name} can be specified only once.");
                    return 2;
                }
                index += 2;
            }
            _ => {
                eprintln!("Unknown scan option {name}.");
                return 2;
            }
        }
    }
    let Some(root) = root else {
        eprintln!("Scan requires --root PATH.");
        return 2;
    };
    match media_scan::scan_media(
        media_scan::ScanOptions {
            root,
            recursive,
            minimum_size_mib,
            filename_include_patterns,
            filename_exclude_patterns,
            tool_paths,
        },
        |current, total, file| eprintln!("scan\t{current}\t{total}\t{file}"),
    ) {
        Ok(payload) => match serde_json::to_string(&payload) {
            Ok(json) => {
                println!("{json}");
                0
            }
            Err(error) => {
                eprintln!("Could not serialize media analysis: {error}");
                2
            }
        },
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

fn run_environment_json_cli(args: &[String]) -> i32 {
    let mut paths = ToolPaths::default();
    let mut legacy_ffmpeg = None;
    let mut index = 1;
    while index < args.len() {
        let name = args[index].as_str();
        let Some(value) = args.get(index + 1) else {
            eprintln!("Environment option {name} requires a path.");
            return 2;
        };
        match name {
            "--ffmpeg-path" => {
                legacy_ffmpeg = Some(value.clone());
                paths.ffmpeg = Some(value.clone());
            }
            "--ffprobe-path" => paths.ffprobe = Some(value.clone()),
            "--mediainfo-path" => paths.mediainfo = Some(value.clone()),
            "--mkvpropedit-path" => paths.mkvpropedit = Some(value.clone()),
            "--mkvmerge-path" => paths.mkvmerge = Some(value.clone()),
            "--mkvextract-path" => paths.mkvextract = Some(value.clone()),
            _ => {
                eprintln!("Unknown environment option {name}.");
                return 2;
            }
        }
        index += 2;
    }
    match environment::inspect_environment(paths, legacy_ffmpeg) {
        Ok(payload) => match serde_json::to_string(&payload) {
            Ok(json) => {
                println!("{json}");
                0
            }
            Err(error) => {
                eprintln!("Could not serialize environment report: {error}");
                2
            }
        },
        Err(error) => {
            eprintln!("{error}");
            2
        }
    }
}

fn parse_apt_upgrade_cli_args(args: &[String]) -> Result<Vec<String>, String> {
    let mut confirmed = false;
    let mut packages = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--confirm" if !confirmed => confirmed = true,
            "--confirm" => return Err("APT package upgrades accept --confirm only once.".into()),
            "--package" => {
                let Some(package) = args.get(index + 1) else {
                    return Err("APT package upgrades require a name after --package.".into());
                };
                packages.push(package.clone());
                index += 1;
            }
            option if option.starts_with("--package=") => {
                packages.push(option["--package=".len()..].to_string());
            }
            option => return Err(format!("Unknown apt-upgrade option {option}.")),
        }
        index += 1;
    }
    if !confirmed {
        return Err("APT package upgrades require --confirm.".into());
    }
    if packages.is_empty() {
        return Err("APT package upgrades require at least one --package NAME entry.".into());
    }
    Ok(packages)
}

fn run_apt_cli_command(arguments: &[String]) -> Result<(), String> {
    if !cfg!(target_os = "linux") {
        return Err("APT package operations are available only on Linux hosts.".into());
    }
    if arguments.is_empty() {
        return Err("APT command arguments are missing.".into());
    }
    let mut command = if unsafe { libc_geteuid() } == 0 {
        let mut command = Command::new("apt-get");
        command.args(arguments);
        command
    } else {
        if !executable_in_path("sudo") {
            return Err("CLI administrator authentication (sudo) is not installed.".into());
        }
        let mut command = Command::new("sudo");
        command.arg("apt-get").args(arguments);
        command
    };
    let mut child = command
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("Could not start the APT operation: {error}"))?;
    let process_id = child.id();
    let stdout_reader = child
        .stdout
        .take()
        .map(|reader| thread::spawn(move || forward_command_output(reader, process_id)));
    let stderr_reader = child
        .stderr
        .take()
        .map(|reader| thread::spawn(move || forward_command_output(reader, process_id)));
    let status = child.wait();
    if let Some(reader) = stdout_reader {
        reader
            .join()
            .map_err(|_| "Could not forward APT standard output.".to_string())??;
    }
    if let Some(reader) = stderr_reader {
        reader
            .join()
            .map_err(|_| "Could not forward APT diagnostics.".to_string())??;
    }
    let status =
        status.map_err(|error| format!("Could not wait for the APT operation: {error}"))?;
    if !status.success() {
        return Err(format!("APT operation exited with {status}."));
    }
    Ok(())
}

fn forward_command_output(mut reader: impl Read, process_id: u32) -> Result<(), String> {
    // Preserve raw CLI bytes and interactive sudo's foreground-terminal group.
    // Unlike noninteractive GUI/MCP jobs, this command must not create a group.
    let result = (|| {
        let mut bytes = [0_u8; 8192];
        loop {
            let count = reader
                .read(&mut bytes)
                .map_err(|error| format!("Could not read APT output: {error}"))?;
            if count == 0 {
                return Ok(());
            }
            let stderr = std::io::stderr();
            let mut stderr = stderr.lock();
            stderr
                .write_all(&bytes[..count])
                .and_then(|_| stderr.flush())
                .map_err(|error| format!("Could not forward APT output: {error}"))?;
        }
    })();
    if result.is_err() {
        #[cfg(unix)]
        unsafe {
            if let Ok(pid) = libc::pid_t::try_from(process_id) {
                libc::kill(pid, libc::SIGKILL);
            }
        }
    }
    result
}

fn run_json_cli_apt_refresh(args: &[String]) -> i32 {
    if args != ["--confirm"] {
        eprintln!("APT catalog refresh requires only the explicit --confirm option.");
        return 2;
    }
    if let Err(error) = dependencies::apt_update_status() {
        eprintln!("{error}");
        return 1;
    }
    if let Err(error) = run_apt_cli_command(&["update".into()]) {
        eprintln!("{error}");
        return 1;
    }
    println!("{{\"refreshed\":true,\"note\":\"Package indexes refreshed; no packages were installed or upgraded.\"}}");
    0
}

fn run_json_cli_apt_upgrade(args: &[String]) -> i32 {
    let confirmed = match parse_apt_upgrade_cli_args(args) {
        Ok(confirmed) => confirmed,
        Err(error) => {
            eprintln!("{error}");
            return 2;
        }
    };
    let status = match dependencies::apt_update_status() {
        Ok(status) => status,
        Err(error) => {
            eprintln!("{error}");
            return 1;
        }
    };
    let packages = match validate_apt_update_confirmation(&status, confirmed) {
        Ok(packages) => packages,
        Err(error) => {
            eprintln!("{error}");
            return 1;
        }
    };
    if let Err(error) = validate_apt_upgrade_plan(&packages) {
        eprintln!("{error}");
        return 1;
    }
    let mut arguments = vec![
        "install".into(),
        "--only-upgrade".into(),
        "--no-install-recommends".into(),
        "--no-remove".into(),
        "--yes".into(),
        "--".into(),
    ];
    arguments.extend(packages.iter().cloned());
    if let Err(error) = run_apt_cli_command(&arguments) {
        eprintln!("{error}");
        return 1;
    }
    match serde_json::to_string(&json!({
        "upgraded": packages,
        "note": "The confirmed allowlisted package updates completed.",
    })) {
        Ok(payload) => {
            println!("{payload}");
            0
        }
        Err(error) => {
            eprintln!("Could not serialize the APT upgrade result: {error}");
            2
        }
    }
}

fn run_reports_json_cli(args: &[String]) -> i32 {
    let mut limit = 12_usize;
    let mut index = 1;
    while index < args.len() {
        if args[index] != "--limit" {
            eprintln!("Unknown reports option {}.", args[index]);
            return 2;
        }
        let Some(value) = args.get(index + 1) else {
            eprintln!("Reports option --limit requires a number.");
            return 2;
        };
        let Ok(parsed) = value.trim().parse::<i128>() else {
            eprintln!("Reports option --limit must be a positive integer.");
            return 2;
        };
        limit = parsed.clamp(1, 50) as usize;
        index += 2;
    }
    let directory = match state_log_directory() {
        Ok(directory) => directory,
        Err(error) => {
            eprintln!("{error}");
            return 1;
        }
    };
    match serde_json::to_string(&reports::list_reports(&directory, limit)) {
        Ok(payload) => {
            println!("{payload}");
            0
        }
        Err(error) => {
            eprintln!("Could not serialize report history: {error}");
            2
        }
    }
}

pub fn cli_mode(mode: &str, args: Vec<std::ffi::OsString>) -> i32 {
    let strings: Vec<String> = args
        .into_iter()
        .map(|value| value.to_string_lossy().into_owned())
        .collect();
    if mode == "json" {
        return run_json_cli(&strings);
    }
    match standalone_command("hevc_rebuild", &strings) {
        Ok(mut command) => match command.status() {
            Ok(status) => status.code().unwrap_or(1),
            Err(error) => {
                eprintln!("Could not start Python media engine: {error}");
                2
            }
        },
        Err(error) => {
            eprintln!("{error}");
            2
        }
    }
}

pub fn internal_cli_mode(args: Vec<std::ffi::OsString>) -> i32 {
    let mut args = args.into_iter();
    let Some(action) = args.next() else {
        eprintln!("Missing internal helper action.");
        return 2;
    };
    match action.to_string_lossy().as_ref() {
        "packet-fingerprints" | "stream-copy-hash" => {
            packet_verify::cli_mode(std::iter::once(action).chain(args).collect())
        }
        "validate-streams" => validation::cli_mode(args.collect()),
        "runtime-guard" => runtime_guard::cli_mode(args.collect()),
        "encode-plan" => encode_plan::cli_mode(args.collect()),
        "encode-supervisor" => encode_supervisor::cli_mode(args.collect()),
        "hdr-metadata" => hdr_metadata::cli_mode(args.collect()),
        "commit-output" => transactions::cli_mode(args.collect()),
        "install-output-copy" => transactions::install_copy_cli_mode(args.collect()),
        "calibration" => calibration::cli_mode(args.collect()),
        "content-tune" => content::cli_mode(args.collect()),
        "metadata-text" => metadata_text::cli_mode(args.collect()),
        "report-append" => reports::cli_mode(args.collect()),
        "resource-plan" => resource_plan::runtime_cli_mode(args.collect()),
        "resource-retry" => resource_plan::retry_cli_mode(args.collect()),
        "youtube" => youtube::cli_mode(args.collect()),
        "discover" => media_scan::discover_cli_mode(),
        "engine-profile" => {
            if args.next().is_some() {
                eprintln!("The internal engine-profile command does not accept options.");
                2
            } else {
                media_scan::engine_profile_cli_mode()
            }
        }
        "engine-profile-tracks" => {
            if args.next().is_some() {
                eprintln!("The internal engine-profile-tracks command does not accept options.");
                2
            } else {
                media_scan::engine_track_profiles_cli_mode()
            }
        }
        "resolve-toolchain" => environment::resolve_toolchain_cli_mode(),
        _ => {
            eprintln!("Unknown internal helper action.");
            2
        }
    }
}

pub fn metadata_cli_mode(args: Vec<std::ffi::OsString>) -> i32 {
    let args: Vec<String> = args
        .into_iter()
        .map(|value| value.to_string_lossy().into_owned())
        .collect();
    if args
        .first()
        .is_some_and(|command| command == "write-preview" || command == "write-apply")
    {
        return library_nfo::metadata_cli_mode(&args);
    }
    if args.first().is_some_and(|command| command == "assets") {
        return library_assets::cli_mode(&args);
    }
    let usage = "Usage: hevc-rebuild-studio metadata movie search TITLE | movie details WIKIDATA_ID | tv search TITLE | tv details TVMAZE_ID | artwork save REQUEST.json | assets inspect REQUEST.json | write-preview REQUEST.json | write-apply REQUEST.json --preview-id ID --confirm";
    if args.len() < 3 {
        eprintln!("{usage}");
        return 2;
    }
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("Could not start metadata lookup runtime: {error}");
            return 2;
        }
    };
    let result = match (args[0].as_str(), args[1].as_str()) {
        ("movie", "search") if args.len() >= 3 => {
            let query = args[2..].join(" ");
            runtime
                .block_on(library_metadata::search_films(&query))
                .and_then(|value| serde_json::to_value(value).map_err(|error| error.to_string()))
        }
        ("movie", "details") if args.len() == 3 => runtime
            .block_on(library_metadata::load_film_details(&args[2]))
            .and_then(|value| serde_json::to_value(value).map_err(|error| error.to_string())),
        ("tv", "search") if args.len() >= 3 => {
            let query = args[2..].join(" ");
            runtime
                .block_on(library_metadata::search_tv_shows(&query))
                .and_then(|value| serde_json::to_value(value).map_err(|error| error.to_string()))
        }
        ("tv", "details") if args.len() == 3 => {
            let Ok(show_id) = args[2].parse::<u64>() else {
                eprintln!("TVmaze show ID must be a positive integer.");
                return 2;
            };
            runtime
                .block_on(library_metadata::load_tv_series_details(show_id))
                .and_then(|value| serde_json::to_value(value).map_err(|error| error.to_string()))
        }
        ("artwork", "save") if args.len() == 3 => {
            let request_text = match fs::read_to_string(&args[2]) {
                Ok(value) if value.len() <= 1024 * 1024 => value,
                Ok(_) => {
                    eprintln!("Artwork request JSON is limited to 1 MiB.");
                    return 2;
                }
                Err(error) => {
                    eprintln!("Could not read artwork request JSON: {error}");
                    return 2;
                }
            };
            match serde_json::from_str::<library_artwork::ArtworkSaveRequest>(&request_text) {
                Ok(request) => runtime
                    .block_on(library_artwork::save_artwork(
                        request.candidate,
                        request.root,
                        request.targets,
                    ))
                    .and_then(|value| {
                        serde_json::to_value(value).map_err(|error| error.to_string())
                    }),
                Err(error) => Err(format!("Artwork request JSON is invalid: {error}")),
            }
        }
        _ => {
            eprintln!("{usage}");
            return 2;
        }
    };
    match result {
        Ok(value) => match serde_json::to_string_pretty(&value) {
            Ok(json) => {
                println!("{json}");
                0
            }
            Err(error) => {
                eprintln!("Could not format metadata JSON: {error}");
                2
            }
        },
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

pub fn organizer_cli_mode(args: Vec<std::ffi::OsString>) -> i32 {
    let strings = args
        .into_iter()
        .map(|value| value.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    library_organizer::organizer_cli_mode(&strings)
}

pub fn mcp_mode() -> i32 {
    mcp_server::run()
}

fn engine_command(app: &AppHandle, args: &[String]) -> Result<Command, String> {
    let root = engine_root(app)?;
    let mut command = python_module_command("hevc_rebuild");
    command
        .args(args)
        .current_dir(root)
        .env("HEVC_REBUILD_GUI_EVENTS", "1");
    if let Ok(executable) = std::env::current_exe() {
        command.env("HEVC_REBUILD_NATIVE_HELPER", executable);
    }
    Ok(command)
}

fn forward_lines<R: Read + Send + 'static>(
    app: AppHandle,
    event: &'static str,
    run_id: String,
    stream: R,
    process_id: u32,
) -> thread::JoinHandle<Result<(), String>> {
    thread::spawn(move || {
        crate::process_output::read_process_lines(stream, process_id, |line| {
            if event == "run-line" {
                if let Some(progress) = parse_engine_progress(&run_id, &line) {
                    let _ = app.emit("run-progress", progress);
                    return;
                }
            }
            let _ = app.emit(
                event,
                RunLine {
                    run_id: run_id.clone(),
                    line,
                },
            );
        })
    })
}

fn emit_repair_line(
    app: &AppHandle,
    logger: &tool_diagnostics::ToolSetupLogger,
    job_id: &str,
    line: String,
) {
    logger.record(&line);
    let _ = app.emit(
        "repair-line",
        RunLine {
            run_id: job_id.to_string(),
            line,
        },
    );
}

fn forward_repair_lines<R: Read + Send + 'static>(
    app: AppHandle,
    job_id: String,
    logger: Arc<tool_diagnostics::ToolSetupLogger>,
    stream: R,
    process_id: u32,
) -> thread::JoinHandle<Result<(), String>> {
    thread::spawn(move || {
        crate::process_output::read_process_lines(stream, process_id, |line| {
            emit_repair_line(&app, &logger, &job_id, line);
        })
    })
}

fn parse_engine_progress(run_id: &str, line: &str) -> Option<RunProgress> {
    let payload = line.strip_prefix("#HEVC_REBUILD_PROGRESS\t")?;
    let event: EngineProgress = serde_json::from_str(payload).ok()?;
    if event.file_name.trim().is_empty()
        || event.file_name.len() > 512
        || event.file_path.as_deref().is_some_and(|path| {
            path.trim().is_empty() || path.len() > 8192 || !Path::new(path).is_absolute()
        })
        || event.phase.trim().is_empty()
        || event.phase.len() > 96
        || event.batch_total == 0
        || event.batch_number == 0
        || event.batch_number > event.batch_total
        || event
            .file_percent
            .is_some_and(|value| !(0.0..=100.0).contains(&value))
        || event
            .queue_percent
            .is_some_and(|value| !(0.0..=100.0).contains(&value))
        || event.speed.is_some_and(|value| value < 0.0)
        || event.eta_seconds.is_some_and(|value| value < 0.0)
        || event.elapsed_seconds.is_some_and(|value| value < 0.0)
    {
        return None;
    }
    Some(RunProgress {
        run_id: run_id.to_string(),
        file_name: event.file_name,
        file_path: event.file_path,
        phase: event.phase,
        batch_number: event.batch_number,
        batch_total: event.batch_total,
        file_percent: event.file_percent,
        queue_percent: event.queue_percent,
        speed: event.speed,
        eta_seconds: event.eta_seconds,
        elapsed_seconds: event.elapsed_seconds,
        input_bytes: event.input_bytes,
        output_bytes: event.output_bytes,
        estimated_output_bytes: event.estimated_output_bytes,
    })
}

#[tauri::command]
fn open_file_location(path: String) -> Result<(), String> {
    let path = PathBuf::from(path.trim());
    if !path.is_absolute() {
        return Err("Choose a full file path.".into());
    }
    let location = if path.is_dir() {
        path
    } else {
        path.parent()
            .ok_or_else(|| "This file has no folder.".to_owned())?
            .to_path_buf()
    };
    if !location.is_dir() {
        return Err("That folder is not on this computer.".into());
    }
    #[cfg(target_os = "windows")]
    let mut opener = Command::new("explorer");
    #[cfg(target_os = "macos")]
    let mut opener = Command::new("open");
    #[cfg(target_os = "linux")]
    let mut opener = Command::new("xdg-open");
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    return Err("Opening a folder is not supported on this operating system.".into());

    opener
        .arg(&location)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("Could not open the folder: {error}"))?;
    Ok(())
}

#[tauri::command]
async fn inspect_environment(tool_paths: Option<ToolPaths>) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        environment::inspect_environment(tool_paths.unwrap_or_default(), None)
    })
    .await
    .map_err(|error| format!("Environment scan task failed: {error}"))?
}

#[tauri::command]
fn metadata_filename_clue(
    filename: String,
    patterns: Vec<String>,
) -> Result<library_metadata::FilenameMetadataClue, String> {
    library_metadata::filename_metadata_clue_with_patterns(&filename, &patterns)
}

#[tauri::command]
fn metadata_filename_clues(
    filenames: Vec<String>,
    patterns: Vec<String>,
) -> Result<Vec<library_metadata::FilenameMetadataClue>, String> {
    if filenames.len() > 10_000 {
        return Err("Filename clue requests are limited to 10,000 media items.".into());
    }
    filenames
        .iter()
        .map(|filename| library_metadata::filename_metadata_clue_with_patterns(filename, &patterns))
        .collect()
}

// Online metadata lookups are explicit GUI operations. TMDB credentials are
// retrieved only from the operating-system keyring and stay out of CLI/MCP.
#[tauri::command]
async fn search_public_films(
    query: String,
    language: String,
) -> Result<Vec<library_metadata::FilmSearchResult>, String> {
    library_metadata::search_films_in_language(&query, &language).await
}

#[tauri::command]
async fn load_public_film_details(
    entity_id: String,
    language: String,
) -> Result<library_metadata::FilmMetadataDetails, String> {
    library_metadata::load_film_details_in_language(&entity_id, &language).await
}

#[tauri::command]
async fn search_public_tv_shows(
    query: String,
) -> Result<Vec<library_metadata::TvShowSearchResult>, String> {
    library_metadata::search_tv_shows(&query).await
}

#[tauri::command]
async fn load_public_tv_details(show_id: u64) -> Result<library_metadata::TvSeriesDetails, String> {
    library_metadata::load_tv_series_details(show_id).await
}

#[tauri::command]
async fn search_tmdb_movies(
    query: String,
    language: String,
) -> Result<Vec<library_metadata::TmdbSearchResult>, String> {
    let token = read_tmdb_read_token().await?;
    library_metadata::search_tmdb_movies(&query, &token, &language).await
}

#[tauri::command]
async fn search_tmdb_tv(
    query: String,
    language: String,
) -> Result<Vec<library_metadata::TmdbSearchResult>, String> {
    let token = read_tmdb_read_token().await?;
    library_metadata::search_tmdb_tv(&query, &token, &language).await
}

#[tauri::command]
async fn load_tmdb_movie_details(
    id: u64,
    language: String,
) -> Result<library_metadata::TmdbMovieDetails, String> {
    let token = read_tmdb_read_token().await?;
    library_metadata::load_tmdb_movie_details(id, &token, &language).await
}

#[tauri::command]
async fn load_tmdb_tv_details(
    id: u64,
    language: String,
) -> Result<library_metadata::TmdbTvDetails, String> {
    let token = read_tmdb_read_token().await?;
    library_metadata::load_tmdb_tv_details(id, &token, &language).await
}

#[tauri::command]
async fn tmdb_tv_certification_for_tvdb(
    tvdb_id: u64,
    language: String,
) -> Result<Option<String>, String> {
    let token = read_tmdb_read_token().await?;
    library_metadata::tmdb_tv_certification_for_tvdb(tvdb_id, &token, &language).await
}

async fn read_tmdb_read_token() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(metadata_credentials::read_tmdb_token)
        .await
        .map_err(|error| format!("TMDB credential lookup task failed: {error}"))?
}

#[tauri::command]
async fn tmdb_credential_status() -> Result<metadata_credentials::TmdbCredentialStatus, String> {
    tauri::async_runtime::spawn_blocking(metadata_credentials::tmdb_status)
        .await
        .map_err(|error| format!("Credential store status task failed: {error}"))
}

#[tauri::command]
async fn save_tmdb_read_token(token: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || metadata_credentials::save_tmdb_token(&token))
        .await
        .map_err(|error| format!("Credential save task failed: {error}"))?
}

/// Reveal the saved token only after an explicit request from the native UI.
/// This command is intentionally absent from CLI and MCP interfaces.
#[tauri::command]
async fn reveal_tmdb_read_token() -> Result<String, String> {
    read_tmdb_read_token().await
}

#[tauri::command]
async fn remove_tmdb_read_token() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(metadata_credentials::remove_tmdb_token)
        .await
        .map_err(|error| format!("Credential removal task failed: {error}"))?
}

async fn read_tvdb_credentials() -> Result<metadata_credentials::TvdbCredentials, String> {
    tauri::async_runtime::spawn_blocking(metadata_credentials::read_tvdb_credentials)
        .await
        .map_err(|error| format!("TheTVDB credential lookup task failed: {error}"))?
}

/// Reveal TheTVDB credentials only after an explicit request from the native UI.
/// This command is intentionally absent from CLI and MCP interfaces.
#[tauri::command]
async fn reveal_tvdb_credentials() -> Result<metadata_credentials::TvdbCredentials, String> {
    read_tvdb_credentials().await
}

#[tauri::command]
async fn tvdb_credential_status() -> Result<metadata_credentials::TvdbCredentialStatus, String> {
    tauri::async_runtime::spawn_blocking(metadata_credentials::tvdb_status)
        .await
        .map_err(|error| format!("Credential store status task failed: {error}"))
}

#[tauri::command]
async fn save_tvdb_credentials(
    api_key: String,
    subscriber_pin: Option<String>,
    access_confirmed: bool,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        metadata_credentials::save_tvdb_credentials(
            &api_key,
            subscriber_pin.as_deref().unwrap_or_default(),
            access_confirmed,
        )
    })
    .await
    .map_err(|error| format!("Credential save task failed: {error}"))?
}

#[tauri::command]
async fn remove_tvdb_credentials() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(metadata_credentials::remove_tvdb_credentials)
        .await
        .map_err(|error| format!("Credential removal task failed: {error}"))?
}

#[tauri::command]
async fn search_tvdb_titles(
    query: String,
    kind: String,
    language: String,
) -> Result<Vec<library_metadata::TvdbSearchResult>, String> {
    let credentials = read_tvdb_credentials().await?;
    library_metadata::search_tvdb_titles(
        &query,
        &kind,
        &language,
        &credentials.api_key,
        credentials.subscriber_pin.as_deref(),
    )
    .await
}

#[tauri::command]
async fn load_tvdb_details(
    id: u64,
    kind: String,
    language: String,
) -> Result<library_metadata::TvdbMetadataDetails, String> {
    let credentials = read_tvdb_credentials().await?;
    library_metadata::load_tvdb_details(
        id,
        &kind,
        &language,
        &credentials.api_key,
        credentials.subscriber_pin.as_deref(),
    )
    .await
}

#[tauri::command]
async fn save_library_artwork(
    request: library_artwork::ArtworkSaveRequest,
) -> Result<library_artwork::ArtworkSaveResult, String> {
    library_artwork::save_artwork(request.candidate, request.root, request.targets).await
}

#[tauri::command]
async fn cache_library_artwork(
    candidate: library_metadata::ArtworkCandidate,
) -> Result<library_artwork::ArtworkSaveResult, String> {
    library_artwork::save_artwork_to_cache(candidate).await
}

#[tauri::command]
fn save_local_library_artwork(
    request: library_artwork::LocalArtworkSaveRequest,
) -> Result<library_artwork::LocalArtworkSaveResult, String> {
    library_artwork::save_local_artwork(request)
}

#[tauri::command]
async fn cache_local_library_artwork(
    source_path: String,
) -> Result<library_artwork::LocalArtworkSaveResult, String> {
    library_artwork::save_local_artwork_to_cache(source_path).await
}

#[tauri::command]
async fn inspect_app_artwork_cache() -> Result<library_artwork::ArtworkCacheInfo, String> {
    library_artwork::inspect_app_artwork_cache().await
}

#[tauri::command]
async fn purge_unused_app_artwork_cache(
    state: State<'_, RunState>,
    preview_id: String,
    confirmed: bool,
) -> Result<library_artwork::ArtworkCachePurgeResult, String> {
    let id = new_job_id();
    claim_job(
        state.inner(),
        ActiveJob {
            id: id.clone(),
            stop_file: None,
        },
    )?;
    let result = library_artwork::purge_unused_app_artwork_cache(&preview_id, confirmed).await;
    release_job(state.inner(), &id);
    result
}

#[tauri::command]
fn scan_local_library_assets(
    request: library_assets::LocalAssetsRequest,
) -> Result<library_assets::LocalAssetsScan, String> {
    library_assets::scan_local_assets(request)
}

#[tauri::command]
fn read_local_library_artwork_preview(
    request: library_assets::ArtworkPreviewRequest,
) -> Result<library_assets::ArtworkPreview, String> {
    library_assets::read_artwork_preview(request)
}

#[tauri::command]
fn preview_library_metadata_write(
    request: library_nfo::MetadataWriteRequest,
) -> Result<library_nfo::MetadataWritePreview, String> {
    library_nfo::preview_metadata_write(request)
}

#[tauri::command]
fn apply_library_metadata_write(
    state: State<'_, RunState>,
    request: library_nfo::MetadataWriteRequest,
    preview_id: String,
    confirmed: bool,
) -> Result<library_nfo::MetadataWriteResult, String> {
    let id = new_job_id();
    claim_job(
        state.inner(),
        ActiveJob {
            id: id.clone(),
            stop_file: None,
        },
    )?;
    let result = library_nfo::apply_metadata_write(request, preview_id, confirmed);
    release_job(state.inner(), &id);
    result
}

#[tauri::command]
fn scan_organizer_folder(root: String) -> Result<library_organizer::OrganizerFolderScan, String> {
    library_organizer::scan_organizer_folder(root)
}

#[tauri::command]
fn preview_library_rename(
    root: String,
    items: Vec<library_organizer::RenameTemplateInput>,
    text_blacklist_patterns: Vec<String>,
) -> Result<library_organizer::RenamePreview, String> {
    library_organizer::preview_rename_batch_with_patterns(root, items, text_blacklist_patterns)
}

#[tauri::command]
fn apply_library_rename(
    root: String,
    items: Vec<library_organizer::RenameTemplateInput>,
    text_blacklist_patterns: Vec<String>,
    preview_id: String,
    confirmed: bool,
) -> Result<library_organizer::RenameApplyResult, String> {
    library_organizer::apply_rename_batch_with_patterns(
        root,
        items,
        text_blacklist_patterns,
        preview_id,
        confirmed,
    )
}

#[tauri::command]
async fn load_settings(root: String) -> Result<settings::EffectiveSettings, String> {
    tauri::async_runtime::spawn_blocking(move || settings::load_effective_settings(&root))
        .await
        .map_err(|error| format!("Settings load task failed: {error}"))?
}

#[tauri::command]
fn default_media_root() -> Result<String, String> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or("Could not locate the user's home folder.")?;
    let videos = home.join("Videos");
    Ok(if videos.is_dir() { videos } else { home }
        .to_string_lossy()
        .into_owned())
}

#[tauri::command]
async fn scan_folder(
    app: AppHandle,
    root: String,
    recursive: bool,
    minimum_size_mib: u32,
    filename_include_patterns: Vec<String>,
    filename_exclude_patterns: Vec<String>,
    tool_paths: Option<ToolPaths>,
) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        media_scan::scan_media(
            media_scan::ScanOptions {
                root,
                recursive,
                minimum_size_mib,
                filename_include_patterns,
                filename_exclude_patterns,
                tool_paths: tool_paths.unwrap_or_default(),
            },
            |current, total, file| {
                let _ = app.emit(
                    "scan-progress",
                    json!({ "current": current, "total": total, "file": file }),
                );
            },
        )
    })
    .await
    .map_err(|error| format!("Folder scan task failed: {error}"))?
}

#[tauri::command]
async fn reconcile_interrupted_runs(
    state: State<'_, RunState>,
) -> Result<media_queue::InterruptedRecovery, String> {
    let job_active = state.0.lock().map(|guard| guard.is_some()).unwrap_or(true);
    let queue_file = media_queue_file()?;
    tauri::async_runtime::spawn_blocking(move || {
        if job_active {
            return media_queue::interrupted_recovery_skipped(&queue_file);
        }
        media_queue::reconcile_interrupted(&queue_file)
    })
    .await
    .map_err(|error| format!("Interrupted-session recovery failed: {error}"))?
}

#[tauri::command]
async fn load_library_queue() -> Result<media_queue::QueueLoadResult, String> {
    tauri::async_runtime::spawn_blocking(|| media_queue::load(&media_queue_file()?))
        .await
        .map_err(|error| format!("Queue load task failed: {error}"))?
}

#[tauri::command]
async fn save_library_queue(queue: media_queue::QueueDocument) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || media_queue::save(&media_queue_file()?, queue))
        .await
        .map_err(|error| format!("Queue save task failed: {error}"))?
}

#[tauri::command]
async fn save_library_queue_selection(
    selected_media_paths: Vec<String>,
    active_root: Option<String>,
    file_tune_overrides: BTreeMap<String, String>,
    file_stream_overrides: BTreeMap<String, Vec<u16>>,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        media_queue::save_selection(
            &media_queue_file()?,
            selected_media_paths,
            active_root,
            file_tune_overrides,
            file_stream_overrides,
        )
    })
    .await
    .map_err(|error| format!("Queue selection save task failed: {error}"))?
}

#[tauri::command]
async fn save_library_queue_movie_reviews(
    movie_reviews: BTreeMap<String, media_queue::QueueMovieReview>,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        media_queue::save_movie_reviews(&media_queue_file()?, movie_reviews)
    })
    .await
    .map_err(|error| format!("Queue metadata save task failed: {error}"))?
}

#[tauri::command]
async fn save_library_queue_series_reviews(
    series_reviews: BTreeMap<String, media_queue::QueueSeriesReview>,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        media_queue::save_series_reviews(&media_queue_file()?, series_reviews)
    })
    .await
    .map_err(|error| format!("Queue series metadata save task failed: {error}"))?
}

#[tauri::command]
async fn save_library_queue_metadata_suggestions(
    match_suggestions: BTreeMap<String, media_queue::QueueMetadataSuggestion>,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        media_queue::save_metadata_suggestions(&media_queue_file()?, match_suggestions)
    })
    .await
    .map_err(|error| format!("Queue metadata suggestion save task failed: {error}"))?
}

#[tauri::command]
async fn preview_queue_output_targets(
    request: output_targets::OutputPreviewRequest,
) -> Result<output_targets::OutputPreview, String> {
    tauri::async_runtime::spawn_blocking(move || output_targets::preview(request))
        .await
        .map_err(|error| format!("Output preview task failed: {error}"))?
}

#[tauri::command]
async fn delete_reports() -> Result<usize, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let directory = state_log_directory()?;
        reports::delete_saved_reports(&directory)
    })
    .await
    .map_err(|error| format!("Report deletion task failed: {error}"))?
}

#[tauri::command]
async fn list_reports(limit: u8) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let directory = state_log_directory()?;
        Ok(reports::list_reports(
            &directory,
            limit.clamp(1, 50) as usize,
        ))
    })
    .await
    .map_err(|error| format!("Report listing task failed: {error}"))?
}

fn state_log_directory() -> Result<PathBuf, String> {
    Ok(app_state_directory()?.join("logs"))
}

fn tool_setup_log_directory() -> Result<PathBuf, String> {
    Ok(state_log_directory()?.join("tool-setup"))
}

#[tauri::command]
fn list_tool_setup_logs() -> Result<Vec<tool_diagnostics::ToolSetupLogSummary>, String> {
    tool_diagnostics::list(&tool_setup_log_directory()?)
}

#[tauri::command]
fn read_tool_setup_log(job_id: String) -> Result<String, String> {
    tool_diagnostics::read(&tool_setup_log_directory()?, &job_id)
}

#[tauri::command]
fn export_tool_setup_log(job_id: String, destination: String) -> Result<String, String> {
    tool_diagnostics::export(
        &tool_setup_log_directory()?,
        &job_id,
        Path::new(&destination),
    )
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppPreferencesPayload {
    preferences: Value,
    settings_file: String,
    has_saved_settings: bool,
    portable: bool,
    storage_warning: Option<String>,
}

static PREFERENCES_WRITE_LOCK: Mutex<()> = Mutex::new(());

fn app_preferences_file() -> Result<PathBuf, String> {
    Ok(app_config_directory()?.join("settings.json"))
}

pub(crate) fn user_home_directory() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        if let Some(home) = std::env::var_os("USERPROFILE") {
            return Some(PathBuf::from(home));
        }
        if let (Some(drive), Some(path)) =
            (std::env::var_os("HOMEDRIVE"), std::env::var_os("HOMEPATH"))
        {
            return Some(PathBuf::from(format!(
                "{}{}",
                drive.to_string_lossy(),
                path.to_string_lossy()
            )));
        }
    }
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

pub(crate) fn app_config_directory() -> Result<PathBuf, String> {
    if let Some(portable) = std::env::var_os("HEVC_REBUILD_PORTABLE_DATA_DIR") {
        return Ok(PathBuf::from(portable));
    }
    #[cfg(target_os = "windows")]
    {
        let root = std::env::var_os("APPDATA")
            .or_else(|| std::env::var_os("LOCALAPPDATA"))
            .map(PathBuf::from)
            .or_else(|| user_home_directory().map(|home| home.join("AppData/Roaming")))
            .ok_or("Could not locate the current user's Windows application-data folder.")?;
        return Ok(root.join("hevc-rebuild-studio"));
    }
    #[cfg(target_os = "macos")]
    {
        let home = user_home_directory()
            .ok_or("Could not locate the current user's macOS home folder.")?;
        return Ok(home.join("Library/Application Support/hevc-rebuild-studio"));
    }
    #[cfg(target_os = "linux")]
    {
        let root = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| user_home_directory().map(|home| home.join(".config")))
            .ok_or("Could not locate the current user's settings folder.")?;
        return Ok(root.join("hevc-rebuild-studio"));
    }
    user_home_directory()
        .map(|home| home.join(".hevc-rebuild-studio"))
        .ok_or_else(|| "Could not locate the current user's application-data folder.".to_string())
}

fn app_state_directory() -> Result<PathBuf, String> {
    if let Some(portable) = std::env::var_os("HEVC_REBUILD_PORTABLE_DATA_DIR") {
        return Ok(PathBuf::from(portable));
    }
    #[cfg(target_os = "windows")]
    {
        let root = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .or_else(|| user_home_directory().map(|home| home.join("AppData/Local")))
            .ok_or("Could not locate the current user's Windows local-data folder.")?;
        return Ok(root.join("hevc-rebuild-studio"));
    }
    #[cfg(target_os = "macos")]
    {
        let home = user_home_directory()
            .ok_or("Could not locate the current user's macOS home folder.")?;
        return Ok(home.join("Library/Logs/hevc-rebuild-studio"));
    }
    #[cfg(target_os = "linux")]
    {
        let root = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| user_home_directory().map(|home| home.join(".local/state")))
            .ok_or("Could not locate the current user's state folder.")?;
        return Ok(root.join("hevc-rebuild"));
    }
    app_config_directory()
}

pub(crate) fn app_cache_directory() -> Result<PathBuf, String> {
    if let Some(portable) = std::env::var_os("HEVC_REBUILD_PORTABLE_DATA_DIR") {
        return Ok(PathBuf::from(portable).join("cache"));
    }
    #[cfg(target_os = "windows")]
    {
        let root = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .or_else(|| user_home_directory().map(|home| home.join("AppData/Local")))
            .ok_or("Could not locate the current user's Windows cache folder.")?;
        return Ok(root.join("hevc-rebuild-studio/cache"));
    }
    #[cfg(target_os = "macos")]
    {
        let home = user_home_directory()
            .ok_or("Could not locate the current user's macOS home folder.")?;
        return Ok(home.join("Library/Caches/hevc-rebuild-studio"));
    }
    #[cfg(target_os = "linux")]
    {
        let root = std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| user_home_directory().map(|home| home.join(".cache")))
            .ok_or("Could not locate the current user's cache folder.")?;
        return Ok(root.join("hevc-rebuild"));
    }
    app_config_directory()
}

pub(crate) fn media_queue_file() -> Result<PathBuf, String> {
    let preferences = app_preferences_file()?;
    let directory = preferences
        .parent()
        .ok_or("Could not locate the application profile folder.")?;
    Ok(directory.join("library-queue.json"))
}

pub fn initialize_portable_storage() {
    if std::env::var_os("HEVC_REBUILD_PORTABLE_DATA_DIR").is_some() {
        return;
    }
    let Some(appimage) = std::env::var_os("APPIMAGE") else {
        return;
    };
    let image_path = PathBuf::from(appimage.clone())
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(appimage));
    let mut sidecar_name = image_path.into_os_string();
    sidecar_name.push(".data");
    let sidecar = PathBuf::from(sidecar_name);
    let probe = sidecar.join(format!(".write-check-{}", std::process::id()));
    let available = (|| {
        fs::create_dir_all(&sidecar)?;
        #[cfg(unix)]
        fs::set_permissions(&sidecar, fs::Permissions::from_mode(0o700))?;
        fs::write(&probe, b"portable storage check")?;
        fs::remove_file(&probe)
    })();
    match available {
        Ok(()) => std::env::set_var("HEVC_REBUILD_PORTABLE_DATA_DIR", sidecar),
        Err(error) => std::env::set_var(
            "HEVC_REBUILD_PORTABLE_STORAGE_WARNING",
            format!(
                "The AppImage sidecar folder could not be written ({error}); settings will use this user's config folder."
            ),
        ),
    }
}

fn default_app_preferences() -> Value {
    json!({
        "theme": "light",
        "windowState": {"width": 1600, "height": 1000, "maximized": false},
        "lastPage": "home",
        "root": null,
        "toolPaths": {
            "ffmpeg": null,
            "ffprobe": null,
            "mediainfo": null,
            "mkvpropedit": null,
            "mkvmerge": null,
            "mkvextract": null
        },
        "recursive": true,
        "minimumSizeMib": "5",
        "stopOnError": false,
        "youtubeResolution": "1440p",
        "youtubeQualityMode": "balanced",
        "youtubeSourceMultiplier": 2.0,
        "youtubeOutputDir": null,
        "rebuildOutputDir": null,
        "fileOutputDirs": {},
        "filenameIncludePatterns": [],
        "filenameExcludePatterns": [
            "old", ".*", ".renamethethings", "@eadir", "extras", "extra", "trailers", "trailer",
            "samples", "sample", "behind the scenes", "behindthescenes", "deleted scenes",
            "deleted", "interviews", "interview", "scenes", "clips", "clip", "featurettes",
            "featurette", "shorts", "short", "other", "theme-music", "thememusic",
            "backdrops", "extrafanart"
        ],
        "filenameExcludeDefaultsVersion": 2,
        "savePosterBesideMedia": true,
        "metadataAutoLookup": true,
        "metadataLanguage": "en-US",
        "metadataSources": {
            "tmdb": true,
            "wikidata": true,
            "tvmaze": true,
            "tvdb": false
        },
        "organizerTemplates": {
            "movie": "{title} [[({year} {certification})]]{ext}",
            "series": "{series_title}",
            "season": "Season {season:02}",
            "episode": "S{season:02}E{episode:02}.{episode_title}{ext}"
        },
        "settings": null
    })
}

fn normalize_app_preferences(value: Value) -> Result<Value, String> {
    let object = value
        .as_object()
        .ok_or("Application preferences must be a JSON object.")?;
    let mut normalized = default_app_preferences();
    let result = normalized
        .as_object_mut()
        .expect("default preferences are an object");

    if let Some(theme) = object.get("theme") {
        let theme = theme
            .as_str()
            .ok_or("Appearance must be light, dark, phosphor, ion, or breach.")?;
        let theme = match theme {
            "hacker" => "phosphor",
            "light" | "dark" | "phosphor" | "ion" | "breach" => theme,
            _ => return Err("Appearance must be light, dark, phosphor, ion, or breach.".into()),
        };
        result.insert("theme".into(), json!(theme));
    }
    if let Some(last_page) = object.get("lastPage") {
        let last_page = last_page
            .as_str()
            .ok_or("The saved workspace tab must be text.")?;
        if ![
            "home", "library", "metadata", "youtube", "run", "settings", "reports", "help",
        ]
        .contains(&last_page)
        {
            return Err("The saved workspace tab is not one of the app pages.".into());
        }
        result.insert("lastPage".into(), json!(last_page));
    }
    if let Some(window_state) = object.get("windowState") {
        let window_state = window_state
            .as_object()
            .ok_or("Window state must be an object.")?;
        let width = window_state
            .get("width")
            .and_then(Value::as_u64)
            .filter(|width| (1120..=8192).contains(width))
            .ok_or("Saved window width must be between 1120 and 8192 logical pixels.")?;
        let height = window_state
            .get("height")
            .and_then(Value::as_u64)
            .filter(|height| (720..=8192).contains(height))
            .ok_or("Saved window height must be between 720 and 8192 logical pixels.")?;
        let maximized = window_state
            .get("maximized")
            .and_then(Value::as_bool)
            .ok_or("Saved maximized state must be true or false.")?;
        result.insert(
            "windowState".into(),
            json!({"width": width, "height": height, "maximized": maximized}),
        );
    }
    for key in ["root"] {
        if let Some(value) = object.get(key) {
            if !value.is_null() && !value.as_str().is_some_and(|text| text.len() <= 8192) {
                return Err(format!("{key} must be a path or null."));
            }
            result.insert(key.into(), value.clone());
        }
    }
    if let Some(output_dir) = object.get("rebuildOutputDir") {
        if !output_dir.is_null()
            && !output_dir
                .as_str()
                .is_some_and(|text| text.len() <= 8192 && Path::new(text).is_absolute())
        {
            return Err("Default rebuild output folder must be an absolute path or null.".into());
        }
        result.insert("rebuildOutputDir".into(), output_dir.clone());
    }
    if let Some(output_dirs) = object.get("fileOutputDirs") {
        let output_dirs = output_dirs
            .as_object()
            .ok_or("Per-file output folders must be a JSON object.")?;
        if output_dirs.len() > 512 {
            return Err("Per-file output folders are limited to 512 media files.".into());
        }
        let mut normalized_dirs = serde_json::Map::new();
        for (source, destination) in output_dirs {
            if source.len() > 8192
                || !Path::new(source).is_absolute()
                || !destination
                    .as_str()
                    .is_some_and(|text| text.len() <= 8192 && Path::new(text).is_absolute())
            {
                return Err("Per-file output source and folder paths must be absolute and no longer than 8192 characters.".into());
            }
            normalized_dirs.insert(source.clone(), destination.clone());
        }
        result.insert("fileOutputDirs".into(), Value::Object(normalized_dirs));
    }
    for (key, label) in [
        ("filenameIncludePatterns", "include"),
        ("filenameExcludePatterns", "exclude"),
    ] {
        if let Some(value) = object.get(key) {
            let mut patterns = value
                .as_array()
                .ok_or_else(|| format!("{key} must be a list of filename patterns."))?
                .iter()
                .map(|pattern| {
                    pattern
                        .as_str()
                        .map(str::to_string)
                        .ok_or_else(|| format!("{key} entries must be text."))
                })
                .collect::<Result<Vec<_>, _>>()?;
            if label == "exclude"
                && object
                    .get("filenameExcludeDefaultsVersion")
                    .and_then(Value::as_u64)
                    .is_none_or(|version| version < 2)
            {
                let defaults_value = default_app_preferences();
                let defaults = defaults_value["filenameExcludePatterns"]
                    .as_array()
                    .expect("default filename exclusions are an array")
                    .iter()
                    .filter_map(Value::as_str);
                let mut merged = defaults.map(str::to_owned).collect::<Vec<_>>();
                for pattern in patterns {
                    if !merged
                        .iter()
                        .any(|existing| existing.eq_ignore_ascii_case(&pattern))
                    {
                        merged.push(pattern);
                    }
                }
                patterns = merged;
            }
            media_scan::validate_filename_patterns(&patterns, label)?;
            result.insert(key.into(), json!(patterns));
        }
    }
    result.insert("filenameExcludeDefaultsVersion".into(), json!(2));
    if let Some(auto_lookup) = object.get("metadataAutoLookup") {
        if !auto_lookup.is_boolean() {
            return Err("Automatic metadata suggestions must be true or false.".into());
        }
        result.insert("metadataAutoLookup".into(), auto_lookup.clone());
    }
    if let Some(language) = object.get("metadataLanguage") {
        let language = language.as_str().ok_or("Metadata language must be text.")?;
        let normalized_language = library_metadata::validate_metadata_language(language)?;
        result.insert("metadataLanguage".into(), json!(normalized_language));
    }
    if let Some(sources) = object.get("metadataSources") {
        let sources = sources
            .as_object()
            .ok_or("Metadata source preferences must be an object.")?;
        let mut normalized_sources = default_app_preferences()["metadataSources"].clone();
        let normalized_source_object = normalized_sources
            .as_object_mut()
            .expect("default metadata sources are an object");
        for source in ["tmdb", "wikidata", "tvmaze", "tvdb"] {
            if let Some(enabled) = sources.get(source) {
                if !enabled.is_boolean() {
                    return Err(format!(
                        "The {source} source setting must be true or false."
                    ));
                }
                normalized_source_object.insert(source.into(), enabled.clone());
            }
        }
        result.insert("metadataSources".into(), normalized_sources);
    }
    let mut tool_paths = object
        .get("toolPaths")
        .cloned()
        .map(serde_json::from_value::<ToolPaths>)
        .transpose()
        .map_err(|error| format!("Tool executable settings are invalid: {error}"))?
        .unwrap_or_default();
    if let Some(legacy_ffmpeg) = object.get("ffmpegPath") {
        if !legacy_ffmpeg.is_null()
            && !legacy_ffmpeg
                .as_str()
                .is_some_and(|text| text.len() <= 8192)
        {
            return Err("ffmpegPath must be a path or null.".into());
        }
        if tool_paths.ffmpeg.is_none() {
            tool_paths.ffmpeg = legacy_ffmpeg.as_str().map(str::to_string);
        }
    }
    for (name, value) in [
        ("FFmpeg", &tool_paths.ffmpeg),
        ("FFprobe", &tool_paths.ffprobe),
        ("MediaInfo", &tool_paths.mediainfo),
        ("MKVToolNix mkvpropedit", &tool_paths.mkvpropedit),
        ("MKVToolNix mkvmerge", &tool_paths.mkvmerge),
        ("MKVToolNix mkvextract", &tool_paths.mkvextract),
    ] {
        if value.as_ref().is_some_and(|path| path.len() > 8192) {
            return Err(format!("{name} path must be 8192 characters or fewer."));
        }
    }
    result.insert(
        "toolPaths".into(),
        serde_json::to_value(tool_paths)
            .map_err(|error| format!("Could not normalize tool paths: {error}"))?,
    );
    if let Some(recursive) = object.get("recursive") {
        if !recursive.is_boolean() {
            return Err("Folder recursion must be true or false.".into());
        }
        result.insert("recursive".into(), recursive.clone());
    }
    if let Some(stop_on_error) = object.get("stopOnError") {
        if !stop_on_error.is_boolean() {
            return Err("Stop on first error must be true or false.".into());
        }
        result.insert("stopOnError".into(), stop_on_error.clone());
    }
    if let Some(save_poster_beside_media) = object.get("savePosterBesideMedia") {
        if !save_poster_beside_media.is_boolean() {
            return Err("Save poster beside media must be true or false.".into());
        }
        result.insert(
            "savePosterBesideMedia".into(),
            save_poster_beside_media.clone(),
        );
    }
    if let Some(resolution) = object.get("youtubeResolution") {
        let resolution = resolution
            .as_str()
            .ok_or("YouTube resolution must be source, 1440p, or 4k.")?;
        if !["source", "1440p", "4k"].contains(&resolution) {
            return Err("YouTube resolution must be source, 1440p, or 4k.".into());
        }
        result.insert("youtubeResolution".into(), json!(resolution));
    }
    // Older profiles saved the old `source` default whenever any preference
    // changed. Treat that pre-quality-mode value as the old default once; a
    // current profile with youtubeQualityMode set can still explicitly choose source.
    if object.get("youtubeQualityMode").is_none()
        && object.get("youtubeResolution").and_then(Value::as_str) == Some("source")
    {
        result.insert("youtubeResolution".into(), json!("1440p"));
    }
    if let Some(quality_mode) = object.get("youtubeQualityMode") {
        let quality_mode = quality_mode
            .as_str()
            .ok_or("YouTube size profile must be balanced or recommended.")?;
        if !["balanced", "recommended"].contains(&quality_mode) {
            return Err("YouTube size profile must be balanced or recommended.".into());
        }
        result.insert("youtubeQualityMode".into(), json!(quality_mode));
    }
    if let Some(multiplier) = object.get("youtubeSourceMultiplier") {
        let multiplier = multiplier
            .as_f64()
            .ok_or("YouTube source-size multiplier must be a number between 0.5 and 4.0.")?;
        if !multiplier.is_finite() || !(0.5..=4.0).contains(&multiplier) {
            return Err("YouTube source-size multiplier must be between 0.5 and 4.0.".into());
        }
        result.insert("youtubeSourceMultiplier".into(), json!(multiplier));
    }
    if let Some(output_dir) = object.get("youtubeOutputDir") {
        if !output_dir.is_null() && !output_dir.as_str().is_some_and(|text| text.len() <= 8192) {
            return Err("YouTube output folder must be a path or null.".into());
        }
        result.insert("youtubeOutputDir".into(), output_dir.clone());
    }
    if let Some(templates) = object.get("organizerTemplates") {
        let templates = templates
            .as_object()
            .ok_or("Organizer naming patterns must be an object.")?;
        let mut normalized_templates = default_app_preferences()["organizerTemplates"].clone();
        let normalized_object = normalized_templates
            .as_object_mut()
            .expect("default organizer templates are an object");
        for name in ["movie", "series", "season", "episode"] {
            if let Some(value) = templates.get(name) {
                let pattern = value
                    .as_str()
                    .ok_or_else(|| format!("The {name} naming pattern must be text."))?;
                if pattern.trim().is_empty() || pattern.len() > 1_024 {
                    return Err(format!(
                        "The {name} naming pattern must contain 1 to 1,024 UTF-8 bytes."
                    ));
                }
                normalized_object.insert(name.into(), json!(pattern));
            }
        }
        result.insert("organizerTemplates".into(), normalized_templates);
    }
    if let Some(minimum) = object.get("minimumSizeMib") {
        let text = minimum.as_str().ok_or("Minimum file size must be text.")?;
        let number = text
            .parse::<f64>()
            .map_err(|_| "Minimum file size must be a number.")?;
        if !number.is_finite() || !(0.0..=4096.0).contains(&number) {
            return Err("Minimum file size must be between 0 and 4096 MiB.".into());
        }
        result.insert("minimumSizeMib".into(), json!(text));
    }
    if let Some(settings) = object.get("settings") {
        if !settings.is_null() {
            settings_arguments(settings)?;
        }
        result.insert("settings".into(), settings.clone());
    }
    Ok(normalized)
}

fn preference_payload(
    preferences: Value,
    has_saved_settings: bool,
) -> Result<AppPreferencesPayload, String> {
    Ok(AppPreferencesPayload {
        preferences,
        settings_file: app_preferences_file()?.display().to_string(),
        has_saved_settings,
        portable: std::env::var_os("HEVC_REBUILD_PORTABLE_DATA_DIR").is_some(),
        storage_warning: std::env::var("HEVC_REBUILD_PORTABLE_STORAGE_WARNING").ok(),
    })
}

#[tauri::command]
fn load_app_preferences() -> Result<AppPreferencesPayload, String> {
    let path = app_preferences_file()?;
    let has_saved_settings = path.is_file();
    let preferences = if has_saved_settings {
        let bytes = fs::read(&path)
            .map_err(|error| format!("Could not read application settings: {error}"))?;
        let parsed = serde_json::from_slice(&bytes)
            .map_err(|error| format!("Application settings are not valid JSON: {error}"))?;
        normalize_app_preferences(parsed)?
    } else {
        default_app_preferences()
    };
    preference_payload(preferences, has_saved_settings)
}

#[tauri::command]
fn save_app_preferences(preferences: Value) -> Result<AppPreferencesPayload, String> {
    let normalized = normalize_app_preferences(preferences)?;
    let _guard = PREFERENCES_WRITE_LOCK
        .lock()
        .map_err(|_| "Could not lock application settings.")?;
    let path = app_preferences_file()?;
    let parent = path
        .parent()
        .ok_or("Could not locate the application settings folder.")?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("Could not create the application settings folder: {error}"))?;
    let temp = Path::new(parent).join(format!("settings.json.tmp-{}", std::process::id()));
    let bytes = serde_json::to_vec_pretty(&normalized)
        .map_err(|error| format!("Could not encode application settings: {error}"))?;
    let mut options = fs::OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);
    let write_result = (|| {
        let mut file = options
            .open(&temp)
            .map_err(|error| format!("Could not write application settings: {error}"))?;
        file.write_all(&bytes)
            .map_err(|error| format!("Could not write application settings: {error}"))?;
        file.sync_all()
            .map_err(|error| format!("Could not flush application settings: {error}"))
    })();
    if let Err(error) = write_result {
        let _ = fs::remove_file(&temp);
        return Err(error);
    }
    fs::rename(&temp, &path).map_err(|error| {
        let _ = fs::remove_file(&temp);
        format!("Could not save application settings: {error}")
    })?;
    preference_payload(normalized, true)
}

#[tauri::command]
fn read_log(path: String) -> Result<String, String> {
    let allowed = state_log_directory()?
        .canonicalize()
        .map_err(|error| format!("Diagnostic log folder is unavailable: {error}"))?;
    let requested = PathBuf::from(path)
        .canonicalize()
        .map_err(|error| format!("Could not open the diagnostic file: {error}"))?;
    if !requested.starts_with(&allowed)
        || requested.extension().and_then(|value| value.to_str()) != Some("log")
    {
        return Err(
            "Only HEVC Rebuild log files from this user's log folder can be read here.".into(),
        );
    }
    let mut file = File::open(&requested)
        .map_err(|error| format!("Could not read the diagnostic file: {error}"))?;
    let size = file.metadata().map_err(|error| error.to_string())?.len();
    let limit = 1_000_000_u64;
    let start = size.saturating_sub(limit);
    file.seek(SeekFrom::Start(start))
        .map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    let mut result = String::from_utf8_lossy(&bytes).into_owned();
    if start > 0 {
        result.insert_str(0, "[Showing the final 1 MB of this diagnostic log]\n\n");
    }
    Ok(result)
}

fn new_job_id() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{}-{now}", std::process::id())
}

fn claim_job(state: &RunState, job: ActiveJob) -> Result<(), String> {
    let mut current = state
        .0
        .lock()
        .map_err(|_| "Could not lock the desktop job state.")?;
    if current.is_some() {
        return Err("Another run or repair is already active.".into());
    }
    *current = Some(job);
    Ok(())
}

fn release_job(state: &RunState, id: &str) {
    if let Ok(mut current) = state.0.lock() {
        if current.as_ref().is_some_and(|job| job.id == id) {
            *current = None;
        }
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct RunRequest {
    root: String,
    recursive: bool,
    minimum_size_mib: u32,
    ffmpeg_path: Option<String>,
    #[serde(default)]
    tool_paths: Option<ToolPaths>,
    mode: String,
    youtube_resolution: Option<String>,
    youtube_quality_mode: Option<String>,
    youtube_source_multiplier: Option<f64>,
    youtube_output_dir: Option<String>,
    rebuild_output_dir: Option<String>,
    settings: Value,
    stop_on_error: bool,
    full_rebuild_confirmed: bool,
    #[serde(default)]
    filename_include_patterns: Vec<String>,
    #[serde(default)]
    filename_exclude_patterns: Vec<String>,
    #[serde(default)]
    selected_files: Vec<String>,
    #[serde(default)]
    file_tune_overrides: BTreeMap<String, String>,
    #[serde(default)]
    file_stream_overrides: BTreeMap<String, Vec<u16>>,
    #[serde(default)]
    file_settings_overrides: BTreeMap<String, Value>,
    #[serde(default)]
    file_poster_overrides: BTreeMap<String, String>,
    #[serde(default)]
    file_metadata_overrides: BTreeMap<String, Value>,
    #[serde(default)]
    file_output_dirs: BTreeMap<String, String>,
    #[serde(default)]
    allow_youtube_hdr10plus_loss_paths: Vec<String>,
}

#[tauri::command]
fn start_run(
    app: AppHandle,
    state: State<'_, RunState>,
    request: RunRequest,
) -> Result<String, String> {
    if request.minimum_size_mib > 4096 {
        return Err("Minimum input size must be 4096 MiB or less.".into());
    }
    let root = PathBuf::from(&request.root)
        .canonicalize()
        .map_err(|error| format!("Could not open the selected media path: {error}"))?;
    if !root.is_dir() && !root.is_file() {
        return Err("The selected media path is not a file or folder.".into());
    }
    if !["check", "metadata", "test", "full", "youtube"].contains(&request.mode.as_str()) {
        return Err("Choose Check only, Metadata-only MKV, Short test, Full rebuild, or YouTube upload prep.".into());
    }
    if request.mode == "full" && !request.full_rebuild_confirmed {
        return Err("Confirm the full rebuild before starting it.".into());
    }
    let has_rebuild_output = request
        .rebuild_output_dir
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
        || !request.file_output_dirs.is_empty();
    if has_rebuild_output && !["metadata", "test", "full"].contains(&request.mode.as_str()) {
        return Err(
            "Output folders apply only to metadata-only output, short tests, and full rebuilds."
                .into(),
        );
    }
    let (rebuild_output_args, rebuild_resource_path) =
        if ["metadata", "test", "full"].contains(&request.mode.as_str()) {
            rebuild_output_arguments(
                &root,
                request.rebuild_output_dir.as_deref(),
                request.file_output_dirs.clone(),
            )?
        } else {
            (Vec::new(), None)
        };
    let selected_disk_path = if root.is_file() {
        root.parent().unwrap_or(Path::new(".")).to_path_buf()
    } else {
        root.clone()
    };
    let resource_disk_path = if request.mode == "youtube" {
        request
            .youtube_output_dir
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .map(PathBuf::from)
            .unwrap_or(selected_disk_path)
    } else {
        rebuild_resource_path.unwrap_or(selected_disk_path)
    };
    let mut args = vec![
        root.to_string_lossy().into_owned(),
        "--min-size-mib".into(),
        request.minimum_size_mib.to_string(),
        "--no-dependency-repair".into(),
        "--skip-update-check".into(),
    ];
    if request.recursive {
        args.push("--recursive".into());
    }
    append_filename_filter_arguments(
        &mut args,
        &request.filename_include_patterns,
        &request.filename_exclude_patterns,
    )?;
    append_selected_file_arguments(&mut args, &root, &request.selected_files)?;
    args.extend(rebuild_output_args);
    if request.mode == "youtube" {
        let resolution = request.youtube_resolution.as_deref().unwrap_or("1440p");
        if !["source", "1440p", "4k"].contains(&resolution) {
            return Err("YouTube output size must be source, 1440p, or 4K.".into());
        }
        let quality_mode = request
            .youtube_quality_mode
            .as_deref()
            .unwrap_or("balanced");
        if !["balanced", "recommended"].contains(&quality_mode) {
            return Err("YouTube size profile must be balanced or recommended.".into());
        }
        let source_multiplier = request.youtube_source_multiplier.unwrap_or(2.0);
        if !source_multiplier.is_finite() || !(0.5..=4.0).contains(&source_multiplier) {
            return Err("YouTube source-size multiplier must be between 0.5 and 4.0.".into());
        }
        args.push("--youtube-upload".into());
        args.extend(["--upload-resolution".into(), resolution.into()]);
        args.extend(["--upload-quality-mode".into(), quality_mode.into()]);
        args.extend([
            "--upload-source-multiplier".into(),
            source_multiplier.to_string(),
        ]);
        if let Some(output_dir) = request
            .youtube_output_dir
            .filter(|value| !value.trim().is_empty())
        {
            let output_dir = PathBuf::from(output_dir);
            if !output_dir.is_absolute() {
                return Err("Choose an absolute YouTube output folder path.".into());
            }
            args.extend([
                "--upload-output-dir".into(),
                output_dir.to_string_lossy().into_owned(),
            ]);
        }
    }
    append_tool_path_arguments(
        &mut args,
        request.tool_paths.unwrap_or_default(),
        request.ffmpeg_path,
    )?;
    if request.mode == "youtube" && !request.file_stream_overrides.is_empty() {
        return Err("Per-file stream selections do not apply to YouTube export mode.".into());
    }
    if request.mode == "metadata" && !request.file_stream_overrides.is_empty() {
        return Err("Metadata-only MKV always preserves every source stream; per-file stream selection is not allowed.".into());
    }
    if request.mode != "youtube" && !request.allow_youtube_hdr10plus_loss_paths.is_empty() {
        return Err("HDR10+ metadata-loss consent applies only to YouTube export mode.".into());
    }
    args.extend(youtube_hdr10plus_loss_arguments(
        &root,
        request.allow_youtube_hdr10plus_loss_paths,
    )?);
    args.extend(stream_override_arguments(
        &root,
        request.file_stream_overrides,
    )?);
    if !request.file_settings_overrides.is_empty()
        && ["metadata", "youtube"].contains(&request.mode.as_str())
    {
        return Err(
            "Current-run encoder settings apply only to check, short-test, and full-rebuild modes."
                .into(),
        );
    }
    args.extend(file_settings_override_arguments(
        &root,
        request.file_settings_overrides,
    )?);
    if !request.file_poster_overrides.is_empty()
        && !["metadata", "test", "full"].contains(&request.mode.as_str())
    {
        return Err(
            "Selected poster images can be attached only to metadata-only, short-test, and full rebuild outputs."
                .into(),
        );
    }
    args.extend(poster_override_arguments(
        &root,
        request.file_poster_overrides,
        &library_artwork::app_artwork_cache_root()?,
    )?);
    if !request.file_metadata_overrides.is_empty()
        && !["metadata", "test", "full"].contains(&request.mode.as_str())
    {
        return Err("Reviewed metadata applies only to metadata-only, short-test, and full rebuild outputs.".into());
    }
    args.extend(metadata_override_arguments(
        &root,
        request.file_metadata_overrides,
    )?);
    if request.file_tune_overrides.len() > 512 {
        return Err("Per-file tune overrides are limited to 512 media files.".into());
    }
    if request.mode == "metadata" && !request.file_tune_overrides.is_empty() {
        return Err("Per-file encoding tunes do not apply to metadata-only MKV output.".into());
    }
    let mut validated_overrides = BTreeMap::new();
    for (path, tune) in request.file_tune_overrides {
        if !["animation", "none"].contains(&tune.as_str()) {
            return Err("Per-file tune overrides must be 'animation' or 'none'.".into());
        }
        let selected = PathBuf::from(path)
            .canonicalize()
            .map_err(|error| format!("Per-file tune source is unavailable: {error}"))?;
        if !selected.is_file()
            || (root.is_dir() && !selected.starts_with(&root))
            || (root.is_file() && selected != root)
        {
            return Err(
                "Per-file tune overrides must refer to media inside the selected input.".into(),
            );
        }
        validated_overrides.insert(selected.to_string_lossy().into_owned(), tune);
    }
    if !validated_overrides.is_empty() {
        let serialized = serde_json::to_string(&validated_overrides)
            .map_err(|error| format!("Could not encode per-file tune choices: {error}"))?;
        if serialized.len() > 65536 {
            return Err("Per-file tune overrides exceed the 64 KiB limit.".into());
        }
        args.extend(["--file-tune-overrides-json".into(), serialized]);
    }
    args.extend(settings_arguments(&request.settings)?);
    if request.stop_on_error {
        args.push("--stop-on-error".into());
    }
    match request.mode.as_str() {
        "check" => args.push("--info-only".into()),
        "metadata" => args.push("--metadata-only".into()),
        "test" => args.push("--test".into()),
        _ => {}
    }

    let id = new_job_id();
    let stop_token = stop_control::StopToken::new()?;
    let stop_file = stop_token.path().to_path_buf();
    args.extend([
        "--stop-request-file".into(),
        stop_file.to_string_lossy().into_owned(),
    ]);
    let active = ActiveJob {
        id: id.clone(),
        stop_file: Some(stop_file.clone()),
    };
    claim_job(state.inner(), active)?;

    let app_handle = app.clone();
    let run_state = state.inner().clone();
    let worker_id = id.clone();
    thread::spawn(move || {
        let _stop_token = stop_token;
        let mut log_path = None;
        let mut report_path = None;
        let mut error = None;
        let mut exit_code = None;
        match engine_command(&app_handle, &args).and_then(|mut command| {
            command.stdout(Stdio::piped()).stderr(Stdio::piped());
            crate::process_output::spawn_owned(&mut command)
                .map_err(|error| format!("Could not start the rebuild engine: {error}"))
        }) {
            Ok(mut child) => {
                let resource_app = app_handle.clone();
                let resource_run_id = worker_id.clone();
                let resource_monitor = resource_monitor::monitor_process(
                    child.id(),
                    Some(resource_disk_path),
                    std::time::Duration::from_secs(1),
                    move |snapshot| {
                        let _ = resource_app.emit(
                            "run-resources",
                            RunResourceEvent {
                                run_id: resource_run_id.clone(),
                                snapshot,
                            },
                        );
                    },
                );
                let stderr_thread = child.stderr.take().map(|stream| {
                    forward_lines(
                        app_handle.clone(),
                        "run-line",
                        worker_id.clone(),
                        stream,
                        child.id(),
                    )
                });
                if let Some(stdout) = child.stdout.take() {
                    for line in BufReader::new(stdout).lines() {
                        match line {
                            Ok(line) => {
                                if let Some(value) = line.strip_prefix("Log:") {
                                    log_path = Some(value.trim().to_string());
                                } else if let Some(value) = line.strip_prefix("Run report:") {
                                    report_path = Some(value.trim().to_string());
                                }
                                let _ = app_handle.emit(
                                    "run-line",
                                    RunLine {
                                        run_id: worker_id.clone(),
                                        line,
                                    },
                                );
                            }
                            Err(read_error) => {
                                error =
                                    Some(format!("Could not read rebuild output: {read_error}"));
                                let _ = child.kill();
                                break;
                            }
                        }
                    }
                }
                if let Some(handle) = stderr_thread {
                    match handle.join() {
                        Ok(Err(read_error)) => error = Some(read_error),
                        Err(_) => error = Some("Rebuild diagnostic reader panicked.".into()),
                        Ok(Ok(())) => {}
                    }
                }
                match child.wait() {
                    Ok(status) => exit_code = status.code(),
                    Err(wait_error) => {
                        error = Some(format!(
                            "Could not wait for the rebuild engine: {wait_error}"
                        ))
                    }
                }
                resource_monitor.stop();
            }
            Err(start_error) => error = Some(start_error),
        }
        let stop_requested = stop_control::requested(&stop_file);
        let _ = fs::remove_file(&stop_file);
        let _ = app_handle.emit(
            "run-finished",
            RunFinished {
                run_id: worker_id.clone(),
                exit_code,
                stop_requested,
                log_path,
                report_path,
                error,
            },
        );
        release_job(&run_state, &worker_id);
    });
    Ok(id)
}

#[tauri::command]
fn request_stop(state: State<'_, RunState>, run_id: String) -> Result<(), String> {
    let path = {
        let current = state
            .0
            .lock()
            .map_err(|_| "Could not lock the desktop job state.")?;
        let job = current.as_ref().ok_or("There is no active run.")?;
        if job.id != run_id {
            return Err("That run is no longer active.".into());
        }
        job.stop_file
            .clone()
            .ok_or("This system repair cannot be stopped from the run controls.")?
    };
    stop_control::request(&path)
}

pub(crate) fn executable_in_path(program: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|directory| {
        let mut candidates = vec![directory.join(program)];
        #[cfg(target_os = "windows")]
        if Path::new(program).extension().is_none() {
            if let Some(extensions) = std::env::var_os("PATHEXT") {
                candidates.extend(
                    extensions
                        .to_string_lossy()
                        .split(';')
                        .filter(|extension| !extension.is_empty())
                        .map(|extension| directory.join(format!("{program}{extension}"))),
                );
            }
        }
        candidates.into_iter().any(|candidate| {
            let Ok(metadata) = candidate.metadata() else {
                return false;
            };
            if !metadata.is_file() {
                return false;
            }
            #[cfg(unix)]
            {
                metadata.permissions().mode() & 0o111 != 0
            }
            #[cfg(not(unix))]
            {
                true
            }
        })
    })
}

#[tauri::command]
async fn check_ubuntu_package_updates() -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(dependencies::apt_update_status)
        .await
        .map_err(|error| format!("Ubuntu package update check failed: {error}"))?
}

fn start_apt_command_job(
    app: AppHandle,
    state: &RunState,
    command_parts: Vec<String>,
) -> Result<String, String> {
    start_apt_commands_job(app, state, vec![command_parts])
}

fn start_apt_commands_job(
    app: AppHandle,
    state: &RunState,
    commands: Vec<Vec<String>>,
) -> Result<String, String> {
    if !cfg!(target_os = "linux") {
        return Err("APT package operations are available only on Linux hosts.".into());
    }
    if commands.is_empty() || commands.iter().any(Vec::is_empty) {
        return Err("The Ubuntu package operation has no valid commands.".into());
    }
    let id = new_job_id();
    claim_job(
        state,
        ActiveJob {
            id: id.clone(),
            stop_file: None,
        },
    )?;
    let run_state = state.clone();
    let worker_id = id.clone();
    thread::spawn(move || {
        let app_handle = app;
        let logger = Arc::new(match tool_setup_log_directory() {
            Ok(directory) => tool_diagnostics::ToolSetupLogger::create(
                &directory,
                &worker_id,
                "Ubuntu package setup",
            ),
            Err(error) => tool_diagnostics::ToolSetupLogger::unavailable(error),
        });
        let mut success = false;
        let mut error_message = None;
        for (index, mut command_parts) in commands.into_iter().enumerate() {
            if unsafe { libc_geteuid() } != 0 {
                command_parts.insert(0, "pkexec".into());
            }
            let Some(program) = command_parts.first().cloned() else {
                error_message = Some("The Ubuntu package operation has no executable.".into());
                break;
            };
            let stage = format!(
                "Package setup step {} · {}",
                index + 1,
                command_parts.join(" ")
            );
            emit_repair_line(&app_handle, &logger, &worker_id, stage);
            match crate::process_output::spawn_owned(
                Command::new(program)
                    .args(command_parts.iter().skip(1))
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped()),
            ) {
                Ok(mut child) => {
                    let stderr_thread = child.stderr.take().map(|stream| {
                        forward_repair_lines(
                            app_handle.clone(),
                            worker_id.clone(),
                            logger.clone(),
                            stream,
                            child.id(),
                        )
                    });
                    let mut output_result = Ok(());
                    if let Some(stdout) = child.stdout.take() {
                        output_result =
                            crate::process_output::read_process_lines(stdout, child.id(), |line| {
                                emit_repair_line(&app_handle, &logger, &worker_id, line);
                            });
                    }
                    let diagnostic_result = match stderr_thread {
                        Some(handle) => handle
                            .join()
                            .unwrap_or_else(|_| Err("Package diagnostic reader panicked.".into())),
                        None => Ok(()),
                    };
                    let reader_result = output_result.and(diagnostic_result);
                    match child.wait() {
                        Ok(_) if reader_result.is_err() => {
                            success = false;
                            error_message = reader_result.err();
                        }
                        Ok(status) if status.success() => success = true,
                        Ok(status) => {
                            error_message =
                                Some(format!("APT package operation exited with {status}."));
                        }
                        Err(error) => {
                            error_message = Some(format!("Could not wait for APT: {error}"));
                        }
                    }
                }
                Err(error) => {
                    error_message = Some(format!(
                        "Could not start the APT package operation: {error}"
                    ));
                }
            }
            if error_message.is_some() {
                success = false;
                break;
            }
        }
        logger.finish(success, error_message.as_deref());
        let _ = app_handle.emit(
            "repair-finished",
            json!({"jobId": worker_id, "success": success, "error": error_message, "logWarning": logger.warning()}),
        );
        release_job(&run_state, &worker_id);
    });
    Ok(id)
}

fn start_apt_update_job(
    app: AppHandle,
    state: &RunState,
    packages: Vec<String>,
) -> Result<String, String> {
    let mut operation = vec![
        "apt-get".to_string(),
        "install".into(),
        "--only-upgrade".into(),
        "--no-install-recommends".into(),
        "--no-remove".into(),
        "--yes".into(),
        "--".into(),
    ];
    operation.extend(packages);
    start_apt_command_job(app, state, operation)
}

#[tauri::command]
async fn refresh_ubuntu_package_catalog(
    app: AppHandle,
    state: State<'_, RunState>,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(|| -> Result<(), String> {
        let environment = environment::inspect_environment(ToolPaths::default(), None)?;
        if environment
            .pointer("/os/supported")
            .and_then(Value::as_bool)
            != Some(true)
        {
            return Err(
                "APT catalog refresh is limited to supported Ubuntu and Linux Mint releases."
                    .into(),
            );
        }
        if environment
            .pointer("/os/package_manager")
            .and_then(Value::as_str)
            != Some("apt")
            || environment
                .pointer("/os/package_manager_available")
                .and_then(Value::as_bool)
                != Some(true)
        {
            return Err(
                "APT catalog refresh is available only when APT is installed on Linux.".into(),
            );
        }
        if unsafe { libc_geteuid() } != 0 && !executable_in_path("pkexec") {
            return Err("Desktop administrator authentication (pkexec) is not installed.".into());
        }
        Ok(())
    })
    .await
    .map_err(|error| format!("APT catalog refresh preflight failed: {error}"))??;
    start_apt_command_job(app, state.inner(), vec!["apt-get".into(), "update".into()])
}

#[tauri::command]
async fn upgrade_ubuntu_packages(
    app: AppHandle,
    state: State<'_, RunState>,
    confirmed_package_names: Vec<String>,
) -> Result<String, String> {
    let packages = tauri::async_runtime::spawn_blocking(move || -> Result<Vec<String>, String> {
        let status = dependencies::apt_update_status()?;
        let packages = validate_apt_update_confirmation(&status, confirmed_package_names)?;
        validate_apt_upgrade_plan(&packages)?;
        if unsafe { libc_geteuid() } != 0 && !executable_in_path("pkexec") {
            return Err("Desktop administrator authentication (pkexec) is not installed.".into());
        }
        Ok(packages)
    })
    .await
    .map_err(|error| format!("APT update preflight failed: {error}"))??;
    start_apt_update_job(app, state.inner(), packages)
}

#[tauri::command]
async fn install_ubuntu_toolchain(
    app: AppHandle,
    state: State<'_, RunState>,
    tool_paths: ToolPaths,
    confirmed_missing_packages: Vec<String>,
    confirmed_update_packages: Vec<String>,
) -> Result<String, String> {
    let (missing, updates) = tauri::async_runtime::spawn_blocking(move || -> Result<_, String> {
        let environment = environment::inspect_environment(tool_paths, None)?;
        if environment
            .pointer("/os/supported")
            .and_then(Value::as_bool)
            != Some(true)
        {
            return Err(
                "Toolchain package repair is limited to supported Ubuntu and Linux Mint releases."
                    .into(),
            );
        }
        let status = dependencies::apt_update_status()?;
        let (missing, updates) = validate_apt_toolchain_confirmation(
            &environment,
            &status,
            confirmed_missing_packages,
            confirmed_update_packages,
        )?;
        if unsafe { libc_geteuid() } != 0 && !executable_in_path("pkexec") {
            return Err("Desktop administrator authentication (pkexec) is not installed.".into());
        }
        Ok((missing, updates))
    })
    .await
    .map_err(|error| format!("Ubuntu toolchain setup preflight failed: {error}"))??;

    let mut commands = Vec::new();
    if !missing.is_empty() {
        let mut install = vec![
            "apt-get".into(),
            "install".into(),
            "--yes".into(),
            "--no-upgrade".into(),
            "--no-install-recommends".into(),
            "--no-remove".into(),
            "--".into(),
        ];
        install.extend(missing);
        commands.push(install);
    }
    if !updates.is_empty() {
        let mut upgrade = vec![
            "apt-get".into(),
            "install".into(),
            "--only-upgrade".into(),
            "--no-install-recommends".into(),
            "--no-remove".into(),
            "--yes".into(),
            "--".into(),
        ];
        upgrade.extend(updates);
        commands.push(upgrade);
    }
    start_apt_commands_job(app, state.inner(), commands)
}

#[tauri::command]
fn install_apt_tools(
    app: AppHandle,
    state: State<'_, RunState>,
    packages: Vec<String>,
) -> Result<String, String> {
    if !cfg!(target_os = "linux") {
        return Err("APT package operations are available only on Linux hosts.".into());
    }
    if packages.is_empty()
        || packages
            .iter()
            .any(|package| !APT_ALLOWLIST.contains(&package.as_str()))
    {
        return Err("APT repair accepts only the application’s fixed Ubuntu package list.".into());
    }
    let id = new_job_id();
    claim_job(
        state.inner(),
        ActiveJob {
            id: id.clone(),
            stop_file: None,
        },
    )?;
    let run_state = state.inner().clone();
    let worker_id = id.clone();
    thread::spawn(move || {
        let app_handle = app;
        let logger = Arc::new(match tool_setup_log_directory() {
            Ok(directory) => tool_diagnostics::ToolSetupLogger::create(
                &directory,
                &worker_id,
                "Ubuntu package repair",
            ),
            Err(error) => tool_diagnostics::ToolSetupLogger::unavailable(error),
        });
        let mut last_error = None;
        let prefix = if cfg!(target_os = "linux") && unsafe { libc_geteuid() } != 0 {
            Some("pkexec")
        } else {
            None
        };
        let commands = [vec!["apt-get".to_string(), "update".to_string()], {
            let mut command = vec![
                "apt-get".into(),
                "install".into(),
                "--yes".into(),
                "--no-upgrade".into(),
                "--".into(),
            ];
            command.extend(packages);
            command
        }];
        let mut success = true;
        for mut parts in commands {
            if let Some(elevation) = prefix {
                parts.insert(0, elevation.to_string());
            }
            let Some(program) = parts.first().cloned() else {
                continue;
            };
            emit_repair_line(
                &app_handle,
                &logger,
                &worker_id,
                format!("Running package repair command: {}", parts.join(" ")),
            );
            let mut command = Command::new(program);
            command
                .args(parts.iter().skip(1))
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            match crate::process_output::spawn_owned(&mut command) {
                Ok(mut child) => {
                    let stderr_thread = child.stderr.take().map(|stream| {
                        forward_repair_lines(
                            app_handle.clone(),
                            worker_id.clone(),
                            logger.clone(),
                            stream,
                            child.id(),
                        )
                    });
                    let mut output_result = Ok(());
                    if let Some(stdout) = child.stdout.take() {
                        output_result =
                            crate::process_output::read_process_lines(stdout, child.id(), |line| {
                                emit_repair_line(&app_handle, &logger, &worker_id, line);
                            });
                    }
                    let diagnostic_result = match stderr_thread {
                        Some(handle) => handle
                            .join()
                            .unwrap_or_else(|_| Err("Package diagnostic reader panicked.".into())),
                        None => Ok(()),
                    };
                    let reader_result = output_result.and(diagnostic_result);
                    match child.wait() {
                        Ok(_) if reader_result.is_err() => {
                            success = false;
                            last_error = reader_result.err();
                        }
                        Ok(status) if status.success() => {}
                        Ok(status) => {
                            success = false;
                            last_error = Some(format!(
                                "APT exited with status {status}; inspect the repair output."
                            ));
                            break;
                        }
                        Err(error) => {
                            success = false;
                            last_error = Some(format!("Could not wait for APT: {error}"));
                            break;
                        }
                    }
                }
                Err(error) => {
                    success = false;
                    last_error = Some(format!("Could not start the package manager: {error}"));
                    break;
                }
            }
        }
        logger.finish(success, last_error.as_deref());
        let _ = app_handle.emit(
            "repair-finished",
            json!({"jobId": worker_id, "success": success, "error": last_error, "logWarning": logger.warning()}),
        );
        release_job(&run_state, &worker_id);
    });
    Ok(id)
}

#[cfg(target_os = "linux")]
unsafe fn libc_geteuid() -> u32 {
    unsafe extern "C" {
        fn geteuid() -> u32;
    }
    geteuid()
}

#[cfg(not(target_os = "linux"))]
unsafe fn libc_geteuid() -> u32 {
    1
}

#[tauri::command]
async fn install_hdr10plus(
    app: AppHandle,
    state: State<'_, RunState>,
    job_id: String,
) -> Result<String, String> {
    tool_diagnostics::validate_job_id(&job_id)?;
    let log_directory = tool_setup_log_directory();
    let id = new_job_id();
    claim_job(
        state.inner(),
        ActiveJob {
            id: id.clone(),
            stop_file: None,
        },
    )?;
    let run_state = state.inner().clone();
    let logger = Arc::new(match log_directory {
        Ok(directory) => tool_diagnostics::ToolSetupLogger::create(
            &directory,
            &job_id,
            "HDR10+ helper installation",
        ),
        Err(error) => tool_diagnostics::ToolSetupLogger::unavailable(error),
    });
    logger.record("Downloading the helper release and verifying its published digest.");
    let task =
        tauri::async_runtime::spawn_blocking(dependencies::install_latest_hdr10plus_tool).await;
    let result = match task {
        Ok(result) => result,
        Err(error) => Err(format!("HDR10+ helper setup task failed: {error}")),
    };
    match &result {
        Ok(path) => logger.record(&format!("HDR10+ helper installed at {path}.")),
        Err(error) => logger.record(&format!("ERROR: {error}")),
    }
    logger.finish(result.is_ok(), result.as_ref().err().map(String::as_str));
    if let Some(warning) = logger.warning() {
        let _ = app.emit(
            "repair-log-warning",
            RunLine {
                run_id: job_id,
                line: warning,
            },
        );
    }
    release_job(&run_state, &id);
    result
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(RunState::default())
        .invoke_handler(tauri::generate_handler![
            load_app_preferences,
            save_app_preferences,
            inspect_environment,
            open_file_location,
            metadata_filename_clue,
            metadata_filename_clues,
            search_public_films,
            load_public_film_details,
            search_public_tv_shows,
            load_public_tv_details,
            search_tmdb_movies,
            search_tmdb_tv,
            load_tmdb_movie_details,
            load_tmdb_tv_details,
            tmdb_tv_certification_for_tvdb,
            tmdb_credential_status,
            save_tmdb_read_token,
            reveal_tmdb_read_token,
            remove_tmdb_read_token,
            tvdb_credential_status,
            save_tvdb_credentials,
            reveal_tvdb_credentials,
            remove_tvdb_credentials,
            search_tvdb_titles,
            load_tvdb_details,
            save_library_artwork,
            cache_library_artwork,
            save_local_library_artwork,
            cache_local_library_artwork,
            inspect_app_artwork_cache,
            purge_unused_app_artwork_cache,
            scan_local_library_assets,
            read_local_library_artwork_preview,
            preview_library_metadata_write,
            apply_library_metadata_write,
            scan_organizer_folder,
            preview_library_rename,
            apply_library_rename,
            default_media_root,
            load_settings,
            scan_folder,
            load_library_queue,
            reconcile_interrupted_runs,
            save_library_queue,
            save_library_queue_selection,
            save_library_queue_movie_reviews,
            save_library_queue_series_reviews,
            save_library_queue_metadata_suggestions,
            preview_queue_output_targets,
            list_reports,
            delete_reports,
            list_tool_setup_logs,
            read_tool_setup_log,
            export_tool_setup_log,
            read_log,
            start_run,
            request_stop,
            check_ubuntu_package_updates,
            refresh_ubuntu_package_catalog,
            upgrade_ubuntu_packages,
            install_ubuntu_toolchain,
            install_apt_tools,
            install_hdr10plus,
        ])
        .run(tauri::generate_context!())
        .expect("error while running HEVC Rebuild Studio");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_module_command_ignores_appimage_python_home() {
        let command = python_module_command("hevc_rebuild");
        assert!(command
            .get_envs()
            .any(|(name, value)| name == std::ffi::OsStr::new("PYTHONHOME") && value.is_none()));
        assert!(command.get_envs().any(|(name, value)| name
            == std::ffi::OsStr::new("PYTHONUNBUFFERED")
            && value == Some(std::ffi::OsStr::new("1"))));
    }

    #[test]
    fn gui_progress_lines_become_bounded_typed_events() {
        let line = concat!(
            "#HEVC_REBUILD_PROGRESS\t",
            r#"{"fileName":"Film.mkv","filePath":"/media/Film.mkv","phase":"Encoding","batchNumber":2,"batchTotal":4,"filePercent":40.0,"queuePercent":35.0,"speed":1.5,"etaSeconds":60.0,"elapsedSeconds":25.0,"inputBytes":1000,"outputBytes":400,"estimatedOutputBytes":1000}"#
        );
        let event = parse_engine_progress("run-123", line).expect("parse valid GUI progress");
        assert_eq!(event.run_id, "run-123");
        assert_eq!(event.file_name, "Film.mkv");
        assert_eq!(event.file_path.as_deref(), Some("/media/Film.mkv"));
        assert_eq!(event.phase, "Encoding");
        assert_eq!(event.batch_number, 2);
        assert_eq!(event.batch_total, 4);
        assert_eq!(event.file_percent, Some(40.0));
        assert_eq!(event.queue_percent, Some(35.0));
        assert_eq!(event.input_bytes, Some(1000));
        assert_eq!(event.estimated_output_bytes, Some(1000));
    }

    #[test]
    fn malformed_or_out_of_range_gui_progress_stays_a_diagnostic_line() {
        assert!(parse_engine_progress("run-123", "ordinary engine output").is_none());
        let invalid = concat!(
            "#HEVC_REBUILD_PROGRESS\t",
            r#"{"fileName":"Film.mkv","filePath":"relative/Film.mkv","phase":"Encoding","batchNumber":1,"batchTotal":1,"filePercent":101.0,"queuePercent":20.0,"speed":1.0,"etaSeconds":1.0,"elapsedSeconds":1.0,"inputBytes":1000,"outputBytes":400,"estimatedOutputBytes":1000}"#
        );
        assert!(parse_engine_progress("run-123", invalid).is_none());
    }

    #[test]
    fn metadata_blacklist_is_forwarded_as_individual_engine_arguments() {
        let settings = json!({"metadata": {"text_blacklist": ["Movie*Group", "Explicit"]}});
        assert_eq!(
            metadata_text_blacklist_arguments(&settings).unwrap(),
            vec![
                "--embed-artwork",
                "--replace-metadata-text-blacklist",
                "--metadata-text-blacklist",
                "Movie*Group",
                "--metadata-text-blacklist",
                "Explicit"
            ]
        );
        assert!(metadata_text_blacklist_arguments(&json!({"metadata": {}}))
            .unwrap()
            .eq(&vec!["--embed-artwork"]));
        assert_eq!(
            metadata_text_blacklist_arguments(&json!({"metadata": {"text_blacklist": []}}))
                .unwrap(),
            vec!["--embed-artwork", "--replace-metadata-text-blacklist"]
        );
    }

    #[test]
    fn metadata_blacklist_rejects_empty_wildcard_and_duplicate_patterns() {
        assert!(metadata_text_blacklist_arguments(
            &json!({"metadata": {"text_blacklist": ["***"]}})
        )
        .is_err());
        assert!(metadata_text_blacklist_arguments(
            &json!({"metadata": {"text_blacklist": ["Movie*Group", "movie*group"]}})
        )
        .is_err());
    }

    #[test]
    fn metadata_blacklist_rejects_oversized_and_controlled_patterns() {
        assert!(metadata_text_blacklist_arguments(
            &json!({"metadata": {"text_blacklist": ["x".repeat(129)]}})
        )
        .is_err());
        assert!(metadata_text_blacklist_arguments(
            &json!({"metadata": {"text_blacklist": ["bad\npattern"]}})
        )
        .is_err());
    }

    #[test]
    fn builtin_secondary_video_default_and_all_supported_policies_reach_engine() {
        let defaults = settings::default_settings();
        let args = settings_arguments(&defaults)
            .expect("the built-in settings must pass the Rust engine adapter");
        let option = args
            .windows(2)
            .find(|pair| pair[0] == "--secondary-video")
            .expect("secondary video policy is forwarded");
        assert_eq!(option[1], "encode");

        for policy in ["encode", "copy", "drop"] {
            let mut configured = settings::default_settings();
            configured["streams"]["secondary_video"] = json!(policy);
            let args = settings_arguments(&configured)
                .expect("all documented secondary video policies must be accepted");
            let option = args
                .windows(2)
                .find(|pair| pair[0] == "--secondary-video")
                .expect("secondary video policy is forwarded");
            assert_eq!(option[1], policy);
        }
    }

    #[test]
    fn selected_media_files_are_canonical_and_confined_to_the_analyzed_root() {
        let root = test_dir("selected-media-root");
        let outside = test_dir("selected-media-outside");
        let source = root.join("film.mkv");
        let other = outside.join("outside.mkv");
        fs::write(&source, b"fixture").unwrap();
        fs::write(&other, b"fixture").unwrap();
        let mut args = Vec::new();
        append_selected_file_arguments(&mut args, &root, &[source.to_string_lossy().into_owned()])
            .unwrap();
        assert_eq!(args[0], "--selected-file");
        assert_eq!(
            args[1],
            source
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        );
        assert!(append_selected_file_arguments(
            &mut Vec::new(),
            &root,
            &[other.to_string_lossy().into_owned()]
        )
        .is_err());
        assert!(
            append_selected_file_arguments(&mut Vec::new(), &root, &["relative.mkv".into()])
                .is_err()
        );
        let duplicate = source.to_string_lossy().into_owned();
        assert!(append_selected_file_arguments(
            &mut Vec::new(),
            &root,
            &[duplicate.clone(), duplicate]
        )
        .is_err());
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }

    #[test]
    fn every_production_python_script_is_listed_in_the_python_rust_conversion_ledger() {
        fn collect_python_files(directory: &Path, root: &Path, files: &mut BTreeSet<String>) {
            let entries = fs::read_dir(directory).expect("read Python source directory");
            for entry in entries {
                let entry = entry.expect("read Python source entry");
                let path = entry.path();
                let metadata = entry.file_type().expect("inspect Python source entry");
                if metadata.is_dir() {
                    if entry.file_name() != "__pycache__" {
                        collect_python_files(&path, root, files);
                    }
                } else if metadata.is_file() && path.extension().is_some_and(|ext| ext == "py") {
                    files.insert(
                        path.strip_prefix(root)
                            .expect("Python source remains inside the repository")
                            .to_string_lossy()
                            .replace('\\', "/"),
                    );
                }
            }
        }

        let crate_root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let project_root = crate_root
            .parent()
            .and_then(Path::parent)
            .expect("resolve repository root from the Tauri crate");
        let mut production_sources = BTreeSet::new();
        collect_python_files(
            &project_root.join("hevc_rebuild"),
            project_root,
            &mut production_sources,
        );
        let root_entrypoint = project_root.join("hevc_rebuild.py");
        if root_entrypoint.is_file() {
            production_sources.insert("hevc_rebuild.py".into());
        }
        let conversion_ledger =
            fs::read_to_string(project_root.join("docs/maintainer/python-rust-convert.md"))
                .expect("read the Python-to-Rust conversion ledger");
        let missing = production_sources
            .iter()
            .filter(|source| !conversion_ledger.contains(&format!("`{source}`")))
            .cloned()
            .collect::<Vec<_>>();

        assert!(
            missing.is_empty(),
            "Python production files missing from python-rust-convert.md: {missing:?}"
        );
    }

    fn test_dir(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("hevc-rebuild-{label}-{}", new_job_id()));
        fs::create_dir_all(&path).expect("create isolated test directory");
        path
    }

    #[test]
    fn poster_override_arguments_validate_and_forward_scoped_images() {
        let root = test_dir("poster-overrides");
        let media = root.join("Film.mkv");
        let artwork_dir = root.join("Artwork");
        fs::create_dir_all(&artwork_dir).unwrap();
        fs::write(&media, b"media fixture").unwrap();
        let poster = artwork_dir.join("selected.png");
        fs::write(&poster, b"\x89PNG\r\n\x1a\n\x00\x00\x00\x00").unwrap();

        let args = poster_override_arguments(
            &root,
            BTreeMap::from([(
                media.to_string_lossy().into_owned(),
                poster.to_string_lossy().into_owned(),
            )]),
            &root.join("private-cache/artwork"),
        )
        .expect("accept a valid image under the selected library root");
        assert_eq!(args[0], "--file-poster-overrides-json");
        let decoded: BTreeMap<String, String> = serde_json::from_str(&args[1]).unwrap();
        assert_eq!(
            decoded.get(&media.canonicalize().unwrap().to_string_lossy().into_owned()),
            Some(
                &poster
                    .canonicalize()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            )
        );

        let external = test_dir("poster-outside").join("outside.jpg");
        fs::write(&external, b"\xff\xd8\xff".repeat(5)).unwrap();
        assert!(poster_override_arguments(
            &root,
            BTreeMap::from([(
                media.to_string_lossy().into_owned(),
                external.to_string_lossy().into_owned(),
            )]),
            &root.join("private-cache/artwork"),
        )
        .unwrap_err()
        .contains("analyzed input"));

        let cache = root.join("private-cache/artwork");
        fs::create_dir_all(&cache).unwrap();
        let cached_poster = cache.join("poster.webp");
        fs::write(&cached_poster, b"RIFF0000WEBPVP8 ").unwrap();
        let args = poster_override_arguments(
            &root,
            BTreeMap::from([(
                media.to_string_lossy().into_owned(),
                cached_poster.to_string_lossy().into_owned(),
            )]),
            &cache,
        )
        .expect("accept valid artwork from the approved private cache");
        assert_eq!(args[0], "--allowed-poster-cache-root");
        assert_eq!(args[2], "--file-poster-overrides-json");

        fs::remove_dir_all(root).unwrap();
        let external_root = external.parent().unwrap();
        fs::remove_dir_all(external_root).unwrap();
    }

    #[test]
    fn metadata_override_arguments_validate_scope_and_forward_reviewed_fields() {
        let root = test_dir("metadata-overrides");
        let media = root.join("Film.mkv");
        fs::write(&media, b"media fixture").unwrap();
        let args = metadata_override_arguments(
            &root,
            BTreeMap::from([(
                media.to_string_lossy().into_owned(),
                json!({"title": "Reviewed title", "genres": ["Drama", "Animation"]}),
            )]),
        )
        .expect("accept reviewed fields for a file inside the selected library");
        assert_eq!(args[0], "--file-metadata-overrides-json");
        let decoded: BTreeMap<String, Value> = serde_json::from_str(&args[1]).unwrap();
        let canonical = media.canonicalize().unwrap().to_string_lossy().into_owned();
        assert_eq!(decoded[&canonical]["title"], "Reviewed title");
        assert_eq!(decoded[&canonical]["genres"][0], "Drama");

        let outside_root = test_dir("metadata-overrides-outside");
        let outside = outside_root.join("Outside.mkv");
        fs::write(&outside, b"fixture").unwrap();
        assert!(metadata_override_arguments(
            &root,
            BTreeMap::from([(
                outside.to_string_lossy().into_owned(),
                json!({"title": "Outside"}),
            )]),
        )
        .unwrap_err()
        .contains("inside the analyzed input"));
        assert!(metadata_override_arguments(
            &root,
            BTreeMap::from([(media.to_string_lossy().into_owned(), json!({}))]),
        )
        .is_err());

        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(outside_root).unwrap();
    }

    #[test]
    fn youtube_metadata_loss_arguments_are_canonical_and_scoped() {
        let root = test_dir("hdr10plus-root");
        let source = root.join("source.mkv");
        fs::write(&source, b"fixture").expect("create test source");
        let arguments =
            youtube_hdr10plus_loss_arguments(&root, vec![source.to_string_lossy().into_owned()])
                .expect("accept consent for a source in the selected root");
        let canonical = source
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert_eq!(
            arguments,
            vec![String::from("--allow-youtube-hdr10plus-loss"), canonical]
        );
        fs::remove_dir_all(root).expect("remove test directory");
    }

    #[test]
    fn youtube_metadata_loss_arguments_reject_duplicates_and_out_of_root_paths() {
        let root = test_dir("hdr10plus-scope");
        let outside = test_dir("hdr10plus-outside");
        let source = root.join("source.mkv");
        let other = outside.join("outside.mkv");
        fs::write(&source, b"fixture").expect("create in-root source");
        fs::write(&other, b"fixture").expect("create outside source");
        let raw_source = source.to_string_lossy().into_owned();
        assert!(
            youtube_hdr10plus_loss_arguments(&root, vec![raw_source.clone(), raw_source]).is_err()
        );
        assert!(youtube_hdr10plus_loss_arguments(
            &root,
            vec![other.to_string_lossy().into_owned()]
        )
        .is_err());
        fs::remove_dir_all(root).expect("remove root fixture");
        fs::remove_dir_all(outside).expect("remove outside fixture");
    }

    #[test]
    fn apt_upgrade_confirmation_must_match_the_fresh_exact_list() {
        let status = json!({
            "catalog_stale": false,
            "packages": [
                {"name": "ffmpeg"},
                {"name": "mediainfo"}
            ]
        });
        let approved =
            validate_apt_update_confirmation(&status, vec!["mediainfo".into(), "ffmpeg".into()])
                .expect("accept exact package list independent of order");
        assert_eq!(approved, vec!["ffmpeg", "mediainfo"]);
        assert!(validate_apt_update_confirmation(&status, vec!["ffmpeg".into()]).is_err());
        assert!(validate_apt_update_confirmation(
            &status,
            vec!["ffmpeg".into(), "ffmpeg".into(), "mediainfo".into()]
        )
        .is_err());
    }

    #[test]
    fn apt_toolchain_setup_requires_exact_missing_package_confirmation() {
        let environment = json!({"missing_apt_packages": ["mediainfo", "ffmpeg"]});
        let approved = validate_apt_missing_confirmation(
            &environment,
            vec!["ffmpeg".into(), "mediainfo".into()],
        )
        .expect("accept the exact missing package list independent of order");
        assert_eq!(approved, vec!["ffmpeg", "mediainfo"]);
        assert!(validate_apt_missing_confirmation(&environment, vec!["ffmpeg".into()]).is_err());
        assert!(validate_apt_missing_confirmation(
            &environment,
            vec!["ffmpeg".into(), "ffmpeg".into(), "mediainfo".into()]
        )
        .is_err());
        assert!(validate_apt_missing_confirmation(
            &json!({"missing_apt_packages": ["unlisted-package"]}),
            vec!["unlisted-package".into()]
        )
        .is_err());
    }

    #[test]
    fn apt_toolchain_setup_rejects_stale_catalog_before_any_package_plan() {
        let error = validate_apt_toolchain_confirmation(
            &json!({"missing_apt_packages": []}),
            &json!({"catalog_stale": true, "packages": []}),
            Vec::new(),
            Vec::new(),
        )
        .expect_err("stale package catalogs must be refreshed before changes are reviewed");
        assert!(error.contains("stale"));
    }

    #[test]
    fn apt_upgrade_cli_parser_requires_confirmation_and_explicit_packages() {
        assert!(parse_apt_upgrade_cli_args(&["--package".into(), "ffmpeg".into()]).is_err());
        assert!(parse_apt_upgrade_cli_args(&["--confirm".into()]).is_err());
        assert!(parse_apt_upgrade_cli_args(&["--confirm".into(), "--confirm".into()]).is_err());
        assert!(parse_apt_upgrade_cli_args(&["--confirm".into(), "--unexpected".into()]).is_err());
    }

    #[test]
    fn apt_upgrade_cli_parser_accepts_repeated_exact_package_arguments() {
        let packages = parse_apt_upgrade_cli_args(&[
            "--confirm".into(),
            "--package".into(),
            "ffmpeg".into(),
            "--package=mediainfo".into(),
        ])
        .expect("parse explicit repeated package arguments");
        assert_eq!(packages, vec!["ffmpeg", "mediainfo"]);
    }

    #[test]
    fn apt_upgrade_confirmation_rejects_a_stale_catalog() {
        let status = json!({"catalog_stale": true, "packages": [{"name": "ffmpeg"}]});
        assert!(validate_apt_update_confirmation(&status, vec!["ffmpeg".into()]).is_err());
    }

    #[test]
    fn organizer_templates_are_saved_and_missing_values_keep_defaults() {
        let preferences = normalize_app_preferences(json!({
            "organizerTemplates": {"movie": "{title}{ext}"}
        }))
        .expect("accept valid organizer preferences");
        assert_eq!(preferences["organizerTemplates"]["movie"], "{title}{ext}");
        assert_eq!(
            preferences["organizerTemplates"]["series"],
            "{series_title}"
        );
        assert_eq!(
            preferences["organizerTemplates"]["season"],
            "Season {season:02}"
        );
    }

    #[test]
    fn organizer_templates_reject_empty_or_oversized_patterns() {
        assert!(normalize_app_preferences(json!({
            "organizerTemplates": {"movie": "  "}
        }))
        .is_err());
        assert!(normalize_app_preferences(json!({
            "organizerTemplates": {"episode": "x".repeat(1025)}
        }))
        .is_err());
    }

    #[test]
    fn rebuild_output_preferences_default_to_in_place_and_normalize_folder_maps() {
        let defaults = default_app_preferences();
        assert!(defaults["rebuildOutputDir"].is_null());
        assert_eq!(defaults["fileOutputDirs"], json!({}));

        let normalized = normalize_app_preferences(json!({
            "rebuildOutputDir": "/tmp/rebuild-output",
            "fileOutputDirs": {"/tmp/source.mkv": "/tmp/one-file"}
        }))
        .expect("accept absolute output destinations");
        assert_eq!(normalized["rebuildOutputDir"], "/tmp/rebuild-output");
        assert_eq!(
            normalized["fileOutputDirs"]["/tmp/source.mkv"],
            "/tmp/one-file"
        );
        assert!(normalize_app_preferences(json!({"rebuildOutputDir": "relative"})).is_err());
        assert!(normalize_app_preferences(json!({
            "fileOutputDirs": {"relative.mkv": "/tmp/output"}
        }))
        .is_err());
    }

    #[test]
    fn saved_workspace_tab_defaults_to_home_and_rejects_unknown_pages() {
        let defaults = default_app_preferences();
        assert_eq!(defaults["lastPage"], "home");
        let restored = normalize_app_preferences(json!({"lastPage": "library"}))
            .expect("restore a known workspace tab");
        assert_eq!(restored["lastPage"], "library");
        assert!(normalize_app_preferences(json!({"lastPage": "dashboard"})).is_err());
    }

    #[test]
    fn window_preferences_default_wide_and_reject_unusable_dimensions() {
        let defaults = default_app_preferences();
        assert_eq!(defaults["windowState"]["width"], 1600);
        assert_eq!(defaults["windowState"]["height"], 1000);

        let restored = normalize_app_preferences(json!({
            "windowState": {"width": 1720, "height": 1080, "maximized": true}
        }))
        .expect("restore a previously saved window state");
        assert_eq!(
            restored["windowState"],
            json!({"width": 1720, "height": 1080, "maximized": true})
        );
        assert!(normalize_app_preferences(json!({
            "windowState": {"width": 900, "height": 900, "maximized": false}
        }))
        .is_err());
        assert!(normalize_app_preferences(json!({
            "windowState": {"width": 1600, "height": 500, "maximized": false}
        }))
        .is_err());
    }

    #[test]
    fn legacy_profiles_receive_removable_renamer_exclusions_once() {
        let migrated = normalize_app_preferences(json!({
            "filenameExcludePatterns": ["my-custom-extra"]
        }))
        .expect("migrate a legacy preference profile");
        let exclusions = migrated["filenameExcludePatterns"]
            .as_array()
            .expect("migrated exclusions are an array");
        assert!(exclusions.iter().any(|pattern| pattern == "old"));
        assert!(exclusions.iter().any(|pattern| pattern == ".*"));
        assert!(exclusions
            .iter()
            .any(|pattern| pattern == "behindthescenes"));
        assert!(exclusions
            .iter()
            .any(|pattern| pattern == "my-custom-extra"));
        assert_eq!(migrated["filenameExcludeDefaultsVersion"], 2);

        let cleared = normalize_app_preferences(json!({
            "filenameExcludePatterns": [],
            "filenameExcludeDefaultsVersion": 2
        }))
        .expect("keep a user's explicit clear-all choice");
        assert_eq!(cleared["filenameExcludePatterns"], json!([]));
    }

    #[test]
    fn rebuild_output_arguments_validate_sources_and_forward_canonical_paths() {
        let root = std::env::temp_dir().join(format!("hevc-rebuild-output-test-{}", new_job_id()));
        let source = root.join("Season 1").join("Episode.mkv");
        let output = root.join("output");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::create_dir_all(&output).unwrap();
        fs::write(&source, b"source").unwrap();

        let (args, resource_path) = rebuild_output_arguments(
            &root,
            None,
            BTreeMap::from([(
                source.to_string_lossy().into_owned(),
                output.to_string_lossy().into_owned(),
            )]),
        )
        .expect("build validated output arguments");
        assert_eq!(resource_path, Some(output.canonicalize().unwrap()));
        assert_eq!(args[0], "--file-output-dirs-json");
        let destinations: BTreeMap<String, String> = serde_json::from_str(&args[1]).unwrap();
        assert_eq!(
            destinations.get(
                &source
                    .canonicalize()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            ),
            Some(
                &output
                    .canonicalize()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            )
        );
        assert!(rebuild_output_arguments(
            &root,
            None,
            BTreeMap::from([(
                "/outside/video.mkv".into(),
                output.to_string_lossy().into_owned()
            )]),
        )
        .is_err());

        fs::remove_dir_all(root).unwrap();
    }
}
