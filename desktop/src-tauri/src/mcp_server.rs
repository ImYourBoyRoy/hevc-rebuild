use super::{settings_arguments, standalone_command};
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    schemars, tool, tool_handler, tool_router, ErrorData, ServerHandler, ServiceExt,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    fs::{self, File},
    io::{BufRead, BufReader, Read, Write},
    path::PathBuf,
    process::{Command, Output, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

type Jobs = Arc<Mutex<HashMap<String, Job>>>;

#[derive(Clone, Serialize)]
#[serde(rename_all = "snake_case")]
struct Job {
    id: String,
    kind: String,
    status: String,
    current: usize,
    total: usize,
    last_file: Option<String>,
    last_lines: Vec<String>,
    exit_code: Option<i32>,
    log_path: Option<String>,
    report_path: Option<String>,
    error: Option<String>,
    #[serde(skip_serializing)]
    stop_file: Option<PathBuf>,
    result: Option<Value>,
}

impl Job {
    fn new(id: String, kind: &str) -> Self {
        Self {
            id,
            kind: kind.into(),
            status: "queued".into(),
            current: 0,
            total: 0,
            last_file: None,
            last_lines: Vec::new(),
            exit_code: None,
            log_path: None,
            report_path: None,
            error: None,
            stop_file: None,
            result: None,
        }
    }
}

#[derive(Clone)]
struct McpServer {
    jobs: Jobs,
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
}

impl McpServer {
    fn new() -> Self {
        Self {
            jobs: Arc::new(Mutex::new(HashMap::new())),
            tool_router: Self::tool_router(),
        }
    }

    fn insert_job(&self, job: Job) -> Result<(), String> {
        let mut jobs = self
            .jobs
            .lock()
            .map_err(|_| "The MCP job registry is unavailable.")?;
        let active: Vec<&Job> = jobs
            .values()
            .filter(|existing| existing.status == "queued" || existing.status == "running")
            .collect();
        if job.kind == "folder_analysis" {
            if active
                .iter()
                .any(|existing| existing.kind != "folder_analysis")
            {
                return Err("A media run, system repair, or helper installation is active; wait for it to finish before analyzing folders.".into());
            }
            if active.len() >= 2 {
                return Err("Two folder analyses are already active; wait for one to finish before starting another.".into());
            }
        } else if !active.is_empty() {
            return Err("Another analysis, media run, repair, or helper installation is active; wait for it to finish before starting this operation.".into());
        }
        if jobs.len() >= 100 {
            let mut finished: Vec<String> = jobs
                .values()
                .filter(|job| {
                    job.status == "completed" || job.status == "failed" || job.status == "stopped"
                })
                .map(|job| job.id.clone())
                .collect();
            finished.sort();
            while jobs.len() >= 90 {
                let Some(id) = finished.first().cloned() else {
                    return Err("The MCP job registry is full; wait for active jobs to finish before starting another.".into());
                };
                finished.remove(0);
                jobs.remove(&id);
            }
        }
        jobs.insert(job.id.clone(), job);
        Ok(())
    }
}

fn bridge_json(args: &[String]) -> Result<Value, String> {
    let output = standalone_command("hevc_rebuild.bridge", args).and_then(|mut command| {
        command
            .output()
            .map_err(|error| format!("Could not start the media engine: {error}"))
    })?;
    if !output.status.success() {
        return Err(process_error(&output));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let payload = stdout
        .lines()
        .filter(|line| !line.starts_with("__HEVC_SCAN_PROGRESS__\t"))
        .last()
        .ok_or("The media engine returned no JSON result.")?;
    serde_json::from_str(payload)
        .map_err(|error| format!("The media engine returned invalid JSON: {error}"))
}

fn process_error(output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if stderr.is_empty() {
        format!("Media engine exited with status {}.", output.status)
    } else {
        stderr.chars().take(4000).collect()
    }
}

fn job_id() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{}-{now}", std::process::id())
}

fn record_line(jobs: &Jobs, id: &str, line: String) {
    if let Ok(mut jobs) = jobs.lock() {
        if let Some(job) = jobs.get_mut(id) {
            let line = if line.chars().count() > 4096 {
                format!(
                    "{} …[line truncated]",
                    line.chars().take(4096).collect::<String>()
                )
            } else {
                line
            };
            if let Some(path) = line.strip_prefix("Log:") {
                job.log_path = Some(path.trim().to_string());
            }
            if let Some(path) = line.strip_prefix("Run report:") {
                job.report_path = Some(path.trim().to_string());
            }
            job.last_lines.push(line);
            if job.last_lines.len() > 120 {
                job.last_lines.drain(..job.last_lines.len() - 120);
            }
        }
    }
}

fn update_failed(jobs: &Jobs, id: &str, error: String) {
    if let Ok(mut jobs) = jobs.lock() {
        if let Some(job) = jobs.get_mut(id) {
            job.status = "failed".into();
            job.error = Some(error);
        }
    }
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct EnvironmentParams {
    ffmpeg_path: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct RootParams {
    root: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct ScanParams {
    root: String,
    recursive: Option<bool>,
    minimum_size_mib: Option<u32>,
    ffmpeg_path: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct JobParams {
    job_id: String,
    offset: Option<usize>,
    limit: Option<usize>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct StartRunParams {
    root: String,
    mode: Option<String>,
    recursive: Option<bool>,
    minimum_size_mib: Option<u32>,
    ffmpeg_path: Option<String>,
    settings_json: Option<String>,
    stop_on_error: Option<bool>,
    confirm_test_output: Option<bool>,
    confirm_full_rebuild: Option<bool>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct StopParams {
    job_id: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct RepairParams {
    confirmed_package_names: Vec<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct HelperParams {
    confirm_installation: bool,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct ReportsParams {
    limit: Option<usize>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct LogParams {
    path: String,
}

fn canonical_folder(value: &str) -> Result<PathBuf, String> {
    let root = PathBuf::from(value)
        .canonicalize()
        .map_err(|error| format!("Could not open media folder: {error}"))?;
    if !root.is_dir() {
        return Err("The selected media path is not a folder.".into());
    }
    Ok(root)
}

fn checked_ffmpeg(value: Option<String>) -> Result<Option<String>, String> {
    let Some(value) = value.filter(|value| !value.trim().is_empty()) else {
        return Ok(None);
    };
    let binary = PathBuf::from(value)
        .canonicalize()
        .map_err(|error| format!("Selected FFmpeg executable is unavailable: {error}"))?;
    if !binary.is_file() {
        return Err("Selected FFmpeg path is not a file.".into());
    }
    Ok(Some(binary.to_string_lossy().into_owned()))
}

fn json_result(
    result: Result<Value, String>,
) -> Result<rmcp::handler::server::wrapper::Json<Value>, ErrorData> {
    result
        .map(rmcp::handler::server::wrapper::Json)
        .map_err(|error| ErrorData::invalid_params(error, None))
}

#[tool_router]
impl McpServer {
    #[tool(
        name = "inspect_environment",
        description = "Read the host OS, CPU, memory, GPU IDs, media-tool versions, readiness, and missing Ubuntu package recommendations. This does not install or modify anything."
    )]
    fn inspect_environment(
        &self,
        Parameters(params): Parameters<EnvironmentParams>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, ErrorData> {
        let args = checked_ffmpeg(params.ffmpeg_path)
            .map_err(|error| ErrorData::invalid_params(error, None))?;
        let mut bridge_args = vec!["environment".into()];
        if let Some(path) = args {
            bridge_args.extend(["--ffmpeg-path".into(), path]);
        }
        json_result(bridge_json(&bridge_args))
    }

    #[tool(
        name = "get_effective_settings",
        description = "Read effective HEVC Rebuild settings for a media root. Uses the same built-in, user, and folder config precedence as the GUI and never creates a config file."
    )]
    fn get_effective_settings(
        &self,
        Parameters(params): Parameters<RootParams>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, ErrorData> {
        let requested_root = params.root.unwrap_or_else(|| ".".into());
        let root = canonical_folder(&requested_root)
            .map_err(|error| ErrorData::invalid_params(error, None))?;
        let args = vec![
            "defaults".into(),
            "--root".into(),
            root.to_string_lossy().into_owned(),
        ];
        json_result(bridge_json(&args))
    }

    #[tool(
        name = "start_folder_analysis",
        description = "Analyze a local folder of media files without encoding or writing to the media folder. Returns a job ID for progress, page retrieval, per-file errors, current available/total memory context, and profile-aware x265 concurrency forecasts. Forecasts are heuristics, not exact peak-RAM guarantees. Only process content you own or are authorized to use."
    )]
    fn start_folder_analysis(
        &self,
        Parameters(params): Parameters<ScanParams>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, ErrorData> {
        let root = canonical_folder(&params.root)
            .map_err(|error| ErrorData::invalid_params(error, None))?;
        let minimum = params.minimum_size_mib.unwrap_or(5);
        if minimum > 4096 {
            return Err(ErrorData::invalid_params(
                "Minimum input size must be 4096 MiB or less.",
                None,
            ));
        }
        let ffmpeg = checked_ffmpeg(params.ffmpeg_path)
            .map_err(|error| ErrorData::invalid_params(error, None))?;
        let mut args = vec![
            "scan".into(),
            "--root".into(),
            root.to_string_lossy().into_owned(),
            "--min-size-mib".into(),
            minimum.to_string(),
        ];
        if params.recursive.unwrap_or(true) {
            args.push("--recursive".into());
        }
        if let Some(path) = ffmpeg {
            args.extend(["--ffmpeg-path".into(), path]);
        }
        let id = job_id();
        let mut job = Job::new(id.clone(), "folder_analysis");
        job.status = "running".into();
        self.insert_job(job)
            .map_err(|error| ErrorData::internal_error(error, None))?;
        let jobs = self.jobs.clone();
        let worker_id = id.clone();
        thread::spawn(move || run_scan_job(jobs, worker_id, args));
        Ok(rmcp::handler::server::wrapper::Json(
            json!({"job_id": id, "status": "running", "poll_with": "get_job"}),
        ))
    }

    #[tool(
        name = "get_job",
        description = "Get scan, encode, repair, or helper-install progress. For folder analysis, request bounded result pages with offset and limit (maximum 250). Includes recent diagnostic lines, job status, and per-file errors."
    )]
    fn get_job(
        &self,
        Parameters(params): Parameters<JobParams>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, ErrorData> {
        let jobs = self
            .jobs
            .lock()
            .map_err(|_| ErrorData::internal_error("The MCP job registry is unavailable.", None))?;
        let job = jobs
            .get(&params.job_id)
            .cloned()
            .ok_or_else(|| ErrorData::invalid_params("Unknown or expired job ID.", None))?;
        drop(jobs);
        let Some(result) = job.result.as_ref() else {
            return Ok(rmcp::handler::server::wrapper::Json(json!({"job": job})));
        };
        let entries = result
            .get("entries")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let offset = params.offset.unwrap_or(0).min(entries.len());
        let limit = params.limit.unwrap_or(100).clamp(1, 250);
        let page: Vec<Value> = entries.into_iter().skip(offset).take(limit).collect();
        let mut summary = result.clone();
        if let Some(object) = summary.as_object_mut() {
            object.remove("entries");
        }
        Ok(rmcp::handler::server::wrapper::Json(
            json!({"job": job, "summary": summary, "entries": page, "offset": offset, "limit": limit}),
        ))
    }

    #[tool(
        name = "start_media_run",
        description = "Start check-only, short-test, or full rebuild for local media you own or are authorized to process. Uses GUI-equivalent validated settings and hardware-aware defaults. Full rebuild requires confirm_full_rebuild=true; short-test output requires confirm_test_output=true. Returns a job ID; poll get_job and use request_safe_stop to finish the current file then stop."
    )]
    fn start_media_run(
        &self,
        Parameters(params): Parameters<StartRunParams>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, ErrorData> {
        let root = canonical_folder(&params.root)
            .map_err(|error| ErrorData::invalid_params(error, None))?;
        let mode = params.mode.as_deref().unwrap_or("check");
        if !["check", "test", "full"].contains(&mode) {
            return Err(ErrorData::invalid_params(
                "Mode must be check, test, or full.",
                None,
            ));
        }
        if mode == "full" && params.confirm_full_rebuild != Some(true) {
            return Err(ErrorData::invalid_params(
                "Set confirm_full_rebuild=true to authorize this full rebuild.",
                None,
            ));
        }
        if mode == "test" && params.confirm_test_output != Some(true) {
            return Err(ErrorData::invalid_params(
                "Set confirm_test_output=true to authorize writing short-test outputs.",
                None,
            ));
        }
        let minimum = params.minimum_size_mib.unwrap_or(5);
        if minimum > 4096 {
            return Err(ErrorData::invalid_params(
                "Minimum input size must be 4096 MiB or less.",
                None,
            ));
        }
        let ffmpeg = checked_ffmpeg(params.ffmpeg_path)
            .map_err(|error| ErrorData::invalid_params(error, None))?;
        let settings = if let Some(serialized) = params.settings_json {
            serde_json::from_str::<Value>(&serialized).map_err(|error| {
                ErrorData::invalid_params(format!("settings_json is invalid: {error}"), None)
            })?
        } else {
            bridge_json(&[
                "defaults".into(),
                "--root".into(),
                root.to_string_lossy().into_owned(),
            ])
            .map_err(|error| ErrorData::internal_error(error, None))?
            .get("settings")
            .cloned()
            .ok_or_else(|| ErrorData::internal_error("Could not load effective settings.", None))?
        };
        let mut args = vec![
            root.to_string_lossy().into_owned(),
            "--min-size-mib".into(),
            minimum.to_string(),
            "--no-dependency-repair".into(),
            "--skip-update-check".into(),
        ];
        if params.recursive.unwrap_or(true) {
            args.push("--recursive".into());
        }
        if let Some(path) = ffmpeg {
            args.extend(["--ffmpeg-path".into(), path]);
        }
        args.extend(
            settings_arguments(&settings)
                .map_err(|error| ErrorData::invalid_params(error, None))?,
        );
        if params.stop_on_error.unwrap_or(false) {
            args.push("--stop-on-error".into());
        }
        match mode {
            "check" => args.push("--info-only".into()),
            "test" => args.push("--test".into()),
            _ => {}
        }
        let id = job_id();
        let stop_file = std::env::temp_dir().join(format!("hevc-rebuild-stop-{id}"));
        args.extend([
            "--stop-request-file".into(),
            stop_file.to_string_lossy().into_owned(),
        ]);
        let mut job = Job::new(id.clone(), "media_run");
        job.status = "running".into();
        job.stop_file = Some(stop_file.clone());
        self.insert_job(job)
            .map_err(|error| ErrorData::internal_error(error, None))?;
        let jobs = self.jobs.clone();
        let worker_id = id.clone();
        thread::spawn(move || run_media_job(jobs, worker_id, args, stop_file));
        Ok(rmcp::handler::server::wrapper::Json(
            json!({"job_id": id, "status": "running", "poll_with": "get_job"}),
        ))
    }

    #[tool(
        name = "request_safe_stop",
        description = "Request a running media job to finish its current file and stop before starting another. This does not kill FFmpeg mid-file."
    )]
    fn request_safe_stop(
        &self,
        Parameters(params): Parameters<StopParams>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, ErrorData> {
        let path = {
            let jobs = self.jobs.lock().map_err(|_| {
                ErrorData::internal_error("The MCP job registry is unavailable.", None)
            })?;
            let job = jobs
                .get(&params.job_id)
                .ok_or_else(|| ErrorData::invalid_params("Unknown job ID.", None))?;
            if job.kind != "media_run" || job.status != "running" {
                return Err(ErrorData::invalid_params(
                    "Only an active media run can be safely stopped.",
                    None,
                ));
            }
            job.stop_file.clone().ok_or_else(|| {
                ErrorData::internal_error("The media run has no stop-request file.", None)
            })?
        };
        if !path.exists() {
            File::create(&path)
                .and_then(|mut file| file.write_all(b"finish current file, then stop\n"))
                .map_err(|error| {
                    ErrorData::internal_error(
                        format!("Could not request a safe stop: {error}"),
                        None,
                    )
                })?;
        }
        Ok(rmcp::handler::server::wrapper::Json(
            json!({"job_id": params.job_id, "stop_requested": true, "behavior": "finish_current_file"}),
        ))
    }

    #[tool(
        name = "list_reports",
        description = "List recent local HEVC Rebuild reports, result counts, and diagnostic-log paths."
    )]
    fn list_reports(
        &self,
        Parameters(params): Parameters<ReportsParams>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, ErrorData> {
        let limit = params.limit.unwrap_or(12).clamp(1, 50);
        json_result(bridge_json(&[
            "reports".into(),
            "--limit".into(),
            limit.to_string(),
        ]))
    }

    #[tool(
        name = "read_diagnostic_log",
        description = "Read at most the final 1 MB of one HEVC Rebuild .log file from this user's application log folder. Other filesystem paths are rejected."
    )]
    fn read_diagnostic_log(
        &self,
        Parameters(params): Parameters<LogParams>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, ErrorData> {
        let home = std::env::var_os("HOME").ok_or_else(|| {
            ErrorData::internal_error("Could not locate this user's home folder.", None)
        })?;
        let allowed = PathBuf::from(home)
            .join(".local/state/hevc-rebuild/logs")
            .canonicalize()
            .map_err(|error| {
                ErrorData::invalid_params(
                    format!("Diagnostic log folder is unavailable: {error}"),
                    None,
                )
            })?;
        let requested = PathBuf::from(&params.path)
            .canonicalize()
            .map_err(|error| {
                ErrorData::invalid_params(format!("Could not open diagnostic log: {error}"), None)
            })?;
        if !requested.starts_with(&allowed)
            || requested.extension().and_then(|value| value.to_str()) != Some("log")
        {
            return Err(ErrorData::invalid_params(
                "Only HEVC Rebuild log files from this user's log folder can be read.",
                None,
            ));
        }
        let mut file = File::open(&requested).map_err(|error| {
            ErrorData::internal_error(format!("Could not read diagnostic log: {error}"), None)
        })?;
        let size = file
            .metadata()
            .map_err(|error| ErrorData::internal_error(error.to_string(), None))?
            .len();
        let offset = size.saturating_sub(1_000_000);
        use std::io::{Seek, SeekFrom};
        file.seek(SeekFrom::Start(offset))
            .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
        let mut content = String::new();
        file.take(1_000_000)
            .read_to_string(&mut content)
            .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
        if offset > 0 {
            content.insert_str(0, "[Showing the final 1 MB of this diagnostic log]\n\n");
        }
        Ok(rmcp::handler::server::wrapper::Json(
            json!({"path": requested, "content": content}),
        ))
    }

    #[tool(
        name = "repair_system_dependencies",
        description = "Plan and optionally install only the currently missing packages from HEVC Rebuild's fixed Ubuntu allowlist. First inspect_environment; then pass confirmed_package_names exactly equal to its missing_apt_packages list. This starts an administrator-authenticated APT repair job."
    )]
    fn repair_system_dependencies(
        &self,
        Parameters(params): Parameters<RepairParams>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, ErrorData> {
        let environment = bridge_json(&["environment".into()])
            .map_err(|error| ErrorData::internal_error(error, None))?;
        if environment
            .pointer("/os/supported")
            .and_then(Value::as_bool)
            != Some(true)
        {
            return Err(ErrorData::invalid_params(
                "Automatic package repair is limited to supported Ubuntu and Linux Mint releases.",
                None,
            ));
        }
        const ALLOW: &[&str] = &[
            "python3",
            "ffmpeg",
            "mediainfo",
            "mkvtoolnix",
            "fontconfig",
            "ca-certificates",
        ];
        let mut missing: Vec<String> = environment
            .get("missing_apt_packages")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect();
        missing.sort();
        missing.dedup();
        let provided_count = params.confirmed_package_names.len();
        let mut confirmed = params.confirmed_package_names;
        confirmed.sort();
        confirmed.dedup();
        if confirmed.len() != provided_count {
            return Err(ErrorData::invalid_params(
                "Each package can appear only once in confirmed_package_names.",
                None,
            ));
        }
        if missing.is_empty() {
            return Ok(rmcp::handler::server::wrapper::Json(
                json!({"status":"already_ready","packages":[]}),
            ));
        }
        if missing
            .iter()
            .any(|package| !ALLOW.contains(&package.as_str()))
            || missing != confirmed
        {
            return Err(ErrorData::invalid_params(json!({"message":"For system repair, confirm the exact missing-package list returned by inspect_environment.","missing_apt_packages":missing}).to_string(), None));
        }
        let id = job_id();
        let mut job = Job::new(id.clone(), "system_repair");
        job.status = "running".into();
        self.insert_job(job)
            .map_err(|error| ErrorData::internal_error(error, None))?;
        let jobs = self.jobs.clone();
        let worker_id = id.clone();
        thread::spawn(move || run_apt_job(jobs, worker_id, missing));
        Ok(rmcp::handler::server::wrapper::Json(
            json!({"job_id":id,"status":"running","poll_with":"get_job"}),
        ))
    }

    #[tool(
        name = "install_hdr10plus_helper",
        description = "Install the upstream HDR10+ helper after explicit confirmation. The engine verifies the helper release before placing it in this user's application data directory."
    )]
    fn install_hdr10plus_helper(
        &self,
        Parameters(params): Parameters<HelperParams>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, ErrorData> {
        if !params.confirm_installation {
            return Err(ErrorData::invalid_params(
                "Set confirm_installation=true to start the verified helper installation.",
                None,
            ));
        }
        let id = job_id();
        let mut job = Job::new(id.clone(), "helper_install");
        job.status = "running".into();
        self.insert_job(job)
            .map_err(|error| ErrorData::internal_error(error, None))?;
        let jobs = self.jobs.clone();
        let worker_id = id.clone();
        thread::spawn(move || match bridge_json(&["install-hdr10plus".into()]) {
            Ok(result) => {
                if let Ok(mut jobs) = jobs.lock() {
                    if let Some(job) = jobs.get_mut(&worker_id) {
                        job.status = "completed".into();
                        job.result = Some(result);
                    }
                }
            }
            Err(error) => update_failed(&jobs, &worker_id, error),
        });
        Ok(rmcp::handler::server::wrapper::Json(
            json!({"job_id":id,"status":"running","poll_with":"get_job"}),
        ))
    }
}

#[tool_handler(
    name = "hevc-rebuild-studio",
    version = "1.3.0",
    instructions = "Local Ubuntu-first media library analysis and HEVC rebuild MCP server. Inspect first; analysis is read-only. Runs default to check mode. Require explicit confirmation flags before short-test or full rebuild outputs. System package repair requires the exact package list from inspect_environment. Only process media the user owns or is authorized to use."
)]
impl ServerHandler for McpServer {}

