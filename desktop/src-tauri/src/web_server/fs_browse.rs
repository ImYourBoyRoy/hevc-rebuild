//! Server-side folder listing behind the browser's replacement for the native file/folder pickers.
//! Read-only: it lists names, never contents.

use serde::Serialize;
use std::path::{Path, PathBuf};

const MAX_ENTRIES: usize = 5000;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DirectoryEntry {
    pub(crate) name: String,
    pub(crate) path: String,
    pub(crate) is_dir: bool,
    pub(crate) size: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DirectoryListing {
    pub(crate) path: String,
    pub(crate) parent: Option<String>,
    pub(crate) entries: Vec<DirectoryEntry>,
    pub(crate) truncated: bool,
}

fn start_directory(requested: Option<&str>) -> Result<PathBuf, String> {
    let requested = requested.map(str::trim).filter(|path| !path.is_empty());
    let path = match requested {
        Some(path) => PathBuf::from(path),
        None => PathBuf::from(crate::commands_workspace::default_media_root()?),
    };
    // A file path (a remembered default) lists its folder.
    let directory = if path.is_file() {
        path.parent().map(Path::to_path_buf).unwrap_or(path)
    } else {
        path
    };
    directory
        .canonicalize()
        .map_err(|error| format!("Could not open {}: {error}", directory.display()))
}

pub(crate) fn list_directory(
    path: Option<&str>,
    show_hidden: bool,
) -> Result<DirectoryListing, String> {
    let directory = start_directory(path)?;
    if !directory.is_dir() {
        return Err(format!("{} is not a folder.", directory.display()));
    }
    let reader = std::fs::read_dir(&directory)
        .map_err(|error| format!("Could not read {}: {error}", directory.display()))?;
    let mut entries = Vec::new();
    let mut truncated = false;
    for item in reader.flatten() {
        let name = item.file_name().to_string_lossy().into_owned();
        if !show_hidden && name.starts_with('.') {
            continue;
        }
        if entries.len() >= MAX_ENTRIES {
            truncated = true;
            break;
        }
        let metadata = item.metadata().ok();
        let is_dir = item.path().is_dir();
        entries.push(DirectoryEntry {
            name,
            path: item.path().to_string_lossy().into_owned(),
            is_dir,
            size: if is_dir {
                None
            } else {
                metadata.map(|metadata| metadata.len())
            },
        });
    }
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(DirectoryListing {
        path: directory.to_string_lossy().into_owned(),
        parent: directory
            .parent()
            .map(|parent| parent.to_string_lossy().into_owned()),
        entries,
        truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_folders_first_then_files_and_hides_dotfiles_by_default() {
        let root = crate::test_dir("web-browse");
        std::fs::create_dir(root.join("b-folder")).unwrap();
        std::fs::create_dir(root.join("A-folder")).unwrap();
        std::fs::write(root.join("movie.mkv"), b"12345").unwrap();
        std::fs::write(root.join(".hidden"), b"x").unwrap();
        let listing = list_directory(root.to_str(), false).unwrap();
        let names: Vec<_> = listing
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect();
        assert_eq!(names, ["A-folder", "b-folder", "movie.mkv"]);
        assert_eq!(listing.entries[2].size, Some(5));
        assert!(listing.entries[0].is_dir && listing.entries[0].size.is_none());
        assert_eq!(
            listing.parent.as_deref(),
            root.parent().and_then(Path::to_str)
        );
        let with_hidden = list_directory(root.to_str(), true).unwrap();
        assert!(with_hidden
            .entries
            .iter()
            .any(|entry| entry.name == ".hidden"));
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn a_file_path_lists_its_folder_and_a_missing_path_is_an_error() {
        let root = crate::test_dir("web-browse-file");
        let file = root.join("clip.mkv");
        std::fs::write(&file, b"x").unwrap();
        let listing = list_directory(file.to_str(), false).unwrap();
        assert_eq!(listing.path, root.to_string_lossy());
        assert!(list_directory(root.join("missing").to_str(), false).is_err());
        std::fs::remove_dir_all(root).ok();
    }
}
