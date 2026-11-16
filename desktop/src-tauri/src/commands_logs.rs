//! Tauri commands for reports, tool-setup logs and confined diagnostic log reads.

use super::*;

#[tauri::command]
pub(crate) async fn delete_reports() -> Result<usize, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let directory = state_log_directory()?;
        reports::delete_saved_reports(&directory)
    })
    .await
    .map_err(|error| format!("Report deletion task failed: {error}"))?
}

#[tauri::command]
pub(crate) async fn list_reports(limit: u8) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let directory = state_log_directory()?;
        Ok(reports::list_reports(
            &directory,
            limit.clamp(1, 50) as usize,
        ))
    })
    .await
    .map_err(|error| format!("Report listing task failed: {error}"))?
}

#[tauri::command]
pub(crate) fn list_tool_setup_logs() -> Result<Vec<tool_diagnostics::ToolSetupLogSummary>, String> {
    tool_diagnostics::list(&tool_setup_log_directory()?)
}

#[tauri::command]
pub(crate) fn read_tool_setup_log(job_id: String) -> Result<String, String> {
    tool_diagnostics::read(&tool_setup_log_directory()?, &job_id)
}

#[tauri::command]
pub(crate) fn export_tool_setup_log(job_id: String, destination: String) -> Result<String, String> {
    tool_diagnostics::export(
        &tool_setup_log_directory()?,
        &job_id,
        Path::new(&destination),
    )
}

#[tauri::command]
pub(crate) fn read_log(path: String) -> Result<String, String> {
    let allowed = state_log_directory()?
        .canonicalize()
        .map_err(|error| format!("Diagnostic log folder is unavailable: {error}"))?;
    let requested = PathBuf::from(path)
        .canonicalize()
        .map_err(|error| format!("Could not open the diagnostic file: {error}"))?;
    if !requested.starts_with(&allowed)
        || requested.extension().and_then(|value| value.to_str()) != Some("log")
    {
        return Err(
            "Only HEVC Rebuild log files from this user's log folder can be read here.".into(),
        );
    }
    let mut file = File::open(&requested)
        .map_err(|error| format!("Could not read the diagnostic file: {error}"))?;
    let size = file.metadata().map_err(|error| error.to_string())?.len();
    let limit = 1_000_000_u64;
    let start = size.saturating_sub(limit);
    file.seek(SeekFrom::Start(start))
        .map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    let mut result = String::from_utf8_lossy(&bytes).into_owned();
    if start > 0 {
        result.insert_str(0, "[Showing the final 1 MB of this diagnostic log]\n\n");
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_reads_stay_confined_to_the_state_log_folder() {
        let _guard = TEST_STATE_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = std::env::var_os("HEVC_REBUILD_PORTABLE_DATA_DIR");
        let root = test_dir("log-reads");
        std::env::set_var("HEVC_REBUILD_PORTABLE_DATA_DIR", &root);

        let logs = root.join("logs");
        fs::create_dir_all(&logs).unwrap();
        fs::write(logs.join("run.log"), "hello log").unwrap();
        fs::write(logs.join("run.txt"), "not a log").unwrap();
        fs::write(root.join("outside.log"), "outside").unwrap();
        let read = read_log(logs.join("run.log").to_string_lossy().into_owned());
        assert_eq!(read.unwrap(), "hello log");
        assert!(read_log(logs.join("run.txt").to_string_lossy().into_owned()).is_err());
        assert!(read_log(root.join("outside.log").to_string_lossy().into_owned()).is_err());

        match previous {
            Some(value) => std::env::set_var("HEVC_REBUILD_PORTABLE_DATA_DIR", value),
            None => std::env::remove_var("HEVC_REBUILD_PORTABLE_DATA_DIR"),
        }
        fs::remove_dir_all(root).unwrap();
    }
}