fn run_scan_job(jobs: Jobs, id: String, args: Vec<String>) {
    let result = (|| -> Result<Value, String> {
        let mut command = standalone_command("hevc_rebuild.bridge", &args)?;
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .map_err(|error| format!("Could not start folder analysis: {error}"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or("Could not read folder analysis output.")?;
        let stderr = child
            .stderr
            .take()
            .ok_or("Could not read folder analysis diagnostics.")?;
        let stderr_thread = thread::spawn(move || {
            let mut text = String::new();
            let _ = BufReader::new(stderr).read_to_string(&mut text);
            text
        });
        let mut payload = None;
        for line in BufReader::new(stdout).lines() {
            let line = line.map_err(|error| format!("Could not read folder analysis: {error}"))?;
            if let Some(progress) = line.strip_prefix("__HEVC_SCAN_PROGRESS__\t") {
                let parts: Vec<&str> = progress.splitn(3, '\t').collect();
                if parts.len() == 3 {
                    if let Ok(mut jobs) = jobs.lock() {
                        if let Some(job) = jobs.get_mut(&id) {
                            job.current = parts[0].parse().unwrap_or(0);
                            job.total = parts[1].parse().unwrap_or(0);
                            job.last_file = Some(parts[2].to_string());
                        }
                    }
                }
            } else if !line.trim().is_empty() {
                payload = Some(line);
            }
        }
        let status = child
            .wait()
            .map_err(|error| format!("Could not finish folder analysis: {error}"))?;
        let stderr = stderr_thread.join().unwrap_or_default();
        if !status.success() {
            return Err(if stderr.trim().is_empty() {
                format!("Folder analysis exited with {status}.")
            } else {
                stderr.chars().take(4000).collect()
            });
        }
        serde_json::from_str(
            payload
                .as_deref()
                .ok_or("Folder analysis returned no result.")?,
        )
        .map_err(|error| format!("Folder analysis returned invalid JSON: {error}"))
    })();
    if let Ok(mut jobs) = jobs.lock() {
        if let Some(job) = jobs.get_mut(&id) {
            match result {
                Ok(payload) => {
                    job.status = "completed".into();
                    job.result = Some(payload);
                    if job.total == 0 {
                        job.total = job
                            .result
                            .as_ref()
                            .and_then(|v| v.get("candidate_count"))
                            .and_then(Value::as_u64)
                            .unwrap_or(0) as usize;
                        job.current = job.total;
                    }
                }
                Err(error) => {
                    job.status = "failed".into();
                    job.error = Some(error);
                }
            }
        }
    }
}

fn run_media_job(jobs: Jobs, id: String, args: Vec<String>, stop_file: PathBuf) {
    let outcome = (|| -> Result<i32, String> {
        let mut command = standalone_command("hevc_rebuild", &args)?;
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .map_err(|error| format!("Could not start the media engine: {error}"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or("Could not read engine diagnostics.")?;
        let jobs_for_err = jobs.clone();
        let id_for_err = id.clone();
        let stderr_thread = thread::spawn(move || {
            for line in BufReader::new(stderr).lines().flatten() {
                record_line(&jobs_for_err, &id_for_err, line);
            }
        });
        if let Some(stdout) = child.stdout.take() {
            for line in BufReader::new(stdout).lines().flatten() {
                record_line(&jobs, &id, line);
            }
        }
        let _ = stderr_thread.join();
        child
            .wait()
            .map(|status| status.code().unwrap_or(1))
            .map_err(|error| format!("Could not wait for the media engine: {error}"))
    })();
    let stop_requested = stop_file.exists();
    let _ = fs::remove_file(&stop_file);
    if let Ok(mut jobs) = jobs.lock() {
        if let Some(job) = jobs.get_mut(&id) {
            job.stop_file = None;
            match outcome {
                Ok(code) => {
                    job.exit_code = Some(code);
                    job.status = if code == 0 {
                        if stop_requested {
                            "stopped"
                        } else {
                            "completed"
                        }
                    } else {
                        "failed"
                    }
                    .into();
                    if code != 0 {
                        job.error=Some(format!("Media engine exited with code {code}; inspect recent diagnostic lines."));
                    }
                }
                Err(error) => {
                    job.status = "failed".into();
                    job.error = Some(error);
                }
            }
        }
    }
}

fn run_apt_job(jobs: Jobs, id: String, packages: Vec<String>) {
    let operations = [vec!["apt-get".to_string(), "update".to_string()], {
        let mut install = vec![
            "apt-get".into(),
            "install".into(),
            "--yes".into(),
            "--no-upgrade".into(),
            "--".into(),
        ];
        install.extend(packages);
        install
    }];
    for (index, mut operation) in operations.into_iter().enumerate() {
        if unsafe { geteuid() } != 0 {
            operation.insert(0, "pkexec".into());
        }
        let Some(program) = operation.first().cloned() else {
            continue;
        };
        let result = Command::new(program)
            .args(operation.iter().skip(1))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();
        let mut child = match result {
            Ok(child) => child,
            Err(error) => {
                update_failed(
                    &jobs,
                    &id,
                    format!("Could not start the package manager: {error}"),
                );
                return;
            }
        };
        let stderr = match child.stderr.take() {
            Some(stream) => stream,
            None => {
                update_failed(&jobs, &id, "Could not read APT diagnostics.".into());
                return;
            }
        };
        let jobs_for_err = jobs.clone();
        let err_id = id.clone();
        let thread = thread::spawn(move || {
            for line in BufReader::new(stderr).lines().flatten() {
                record_line(&jobs_for_err, &err_id, line);
            }
        });
        if let Some(stdout) = child.stdout.take() {
            for line in BufReader::new(stdout).lines().flatten() {
                record_line(&jobs, &id, line);
            }
        }
        let _ = thread.join();
        match child.wait() {
            Ok(status) if status.success() => {}
            Ok(status) => {
                update_failed(
                    &jobs,
                    &id,
                    format!("APT operation {} exited with {status}.", index + 1),
                );
                return;
            }
            Err(error) => {
                update_failed(&jobs, &id, format!("Could not wait for APT: {error}"));
                return;
            }
        }
    }
    if let Ok(mut jobs) = jobs.lock() {
        if let Some(job) = jobs.get_mut(&id) {
            job.status = "completed".into();
        }
    }
}

#[cfg(target_os = "linux")]
unsafe fn geteuid() -> u32 {
    unsafe extern "C" {
        fn geteuid() -> u32;
    }
    geteuid()
}
#[cfg(not(target_os = "linux"))]
unsafe fn geteuid() -> u32 {
    1
}

pub fn run() -> i32 {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("Could not start the MCP async runtime: {error}");
            return 2;
        }
    };
    runtime.block_on(async {
        match McpServer::new().serve(rmcp::transport::stdio()).await {
            Ok(service) => match service.waiting().await {
                Ok(_) => 0,
                Err(error) => {
                    eprintln!("MCP connection ended with an error: {error}");
                    1
                }
            },
            Err(error) => {
                eprintln!("Could not start the MCP stdio server: {error}");
                2
            }
        }
    })
}
