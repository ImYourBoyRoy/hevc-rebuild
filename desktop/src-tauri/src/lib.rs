use serde::Serialize;
use serde_json::{json, Value};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager, State};

mod mcp_server;

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

fn standalone_engine_root() -> Result<PathBuf, String> {
    if let Some(configured) = std::env::var_os("HEVC_REBUILD_ENGINE_ROOT") {
        let root = PathBuf::from(configured).canonicalize()
            .map_err(|error| format!("Configured HEVC_REBUILD_ENGINE_ROOT is unavailable: {error}"))?;
        if root.join("hevc_rebuild").is_dir() { return Ok(root); }
        return Err("HEVC_REBUILD_ENGINE_ROOT must contain the hevc_rebuild Python package.".into());
    }

    #[cfg(debug_assertions)]
    {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize()
            .map_err(|error| format!("Could not locate the Python media engine: {error}"))?;
        if root.join("hevc_rebuild").is_dir() { return Ok(root); }
    }

    #[cfg(target_os = "linux")]
    if let Some(app_dir) = std::env::var_os("APPDIR") {
        let candidate = PathBuf::from(app_dir).join("usr/lib/HEVC Rebuild Studio/engine");
        if candidate.join("hevc_rebuild").is_dir() { return Ok(candidate); }
    }

    #[cfg(target_os = "linux")]
    for candidate in [
        PathBuf::from("/usr/lib/HEVC Rebuild Studio/engine"),
        PathBuf::from("/usr/lib/hevc-rebuild-studio/engine"),
        PathBuf::from("/usr/share/hevc-rebuild-studio/engine"),
    ] {
        if candidate.join("hevc_rebuild").is_dir() { return Ok(candidate); }
    }
    Err("Could not locate the Python media engine. Set HEVC_REBUILD_ENGINE_ROOT to its folder.".into())
}

fn standalone_command(module: &str, args: &[String]) -> Result<Command, String> {
    let root = standalone_engine_root()?;
    let mut command = Command::new(python_program());
    command.arg("-m").arg(module).args(args).env("PYTHONUNBUFFERED", "1");
    let mut python_paths = vec![root];
    if let Some(existing) = std::env::var_os("PYTHONPATH") {
        python_paths.extend(std::env::split_paths(&existing));
    }
    let joined = std::env::join_paths(python_paths).map_err(|error| format!("Could not configure Python's module path: {error}"))?;
    command.env("PYTHONPATH", joined);
    Ok(command)
}

fn run_json_cli(args: &[String]) -> i32 {
    if args.is_empty() {
        eprintln!("Choose a JSON action: environment, scan, defaults, reports, install-hdr10plus.");
        return 2;
    }
    let mut command = match standalone_command("hevc_rebuild.bridge", args) {
        Ok(command) => command,
        Err(error) => { eprintln!("{error}"); return 2; }
    };
    match command.output() {
        Ok(output) => {
            if !output.stderr.is_empty() { eprint!("{}", String::from_utf8_lossy(&output.stderr)); }
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                if let Some(progress) = line.strip_prefix("__HEVC_SCAN_PROGRESS__\t") {
                    eprintln!("scan\t{progress}");
                } else if !line.trim().is_empty() {
                    println!("{line}");
                }
            }
            output.status.code().unwrap_or(1)
        }
        Err(error) => { eprintln!("Could not start Python media engine: {error}"); 2 }
    }
}

pub fn cli_mode(mode: &str, args: Vec<std::ffi::OsString>) -> i32 {
    let strings: Vec<String> = args.into_iter().map(|value| value.to_string_lossy().into_owned()).collect();
    if mode == "json" { return run_json_cli(&strings); }
    match standalone_command("hevc_rebuild", &strings) {
        Ok(mut command) => match command.status() {
            Ok(status) => status.code().unwrap_or(1),
            Err(error) => { eprintln!("Could not start Python media engine: {error}"); 2 }
        },
        Err(error) => { eprintln!("{error}"); 2 }
    }
}

pub fn mcp_mode() -> i32 { mcp_server::run() }

fn bridge_command(app: &AppHandle, args: &[String]) -> Result<Command, String> {
    let root = engine_root(app)?;
    let mut command = Command::new(python_program());
    command
        .arg("-m")
        .arg("hevc_rebuild.bridge")
        .args(args)
        .env("PYTHONUNBUFFERED", "1")
        .current_dir(root);
    Ok(command)
}

