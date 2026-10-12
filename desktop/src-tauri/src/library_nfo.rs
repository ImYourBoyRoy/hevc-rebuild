use crate::library_organizer::rename_without_replacing;
use rmcp::schemars;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const MAX_DOCUMENTS: usize = 10_001;
const MAX_REQUEST_BYTES: usize = 8 * 1024 * 1024;
const MAX_NFO_BYTES: usize = 128 * 1024;
static FILE_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, Deserialize, schemars::JsonSchema, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NfoKind {
    Movie,
    TvShow,
    TvEpisode,
}

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct MetadataDocument {
    /// A movie or episode video path; the selected library root for a TV show.
    pub(crate) source_path: String,
    pub(crate) kind: NfoKind,
    pub(crate) title: String,
    pub(crate) year: Option<String>,
    pub(crate) certification: Option<String>,
    pub(crate) short_description: Option<String>,
    pub(crate) plot: Option<String>,
    pub(crate) genres: Vec<String>,
    /// Provider audience rating on a 0-10 scale. The rebuild maps it to Matroska's 0-5 RATING scale.
    pub(crate) rating: Option<f64>,
    pub(crate) source_url: String,
    pub(crate) show_title: Option<String>,
    pub(crate) season: Option<u32>,
    pub(crate) episode: Option<u32>,
    pub(crate) airdate: Option<String>,
}

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct MetadataWriteRequest {
    pub(crate) root: String,
    pub(crate) documents: Vec<MetadataDocument>,
}

#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MetadataWritePreviewItem {
    pub(crate) source_path: String,
    pub(crate) target_path: String,
    pub(crate) kind: NfoKind,
    pub(crate) replaces_app_sidecar: bool,
}

#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MetadataWritePreview {
    pub(crate) preview_id: String,
    pub(crate) root: String,
    pub(crate) items: Vec<MetadataWritePreviewItem>,
    pub(crate) can_apply: bool,
}

#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MetadataWriteResult {
    pub(crate) files_written: usize,
    pub(crate) files_replaced: usize,
    pub(crate) items: Vec<MetadataWritePreviewItem>,
    pub(crate) message: String,
}

struct PlannedFile {
    target: PathBuf,
    bytes: Vec<u8>,
    previous: Option<Vec<u8>>,
    preview_item: MetadataWritePreviewItem,
}

struct WritePlan {
    preview: MetadataWritePreview,
    files: Vec<PlannedFile>,
}

fn is_inside(root: &Path, path: &Path) -> bool {
    path == root || path.starts_with(root)
}

fn is_video_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "mkv"
                    | "mp4"
                    | "m4v"
                    | "mov"
                    | "avi"
                    | "webm"
                    | "wmv"
                    | "mpg"
                    | "mpeg"
                    | "m2ts"
                    | "mts"
                    | "ts"
                    | "vob"
                    | "flv"
                    | "ogv"
            )
        })
}

fn validate_text(
    value: &str,
    label: &str,
    max_bytes: usize,
    allow_newlines: bool,
) -> Result<(), String> {
    if value.len() > max_bytes {
        return Err(format!(
            "Metadata {label} exceeds the {max_bytes}-byte limit."
        ));
    }
    if value.chars().any(|character| {
        character.is_control() && !(allow_newlines && matches!(character, '\n' | '\r' | '\t'))
    }) {
        return Err(format!(
            "Metadata {label} contains unsupported control characters."
        ));
    }
    Ok(())
}

fn optional_text(
    value: &Option<String>,
    label: &str,
    max_bytes: usize,
    multiline: bool,
) -> Result<(), String> {
    if let Some(value) = value {
        validate_text(value, label, max_bytes, multiline)?;
    }
    Ok(())
}

fn xml_escape(value: &str) -> Result<String, String> {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            character
                if (character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
                    || matches!(character as u32, 0xFFFE | 0xFFFF) =>
            {
                return Err("Metadata contains a character XML cannot represent.".into());
            }
            character => escaped.push(character),
        }
    }
    Ok(escaped)
}

