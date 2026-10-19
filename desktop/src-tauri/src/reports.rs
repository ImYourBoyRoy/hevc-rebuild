use serde::Deserialize;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

const REPORT_FIELDS: &[&str] = &[
    "input",
    "output",
    "status",
    "message",
    "primary_video",
    "resolution",
    "audio_count",
    "subtitle_count",
    "subtitle_note",
    "hdr10",
    "hdr10plus",
    "hdr10plus_note",
    "dolby_vision",
    "dovi_profile",
    "dovi_source",
    "dolby_vision_action",
    "memory_forecast",
    "memory_available_gib",
    "memory_total_gib",
    "memory_swap_used_percent",
    "memory_budget_gib",
    "memory_worker_cap",
    "planned_workers",
    "planned_frame_threads",
    "memory_forecast_message",
    "calibration_source",
    "calibration_workers",
    "calibration_frame_threads",
    "calibration_candidate_count",
    "calibration_note",
    "min_memory_available_gib",
    "max_encoder_rss_gib",
    "max_swap_used_percent",
    "max_encoder_swap_gib",
    "max_swap_in_mib_s",
    "max_swap_out_mib_s",
    "max_memory_full_psi_percent",
    "input_bytes",
    "output_bytes",
    "upload_resolution",
    "upload_quality_mode",
    "upload_source_multiplier",
    "output_resolution",
    "upload_video_bitrate_kbps",
    "youtube_recommended_video_bitrate_kbps",
    "estimated_output_bytes",
    "metadata_text_scrub_count",
    "metadata_text_scrub_details",
    "elapsed_seconds",
];
const MAX_READ_REPORT_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Deserialize)]
struct AppendReportRequest {
    path: String,
    row: Map<String, Value>,
}

/// Append one engine report row through the native Rust process boundary.
/// This action is private to the bundled Rust-launched compatibility engine.
pub(crate) fn cli_mode(args: Vec<OsString>) -> i32 {
    if !args.is_empty() {
        eprintln!("The internal report append helper accepts no options.");
        return 2;
    }
    let mut bytes = Vec::new();
    if let Err(error) = io::stdin()
        .lock()
        .take(8 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
    {
        eprintln!("Could not read report append request: {error}");
        return 1;
    }
    if bytes.len() > 8 * 1024 * 1024 {
        eprintln!("Report append request exceeds the 8 MiB limit.");
        return 2;
    }
    let request = match serde_json::from_slice::<AppendReportRequest>(&bytes) {
        Ok(request) => request,
        Err(error) => {
            eprintln!("Invalid report append request: {error}");
            return 2;
        }
    };
    match append_report(&request) {
        Ok(()) => match serde_json::to_writer(
            io::stdout().lock(),
            &serde_json::json!({ "appended": true, "durability": "confirmed" }),
        ) {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("Could not write report append response: {error}");
                1
            }
        },
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

fn append_report(request: &AppendReportRequest) -> Result<(), String> {
    let requested = PathBuf::from(&request.path);
    if !requested.is_absolute() {
        return Err("Report path must be absolute.".into());
    }
    let allowed = crate::state_log_directory()?
        .canonicalize()
        .map_err(|error| format!("Run report directory is unavailable: {error}"))?;
    let parent = requested
        .parent()
        .ok_or("Report path has no parent directory.")?
        .canonicalize()
        .map_err(|error| format!("Could not resolve run report directory: {error}"))?;
    if parent != allowed {
        return Err("Reports can only be written to this user's HEVC Rebuild log folder.".into());
    }
    let file_name = requested
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or("Run report name is not valid UTF-8.")?;
    if !file_name.starts_with("hevc-rebuild-")
        || requested.extension().and_then(|value| value.to_str()) != Some("csv")
    {
        return Err("Run report name must use the hevc-rebuild-*.csv pattern.".into());
    }
    if let Ok(metadata) = fs::symlink_metadata(&requested) {
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err("Run report target must be a regular file.".into());
        }
    }
    let mut values = Vec::with_capacity(REPORT_FIELDS.len());
    for field in REPORT_FIELDS {
        values.push(csv_value(request.row.get(*field))?);
    }
    append_report_file(&requested, values)
}

// The descriptor owns the flock, including on an error return. Python uses the
// same lock on the CSV inode, so mixed native/direct writers also serialize.
fn lock_report(file: &File) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        file.lock()
    }
}