fn engine_command(app: &AppHandle, args: &[String]) -> Result<Command, String> {
    let root = engine_root(app)?;
    let mut command = Command::new(python_program());
    command.arg("-m").arg("hevc_rebuild").args(args).env("PYTHONUNBUFFERED", "1").current_dir(root);
    Ok(command)
}

fn format_process_error(output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if stderr.is_empty() {
        format!("Python media engine exited with status {}.", output.status)
    } else {
        stderr.chars().take(4000).collect()
    }
}

fn forward_lines<R: Read + Send + 'static>(
    app: AppHandle,
    event: &'static str,
    run_id: String,
    stream: R,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        for line in BufReader::new(stream).lines().flatten() {
            let _ = app.emit(event, RunLine { run_id: run_id.clone(), line });
        }
    })
}

fn run_bridge_json(app: AppHandle, args: Vec<String>, progress: bool) -> Result<Value, String> {
    let mut command = bridge_command(&app, &args)?;
    if !progress {
        let output = command
            .output()
            .map_err(|error| format!("Could not start Python media engine: {error}"))?;
        if !output.status.success() {
            return Err(format_process_error(&output));
        }
        return serde_json::from_slice(&output.stdout)
            .map_err(|error| format!("The media engine returned invalid JSON: {error}"));
    }

    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| format!("Could not start Python media engine: {error}"))?;
    let stdout = child.stdout.take().ok_or("Could not read scan progress.")?;
    let stderr = child.stderr.take().ok_or("Could not read scan diagnostics.")?;
    let stderr_thread = thread::spawn(move || {
        let mut text = String::new();
        let mut reader = BufReader::new(stderr);
        let _ = reader.read_to_string(&mut text);
        text
    });
    let mut json_lines = Vec::new();
    for line in BufReader::new(stdout).lines() {
        let line = line.map_err(|error| format!("Could not read folder analysis: {error}"))?;
        if let Some(rest) = line.strip_prefix("__HEVC_SCAN_PROGRESS__\t") {
            let parts: Vec<&str> = rest.splitn(3, '\t').collect();
            if parts.len() == 3 {
                let _ = app.emit(
                    "scan-progress",
                    json!({
                        "current": parts[0].parse::<usize>().unwrap_or(0),
                        "total": parts[1].parse::<usize>().unwrap_or(0),
                        "file": parts[2],
                    }),
                );
            }
        } else if !line.trim().is_empty() {
            json_lines.push(line);
        }
    }
    let status = child
        .wait()
        .map_err(|error| format!("Could not finish folder analysis: {error}"))?;
    let stderr = stderr_thread.join().unwrap_or_default();
    if !status.success() {
        return Err(if stderr.trim().is_empty() {
            format!("Folder analysis exited with status {status}.")
        } else {
            stderr.chars().take(4000).collect()
        });
    }
    let payload = json_lines
        .last()
        .ok_or_else(|| if stderr.is_empty() { "Folder analysis returned no result.".to_string() } else { stderr })?;
    serde_json::from_str(payload).map_err(|error| format!("Folder analysis returned invalid JSON: {error}"))
}

#[tauri::command]
async fn inspect_environment(app: AppHandle, ffmpeg_path: Option<String>) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut args = vec!["environment".to_string()];
        if let Some(path) = ffmpeg_path.filter(|value| !value.trim().is_empty()) {
            args.extend(["--ffmpeg-path".into(), path]);
        }
        run_bridge_json(app, args, false)
    })
    .await
    .map_err(|error| format!("Environment scan task failed: {error}"))?
}

#[tauri::command]
async fn load_settings(app: AppHandle, root: String) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        run_bridge_json(
            app,
            vec!["defaults".into(), "--root".into(), root],
            false,
        )
    })
    .await
    .map_err(|error| format!("Settings load task failed: {error}"))?
}

#[tauri::command]
fn default_media_root() -> Result<String, String> {
    let home = std::env::var_os("HOME").map(PathBuf::from).ok_or("Could not locate the user's home folder.")?;
    let videos = home.join("Videos");
    Ok(if videos.is_dir() { videos } else { home }.to_string_lossy().into_owned())
}

#[tauri::command]
async fn scan_folder(
    app: AppHandle,
    root: String,
    recursive: bool,
    minimum_size_mib: u32,
    ffmpeg_path: Option<String>,
) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut args = vec![
            "scan".into(),
            "--root".into(),
            root,
            "--min-size-mib".into(),
            minimum_size_mib.to_string(),
        ];
        if recursive {
            args.push("--recursive".into());
        }
        if let Some(path) = ffmpeg_path.filter(|value| !value.trim().is_empty()) {
            args.extend(["--ffmpeg-path".into(), path]);
        }
        run_bridge_json(app, args, true)
    })
    .await
    .map_err(|error| format!("Folder scan task failed: {error}"))?
}

