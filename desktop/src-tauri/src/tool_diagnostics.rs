//! Private, persistent diagnostics for reviewed toolchain setup operations.

use serde::Serialize;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_LOG_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ToolSetupLogSummary {
    pub(crate) job_id: String,
    pub(crate) modified_unix: u64,
    pub(crate) size_bytes: u64,
    pub(crate) truncated: bool,
}

/// One append-only log for one setup transaction. Logging failures never block
/// a user-confirmed package operation; the UI can display the returned warning.
#[derive(Clone)]
pub(crate) struct ToolSetupLogger {
    inner: Arc<ToolSetupLoggerInner>,
}

struct ToolSetupLoggerInner {
    file: Mutex<Option<File>>,
    warning: Mutex<Option<String>>,
    truncated: AtomicBool,
}

impl ToolSetupLogger {
    pub(crate) fn unavailable(reason: String) -> Self {
        let logger = Self {
            inner: Arc::new(ToolSetupLoggerInner {
                file: Mutex::new(None),
                warning: Mutex::new(None),
                truncated: AtomicBool::new(false),
            }),
        };
        logger.set_warning(reason);
        logger
    }

    pub(crate) fn create(directory: &Path, job_id: &str, operation: &str) -> Self {
        let logger = Self {
            inner: Arc::new(ToolSetupLoggerInner {
                file: Mutex::new(None),
                warning: Mutex::new(None),
                truncated: AtomicBool::new(false),
            }),
        };

        if let Err(error) = logger.create_file(directory, job_id, operation) {
            logger.set_warning(format!(
                "Persistent setup log could not be created: {error}"
            ));
        }
        logger
    }

    fn create_file(&self, directory: &Path, job_id: &str, operation: &str) -> Result<(), String> {
        let path = log_path(directory, job_id)?;
        fs::create_dir_all(directory)
            .map_err(|error| format!("Could not create the setup log folder: {error}"))?;
        set_private_directory(directory)?;

        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        set_private_file(&mut options);
        let mut file = options
            .open(&path)
            .map_err(|error| format!("Could not create {}: {error}", path.display()))?;
        writeln!(file, "HEVC Rebuild Studio tool setup")
            .and_then(|_| writeln!(file, "Operation: {}", clean_line(operation)))
            .and_then(|_| writeln!(file, "Job: {job_id}"))
            .and_then(|_| writeln!(file, "Started (Unix): {}", now_unix()))
            .and_then(|_| writeln!(file))
            .and_then(|_| file.sync_all())
            .map_err(|error| format!("Could not write the setup log header: {error}"))?;
        *self
            .inner
            .file
            .lock()
            .map_err(|_| "Could not lock the setup log file.")? = Some(file);
        Ok(())
    }

    /// Returns false when the safe per-operation size limit has been reached.
    pub(crate) fn append(&self, line: &str) -> Result<bool, String> {
        if self.inner.truncated.load(Ordering::Relaxed) {
            return Ok(false);
        }
        let mut slot = self
            .inner
            .file
            .lock()
            .map_err(|_| "Could not lock the setup log file.")?;
        let Some(file) = slot.as_mut() else {
            return Err(self
                .warning()
                .unwrap_or_else(|| "Persistent setup log is unavailable.".into()));
        };
        let clean = clean_line(line);
        let bytes = clean.as_bytes();
        let current = file
            .metadata()
            .map_err(|error| format!("Could not inspect the setup log: {error}"))?
            .len();
        if current.saturating_add(bytes.len() as u64).saturating_add(1) > MAX_LOG_BYTES {
            let marker = "[Log reached the 16 MiB per-operation limit; later output is available only in the live session.]\n";
            let _ = file.write_all(marker.as_bytes());
            let _ = file.flush().and_then(|_| file.sync_data());
            self.inner.truncated.store(true, Ordering::Relaxed);
            return Ok(false);
        }
        file.write_all(bytes)
            .and_then(|_| file.write_all(b"\n"))
            .and_then(|_| file.flush())
            .and_then(|_| file.sync_data())
            .map_err(|error| format!("Could not append to the setup log: {error}"))?;
        Ok(true)
    }

    pub(crate) fn record(&self, line: &str) {
        match self.append(line) {
            Ok(true) => {}
            Ok(false) => self.set_warning(
                "Persistent setup log reached its 16 MiB limit; later output was not saved.".into(),
            ),
            Err(error) => self.set_warning(error),
        }
    }

    pub(crate) fn finish(&self, success: bool, error: Option<&str>) {
        let result = if success { "Completed" } else { "Failed" };
        let detail = error
            .map(|value| format!(" · {}", clean_line(value)))
            .unwrap_or_default();
        self.record(&format!("\n{result} (Unix): {}{detail}", now_unix()));
    }

    pub(crate) fn warning(&self) -> Option<String> {
        self.inner
            .warning
            .lock()
            .ok()
            .and_then(|warning| warning.clone())
    }