fn add_element(xml: &mut String, name: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Ok(());
    }
    xml.push('<');
    xml.push_str(name);
    xml.push('>');
    xml.push_str(&xml_escape(value)?);
    xml.push_str("</");
    xml.push_str(name);
    xml.push('>');
    Ok(())
}

fn year_value(value: &Option<String>) -> Result<Option<String>, String> {
    let Some(value) = value else { return Ok(None) };
    let trimmed = value.trim();
    let first = trimmed
        .get(..4)
        .filter(|year| year.bytes().all(|byte| byte.is_ascii_digit()))
        .ok_or("Metadata year must use YYYY or YYYY-YYYY format.")?;
    let first_year = first.parse::<u16>().unwrap_or(0);
    let suffix = &trimmed[4..];
    if first_year == 0 || (!suffix.is_empty() && !valid_year_range_suffix(suffix, first_year)) {
        return Err("Metadata year must use YYYY or an ascending YYYY-YYYY range.".into());
    }
    Ok(Some(first.to_string()))
}

fn valid_year_range_suffix(suffix: &str, first_year: u16) -> bool {
    let Some(rest) = suffix
        .strip_prefix('-')
        .or_else(|| suffix.strip_prefix('–'))
    else {
        return false;
    };
    rest.len() == 4
        && rest.bytes().all(|byte| byte.is_ascii_digit())
        && rest
            .parse::<u16>()
            .is_ok_and(|last_year| last_year >= first_year)
}

fn validate_document(document: &MetadataDocument) -> Result<(), String> {
    validate_text(&document.title, "title", 1024, false)?;
    if document.title.trim().is_empty() {
        return Err("Metadata title cannot be blank.".into());
    }
    validate_text(&document.source_url, "source URL", 4096, false)?;
    if !document.source_url.is_empty() {
        let source_url = reqwest::Url::parse(&document.source_url)
            .map_err(|_| "Metadata source URL is invalid.")?;
        if source_url.scheme() != "https"
            || source_url.host_str().is_none()
            || !source_url.username().is_empty()
            || source_url.password().is_some()
        {
            return Err("Metadata source URL must use HTTPS.".into());
        }
    }
    optional_text(&document.year, "year", 128, false)?;
    year_value(&document.year)?;
    optional_text(&document.certification, "certification", 256, false)?;
    optional_text(&document.short_description, "short description", 4096, true)?;
    optional_text(&document.plot, "plot", 32 * 1024, true)?;
    optional_text(&document.show_title, "series title", 1024, false)?;
    optional_text(&document.airdate, "air date", 16, false)?;
    if let Some(date) = &document.airdate {
        if !valid_airdate(date) {
            return Err("Episode air date must use YYYY-MM-DD format.".into());
        }
    }
    if let Some(rating) = document.rating {
        if !rating.is_finite() || !(0.0..=10.0).contains(&rating) {
            return Err("Metadata audience rating must be between 0 and 10.".into());
        }
    }
    if document.genres.len() > 100 {
        return Err("Metadata can contain at most 100 genres.".into());
    }
    for genre in &document.genres {
        validate_text(genre, "genre", 256, false)?;
    }
    match document.kind {
        NfoKind::Movie => {
            if document.season.is_some()
                || document.episode.is_some()
                || document.show_title.is_some()
            {
                return Err("Movie metadata cannot contain TV episode fields.".into());
            }
        }
        NfoKind::TvShow => {
            if document.show_title.is_some()
                || document.season.is_some()
                || document.episode.is_some()
                || document.airdate.is_some()
            {
                return Err("TV series metadata cannot contain episode fields.".into());
            }
        }
        NfoKind::TvEpisode => {
            if document.season.is_none()
                || document.episode.is_none()
                || document
                    .show_title
                    .as_deref()
                    .unwrap_or("")
                    .trim()
                    .is_empty()
            {
                return Err(
                    "Episode metadata requires a series title, season, and episode number.".into(),
                );
            }
        }
    }
    Ok(())
}