fn append_report_file(requested: &Path, values: Vec<String>) -> Result<(), String> {
    let mut options = OpenOptions::new();
    options.create(true).read(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW).mode(0o600);
    }
    let mut file = options
        .open(requested)
        .map_err(|error| format!("Could not open run report: {error}"))?;
    lock_report(&file).map_err(|error| format!("Could not lock run report: {error}"))?;
    let original_size = file.metadata().map_err(|error| error.to_string())?.len();
    let mut header = Vec::new();
    append_csv_record(
        &mut header,
        REPORT_FIELDS.iter().copied().map(str::to_owned),
    );
    if original_size != 0 {
        let mut existing_header = vec![0; header.len()];
        file.read_exact(&mut existing_header)
            .map_err(|error| format!("Run report header is incomplete: {error}"))?;
        if existing_header != header {
            return Err("Run report schema mismatch; existing report was preserved.".into());
        }
        file.seek(SeekFrom::End(-1))
            .map_err(|error| error.to_string())?;
        let mut tail = [0];
        file.read_exact(&mut tail)
            .map_err(|error| error.to_string())?;
        if tail != *b"\n" {
            return Err("Run report has an incomplete write; review before retrying.".into());
        }
    }
    let mut output = Vec::new();
    if original_size == 0 {
        output.extend_from_slice(&header);
    }
    append_csv_record(&mut output, values);
    if let Err(error) = file.write_all(&output).and_then(|()| file.flush()) {
        let rollback = file.set_len(original_size);
        return Err(format!(
            "Could not append run report: {error}; append rollback: {rollback:?}"
        ));
    }
    confirm_report_durability(
        || file.sync_all(),
        || {
            if original_size == 0 {
                File::open(
                    requested
                        .parent()
                        .ok_or_else(|| io::Error::other("Report has no parent directory"))?,
                )
                .and_then(|directory| directory.sync_all())
            } else {
                Ok(())
            }
        },
    )
}

fn confirm_report_durability(
    sync_file: impl FnOnce() -> io::Result<()>,
    sync_directory: impl FnOnce() -> io::Result<()>,
) -> Result<(), String> {
    sync_file().map_err(|error| {
        format!("Run report was written but file durability is unconfirmed: {error}")
    })?;
    sync_directory().map_err(|error| {
        format!("Run report was written but directory durability is unconfirmed: {error}")
    })
}

fn csv_value(value: Option<&Value>) -> Result<String, String> {
    match value {
        None | Some(Value::Null) => Ok(String::new()),
        Some(Value::String(value)) => Ok(value.clone()),
        Some(Value::Bool(value)) => Ok(if *value { "True" } else { "False" }.into()),
        Some(Value::Number(value)) => Ok(value.to_string()),
        Some(Value::Array(_) | Value::Object(_)) => {
            Err("Run report row values must be scalar JSON values.".into())
        }
    }
}

fn append_csv_record(output: &mut Vec<u8>, values: impl IntoIterator<Item = String>) {
    let mut first = true;
    for value in values {
        if !first {
            output.push(b',');
        }
        first = false;
        if value.contains(',')
            || value.contains('"')
            || value.contains('\r')
            || value.contains('\n')
        {
            output.push(b'"');
            output.extend_from_slice(value.replace('"', "\"\"").as_bytes());
            output.push(b'"');
        } else {
            output.extend_from_slice(value.as_bytes());
        }
    }
    output.extend_from_slice(b"\r\n");
}

