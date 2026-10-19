use super::{
    append_tool_path_arguments, executable_in_path, settings_arguments, standalone_command,
    stream_override_arguments, validate_apt_update_confirmation, validate_apt_upgrade_plan,
    youtube_hdr10plus_loss_arguments, ToolPaths, APT_ALLOWLIST,
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
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
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
    resource_snapshot: Option<super::resource_monitor::RuntimeResourceSnapshot>,
    queue_sync_warning: Option<String>,
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
            resource_snapshot: None,
            queue_sync_warning: None,
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
            return Err("Another analysis, media run, repair, helper installation, or organizer operation is active; wait for it to finish before starting this operation.".into());
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
    tool_paths: Option<ToolPaths>,
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
    filename_include_patterns: Option<Vec<String>>,
    filename_exclude_patterns: Option<Vec<String>>,
    ffmpeg_path: Option<String>,
    tool_paths: Option<ToolPaths>,
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
struct QueuePageParams {
    offset: Option<usize>,
    limit: Option<usize>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct QueueSelectionParams {
    selected_media_paths: Vec<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct ArtworkCachePurgeParams {
    preview_id: String,
    confirm_purge: bool,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct LocalArtworkCacheParams {
    source_path: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct StartRunParams {
    root: String,
    mode: Option<String>,
    youtube_resolution: Option<String>,
    youtube_quality_mode: Option<String>,
    youtube_source_multiplier: Option<f64>,
    upload_output_dir: Option<String>,
    output_dir: Option<String>,
    recursive: Option<bool>,
    minimum_size_mib: Option<u32>,
    ffmpeg_path: Option<String>,
    tool_paths: Option<ToolPaths>,
    settings_json: Option<String>,
    stop_on_error: Option<bool>,
    confirm_test_output: Option<bool>,
    confirm_full_rebuild: Option<bool>,
    confirm_youtube_upload: Option<bool>,
    filename_include_patterns: Option<Vec<String>>,
    filename_exclude_patterns: Option<Vec<String>>,
    selected_files: Option<Vec<String>>,
    file_tune_overrides: Option<BTreeMap<String, String>>,
    file_stream_overrides: Option<BTreeMap<String, Vec<u16>>>,
    file_poster_overrides: Option<BTreeMap<String, String>>,
    file_metadata_overrides: Option<BTreeMap<String, Value>>,
    file_output_dirs: Option<BTreeMap<String, String>>,
    allow_youtube_hdr10plus_loss_paths: Option<Vec<String>>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct StartQueueRunParams {
    /// A list of single-source run requests. Every request must include its explicit selected_files list.
    runs: Vec<StartRunParams>,
    /// Stop before starting another source after a non-zero engine result. Defaults to true.
    stop_on_error: Option<bool>,
    /// Explicitly acknowledge review of any selected interrupted or uncertain queue outputs.
    acknowledge_review_paths: Option<Vec<String>>,
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
struct AptUpdateParams {}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct AptRefreshParams {
    confirm_refresh: bool,
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

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct MetadataSearchParams {
    query: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct FilmMetadataParams {
    entity_id: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct TvMetadataParams {
    show_id: u64,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OrganizerFolderParams {
    root: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RenamePreviewParams {
    root: String,
    items: Vec<crate::library_organizer::RenameTemplateInput>,
    text_blacklist_patterns: Option<Vec<String>>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RenameApplyParams {
    root: String,
    items: Vec<crate::library_organizer::RenameTemplateInput>,
    text_blacklist_patterns: Option<Vec<String>>,
    preview_id: String,
    confirm_rename: bool,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MetadataWriteApplyParams {
    request: crate::library_nfo::MetadataWriteRequest,
    preview_id: String,
    confirm_write: bool,
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
        description = "Inspect local NFO metadata and recognized poster/artwork sidecars for media paths and folders under one selected library root. This read-only operation does not contact public metadata providers or alter files. NFOs are preferred local sources; use public search tools only when local metadata is absent or a user explicitly wants another candidate. Artwork bytes stay on the device and only paths are returned."
    )]
    fn inspect_local_library_assets(
        &self,
        Parameters(request): Parameters<crate::library_assets::LocalAssetsRequest>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, ErrorData> {
        serialized_result(crate::library_assets::scan_local_assets(request))
    }

    #[tool(
        name = "preview_library_metadata_write",
        description = "Preview exact Kodi-compatible movie/show/episode NFO destinations for reviewed metadata. This reads only file metadata, preserves existing user-owned NFO files, and performs no write. Apply requires the returned previewId and a separate explicit confirmation."
    )]
    fn preview_library_metadata_write(
        &self,
        Parameters(request): Parameters<crate::library_nfo::MetadataWriteRequest>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, ErrorData> {
        serialized_result(crate::library_nfo::preview_metadata_write(request))
    }

    #[tool(
        name = "apply_library_metadata_write",
        description = "Write reviewed Kodi-compatible metadata sidecars after preview. Provide the same request, exact previewId, and confirmWrite=true. Existing standard NFO files are preserved; only HEVC Rebuild managed sidecars can be replaced. A later rebuild embeds supported tags in a new Matroska output; this operation does not alter media bytes."
    )]
    fn apply_library_metadata_write(
        &self,
        Parameters(params): Parameters<MetadataWriteApplyParams>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, ErrorData> {
        let id = job_id();
        let mut job = Job::new(id.clone(), "metadata_write");
        job.status = "running".into();
        self.insert_job(job)
            .map_err(|error| ErrorData::internal_error(error, None))?;
        let result = crate::library_nfo::apply_metadata_write(
            params.request,
            params.preview_id,
            params.confirm_write,
        )
        .and_then(|value| {
            serde_json::to_value(value)
                .map_err(|error| format!("Metadata write result could not be serialized: {error}"))
        });
        if let Ok(mut jobs) = self.jobs.lock() {
            if let Some(job) = jobs.get_mut(&id) {
                match &result {
                    Ok(value) => {
                        job.status = "completed".into();
                        job.result = Some(value.clone());
                    }
                    Err(error) => {
                        job.status = "failed".into();
                        job.error = Some(error.clone());
                    }
                }
            }
        }
        json_result(result)
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
        description = "Analyze one local media file or a folder without encoding or writing to the media input. Optional tool_paths selects explicit ffmpeg, ffprobe, mediainfo, mkvpropedit, mkvmerge, or mkvextract executables. Returns a job ID for progress, paginated per-file errors and stream metadata, current available/total memory context, profile-aware rebuild forecasts, and YouTube size/memory estimates for source, 1440p, and 4K targets. Forecasts are heuristics, not exact peak-RAM or output-size guarantees. Only process content you own or are authorized to use."
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
        let options = crate::media_scan::ScanOptions {
            root: root.to_string_lossy().into_owned(),
            recursive: params.recursive.unwrap_or(true),
            minimum_size_mib: minimum,
            filename_include_patterns: params.filename_include_patterns.unwrap_or_default(),
            filename_exclude_patterns: params.filename_exclude_patterns.unwrap_or_default(),
            tool_paths,
        };
        let id = job_id();
        let mut job = Job::new(id.clone(), "folder_analysis");
        job.status = "running".into();
        self.insert_job(job)
            .map_err(|error| ErrorData::internal_error(error, None))?;
        let jobs = self.jobs.clone();
        let worker_id = id.clone();
        thread::spawn(move || run_scan_job(jobs, worker_id, options));
        Ok(rmcp::handler::server::wrapper::Json(
            json!({"job_id": id, "status": "running", "poll_with": "get_job"}),
        ))
    }

    #[tool(
        name = "get_job",
        description = "Get scan, encode, repair, or helper-install progress. For media runs, resource_snapshot contains the latest Rust-observed system RAM/swap pressure, CPU/load, disk free space, and engine/FFmpeg process memory where supported; active safety guards and pressure retries still run in the Python compatibility engine. For folder analysis, request bounded result pages with offset and limit (maximum 250). Includes recent diagnostic lines, job status, and per-file errors."
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
        description = "Start a check, metadata-only MKV copy, short test, full library rebuild, or YouTube upload-preparation run for one local media file or a folder. Metadata-only mode uses MKVToolNix to copy every source stream into a new MKV, applies reviewed metadata/artwork, verifies copied compressed-stream hashes and HDR signaling, and never invokes a video encoder or replaces the source. It preserves all tracks; file_stream_overrides is therefore not accepted for metadata-only mode. Uses hardware-aware resource limits for encoding modes. While the job runs, get_job includes the latest Rust-observed system RAM/swap pressure, CPU/load, disk free space, and engine/FFmpeg process memory snapshot where the platform exposes them; the Python engine retains active memory/disk enforcement and pressure retries during migration. Optional tool_paths selects explicit ffmpeg, ffprobe, mediainfo, mkvpropedit, mkvmerge, or mkvextract executables. filename_include_patterns and filename_exclude_patterns filter basenames case-insensitively with * and ?; exclusions take precedence. selected_files optionally restricts this run to absolute file paths from the analyzed input; omit it to process every eligible file. When the paths match saved queue entries, the tool marks those entries running and then records matching report outcomes in the persistent queue; paths without report evidence remain needs_review, never completed. For metadata/test/full output, output_dir sets a default existing destination folder and file_output_dirs maps individual absolute media paths to existing destination folders. file_poster_overrides maps absolute media paths to valid JPEG, PNG, or WebP artwork paths inside the analyzed input or to a private app-cache path returned by cache_library_artwork/cache_local_library_artwork; selected artwork can be attached to new MKV outputs when metadata.embed_artwork is true. file_metadata_overrides maps absolute media paths to reviewed title, year, certification, summaries, plot, genres, rating, source URL, show title, season, episode, and air date fields; the selected reviewed values override local NFO values in output metadata. A default folder mirrors the input's relative folder structure; a per-file folder stores that file directly there. Destination files are never overwritten and source files stay in place. Without an output folder, metadata-only output creates a sibling .metadata.mkv when needed; full rebuilds use the validated in-place replacement transaction. For test/full rebuilds, file_stream_overrides maps absolute analyzed media paths to unique FFprobe stream indices; selected real video tracks are encoded to HEVC and other selected tracks are copied. Omit an entry to use the configured default selection. settings_json accepts the normal settings object, including metadata.text_blacklist as one pattern per array entry; patterns are case-insensitive substring globs and '*' matches any sequence (for example, Movie*Group). Full rebuild, test output, and YouTube export each require their matching confirmation field. Per-file tune overrides may set animation or normal for selected paths. YouTube copies use a source-aware 1440p/balanced-size default; youtube_resolution and youtube_quality_mode can be set explicitly. youtube_source_multiplier adjusts balanced bitrate from 0.5x to 4x the source average (default 2x), still capped at YouTube's recommended rate; estimates are not strict file-size limits. HDR10+ sources are skipped by default because this exporter does not retain dynamic metadata; pass allow_youtube_hdr10plus_loss_paths with only the analyzed files whose dynamic metadata loss you explicitly accept. Recommended mode may create much larger files. Originals stay in place. Poll get_job and use request_safe_stop to finish the current file then stop. Only process content you own or are authorized to use."
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
        let args = prepared.args;
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
            run_media_job(jobs, worker_id, args, stop_file, queue_sync)
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
            if !matches!(job.kind.as_str(), "media_run" | "media_queue_run")
                || job.status != "running"
            {
                return Err(ErrorData::invalid_params(
                    "Only an active media run can be safely stopped.",
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
            super::reports::list_reports(&directory, limit as usize),
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
        name = "refresh_ubuntu_package_catalog",
        description = "Refresh the locally configured Ubuntu APT package catalog after the caller explicitly sets confirm_refresh=true. This contacts the configured package repositories and changes only local package-index metadata; it does not install or upgrade packages. Starts an administrator-authenticated job."
    )]
    fn refresh_ubuntu_package_catalog(
        &self,
        Parameters(params): Parameters<AptRefreshParams>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, ErrorData> {
        if !params.confirm_refresh {
            return Err(ErrorData::invalid_params(
                "Set confirm_refresh=true to refresh the package catalog.",
                None,
            ));
        }
        let environment = super::environment::inspect_environment(ToolPaths::default(), None)
            .map_err(|error| ErrorData::internal_error(error, None))?;
        if environment
            .pointer("/os/supported")
            .and_then(Value::as_bool)
            != Some(true)
        {
            return Err(ErrorData::invalid_params(
                "APT catalog refresh is limited to supported Ubuntu and Linux Mint releases.",
                None,
            ));
        }
        if unsafe { geteuid() } != 0 && !executable_in_path("pkexec") {
            return Err(ErrorData::invalid_params(
                "Desktop administrator authentication (pkexec) is not installed.",
                None,
            ));
        }
        let id = job_id();
        let mut job = Job::new(id.clone(), "apt_catalog_refresh");
        job.status = "running".into();
        self.insert_job(job)
            .map_err(|error| ErrorData::internal_error(error, None))?;
        let jobs = self.jobs.clone();
        let worker_id = id.clone();
        thread::spawn(move || {
            run_apt_command_job(jobs, worker_id, vec!["apt-get".into(), "update".into()])
        });
        Ok(rmcp::handler::server::wrapper::Json(
            json!({"job_id":id,"status":"running","poll_with":"get_job"}),
        ))
    }

    #[tool(
        name = "upgrade_ubuntu_packages",
        description = "Install only the available upgrades from the fixed Ubuntu package allowlist. First call check_ubuntu_package_updates, then pass confirmed_package_names exactly equal to its current package list. The server rechecks that list and simulates the APT transaction; it refuses if APT would change any extra package or remove anything. No catalog refresh happens automatically. Starts an administrator-authenticated job."
    )]
    fn upgrade_ubuntu_packages(
        &self,
        Parameters(params): Parameters<RepairParams>,
    ) -> Result<rmcp::handler::server::wrapper::Json<Value>, ErrorData> {
        let status = super::dependencies::apt_update_status()
            .map_err(|error| ErrorData::internal_error(error, None))?;
        let packages = validate_apt_update_confirmation(&status, params.confirmed_package_names)
            .map_err(|error| ErrorData::invalid_params(error, None))?;
        validate_apt_upgrade_plan(&packages)
            .map_err(|error| ErrorData::invalid_params(error, None))?;
        if unsafe { geteuid() } != 0 && !executable_in_path("pkexec") {
            return Err(ErrorData::invalid_params(
                "Desktop administrator authentication (pkexec) is not installed.",
                None,
            ));
        }
        let id = job_id();
        let mut job = Job::new(id.clone(), "package_update");
        job.status = "running".into();
        self.insert_job(job)
            .map_err(|error| ErrorData::internal_error(error, None))?;
        let jobs = self.jobs.clone();
        let worker_id = id.clone();
        thread::spawn(move || run_apt_update_job(jobs, worker_id, packages));
        Ok(rmcp::handler::server::wrapper::Json(
            json!({"job_id":id,"status":"running","poll_with":"get_job"}),
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
        let environment = super::environment::inspect_environment(ToolPaths::default(), None)
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
            .any(|package| !APT_ALLOWLIST.contains(&package.as_str()))
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
    version = "1.3.0",
    instructions = "Local Ubuntu-first media library analysis and HEVC rebuild MCP server. Inspect first; analysis is read-only. Runs default to check mode. Require explicit confirmation flags before short-test or full rebuild outputs. System package repair requires the exact package list from inspect_environment. Only process media the user owns or is authorized to use."
)]
impl ServerHandler for McpServer {}

struct PreparedMediaRun {
    root: PathBuf,
    mode: String,
    selected_files: Vec<String>,
    args: Vec<String>,
    output_dir: Option<String>,
    file_output_dirs: BTreeMap<String, String>,
}

struct QueueRunPlan {
    run: PreparedMediaRun,
    queue_sync: Option<QueueRunSync>,
}

fn validate_prepared_queue_runs(runs: &[PreparedMediaRun]) -> Result<(), ErrorData> {
    if runs.is_empty() || runs.len() > 500 {
        return Err(ErrorData::invalid_params(
            "A queued run must contain between 1 and 500 source runs.",
            None,
        ));
    }
    let mode = &runs[0].mode;
    let mut roots = std::collections::BTreeSet::new();
    let mut files = std::collections::BTreeSet::new();
    let mut total_files = 0usize;
    for run in runs {
        if &run.mode != mode {
            return Err(ErrorData::invalid_params(
                "All source runs in one queue batch must use the same mode.",
                None,
            ));
        }
        if !roots.insert(run.root.clone()) {
            return Err(ErrorData::invalid_params(
                "A source root can appear only once in a queued run.",
                None,
            ));
        }
        if run.selected_files.is_empty() {
            return Err(ErrorData::invalid_params(
                "Every queued source run requires an explicit non-empty selected_files list.",
                None,
            ));
        }
        total_files = total_files.saturating_add(run.selected_files.len());
        if total_files > 25_000 {
            return Err(ErrorData::invalid_params(
                "A queued run is limited to 25,000 explicitly selected files.",
                None,
            ));
        }
        for raw_path in &run.selected_files {
            let path = PathBuf::from(raw_path).canonicalize().map_err(|error| {
                ErrorData::invalid_params(
                    format!("A selected media file is no longer available: {error}"),
                    None,
                )
            })?;
            if !files.insert(path) {
                return Err(ErrorData::invalid_params(
                    "Selected media paths must be unique across the complete queue batch.",
                    None,
                ));
            }
        }
    }

    if ["metadata", "test", "full"].contains(&mode.as_str()) {
        let default_output = runs[0]
            .output_dir
            .as_deref()
            .filter(|path| !path.trim().is_empty());
        let canonical_default = default_output
            .map(Path::new)
            .map(Path::canonicalize)
            .transpose()
            .map_err(|error| {
                ErrorData::invalid_params(
                    format!("The default output folder could not be resolved: {error}"),
                    None,
                )
            })?;
        let mut per_file_output_dirs = HashMap::new();
        let mut preview_sources = Vec::with_capacity(runs.len());
        for run in runs {
            let output = run
                .output_dir
                .as_deref()
                .filter(|path| !path.trim().is_empty())
                .map(Path::new)
                .map(Path::canonicalize)
                .transpose()
                .map_err(|error| {
                    ErrorData::invalid_params(
                        format!("A source output folder could not be resolved: {error}"),
                        None,
                    )
                })?;
            if output != canonical_default {
                return Err(ErrorData::invalid_params(
                    "Use one shared default output folder for a multi-source batch; per-file output folders may still differ.",
                    None,
                ));
            }
            for (path, folder) in &run.file_output_dirs {
                if per_file_output_dirs
                    .insert(path.clone(), folder.clone())
                    .is_some()
                {
                    return Err(ErrorData::invalid_params(
                        "A selected file has conflicting per-file output folders across source runs.",
                        None,
                    ));
                }
            }
            preview_sources.push(crate::output_targets::OutputPreviewSource {
                root: run.root.to_string_lossy().into_owned(),
                files: run.selected_files.clone(),
            });
        }
        let preview = crate::output_targets::preview(crate::output_targets::OutputPreviewRequest {
            mode: mode.clone(),
            sources: preview_sources,
            output_dir: default_output.map(str::to_owned),
            file_output_dirs: per_file_output_dirs,
        })
        .map_err(|error| ErrorData::invalid_params(error, None))?;
        if !preview.eligible {
            return Err(ErrorData::invalid_params(
                json!({
                    "message": "Output preflight found occupied destinations or duplicate target names across the selected sources.",
                    "collision_count": preview.collision_count,
                    "collisions": preview.collisions,
                })
                .to_string(),
                None,
            ));
        }
    }
    Ok(())
}

fn prepare_media_run(
    params: StartRunParams,
    stop_file: &std::path::Path,
) -> Result<PreparedMediaRun, ErrorData> {
    let preflight_output_dir = params.output_dir.clone();
    let preflight_file_output_dirs = params.file_output_dirs.clone().unwrap_or_default();
    let root = canonical_media_input(&params.root)
        .map_err(|error| ErrorData::invalid_params(error, None))?;
    let mode = params.mode.as_deref().unwrap_or("check");
    if !["check", "metadata", "test", "full", "youtube"].contains(&mode) {
        return Err(ErrorData::invalid_params(
            "Mode must be check, metadata, test, full, or youtube.",
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
    if mode == "youtube" && params.confirm_youtube_upload != Some(true) {
        return Err(ErrorData::invalid_params(
            "Set confirm_youtube_upload=true to authorize creating separate YouTube upload copies.",
            None,
        ));
    }
    let youtube_resolution = params.youtube_resolution.as_deref().unwrap_or("1440p");
    if !["source", "1440p", "4k"].contains(&youtube_resolution) {
        return Err(ErrorData::invalid_params(
            "youtube_resolution must be source, 1440p, or 4k.",
            None,
        ));
    }
    let youtube_quality_mode = params.youtube_quality_mode.as_deref().unwrap_or("balanced");
    if !["balanced", "recommended"].contains(&youtube_quality_mode) {
        return Err(ErrorData::invalid_params(
            "youtube_quality_mode must be balanced or recommended.",
            None,
        ));
    }
    let youtube_source_multiplier = params.youtube_source_multiplier.unwrap_or(2.0);
    if !youtube_source_multiplier.is_finite() || !(0.5..=4.0).contains(&youtube_source_multiplier) {
        return Err(ErrorData::invalid_params(
            "youtube_source_multiplier must be between 0.5 and 4.0.",
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
    let settings = if let Some(serialized) = params.settings_json {
        serde_json::from_str::<Value>(&serialized).map_err(|error| {
            ErrorData::invalid_params(format!("settings_json is invalid: {error}"), None)
        })?
    } else {
        let effective = crate::settings::load_effective_settings(&root.to_string_lossy())
            .map_err(|error| ErrorData::internal_error(error, None))?;
        serde_json::to_value(effective)
            .map_err(|error| ErrorData::internal_error(error.to_string(), None))?
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
    crate::append_filename_filter_arguments(
        &mut args,
        &params.filename_include_patterns.unwrap_or_default(),
        &params.filename_exclude_patterns.unwrap_or_default(),
    )
    .map_err(|error| ErrorData::invalid_params(error, None))?;
    let selected_files = params.selected_files.unwrap_or_default();
    crate::append_selected_file_arguments(&mut args, &root, &selected_files)
        .map_err(|error| ErrorData::invalid_params(error, None))?;
    let file_output_dirs = params.file_output_dirs.unwrap_or_default();
    let has_rebuild_output = params
        .output_dir
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
        || !file_output_dirs.is_empty();
    if has_rebuild_output && !["metadata", "test", "full"].contains(&mode) {
        return Err(ErrorData::invalid_params(
            "output_dir and file_output_dirs apply only to metadata-only output, short tests, and full rebuilds.",
            None,
        ));
    }
    if ["metadata", "test", "full"].contains(&mode) {
        let (output_arguments, _) =
            crate::rebuild_output_arguments(&root, params.output_dir.as_deref(), file_output_dirs)
                .map_err(|error| ErrorData::invalid_params(error, None))?;
        args.extend(output_arguments);
    }
    args.extend(
        tool_path_arguments(params.tool_paths, params.ffmpeg_path)
            .map_err(|error| ErrorData::invalid_params(error, None))?,
    );
    let stream_overrides = params.file_stream_overrides.unwrap_or_default();
    if (mode == "youtube" || mode == "metadata") && !stream_overrides.is_empty() {
        return Err(ErrorData::invalid_params(
            "Per-file stream selections do not apply to YouTube or metadata-only output.",
            None,
        ));
    }
    args.extend(
        stream_override_arguments(&root, stream_overrides)
            .map_err(|error| ErrorData::invalid_params(error, None))?,
    );
    let poster_overrides = params.file_poster_overrides.unwrap_or_default();
    if !poster_overrides.is_empty() && !["metadata", "test", "full"].contains(&mode) {
        return Err(ErrorData::invalid_params(
            "Selected poster images can be attached only to metadata-only, short-test, and full rebuild outputs.",
            None,
        ));
    }
    args.extend(
        crate::poster_override_arguments(
            &root,
            poster_overrides,
            &crate::library_artwork::app_artwork_cache_root()
                .map_err(|error| ErrorData::internal_error(error, None))?,
        )
        .map_err(|error| ErrorData::invalid_params(error, None))?,
    );
    let metadata_overrides = params.file_metadata_overrides.unwrap_or_default();
    if !metadata_overrides.is_empty() && !["metadata", "test", "full"].contains(&mode) {
        return Err(ErrorData::invalid_params(
            "Reviewed metadata applies only to metadata-only, short-test, and full rebuild outputs.",
            None,
        ));
    }
    args.extend(
        crate::metadata_override_arguments(&root, metadata_overrides)
            .map_err(|error| ErrorData::invalid_params(error, None))?,
    );
    let hdr10plus_loss_paths = params
        .allow_youtube_hdr10plus_loss_paths
        .unwrap_or_default();
    if mode != "youtube" && !hdr10plus_loss_paths.is_empty() {
        return Err(ErrorData::invalid_params(
            "HDR10+ metadata-loss consent applies only to YouTube export mode.",
            None,
        ));
    }
    args.extend(
        youtube_hdr10plus_loss_arguments(&root, hdr10plus_loss_paths)
            .map_err(|error| ErrorData::invalid_params(error, None))?,
    );
    if mode == "youtube" {
        args.push("--youtube-upload".into());
        args.extend(["--upload-resolution".into(), youtube_resolution.into()]);
        args.extend(["--upload-quality-mode".into(), youtube_quality_mode.into()]);
        args.extend([
            "--upload-source-multiplier".into(),
            youtube_source_multiplier.to_string(),
        ]);
        if let Some(output_dir) = params
            .upload_output_dir
            .filter(|value| !value.trim().is_empty())
        {
            let output_dir = PathBuf::from(output_dir);
            if !output_dir.is_absolute() {
                return Err(ErrorData::invalid_params(
                    "upload_output_dir must be an absolute folder path.",
                    None,
                ));
            }
            args.extend([
                "--upload-output-dir".into(),
                output_dir.to_string_lossy().into_owned(),
            ]);
        }
    }
    let per_file = params.file_tune_overrides.unwrap_or_default();
    if mode == "metadata" && !per_file.is_empty() {
        return Err(ErrorData::invalid_params(
            "Per-file encoding tunes do not apply to metadata-only MKV output.",
            None,
        ));
    }
    if mode == "youtube" && !per_file.is_empty() {
        return Err(ErrorData::invalid_params(
            "Per-file x265 content tunes do not apply to YouTube export mode.",
            None,
        ));
    }
    if per_file.len() > 512 {
        return Err(ErrorData::invalid_params(
            "Per-file tune overrides are limited to 512 media files.",
            None,
        ));
    }
    let mut validated_per_file = BTreeMap::new();
    for (raw_path, tune) in per_file {
        if !["animation", "none"].contains(&tune.as_str()) {
            return Err(ErrorData::invalid_params(
                "Per-file tune overrides must be 'animation' or 'none'.",
                None,
            ));
        }
        let selected = PathBuf::from(raw_path).canonicalize().map_err(|error| {
            ErrorData::invalid_params(
                format!("Per-file tune source is unavailable: {error}"),
                None,
            )
        })?;
        if !selected.is_file()
            || (root.is_dir() && !selected.starts_with(&root))
            || (root.is_file() && selected != root)
        {
            return Err(ErrorData::invalid_params(
                "Per-file tune overrides must refer to media inside the selected input.",
                None,
            ));
        }
        validated_per_file.insert(selected.to_string_lossy().into_owned(), tune);
    }
    if !validated_per_file.is_empty() {
        let serialized = serde_json::to_string(&validated_per_file).map_err(|error| {
            ErrorData::invalid_params(
                format!("Could not encode per-file tune choices: {error}"),
                None,
            )
        })?;
        if serialized.len() > 65536 {
            return Err(ErrorData::invalid_params(
                "Per-file tune overrides exceed the 64 KiB limit.",
                None,
            ));
        }
        args.extend(["--file-tune-overrides-json".into(), serialized]);
    }
    args.extend(
        settings_arguments(&settings).map_err(|error| ErrorData::invalid_params(error, None))?,
    );
    if params.stop_on_error.unwrap_or(false) {
        args.push("--stop-on-error".into());
    }
    match mode {
        "check" => args.push("--info-only".into()),
        "metadata" => args.push("--metadata-only".into()),
        "test" => args.push("--test".into()),
        _ => {}
    }
    args.extend([
        "--stop-request-file".into(),
        stop_file.to_string_lossy().into_owned(),
    ]);
    Ok(PreparedMediaRun {
        root,
        mode: mode.to_owned(),
        selected_files,
        args,
        output_dir: preflight_output_dir,
        file_output_dirs: preflight_file_output_dirs,
    })
}

fn run_scan_job(jobs: Jobs, id: String, options: crate::media_scan::ScanOptions) {
    let result = crate::media_scan::scan_media(options, |current, total, file| {
        if let Ok(mut jobs) = jobs.lock() {
            if let Some(job) = jobs.get_mut(&id) {
                job.current = current;
                job.total = total;
                job.last_file = Some(file.to_string());
            }
        }
    });
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

#[derive(Clone)]
struct QueueRunSync {
    queue_path: PathBuf,
    media_paths: Vec<String>,
    mode: String,
}

fn queue_run_sync(
    root: &std::path::Path,
    selected_files: &[String],
    mode: &str,
) -> Result<Option<QueueRunSync>, String> {
    let queue_path = crate::media_queue_file()?;
    queue_run_sync_at(&queue_path, root, selected_files, mode)
}

fn queue_run_sync_at(
    queue_path: &std::path::Path,
    root: &std::path::Path,
    selected_files: &[String],
    mode: &str,
) -> Result<Option<QueueRunSync>, String> {
    let queue = crate::media_queue::read_snapshot(queue_path)?;
    let selected = selected_files
        .iter()
        .filter_map(|path| PathBuf::from(path).canonicalize().ok())
        .collect::<std::collections::BTreeSet<_>>();
    let input_has_explicit_selection = !selected_files.is_empty();
    let mut tracked = std::collections::BTreeMap::<String, ()>::new();

    for source in queue.sources {
        let source_matches = PathBuf::from(&source.root)
            .canonicalize()
            .is_ok_and(|path| path == root);
        if !input_has_explicit_selection && !source_matches {
            continue;
        }
        let Some(entries) = source
            .scan_result
            .as_ref()
            .and_then(|result| result.get("entries"))
            .and_then(Value::as_array)
        else {
            continue;
        };
        for entry in entries {
            let Some(path) = entry.get("path").and_then(Value::as_str) else {
                continue;
            };
            let resolved = PathBuf::from(path).canonicalize().ok();
            let Some(resolved) = resolved else {
                continue;
            };
            if input_has_explicit_selection && !selected.contains(&resolved) {
                continue;
            }
            if !input_has_explicit_selection && !source_matches {
                continue;
            }
            tracked.insert(path.to_owned(), ());
        }
    }

    let media_paths = tracked.into_keys().collect::<Vec<_>>();
    if media_paths.is_empty() {
        return Ok(None);
    }
    Ok(Some(QueueRunSync {
        queue_path: queue_path.to_path_buf(),
        media_paths,
        mode: mode.to_owned(),
    }))
}

fn validate_queue_recovery(
    runs: &[PreparedMediaRun],
    acknowledged_paths: &[String],
) -> Result<(), ErrorData> {
    if acknowledged_paths.len() > 25_000 {
        return Err(ErrorData::invalid_params(
            "Review acknowledgments are limited to 25,000 media paths.",
            None,
        ));
    }
    let selected = runs
        .iter()
        .flat_map(|run| &run.selected_files)
        .map(|raw_path| PathBuf::from(raw_path).canonicalize())
        .collect::<Result<std::collections::BTreeSet<_>, _>>()
        .map_err(|error| {
            ErrorData::invalid_params(
                format!("A selected media file is no longer available: {error}"),
                None,
            )
        })?;
    let acknowledged = acknowledged_paths
        .iter()
        .map(|raw_path| PathBuf::from(raw_path).canonicalize())
        .collect::<Result<std::collections::BTreeSet<_>, _>>()
        .map_err(|error| {
            ErrorData::invalid_params(
                format!("A review acknowledgment path is unavailable: {error}"),
                None,
            )
        })?;
    if acknowledged.iter().any(|path| !selected.contains(path)) {
        return Err(ErrorData::invalid_params(
            "Every acknowledged review path must also be selected for this batch.",
            None,
        ));
    }
    let queue_path =
        crate::media_queue_file().map_err(|error| ErrorData::internal_error(error, None))?;
    let queue = crate::media_queue::read_snapshot(&queue_path)
        .map_err(|error| ErrorData::internal_error(error, None))?;
    let mut review_required = std::collections::BTreeSet::new();
    for (path, state) in queue.run_states {
        if !matches!(state.status.as_str(), "interrupted" | "needs_review") {
            continue;
        }
        if let Ok(canonical) = PathBuf::from(path).canonicalize() {
            if selected.contains(&canonical) && !acknowledged.contains(&canonical) {
                review_required.insert(canonical);
            }
        }
    }
    if !review_required.is_empty() {
        return Err(ErrorData::invalid_params(
            json!({
                "message": "Review the saved report/output for every interrupted or uncertain selected file, then include those paths in acknowledge_review_paths.",
                "review_required_count": review_required.len(),
                "review_required_paths": review_required.iter().take(50).collect::<Vec<_>>(),
            })
            .to_string(),
            None,
        ));
    }
    Ok(())
}

pub(crate) fn queue_run_json_cli(args: &[String]) -> i32 {
    if args.len() != 2 || args[0] != "--request" {
        eprintln!("Usage: hevc-rebuild-studio json queue-run --request QUEUE_RUN.json");
        return 2;
    }
    let request_path = PathBuf::from(&args[1]);
    let metadata = match request_path.metadata() {
        Ok(metadata) => metadata,
        Err(_) => {
            eprintln!("Queue run request file is unavailable.");
            return 2;
        }
    };
    if !metadata.is_file() || metadata.len() > 16 * 1024 * 1024 {
        eprintln!("Queue run request must be a regular file no larger than 16 MiB.");
        return 2;
    }
    let request_bytes = match fs::read(&request_path) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("Could not read queue run request: {error}");
            return 2;
        }
    };
    let request = match serde_json::from_slice::<StartQueueRunParams>(&request_bytes) {
        Ok(request) => request,
        Err(error) => {
            eprintln!("Invalid queue run JSON: {error}");
            return 2;
        }
    };
    if request.runs.is_empty() || request.runs.len() > 500 {
        eprintln!("A queued run must contain between 1 and 500 source runs.");
        return 2;
    }
    let stop_on_error = request.stop_on_error.unwrap_or(true);
    let acknowledge_review_paths = request.acknowledge_review_paths.unwrap_or_default();
    let id = job_id();
    let stop_token = match crate::stop_control::StopToken::new() {
        Ok(token) => token,
        Err(error) => {
            eprintln!("{error}");
            return 1;
        }
    };
    let stop_file = stop_token.path().to_path_buf();
    let mut prepared = Vec::with_capacity(request.runs.len());
    for mut run in request.runs {
        if run.selected_files.as_ref().is_none_or(Vec::is_empty) {
            eprintln!(
                "Every queued source run requires an explicit non-empty selected_files list."
            );
            return 2;
        }
        run.stop_on_error = Some(stop_on_error);
        match prepare_media_run(run, &stop_file) {
            Ok(plan) => prepared.push(plan),
            Err(error) => {
                eprintln!("Could not prepare queued media run: {error}");
                return 2;
            }
        }
    }
    if let Err(error) = validate_prepared_queue_runs(&prepared) {
        eprintln!("Queued run validation failed: {error}");
        return 2;
    }
    if let Err(error) = validate_queue_recovery(&prepared, &acknowledge_review_paths) {
        eprintln!("Queued run recovery check failed: {error}");
        return 2;
    }

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
    let mut queue_items_tracked = 0usize;
    if queue_sync_warning.is_none() {
        let mut queued_states = BTreeMap::new();
        for sync in queue_syncs.iter().flatten() {
            queue_items_tracked = queue_items_tracked.saturating_add(sync.media_paths.len());
            queued_states.extend(queue_run_states(
                &sync.media_paths,
                &sync.mode,
                "queued",
                Some("Waiting for the preceding source batch."),
            ));
        }
        if let Some(queue_path) = queue_syncs
            .iter()
            .flatten()
            .next()
            .map(|sync| &sync.queue_path)
        {
            if let Err(error) = crate::media_queue::save_run_states(queue_path, queued_states) {
                queue_sync_warning = Some(format!(
                    "The media queue will run, but queued states could not be saved: {error}"
                ));
                queue_syncs = vec![None; prepared.len()];
                queue_items_tracked = 0;
            }
        }
    }
    let plans = prepared
        .into_iter()
        .zip(queue_syncs)
        .map(|(run, queue_sync)| QueueRunPlan { run, queue_sync })
        .collect::<Vec<_>>();
    let source_count = plans.len();
    let file_count = plans
        .iter()
        .map(|plan| plan.run.selected_files.len())
        .sum::<usize>();
    let jobs: Jobs = Arc::new(Mutex::new(HashMap::new()));
    let mut job = Job::new(id.clone(), "media_queue_run");
    job.status = "running".into();
    job.total = file_count;
    job.stop_file = Some(stop_file.clone());
    job.queue_sync_warning = queue_sync_warning;
    if let Ok(mut registry) = jobs.lock() {
        registry.insert(id.clone(), job);
    }
    run_media_queue_job(
        jobs.clone(),
        id.clone(),
        plans,
        stop_file,
        stop_on_error,
        true,
    );
    let Some(job) = jobs
        .lock()
        .ok()
        .and_then(|registry| registry.get(&id).cloned())
    else {
        eprintln!("Could not read the completed queue-run status.");
        return 1;
    };
    let exit_code = job.exit_code.unwrap_or(1);
    let payload = json!({
        "job_id": job.id,
        "status": job.status,
        "source_count": source_count,
        "file_count": file_count,
        "completed_files": job.current,
        "exit_code": exit_code,
        "last_source": job.last_file,
        "report_path": job.report_path,
        "log_path": job.log_path,
        "error": job.error,
        "queue_items_tracked": queue_items_tracked,
        "queue_sync_warning": job.queue_sync_warning,
    });
    match serde_json::to_string(&payload) {
        Ok(value) => println!("{value}"),
        Err(error) => {
            eprintln!("Could not encode queued run result: {error}");
            return 1;
        }
    }
    match job.status.as_str() {
        "completed" => 0,
        "stopped" => 130,
        _ => exit_code.max(1),
    }
}

fn queue_run_states(
    paths: &[String],
    mode: &str,
    status: &str,
    message: Option<&str>,
) -> BTreeMap<String, crate::media_queue::QueueRunState> {
    let updated_at_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64;
    paths
        .iter()
        .map(|path| {
            (
                path.clone(),
                crate::media_queue::QueueRunState {
                    status: status.into(),
                    mode: Some(mode.into()),
                    message: message.map(str::to_owned),
                    output_path: None,
                    report_path: None,
                    report_status: None,
                    updated_at_ms,
                },
            )
        })
        .collect()
}

fn queue_state_from_report_row(
    mode: &str,
    report_path: Option<&str>,
    row: Option<&Value>,
    exit_code: Option<i32>,
    stop_requested: bool,
) -> crate::media_queue::QueueRunState {
    let report_status = row
        .and_then(|value| value.get("status"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_uppercase);
    let status = match report_status.as_deref() {
        Some("OK" | "METADATA_OK" | "TEST_OK" | "INFO") => "completed",
        Some("FAILED") => "failed",
        Some("SKIPPED") => "skipped",
        Some("INTERRUPTED") => "interrupted",
        _ => "needs_review",
    };
    let output_path = row
        .and_then(|value| value.get("output"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let message = row
        .and_then(|value| value.get("message"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .or_else(|| match (exit_code, stop_requested) {
            (_, true) => {
                Some("No matching report row was found; review the output before retrying.".into())
            }
            (Some(0), false) => {
                Some("No matching report row was found; completion was not assumed.".into())
            }
            _ => {
                Some("The run ended without a matching report row; review before retrying.".into())
            }
        });
    let updated_at_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64;
    crate::media_queue::QueueRunState {
        status: status.into(),
        mode: Some(mode.into()),
        message,
        output_path,
        report_path: report_path.map(str::to_owned),
        report_status,
        updated_at_ms,
    }
}

fn sync_queue_run_result(
    sync: QueueRunSync,
    report_path: Option<&str>,
    exit_code: Option<i32>,
    stop_requested: bool,
) -> Result<(), String> {
    let directory = crate::state_log_directory()?;
    sync_queue_run_result_in(&sync, &directory, report_path, exit_code, stop_requested)
}

fn sync_queue_run_result_in(
    sync: &QueueRunSync,
    report_directory: &std::path::Path,
    report_path: Option<&str>,
    exit_code: Option<i32>,
    stop_requested: bool,
) -> Result<(), String> {
    let report = report_path
        .map(|path| crate::reports::read_report(report_directory, &PathBuf::from(path)))
        .transpose()?;
    let rows = report
        .as_ref()
        .and_then(|value| value.get("rows"))
        .and_then(Value::as_array);
    let updated = sync
        .media_paths
        .iter()
        .map(|path| {
            let row = rows.and_then(|rows| {
                rows.iter().find(|row| {
                    row.get("input")
                        .and_then(Value::as_str)
                        .is_some_and(|input| input == path)
                })
            });
            (
                path.clone(),
                queue_state_from_report_row(
                    &sync.mode,
                    report_path,
                    row,
                    exit_code,
                    stop_requested,
                ),
            )
        })
        .collect();
    crate::media_queue::save_run_states(&sync.queue_path, updated)
}

fn execute_media_args(
    jobs: &Jobs,
    id: &str,
    args: &[String],
    echo_to_stderr: bool,
) -> Result<i32, String> {
    let mut command = standalone_command("hevc_rebuild", args)?;
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = crate::process_output::spawn_owned(&mut command)
        .map_err(|error| format!("Could not start the media engine: {error}"))?;
    let selected_disk_path = args
        .windows(2)
        .find(|pair| pair[0] == "--upload-output-dir")
        .map(|pair| PathBuf::from(&pair[1]))
        .or_else(|| args.first().map(PathBuf::from));
    let disk_path = selected_disk_path.map(|path| {
        if path.is_file() {
            path.parent().map(PathBuf::from).unwrap_or(path)
        } else {
            path
        }
    });
    let resource_jobs = jobs.clone();
    let resource_id = id.to_owned();
    let resource_monitor = super::resource_monitor::monitor_process(
        child.id(),
        disk_path,
        Duration::from_secs(1),
        move |snapshot| {
            if let Ok(mut registry) = resource_jobs.lock() {
                if let Some(job) = registry.get_mut(&resource_id) {
                    job.resource_snapshot = Some(snapshot);
                }
            }
        },
    );
    let process_id = child.id();
    let Some(stderr) = child.stderr.take() else {
        resource_monitor.stop();
        return Err("Could not read engine diagnostics.".into());
    };
    let stderr_jobs = jobs.clone();
    let stderr_id = id.to_owned();
    let stderr_thread = thread::spawn(move || {
        crate::process_output::read_process_lines(stderr, process_id, |line| {
            if echo_to_stderr {
                eprintln!("{line}");
            }
            record_line(&stderr_jobs, &stderr_id, line);
        })
    });
    let mut output_result = Ok(());
    if let Some(stdout) = child.stdout.take() {
        output_result = crate::process_output::read_process_lines(stdout, process_id, |line| {
            if echo_to_stderr {
                eprintln!("{line}");
            }
            record_line(jobs, id, line);
        });
    }
    let diagnostic_result = stderr_thread
        .join()
        .unwrap_or_else(|_| Err("Engine diagnostic reader panicked.".into()));
    let status = child.wait();
    resource_monitor.stop();
    output_result?;
    diagnostic_result?;
    status
        .map(|status| status.code().unwrap_or(1))
        .map_err(|error| format!("Could not wait for the media engine: {error}"))
}

fn run_media_job(
    jobs: Jobs,
    id: String,
    args: Vec<String>,
    stop_file: PathBuf,
    queue_sync: Option<QueueRunSync>,
) {
    let outcome = execute_media_args(&jobs, &id, &args, false);
    let stop_requested = crate::stop_control::requested(&stop_file);
    let _ = fs::remove_file(&stop_file);
    let exit_code = outcome.as_ref().ok().copied();
    let report_path = jobs
        .lock()
        .ok()
        .and_then(|registry| registry.get(&id).and_then(|job| job.report_path.clone()));
    let mut queue_sync_warning = None;
    if let Some(sync) = queue_sync {
        if let Err(error) =
            sync_queue_run_result(sync, report_path.as_deref(), exit_code, stop_requested)
        {
            queue_sync_warning = Some(format!(
                "The media run ended, but its per-file queue results could not be saved: {error}"
            ));
        }
    }
    if let Ok(mut jobs) = jobs.lock() {
        if let Some(job) = jobs.get_mut(&id) {
            job.stop_file = None;
            if queue_sync_warning.is_some() {
                job.queue_sync_warning = queue_sync_warning;
            }
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
                        job.error = Some(format!(
                            "Media engine exited with code {code}; inspect recent diagnostic lines."
                        ));
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

fn run_media_queue_job(
    jobs: Jobs,
    id: String,
    plans: Vec<QueueRunPlan>,
    stop_file: PathBuf,
    stop_on_error: bool,
    echo_to_stderr: bool,
) {
    let total_files = plans
        .iter()
        .map(|plan| plan.run.selected_files.len())
        .sum::<usize>();
    let source_count = plans.len();
    let mut completed_files = 0usize;
    let mut failure_code = None;
    let mut first_error = None;
    let mut stop_requested = false;
    let mut queue_sync_warning = None;

    for (index, plan) in plans.into_iter().enumerate() {
        if crate::stop_control::requested(&stop_file) {
            stop_requested = true;
            break;
        }
        let source_label = plan.run.root.to_string_lossy().into_owned();
        if echo_to_stderr {
            eprintln!(
                "Queue source {}/{}: {}",
                index + 1,
                source_count,
                source_label
            );
        }
        if let Ok(mut registry) = jobs.lock() {
            if let Some(job) = registry.get_mut(&id) {
                job.current = completed_files;
                job.total = total_files;
                job.last_file = Some(source_label.clone());
                job.report_path = None;
            }
        }
        if let Some(sync) = plan.queue_sync.as_ref() {
            let states = queue_run_states(
                &sync.media_paths,
                &sync.mode,
                "running",
                Some("This source is being processed in sequence."),
            );
            if let Err(error) = crate::media_queue::save_run_states(&sync.queue_path, states) {
                queue_sync_warning.get_or_insert_with(|| {
                    format!("Could not mark a source batch active in the saved queue: {error}")
                });
            }
        }

        let outcome = execute_media_args(&jobs, &id, &plan.run.args, echo_to_stderr);
        let exit_code = outcome.as_ref().ok().copied();
        let report_path = jobs
            .lock()
            .ok()
            .and_then(|registry| registry.get(&id).and_then(|job| job.report_path.clone()));
        let source_stop_requested = crate::stop_control::requested(&stop_file);
        if let Some(sync) = plan.queue_sync {
            if let Err(error) = sync_queue_run_result(
                sync,
                report_path.as_deref(),
                exit_code,
                source_stop_requested,
            ) {
                queue_sync_warning.get_or_insert_with(|| {
                    format!("A source finished, but its queue result could not be saved: {error}")
                });
            }
        }

        match outcome {
            Ok(0) => {
                completed_files = completed_files.saturating_add(plan.run.selected_files.len())
            }
            Ok(code) => {
                failure_code.get_or_insert(code);
                first_error.get_or_insert_with(|| {
                    format!("Media engine exited with code {code} for {source_label}.")
                });
            }
            Err(error) => {
                failure_code.get_or_insert(1);
                first_error.get_or_insert_with(|| format!("{source_label}: {error}"));
            }
        }
        stop_requested = source_stop_requested;
        if let Ok(mut registry) = jobs.lock() {
            if let Some(job) = registry.get_mut(&id) {
                job.current = completed_files;
                job.total = total_files;
            }
        }
        if stop_requested || (failure_code.is_some() && stop_on_error) {
            break;
        }
    }

    let stop_requested = stop_requested || crate::stop_control::requested(&stop_file);
    let _ = fs::remove_file(&stop_file);
    if let Ok(mut registry) = jobs.lock() {
        if let Some(job) = registry.get_mut(&id) {
            job.stop_file = None;
            job.current = completed_files;
            job.total = total_files;
            if queue_sync_warning.is_some() {
                job.queue_sync_warning = queue_sync_warning;
            }
            if let Some(code) = failure_code {
                job.status = "failed".into();
                job.exit_code = Some(code);
                job.error = first_error;
            } else if stop_requested {
                job.status = "stopped".into();
                job.exit_code = Some(130);
            } else {
                job.status = "completed".into();
                job.exit_code = Some(0);
            }
        }
    }
}

#[cfg(test)]
mod queue_sync_tests {
    use super::*;
    use crate::media_queue::{QueueDocument, QueueSource, QueueSourceStatus};

    fn temporary_root(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "hevc-mcp-queue-{label}-{}-{nonce}",
            std::process::id()
        ))
    }

    fn prepared_run(root: &Path, media: &Path, mode: &str) -> PreparedMediaRun {
        PreparedMediaRun {
            root: root.canonicalize().unwrap(),
            mode: mode.to_owned(),
            selected_files: vec![media.canonicalize().unwrap().to_string_lossy().into_owned()],
            args: Vec::new(),
            output_dir: None,
            file_output_dirs: BTreeMap::new(),
        }
    }

    #[test]
    fn queue_batch_rejects_duplicate_canonical_files_across_overlapping_roots() {
        let root = temporary_root("overlap");
        let nested = root.join("Nested");
        fs::create_dir_all(&nested).unwrap();
        let media = nested.join("Sample.mkv");
        fs::write(&media, b"synthetic media placeholder").unwrap();
        let runs = vec![
            prepared_run(&root, &media, "check"),
            prepared_run(&nested, &media, "check"),
        ];
        assert!(validate_prepared_queue_runs(&runs).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn queue_batch_preflights_duplicate_output_names_before_start() {
        let root = temporary_root("output-collision");
        let first = root.join("first");
        let second = root.join("second");
        let output = root.join("output");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        fs::create_dir_all(&output).unwrap();
        let first_media = first.join("SameName.mkv");
        let second_media = second.join("SameName.mkv");
        fs::write(&first_media, b"first synthetic media").unwrap();
        fs::write(&second_media, b"second synthetic media").unwrap();
        let mut runs = vec![
            prepared_run(&first, &first_media, "full"),
            prepared_run(&second, &second_media, "full"),
        ];
        for run in &mut runs {
            run.output_dir = Some(output.to_string_lossy().into_owned());
        }
        assert!(validate_prepared_queue_runs(&runs).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn queue_batch_accepts_distinct_sources_and_shared_output_plan() {
        let root = temporary_root("valid-batch");
        let first = root.join("first");
        let second = root.join("second");
        let output = root.join("output");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        fs::create_dir_all(&output).unwrap();
        let first_media = first.join("First.mkv");
        let second_media = second.join("Second.mkv");
        fs::write(&first_media, b"first synthetic media").unwrap();
        fs::write(&second_media, b"second synthetic media").unwrap();
        let mut runs = vec![
            prepared_run(&first, &first_media, "metadata"),
            prepared_run(&second, &second_media, "metadata"),
        ];
        for run in &mut runs {
            run.output_dir = Some(output.to_string_lossy().into_owned());
        }
        assert!(validate_prepared_queue_runs(&runs).is_ok());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn explicit_queue_run_paths_record_running_state_without_losing_selection() {
        let root = temporary_root("sync");
        fs::create_dir_all(&root).unwrap();
        let media = root.join("Sample.mkv");
        fs::write(&media, b"synthetic media placeholder").unwrap();
        let queue_path = root.join("library-queue.json");
        let media_path = media.to_string_lossy().into_owned();
        let root_path = root.to_string_lossy().into_owned();
        let mut queue = QueueDocument::default();
        queue.sources.push(QueueSource {
            root: root_path.clone(),
            status: QueueSourceStatus::Ready,
            scan_result: Some(json!({"entries": [{"path": media_path, "size_bytes": 28}]})),
            error: None,
            updated_at_ms: 1,
            changed_files: Vec::new(),
            missing_files: Vec::new(),
        });
        queue.selected_media_paths.push(media_path.clone());
        queue.active_root = Some(root_path);
        crate::media_queue::save(&queue_path, queue).unwrap();

        let sync = queue_run_sync_at(
            &queue_path,
            &root,
            std::slice::from_ref(&media_path),
            "check",
        )
        .unwrap()
        .unwrap();
        assert_eq!(sync.media_paths, vec![media_path.clone()]);
        crate::media_queue::save_run_states(
            &sync.queue_path,
            queue_run_states(
                &sync.media_paths,
                "check",
                "running",
                Some("Started through MCP."),
            ),
        )
        .unwrap();

        let report_directory = root.join("logs");
        fs::create_dir_all(&report_directory).unwrap();
        let report_path = report_directory.join("hevc-rebuild-queue-sync.csv");
        fs::write(
            &report_path,
            format!(
                "input,output,status,message\n\"{media_path}\",\"{}\",OK,Validated\n",
                root.join("Sample.metadata.mkv").display()
            ),
        )
        .unwrap();
        sync_queue_run_result_in(
            &sync,
            &report_directory,
            Some(&report_path.to_string_lossy()),
            Some(0),
            false,
        )
        .unwrap();

        let summary = crate::media_queue::summarize(&queue_path, 0, 10).unwrap();
        assert_eq!(summary["selectedCount"], 1);
        assert_eq!(summary["entries"][0]["runStatus"], "completed");
        assert_eq!(summary["entries"][0]["runMode"], "check");
        assert_eq!(summary["entries"][0]["reportStatus"], "OK");

        // A sequential job for the same source must consume its exact report,
        // rather than inheriting the first job's success. Unselected rows in a
        // different report cannot complete this file.
        let next_report = report_directory.join("hevc-rebuild-next-run.csv");
        fs::write(&next_report, format!("input,status,message\n\"{media_path}\",FAILED,Second run failed\nother.mkv,OK,Unrelated\n")).unwrap();
        sync_queue_run_result_in(
            &sync,
            &report_directory,
            Some(&next_report.to_string_lossy()),
            Some(1),
            false,
        )
        .unwrap();
        let summary = crate::media_queue::summarize(&queue_path, 0, 10).unwrap();
        assert_eq!(summary["entries"][0]["runStatus"], "failed");
        assert_eq!(summary["entries"][0]["reportStatus"], "FAILED");
        fs::write(&next_report, "input,status\nother.mkv,OK\n").unwrap();
        sync_queue_run_result_in(
            &sync,
            &report_directory,
            Some(&next_report.to_string_lossy()),
            Some(0),
            false,
        )
        .unwrap();
        let summary = crate::media_queue::summarize(&queue_path, 0, 10).unwrap();
        assert_eq!(summary["entries"][0]["runStatus"], "needs_review");
        assert_eq!(summary["selectedCount"], 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn queue_completion_requires_a_matching_report_status() {
        let successful = json!({
            "status": "METADATA_OK",
            "output": "/tmp/Sample.metadata.mkv",
            "message": "Validated metadata copy",
        });
        let completed = queue_state_from_report_row(
            "metadata",
            Some("/tmp/hevc-rebuild-job.csv"),
            Some(&successful),
            Some(0),
            false,
        );
        assert_eq!(completed.status, "completed");
        assert_eq!(
            completed.output_path.as_deref(),
            Some("/tmp/Sample.metadata.mkv")
        );

        let missing = queue_state_from_report_row("metadata", None, None, Some(0), false);
        assert_eq!(missing.status, "needs_review");
        assert!(missing
            .message
            .unwrap()
            .contains("completion was not assumed"));
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
        let result = crate::process_output::spawn_owned(
            Command::new(program)
                .args(operation.iter().skip(1))
                .stdout(Stdio::piped())
                .stderr(Stdio::piped()),
        );
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
        let process_id = child.id();
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
            crate::process_output::read_process_lines(stderr, process_id, |line| {
                record_line(&jobs_for_err, &err_id, line);
            })
        });
        let mut output_result = Ok(());
        if let Some(stdout) = child.stdout.take() {
            output_result = crate::process_output::read_process_lines(stdout, process_id, |line| {
                record_line(&jobs, &id, line);
            });
        }
        let diagnostic_result = thread
            .join()
            .unwrap_or_else(|_| Err("APT diagnostic reader panicked.".into()));
        let reader_result = output_result.and(diagnostic_result);
        match child.wait() {
            Ok(_) if reader_result.is_err() => {
                update_failed(&jobs, &id, reader_result.unwrap_err());
                return;
            }
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

fn run_apt_command_job(jobs: Jobs, id: String, mut operation: Vec<String>) {
    if unsafe { geteuid() } != 0 {
        operation.insert(0, "pkexec".into());
    }
    let Some(program) = operation.first().cloned() else {
        update_failed(&jobs, &id, "The package update command was empty.".into());
        return;
    };
    let result = crate::process_output::spawn_owned(
        Command::new(program)
            .args(operation.iter().skip(1))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    );
    let mut child = match result {
        Ok(child) => child,
        Err(error) => {
            update_failed(
                &jobs,
                &id,
                format!("Could not start the APT package operation: {error}"),
            );
            return;
        }
    };
    let process_id = child.id();
    let Some(stderr) = child.stderr.take() else {
        update_failed(&jobs, &id, "Could not read APT updater diagnostics.".into());
        return;
    };
    let jobs_for_err = jobs.clone();
    let error_id = id.clone();
    let stderr_thread = thread::spawn(move || {
        crate::process_output::read_process_lines(stderr, process_id, |line| {
            record_line(&jobs_for_err, &error_id, line);
        })
    });
    let mut output_result = Ok(());
    if let Some(stdout) = child.stdout.take() {
        output_result = crate::process_output::read_process_lines(stdout, process_id, |line| {
            record_line(&jobs, &id, line);
        });
    }
    let diagnostic_result = stderr_thread
        .join()
        .unwrap_or_else(|_| Err("APT diagnostic reader panicked.".into()));
    let reader_result = output_result.and(diagnostic_result);
    match child.wait() {
        Ok(_) if reader_result.is_err() => {
            update_failed(&jobs, &id, reader_result.unwrap_err());
            return;
        }
        Ok(status) if status.success() => {
            if let Ok(mut jobs) = jobs.lock() {
                if let Some(job) = jobs.get_mut(&id) {
                    job.status = "completed".into();
                }
            }
        }
        Ok(status) => update_failed(
            &jobs,
            &id,
            format!("APT package operation exited with {status}."),
        ),
        Err(error) => update_failed(
            &jobs,
            &id,
            format!("Could not wait for the APT package operation: {error}"),
        ),
    }
}

fn run_apt_update_job(jobs: Jobs, id: String, packages: Vec<String>) {
    let mut operation = vec![
        "apt-get".to_string(),
        "install".into(),
        "--only-upgrade".into(),
        "--no-install-recommends".into(),
        "--yes".into(),
        "--".into(),
    ];
    operation.extend(packages);
    run_apt_command_job(jobs, id, operation);
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