fn valid_airdate(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return false;
    }
    if !bytes
        .iter()
        .enumerate()
        .all(|(index, byte)| matches!(index, 4 | 7) || byte.is_ascii_digit())
    {
        return false;
    }
    let year = value[..4].parse::<u16>().unwrap_or(0);
    let month = value[5..7].parse::<u8>().unwrap_or(0);
    let day = value[8..10].parse::<u8>().unwrap_or(0);
    if year == 0 || !(1..=12).contains(&month) || day == 0 {
        return false;
    }
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let last_day = match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    day <= last_day
}

fn serialize_document(document: &MetadataDocument) -> Result<Vec<u8>, String> {
    validate_document(document)?;
    let root = match document.kind {
        NfoKind::Movie => "movie",
        NfoKind::TvShow => "tvshow",
        NfoKind::TvEpisode => "episodedetails",
    };
    let mut xml =
        format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\" ?>\n<{root}>");
    xml.push_str("<!-- HEVC Rebuild Studio managed metadata -->");
    add_element(&mut xml, "title", &document.title)?;
    match document.kind {
        NfoKind::Movie => {
            if let Some(year) = year_value(&document.year)? {
                add_element(&mut xml, "year", &year)?;
            }
            if let Some(certification) = &document.certification {
                add_element(&mut xml, "mpaa", certification)?;
            }
            if let Some(tagline) = &document.short_description {
                add_element(&mut xml, "tagline", tagline)?;
            }
            if let Some(plot) = &document.plot {
                add_element(&mut xml, "plot", plot)?;
            }
            add_common(document, &mut xml)?;
        }
        NfoKind::TvShow => {
            if let Some(year) = year_value(&document.year)? {
                add_element(&mut xml, "year", &year)?;
            }
            if let Some(certification) = &document.certification {
                add_element(&mut xml, "mpaa", certification)?;
            }
            if let Some(tagline) = &document.short_description {
                add_element(&mut xml, "tagline", tagline)?;
            }
            if let Some(plot) = &document.plot {
                add_element(&mut xml, "plot", plot)?;
            }
            add_common(document, &mut xml)?;
        }
        NfoKind::TvEpisode => {
            add_element(
                &mut xml,
                "showtitle",
                document.show_title.as_deref().unwrap_or_default(),
            )?;
            add_element(
                &mut xml,
                "season",
                &document.season.unwrap_or_default().to_string(),
            )?;
            add_element(
                &mut xml,
                "episode",
                &document.episode.unwrap_or_default().to_string(),
            )?;
            if let Some(airdate) = &document.airdate {
                add_element(&mut xml, "aired", airdate)?;
            }
            if let Some(plot) = &document.plot {
                add_element(&mut xml, "plot", plot)?;
            }
            if let Some(rating) = document.rating {
                add_rating(&mut xml, rating)?;
            }
            if !document.source_url.is_empty() {
                add_element(&mut xml, "source", &document.source_url)?;
            }
        }
    }
    xml.push_str("</");
    xml.push_str(root);
    xml.push('>');
    if xml.len() > MAX_NFO_BYTES {
        return Err("Generated metadata sidecar exceeds the 128 KiB limit.".into());
    }
    Ok(xml.into_bytes())
}

fn add_common(document: &MetadataDocument, xml: &mut String) -> Result<(), String> {
    if let Some(rating) = document.rating {
        add_rating(xml, rating)?;
    }
    for genre in &document.genres {
        add_element(xml, "genre", genre)?;
    }
    if !document.source_url.is_empty() {
        add_element(xml, "source", &document.source_url)?;
    }
    Ok(())
}

fn add_rating(xml: &mut String, rating: f64) -> Result<(), String> {
    xml.push_str("<userrating max=\"10\">");
    xml.push_str(&xml_escape(&format!("{rating:.2}"))?);
    xml.push_str("</userrating>");
    Ok(())
}

