//! Where the originals go after a rebuild.
//!
//! A validated output replaces each original in place, and the original moves
//! into a hidden folder beside it. The folder is hidden so media servers and
//! file browsers do not list the backups as library content, and so it cannot be
//! mistaken for a user's own folder named `old`. Libraries rebuilt by earlier
//! versions used a plain `old/` folder: those backups are still recognized (a
//! file with a backup there is skipped, and the folder is excluded from
//! discovery), but new backups only go to the hidden folder. Nothing is moved
//! or deleted automatically.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// The hidden folder that receives each original after its rebuild is validated.
pub(crate) const BACKUP_FOLDER: &str = ".hevc-rebuild-originals";

/// The visible folder earlier versions used. Read, never written.
pub(crate) const LEGACY_BACKUP_FOLDER: &str = "old";

/// The folder that receives new backups for files in `parent`.
pub(crate) fn backup_dir(parent: &Path) -> PathBuf {
    parent.join(BACKUP_FOLDER)
}

/// True when `parent/<name>` already has a backup in the current or the legacy
/// folder, meaning that file was rebuilt before and must not be rebuilt again.
pub(crate) fn has_backup(parent: &Path, name: &OsStr) -> bool {
    parent.join(BACKUP_FOLDER).join(name).exists()
        || parent.join(LEGACY_BACKUP_FOLDER).join(name).exists()
}

/// Exclusions applied when a caller names none. The current backup folder is
/// always skipped by discovery itself; this list covers the legacy `old/`
/// folder so a command-line scan or rebuild of an older library does not treat
/// its backups as new work.
pub(crate) fn default_exclusions() -> Vec<String> {
    vec![LEGACY_BACKUP_FOLDER.to_owned()]
}

/// Fills in the default exclusions unless the caller made its own choice, either
/// by listing patterns or by passing `--no-default-exclusions` (the desktop app
/// always does: its list is an explicit preference and may be empty).
pub(crate) fn apply_default_exclusions(exclude: &mut Vec<String>, explicit: bool) {
    if exclude.is_empty() && !explicit {
        *exclude = default_exclusions();
    }
}

/// Marks a just-created backup folder hidden where a leading dot is not enough.
/// Windows hides by attribute, not by name; macOS and Linux already hide the
/// dot-folder. Best effort: an unhidden backup folder is only less tidy, so a
/// failure here must never fail a rebuild.
pub(crate) fn mark_hidden(directory: &Path) {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        extern "system" {
            fn GetFileAttributesW(path: *const u16) -> u32;
            fn SetFileAttributesW(path: *const u16, attributes: u32) -> i32;
        }
        const INVALID_FILE_ATTRIBUTES: u32 = u32::MAX;
        const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
        let wide: Vec<u16> = directory
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        // SAFETY: `wide` is NUL-terminated and outlives both calls.
        unsafe {
            let current = GetFileAttributesW(wide.as_ptr());
            if current != INVALID_FILE_ATTRIBUTES {
                SetFileAttributesW(wide.as_ptr(), current | FILE_ATTRIBUTE_HIDDEN);
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = directory;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_backups_go_to_the_hidden_folder_beside_the_file() {
        let dir = backup_dir(Path::new("/media/Movies"));
        assert_eq!(dir, Path::new("/media/Movies/.hevc-rebuild-originals"));
        assert!(
            dir.file_name().unwrap().to_string_lossy().starts_with('.'),
            "the folder must be hidden by name on macOS and Linux"
        );
    }

    #[test]
    fn a_backup_in_either_folder_marks_the_file_as_already_rebuilt() {
        let root = crate::test_dir("backup-folder");
        let name = OsStr::new("Movie.mkv");
        assert!(!has_backup(&root, name));

        std::fs::create_dir(root.join(LEGACY_BACKUP_FOLDER)).unwrap();
        std::fs::write(root.join(LEGACY_BACKUP_FOLDER).join(name), b"x").unwrap();
        assert!(
            has_backup(&root, name),
            "a library rebuilt by an older version"
        );
        assert!(!has_backup(&root, OsStr::new("Other.mkv")));

        std::fs::create_dir(root.join(BACKUP_FOLDER)).unwrap();
        std::fs::write(root.join(BACKUP_FOLDER).join("New.mkv"), b"x").unwrap();
        assert!(has_backup(&root, OsStr::new("New.mkv")));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn exclusions_default_to_the_legacy_folder_unless_the_caller_chose() {
        let mut none = Vec::new();
        apply_default_exclusions(&mut none, false);
        assert_eq!(none, ["old"]);

        let mut opted_out = Vec::new();
        apply_default_exclusions(&mut opted_out, true);
        assert!(opted_out.is_empty(), "an explicit empty list stays empty");

        let mut own = vec!["extras".to_string()];
        apply_default_exclusions(&mut own, false);
        assert_eq!(own, ["extras"], "the caller's own list wins");
    }

    #[test]
    fn hiding_a_folder_never_fails_or_removes_it() {
        let root = crate::test_dir("backup-hide");
        let folder = backup_dir(&root);
        std::fs::create_dir(&folder).unwrap();
        mark_hidden(&folder);
        mark_hidden(&root.join("does-not-exist"));
        assert!(folder.is_dir());
        let _ = std::fs::remove_dir_all(root);
    }
}
