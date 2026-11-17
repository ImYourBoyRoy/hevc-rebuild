//! The installed `json` command: scan, queue, work-directory, artwork-cache, environment and report actions.

use super::*;

pub(crate) fn run_json_cli(args: &[String]) -> i32 {
    if args.is_empty() {
        eprintln!("Choose a JSON action: environment, scan, defaults, work-directory, queue, queue-select, queue-run, output-plan, reports, apt-updates, apt-refresh, apt-upgrade, provision-plan, provision, install-hdr10plus, integrity, check.");
        return 2;
    }
    if args[0] == "output-plan" {
        return output_targets::cli_mode(&args[1..]);
    }
    if args[0] == "scan" {
        return run_json_scan_cli(args);
    }
    if args[0] == "integrity" {
        return integrity::cli_mode(&args[1..]);
    }
    if args[0] == "check" {
        return run_coordinator::cli_mode(&args[1..]);
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
    if args[0] == "provision-plan" {
        return run_json_cli_provision(&args[1..], false);
    }
    if args[0] == "provision" {
        return run_json_cli_provision(&args[1..], true);
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
    if args[0] == "work-directory" {
        return run_json_work_directory_cli(&args[1..]);
    }
    eprintln!(
        "Unknown JSON action {:?}. Available actions: environment, scan, defaults, work-directory, queue, queue-select, queue-run, output-plan, reports, apt-updates, apt-refresh, apt-upgrade, provision-plan, provision, install-hdr10plus, artwork-cache, integrity.",
        args[0]
    );
    2
}

fn run_json_work_directory_cli(args: &[String]) -> i32 {
    let mut root = None;
    let mut work_root = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--root" => {
                let Some(value) = args.get(index + 1) else {
                    eprintln!("work-directory --root requires a path.");
                    return 2;
                };
                if root.replace(value.clone()).is_some() {
                    eprintln!("work-directory --root can be specified only once.");
                    return 2;
                }
                index += 2;
            }
            "--work-root" => {
                let Some(value) = args.get(index + 1) else {
                    eprintln!("work-directory --work-root requires 'auto' or an absolute path.");
                    return 2;
                };
                if work_root.replace(value.clone()).is_some() {
                    eprintln!("work-directory --work-root can be specified only once.");
                    return 2;
                }
                index += 2;
            }
            other => {
                eprintln!("Unknown work-directory option {other}.");
                return 2;
            }
        }
    }
    let Some(root) = root else {
        eprintln!("work-directory requires --root PATH.");
        return 2;
    };
    match resolve_work_directory(&root, work_root.as_deref())
        .and_then(|payload| serde_json::to_string(&payload).map_err(|error| error.to_string()))
    {
        Ok(payload) => {
            println!("{payload}");
            0
        }
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
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
    let mut explicit_exclusions = false;
    let mut stop_file = None;
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
            "--no-default-exclusions" => {
                explicit_exclusions = true;
                index += 1;
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
            "--stop-request-file" => {
                let Some(value) = args.get(index + 1) else {
                    eprintln!("Scan option --stop-request-file requires a path.");
                    return 2;
                };
                if stop_file.replace(PathBuf::from(value)).is_some() {
                    eprintln!("Scan option --stop-request-file can be specified only once.");
                    return 2;
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
    crate::backup_folder::apply_default_exclusions(
        &mut filename_exclude_patterns,
        explicit_exclusions,
    );
    match media_scan::scan_media(
        media_scan::ScanOptions {
            root,
            recursive,
            minimum_size_mib,
            filename_include_patterns,
            filename_exclude_patterns,
            tool_paths,
            stop_file,
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