fn target_path(root: &Path, source: &Path, kind: &NfoKind) -> Result<(PathBuf, bool), String> {
    let (canonical, app_managed) = match kind {
        NfoKind::TvShow => (
            root.join("tvshow.nfo"),
            root.join("tvshow.hevc-rebuild.nfo"),
        ),
        NfoKind::Movie | NfoKind::TvEpisode => {
            let stem = source
                .file_stem()
                .ok_or("A video source has no filename stem.")?;
            let mut canonical = source.with_extension("nfo");
            if source.extension().is_none() {
                canonical = source.with_file_name(format!("{}.nfo", stem.to_string_lossy()));
            }
            let mut managed_name = stem.to_os_string();
            managed_name.push(".hevc-rebuild.nfo");
            (canonical, source.with_file_name(managed_name))
        }
    };
    let canonical_exists = match fs::symlink_metadata(&canonical) {
        Ok(_) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => {
            return Err(format!(
                "Could not inspect existing NFO {}: {error}",
                canonical.display()
            ))
        }
    };
    let mut canonical_is_app_owned = false;
    if canonical_exists
        && fs::symlink_metadata(&canonical)
            .is_ok_and(|metadata| metadata.is_file() && metadata.len() <= MAX_NFO_BYTES as u64)
    {
        if let Ok(bytes) = fs::read(&canonical) {
            canonical_is_app_owned = String::from_utf8_lossy(&bytes)
                .contains("<!-- HEVC Rebuild Studio managed metadata -->");
        }
    }
    let (target, app_managed) = if canonical_is_app_owned {
        (canonical, true)
    } else if canonical_exists {
        (app_managed, true)
    } else {
        (canonical, false)
    };
    if let Ok(metadata) = fs::symlink_metadata(&target) {
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(format!(
                "Metadata destination is not a regular file: {}",
                target.display()
            ));
        }
        if metadata.len() > MAX_NFO_BYTES as u64 {
            return Err(format!(
                "Existing app metadata sidecar exceeds the 128 KiB safety limit: {}",
                target.display()
            ));
        }
    }
    Ok((target, app_managed))
}