fn parse_csv_records(input: &str) -> Result<Vec<Vec<String>>, ()> {
    let mut records = Vec::new();
    let mut record = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if quoted {
            match ch {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    field.push('"');
                }
                '"' => quoted = false,
                _ => field.push(ch),
            }
            continue;
        }

        match ch {
            '"' if field.is_empty() => quoted = true,
            ',' => record.push(std::mem::take(&mut field)),
            '\n' => {
                record.push(std::mem::take(&mut field));
                if !(record.len() == 1 && record[0].is_empty()) {
                    records.push(std::mem::take(&mut record));
                } else {
                    record.clear();
                }
            }
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                record.push(std::mem::take(&mut field));
                if !(record.len() == 1 && record[0].is_empty()) {
                    records.push(std::mem::take(&mut record));
                } else {
                    record.clear();
                }
            }
            _ => field.push(ch),
        }
    }

    if quoted {
        return Err(());
    }
    if !field.is_empty() || !record.is_empty() {
        record.push(field);
        records.push(record);
    }
    Ok(records)
}

fn rows_from_csv(input: &str) -> Result<Vec<Map<String, Value>>, ()> {
    let mut records = parse_csv_records(input)?.into_iter();
    let Some(headers) = records.next() else {
        return Ok(Vec::new());
    };
    Ok(records
        .map(|record| {
            let mut row = Map::new();
            for (index, name) in headers.iter().enumerate() {
                let value = record
                    .get(index)
                    .map(|value| Value::String(value.clone()))
                    .unwrap_or(Value::Null);
                row.insert(name.clone(), value);
            }
            row
        })
        .collect())
}

fn report_value(path: &Path, created: f64, rows: Vec<Map<String, Value>>) -> Value {
    let log_path = path.with_extension("log");
    let mut counts = BTreeMap::<String, usize>::new();
    for row in &rows {
        let status = row
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("UNKNOWN");
        *counts.entry(status.to_string()).or_default() += 1;
    }
    serde_json::json!({
        "csv_path": path.to_string_lossy(),
        "log_path": log_path.exists().then(|| log_path.to_string_lossy().into_owned()),
        "created": created,
        "counts": counts,
        "rows": rows,
    })
}

/// Load recent local CSV reports without starting the Python compatibility bridge.
/// Unavailable folders and malformed individual reports are treated as an empty/omitted result.
pub(crate) fn list_reports(directory: &Path, limit: usize) -> Value {
    let Ok(entries) = fs::read_dir(directory) else {
        return serde_json::json!({ "reports": [] });
    };
    let mut paths: Vec<(PathBuf, f64)> = entries
        .filter_map(Result::ok)
        .filter(|entry| {
            entry.file_type().is_ok_and(|kind| kind.is_file())
                && entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("hevc-rebuild-")
                && entry
                    .path()
                    .extension()
                    .and_then(|extension| extension.to_str())
                    == Some("csv")
        })
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            let modified = metadata
                .modified()
                .ok()?
                .duration_since(UNIX_EPOCH)
                .ok()?
                .as_secs_f64();
            Some((entry.path(), modified))
        })
        .collect();
    paths.sort_by(|left, right| right.1.total_cmp(&left.1));

    let reports = paths
        .into_iter()
        .take(limit.clamp(1, 50))
        .filter_map(|(path, modified)| {
            let input = fs::read_to_string(&path).ok()?;
            let rows = rows_from_csv(&input).ok()?;
            Some(report_value(&path, modified, rows))
        })
        .collect::<Vec<_>>();
    serde_json::json!({ "reports": reports })
}

