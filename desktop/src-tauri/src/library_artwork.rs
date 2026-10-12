use crate::library_metadata::{ArtworkCandidate, MetadataProvider};
use reqwest::{redirect, Client, Url};
use rmcp::schemars;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs::{self, Metadata, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, UNIX_EPOCH};
use unicode_normalization::UnicodeNormalization;

const MAX_ARTWORK_BYTES: usize = 16 * 1024 * 1024;
const MAX_ARTWORK_TARGETS: usize = 128;
const MAX_BATCH_BYTES: usize = 512 * 1024 * 1024;
const MAX_ATTRIBUTION_BYTES: usize = 16 * 1024;
static FILE_COUNTER: AtomicU64 = AtomicU64::new(1);
static ARTWORK_CACHE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ArtworkTarget {
    pub(crate) directory: String,
    pub(crate) basename: String,
}

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ArtworkSaveRequest {
    pub(crate) root: String,
    pub(crate) candidate: ArtworkCandidate,
    pub(crate) targets: Vec<ArtworkTarget>,
}

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct LocalArtworkSaveRequest {
    pub(crate) root: String,
    pub(crate) source_path: String,
    pub(crate) targets: Vec<ArtworkTarget>,
}

#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SavedArtworkFile {
    pub(crate) path: String,
    pub(crate) attribution_path: String,
}

#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ArtworkSaveResult {
    pub(crate) files: Vec<SavedArtworkFile>,
    pub(crate) bytes_per_file: usize,
    pub(crate) message: String,
}

pub(crate) fn app_artwork_cache_root() -> Result<PathBuf, String> {
    let preferences = crate::app_preferences_file()?;
    let profile = preferences
        .parent()
        .ok_or("Could not locate the application profile folder.")?;
    Ok(profile.join(".cache/artwork"))
}

#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ArtworkCacheInfo {
    pub(crate) available: bool,
    pub(crate) image_count: usize,
    pub(crate) total_file_count: usize,
    pub(crate) total_bytes: u64,
    pub(crate) purgeable_image_count: usize,
    pub(crate) purgeable_bytes: u64,
    pub(crate) protected_image_count: usize,
    pub(crate) protected_bytes: u64,
    pub(crate) unmanaged_entry_count: usize,
    pub(crate) preview_id: String,
}

#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ArtworkCachePurgeResult {
    pub(crate) deleted_image_count: usize,
    pub(crate) deleted_bytes: u64,
    pub(crate) failed_file_count: usize,
    pub(crate) remaining: ArtworkCacheInfo,
    pub(crate) message: String,
}

#[derive(Clone)]
struct CacheFileSnapshot {
    path: PathBuf,
    name: String,
    bytes: u64,
    modified_ns: u128,
}

#[derive(Clone)]
struct CachedArtworkSnapshot {
    image: CacheFileSnapshot,
    attribution: Option<CacheFileSnapshot>,
    protected: bool,
}

struct ArtworkCacheSnapshot {
    root: Option<PathBuf>,
    info: ArtworkCacheInfo,
    artwork: Vec<CachedArtworkSnapshot>,
    references: HashSet<PathBuf>,
}

fn cache_modified_ns(metadata: &Metadata) -> u128 {
    metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |duration| duration.as_nanos())
}

fn is_managed_artwork_name(name: &str) -> bool {
    let Some((identity, extension)) = name.rsplit_once('.') else {
        return false;
    };
    let Some((kind, digest)) = identity.split_once('-') else {
        return false;
    };
    matches!(kind, "provider" | "local")
        && digest.len() == 32
        && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
        && matches!(
            extension.to_ascii_lowercase().as_str(),
            "jpg" | "png" | "webp"
        )
}

fn canonical_cache_root() -> Result<Option<PathBuf>, String> {
    let cache = app_artwork_cache_root()?;
    let profile = cache
        .parent()
        .and_then(Path::parent)
        .ok_or("Could not locate the application profile folder.")?;
    if !profile.exists() {
        return Ok(None);
    }
    let canonical_profile = profile
        .canonicalize()
        .map_err(|error| format!("Could not resolve the application profile: {error}"))?;
    let metadata = match fs::symlink_metadata(&cache) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("Could not inspect the artwork cache: {error}")),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(
            "The artwork cache must be a real folder inside the application profile.".into(),
        );
    }
    let canonical_cache = cache
        .canonicalize()
        .map_err(|error| format!("Could not resolve the artwork cache: {error}"))?;
    if !canonical_cache.starts_with(&canonical_profile) {
        return Err("The artwork cache must remain inside the application profile folder.".into());
    }
    Ok(Some(canonical_cache))
}

fn queued_artwork_references(cache: &Path) -> Result<HashSet<PathBuf>, String> {
    let queue_path = crate::media_queue_file()?;
    let queue = crate::media_queue::read_snapshot(&queue_path)?;
    let mut referenced = HashSet::new();
    let paths = queue
        .movie_reviews
        .values()
        .map(|review| review.local_poster_path.as_str())
        .chain(queue.series_reviews.values().flat_map(|review| {
            std::iter::once(review.local_poster_path.as_str()).chain(
                review
                    .local_season_poster_paths
                    .values()
                    .map(String::as_str),
            )
        }));
    for value in paths.filter(|value| !value.is_empty()) {
        if let Ok(path) = Path::new(value).canonicalize() {
            if path.starts_with(cache) {
                referenced.insert(path);
            }
        }
    }
    Ok(referenced)
}

fn snapshot_cache_file(path: &Path, metadata: &Metadata) -> CacheFileSnapshot {
    CacheFileSnapshot {
        path: path.to_path_buf(),
        name: path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        bytes: metadata.len(),
        modified_ns: cache_modified_ns(metadata),
    }
}