fn build_plan(request: MetadataWriteRequest) -> Result<WritePlan, String> {
    let request_bytes = serde_json::to_vec(&request)
        .map_err(|error| format!("Could not encode metadata request: {error}"))?;
    if request_bytes.len() > MAX_REQUEST_BYTES {
        return Err("Metadata write request exceeds the 8 MiB limit.".into());
    }
    if request.documents.is_empty() || request.documents.len() > MAX_DOCUMENTS {
        return Err(format!(
            "Metadata write requires 1 to {MAX_DOCUMENTS} documents."
        ));
    }
    let root = Path::new(&request.root)
        .canonicalize()
        .map_err(|error| format!("Could not open metadata library root: {error}"))?;
    if !root.is_dir() {
        return Err("Metadata library root must be a directory.".into());
    }
    let mut files = Vec::with_capacity(request.documents.len());
    let mut targets = HashSet::new();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    root.to_string_lossy().hash(&mut hasher);
    for document in request.documents {
        validate_document(&document)?;
        let requested_source = Path::new(&document.source_path);
        let requested_metadata = fs::symlink_metadata(requested_source).map_err(|error| {
            format!(
                "Could not inspect metadata source {}: {error}",
                requested_source.display()
            )
        })?;
        if requested_metadata.file_type().is_symlink() {
            return Err(format!(
                "Metadata source cannot be a symlink: {}",
                requested_source.display()
            ));
        }
        let source = requested_source.canonicalize().map_err(|error| {
            format!(
                "Could not open metadata source {}: {error}",
                requested_source.display()
            )
        })?;
        if !is_inside(&root, &source) {
            return Err(format!(
                "Metadata source is outside the selected library root: {}",
                source.display()
            ));
        }
        match document.kind {
            NfoKind::TvShow if source != root || !source.is_dir() => {
                return Err("TV show metadata must target the selected library root folder.".into())
            }
            NfoKind::Movie | NfoKind::TvEpisode if !source.is_file() || !is_video_file(&source) => {
                return Err(format!(
                    "Metadata source is not a supported video file: {}",
                    source.display()
                ))
            }
            _ => {}
        }
        let (target, app_managed) = target_path(&root, &source, &document.kind)?;
        if !is_inside(&root, &target) {
            return Err("Generated metadata destination escaped the selected library root.".into());
        }
        if !targets.insert(target.clone()) {
            return Err(format!(
                "Two metadata documents would write the same destination: {}",
                target.display()
            ));
        }
        let bytes = serialize_document(&document)?;
        serde_json::to_vec(&document)
            .map_err(|error| format!("Could not encode metadata document: {error}"))?
            .hash(&mut hasher);
        source.to_string_lossy().hash(&mut hasher);
        let source_meta = fs::metadata(&source).map_err(|error| {
            format!(
                "Could not inspect metadata source {}: {error}",
                source.display()
            )
        })?;
        source_meta.len().hash(&mut hasher);
        source_meta
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|time| time.as_nanos())
            .hash(&mut hasher);
        target.to_string_lossy().hash(&mut hasher);
        app_managed.hash(&mut hasher);
        let previous = match fs::read(&target) {
            Ok(bytes) => {
                if !app_managed {
                    return Err(format!(
                        "Refusing to replace a user-owned NFO file: {}",
                        target.display()
                    ));
                }
                if bytes.len() > MAX_NFO_BYTES {
                    return Err(format!(
                        "Existing app metadata sidecar exceeds the 128 KiB safety limit: {}",
                        target.display()
                    ));
                }
                bytes.hash(&mut hasher);
                Some(bytes)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                0u8.hash(&mut hasher);
                None
            }
            Err(error) => {
                return Err(format!(
                    "Could not read metadata destination {}: {error}",
                    target.display()
                ))
            }
        };
        let item = MetadataWritePreviewItem {
            source_path: source.to_string_lossy().into_owned(),
            target_path: target.to_string_lossy().into_owned(),
            kind: document.kind,
            replaces_app_sidecar: app_managed && previous.is_some(),
        };
        files.push(PlannedFile {
            target,
            bytes,
            previous,
            preview_item: item,
        });
    }
    let preview_id = format!("metadata-{:016x}", hasher.finish());
    let items = files
        .iter()
        .map(|file| file.preview_item.clone())
        .collect::<Vec<_>>();
    Ok(WritePlan {
        preview: MetadataWritePreview {
            preview_id,
            root: root.to_string_lossy().into_owned(),
            items,
            can_apply: true,
        },
        files,
    })
}

pub(crate) fn preview_metadata_write(
    request: MetadataWriteRequest,
) -> Result<MetadataWritePreview, String> {
    Ok(build_plan(request)?.preview)
}

fn unique_sibling(path: &Path, marker: &str) -> PathBuf {
    let sequence = FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{marker}.{}.{}", std::process::id(), sequence));
    path.with_file_name(name)
}

fn write_new_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| {
            format!(
                "Could not stage metadata sidecar {}: {error}",
                path.display()
            )
        })?;
    if let Err(error) = file.write_all(bytes).and_then(|_| file.sync_all()) {
        let _ = fs::remove_file(path);
        return Err(format!(
            "Could not write metadata sidecar {}: {error}",
            path.display()
        ));
    }
    Ok(())
}