/// Permanently delete saved run reports and their companion logs.
/// Only regular `hevc-rebuild-*.csv` and `hevc-rebuild-*.log` files directly
/// inside the application log folder are removed. Setup logs, other files,
/// directories, and symlinks stay in place.
pub(crate) fn delete_saved_reports(directory: &Path) -> Result<usize, String> {
    let allowed = directory
        .canonicalize()
        .map_err(|error| format!("Run report directory is unavailable: {error}"))?;
    let entries = fs::read_dir(&allowed)
        .map_err(|error| format!("Could not read the report folder: {error}"))?;
    let mut deleted = 0_usize;
    for entry in entries {
        let entry = entry.map_err(|error| format!("Could not read a report entry: {error}"))?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let report_file = name.starts_with("hevc-rebuild-")
            && (name.ends_with(".csv") || name.ends_with(".log"))
            && !name.contains('/')
            && !name.contains('\\');
        if !report_file {
            continue;
        }
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("Could not inspect {name}: {error}"))?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            continue;
        }
        let resolved = path
            .canonicalize()
            .map_err(|error| format!("Could not resolve {name}: {error}"))?;
        if resolved.parent() != Some(allowed.as_path()) {
            continue;
        }
        fs::remove_file(&resolved).map_err(|error| format!("Could not delete {name}: {error}"))?;
        deleted += 1;
    }
    Ok(deleted)
}

