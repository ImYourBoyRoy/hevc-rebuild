//! The private app artwork cache: location, inspection, snapshot and the reviewed purge of unused files.

use super::*;

pub(super) static ARTWORK_CACHE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub(crate) fn app_artwork_cache_root() -> Result<PathBuf, String> {
    let preferred = crate::app_cache_directory()?.join("artwork");
    if preferred.exists() {
        return Ok(preferred);
    }
    // Preserve artwork cached by earlier builds, which stored it below the
    // settings profile. New writes use the native cache directory above.
    let legacy = crate::app_preferences_file()?
        .parent()
        .map(|profile| profile.join(".cache/artwork"));
    if let Some(legacy) = legacy.filter(|path| path.exists()) {
        return Ok(legacy);
    }
    Ok(preferred)
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
pub(super) struct CacheFileSnapshot {
    path: PathBuf,
    name: String,
    bytes: u64,
    modified_ns: u128,
}

#[derive(Clone)]
pub(super) struct CachedArtworkSnapshot {
    image: CacheFileSnapshot,
    attribution: Option<CacheFileSnapshot>,
    protected: bool,
}

pub(super) struct ArtworkCacheSnapshot {
    root: Option<PathBuf>,
    pub(super) info: ArtworkCacheInfo,
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

pub(super) fn canonical_cache_root() -> Result<Option<PathBuf>, String> {
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

pub(super) fn build_artwork_cache_snapshot_at(
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

pub(super) fn purge_artwork_cache_snapshot(
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

pub(super) fn prepare_private_artwork_cache() -> Result<PathBuf, String> {
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

pub(super) fn make_cached_files_private(files: &[SavedArtworkFile]) -> Result<(), String> {
    #[cfg(not(unix))]
    let _ = files;
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
