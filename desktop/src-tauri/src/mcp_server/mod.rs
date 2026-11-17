//! Typed local stdio MCP server: the tool router, the in-memory job registry, and server startup.
//! Request types, argument preparation, batch planning, queue sync, job executors and apt jobs
//! live in the sibling modules declared below.

use super::{
    append_tool_path_arguments, file_settings_override_arguments, settings_arguments,
    stream_override_arguments, youtube_hdr10plus_loss_arguments, ToolPaths,
};
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    schemars, tool, tool_handler, tool_router, ErrorData, ServerHandler, ServiceExt,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashMap},
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

type Jobs = Arc<Mutex<HashMap<String, Job>>>;

/// What a running job last reported for its current file, beyond "file N of M": phase, percentages,
/// speed, ETA and sizes. Polled by agents (`get_job`) and sent to the browser as `run-progress`.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub(crate) struct ProgressDetail {
    pub(crate) phase: String,
    pub(crate) file_percent: Option<f64>,
    pub(crate) queue_percent: Option<f64>,
    pub(crate) speed: Option<f64>,
    pub(crate) eta_seconds: Option<f64>,
    pub(crate) elapsed_seconds: Option<f64>,
    pub(crate) input_bytes: Option<u64>,
    pub(crate) output_bytes: Option<u64>,
    pub(crate) estimated_output_bytes: Option<u64>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "snake_case")]
struct Job {
    id: String,
    kind: String,
    status: String,
    current: usize,
    total: usize,
    last_file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    progress_detail: Option<ProgressDetail>,
    last_lines: Vec<String>,
    exit_code: Option<i32>,
    log_path: Option<String>,
    report_path: Option<String>,
    error: Option<String>,
    resource_snapshot: Option<super::resource_monitor::RuntimeResourceSnapshot>,
    queue_sync_warning: Option<String>,
    #[serde(skip_serializing)]
    stop_file: Option<PathBuf>,
    /// Lines ever recorded (`last_lines` keeps only the newest 120); lets the web bridge send each once.
    #[serde(skip_serializing)]
    lines_total: u64,
    result: Option<Value>,
    /// Live per-file results and disk use for rebuild jobs, plus a closing report.
    #[serde(skip_serializing_if = "Option::is_none")]
    batch: Option<Value>,
}

impl Job {
    /// Record where the job is and what it last reported for the current file.
    fn set_progress(
        &mut self,
        current: usize,
        total: usize,
        file_path: String,
        detail: ProgressDetail,
    ) {
        self.current = current;
        self.total = total;
        self.last_file = Some(file_path);
        self.progress_detail = Some(detail);
    }

    fn new(id: String, kind: &str) -> Self {
        Self {
            id,
            kind: kind.into(),
            status: "queued".into(),
            current: 0,
            total: 0,
            last_file: None,
            progress_detail: None,
            last_lines: Vec::new(),
            exit_code: None,
            log_path: None,
            report_path: None,
            error: None,
            resource_snapshot: None,
            queue_sync_warning: None,
            stop_file: None,
            lines_total: 0,
            result: None,
            batch: None,
        }
    }
}

/// Exit code a rebuild uses when it paused itself (low disk space or a failure
/// streak) rather than failing or finishing. Mirrors EX_TEMPFAIL.
const EXIT_PAUSED: i32 = 75;

#[derive(Clone)]
pub(crate) struct McpServer {
    jobs: Jobs,
    /// Set when hosted inside the web server: one job slot and event bus shared with the browser.
    shared: Option<shared::Shared>,
    plans: Arc<Mutex<crate::batch_plan::PlanBook>>,
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
}

impl McpServer {
    fn new() -> Self {
        Self {
            jobs: Arc::new(Mutex::new(HashMap::new())),
            shared: None,
            plans: Arc::new(Mutex::new(crate::batch_plan::PlanBook::default())),
            tool_router: Self::tool_router(),
        }
    }

    /// An MCP server hosted by the web server (`/mcp`): it takes turns with the browser on the single job
    /// slot and its jobs appear on the shared event bus.
    pub(crate) fn new_shared(shared: shared::Shared) -> Self {
        let mut server = Self::new();
        server.shared = Some(shared);
        server
    }

    /// Start publishing this server's job changes as events. Call once, after `new_shared`.
    pub(crate) fn start_event_bridge(&self) {
        if let Some(shared) = self.shared.clone() {
            shared::spawn(self.jobs.clone(), shared);
        }
    }

