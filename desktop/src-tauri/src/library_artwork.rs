use crate::library_metadata::{ArtworkCandidate, MetadataProvider};
use reqwest::{redirect, Client, Url};
use rmcp::schemars;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs::{self, Metadata, OpenOptions};
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, UNIX_EPOCH};
use unicode_normalization::UnicodeNormalization;

mod cache;
mod types;
mod validate;
pub(crate) use validate::image_hosts as artwork_image_hosts;

use cache::*;
pub(crate) use cache::{
    app_artwork_cache_root, inspect_app_artwork_cache, purge_unused_app_artwork_cache,
    ArtworkCacheInfo, ArtworkCachePurgeResult,
};
pub(crate) use types::{
    ArtworkSaveRequest, ArtworkSaveResult, ArtworkTarget, LocalArtworkSaveRequest,
    LocalArtworkSaveResult, SavedArtworkFile,
};
use validate::*;

const MAX_ARTWORK_BYTES: usize = 16 * 1024 * 1024;
const MAX_ARTWORK_TARGETS: usize = 128;
const MAX_BATCH_BYTES: usize = 512 * 1024 * 1024;
const MAX_ATTRIBUTION_BYTES: usize = 16 * 1024;
static FILE_COUNTER: AtomicU64 = AtomicU64::new(1);

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
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
mod tests;