#[tauri::command]
async fn list_reports(app: AppHandle, limit: u8) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        run_bridge_json(
            app,
            vec!["reports".into(), "--limit".into(), limit.clamp(1, 50).to_string()],
            false,
        )
    })
    .await
    .map_err(|error| format!("Report listing task failed: {error}"))?
}

fn state_log_directory() -> Result<PathBuf, String> {
    if let Some(portable) = std::env::var_os("HEVC_REBUILD_PORTABLE_DATA_DIR") {
        return Ok(PathBuf::from(portable).join("logs"));
    }
    if let Some(state_home) = std::env::var_os("XDG_STATE_HOME") {
        return Ok(PathBuf::from(state_home).join("hevc-rebuild/logs"));
    }
    let home = std::env::var_os("HOME").ok_or("Could not locate the current user's home folder.")?;
    Ok(PathBuf::from(home).join(".local/state/hevc-rebuild/logs"))
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
    if let Some(portable) = std::env::var_os("HEVC_REBUILD_PORTABLE_DATA_DIR") {
        return Ok(PathBuf::from(portable).join("settings.json"));
    }
    let config_dir = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .ok_or("Could not locate the current user's settings folder.")?;
    Ok(config_dir.join("hevc-rebuild-studio/settings.json"))
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
        "root": null,
        "ffmpegPath": null,
        "recursive": true,
        "minimumSizeMib": "5",
        "stopOnError": false,
        "settings": null
    })
}

fn normalize_app_preferences(value: Value) -> Result<Value, String> {
    let object = value
        .as_object()
        .ok_or("Application preferences must be a JSON object.")?;
    let mut normalized = default_app_preferences();
    let result = normalized.as_object_mut().expect("default preferences are an object");

    if let Some(theme) = object.get("theme") {
        let theme = theme.as_str().ok_or("Appearance must be light, dark, or hacker.")?;
        if !["light", "dark", "hacker"].contains(&theme) {
            return Err("Appearance must be light, dark, or hacker.".into());
        }
        result.insert("theme".into(), json!(theme));
    }
    for key in ["root", "ffmpegPath"] {
        if let Some(value) = object.get(key) {
            if !value.is_null() && !value.as_str().is_some_and(|text| text.len() <= 8192) {
                return Err(format!("{key} must be a path or null."));
            }
            result.insert(key.into(), value.clone());
        }
    }
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
        if !settings.is_null() { settings_arguments(settings)?; }
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
    let allowed = state_log_directory()?.canonicalize().map_err(|error| format!("Diagnostic log folder is unavailable: {error}"))?;
    let requested = PathBuf::from(path)
        .canonicalize()
        .map_err(|error| format!("Could not open the diagnostic file: {error}"))?;
    if !requested.starts_with(&allowed) || requested.extension().and_then(|value| value.to_str()) != Some("log") {
        return Err("Only HEVC Rebuild log files from this user's log folder can be read here.".into());
    }
    let mut file = File::open(&requested).map_err(|error| format!("Could not read the diagnostic file: {error}"))?;
    let size = file.metadata().map_err(|error| error.to_string())?.len();
    let limit = 1_000_000_u64;
    let start = size.saturating_sub(limit);
    file.seek(SeekFrom::Start(start)).map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(|error| error.to_string())?;
    let mut result = String::from_utf8_lossy(&bytes).into_owned();
    if start > 0 {
        result.insert_str(0, "[Showing the final 1 MB of this diagnostic log]\n\n");
    }
    Ok(result)
}

fn config_value<'a>(settings: &'a Value, path: &str) -> Result<&'a Value, String> {
    let mut value = settings;
    for key in path.split('.') {
        value = value.get(key).ok_or_else(|| format!("Setting {path} is missing."))?;
    }
    Ok(value)
}

fn config_string(settings: &Value, path: &str, allowed: &[&str]) -> Result<String, String> {
    let value = config_value(settings, path)?
        .as_str()
        .ok_or_else(|| format!("Setting {path} must be text."))?;
    if !allowed.contains(&value) {
        return Err(format!("Setting {path} has an unsupported value."));
    }
    Ok(value.to_string())
}