    fn privilege(&self) -> super::provision::Privilege {
        match &self.shared {
            Some(shared) => (shared.privilege)(),
            None => super::provision::Privilege::Desktop,
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
            return Err("Another analysis, media run, repair, helper installation, or organizer operation is active; wait for it to finish before starting this operation.".into());
        }
        if jobs.len() >= 100 {
            let mut finished: Vec<String> = jobs
                .values()
                .filter(|job| {
                    matches!(
                        job.status.as_str(),
                        "completed" | "completed_with_failures" | "failed" | "stopped" | "paused"
                    )
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
        if let Some(shared) = &self.shared {
            shared.claim(&jobs, &job)?;
        }
        jobs.insert(job.id.clone(), job);
        Ok(())
    }
}

/// The setup plan reads the network and the package database with blocking clients, which must not run
/// on the async runtime that serves MCP requests (it panics when the blocking client is dropped there).
async fn plan_off_the_runtime(
    privilege: super::provision::Privilege,
) -> Result<(super::provision::ProvisionPlan, Option<String>), ErrorData> {
    tokio::task::spawn_blocking(move || {
        super::provision::current_plan_for(ToolPaths::default(), privilege)
    })
    .await
    .map_err(|error| {
        ErrorData::internal_error(format!("The setup plan task failed: {error}"), None)
    })?
    .map_err(|error| ErrorData::internal_error(error, None))
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
            job.lines_total += 1;
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

fn canonical_folder(value: &str) -> Result<PathBuf, String> {
    let root = PathBuf::from(value)
        .canonicalize()
        .map_err(|error| format!("Could not open media folder: {error}"))?;
    if !root.is_dir() {
        return Err("The selected media path is not a folder.".into());
    }
    Ok(root)
}

fn canonical_media_input(value: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(value)
        .canonicalize()
        .map_err(|error| format!("Could not open media input: {error}"))?;
    if !path.is_dir() && !path.is_file() {
        return Err("The selected media path is not a file or folder.".into());
    }
    Ok(path)
}

fn tool_path_arguments(
    paths: Option<ToolPaths>,
    legacy_ffmpeg_path: Option<String>,
) -> Result<Vec<String>, String> {
    let mut args = Vec::new();
    append_tool_path_arguments(&mut args, paths.unwrap_or_default(), legacy_ffmpeg_path)?;
    Ok(args)
}

fn json_result(
    result: Result<Value, String>,
) -> Result<rmcp::handler::server::wrapper::Json<Value>, ErrorData> {
    result
        .map(rmcp::handler::server::wrapper::Json)
        .map_err(|error| ErrorData::invalid_params(error, None))
}

fn serialized_result<T: Serialize>(
    result: Result<T, String>,
) -> Result<rmcp::handler::server::wrapper::Json<Value>, ErrorData> {
    let value = result.and_then(|payload| {
        serde_json::to_value(payload)
            .map_err(|error| format!("Metadata response could not be serialized: {error}"))
    });
    json_result(value)
}

#[tool_router]
impl McpServer {
    #[tool(
        name = "inspect_environment",
        description = "Read the host OS, CPU, memory, GPU IDs, media-tool versions, readiness, and missing Ubuntu package recommendations. Optional tool_paths selects explicit ffmpeg, ffprobe, mediainfo, mkvpropedit, mkvmerge, or mkvextract executables; blank values use PATH, and an FFprobe sibling is detected beside FFmpeg when available. This does not install or modify anything."
    )]
    fn inspect_environment(
        &self,
        Parameters(params): Parameters<EnvironmentParams>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, ErrorData> {
        json_result(super::environment::inspect_environment(
            params.tool_paths.unwrap_or_default(),
            params.ffmpeg_path,
        ))
    }

    #[tool(
        name = "inspect_media_queue",
        description = "Read a compact page of the app's saved local media queue, including source roots, analyzed file paths, technical summaries, selected state, and per-file run outcomes. The response is paged (up to 200 files) and omits raw scan payloads, artwork bytes, and credentials. This does not scan or modify media."
    )]
    fn inspect_media_queue(
        &self,
        Parameters(params): Parameters<QueuePageParams>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, ErrorData> {
        let path =
            crate::media_queue_file().map_err(|error| ErrorData::internal_error(error, None))?;
        json_result(crate::media_queue::summarize(
            &path,
            params.offset.unwrap_or(0),
            params.limit.unwrap_or(100),
        ))
    }

    #[tool(
        name = "inspect_artwork_cache",
        description = "Read the private artwork-cache size and count. The result separates queue-assigned posters (protected), unused app-managed images with attribution sidecars (purgeable), and unrecognized entries (left untouched). This does not inspect or clear the separate x265 calibration cache."
    )]
    async fn inspect_artwork_cache(
        &self,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, ErrorData> {
        serialized_result(crate::library_artwork::inspect_app_artwork_cache().await)
    }

    #[tool(
        name = "purge_unused_artwork_cache",
        description = "Remove only unused app-managed poster images and their matching attribution files after the user has reviewed inspect_artwork_cache. Pass its current preview_id and set confirm_purge=true. Queue-assigned images, unrecognized files, and the separate x265 calibration cache are retained. A changed cache requires a fresh inspection."
    )]
    async fn purge_unused_artwork_cache(
        &self,
        Parameters(params): Parameters<ArtworkCachePurgeParams>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, ErrorData> {
        let id = job_id();
        self.insert_job(Job::new(id.clone(), "artwork_cache_purge"))
            .map_err(|error| ErrorData::invalid_params(error, None))?;
        let result = crate::library_artwork::purge_unused_app_artwork_cache(
            &params.preview_id,
            params.confirm_purge,
        )
        .await;
        let serialized = result.and_then(|payload| {
            serde_json::to_value(payload).map_err(|error| {
                format!("Artwork-cache purge response could not be serialized: {error}")
            })
        });
        if let Ok(mut jobs) = self.jobs.lock() {
            if let Some(job) = jobs.get_mut(&id) {
                job.status = if serialized.is_ok() {
                    "completed"
                } else {
                    "failed"
                }
                .into();
                job.error = serialized.as_ref().err().cloned();
                job.result = serialized.as_ref().ok().cloned();
            }
        }
        json_result(serialized.map(|payload| json!({ "job_id": id, "result": payload })))
    }

    #[tool(
        name = "set_media_selection",
        description = "Set the app's persistent global selection to exact absolute file paths already present in its analyzed local media queue. This updates queue selection only; scans, metadata reviews, tune/stream choices, run history, and media bytes are preserved. Use inspect_media_queue to discover eligible paths."
    )]
    fn set_media_selection(
        &self,
        Parameters(params): Parameters<QueueSelectionParams>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, ErrorData> {
        let path =
            crate::media_queue_file().map_err(|error| ErrorData::internal_error(error, None))?;
        json_result(
            crate::media_queue::save_selected_paths(&path, params.selected_media_paths)
                .and_then(|()| crate::media_queue::summarize(&path, 0, 100)),
        )
    }

    #[tool(
        name = "inspect_work_directory",
        description = "Resolve the temporary work directory for a local media file or folder without creating it. The default is dynamically source-adjacent `.hevc-rebuild-work`; an effective paths.work_root setting, optional work_root override, or HEVC_REBUILD_WORK_ROOT process override is reported explicitly. The response includes the app-state fallback used only when an automatic adjacent folder cannot be created."
    )]
    fn inspect_work_directory(
        &self,
        Parameters(params): Parameters<WorkDirectoryParams>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, ErrorData> {
        let payload = crate::resolve_work_directory(&params.root, params.work_root.as_deref())
            .map_err(|error| ErrorData::invalid_params(error, None))?;
        serde_json::to_value(payload)
            .map(rmcp::handler::server::wrapper::Json)
            .map_err(|error| ErrorData::internal_error(error.to_string(), None))
    }

    #[tool(
        name = "search_public_films",
        description = "Search Wikidata for film title candidates. Sends only the supplied search phrase to Wikidata; it does not access the media library or upload file contents. Results are suggestions and must be reviewed before applying."
    )]
    async fn search_public_films(
        &self,
        Parameters(params): Parameters<MetadataSearchParams>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, ErrorData> {
        serialized_result(crate::library_metadata::search_films(&params.query).await)
    }

    #[tool(
        name = "get_public_film_metadata",
        description = "Get Wikidata/Wikipedia film details and Commons artwork candidates by a reviewed Wikidata item ID. Artwork includes source and license details where available; this read-only lookup never changes media."
    )]
    async fn get_public_film_metadata(
        &self,
        Parameters(params): Parameters<FilmMetadataParams>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, ErrorData> {
        serialized_result(crate::library_metadata::load_film_details(&params.entity_id).await)
    }

    #[tool(
        name = "search_public_tv_series",
        description = "Search TVmaze for TV series candidates. Sends only the supplied search phrase to TVmaze; it does not access the media library or upload file contents. Results are suggestions and must be reviewed before applying."
    )]
    async fn search_public_tv_series(
        &self,
        Parameters(params): Parameters<MetadataSearchParams>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, ErrorData> {
        serialized_result(crate::library_metadata::search_tv_shows(&params.query).await)
    }

    #[tool(
        name = "get_public_tv_series_metadata",
        description = "Get TVmaze season and episode metadata for a reviewed show ID, including artwork source/license attribution where available. This read-only lookup never changes media."
    )]
    async fn get_public_tv_series_metadata(
        &self,
        Parameters(params): Parameters<TvMetadataParams>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, ErrorData> {
        serialized_result(crate::library_metadata::load_tv_series_details(params.show_id).await)
    }

    #[tool(
        name = "save_library_artwork",
        description = "Save a user-selected Wikidata/Wikimedia Commons, TVmaze, or TMDB poster into existing folders under one selected library root. Provide an artwork candidate returned by the metadata lookup and explicit targets (folder plus filename stem). The server verifies provider HTTPS hosts, image bytes and type, writes attribution/license JSON beside each image, refuses to overwrite anything, and rolls back created sidecars on operation errors. It does not modify video files; the next HEVC rebuild attaches a matching movie/season/show poster to its new Matroska output. This performs an external download only after this explicit tool call."
    )]
    async fn save_library_artwork(
        &self,
        Parameters(request): Parameters<crate::library_artwork::ArtworkSaveRequest>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, ErrorData> {
        serialized_result(
            crate::library_artwork::save_artwork(request.candidate, request.root, request.targets)
                .await,
        )
    }

    #[tool(
        name = "cache_library_artwork",
        description = "Download one user-selected artwork candidate returned by metadata search into HEVC Rebuild Studio's private app artwork cache, alongside its source/license details. The image is made available for preview and poster overrides without placing files beside media. The download occurs only after this explicit tool call and only from an approved provider HTTPS host."
    )]
    async fn cache_library_artwork(
        &self,
        Parameters(candidate): Parameters<crate::library_metadata::ArtworkCandidate>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, ErrorData> {
        serialized_result(crate::library_artwork::save_artwork_to_cache(candidate).await)
    }

    #[tool(
        name = "cache_local_library_artwork",
        description = "Copy one explicitly selected local JPEG, PNG, or WebP image into the private app artwork cache. This reads only the provided image file, validates its bytes and size, and never overwrites an existing cache entry."
    )]
    async fn cache_local_library_artwork(
        &self,
        Parameters(params): Parameters<LocalArtworkCacheParams>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, ErrorData> {
        serialized_result(
            crate::library_artwork::save_local_artwork_to_cache(params.source_path).await,
        )
    }

    #[tool(
        name = "inspect_local_library_assets",
        description = "Inspect recognized poster/artwork sidecars for media paths and folders under one selected library root. This read-only operation does not contact public metadata providers or alter files. Sidecar NFO files are not read; use the public search tools for titles and descriptions. Artwork bytes stay on the device and only paths are returned."
    )]
    fn inspect_local_library_assets(
        &self,
        Parameters(request): Parameters<crate::library_assets::LocalAssetsRequest>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, ErrorData> {
        serialized_result(crate::library_assets::scan_local_assets(request))
    }

    #[tool(
        name = "scan_library_names",
        description = "List local video filenames and parse common SxxEyy episode codes under one user-selected folder. This bounded Rust scan does not inspect file contents, contact metadata providers, or write anything. It skips symlinks and common output/backup folders and stops at 10,000 video files."
    )]
    fn scan_library_names(
        &self,
        Parameters(params): Parameters<OrganizerFolderParams>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, ErrorData> {
        let id = job_id();
        let mut job = Job::new(id.clone(), "organizer_scan");
        job.status = "running".into();
        self.insert_job(job)
            .map_err(|error| ErrorData::internal_error(error, None))?;
        match crate::library_organizer::scan_organizer_folder(params.root) {
            Ok(result) => {
                let value = serde_json::to_value(result)
                    .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
                if let Ok(mut jobs) = self.jobs.lock() {
                    if let Some(job) = jobs.get_mut(&id) {
                        job.status = "completed".into();
                        job.result = Some(value.clone());
                    }
                }
                Ok(rmcp::handler::server::wrapper::Json(value))
            }
            Err(error) => {
                update_failed(&self.jobs, &id, error.clone());
                Err(ErrorData::invalid_params(error, None))
            }
        }
    }

    #[tool(
        name = "preview_library_rename",
        description = "Render and collision-check a batch of local movie/TV naming templates without changing files. Pass root and item entries containing sourcePath, kind (movie, series, season, episode), template, and shortcode values. The result includes exact before/after paths, Unicode/case-normalized duplicates, occupied destinations, and a previewId. Resolve every conflict and inspect the full result before apply."
    )]
    fn preview_library_rename(
        &self,
        Parameters(params): Parameters<RenamePreviewParams>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, ErrorData> {
        let id = job_id();
        let mut job = Job::new(id.clone(), "rename_preview");
        job.status = "running".into();
        self.insert_job(job)
            .map_err(|error| ErrorData::internal_error(error, None))?;
        let patterns = match params.text_blacklist_patterns {
            Some(patterns) => patterns,
            None => crate::settings::metadata_text_blacklist_for_root(&params.root)
                .map_err(|error| ErrorData::invalid_params(error, None))?,
        };
        match crate::library_organizer::preview_rename_batch_with_patterns(
            params.root,
            params.items,
            patterns,
        ) {
            Ok(result) => {
                let value = serde_json::to_value(result)
                    .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
                if let Ok(mut jobs) = self.jobs.lock() {
                    if let Some(job) = jobs.get_mut(&id) {
                        job.status = "completed".into();
                        job.result = Some(value.clone());
                    }
                }
                Ok(rmcp::handler::server::wrapper::Json(value))
            }
            Err(error) => {
                update_failed(&self.jobs, &id, error.clone());
                Err(ErrorData::invalid_params(error, None))
            }
        }
    }

    #[tool(
        name = "apply_library_rename",
        description = "Apply a previously previewed local naming plan only when confirmRename=true and previewId matches the current paths and filesystem state. The backend rechecks collisions and source snapshots, uses staged same-directory renames to handle cycles without overwriting, and rolls back on an operation error. File contents are not edited. Poll get_job for completion."
    )]
    fn apply_library_rename(
        &self,
        Parameters(params): Parameters<RenameApplyParams>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, ErrorData> {
        if !params.confirm_rename {
            return Err(ErrorData::invalid_params(
                "Set confirm_rename=true only after reviewing a conflict-free rename preview.",
                None,
            ));
        }
        let id = job_id();
        let mut job = Job::new(id.clone(), "rename_apply");
        job.status = "running".into();
        self.insert_job(job)
            .map_err(|error| ErrorData::invalid_params(error, None))?;
        let patterns = match params.text_blacklist_patterns {
            Some(patterns) => patterns,
            None => crate::settings::metadata_text_blacklist_for_root(&params.root)
                .map_err(|error| ErrorData::invalid_params(error, None))?,
        };
        let jobs = self.jobs.clone();
        let worker_id = id.clone();
        thread::spawn(move || {
            match crate::library_organizer::apply_rename_batch_with_patterns(
                params.root,
                params.items,
                patterns,
                params.preview_id,
                true,
            ) {
                Ok(result) => {
                    let value = serde_json::to_value(result)
                        .unwrap_or_else(|error| json!({"error": error.to_string()}));
                    if let Ok(mut jobs) = jobs.lock() {
                        if let Some(job) = jobs.get_mut(&worker_id) {
                            job.status = "completed".into();
                            job.result = Some(value);
                        }
                    }
                }
                Err(error) => update_failed(&jobs, &worker_id, error),
            }
        });
        Ok(rmcp::handler::server::wrapper::Json(
            json!({"job_id": id, "status": "running", "poll_with": "get_job"}),
        ))
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
        let payload = crate::settings::load_effective_settings(&root.to_string_lossy())
            .map_err(|error| ErrorData::internal_error(error, None))?;
        let payload = serde_json::to_value(payload)
            .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
        Ok(rmcp::handler::server::wrapper::Json(payload))
    }

    #[tool(
        name = "start_folder_analysis",
        description = "Analyze one local media file or a folder without encoding or writing to the media input. Optional tool_paths selects explicit ffmpeg, ffprobe, mediainfo, mkvpropedit, mkvmerge, or mkvextract executables. Returns a job ID for progress, paginated per-file errors and stream metadata, current available/total memory context, profile-aware rebuild forecasts, and YouTube size/memory estimates for source, 1440p, and 4K targets. Forecasts are heuristics, not exact peak-RAM or output-size guarantees. Use request_safe_stop on the job ID to finish the current file and skip the rest. Only process content you own or are authorized to use."
    )]
    fn start_folder_analysis(
        &self,
        Parameters(params): Parameters<ScanParams>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, ErrorData> {
        let root = canonical_media_input(&params.root)
            .map_err(|error| ErrorData::invalid_params(error, None))?;
        let minimum = params.minimum_size_mib.unwrap_or(5);
        if minimum > 4096 {
            return Err(ErrorData::invalid_params(
                "Minimum input size must be 4096 MiB or less.",
                None,
            ));
        }
        let mut tool_paths = params.tool_paths.unwrap_or_default();
        if tool_paths.ffmpeg.is_none() {
            tool_paths.ffmpeg = params.ffmpeg_path;
        }
        let stop_token = crate::stop_control::StopToken::new()
            .map_err(|error| ErrorData::internal_error(error, None))?;
        let stop_file = stop_token.path().to_path_buf();
        let options = crate::media_scan::ScanOptions {
            root: root.to_string_lossy().into_owned(),
            recursive: params.recursive.unwrap_or(true),
            minimum_size_mib: minimum,
            filename_include_patterns: params.filename_include_patterns.unwrap_or_default(),
            filename_exclude_patterns: params.filename_exclude_patterns.unwrap_or_default(),
            tool_paths,
            stop_file: Some(stop_file.clone()),
        };
        let id = job_id();
        let mut job = Job::new(id.clone(), "folder_analysis");
        job.status = "running".into();
        job.stop_file = Some(stop_file);
        self.insert_job(job)
            .map_err(|error| ErrorData::internal_error(error, None))?;
        let jobs = self.jobs.clone();
        let worker_id = id.clone();
        thread::spawn(move || {
            let _stop_token = stop_token;
            run_scan_job(jobs, worker_id, options);
        });
        Ok(rmcp::handler::server::wrapper::Json(
            json!({"job_id": id, "status": "running", "poll_with": "get_job"}),
        ))
    }

    #[tool(
        name = "start_integrity_check",
        description = "Check one or more local media files for container or bitstream damage without writing output. level=quick inspects structure and decodes a short sample; level=full decodes every stream to FFmpeg's null muxer. Returns a job ID; poll with get_job. Only process content you own or are authorized to use."
    )]
    fn start_integrity_check(
        &self,
        Parameters(params): Parameters<IntegrityParams>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, ErrorData> {
        if params.files.is_empty() || params.files.len() > 10_000 {
            return Err(ErrorData::invalid_params(
                "Integrity checks require 1 to 10,000 absolute media files.",
                None,
            ));
        }
        let level =
            crate::integrity::IntegrityLevel::parse(params.level.as_deref().unwrap_or("quick"))
                .map_err(|error| ErrorData::invalid_params(error, None))?;
        let mut files = Vec::new();
        for raw in &params.files {
            let path = canonical_media_input(raw)
                .map_err(|error| ErrorData::invalid_params(error, None))?;
            if !path.is_file() {
                return Err(ErrorData::invalid_params(
                    format!("{} is not a file.", path.display()),
                    None,
                ));
            }
            files.push(path);
        }
        let mut tool_paths = params.tool_paths.unwrap_or_default();
        if tool_paths.ffmpeg.is_none() {
            tool_paths.ffmpeg = params.ffmpeg_path;
        }
        let id = job_id();
        let stop_token = crate::stop_control::StopToken::new()
            .map_err(|error| ErrorData::internal_error(error, None))?;
        let stop_file = stop_token.path().to_path_buf();
        let mut job = Job::new(id.clone(), "integrity_check");
        job.status = "running".into();
        job.stop_file = Some(stop_file.clone());
        self.insert_job(job)
            .map_err(|error| ErrorData::internal_error(error, None))?;
        let jobs = self.jobs.clone();
        let worker_id = id.clone();
        thread::spawn(move || {
            let _stop_token = stop_token;
            run_integrity_job(jobs, worker_id, files, level, tool_paths, stop_file);
        });
        Ok(rmcp::handler::server::wrapper::Json(
            json!({"job_id": id, "status": "running", "poll_with": "get_job", "level": params.level.unwrap_or_else(|| "quick".into())}),
        ))
    }

    #[tool(
        name = "explain_settings",
        description = "Read-only. Plain-language explanations of the encoder settings, exactly as the app shows them to people: what CRF means (pass topic=crf and crf=<0-51> for that number's quality band and rough size), presets, content tune, short test, and individual settings such as hdr10plus, dolbyVision, secondaryVideo, copyVerification, qualityCheck, minSsim and the YouTube options. Omit topic for everything. Use this to explain a choice to the user before changing it."
    )]
    fn explain_settings(
        &self,
        Parameters(params): Parameters<ExplainParams>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, ErrorData> {
        super::help_content::explain(params.topic.as_deref(), params.crf)
            .map(rmcp::handler::server::wrapper::Json)
            .map_err(|error| ErrorData::invalid_params(error, None))
    }

    #[tool(
        name = "app_guide",
        description = "Read-only. The application guide from the app itself: section=workflow (a typical agent workflow), modes (what each run mode does and which confirm field it needs), safety (what protects the user's files), headless (running the app in a browser or on a server: sign-in, read-only, --root, HTTPS, setup without a password) or mcp (how this server behaves). Omit section to list them."
    )]
    fn app_guide(
        &self,
        Parameters(params): Parameters<GuideParams>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, ErrorData> {
        super::help_content::guide(params.section.as_deref())
            .map(rmcp::handler::server::wrapper::Json)
            .map_err(|error| ErrorData::invalid_params(error, None))
    }

    #[tool(
        name = "get_job",
        description = "Get scan, encode, repair, or helper-install progress. For media runs, resource_snapshot contains the latest Rust-observed system RAM/swap pressure, CPU/load, disk free space, and engine/FFmpeg process memory where supported; active safety guards and pressure retries run in the native Rust coordinators. For folder analysis, request bounded result pages with offset and limit (maximum 250). Includes recent diagnostic lines, job status, and per-file errors. For rebuild jobs, batch holds live per-file results, space written and originals retained in .hevc-rebuild-originals, free space now, an ETA, the reason for any halt, and a closing report. Status paused means the run stopped itself to protect the disk or after repeated failures; remaining files were not touched. completed_with_failures means every file was attempted and some failed safely (see batch.failures)."
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
        name = "preview_media_outputs",
        description = "Read-only output preflight for a batch spanning one or more scanned roots. Pass mode metadata, test, or full; each source contains an absolute root and selected absolute media paths. Optional output_dir must already exist and mirrors folder-relative paths; file_output_dirs maps individual absolute media paths to existing output folders. Returns bounded proposed paths and every blocking existing-output or duplicate-target collision before any media is written. Full in-place replacement of a source is allowed only when its planned target is that same source. The engine repeats its own per-source checks immediately before writing."
    )]
    fn preview_media_outputs(
        &self,
        Parameters(params): Parameters<crate::output_targets::OutputPreviewRequest>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, ErrorData> {
        let preview = crate::output_targets::preview(params)
            .map_err(|error| ErrorData::invalid_params(error, None))?;
        let payload = serde_json::to_value(preview)
            .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
        Ok(rmcp::handler::server::wrapper::Json(payload))
    }

    #[tool(
        name = "start_media_run",
        description = "Start one run on a local file or folder. mode: check (default, read-only), metadata (copy to a new MKV with reviewed metadata and artwork, no encode), test (short-clip outputs), full (validated in-place HEVC rebuild; the original moves to a hidden .hevc-rebuild-originals folder beside it), or youtube (separate upload copies). To rebuild a whole library unattended, use plan_library_batch then start_planned_batch instead: they add a storage forecast, a protected free-space floor and pause-on-trouble. test, full and youtube each need their confirm_* field. selected_files restricts the run to absolute paths from the analyzed input; omit it to process every eligible file (this app's backup folders, .hevc-rebuild-originals and the legacy old, are skipped unless filename_exclude_patterns is given). Per-file maps are keyed by absolute path and documented on each field. Destination files are never overwritten. Poll get_job for progress and per-file results; request_safe_stop finishes the current file. Only process content you own or are authorized to use."
    )]
    fn start_media_run(
        &self,
        Parameters(params): Parameters<StartRunParams>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, ErrorData> {
        let id = job_id();
        let stop_token = crate::stop_control::StopToken::new()
            .map_err(|error| ErrorData::internal_error(error, None))?;
        let stop_file = stop_token.path().to_path_buf();
        let prepared = prepare_media_run(params, &stop_file)?;
        let root = prepared.root.clone();
        let mode = prepared.mode.clone();
        let selected_files = prepared.selected_files.clone();
        let args = prepared.args.clone();
        let mut job = Job::new(id.clone(), "media_run");
        job.status = "running".into();
        job.stop_file = Some(stop_file.clone());
        let (mut queue_sync, mut queue_sync_warning) =
            match queue_run_sync(&root, &selected_files, &mode) {
                Ok(sync) => (sync, None),
                Err(error) => (
                    None,
                    Some(format!(
                        "The media run will proceed without queue status synchronization: {error}"
                    )),
                ),
            };
        job.queue_sync_warning = queue_sync_warning.clone();
        self.insert_job(job)
            .map_err(|error| ErrorData::internal_error(error, None))?;
        if let Some(sync) = queue_sync.as_ref() {
            let states = queue_run_states(
                &sync.media_paths,
                &mode,
                "running",
                Some("Started through MCP."),
            );
            if let Err(error) = crate::media_queue::save_run_states(&sync.queue_path, states) {
                queue_sync = None;
                queue_sync_warning = Some(format!(
                    "The media run will proceed, but its queue status could not be saved: {error}"
                ));
                if let Ok(mut jobs) = self.jobs.lock() {
                    if let Some(job) = jobs.get_mut(&id) {
                        job.queue_sync_warning = queue_sync_warning.clone();
                    }
                }
            }
        }
        let queue_items_tracked = queue_sync.as_ref().map_or(0, |sync| sync.media_paths.len());
        let jobs = self.jobs.clone();
        let worker_id = id.clone();
        thread::spawn(move || {
            let _stop_token = stop_token;
            if prepared.mode == "check" {
                run_check_job(jobs, worker_id, prepared, stop_file, queue_sync);
            } else {
                run_media_job(jobs, worker_id, args, stop_file, queue_sync);
            }
        });
        Ok(rmcp::handler::server::wrapper::Json(
            json!({"job_id": id, "status": "running", "poll_with": "get_job", "queue_items_tracked": queue_items_tracked, "queue_sync_warning": queue_sync_warning}),
        ))
    }

    #[tool(
        name = "start_media_queue_run",
        description = "Run several explicitly selected media sources sequentially as one resumable MCP job. Pass 1 to 500 runs using the same fields as start_media_run, with a non-empty selected_files array for every source; group each source's selected files under its analyzed root. All sources must use the same mode. Selected canonical paths are deduplicated across roots, and metadata/test/full output destinations are preflighted together before the first engine starts. Each source keeps its own settings, tool paths, and per-file overrides. Queue entries are marked queued/running and synchronized from each source's own CSV report; absent report rows remain needs_review. stop_on_error defaults to true, and request_safe_stop finishes the active file/source before leaving later sources queued. Full rebuild and short-test confirmations are still required on every run object. YouTube output also runs one source at a time and keeps its existing no-overwrite behavior. Only process media you own or are authorized to use."
    )]
    fn start_media_queue_run(
        &self,
        Parameters(params): Parameters<StartQueueRunParams>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, ErrorData> {
        let stop_on_error = params.stop_on_error.unwrap_or(true);
        let acknowledge_review_paths = params.acknowledge_review_paths.unwrap_or_default();
        if params.runs.is_empty() || params.runs.len() > 500 {
            return Err(ErrorData::invalid_params(
                "A queued run must contain between 1 and 500 source runs.",
                None,
            ));
        }
        let id = job_id();
        let stop_token = crate::stop_control::StopToken::new()
            .map_err(|error| ErrorData::internal_error(error, None))?;
        let stop_file = stop_token.path().to_path_buf();
        let mut prepared = Vec::with_capacity(params.runs.len());
        for mut run in params.runs {
            if run.selected_files.as_ref().is_none_or(Vec::is_empty) {
                return Err(ErrorData::invalid_params(
                    "Every queued source run requires an explicit non-empty selected_files list.",
                    None,
                ));
            }
            run.stop_on_error = Some(stop_on_error);
            prepared.push(prepare_media_run(run, &stop_file)?);
        }
        validate_prepared_queue_runs(&prepared)?;
        validate_queue_recovery(&prepared, &acknowledge_review_paths)?;

        let mut queue_syncs = Vec::with_capacity(prepared.len());
        let mut queue_sync_warning = None;
        for run in &prepared {
            match queue_run_sync(&run.root, &run.selected_files, &run.mode) {
                Ok(sync) => queue_syncs.push(sync),
                Err(error) => {
                    queue_sync_warning = Some(format!(
                        "The media queue will run without per-file queue synchronization: {error}"
                    ));
                    queue_syncs = vec![None; prepared.len()];
                    break;
                }
            }
        }
        let mut queue_items_tracked = queue_syncs
            .iter()
            .flatten()
            .map(|sync| sync.media_paths.len())
            .sum::<usize>();
        let mut plans = prepared
            .into_iter()
            .zip(queue_syncs)
            .map(|(run, queue_sync)| QueueRunPlan { run, queue_sync })
            .collect::<Vec<_>>();
        let source_count = plans.len();
        let planned_files = plans.iter().map(|plan| plan.run.selected_files.len()).sum();
        let mut job = Job::new(id.clone(), "media_queue_run");
        job.status = "running".into();
        job.total = planned_files;
        job.stop_file = Some(stop_file.clone());
        job.queue_sync_warning = queue_sync_warning.clone();
        self.insert_job(job)
            .map_err(|error| ErrorData::internal_error(error, None))?;
        if queue_sync_warning.is_none() {
            let mut queued_states = BTreeMap::new();
            for sync in plans.iter().filter_map(|plan| plan.queue_sync.as_ref()) {
                queued_states.extend(queue_run_states(
                    &sync.media_paths,
                    &sync.mode,
                    "queued",
                    Some("Waiting for the preceding source batch."),
                ));
            }
            if let Some(queue_path) = plans
                .iter()
                .filter_map(|plan| plan.queue_sync.as_ref())
                .next()
                .map(|sync| &sync.queue_path)
            {
                if let Err(error) = crate::media_queue::save_run_states(queue_path, queued_states) {
                    queue_sync_warning = Some(format!(
                        "The media queue will run, but queued states could not be saved: {error}"
                    ));
                    queue_items_tracked = 0;
                    for plan in &mut plans {
                        plan.queue_sync = None;
                    }
                    if let Ok(mut registry) = self.jobs.lock() {
                        if let Some(job) = registry.get_mut(&id) {
                            job.queue_sync_warning = queue_sync_warning.clone();
                        }
                    }
                }
            }
        }
        let jobs = self.jobs.clone();
        let worker_id = id.clone();
        thread::spawn(move || {
            let _stop_token = stop_token;
            run_media_queue_job(jobs, worker_id, plans, stop_file, stop_on_error, false)
        });
        Ok(rmcp::handler::server::wrapper::Json(json!({
            "job_id": id,
            "status": "running",
            "poll_with": "get_job",
            "source_count": source_count,
            "file_count": planned_files,
            "queue_items_tracked": queue_items_tracked,
            "stop_on_error": stop_on_error,
            "queue_sync_warning": queue_sync_warning,
        })))
    }

    #[tool(
        name = "plan_library_batch",
        description = "Step 1 of rebuilding a whole library unattended. Read-only: finds every file under an absolute folder that a full rebuild would process, records its size and modification time, forecasts disk use per volume (originals are kept in hidden .hevc-rebuild-originals folders, so usage grows), and chooses a protected free-space floor. Returns plan_id and show_the_user, a plain-language plan. Show show_the_user to the user, wait for their go-ahead, then call start_planned_batch. Nothing is written."
    )]
    fn plan_library_batch(
        &self,
        Parameters(params): Parameters<PlanBatchParams>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, ErrorData> {
        let root = canonical_folder(&params.root)
            .map_err(|error| ErrorData::invalid_params(error, None))?;
        let plan = build_batch_plan(&root, &params)
            .map_err(|error| ErrorData::invalid_params(error, None))?;
        let payload = plan.to_json();
        self.plans
            .lock()
            .map_err(|_| ErrorData::internal_error("The plan registry is unavailable.", None))?
            .insert(plan);
        Ok(rmcp::handler::server::wrapper::Json(payload))
    }

    #[tool(
        name = "start_planned_batch",
        description = "Step 2: run a plan made by plan_library_batch, after the user has agreed to it. Needs only plan_id. Refused if any planned file changed since planning or free space is already below the plan's floor. The run then works through every file unattended: a validated output replaces each original in place (the original moves to a hidden .hevc-rebuild-originals folder), a failing file (including one whose picture-quality check falls below the SSIM floor) is left as it was, and the run pauses itself if free space reaches the floor or several files fail in a row. Poll get_job for live per-file results, disk use and a final report; use request_safe_stop to stop after the current file. To continue after a pause, free space and plan the same folder again; files that already have a backup are skipped."
    )]
    fn start_planned_batch(
        &self,
        Parameters(params): Parameters<StartPlannedParams>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, ErrorData> {
        let plan = self
            .plans
            .lock()
            .map_err(|_| ErrorData::internal_error("The plan registry is unavailable.", None))?
            .take(&params.plan_id)
            .map_err(|error| ErrorData::invalid_params(error, None))?;
        if let Err(problems) = plan.verify_unchanged() {
            return Err(ErrorData::invalid_params(
                format!(
                    "The library changed after it was planned, so this plan is void. Plan again. {}",
                    problems.join(" ")
                ),
                None,
            ));
        }
        if let Some(problem) = plan_space_problem(&plan) {
            let message = format!("{problem} Free some space, then start this plan again.");
            self.plans
                .lock()
                .map_err(|_| ErrorData::internal_error("The plan registry is unavailable.", None))?
                .restore(plan);
            return Err(ErrorData::invalid_params(message, None));
        }
        let id = job_id();
        let stop_token = crate::stop_control::StopToken::new()
            .map_err(|error| ErrorData::internal_error(error, None))?;
        let stop_file = stop_token.path().to_path_buf();
        let run = StartRunParams {
            root: plan.root.to_string_lossy().into_owned(),
            mode: Some("full".into()),
            confirm_full_rebuild: Some(true),
            settings_json: Some(plan.settings_json.clone()),
            selected_files: Some(
                plan.files
                    .iter()
                    .map(|file| file.path.to_string_lossy().into_owned())
                    .collect(),
            ),
            ..StartRunParams::default()
        };
        let mut prepared = prepare_media_run(run, &stop_file)?;
        prepared.args.extend([
            "--max-consecutive-failures".into(),
            plan.max_consecutive_failures.to_string(),
        ]);
        let (queue_sync, queue_sync_warning) =
            match queue_run_sync(&prepared.root, &prepared.selected_files, &prepared.mode) {
                Ok(sync) => (sync, None),
                Err(error) => (
                    None,
                    Some(format!(
                        "The batch will run without queue status synchronization: {error}"
                    )),
                ),
            };
        let mut job = Job::new(id.clone(), "library_batch");
        job.status = "running".into();
        job.total = plan.files.len();
        job.stop_file = Some(stop_file.clone());
        job.queue_sync_warning = queue_sync_warning.clone();
        if let Err(error) = self.insert_job(job) {
            self.plans
                .lock()
                .map_err(|_| ErrorData::internal_error("The plan registry is unavailable.", None))?
                .restore(plan);
            return Err(ErrorData::invalid_params(error, None));
        }
        if let Some(sync) = queue_sync.as_ref() {
            let states = queue_run_states(
                &sync.media_paths,
                &prepared.mode,
                "running",
                Some("Started through an MCP library batch."),
            );
            let _ = crate::media_queue::save_run_states(&sync.queue_path, states);
        }
        let jobs = self.jobs.clone();
        let worker_id = id.clone();
        let file_count = plan.files.len();
        let args = prepared.args;
        thread::spawn(move || {
            let _stop_token = stop_token;
            run_media_job(jobs, worker_id, args, stop_file, queue_sync);
        });
        Ok(rmcp::handler::server::wrapper::Json(json!({
            "job_id": id,
            "status": "running",
            "file_count": file_count,
            "protected_free_space_gib": plan.floor_gib,
            "poll_with": "get_job",
            "queue_sync_warning": queue_sync_warning,
        })))
    }

    #[tool(
        name = "list_jobs",
        description = "List this server's recent jobs (newest first) with id, kind, status and progress, so a job can be found again after context is lost. Use get_job for details."
    )]
    fn list_jobs(&self) -> Result<rmcp::handler::server::wrapper::Json<Value>, ErrorData> {
        let jobs = self
            .jobs
            .lock()
            .map_err(|_| ErrorData::internal_error("The MCP job registry is unavailable.", None))?;
        let mut rows: Vec<&Job> = jobs.values().collect();
        rows.sort_by(|a, b| b.id.cmp(&a.id));
        let rows: Vec<Value> = rows
            .into_iter()
            .take(25)
            .map(|job| {
                json!({
                    "job_id": job.id,
                    "kind": job.kind,
                    "status": job.status,
                    "current": job.current,
                    "total": job.total,
                    "last_file": job.last_file,
                })
            })
            .collect();
        Ok(rmcp::handler::server::wrapper::Json(json!({"jobs": rows})))
    }

    #[tool(
        name = "request_safe_stop",
        description = "Request a running media, integrity, or folder-analysis job to finish its current file and stop before starting another. This does not kill FFmpeg mid-file."
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
            if !matches!(
                job.kind.as_str(),
                "media_run"
                    | "media_queue_run"
                    | "library_batch"
                    | "integrity_check"
                    | "folder_analysis"
            ) || job.status != "running"
            {
                return Err(ErrorData::invalid_params(
                    "Only an active media, integrity, or folder-analysis job can be safely stopped.",
                    None,
                ));
            }
            job.stop_file.clone().ok_or_else(|| {
                ErrorData::internal_error("The media run has no stop-request file.", None)
            })?
        };
        crate::stop_control::request(&path)
            .map_err(|error| ErrorData::internal_error(error, None))?;
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
        let directory =
            super::state_log_directory().map_err(|error| ErrorData::internal_error(error, None))?;
        Ok(rmcp::handler::server::wrapper::Json(
            super::reports::list_reports(&directory, limit),
        ))
    }

    #[tool(
        name = "read_diagnostic_log",
        description = "Read at most the final 1 MB of one HEVC Rebuild .log file from this user's application log folder. Other filesystem paths are rejected."
    )]
    fn read_diagnostic_log(
        &self,
        Parameters(params): Parameters<LogParams>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, ErrorData> {
        let allowed = super::state_log_directory()
            .map_err(|error| ErrorData::internal_error(error, None))?
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
        name = "check_ubuntu_package_updates",
        description = "Read the locally cached Ubuntu APT package index and compare installed/candidate versions for HEVC Rebuild's fixed allowlist. This is opt-in, read-only, never runs apt-get update, and reports whether the local catalog is stale."
    )]
    fn check_ubuntu_package_updates(
        &self,
        Parameters(_params): Parameters<AptUpdateParams>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, ErrorData> {
        json_result(super::dependencies::apt_update_status())
    }

    #[tool(
        name = "plan_system_setup",
        description = "Read-only. Returns the one-pass setup plan: missing and upgradable packages from the fixed Ubuntu allowlist, whether the package catalog needs a refresh, the HDR10+ helper action (install, update, current), and whether one administrator password prompt is needed. Call this first, show the steps to the user, then call provision_system."
    )]
    async fn plan_system_setup(
        &self,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, ErrorData> {
        let privilege = self.privilege();
        let (plan, error) = plan_off_the_runtime(privilege).await?;
        let mut value = super::provision::plan_json(&plan, error.as_deref());
        super::apt_commands::annotate_plan_json(&mut value, privilege);
        Ok(rmcp::handler::server::wrapper::Json(value))
    }

    #[tool(
        name = "provision_system",
        description = "Install and update everything in one pass after the user approves the plan from plan_system_setup (set confirm_plan=true). At most ONE administrator prompt appears on the user's desktop covering the catalog refresh, missing packages and package updates; the HDR10+ helper is installed or updated in user space with no password. The server recomputes the plan itself, so no package list is accepted. The model never sees or handles the password."
    )]
    async fn provision_system(
        &self,
        Parameters(params): Parameters<ProvisionParams>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, ErrorData> {
        if !params.confirm_plan {
            return Err(ErrorData::invalid_params(
                "Set confirm_plan=true after the user has reviewed plan_system_setup.",
                None,
            ));
        }
        let privilege = self.privilege();
        let (plan, _) = plan_off_the_runtime(privilege).await?;
        if plan.nothing_to_do() {
            return Ok(rmcp::handler::server::wrapper::Json(
                json!({"status":"already_ready"}),
            ));
        }
        let id = job_id();
        let mut job = Job::new(id.clone(), "system_provision");
        job.status = "running".into();
        self.insert_job(job)
            .map_err(|error| ErrorData::internal_error(error, None))?;
        let jobs = self.jobs.clone();
        let worker_id = id.clone();
        thread::spawn(move || apt::run_provision_job(jobs, worker_id, plan, privilege));
        Ok(rmcp::handler::server::wrapper::Json(
            json!({"job_id":id,"status":"running","poll_with":"get_job"}),
        ))
    }

    #[tool(
        name = "install_hdr10plus_helper",
        description = "Install the latest upstream HDR10+ helper after explicit confirmation. Rust requires and verifies the release SHA-256 digest, safely reads the expected tar entry, checks the staged binary version, and places it in this user's managed application data directory and the AppImage sidecar when that profile is active."
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
        thread::spawn(
            move || match super::dependencies::install_latest_hdr10plus_tool() {
                Ok(path) => {
                    if let Ok(mut jobs) = jobs.lock() {
                        if let Some(job) = jobs.get_mut(&worker_id) {
                            job.status = "completed".into();
                            job.result = Some(json!({ "installed": path }));
                        }
                    }
                }
                Err(error) => update_failed(&jobs, &worker_id, error),
            },
        );
        Ok(rmcp::handler::server::wrapper::Json(
            json!({"job_id":id,"status":"running","poll_with":"get_job"}),
        ))
    }
}