fn build_artwork_cache_snapshot_at(
    root: Option<PathBuf>,
    references: &HashSet<PathBuf>,
) -> Result<ArtworkCacheSnapshot, String> {
    let Some(root) = root else {
        let preview_id = sha256_hex(b"empty-artwork-cache");
        return Ok(ArtworkCacheSnapshot {
            root: None,
            info: ArtworkCacheInfo {
                available: false,
                image_count: 0,
                total_file_count: 0,
                total_bytes: 0,
                purgeable_image_count: 0,
                purgeable_bytes: 0,
                protected_image_count: 0,
                protected_bytes: 0,
                unmanaged_entry_count: 0,
                preview_id,
            },
            artwork: Vec::new(),
            references: references.clone(),
        });
    };
    let entries = fs::read_dir(&root)
        .map_err(|error| format!("Could not read the artwork cache: {error}"))?;
    let mut total_file_count = 0usize;
    let mut total_bytes = 0u64;
    let mut unmanaged_entry_count = 0usize;
    let mut manifest = Vec::new();
    let mut images = HashMap::<String, CacheFileSnapshot>::new();
    let mut attributions = HashMap::<String, CacheFileSnapshot>::new();

    for entry in entries {
        let entry =
            entry.map_err(|error| format!("Could not read an artwork cache entry: {error}"))?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("Could not inspect an artwork cache entry: {error}"))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !metadata.file_type().is_file() {
            unmanaged_entry_count = unmanaged_entry_count.saturating_add(1);
            manifest.push(format!(
                "unmanaged:{name}:{}:{}",
                metadata.len(),
                cache_modified_ns(&metadata)
            ));
            continue;
        }
        total_file_count = total_file_count.saturating_add(1);
        total_bytes = total_bytes.saturating_add(metadata.len());
        let snapshot = snapshot_cache_file(&path, &metadata);
        manifest.push(format!(
            "file:{name}:{}:{}",
            metadata.len(),
            snapshot.modified_ns
        ));
        if let Some(image_name) = name.strip_suffix(".attribution.json") {
            if is_managed_artwork_name(image_name) {
                attributions.insert(image_name.to_owned(), snapshot);
            } else {
                unmanaged_entry_count = unmanaged_entry_count.saturating_add(1);
            }
        } else if is_managed_artwork_name(&name) {
            images.insert(name, snapshot);
        } else {
            unmanaged_entry_count = unmanaged_entry_count.saturating_add(1);
        }
    }

    let mut artwork = Vec::with_capacity(images.len());
    for (name, image) in images {
        let protected = references.contains(
            &image
                .path
                .canonicalize()
                .unwrap_or_else(|_| image.path.clone()),
        );
        artwork.push(CachedArtworkSnapshot {
            attribution: attributions.remove(&name),
            image,
            protected,
        });
    }
    unmanaged_entry_count = unmanaged_entry_count.saturating_add(attributions.len());
    artwork.sort_by(|left, right| left.image.name.cmp(&right.image.name));
    manifest.sort();
    for item in &artwork {
        manifest.push(format!("assigned:{}:{}", item.image.name, item.protected));
    }

    let image_count = artwork.len();
    let protected_image_count = artwork.iter().filter(|item| item.protected).count();
    let purgeable_image_count = image_count.saturating_sub(protected_image_count);
    let protected_bytes = artwork
        .iter()
        .filter(|item| item.protected)
        .map(|item| {
            item.image
                .bytes
                .saturating_add(item.attribution.as_ref().map_or(0, |file| file.bytes))
        })
        .fold(0u64, u64::saturating_add);
    let purgeable_bytes = artwork
        .iter()
        .filter(|item| !item.protected)
        .map(|item| {
            item.image
                .bytes
                .saturating_add(item.attribution.as_ref().map_or(0, |file| file.bytes))
        })
        .fold(0u64, u64::saturating_add);
    let preview_id = sha256_hex(manifest.join("\n").as_bytes());
    Ok(ArtworkCacheSnapshot {
        root: Some(root),
        info: ArtworkCacheInfo {
            available: true,
            image_count,
            total_file_count,
            total_bytes,
            purgeable_image_count,
            purgeable_bytes,
            protected_image_count,
            protected_bytes,
            unmanaged_entry_count,
            preview_id,
        },
        artwork,
        references: references.clone(),
    })
}

fn build_artwork_cache_snapshot() -> Result<ArtworkCacheSnapshot, String> {
    let Some(root) = canonical_cache_root()? else {
        return build_artwork_cache_snapshot_at(None, &HashSet::new());
    };
    let references = queued_artwork_references(&root)?;
    build_artwork_cache_snapshot_at(Some(root), &references)
}

pub(crate) async fn inspect_app_artwork_cache() -> Result<ArtworkCacheInfo, String> {
    let _cache_lock = ARTWORK_CACHE_LOCK.lock().await;
    let _queue_lock = crate::media_queue::lock_for_artwork_cache_purge()?;
    Ok(build_artwork_cache_snapshot()?.info)
}

pub(crate) async fn purge_unused_app_artwork_cache(
    preview_id: &str,
    confirmed: bool,
) -> Result<ArtworkCachePurgeResult, String> {
    if !confirmed {
        return Err(
            "Artwork-cache cleanup requires explicit confirmation after reviewing its preview."
                .into(),
        );
    }
    if preview_id.trim().is_empty() {
        return Err("Refresh the artwork-cache summary before confirming a purge.".into());
    }
    let _cache_lock = ARTWORK_CACHE_LOCK.lock().await;
    let _queue_lock = crate::media_queue::lock_for_artwork_cache_purge()?;
    let snapshot = build_artwork_cache_snapshot()?;
    if snapshot.info.preview_id != preview_id {
        return Err("The artwork cache or its queue assignments changed since review. Refresh the cache summary before confirming.".into());
    }
    purge_artwork_cache_snapshot(snapshot, preview_id)
}

