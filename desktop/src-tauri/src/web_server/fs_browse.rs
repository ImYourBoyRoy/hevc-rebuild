//! Server-side folder listing behind the browser's replacement for the native file/folder pickers.
//! Read-only: it lists names, never contents.

use super::scope::PathScope;
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

fn start_directory(requested: Option<&str>, scope: &PathScope) -> Result<PathBuf, String> {
    let requested = requested.map(str::trim).filter(|path| !path.is_empty());
    let path = match requested {
        Some(path) => PathBuf::from(path),
        None => match scope.first_root() {
            Some(root) => root.to_path_buf(),
            None => PathBuf::from(crate::commands_workspace::default_media_root()?),
        },
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

/// A path as people should see it. Windows canonicalisation returns verbatim paths (`\\?\C:\...`); these
/// are shown as ordinary drive or UNC paths, which Windows accepts back unchanged.
pub(crate) fn display_path(path: &str) -> String {
    if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{rest}");
    }
    if let Some(rest) = path.strip_prefix(r"\\?\") {
        let mut chars = rest.chars();
        if let (Some(letter), Some(':')) = (chars.next(), chars.next()) {
            if letter.is_ascii_alphabetic() {
                return rest.to_string();
            }
        }
    }
    path.to_string()
}

pub(crate) fn list_directory(
    path: Option<&str>,
    show_hidden: bool,
    scope: &PathScope,
) -> Result<DirectoryListing, String> {
    let directory = start_directory(path, scope)?;
    scope.check(&directory.to_string_lossy())?;
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
        if !scope.contains(&item.path()) {
            continue; // a symlink that leads outside the root
        }
        let metadata = item.metadata().ok();
        let is_dir = item.path().is_dir();
        entries.push(DirectoryEntry {
            name,
            path: display_path(&item.path().to_string_lossy()),
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
        path: display_path(&directory.to_string_lossy()),
        parent: directory
            .parent()
            .filter(|parent| scope.contains(parent))
            .map(|parent| display_path(&parent.to_string_lossy())),
        entries,
        truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_verbatim_prefixes_are_not_shown_to_people() {
        assert_eq!(display_path(r"\\?\C:\Media\Films"), r"C:\Media\Films");
        assert_eq!(display_path(r"\\?\C:\"), r"C:\");
        assert_eq!(
            display_path(r"\\?\UNC\server\share\films"),
            r"\\server\share\films"
        );
        assert_eq!(display_path("/srv/media"), "/srv/media");
        assert_eq!(display_path(r"C:\Media"), r"C:\Media");
        assert_eq!(
            display_path(r"\\?\Volume{1234}\x"),
            r"\\?\Volume{1234}\x",
            "other verbatim forms are left alone"
        );
    }

    #[test]
    fn lists_folders_first_then_files_and_hides_dotfiles_by_default() {
        let root = crate::test_dir("web-browse");
        std::fs::create_dir(root.join("b-folder")).unwrap();
        std::fs::create_dir(root.join("A-folder")).unwrap();
        std::fs::write(root.join("movie.mkv"), b"12345").unwrap();
        std::fs::write(root.join(".hidden"), b"x").unwrap();
        let listing = list_directory(root.to_str(), false, &PathScope::default()).unwrap();
        let names: Vec<_> = listing
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect();
        assert_eq!(names, ["A-folder", "b-folder", "movie.mkv"]);
        assert_eq!(listing.entries[2].size, Some(5));
        assert!(listing.entries[0].is_dir && listing.entries[0].size.is_none());
        // Windows temp paths can be verbatim (`\\?\C:\...`); the listing shows the ordinary form.
        let expected_parent = root.parent().and_then(Path::to_str).map(display_path);
        assert_eq!(listing.parent, expected_parent);
        let with_hidden = list_directory(root.to_str(), true, &PathScope::default()).unwrap();
        assert!(with_hidden
            .entries
            .iter()
            .any(|entry| entry.name == ".hidden"));
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn a_scoped_browser_starts_in_the_first_root_and_cannot_climb_out_of_it() {
        let base = crate::test_dir("web-browse-scope");
        let media = base.join("media");
        std::fs::create_dir_all(media.join("films")).unwrap();
        std::fs::create_dir_all(base.join("secret")).unwrap();
        let scope = PathScope::new(std::slice::from_ref(&media)).unwrap();
        let start = list_directory(None, false, &scope).expect("starts in the root");
        let shown_root = display_path(&media.canonicalize().unwrap().to_string_lossy());
        assert_eq!(start.path, shown_root);
        assert_eq!(start.parent, None, "the root has no way up");
        let films = list_directory(media.join("films").to_str(), false, &scope).unwrap();
        assert_eq!(
            films.parent.as_deref(),
            Some(shown_root.as_str()),
            "inside the root, up works"
        );
        assert!(list_directory(base.join("secret").to_str(), false, &scope)
            .unwrap_err()
            .contains("--root"));
        assert!(
            list_directory(base.to_str(), false, &scope).is_err(),
            "the parent of the root is out of bounds"
        );
        std::fs::remove_dir_all(base).ok();
    }

    #[cfg(unix)]
    #[test]
    fn a_scoped_listing_hides_symlinks_that_lead_outside() {
        let base = crate::test_dir("web-browse-link");
        let media = base.join("media");
        std::fs::create_dir_all(&media).unwrap();
        std::fs::create_dir_all(base.join("secret")).unwrap();
        std::fs::create_dir_all(media.join("real")).unwrap();
        std::os::unix::fs::symlink(base.join("secret"), media.join("escape")).unwrap();
        let scope = PathScope::new(std::slice::from_ref(&media)).unwrap();
        let listing = list_directory(media.to_str(), false, &scope).unwrap();
        let names: Vec<_> = listing
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect();
        assert_eq!(names, ["real"]);
        std::fs::remove_dir_all(base).ok();
    }

    #[test]
    fn a_file_path_lists_its_folder_and_a_missing_path_is_an_error() {
        let root = crate::test_dir("web-browse-file");
        let file = root.join("clip.mkv");
        std::fs::write(&file, b"x").unwrap();
        let listing = list_directory(file.to_str(), false, &PathScope::default()).unwrap();
        assert_eq!(listing.path, display_path(&root.to_string_lossy()));
        assert!(
            list_directory(root.join("missing").to_str(), false, &PathScope::default()).is_err()
        );
        std::fs::remove_dir_all(root).ok();
    }
}