pub(crate) fn apply_metadata_write(
    request: MetadataWriteRequest,
    preview_id: String,
    confirmed: bool,
) -> Result<MetadataWriteResult, String> {
    if !confirmed {
        return Err("Explicit confirmation is required before writing metadata sidecars.".into());
    }
    let plan = build_plan(request)?;
    if plan.preview.preview_id != preview_id {
        return Err(
            "Metadata or destinations changed after preview. Review a fresh metadata preview."
                .into(),
        );
    }
    let mut staged = Vec::with_capacity(plan.files.len());
    for file in &plan.files {
        let stage = unique_sibling(&file.target, "metadata-stage");
        if let Err(error) = write_new_file(&stage, &file.bytes) {
            for path in staged {
                let _ = fs::remove_file(path);
            }
            return Err(error);
        }
        staged.push(stage);
    }
    let mut backups: Vec<Option<PathBuf>> = Vec::with_capacity(plan.files.len());
    let mut placed = Vec::new();
    for (index, file) in plan.files.iter().enumerate() {
        let backup = if file.previous.is_some() {
            let path = unique_sibling(&file.target, "metadata-backup");
            if let Err(error) = rename_without_replacing(&file.target, &path) {
                rollback_metadata(&plan.files, &staged, &backups, &placed);
                return Err(format!(
                    "Could not preserve the existing app metadata sidecar {}: {error}",
                    file.target.display()
                ));
            }
            Some(path)
        } else {
            None
        };
        backups.push(backup);
        if let Err(error) = rename_without_replacing(&staged[index], &file.target) {
            rollback_metadata(&plan.files, &staged, &backups, &placed);
            return Err(format!(
                "Could not place metadata sidecar {}: {error}",
                file.target.display()
            ));
        }
        placed.push(index);
    }
    let files_replaced = backups.iter().filter(|backup| backup.is_some()).count();
    for backup in backups.iter().flatten() {
        let _ = fs::remove_file(backup);
    }
    Ok(MetadataWriteResult {
        files_written: plan.files.len(),
        files_replaced,
        items: plan.preview.items,
        message: "Reviewed Kodi-compatible metadata sidecars were written. A later rebuild reads movie and episode sidecars and embeds supported metadata into the new Matroska file; the original media was not modified.".into(),
    })
}

fn rollback_metadata(
    files: &[PlannedFile],
    staged: &[PathBuf],
    backups: &[Option<PathBuf>],
    placed: &[usize],
) {
    for index in placed.iter().rev() {
        let _ = fs::remove_file(&files[*index].target);
    }
    for (index, backup) in backups.iter().enumerate().rev() {
        if let Some(backup) = backup {
            let _ = rename_without_replacing(backup, &files[index].target);
        }
    }
    for path in staged {
        let _ = fs::remove_file(path);
    }
}

pub(crate) fn read_request(path: &Path) -> Result<MetadataWriteRequest, String> {
    let metadata = fs::metadata(path)
        .map_err(|error| format!("Could not inspect metadata request JSON: {error}"))?;
    if !metadata.is_file() || metadata.len() > MAX_REQUEST_BYTES as u64 {
        return Err("Metadata request JSON must be a regular file no larger than 8 MiB.".into());
    }
    let bytes =
        fs::read(path).map_err(|error| format!("Could not read metadata request JSON: {error}"))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("Metadata request JSON is invalid: {error}"))
}

