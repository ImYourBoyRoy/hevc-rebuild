//! Tests for queue status synchronization.

use super::*;
use crate::media_queue::{QueueDocument, QueueSource, QueueSourceStatus};

fn temporary_root(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "hevc-mcp-queue-{label}-{}-{nonce}",
        std::process::id()
    ))
}

fn prepared_run(root: &Path, media: &Path, mode: &str) -> PreparedMediaRun {
    PreparedMediaRun {
        root: root.canonicalize().unwrap(),
        mode: mode.to_owned(),
        selected_files: vec![media.canonicalize().unwrap().to_string_lossy().into_owned()],
        args: Vec::new(),
        output_dir: None,
        file_output_dirs: BTreeMap::new(),
        tool_paths: ToolPaths::default(),
        recursive: true,
        minimum_size_mib: 5,
        filename_include_patterns: Vec::new(),
        filename_exclude_patterns: Vec::new(),
    }
}

#[test]
fn queue_batch_rejects_duplicate_canonical_files_across_overlapping_roots() {
    let root = temporary_root("overlap");
    let nested = root.join("Nested");
    fs::create_dir_all(&nested).unwrap();
    let media = nested.join("Sample.mkv");
    fs::write(&media, b"synthetic media placeholder").unwrap();
    let runs = vec![
        prepared_run(&root, &media, "check"),
        prepared_run(&nested, &media, "check"),
    ];
    assert!(validate_prepared_queue_runs(&runs).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn queue_batch_preflights_duplicate_output_names_before_start() {
    let root = temporary_root("output-collision");
    let first = root.join("first");
    let second = root.join("second");
    let output = root.join("output");
    fs::create_dir_all(&first).unwrap();
    fs::create_dir_all(&second).unwrap();
    fs::create_dir_all(&output).unwrap();
    let first_media = first.join("SameName.mkv");
    let second_media = second.join("SameName.mkv");
    fs::write(&first_media, b"first synthetic media").unwrap();
    fs::write(&second_media, b"second synthetic media").unwrap();
    let mut runs = vec![
        prepared_run(&first, &first_media, "full"),
        prepared_run(&second, &second_media, "full"),
    ];
    for run in &mut runs {
        run.output_dir = Some(output.to_string_lossy().into_owned());
    }
    assert!(validate_prepared_queue_runs(&runs).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn queue_batch_accepts_distinct_sources_and_shared_output_plan() {
    let root = temporary_root("valid-batch");
    let first = root.join("first");
    let second = root.join("second");
    let output = root.join("output");
    fs::create_dir_all(&first).unwrap();
    fs::create_dir_all(&second).unwrap();
    fs::create_dir_all(&output).unwrap();
    let first_media = first.join("First.mkv");
    let second_media = second.join("Second.mkv");
    fs::write(&first_media, b"first synthetic media").unwrap();
    fs::write(&second_media, b"second synthetic media").unwrap();
    let mut runs = vec![
        prepared_run(&first, &first_media, "metadata"),
        prepared_run(&second, &second_media, "metadata"),
    ];
    for run in &mut runs {
        run.output_dir = Some(output.to_string_lossy().into_owned());
    }
    assert!(validate_prepared_queue_runs(&runs).is_ok());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn explicit_queue_run_paths_record_running_state_without_losing_selection() {
    let root = temporary_root("sync");
    fs::create_dir_all(&root).unwrap();
    let media = root.join("Sample.mkv");
    fs::write(&media, b"synthetic media placeholder").unwrap();
    let queue_path = root.join("library-queue.json");
    let media_path = media.to_string_lossy().into_owned();
    let root_path = root.to_string_lossy().into_owned();
    let mut queue = QueueDocument::default();
    queue.sources.push(QueueSource {
        root: root_path.clone(),
        status: QueueSourceStatus::Ready,
        scan_result: Some(json!({"entries": [{"path": media_path, "size_bytes": 28}]})),
        error: None,
        updated_at_ms: 1,
        changed_files: Vec::new(),
        missing_files: Vec::new(),
        excluded_paths: Vec::new(),
        entry_order: Vec::new(),
    });
    queue.selected_media_paths.push(media_path.clone());
    queue.active_root = Some(root_path);
    crate::media_queue::save(&queue_path, queue).unwrap();

    let sync = queue_run_sync_at(
        &queue_path,
        &root,
        std::slice::from_ref(&media_path),
        "check",
    )
    .unwrap()
    .unwrap();
    assert_eq!(sync.media_paths, vec![media_path.clone()]);
    crate::media_queue::save_run_states(
        &sync.queue_path,
        queue_run_states(
            &sync.media_paths,
            "check",
            "running",
            Some("Started through MCP."),
        ),
    )
    .unwrap();

    let report_directory = root.join("logs");
    fs::create_dir_all(&report_directory).unwrap();
    let report_path = report_directory.join("hevc-rebuild-queue-sync.csv");
    fs::write(
        &report_path,
        format!(
            "input,output,status,message\n\"{media_path}\",\"{}\",OK,Validated\n",
            root.join("Sample.metadata.mkv").display()
        ),
    )
    .unwrap();
    sync_queue_run_result_in(
        &sync,
        &report_directory,
        Some(&report_path.to_string_lossy()),
        Some(0),
        false,
    )
    .unwrap();

    let summary = crate::media_queue::summarize(&queue_path, 0, 10).unwrap();
    assert_eq!(summary["selectedCount"], 1);
    assert_eq!(summary["entries"][0]["runStatus"], "completed");
    assert_eq!(summary["entries"][0]["runMode"], "check");
    assert_eq!(summary["entries"][0]["reportStatus"], "OK");

    // A sequential job for the same source must consume its exact report,
    // rather than inheriting the first job's success. Unselected rows in a
    // different report cannot complete this file.
    let next_report = report_directory.join("hevc-rebuild-next-run.csv");
    fs::write(&next_report, format!("input,status,message\n\"{media_path}\",FAILED,Second run failed\nother.mkv,OK,Unrelated\n")).unwrap();
    sync_queue_run_result_in(
        &sync,
        &report_directory,
        Some(&next_report.to_string_lossy()),
        Some(1),
        false,
    )
    .unwrap();
    let summary = crate::media_queue::summarize(&queue_path, 0, 10).unwrap();
    assert_eq!(summary["entries"][0]["runStatus"], "failed");
    assert_eq!(summary["entries"][0]["reportStatus"], "FAILED");
    fs::write(&next_report, "input,status\nother.mkv,OK\n").unwrap();
    sync_queue_run_result_in(
        &sync,
        &report_directory,
        Some(&next_report.to_string_lossy()),
        Some(0),
        false,
    )
    .unwrap();
    let summary = crate::media_queue::summarize(&queue_path, 0, 10).unwrap();
    assert_eq!(summary["entries"][0]["runStatus"], "needs_review");
    assert_eq!(summary["selectedCount"], 1);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn queue_completion_requires_a_matching_report_status() {
    let successful = json!({
        "status": "METADATA_OK",
        "output": "/tmp/Sample.metadata.mkv",
        "message": "Validated metadata copy",
    });
    let completed = queue_state_from_report_row(
        "metadata",
        Some("/tmp/hevc-rebuild-job.csv"),
        Some(&successful),
        Some(0),
        false,
    );
    assert_eq!(completed.status, "completed");
    assert_eq!(
        completed.output_path.as_deref(),
        Some("/tmp/Sample.metadata.mkv")
    );

    let missing = queue_state_from_report_row("metadata", None, None, Some(0), false);
    assert_eq!(missing.status, "needs_review");
    assert!(missing
        .message
        .unwrap()
        .contains("completion was not assumed"));
}