#[tool_handler(
    name = "hevc-rebuild-studio",
    version = "1.4.1",
    instructions = "Local HEVC rebuild MCP server (Linux, macOS and Windows). Inspect first: inspect_environment, then analyze; analysis is read-only and runs default to check mode. Short tests, full rebuilds, renames, artwork saves and package setup each need their own confirm field: show the plan or preview to the user before setting it. Only one run, scan or setup job is active at a time; a second start reports it is already active, whether it was started here or in the app's browser view, which shows this server's jobs live. Poll get_job. System setup is plan_system_setup then provision_system after the user approves; no password is ever requested, so on a server without a desktop the plan reports what to change. To explain a choice to the user call explain_settings (CRF, presets, tune, HDR and the rest); for the workflow, run modes, safety rules and headless notes call app_guide. A read-only server does not offer this endpoint. Only process media the user owns or is authorized to use."
)]
impl ServerHandler for McpServer {}

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

mod apt;
mod batch;
#[cfg(test)]
mod batch_tool_tests;
mod exec;
mod params;
mod prepare;
mod queue_sync;
#[cfg(test)]
mod queue_sync_tests;
pub(crate) mod shared;

pub(crate) use self::queue_sync::queue_run_json_cli;
use self::{batch::*, exec::*, params::*, prepare::*, queue_sync::*};