/// Read one existing report only from the application's own log directory.
/// The path is normally discovered from the engine's bounded job output.
pub(crate) fn read_report(directory: &Path, requested: &Path) -> Result<Value, String> {
    let allowed = directory
        .canonicalize()
        .map_err(|error| format!("Run report directory is unavailable: {error}"))?;
    let metadata = fs::symlink_metadata(requested)
        .map_err(|error| format!("Run report is unavailable: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("Run report must be a regular file.".into());
    }
    if metadata.len() > MAX_READ_REPORT_BYTES {
        return Err("Run report exceeds the 64 MiB read limit.".into());
    }
    let resolved = requested
        .canonicalize()
        .map_err(|error| format!("Could not resolve run report: {error}"))?;
    if resolved.parent() != Some(allowed.as_path()) {
        return Err("Reports can only be read from this user's HEVC Rebuild log folder.".into());
    }
    let file_name = resolved
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or("Run report name is not valid UTF-8.")?;
    if !file_name.starts_with("hevc-rebuild-")
        || resolved.extension().and_then(|value| value.to_str()) != Some("csv")
    {
        return Err("Run report name must use the hevc-rebuild-*.csv pattern.".into());
    }
    let contents = fs::read_to_string(&resolved)
        .map_err(|error| format!("Could not read run report: {error}"))?;
    let rows = rows_from_csv(&contents)
        .map_err(|_| "Run report contains malformed CSV data.".to_string())?;
    let modified = fs::metadata(&resolved)
        .ok()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs_f64())
        .unwrap_or_default();
    Ok(report_value(&resolved, modified, rows))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_durability_errors_identify_the_unconfirmed_boundary() {
        assert!(confirm_report_durability(
            || Err(io::Error::other("file sync failed")),
            || panic!("directory must not sync after file failure")
        )
        .unwrap_err()
        .contains("file durability is unconfirmed"));
        assert!(confirm_report_durability(
            || Ok(()),
            || Err(io::Error::other("directory sync failed"))
        )
        .unwrap_err()
        .contains("directory durability is unconfirmed"));
        assert!(confirm_report_durability(|| Ok(()), || Ok(())).is_ok());
    }

    #[test]
    fn report_writer_initializes_empty_file_and_rejects_schema_or_partial_tail() {
        let directory =
            std::env::temp_dir().join(format!("hevc-report-append-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("hevc-rebuild-schema.csv");
        fs::write(&path, b"").unwrap();
        let values = || REPORT_FIELDS.iter().map(|_| String::new()).collect();
        append_report_file(&path, values()).unwrap();
        append_report_file(&path, values()).unwrap();
        assert_eq!(
            parse_csv_records(&fs::read_to_string(&path).unwrap())
                .unwrap()
                .len(),
            3
        );
        for bytes in [
            b"wrong,header\n".as_slice(),
            b"input,status\npartial".as_slice(),
        ] {
            fs::write(&path, bytes).unwrap();
            assert!(append_report_file(&path, values()).is_err());
            assert_eq!(fs::read(&path).unwrap(), bytes);
        }
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn csv_reader_handles_quoted_commas_newlines_and_escaped_quotes() {
        let input = "input,status,message\r\nmovie.mkv,OK,done\r\n\"episode, one.mkv\",FAILED,\"line one\nline \"\"two\"\"\"\r\n";
        let rows = rows_from_csv(input).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1]["input"], "episode, one.mkv");
        assert_eq!(rows[1]["message"], "line one\nline \"two\"");
    }

    #[test]
    fn report_counts_match_rows_and_unavailable_log_is_null() {
        let path = Path::new("/tmp/hevc-rebuild-test.csv");
        let rows = rows_from_csv("input,status\na.mkv,OK\nb.mkv,FAILED\nc.mkv,OK\n").unwrap();
        let report = report_value(path, 10.0, rows);
        assert_eq!(report["created"], 10.0);
        assert_eq!(report["counts"]["OK"], 2);
        assert_eq!(report["counts"]["FAILED"], 1);
        assert_eq!(report["log_path"], Value::Null);
    }

    #[test]
    fn malformed_quoted_csv_is_rejected() {
        assert!(rows_from_csv("input,status\n\"unfinished,OK\n").is_err());
    }

    #[test]
    fn queue_report_reader_is_confined_to_the_application_log_directory() {
        let directory =
            std::env::temp_dir().join(format!("hevc-report-reader-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        let report_path = directory.join("hevc-rebuild-smoke.csv");
        fs::write(
            &report_path,
            "input,output,status,message\nSample.mkv,Sample.new.mkv,OK,done\n",
        )
        .unwrap();

        let report = read_report(&directory, &report_path).unwrap();
        assert_eq!(report["rows"][0]["input"], "Sample.mkv");
        assert_eq!(report["rows"][0]["status"], "OK");
        let outside = directory.parent().unwrap().join("hevc-rebuild-outside.csv");
        fs::write(&outside, "input,status\nSample.mkv,OK\n").unwrap();
        assert!(read_report(&directory, &outside).is_err());

        fs::remove_file(outside).unwrap();
        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn deleting_reports_removes_only_saved_run_files() {
        let directory =
            std::env::temp_dir().join(format!("hevc-report-delete-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(directory.join("tool-setup")).unwrap();
        fs::write(
            directory.join("hevc-rebuild-one.csv"),
            "input,status\nFilm.mkv,FAILED\n",
        )
        .unwrap();
        fs::write(directory.join("hevc-rebuild-one.log"), "diagnostic\n").unwrap();
        fs::write(directory.join("notes.txt"), "keep\n").unwrap();
        fs::write(
            directory.join("tool-setup/hevc-rebuild-setup.csv"),
            "keep\n",
        )
        .unwrap();
        fs::create_dir(directory.join("hevc-rebuild-folder.csv")).unwrap();
        let outside = std::env::temp_dir().join(format!(
            "hevc-report-delete-outside-{}.csv",
            std::process::id()
        ));
        fs::write(&outside, "keep\n").unwrap();
        std::os::unix::fs::symlink(&outside, directory.join("hevc-rebuild-link.csv")).unwrap();

        let deleted = delete_saved_reports(&directory).unwrap();
        assert_eq!(deleted, 2);
        assert!(!directory.join("hevc-rebuild-one.csv").exists());
        assert!(!directory.join("hevc-rebuild-one.log").exists());
        assert_eq!(
            fs::read_to_string(directory.join("notes.txt")).unwrap(),
            "keep\n"
        );
        assert!(directory.join("tool-setup/hevc-rebuild-setup.csv").exists());
        assert!(directory.join("hevc-rebuild-folder.csv").is_dir());
        assert!(directory.join("hevc-rebuild-link.csv").exists());
        assert!(outside.exists());

        fs::remove_file(&outside).unwrap();
        fs::remove_dir_all(directory).unwrap();
    }
}