fn config_bool(settings: &Value, path: &str) -> Result<bool, String> {
    config_value(settings, path)?
        .as_bool()
        .ok_or_else(|| format!("Setting {path} must be true or false."))
}

fn config_integer(settings: &Value, path: &str, minimum: i64, maximum: i64) -> Result<i64, String> {
    let value = config_value(settings, path)?
        .as_i64()
        .ok_or_else(|| format!("Setting {path} must be a whole number."))?;
    if value < minimum || value > maximum {
        return Err(format!("Setting {path} must be between {minimum} and {maximum}."));
    }
    Ok(value)
}

fn config_number(settings: &Value, path: &str, minimum: f64, maximum: f64) -> Result<f64, String> {
    let value = config_value(settings, path)?
        .as_f64()
        .ok_or_else(|| format!("Setting {path} must be a number."))?;
    if !value.is_finite() || value < minimum || value > maximum {
        return Err(format!("Setting {path} must be between {minimum} and {maximum}."));
    }
    Ok(value)
}

fn add_boolean_option(args: &mut Vec<String>, name: &str, value: bool) {
    args.push(format!("--{}{}", if value { "" } else { "no-" }, name));
}

fn settings_arguments(settings: &Value) -> Result<Vec<String>, String> {
    let mut args = Vec::new();
    args.extend(["--crf".into(), config_integer(settings, "video.crf", 0, 51)?.to_string()]);
    args.extend(["--preset".into(), config_string(settings, "video.preset", &["ultrafast", "superfast", "veryfast", "faster", "fast", "medium", "slow", "slower", "veryslow"])?]);
    add_boolean_option(&mut args, "promote-8bit-to-10bit", config_bool(settings, "video.promote_8bit_to_10bit")?);
    args.extend(["--tune".into(), config_string(settings, "content.tune", &["auto", "animation", "grain", "none"])?]);
    add_boolean_option(&mut args, "detect-animation", config_bool(settings, "content.detect_animation")?);
    add_boolean_option(&mut args, "preserve-hdr10plus", config_bool(settings, "hdr.preserve_hdr10plus")?);
    args.extend(["--dolby-vision-policy".into(), config_string(settings, "hdr.dolby_vision", &["safe", "skip", "hdr10-fallback"])?]);
    args.extend(["--secondary-video".into(), config_string(settings, "streams.secondary_video", &["copy", "drop"])?]);
    args.extend(["--data-streams".into(), config_string(settings, "streams.data_streams", &["preserve", "drop"])?]);
    add_boolean_option(&mut args, "verify-copy-hashes", config_bool(settings, "safety.verify_copy_hashes")?);

    let comment = config_value(settings, "metadata.comment")?
        .as_str()
        .ok_or("Metadata comment must be text.")?;
    if comment.len() > 500 {
        return Err("Metadata comment is limited to 500 UTF-8 bytes.".into());
    }
    args.extend(["--comment".into(), comment.to_string()]);

    args.extend(["--resource-threading".into(), config_string(settings, "resources.threading", &["auto", "unlimited"])?]);
    args.extend(["--reserved-cpu-threads".into(), config_integer(settings, "resources.reserved_cpu_threads", 0, 64)?.to_string()]);
    args.extend(["--max-worker-threads".into(), config_integer(settings, "resources.max_worker_threads", 0, 64)?.to_string()]);
    args.extend(["--frame-threads".into(), config_integer(settings, "resources.frame_threads", 0, 16)?.to_string()]);
    args.extend(["--min-free-memory-gib".into(), config_number(settings, "resources.min_free_memory_gib", 0.0, 1_000_000.0)?.to_string()]);
    args.extend(["--memory-reserve-percent".into(), config_number(settings, "resources.memory_reserve_percent", 0.0, 50.0)?.to_string()]);
    args.extend(["--min-free-disk-gib".into(), config_number(settings, "resources.min_free_disk_gib", 0.0, 1_000_000.0)?.to_string()]);
    args.extend(["--preflight-output-ratio".into(), config_number(settings, "resources.preflight_output_ratio", 0.25, 4.0)?.to_string()]);
    args.extend(["--disk-projection-margin-percent".into(), config_number(settings, "resources.disk_projection_margin_percent", 0.0, 100.0)?.to_string()]);
    args.extend(["--calibration-mode".into(), config_string(settings, "calibration.mode", &["auto", "ask", "always", "off"])?]);
    args.extend(["--calibration-cache-hours".into(), config_number(settings, "calibration.cache_hours", 0.0, 720.0)?.to_string()]);
    args.extend(["--calibration-large-file-gib".into(), config_number(settings, "calibration.large_file_gib", 0.0, 1000.0)?.to_string()]);
    args.extend(["--calibration-sample-seconds".into(), config_number(settings, "calibration.sample_seconds", 1.0, 15.0)?.to_string()]);
    args.extend(["--calibration-max-candidates".into(), config_integer(settings, "calibration.max_candidates", 1, 4)?.to_string()]);
    args.extend(["--calibration-tolerance-percent".into(), config_number(settings, "calibration.throughput_tolerance_percent", 0.0, 20.0)?.to_string()]);
    Ok(args)
}

