//! No-overwrite file commit for validated media outputs.
//!
//! Callers decide when an output has passed all validation; this module owns the
//! destructive filesystem transition for runs started by the desktop, CLI, and
//! MCP interfaces.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Result of a successful install. The output is already in place; a warning
/// means a trailing durability step (the final directory sync) did not confirm.
/// That is not a failed commit: the validated output and any original backup
/// are both intact, so callers must not treat the file as an error output.
#[derive(Debug, Default, PartialEq, Eq)]
#[must_use = "an install can succeed with a durability warning that must be reported"]
pub(crate) struct InstallOutcome {
    pub(crate) warning: Option<String>,
}

impl InstallOutcome {
    /// Append the warning, when present, to a user-facing message.
    pub(crate) fn annotate(&self, message: String) -> String {
        match &self.warning {
            Some(warning) => format!("{message} Warning: {warning}"),
            None => message,
        }
    }
}

pub(crate) fn cli_mode(args: Vec<OsString>) -> i32 {
    let parsed = match parse_arguments(args) {
        Ok(parsed) => parsed,
        Err(error) => {
            eprintln!("{error}");
            return 2;
        }
    };
    match commit_output(
        &parsed.source,
        &parsed.temporary,
        &parsed.final_output,
        &parsed.backup,
    ) {
        Ok(outcome) => {
            if let Some(warning) = outcome.warning {
                eprintln!("{warning}");
            }
            0
        }
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

pub(crate) fn install_copy_cli_mode(args: Vec<OsString>) -> i32 {
    let parsed = match parse_install_copy_arguments(args) {
        Ok(parsed) => parsed,
        Err(error) => {
            eprintln!("{error}");
            return 2;
        }
    };
    match install_export_output(&parsed.temporary, &parsed.final_output) {
        Ok(outcome) => {
            if let Some(warning) = outcome.warning {
                eprintln!("{warning}");
            }
            0
        }
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

struct InstallCopyArguments {
    temporary: PathBuf,
    final_output: PathBuf,
}

fn parse_install_copy_arguments(args: Vec<OsString>) -> Result<InstallCopyArguments, String> {
    let mut temporary = None;
    let mut final_output = None;
    let mut args = args.into_iter();
    while let Some(option) = args.next() {
        let destination = match option.to_string_lossy().as_ref() {
            "--temporary" => &mut temporary,
            "--final" => &mut final_output,
            _ => return Err("Unknown internal install-output-copy option.".into()),
        };
        if destination.is_some() {
            return Err("Internal install-output-copy options cannot be repeated.".into());
        }
        *destination = Some(PathBuf::from(
            args.next()
                .ok_or("Internal install-output-copy option is missing its path.")?,
        ));
    }
    Ok(InstallCopyArguments {
        temporary: temporary.ok_or("Internal install-output-copy requires --temporary.")?,
        final_output: final_output.ok_or("Internal install-output-copy requires --final.")?,
    })
}

struct CommitArguments {
    source: PathBuf,
    temporary: PathBuf,
    final_output: PathBuf,
    backup: PathBuf,
}

fn parse_arguments(args: Vec<OsString>) -> Result<CommitArguments, String> {
    let mut source = None;
    let mut temporary = None;
    let mut final_output = None;
    let mut backup = None;
    let mut args = args.into_iter();
    while let Some(option) = args.next() {
        let destination = match option.to_string_lossy().as_ref() {
            "--source" => &mut source,
            "--temporary" => &mut temporary,
            "--final" => &mut final_output,
            "--backup" => &mut backup,
            _ => return Err("Unknown internal commit-output option.".into()),
        };
        if destination.is_some() {
            return Err("Internal commit-output options cannot be repeated.".into());
        }
        *destination = Some(PathBuf::from(
            args.next()
                .ok_or("Internal commit-output option is missing its path.")?,
        ));
    }
    Ok(CommitArguments {
        source: source.ok_or("Internal commit-output requires --source.")?,
        temporary: temporary.ok_or("Internal commit-output requires --temporary.")?,
        final_output: final_output.ok_or("Internal commit-output requires --final.")?,
        backup: backup.ok_or("Internal commit-output requires --backup.")?,
    })
}

pub(crate) fn commit_output(
    source: &Path,
    temporary: &Path,
    final_output: &Path,
    backup: &Path,
) -> Result<InstallOutcome, String> {
    commit_output_with_sync(source, temporary, final_output, backup, sync_directory)
}

fn commit_output_with_sync(
    source: &Path,
    temporary: &Path,
    final_output: &Path,
    backup: &Path,
    sync: impl Fn(&Path) -> io::Result<()>,
) -> Result<InstallOutcome, String> {
    let source_parent = source
        .parent()
        .ok_or("Source path has no parent directory.")?
        .canonicalize()
        .map_err(|error| format!("Could not resolve source directory: {error}"))?;
    let temporary_parent = temporary
        .parent()
        .ok_or("Temporary output path has no parent directory.")?
        .canonicalize()
        .map_err(|error| format!("Could not resolve temporary output directory: {error}"))?;
    let final_parent = final_output
        .parent()
        .ok_or("Final output path has no parent directory.")?
        .canonicalize()
        .map_err(|error| format!("Could not resolve final output directory: {error}"))?;
    if source_parent != temporary_parent || source_parent != final_parent {
        return Err("Source, temporary output, and final output must share one directory.".into());
    }
    if source == temporary || final_output == temporary || source == backup || temporary == backup {
        return Err("Commit paths overlap in an unsafe way.".into());
    }

    let source_metadata = fs::symlink_metadata(source)
        .map_err(|error| format!("Could not inspect source before commit: {error}"))?;
    if !source_metadata.file_type().is_file() && !source_metadata.file_type().is_symlink() {
        return Err("Source is not a regular file or file symlink.".into());
    }
    let temporary_metadata = fs::symlink_metadata(temporary)
        .map_err(|error| format!("Could not inspect temporary output before commit: {error}"))?;
    if !temporary_metadata.file_type().is_file() {
        return Err("Temporary output is not a regular file.".into());
    }

    let old_directory = crate::backup_folder::backup_dir(&source_parent);
    match fs::symlink_metadata(&old_directory) {
        Ok(metadata) if !metadata.is_dir() || metadata.file_type().is_symlink() => {
            return Err("The backup path must be a real directory, not a symlink.".into());
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir(&old_directory)
                .map_err(|error| format!("Could not create backup directory: {error}"))?;
            crate::backup_folder::mark_hidden(&old_directory);
            sync(&source_parent)
                .map_err(|error| format!("Could not sync source directory: {error}"))?;
        }
        Err(error) => return Err(format!("Could not inspect backup directory: {error}")),
    }
    let backup_parent = backup
        .parent()
        .ok_or("Backup path has no parent directory.")?
        .canonicalize()
        .map_err(|error| format!("Could not resolve backup directory: {error}"))?;
    let expected_backup_parent = old_directory
        .canonicalize()
        .map_err(|error| format!("Could not resolve backup directory: {error}"))?;
    if backup_parent != expected_backup_parent {
        return Err(
            "Backup directory resolves outside the source folder's backup directory.".into(),
        );
    }

    sync_file(temporary)
        .map_err(|error| format!("Could not make temporary output durable: {error}"))?;
    sync(&source_parent)
        .map_err(|error| format!("Could not sync output directory before commit: {error}"))?;
    if final_output != source && path_exists(final_output)? {
        return Err(format!(
            "Final output already exists and will not be overwritten: {}",
            final_output.display()
        ));
    }
    if path_exists(backup)? {
        return Err(format!(
            "Backup path already exists and will not be overwritten: {}",
            backup.display()
        ));
    }

    rename_no_replace(source, backup).map_err(|error| {
        format!(
            "Could not preserve the original at {}: {error}",
            backup.display()
        )
    })?;
    if let Err(error) = sync(&backup_parent) {
        return rollback_source(source, backup, error, &sync);
    }

    if let Err(error) = rename_no_replace(temporary, final_output) {
        return rollback_source(source, backup, error, &sync);
    }
    // The rename already succeeded, so the validated output is installed and the
    // original is safe in the backup. A failed directory sync only means the
    // rename's durability is unconfirmed; report it without failing the commit.
    Ok(InstallOutcome {
        warning: sync(&final_parent).err().map(|error| {
            format!(
                "Output was installed, but its directory could not be synced: {error}. The output and original backup were retained."
            )
        }),
    })
}

/// Install a validated export copy without moving or replacing its source.
/// The temporary file and final output must be on the same volume so the final
/// no-overwrite rename is atomic.
pub(crate) fn install_export_output(
    temporary: &Path,
    final_output: &Path,
) -> Result<InstallOutcome, String> {
    let temporary_parent = temporary
        .parent()
        .ok_or("Temporary export path has no parent directory.")?
        .canonicalize()
        .map_err(|error| format!("Could not resolve temporary export directory: {error}"))?;
    let final_parent = final_output
        .parent()
        .ok_or("Final export path has no parent directory.")?
        .canonicalize()
        .map_err(|error| format!("Could not resolve final export directory: {error}"))?;
    if temporary_parent != final_parent {
        return Err("Temporary export and final output must share one directory.".into());
    }
    let temporary_name = temporary
        .file_name()
        .ok_or("Temporary export path has no filename.")?;
    let final_name = final_output
        .file_name()
        .ok_or("Final export path has no filename.")?;
    if temporary_name == final_name {
        return Err("Temporary export and final output paths must be different.".into());
    }
    let metadata = fs::symlink_metadata(temporary)
        .map_err(|error| format!("Could not inspect temporary export before install: {error}"))?;
    if !metadata.file_type().is_file() {
        return Err("Temporary export is not a regular file.".into());
    }
    sync_file(temporary)
        .map_err(|error| format!("Could not make temporary export durable: {error}"))?;
    sync_directory(&temporary_parent)
        .map_err(|error| format!("Could not sync export directory before install: {error}"))?;
    if path_exists(final_output)? {
        return Err(format!(
            "Final export already exists and will not be overwritten: {}",
            final_output.display()
        ));
    }
    rename_no_replace(temporary, final_output).map_err(|error| {
        format!(
            "Could not install validated export at {} without overwriting an existing file: {error}",
            final_output.display()
        )
    })?;
    Ok(InstallOutcome {
        warning: sync_directory(&final_parent).err().map(|error| {
            format!(
                "Export was installed, but its directory could not be synced: {error}. The validated copy was retained at {}.",
                final_output.display()
            )
        }),
    })
}

fn rollback_source(
    source: &Path,
    backup: &Path,
    cause: io::Error,
    sync: &impl Fn(&Path) -> io::Result<()>,
) -> Result<InstallOutcome, String> {
    match rename_no_replace(backup, source) {
        Ok(()) => match source.parent().map(sync) {
            Some(Ok(())) => Err(format!(
                "Commit failed and the original was restored: {cause}"
            )),
            Some(Err(rollback_error)) => Err(format!(
                "Commit failed; the original was restored, but its directory could not be synced: {cause}; {rollback_error}"
            )),
            None => Err(format!(
                "Commit failed and the original was restored, but its directory could not be resolved: {cause}"
            )),
        },
        Err(rollback_error) => Err(format!(
            "Commit failed and automatic rollback could not restore the original. The original remains preserved at {}. Commit error: {cause}; rollback error: {rollback_error}",
            backup.display()
        )),
    }
}

fn path_exists(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!(
            "Could not inspect path {}: {error}",
            path.display()
        )),
    }
}

fn sync_file(path: &Path) -> io::Result<()> {
    // Windows' FlushFileBuffers needs write access, so a read-only handle fails
    // with "Access denied" and every output install would be refused. Unix fsync
    // works on a read-only handle, and opening for write there could fail on a
    // read-only file, so only Windows asks for write access.
    #[cfg(windows)]
    {
        std::fs::OpenOptions::new()
            .write(true)
            .open(path)?
            .sync_all()
    }
    #[cfg(not(windows))]
    {
        fs::File::open(path)?.sync_all()
    }
}

#[cfg(unix)]
pub(crate) fn sync_directory(path: &Path) -> io::Result<()> {
    fs::File::open(path)?.sync_all()
}

#[cfg(not(unix))]
pub(crate) fn sync_directory(path: &Path) -> io::Result<()> {
    // Windows cannot flush a directory handle (it will not even open one without
    // special flags) and NTFS journals directory metadata, so there is nothing to
    // confirm. The path must still be a real directory, so a wrong path is not
    // silently accepted.
    if fs::metadata(path)?.is_dir() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "path is not a directory",
        ))
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn rename_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    use std::os::unix::ffi::OsStrExt;

    let from = std::ffi::CString::new(from.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "source path contains NUL"))?;
    let to = std::ffi::CString::new(to.as_os_str().as_bytes()).map_err(|_| {
        io::Error::new(io::ErrorKind::InvalidInput, "destination path contains NUL")
    })?;
    // renameat2 with RENAME_NOREPLACE closes the check-then-rename overwrite race.
    let status = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            from.as_ptr(),
            libc::AT_FDCWD,
            to.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if status == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn rename_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let from = std::ffi::CString::new(from.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "source path contains NUL"))?;
    let to = std::ffi::CString::new(to.as_os_str().as_bytes()).map_err(|_| {
        io::Error::new(io::ErrorKind::InvalidInput, "destination path contains NUL")
    })?;
    // renamex_np with RENAME_EXCL fails if the destination already exists.
    let status = unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_EXCL) };
    if status == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(windows)]
