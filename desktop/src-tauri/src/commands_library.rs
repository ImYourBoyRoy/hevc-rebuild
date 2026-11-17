//! Tauri commands for library artwork, local asset scans, folder organizing and file reveal.

use super::*;

#[cfg_attr(feature = "desktop", tauri::command)]
#[cfg_attr(not(feature = "desktop"), allow(dead_code))]
pub(crate) fn open_file_location(path: String) -> Result<(), String> {
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

#[cfg_attr(feature = "desktop", tauri::command)]
pub(crate) async fn save_library_artwork(
    request: library_artwork::ArtworkSaveRequest,
) -> Result<library_artwork::ArtworkSaveResult, String> {
    library_artwork::save_artwork(request.candidate, request.root, request.targets).await
}

#[cfg_attr(feature = "desktop", tauri::command)]
pub(crate) async fn cache_library_artwork(
    candidate: library_metadata::ArtworkCandidate,
) -> Result<library_artwork::ArtworkSaveResult, String> {
    library_artwork::save_artwork_to_cache(candidate).await
}

#[cfg_attr(feature = "desktop", tauri::command)]
pub(crate) fn save_local_library_artwork(
    request: library_artwork::LocalArtworkSaveRequest,
) -> Result<library_artwork::LocalArtworkSaveResult, String> {
    library_artwork::save_local_artwork(request)
}

#[cfg_attr(feature = "desktop", tauri::command)]
pub(crate) async fn cache_local_library_artwork(
    source_path: String,
) -> Result<library_artwork::LocalArtworkSaveResult, String> {
    library_artwork::save_local_artwork_to_cache(source_path).await
}

#[cfg_attr(feature = "desktop", tauri::command)]
pub(crate) async fn inspect_app_artwork_cache() -> Result<library_artwork::ArtworkCacheInfo, String>
{
    library_artwork::inspect_app_artwork_cache().await
}

#[cfg(feature = "desktop")]
#[tauri::command]
pub(crate) async fn purge_unused_app_artwork_cache(
    state: State<'_, RunState>,
    preview_id: String,
    confirmed: bool,
) -> Result<library_artwork::ArtworkCachePurgeResult, String> {
    purge_unused_app_artwork_cache_with(state.inner(), preview_id, confirmed).await
}

/// Runtime-free body of `purge_unused_app_artwork_cache`, shared with the headless server.
pub(crate) async fn purge_unused_app_artwork_cache_with(
    state: &RunState,
    preview_id: String,
    confirmed: bool,
) -> Result<library_artwork::ArtworkCachePurgeResult, String> {
    let id = new_job_id();
    claim_job(
        state,
        new_active_job_of(id.clone(), None, "artwork_cache_purge"),
    )?;
    let result = library_artwork::purge_unused_app_artwork_cache(&preview_id, confirmed).await;
    release_job(state, &id);
    result
}

#[cfg_attr(feature = "desktop", tauri::command)]
pub(crate) fn scan_local_library_assets(
    request: library_assets::LocalAssetsRequest,
) -> Result<library_assets::LocalAssetsScan, String> {
    library_assets::scan_local_assets(request)
}

#[cfg_attr(feature = "desktop", tauri::command)]
pub(crate) fn read_local_library_artwork_preview(
    request: library_assets::ArtworkPreviewRequest,
) -> Result<library_assets::ArtworkPreview, String> {
    library_assets::read_artwork_preview(request)
}

#[cfg_attr(feature = "desktop", tauri::command)]
pub(crate) fn scan_organizer_folder(
    root: String,
) -> Result<library_organizer::OrganizerFolderScan, String> {
    library_organizer::scan_organizer_folder(root)
}

#[cfg_attr(feature = "desktop", tauri::command)]
pub(crate) fn preview_library_rename(
    root: String,
    items: Vec<library_organizer::RenameTemplateInput>,
    text_blacklist_patterns: Vec<String>,
) -> Result<library_organizer::RenamePreview, String> {
    library_organizer::preview_rename_batch_with_patterns(root, items, text_blacklist_patterns)
}

#[cfg_attr(feature = "desktop", tauri::command)]
pub(crate) fn apply_library_rename(
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