    fn set_warning(&self, message: String) {
        if let Ok(mut warning) = self.inner.warning.lock() {
            if warning.is_none() {
                *warning = Some(message);
            }
        }
    }
}

pub(crate) fn list(directory: &Path) -> Result<Vec<ToolSetupLogSummary>, String> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("Could not list setup logs: {error}")),
    };
    let mut logs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let Some(job_id) = file_name
            .strip_prefix("tool-setup-")
            .and_then(|name| name.strip_suffix(".log"))
        else {
            continue;
        };
        if validate_job_id(job_id).is_err() {
            continue;
        }
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_file() => metadata,
            _ => continue,
        };
        let modified_unix = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs())
            .unwrap_or_default();
        logs.push(ToolSetupLogSummary {
            job_id: job_id.to_string(),
            modified_unix,
            size_bytes: metadata.len(),
            truncated: metadata.len() >= MAX_LOG_BYTES,
        });
    }
    logs.sort_by(|left, right| right.modified_unix.cmp(&left.modified_unix));
    Ok(logs)
}

pub(crate) fn read(directory: &Path, job_id: &str) -> Result<String, String> {
    let path = log_path(directory, job_id)?;
    let metadata = fs::symlink_metadata(&path)
        .map_err(|error| format!("Could not open this setup log: {error}"))?;
    if !metadata.file_type().is_file() {
        return Err("Setup log is not a regular file.".into());
    }
    if metadata.len() > MAX_LOG_BYTES + 256 {
        return Err("Setup log exceeds the supported viewing size.".into());
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    File::open(&path)
        .and_then(|mut file| file.read_to_end(&mut bytes))
        .map_err(|error| format!("Could not read the setup log: {error}"))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

pub(crate) fn export(directory: &Path, job_id: &str, destination: &Path) -> Result<String, String> {
    let content = read(directory, job_id)?;
    if !destination.is_absolute() {
        return Err("Choose an absolute destination for the exported log.".into());
    }
    if !matches!(
        destination
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("log" | "txt")
    ) {
        return Err("Setup logs can be exported as .log or .txt files.".into());
    }
    let parent = destination
        .parent()
        .ok_or("Choose a destination inside an existing folder.")?
        .canonicalize()
        .map_err(|error| format!("Could not open the selected destination folder: {error}"))?;
    let name = destination
        .file_name()
        .ok_or("Choose a filename for the exported setup log.")?;
    let target = parent.join(name);
    if let Ok(metadata) = fs::symlink_metadata(&target) {
        if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
            return Err("The selected export target is not a regular file.".into());
        }
    }
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW);
    let mut file = options
        .open(&target)
        .map_err(|error| format!("Could not export the setup log: {error}"))?;
    file.write_all(content.as_bytes())
        .map_err(|error| format!("Could not write the exported setup log: {error}"))?;
    Ok(target.display().to_string())
}

fn log_path(directory: &Path, job_id: &str) -> Result<PathBuf, String> {
    validate_job_id(job_id)?;
    Ok(directory.join(format!("tool-setup-{job_id}.log")))
}

pub(crate) fn validate_job_id(job_id: &str) -> Result<(), String> {
    if job_id.is_empty()
        || job_id.len() > 64
        || !job_id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'-')
    {
        return Err("Invalid setup log identifier.".into());
    }
    Ok(())
}

fn clean_line(line: &str) -> String {
    let mut cleaned = String::new();
    let mut clipped = false;
    for character in line
        .chars()
        .filter(|character| !character.is_control() || *character == '\t')
    {
        if cleaned.len().saturating_add(character.len_utf8()) > 32 * 1024 {
            clipped = true;
            break;
        }
        cleaned.push(character);
    }
    if clipped {
        cleaned.push_str(" [line clipped at 32 KiB]");
    }
    redact_url_userinfo(&cleaned)
}

fn redact_url_userinfo(line: &str) -> String {
    let mut output = String::with_capacity(line.len());
    let mut cursor = 0;
    while let Some(relative_scheme_end) = line[cursor..].find("://") {
        let authority_start = cursor + relative_scheme_end + 3;
        output.push_str(&line[cursor..authority_start]);
        let authority_end = line[authority_start..]
            .find(|character: char| character.is_whitespace() || "/?#[]()<>\"'".contains(character))
            .map(|offset| authority_start + offset)
            .unwrap_or(line.len());
        let authority = &line[authority_start..authority_end];
        if let Some(at) = authority.rfind('@') {
            output.push_str("[credentials]@");
            output.push_str(&authority[at + 1..]);
        } else {
            output.push_str(authority);
        }
        cursor = authority_end;
    }
    output.push_str(&line[cursor..]);
    output
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn set_private_directory(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|error| format!("Could not restrict setup log folder permissions: {error}"))?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn set_private_file(options: &mut OpenOptions) {
    #[cfg(unix)]
    options.mode(0o600);
    #[cfg(not(unix))]
    let _ = options;
}

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