pub(crate) fn rename_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MoveFileW(existing_file_name: *const u16, new_file_name: *const u16) -> i32;
    }

    let from = from
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let to = to
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    // MoveFileW fails when the destination already exists, unlike fs::rename.
    let status = unsafe { MoveFileW(from.as_ptr(), to.as_ptr()) };
    if status != 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub(crate) fn rename_no_replace(_from: &Path, _to: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "atomic no-overwrite rename is not implemented on this platform",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_directory() -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock is after epoch")
            .as_nanos();
        let path = crate::test_temp_base().join(format!(
            "hevc-rebuild-export-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("create isolated export transaction directory");
        path
    }

    #[test]
    fn installs_validated_export_without_replacing_an_existing_file() {
        let directory = test_directory();
        let temporary = directory.join("temporary.mp4");
        let final_output = directory.join("final.mp4");
        fs::write(&temporary, b"validated output").expect("write temporary export");
        let outcome = install_export_output(&temporary, &final_output).expect("install export");
        assert_eq!(outcome, InstallOutcome::default());
        assert!(!temporary.exists());
        assert_eq!(
            fs::read(&final_output).expect("read final output"),
            b"validated output"
        );

        let next_temporary = directory.join("next.mp4");
        fs::write(&next_temporary, b"different output").expect("write second temporary export");
        assert!(install_export_output(&next_temporary, &final_output).is_err());
        assert!(
            next_temporary.exists(),
            "failed no-overwrite install keeps temporary output"
        );
        assert_eq!(
            fs::read(&final_output).expect("read original final output"),
            b"validated output"
        );
        fs::remove_dir_all(directory).expect("remove test directory");
    }

    #[test]
    fn commit_sync_failures_preserve_original_and_report_installed_state() {
        use std::cell::Cell;
        for fail_at in 1..=4 {
            let directory = test_directory();
            let source = directory.join("source.mkv");
            let temporary = directory.join("temporary.mkv");
            let backup = crate::backup_folder::backup_dir(&directory).join("original.mkv");
            fs::write(&source, b"original").unwrap();
            fs::write(&temporary, b"validated").unwrap();
            let count = Cell::new(0);
            let result = commit_output_with_sync(&source, &temporary, &source, &backup, |_| {
                count.set(count.get() + 1);
                if count.get() == fail_at {
                    Err(io::Error::other("injected sync failure"))
                } else {
                    Ok(())
                }
            });
            if fail_at == 4 {
                // The rename succeeded: the commit is a success with a warning,
                // never an error that would make callers quarantine the output.
                let outcome = result.expect("a post-install sync failure is not a failed commit");
                assert!(outcome.warning.unwrap().contains("Output was installed"));
                assert_eq!(fs::read(&source).unwrap(), b"validated");
                assert_eq!(fs::read(&backup).unwrap(), b"original");
            } else {
                result.unwrap_err();
                assert_eq!(fs::read(&source).unwrap(), b"original");
                assert_eq!(fs::read(&temporary).unwrap(), b"validated");
            }
            fs::remove_dir_all(directory).unwrap();
        }
    }

    #[test]
    fn competing_final_or_rollback_creator_cannot_be_overwritten() {
        for block_rollback in [false, true] {
            let directory = test_directory();
            let source = directory.join("source.mkv");
            let temporary = directory.join("temporary.mkv");
            let final_output = directory.join("final.mkv");
            let backup = crate::backup_folder::backup_dir(&directory).join("original.mkv");
            fs::write(&source, b"original").unwrap();
            fs::write(&temporary, b"validated").unwrap();
            let error =
                commit_output_with_sync(&source, &temporary, &final_output, &backup, |path| {
                    if path == crate::backup_folder::backup_dir(&directory) {
                        fs::write(&final_output, b"competitor").unwrap();
                        if block_rollback {
                            fs::write(&source, b"new source occupant").unwrap();
                        }
                    }
                    Ok(())
                })
                .unwrap_err();
            assert_eq!(fs::read(&final_output).unwrap(), b"competitor");
            assert_eq!(fs::read(&temporary).unwrap(), b"validated");
            if block_rollback {
                assert!(error.contains("original remains preserved"));
                assert_eq!(fs::read(&backup).unwrap(), b"original");
                assert_eq!(fs::read(&source).unwrap(), b"new source occupant");
            } else {
                assert!(error.contains("original was restored"));
                assert_eq!(fs::read(&source).unwrap(), b"original");
            }
            fs::remove_dir_all(directory).unwrap();
        }
    }

    #[test]
    fn no_replace_and_directory_sync_reject_occupied_and_missing_paths() {
        let directory = test_directory();
        let source = directory.join("source");
        let target = directory.join("target");
        fs::write(&source, b"original").unwrap();
        fs::write(&target, b"competitor").unwrap();
        assert!(rename_no_replace(&source, &target).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"competitor");
        assert!(sync_directory(&directory.join("missing")).is_err());
        fs::remove_dir_all(directory).unwrap();
    }
}