fn purge_artwork_cache_snapshot(
    snapshot: ArtworkCacheSnapshot,
    preview_id: &str,
) -> Result<ArtworkCachePurgeResult, String> {
    if snapshot.info.preview_id != preview_id {
        return Err("The artwork cache or its queue assignments changed since review. Refresh the cache summary before confirming.".into());
    }
    let Some(root) = snapshot.root.as_deref() else {
        return Ok(ArtworkCachePurgeResult {
            deleted_image_count: 0,
            deleted_bytes: 0,
            failed_file_count: 0,
            remaining: snapshot.info,
            message: "The private artwork cache is already empty.".into(),
        });
    };
    let purgeable = snapshot
        .artwork
        .iter()
        .filter(|item| !item.protected)
        .collect::<Vec<_>>();

    // Verify the reviewed file set again before removing any entry. Only app-
    // named image files and their matching attribution sidecars are candidates.
    for item in &purgeable {
        for file in std::iter::once(&item.image).chain(item.attribution.iter()) {
            let metadata = fs::symlink_metadata(&file.path).map_err(|error| {
                format!("The reviewed artwork cache changed. Refresh it before purging: {error}")
            })?;
            if !metadata.file_type().is_file()
                || metadata.len() != file.bytes
                || cache_modified_ns(&metadata) != file.modified_ns
                || !file
                    .path
                    .canonicalize()
                    .is_ok_and(|path| path.starts_with(root))
            {
                return Err(
                    "The reviewed artwork cache changed. Refresh it before purging.".into(),
                );
            }
        }
    }

    let mut deleted_image_count = 0usize;
    let mut deleted_bytes = 0u64;
    let mut failed_file_count = 0usize;
    for item in purgeable {
        let mut image_deleted = false;
        for file in item.attribution.iter().chain(std::iter::once(&item.image)) {
            match fs::remove_file(&file.path) {
                Ok(()) => {
                    deleted_bytes = deleted_bytes.saturating_add(file.bytes);
                    if file.path == item.image.path {
                        image_deleted = true;
                    }
                }
                Err(_) => failed_file_count = failed_file_count.saturating_add(1),
            }
        }
        if image_deleted {
            deleted_image_count = deleted_image_count.saturating_add(1);
        }
    }
    let remaining =
        build_artwork_cache_snapshot_at(snapshot.root.clone(), &snapshot.references)?.info;
    let message = if failed_file_count == 0 {
        format!(
            "Removed {deleted_image_count} unused poster image(s), freeing {}.",
            format_cache_bytes(deleted_bytes)
        )
    } else {
        format!(
            "Removed {deleted_image_count} unused poster image(s), freeing {}; {failed_file_count} cache file(s) could not be removed.",
            format_cache_bytes(deleted_bytes)
        )
    };
    Ok(ArtworkCachePurgeResult {
        deleted_image_count,
        deleted_bytes,
        failed_file_count,
        remaining,
        message,
    })
}

fn format_cache_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

fn prepare_private_artwork_cache() -> Result<PathBuf, String> {
    let cache = app_artwork_cache_root()?;
    let profile = cache
        .parent()
        .and_then(Path::parent)
        .ok_or("Could not locate the application profile folder.")?;
    fs::create_dir_all(&cache)
        .map_err(|error| format!("Could not prepare the artwork cache: {error}"))?;
    let canonical_profile = profile
        .canonicalize()
        .map_err(|error| format!("Could not resolve the application profile: {error}"))?;
    let canonical_cache = cache
        .canonicalize()
        .map_err(|error| format!("Could not resolve the artwork cache: {error}"))?;
    if !canonical_cache.is_dir() || !canonical_cache.starts_with(&canonical_profile) {
        return Err("The artwork cache must remain inside the application profile folder.".into());
    }
    #[cfg(unix)]
    fs::set_permissions(&canonical_cache, fs::Permissions::from_mode(0o700))
        .map_err(|error| format!("Could not secure the artwork cache folder: {error}"))?;
    Ok(canonical_cache)
}

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn make_cached_files_private(files: &[SavedArtworkFile]) -> Result<(), String> {
    #[cfg(unix)]
    for path in files
        .iter()
        .flat_map(|file| [&file.path, &file.attribution_path])
    {
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("Could not secure cached artwork file {}: {error}", path))?;
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LocalArtworkSaveResult {
    pub(crate) files: Vec<String>,
    pub(crate) bytes_per_file: usize,
    pub(crate) message: String,
}

fn expected_host(provider: &MetadataProvider) -> Option<&'static str> {
    match provider {
        MetadataProvider::WikimediaCommons => Some("upload.wikimedia.org"),
        MetadataProvider::TvMaze => Some("static.tvmaze.com"),
        MetadataProvider::Tmdb => Some("image.tmdb.org"),
        MetadataProvider::Tvdb => Some("artworks.thetvdb.com"),
        MetadataProvider::Wikidata | MetadataProvider::Wikipedia => None,
    }
}

fn expected_source_host(provider: &MetadataProvider) -> &'static str {
    match provider {
        MetadataProvider::WikimediaCommons => "commons.wikimedia.org",
        MetadataProvider::TvMaze => "www.tvmaze.com",
        MetadataProvider::Tmdb => "www.themoviedb.org",
        MetadataProvider::Tvdb => "www.thetvdb.com",
        MetadataProvider::Wikidata => "www.wikidata.org",
        MetadataProvider::Wikipedia => "en.wikipedia.org",
    }
}

fn normalize_credit_text(
    field: &mut Option<String>,
    label: &str,
    limit: usize,
) -> Result<(), String> {
    let Some(value) = field.take() else {
        return Ok(());
    };
    if value.len() > limit
        || value
            .chars()
            .any(|character| character.is_control() && !character.is_whitespace())
    {
        return Err(format!("Artwork {label} details are invalid or too long."));
    }
    let value = value.split_whitespace().collect::<Vec<_>>().join(" ");
    *field = if value.is_empty() { None } else { Some(value) };
    Ok(())
}

fn normalize_credit_fields(candidate: &mut ArtworkCandidate) -> Result<(), String> {
    normalize_credit_text(&mut candidate.creator, "creator", 2048)?;
    normalize_credit_text(&mut candidate.license, "license", 2048)?;
    normalize_credit_text(&mut candidate.attribution, "attribution", 4096)?;
    Ok(())
}

fn validate_image_url(provider: &MetadataProvider, value: &str) -> Result<Url, String> {
    let expected =
        expected_host(provider).ok_or("This provider does not supply downloadable artwork.")?;
    let url = Url::parse(value).map_err(|_| "Artwork URL is invalid.")?;
    if url.scheme() != "https"
        || url.host_str() != Some(expected)
        || url.username() != ""
        || url.password().is_some()
        || url.port().is_some_and(|port| port != 443)
    {
        return Err(
            "Artwork must use HTTPS from the selected provider's approved image host.".into(),
        );
    }
    let extension = url
        .path()
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !matches!(extension.as_str(), "jpg" | "jpeg" | "png" | "webp") {
        return Err("Only JPEG, PNG, and WebP poster images can be saved.".into());
    }
    Ok(url)
}