pub(crate) fn metadata_cli_mode(args: &[String]) -> i32 {
    let usage = "Usage: hevc-rebuild-studio metadata write-preview REQUEST.json | write-apply REQUEST.json --preview-id ID --confirm";
    let result = match args {
        [mode, request_path] if mode == "write-preview" => read_request(Path::new(request_path))
            .and_then(preview_metadata_write)
            .and_then(|value| serde_json::to_value(value).map_err(|error| error.to_string())),
        [mode, request_path, preview_flag, preview_id, confirm]
            if mode == "write-apply"
                && preview_flag == "--preview-id"
                && confirm == "--confirm" =>
        {
            read_request(Path::new(request_path))
                .and_then(|request| apply_metadata_write(request, preview_id.clone(), true))
                .and_then(|value| serde_json::to_value(value).map_err(|error| error.to_string()))
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
                eprintln!("Could not format metadata result: {error}");
                2
            }
        },
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "hevc-nfo-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn document(source: &Path) -> MetadataDocument {
        MetadataDocument {
            source_path: source.to_string_lossy().into_owned(),
            kind: NfoKind::Movie,
            title: "Gnomeo & Juliet".into(),
            year: Some("2011".into()),
            certification: Some("G".into()),
            short_description: Some("A short <tagline>".into()),
            plot: Some("Plot & detail".into()),
            genres: vec!["Animation".into()],
            rating: Some(7.5),
            source_url: "https://www.themoviedb.org/movie/1".into(),
            show_title: None,
            season: None,
            episode: None,
            airdate: None,
        }
    }

    #[test]
    fn preview_preserves_user_nfo_and_apply_writes_app_managed_xml() {
        let root = temp_root();
        let source = root.join("film.mkv");
        fs::write(&source, b"video").unwrap();
        fs::write(root.join("film.nfo"), b"user-owned NFO").unwrap();
        let request = MetadataWriteRequest {
            root: root.to_string_lossy().into_owned(),
            documents: vec![document(&source)],
        };
        let preview = preview_metadata_write(request.clone()).unwrap();
        assert!(preview.items[0]
            .target_path
            .ends_with("film.hevc-rebuild.nfo"));
        assert!(!preview.items[0].replaces_app_sidecar);
        let result = apply_metadata_write(request, preview.preview_id, true).unwrap();
        assert_eq!(result.files_written, 1);
        assert_eq!(fs::read(root.join("film.nfo")).unwrap(), b"user-owned NFO");
        let xml = fs::read_to_string(root.join("film.hevc-rebuild.nfo")).unwrap();
        assert!(xml.contains("Gnomeo &amp; Juliet"));
        assert!(xml.contains("A short &lt;tagline&gt;"));
        assert!(xml.contains("<userrating max=\"10\">7.50</userrating>"));
        let replacement = preview_metadata_write(MetadataWriteRequest {
            root: root.to_string_lossy().into_owned(),
            documents: vec![document(&source)],
        })
        .unwrap();
        assert!(replacement.items[0].replaces_app_sidecar);
        assert_eq!(
            replacement.items[0].target_path,
            root.join("film.hevc-rebuild.nfo")
                .to_string_lossy()
                .into_owned()
        );
        apply_metadata_write(
            MetadataWriteRequest {
                root: root.to_string_lossy().into_owned(),
                documents: vec![document(&source)],
            },
            replacement.preview_id,
            true,
        )
        .unwrap();
        assert_eq!(fs::read(root.join("film.nfo")).unwrap(), b"user-owned NFO");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn optional_source_url_is_omitted_from_movie_nfo() {
        let root = temp_root();
        let source = root.join("film.mkv");
        fs::write(&source, b"video").unwrap();
        let mut draft = document(&source);
        draft.source_url.clear();

        let xml = String::from_utf8(serialize_document(&draft).unwrap()).unwrap();
        assert!(xml.contains("<title>Gnomeo &amp; Juliet</title>"));
        assert!(!xml.contains("<source>"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn stale_preview_and_missing_confirmation_do_not_write() {
        let root = temp_root();
        let source = root.join("film.mkv");
        fs::write(&source, b"video").unwrap();
        let request = MetadataWriteRequest {
            root: root.to_string_lossy().into_owned(),
            documents: vec![document(&source)],
        };
        let preview = preview_metadata_write(request.clone()).unwrap();
        assert!(apply_metadata_write(request.clone(), preview.preview_id.clone(), false).is_err());
        fs::write(root.join("film.nfo"), b"new user NFO").unwrap();
        assert!(apply_metadata_write(request, preview.preview_id, true).is_err());
        assert!(!root.join("film.hevc-rebuild.nfo").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn invalid_rating_dates_controls_and_outside_paths_are_rejected() {
        let root = temp_root();
        let source = root.join("film.mkv");
        fs::write(&source, b"video").unwrap();
        let mut invalid = document(&source);
        invalid.rating = Some(10.1);
        assert!(serialize_document(&invalid).is_err());
        let mut invalid = document(&source);
        invalid.title = "bad\u{1}".into();
        assert!(serialize_document(&invalid).is_err());
        let outside = temp_root().join("outside.mkv");
        fs::write(&outside, b"video").unwrap();
        let request = MetadataWriteRequest {
            root: root.to_string_lossy().into_owned(),
            documents: vec![document(&outside)],
        };
        assert!(preview_metadata_write(request).is_err());
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(outside.parent().unwrap()).unwrap();
    }
}
