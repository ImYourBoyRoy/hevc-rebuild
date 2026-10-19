//! Private per-job safe-stop marker; no predictable shared /tmp file is opened.
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) struct StopToken {
    path: PathBuf,
}
impl StopToken {
    pub(crate) fn new() -> Result<Self, String> {
        for attempt in 0..100 {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let parent = std::env::temp_dir().join(format!(
                "hevc-rebuild-control-{}-{nonce}-{attempt}",
                std::process::id()
            ));
            let mut builder = fs::DirBuilder::new();
            #[cfg(unix)]
            builder.mode(0o700);
            match builder.create(&parent) {
                Ok(()) => {
                    return Ok(Self {
                        path: parent.join("stop"),
                    })
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(format!("Could not allocate private stop controls: {error}"))
                }
            }
        }
        Err("Could not allocate a unique stop control directory.".into())
    }
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}
impl Drop for StopToken {
    fn drop(&mut self) {
        // Never recursively remove a path; unexpected contents remain for inspection.
        let _ = fs::remove_file(&self.path);
        if let Some(parent) = self.path.parent() {
            let _ = fs::remove_dir(parent);
        }
    }
}
fn validate_parent(path: &Path) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("Stop path has no parent"))?;
    let metadata = fs::symlink_metadata(parent)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(io::Error::other(
            "Stop controls require a real private directory",
        ));
    }
    #[cfg(unix)]
    if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0 {
        return Err(io::Error::other(
            "Stop control directory ownership or permissions changed",
        ));
    }
    Ok(())
}
pub(crate) fn requested(path: &Path) -> bool {
    if validate_parent(path).is_err() {
        return false;
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    let Ok(file) = options.open(path) else {
        return false;
    };
    let Ok(metadata) = file.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0 {
        return false;
    }
    let Ok(current) = fs::symlink_metadata(path) else {
        return false;
    };
    #[cfg(unix)]
    if metadata.dev() != current.dev() || metadata.ino() != current.ino() {
        return false;
    }
    !current.file_type().is_symlink()
}
pub(crate) fn request(path: &Path) -> Result<(), String> {
    validate_parent(path).map_err(|error| error.to_string())?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    match options.open(path) {
        Ok(mut file) => file
            .write_all(b"finish current file, then stop\n")
            .map_err(|error| error.to_string()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists && requested(path) => Ok(()),
        Err(error) => Err(format!("Could not request a safe stop: {error}")),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unique_jobs_normal_cancellation_idempotence_and_cleanup() {
        let first = StopToken::new().unwrap();
        let second = StopToken::new().unwrap();
        let path = first.path().to_path_buf();
        let parent = path.parent().unwrap().to_path_buf();
        assert_ne!(first.path(), second.path());
        assert!(!requested(&path));
        request(&path).unwrap();
        request(&path).unwrap();
        assert!(requested(&path));
        assert!(!requested(second.path()));
        drop(first);
        assert!(!parent.exists());
    }
    #[cfg(unix)]
    #[test]
    fn symlink_and_replaced_marker_never_truncate_another_file() {
        use std::os::unix::fs::symlink;
        let token = StopToken::new().unwrap();
        let victim = token.path().parent().unwrap().join("victim");
        fs::write(&victim, b"untouched").unwrap();
        symlink(&victim, token.path()).unwrap();
        assert!(!requested(token.path()));
        assert!(request(token.path()).is_err());
        assert_eq!(fs::read(&victim).unwrap(), b"untouched");
        fs::remove_file(token.path()).unwrap();
        request(token.path()).unwrap();
        fs::remove_file(token.path()).unwrap();
        symlink(&victim, token.path()).unwrap();
        assert!(!requested(token.path()));
        assert!(request(token.path()).is_err());
        fs::remove_file(victim).unwrap();
    }
}