fn valid_redirect(url: &Url, host: &str) -> bool {
    url.scheme() == "https"
        && url.host_str() == Some(host)
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none_or(|port| port == 443)
}

fn detect_image(bytes: &[u8]) -> Option<(&'static str, &'static str)> {
    if bytes.len() >= 32 && bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some(("jpg", "image/jpeg"))
    } else if bytes.len() >= 32 && bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(("png", "image/png"))
    } else if bytes.len() >= 32 && bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        Some(("webp", "image/webp"))
    } else {
        None
    }
}

fn validate_candidate(candidate: &ArtworkCandidate) -> Result<(Url, String), String> {
    if candidate.id.trim().is_empty()
        || candidate.id.len() > 512
        || candidate.title.trim().is_empty()
        || candidate.title.len() > 1024
    {
        return Err("Artwork details are incomplete or too long.".into());
    }
    for value in [candidate.title.as_str(), candidate.id.as_str()] {
        if value.chars().any(char::is_control) {
            return Err("Artwork details cannot contain control characters.".into());
        }
    }
    for (label, value, limit) in [
        ("creator", candidate.creator.as_deref(), 2048),
        ("license", candidate.license.as_deref(), 2048),
        ("attribution", candidate.attribution.as_deref(), 4096),
    ] {
        if value.is_some_and(|value| value.len() > limit || value.chars().any(char::is_control)) {
            return Err(format!("Artwork {label} details are invalid or too long."));
        }
    }
    for value in [
        candidate.source_url.as_str(),
        candidate.thumbnail_url.as_str(),
    ] {
        if value.len() > 8192 || value.chars().any(char::is_control) {
            return Err("Artwork source links are invalid or too long.".into());
        }
    }
    if candidate.image_url.len() > 8192 || candidate.image_url.chars().any(char::is_control) {
        return Err("Artwork image links are invalid or too long.".into());
    }
    let source =
        Url::parse(&candidate.source_url).map_err(|_| "Artwork source link is invalid.")?;
    if source.scheme() != "https"
        || source.host_str() != Some(expected_source_host(&candidate.provider))
        || !source.username().is_empty()
        || source.password().is_some()
        || source.port().is_some_and(|port| port != 443)
    {
        return Err(
            "Artwork credit must link to the selected provider's HTTPS source page.".into(),
        );
    }
    if let Some(value) = candidate.license_url.as_deref() {
        if value.len() > 8192 || value.chars().any(char::is_control) {
            return Err("Artwork license links are invalid or too long.".into());
        }
        let license = Url::parse(value).map_err(|_| "Artwork license link is invalid.")?;
        if !matches!(license.scheme(), "https" | "http")
            || license.host_str().is_none()
            || !license.username().is_empty()
            || license.password().is_some()
            || license.port().is_some_and(|port| {
                !matches!((license.scheme(), port), ("https", 443) | ("http", 80))
            })
        {
            return Err("Artwork license links must use a public HTTP or HTTPS URL.".into());
        }
    }
    let url = validate_image_url(&candidate.provider, &candidate.image_url)?;
    let advertised_extension = url
        .path()
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    Ok((url, advertised_extension))
}

fn validate_targets(
    root: &str,
    targets: &[ArtworkTarget],
) -> Result<(PathBuf, Vec<(PathBuf, String)>), String> {
    if targets.is_empty() || targets.len() > MAX_ARTWORK_TARGETS {
        return Err(format!(
            "Choose between 1 and {MAX_ARTWORK_TARGETS} artwork destinations."
        ));
    }
    let root = PathBuf::from(root)
        .canonicalize()
        .map_err(|error| format!("Could not open the selected media folder: {error}"))?;
    if !root.is_dir() {
        return Err("The artwork root must be an existing folder.".into());
    }
    // A single reviewed image may be written under several server-specific
    // names in one folder. Uniqueness is therefore per normalized output stem,
    // rather than per destination directory.
    let mut seen = HashSet::new();
    let mut resolved = Vec::with_capacity(targets.len());
    for target in targets {
        if !Path::new(&target.directory).is_absolute() {
            return Err("Artwork destination folders must be absolute paths.".into());
        }
        if target.basename.is_empty()
            || target.basename.len() > 220
            || target.basename == "."
            || target.basename == ".."
            || Path::new(&target.basename)
                .file_name()
                .and_then(|name| name.to_str())
                != Some(target.basename.as_str())
            || target.basename.chars().any(char::is_control)
            || target.basename.contains('/')
            || target.basename.contains('\\')
        {
            return Err("Artwork filenames must be a single safe filename stem.".into());
        }
        let directory = PathBuf::from(&target.directory)
            .canonicalize()
            .map_err(|error| {
                format!(
                    "Could not open artwork destination {}: {error}",
                    target.directory
                )
            })?;
        if !directory.is_dir() || !directory.starts_with(&root) {
            return Err(
                "Artwork destinations must be existing folders under the selected library root."
                    .into(),
            );
        }
        let output_stem = directory.join(&target.basename);
        let normalized_stem = output_stem
            .to_string_lossy()
            .nfkc()
            .flat_map(char::to_lowercase)
            .collect::<String>();
        if !seen.insert(normalized_stem) {
            return Err(
                "Artwork targets must have unique filenames after Unicode and case normalization."
                    .into(),
            );
        }
        resolved.push((directory, target.basename.clone()));
    }
    Ok((root, resolved))
}