fn new_job_id() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{}-{now}", std::process::id())
}

fn claim_job(state: &RunState, job: ActiveJob) -> Result<(), String> {
    let mut current = state.0.lock().map_err(|_| "Could not lock the desktop job state.")?;
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
    mode: String,
    settings: Value,
    stop_on_error: bool,
    full_rebuild_confirmed: bool,
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
        .map_err(|error| format!("Could not open the selected media folder: {error}"))?;
    if !root.is_dir() {
        return Err("The selected media path is not a folder.".into());
    }
    if !["check", "test", "full"].contains(&request.mode.as_str()) {
        return Err("Choose Check only, Short test, or Full rebuild.".into());
    }
    if request.mode == "full" && !request.full_rebuild_confirmed {
        return Err("Confirm the full rebuild before starting it.".into());
    }
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
    if let Some(path) = request.ffmpeg_path.filter(|value| !value.trim().is_empty()) {
        let binary = PathBuf::from(&path)
            .canonicalize()
            .map_err(|error| format!("Selected FFmpeg executable is unavailable: {error}"))?;
        if !binary.is_file() {
            return Err("Selected FFmpeg path is not a file.".into());
        }
        args.extend(["--ffmpeg-path".into(), binary.to_string_lossy().into_owned()]);
    }
    args.extend(settings_arguments(&request.settings)?);
    if request.stop_on_error {
        args.push("--stop-on-error".into());
    }
    match request.mode.as_str() {
        "check" => args.push("--info-only".into()),
        "test" => args.push("--test".into()),
        _ => {}
    }

    let id = new_job_id();
    let stop_file = std::env::temp_dir().join(format!("hevc-rebuild-stop-{id}"));
    args.extend(["--stop-request-file".into(), stop_file.to_string_lossy().into_owned()]);
    let active = ActiveJob { id: id.clone(), stop_file: Some(stop_file.clone()) };
    claim_job(state.inner(), active)?;

    let app_handle = app.clone();
    let run_state = state.inner().clone();
    let worker_id = id.clone();
    thread::spawn(move || {
        let mut log_path = None;
        let mut report_path = None;
        let mut error = None;
        let mut exit_code = None;
        match engine_command(&app_handle, &args).and_then(|mut command| {
            command.stdout(Stdio::piped()).stderr(Stdio::piped());
            command.spawn().map_err(|error| format!("Could not start the rebuild engine: {error}"))
        }) {
            Ok(mut child) => {
                let stderr_thread = child.stderr.take().map(|stream| {
                    forward_lines(app_handle.clone(), "run-line", worker_id.clone(), stream)
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
                                let _ = app_handle.emit("run-line", RunLine { run_id: worker_id.clone(), line });
                            }
                            Err(read_error) => {
                                error = Some(format!("Could not read rebuild output: {read_error}"));
                            }
                        }
                    }
                }
                if let Some(handle) = stderr_thread {
                    let _ = handle.join();
                }
                match child.wait() {
                    Ok(status) => exit_code = status.code(),
                    Err(wait_error) => error = Some(format!("Could not wait for the rebuild engine: {wait_error}")),
                }
            }
            Err(start_error) => error = Some(start_error),
        }
        let stop_requested = stop_file.exists();
        let _ = fs::remove_file(&stop_file);
        let _ = app_handle.emit(
            "run-finished",
            RunFinished { run_id: worker_id.clone(), exit_code, stop_requested, log_path, report_path, error },
        );
        release_job(&run_state, &worker_id);
    });
    Ok(id)
}