async fn fetch_image(
    provider: &MetadataProvider,
    url: Url,
) -> Result<(Vec<u8>, &'static str), String> {
    let host = expected_host(provider)
        .ok_or("This provider does not supply downloadable artwork.")?
        .to_string();
    let redirect_host = host.clone();
    let client = Client::builder()
        .timeout(Duration::from_secs(35))
        .connect_timeout(Duration::from_secs(8))
        .redirect(redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() >= 3 || !valid_redirect(attempt.url(), &redirect_host) {
                attempt.stop()
            } else {
                attempt.follow()
            }
        }))
        .user_agent(format!(
            "HEVC-Rebuild-Studio/{} (user-requested artwork download)",
            env!("CARGO_PKG_VERSION")
        ))
        .build()
        .map_err(|error| format!("Could not initialize the artwork download: {error}"))?;
    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|error| format!("Could not download the selected artwork: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "Artwork provider returned HTTP {}.",
            response.status()
        ));
    }
    if response.url().scheme() != "https" || response.url().host_str() != Some(&host) {
        return Err("Artwork provider redirected to an unapproved image host.".into());
    }
    if response
        .content_length()
        .is_some_and(|length| usize::try_from(length).unwrap_or(usize::MAX) > MAX_ARTWORK_BYTES)
    {
        return Err("The selected artwork exceeds the 16 MiB download limit.".into());
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .unwrap_or("application/octet-stream")
        .trim()
        .to_ascii_lowercase();
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("Artwork download failed while receiving data: {error}"))?
    {
        if bytes.len().saturating_add(chunk.len()) > MAX_ARTWORK_BYTES {
            return Err("The selected artwork exceeds the 16 MiB download limit.".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let Some((extension, detected_type)) = detect_image(&bytes) else {
        return Err("The downloaded file is not a supported JPEG, PNG, or WebP image.".into());
    };
    if content_type != "application/octet-stream"
        && content_type != detected_type
        && !(content_type == "image/jpg" && detected_type == "image/jpeg")
    {
        return Err("The artwork's declared image type does not match its file contents.".into());
    }
    Ok((bytes, extension))
}

fn write_stage(directory: &Path, bytes: &[u8], suffix: &str) -> Result<PathBuf, String> {
    let id = FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = directory.join(format!(
        ".hevc-artwork-{}-{id:016x}-{suffix}.tmp",
        std::process::id()
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|error| {
            format!(
                "Could not create a temporary artwork file in {}: {error}",
                directory.display()
            )
        })?;
    if let Err(error) = file.write_all(bytes).and_then(|_| file.sync_all()) {
        drop(file);
        let _ = fs::remove_file(&path);
        return Err(format!(
            "Could not finish writing artwork in {}: {error}",
            directory.display()
        ));
    }
    Ok(path)
}

pub(crate) async fn save_artwork(
    mut candidate: ArtworkCandidate,
    root: String,
    targets: Vec<ArtworkTarget>,
) -> Result<ArtworkSaveResult, String> {
    normalize_credit_fields(&mut candidate)?;
    let (url, advertised_extension) = validate_candidate(&candidate)?;
    let (_, destinations) = validate_targets(&root, &targets)?;
    let attribution = serde_json::to_vec_pretty(&candidate)
        .map_err(|error| format!("Could not record artwork attribution: {error}"))?;
    if attribution.len() > MAX_ATTRIBUTION_BYTES {
        return Err("Artwork attribution details exceed the safe storage limit.".into());
    }
    let (bytes, extension) = fetch_image(&candidate.provider, url).await?;
    if advertised_extension != extension
        && !matches!(
            (advertised_extension.as_str(), extension),
            ("jpeg", "jpg") | ("jpg", "jpeg")
        )
    {
        return Err("The downloaded image format differs from the provider's artwork URL; nothing was saved.".into());
    }
    if bytes
        .len()
        .checked_mul(destinations.len())
        .is_none_or(|total| total > MAX_BATCH_BYTES)
    {
        return Err("Saving this artwork to all selected folders would exceed the 512 MiB batch limit. Split the destinations into smaller batches.".into());
    }
    let mut output_paths = Vec::with_capacity(destinations.len() * 2);
    for (directory, basename) in &destinations {
        let image = directory.join(format!("{basename}.{extension}"));
        let sidecar = PathBuf::from(format!("{}.attribution.json", image.display()));
        for path in [&image, &sidecar] {
            if fs::symlink_metadata(path).is_ok() {
                return Err(format!(
                    "Artwork save stopped because {} already exists. Nothing was overwritten.",
                    path.display()
                ));
            }
        }
        output_paths.push((image, sidecar));
    }
    let unique_paths = output_paths
        .iter()
        .flat_map(|(image, sidecar)| [image, sidecar])
        .collect::<Vec<_>>();
    let mut all_seen = HashSet::new();
    if unique_paths
        .iter()
        .any(|path| !all_seen.insert((*path).clone()))
    {
        return Err("Artwork targets resolve to duplicate output paths.".into());
    }

    let mut created = Vec::new();
    let mut saved = Vec::with_capacity(output_paths.len());
    for (image, sidecar) in output_paths {
        let parent = image.parent().expect("validated destination has a parent");
        let image_stage = match write_stage(parent, &bytes, "image") {
            Ok(path) => path,
            Err(error) => {
                for path in created.iter().rev() {
                    let _ = fs::remove_file(path);
                }
                return Err(format!(
                    "{error} Previously created artwork sidecars were rolled back where possible."
                ));
            }
        };
        if let Err(error) = crate::library_organizer::rename_without_replacing(&image_stage, &image)
        {
            let _ = fs::remove_file(&image_stage);
            for path in created.iter().rev() {
                let _ = fs::remove_file(path);
            }
            return Err(format!("Could not place artwork at {} without replacing existing data: {error}. Previously created artwork sidecars were rolled back where possible.", image.display()));
        }
        created.push(image.clone());
        let sidecar_parent = sidecar
            .parent()
            .expect("validated destination has a parent");
        let sidecar_stage = match write_stage(sidecar_parent, &attribution, "attribution") {
            Ok(path) => path,
            Err(error) => {
                let _ = fs::remove_file(&image);
                created.retain(|path| path != &image);
                for path in created.iter().rev() {
                    let _ = fs::remove_file(path);
                }
                return Err(format!(
                    "{error} Previously created artwork sidecars were rolled back where possible."
                ));
            }
        };
        if let Err(error) =
            crate::library_organizer::rename_without_replacing(&sidecar_stage, &sidecar)
        {
            let _ = fs::remove_file(&sidecar_stage);
            for path in created.iter().rev() {
                let _ = fs::remove_file(path);
            }
            return Err(format!("Could not place artwork attribution at {} without replacing existing data: {error}. Previously created artwork sidecars were rolled back where possible.", sidecar.display()));
        }
        created.push(sidecar.clone());
        saved.push(SavedArtworkFile {
            path: image.to_string_lossy().into_owned(),
            attribution_path: sidecar.to_string_lossy().into_owned(),
        });
    }
    Ok(ArtworkSaveResult {
        files: saved,
        bytes_per_file: bytes.len(),
        message: "Artwork sidecars and source/license details were saved. A later HEVC rebuild will attach the selected local poster to its Matroska output.".into(),
    })
}

pub(crate) async fn save_artwork_to_cache(
    mut candidate: ArtworkCandidate,
) -> Result<ArtworkSaveResult, String> {
    let _cache_lock = ARTWORK_CACHE_LOCK.lock().await;
    normalize_credit_fields(&mut candidate)?;
    let (url, advertised_extension) = validate_candidate(&candidate)?;
    let cache = prepare_private_artwork_cache()?;
    let identity = serde_json::to_vec(&(
        format!("{:?}", candidate.provider),
        &candidate.id,
        url.as_str(),
    ))
    .map_err(|error| format!("Could not identify selected artwork: {error}"))?;
    let basename = format!("provider-{}", &sha256_hex(&identity)[..32]);
    let extension = if advertised_extension == "jpeg" {
        "jpg"
    } else {
        advertised_extension.as_str()
    };
    let image = cache.join(format!("{basename}.{extension}"));
    let attribution = PathBuf::from(format!("{}.attribution.json", image.display()));
    if image.exists() || attribution.exists() {
        if image.is_file()
            && attribution.is_file()
            && crate::library_assets::valid_image_file(&image)
        {
            let result = ArtworkSaveResult {
                files: vec![SavedArtworkFile {
                    path: image.to_string_lossy().into_owned(),
                    attribution_path: attribution.to_string_lossy().into_owned(),
                }],
                bytes_per_file: fs::metadata(&image)
                    .map(|metadata| metadata.len() as usize)
                    .unwrap_or_default(),
                message: "The selected poster and its source details are already in the private app artwork cache.".into(),
            };
            make_cached_files_private(&result.files)?;
            return Ok(result);
        }
        return Err("A partial or invalid cache entry already uses this artwork identity. Nothing was overwritten; clear the damaged cache entry before retrying.".into());
    }
    let result = save_artwork(
        candidate,
        cache.to_string_lossy().into_owned(),
        vec![ArtworkTarget {
            directory: cache.to_string_lossy().into_owned(),
            basename,
        }],
    )
    .await?;
    make_cached_files_private(&result.files)?;
    Ok(ArtworkSaveResult {
        files: result.files,
        bytes_per_file: result.bytes_per_file,
        message: "The selected poster and its source/license details were saved in the private app artwork cache. The image is ready for preview and can be attached to a later MKV output.".into(),
    })
}

pub(crate) fn save_local_artwork(
    request: LocalArtworkSaveRequest,
) -> Result<LocalArtworkSaveResult, String> {
    let source_path = PathBuf::from(&request.source_path)
        .canonicalize()
        .map_err(|error| format!("Could not open the selected poster image: {error}"))?;
    let metadata = fs::metadata(&source_path)
        .map_err(|error| format!("Could not inspect the selected poster image: {error}"))?;
    if !metadata.is_file() || metadata.len() > MAX_ARTWORK_BYTES as u64 {
        return Err("The selected poster must be a file no larger than 16 MiB.".into());
    }
    let bytes = fs::read(&source_path)
        .map_err(|error| format!("Could not read the selected poster image: {error}"))?;
    let Some((extension, _)) = detect_image(&bytes) else {
        return Err("Choose a valid JPEG, PNG, or WebP poster image.".into());
    };
    let named_extension = source_path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let extension_matches = match extension {
        "jpg" => matches!(named_extension.as_str(), "jpg" | "jpeg"),
        "png" => named_extension == "png",
        "webp" => named_extension == "webp",
        _ => false,
    };
    if !extension_matches {
        return Err(format!(
            "The selected poster contains {extension_upper} image data but its filename ends in .{named_extension}. Choose a file whose extension matches its image type.",
            extension_upper = extension.to_ascii_uppercase()
        ));
    }
    let (_, destinations) = validate_targets(&request.root, &request.targets)?;
    if bytes
        .len()
        .checked_mul(destinations.len())
        .is_none_or(|total| total > MAX_BATCH_BYTES)
    {
        return Err(
            "Saving this poster to all selected destinations would exceed the 512 MiB limit."
                .into(),
        );
    }
    let outputs = destinations
        .iter()
        .map(|(directory, basename)| directory.join(format!("{basename}.{extension}")))
        .collect::<Vec<_>>();
    let mut unique = HashSet::new();
    for path in &outputs {
        if !unique.insert(path.clone()) {
            return Err("Poster targets resolve to duplicate output paths.".into());
        }
        if fs::symlink_metadata(path).is_ok() {
            return Err(format!(
                "Poster save stopped because {} already exists. Nothing was overwritten.",
                path.display()
            ));
        }
    }
    let mut created = Vec::new();
    for path in &outputs {
        let parent = path.parent().expect("validated poster target has a parent");
        let stage = match write_stage(parent, &bytes, "local-image") {
            Ok(stage) => stage,
            Err(error) => {
                for prior in created.iter().rev() {
                    let _ = fs::remove_file(prior);
                }
                return Err(error);
            }
        };
        if let Err(error) = crate::library_organizer::rename_without_replacing(&stage, path) {
            let _ = fs::remove_file(&stage);
            for prior in created.iter().rev() {
                let _ = fs::remove_file(prior);
            }
            return Err(format!(
                "Could not save poster at {} without replacing existing data: {error}",
                path.display()
            ));
        }
        created.push(path.clone());
    }
    Ok(LocalArtworkSaveResult {
        files: outputs.iter().map(|path| path.to_string_lossy().into_owned()).collect(),
        bytes_per_file: bytes.len(),
        message: "Your poster image was copied beside the media file. Existing files were not overwritten.".into(),
    })
}

pub(crate) async fn save_local_artwork_to_cache(
    source_path: String,
) -> Result<LocalArtworkSaveResult, String> {
    let _cache_lock = ARTWORK_CACHE_LOCK.lock().await;
    let source = PathBuf::from(&source_path)
        .canonicalize()
        .map_err(|error| format!("Could not open the selected poster image: {error}"))?;
    let metadata = fs::metadata(&source)
        .map_err(|error| format!("Could not inspect the selected poster image: {error}"))?;
    if !metadata.is_file() || metadata.len() > MAX_ARTWORK_BYTES as u64 {
        return Err("The selected poster must be a file no larger than 16 MiB.".into());
    }
    let bytes = fs::read(&source)
        .map_err(|error| format!("Could not read the selected poster image: {error}"))?;
    let Some((extension, _)) = detect_image(&bytes) else {
        return Err("Choose a valid JPEG, PNG, or WebP poster image.".into());
    };
    let cache = prepare_private_artwork_cache()?;
    let basename = format!("local-{}", &sha256_hex(&bytes)[..32]);
    let image = cache.join(format!("{basename}.{extension}"));
    if image.exists() {
        if image.is_file() && crate::library_assets::valid_image_file(&image) {
            #[cfg(unix)]
            fs::set_permissions(&image, fs::Permissions::from_mode(0o600)).map_err(|error| {
                format!(
                    "Could not secure cached poster {}: {error}",
                    image.display()
                )
            })?;
            return Ok(LocalArtworkSaveResult {
                files: vec![image.to_string_lossy().into_owned()],
                bytes_per_file: bytes.len(),
                message: "This local poster is already saved in the private app artwork cache."
                    .into(),
            });
        }
        return Err("An invalid file already uses this local artwork cache identity. Nothing was overwritten.".into());
    }
    let result = save_local_artwork(LocalArtworkSaveRequest {
        root: cache.to_string_lossy().into_owned(),
        source_path: source.to_string_lossy().into_owned(),
        targets: vec![ArtworkTarget {
            directory: cache.to_string_lossy().into_owned(),
            basename,
        }],
    })?;
    #[cfg(unix)]
    for path in &result.files {
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("Could not secure cached poster {path}: {error}"))?;
    }
    Ok(LocalArtworkSaveResult {
        files: result.files,
        bytes_per_file: result.bytes_per_file,
        message: "The local poster was saved in the private app artwork cache.".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artwork_cache_preview_protects_assignments_and_purge_removes_only_unused_managed_files() {
        let temp = std::env::temp_dir().join(format!(
            "hevc-artwork-cache-test-{}",
            FILE_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&temp).unwrap();
        let root = temp.canonicalize().unwrap();
        let assigned = root.join(format!("local-{}.jpg", "a".repeat(32)));
        let unused = root.join(format!("provider-{}.webp", "b".repeat(32)));
        let attribution = PathBuf::from(format!("{}.attribution.json", unused.display()));
        let unmanaged = root.join("keep-this-note.txt");
        fs::write(&assigned, b"assigned").unwrap();
        fs::write(&unused, b"image").unwrap();
        fs::write(&attribution, b"credit").unwrap();
        fs::write(&unmanaged, b"leave untouched").unwrap();
        let references = HashSet::from([assigned.clone()]);

        let preview = build_artwork_cache_snapshot_at(Some(root.clone()), &references).unwrap();
        assert_eq!(preview.info.image_count, 2);
        assert_eq!(preview.info.protected_image_count, 1);
        assert_eq!(preview.info.purgeable_image_count, 1);
        assert_eq!(preview.info.purgeable_bytes, 11);
        assert_eq!(preview.info.unmanaged_entry_count, 1);

        let stale_preview_id = preview.info.preview_id.clone();
        fs::write(&unmanaged, b"changed after review").unwrap();
        let changed = build_artwork_cache_snapshot_at(Some(root.clone()), &references).unwrap();
        assert!(purge_artwork_cache_snapshot(changed, &stale_preview_id).is_err());
        assert!(assigned.exists());
        assert!(unused.exists());

        let current = build_artwork_cache_snapshot_at(Some(root.clone()), &references).unwrap();
        let current_preview_id = current.info.preview_id.clone();
        let result = purge_artwork_cache_snapshot(current, &current_preview_id).unwrap();
        assert_eq!(result.deleted_image_count, 1);
        assert_eq!(result.deleted_bytes, 11);
        assert_eq!(result.failed_file_count, 0);
        assert!(assigned.exists());
        assert!(!unused.exists());
        assert!(!attribution.exists());
        assert_eq!(fs::read(&unmanaged).unwrap(), b"changed after review");
        assert_eq!(result.remaining.protected_image_count, 1);
        assert_eq!(result.remaining.purgeable_image_count, 0);
        fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn artwork_cache_purge_requires_explicit_confirmation() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let error = runtime
            .block_on(purge_unused_app_artwork_cache("", false))
            .unwrap_err();
        assert!(error.contains("explicit confirmation"));
    }

    fn candidate(provider: MetadataProvider, url: &str) -> ArtworkCandidate {
        let source_url = format!("https://{}/source", expected_source_host(&provider));
        ArtworkCandidate {
            id: "poster-1".into(),
            provider,
            title: "Selected poster".into(),
            image_url: url.into(),
            thumbnail_url: url.into(),
            source_url,
            creator: None,
            license: Some("CC BY-SA".into()),
            license_url: Some("https://example.org/license".into()),
            attribution: Some("Example attribution".into()),
        }
    }

    #[test]
    fn only_provider_owned_https_image_hosts_are_accepted() {
        assert!(validate_candidate(&candidate(
            MetadataProvider::TvMaze,
            "https://static.tvmaze.com/images/poster.jpg"
        ))
        .is_ok());
        assert!(validate_candidate(&candidate(
            MetadataProvider::Tmdb,
            "https://image.tmdb.org/t/p/original/poster.jpg"
        ))
        .is_ok());
        assert!(validate_candidate(&candidate(
            MetadataProvider::WikimediaCommons,
            "https://upload.wikimedia.org/wikipedia/commons/poster.png"
        ))
        .is_ok());
        assert!(validate_candidate(&candidate(
            MetadataProvider::TvMaze,
            "https://evil.example/poster.jpg"
        ))
        .is_err());
        assert!(validate_candidate(&candidate(
            MetadataProvider::TvMaze,
            "http://static.tvmaze.com/poster.jpg"
        ))
        .is_err());
        assert!(validate_candidate(&candidate(
            MetadataProvider::Wikidata,
            "https://upload.wikimedia.org/poster.jpg"
        ))
        .is_err());
        assert!(validate_candidate(&candidate(
            MetadataProvider::TvMaze,
            "https://static.tvmaze.com/poster.svg"
        ))
        .is_err());
    }

    #[test]
    fn downloaded_image_type_is_verified_from_bytes() {
        let mut jpeg = vec![0; 128];
        jpeg[..3].copy_from_slice(&[0xff, 0xd8, 0xff]);
        assert_eq!(detect_image(&jpeg), Some(("jpg", "image/jpeg")));
        let mut png = vec![0; 128];
        png[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        assert_eq!(detect_image(&png), Some(("png", "image/png")));
        let mut webp = vec![0; 128];
        webp[..4].copy_from_slice(b"RIFF");
        webp[8..12].copy_from_slice(b"WEBP");
        assert_eq!(detect_image(&webp), Some(("webp", "image/webp")));
        assert_eq!(detect_image(b"<html>not artwork</html>"), None);
    }

    #[test]
    fn optional_credit_fields_are_bounded_and_safe_to_embed() {
        let mut value = candidate(
            MetadataProvider::TvMaze,
            "https://static.tvmaze.com/images/poster.jpg",
        );
        value.attribution = Some("Example\ncredit".into());
        normalize_credit_fields(&mut value).unwrap();
        assert_eq!(value.attribution.as_deref(), Some("Example credit"));
        assert!(validate_candidate(&value).is_ok());

        value.attribution = Some("Bad\0credit".into());
        assert!(normalize_credit_fields(&mut value).is_err());

        value.attribution = Some("A".repeat(4097));
        assert!(normalize_credit_fields(&mut value).is_err());

        value.attribution = Some("Example attribution".into());
        value.license_url = Some("javascript:alert(1)".into());
        assert!(validate_candidate(&value).is_err());

        value.license_url = Some("https://creativecommons.org/licenses/by/4.0/".into());
        assert!(validate_candidate(&value).is_ok());
    }

    #[test]
    fn artwork_targets_stay_inside_the_selected_root_and_are_unique() {
        let temp = std::env::temp_dir().join(format!(
            "hevc-artwork-test-{}",
            FILE_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let root = temp.join("library");
        let outside = temp.join("outside");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        let inside = root.join("Season 01");
        fs::create_dir_all(&inside).unwrap();
        assert!(validate_targets(
            root.to_str().unwrap(),
            &[ArtworkTarget {
                directory: inside.to_string_lossy().into_owned(),
                basename: "poster".into()
            }]
        )
        .is_ok());
        assert!(validate_targets(
            root.to_str().unwrap(),
            &[ArtworkTarget {
                directory: outside.to_string_lossy().into_owned(),
                basename: "poster".into()
            }]
        )
        .is_err());
        assert!(validate_targets(
            root.to_str().unwrap(),
            &[ArtworkTarget {
                directory: root.to_string_lossy().into_owned(),
                basename: "../poster".into()
            }]
        )
        .is_err());
        let duplicate = ArtworkTarget {
            directory: inside.to_string_lossy().into_owned(),
            basename: "poster".into(),
        };
        assert!(validate_targets(root.to_str().unwrap(), &[duplicate.clone(), duplicate]).is_err());
        fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn artwork_allows_server_specific_names_in_one_folder_but_rejects_normalized_duplicates() {
        let temp = std::env::temp_dir().join(format!(
            "hevc-artwork-names-test-{}",
            FILE_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let root = temp.join("library");
        fs::create_dir_all(&root).unwrap();

        assert!(validate_targets(
            root.to_str().unwrap(),
            &[
                ArtworkTarget {
                    directory: root.to_string_lossy().into_owned(),
                    basename: "folder".into(),
                },
                ArtworkTarget {
                    directory: root.to_string_lossy().into_owned(),
                    basename: "poster".into(),
                },
            ]
        )
        .is_ok());

        assert!(validate_targets(
            root.to_str().unwrap(),
            &[
                ArtworkTarget {
                    directory: root.to_string_lossy().into_owned(),
                    basename: "Season01".into(),
                },
                ArtworkTarget {
                    directory: root.to_string_lossy().into_owned(),
                    basename: "season01".into(),
                },
            ]
        )
        .is_err());

        fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn local_poster_copy_validates_image_and_refuses_to_overwrite() {
        let temp = std::env::temp_dir().join(format!(
            "hevc-local-poster-test-{}",
            FILE_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let root = temp.join("library");
        fs::create_dir_all(&root).unwrap();
        let source = temp.join("custom-poster.jpg");
        let mut bytes = vec![0; 128];
        bytes[..3].copy_from_slice(&[0xff, 0xd8, 0xff]);
        fs::write(&source, &bytes).unwrap();
        let request = LocalArtworkSaveRequest {
            root: root.to_string_lossy().into_owned(),
            source_path: source.to_string_lossy().into_owned(),
            targets: vec![ArtworkTarget {
                directory: root.to_string_lossy().into_owned(),
                basename: "film-poster".into(),
            }],
        };
        let result = save_local_artwork(request.clone()).unwrap();
        assert_eq!(
            result.files,
            vec![root.join("film-poster.jpg").to_string_lossy().into_owned()]
        );
        assert_eq!(fs::read(&result.files[0]).unwrap(), bytes);
        assert!(save_local_artwork(request).is_err());
        fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn local_poster_rejects_an_extension_that_disagrees_with_image_bytes() {
        let temp = std::env::temp_dir().join(format!(
            "hevc-local-poster-extension-test-{}",
            FILE_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let root = temp.join("library");
        fs::create_dir_all(&root).unwrap();
        let source = temp.join("custom-poster.png");
        let mut bytes = vec![0; 128];
        bytes[..3].copy_from_slice(&[0xff, 0xd8, 0xff]);
        fs::write(&source, &bytes).unwrap();
        let request = LocalArtworkSaveRequest {
            root: root.to_string_lossy().into_owned(),
            source_path: source.to_string_lossy().into_owned(),
            targets: vec![ArtworkTarget {
                directory: root.to_string_lossy().into_owned(),
                basename: "film-poster".into(),
            }],
        };

        let error = save_local_artwork(request).unwrap_err();
        assert!(error.contains("contains JPG image data"));
        assert!(!root.join("film-poster.jpg").exists());
        assert!(!root.join("film-poster.png").exists());
        fs::remove_dir_all(temp).unwrap();
    }
}