#[tauri::command]
fn request_stop(state: State<'_, RunState>, run_id: String) -> Result<(), String> {
    let path = {
        let current = state.0.lock().map_err(|_| "Could not lock the desktop job state.")?;
        let job = current.as_ref().ok_or("There is no active run.")?;
        if job.id != run_id {
            return Err("That run is no longer active.".into());
        }
        job.stop_file.clone().ok_or("This system repair cannot be stopped from the run controls.")?
    };
    if !path.exists() {
        File::create(path)
            .and_then(|mut file| file.write_all(b"finish current file, then stop\n"))
            .map_err(|error| format!("Could not request a safe stop: {error}"))?;
    }
    Ok(())
}

const APT_ALLOWLIST: &[&str] = &["python3", "ffmpeg", "mediainfo", "mkvtoolnix", "fontconfig", "ca-certificates"];

#[tauri::command]
fn install_apt_tools(
    app: AppHandle,
    state: State<'_, RunState>,
    packages: Vec<String>,
) -> Result<String, String> {
    if packages.is_empty() || packages.iter().any(|package| !APT_ALLOWLIST.contains(&package.as_str())) {
        return Err("APT repair accepts only the application’s fixed Ubuntu package list.".into());
    }
    let id = new_job_id();
    claim_job(state.inner(), ActiveJob { id: id.clone(), stop_file: None })?;
    let run_state = state.inner().clone();
    let worker_id = id.clone();
    thread::spawn(move || {
        let app_handle = app;
        let mut last_error = None;
        let prefix = if cfg!(target_os = "linux") && unsafe { libc_geteuid() } != 0 {
            Some("pkexec")
        } else {
            None
        };
        let commands = [
            vec!["apt-get".to_string(), "update".to_string()],
            {
                let mut command = vec!["apt-get".into(), "install".into(), "--yes".into(), "--no-upgrade".into(), "--".into()];
                command.extend(packages);
                command
            },
        ];
        let mut success = true;
        for mut parts in commands {
            if let Some(elevation) = prefix {
                parts.insert(0, elevation.to_string());
            }
            let Some(program) = parts.first().cloned() else { continue };
            let mut command = Command::new(program);
            command.args(parts.iter().skip(1)).stdout(Stdio::piped()).stderr(Stdio::piped());
            match command.spawn() {
                Ok(mut child) => {
                    let stderr_thread = child.stderr.take().map(|stream| {
                        forward_lines(app_handle.clone(), "repair-line", worker_id.clone(), stream)
                    });
                    if let Some(stdout) = child.stdout.take() {
                        for line in BufReader::new(stdout).lines().flatten() {
                            let _ = app_handle.emit("repair-line", RunLine { run_id: worker_id.clone(), line });
                        }
                    }
                    if let Some(handle) = stderr_thread {
                        let _ = handle.join();
                    }
                    match child.wait() {
                        Ok(status) if status.success() => {}
                        Ok(status) => {
                            success = false;
                            last_error = Some(format!("APT exited with status {status}; inspect the repair output."));
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
        let _ = app_handle.emit("repair-finished", json!({"jobId": worker_id, "success": success, "error": last_error}));
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
async fn install_hdr10plus(app: AppHandle, state: State<'_, RunState>) -> Result<String, String> {
    let id = new_job_id();
    claim_job(state.inner(), ActiveJob { id: id.clone(), stop_file: None })?;
    let run_state = state.inner().clone();
    let task = tauri::async_runtime::spawn_blocking(move || {
        let output = bridge_command(&app, &["install-hdr10plus".into()])
            .and_then(|mut command| command.output().map_err(|error| format!("Could not start HDR10+ helper setup: {error}")))?;
        if !output.status.success() {
            return Err(format_process_error(&output));
        }
        let payload: Value = serde_json::from_slice(&output.stdout)
            .map_err(|error| format!("HDR10+ helper setup returned invalid output: {error}"))?;
        payload.get("installed").and_then(Value::as_str).map(str::to_string)
            .ok_or("HDR10+ helper setup returned no installed path.".into())
    })
    .await;
    let result = match task {
        Ok(result) => result,
        Err(error) => Err(format!("HDR10+ helper setup task failed: {error}")),
    };
    release_job(&run_state, &id);
    result
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(RunState::default())
        .invoke_handler(tauri::generate_handler![
            load_app_preferences,
            save_app_preferences,
            inspect_environment,
            default_media_root,
            load_settings,
            scan_folder,
            list_reports,
            read_log,
            start_run,
            request_stop,
            install_apt_tools,
            install_hdr10plus,
        ])
        .run(tauri::generate_context!())
        .expect("error while running HEVC Rebuild Studio");
}
