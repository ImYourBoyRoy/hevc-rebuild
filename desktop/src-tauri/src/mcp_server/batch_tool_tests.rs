//! Tests for the unattended batch tools, driven through the same methods the router calls.

use super::*;
use std::process::Command;

const MEDIA_TOOLS: [&str; 5] = ["ffmpeg", "ffprobe", "mediainfo", "mkvmerge", "mkvextract"];

fn scratch(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "hevc-batch-tools-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// The error text of a refused call; `rmcp::Json` has no `Debug`, so `unwrap_err` cannot be used.
fn refusal<T>(result: Result<T, ErrorData>) -> String {
    match result {
        Ok(_) => panic!("the call was expected to be refused"),
        Err(error) => error.message.to_string(),
    }
}

fn plan_params(root: &Path, reserve_gib: f64) -> PlanBatchParams {
    PlanBatchParams {
        root: root.to_string_lossy().into_owned(),
        recursive: None,
        minimum_size_mib: Some(0),
        filename_include_patterns: None,
        filename_exclude_patterns: None,
        reserve_gib: Some(reserve_gib),
        work_root: None,
        settings_json: None,
    }
}

fn plan_id(server: &McpServer, root: &Path, reserve_gib: f64) -> String {
    let plan = server
        .plan_library_batch(Parameters(plan_params(root, reserve_gib)))
        .expect("plan the batch")
        .0;
    plan["plan_id"].as_str().unwrap().to_owned()
}

fn with_isolated_state<T>(label: &str, body: impl FnOnce() -> T) -> T {
    let _guard = crate::TEST_STATE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let state = scratch(label);
    std::env::set_var("HEVC_REBUILD_PORTABLE_DATA_DIR", &state);
    let out = body();
    let _ = fs::remove_dir_all(state);
    out
}

#[test]
fn planning_lists_files_skips_backed_up_ones_and_fixes_the_floor_in_the_settings() {
    with_isolated_state("plan-state", || {
        let root = scratch("plan");
        for name in ["A.mkv", "B.mkv", "Done.mkv"] {
            fs::write(root.join(name), b"media-ish").unwrap();
        }
        fs::create_dir(root.join(crate::backup_folder::BACKUP_FOLDER)).unwrap();
        fs::write(
            root.join(crate::backup_folder::BACKUP_FOLDER)
                .join("Done.mkv"),
            b"backup",
        )
        .unwrap();
        let server = McpServer::new();
        let plan = server
            .plan_library_batch(Parameters(plan_params(&root, 1.0)))
            .expect("plan")
            .0;
        assert_eq!(plan["file_count"], 2);
        assert_eq!(plan["already_processed_skipped"], 1);
        assert_eq!(plan["protected_free_space_gib"], 1.0);
        let shown = plan["show_the_user"].as_str().unwrap();
        assert!(
            shown.contains(".hevc-rebuild-originals") && shown.contains("pauses itself"),
            "{shown}"
        );
        let id = plan["plan_id"].as_str().unwrap();
        let stored = server.plans.lock().unwrap().take(id).unwrap();
        let settings: Value = serde_json::from_str(&stored.settings_json).unwrap();
        assert_eq!(
            settings.pointer("/resources/min_free_disk_gib"),
            Some(&json!(1.0)),
            "the floor must reach the engine through the settings it will use"
        );
        // The default floor protects at least 10 GiB when the caller gives none.
        let mut defaulted = plan_params(&root, 0.0);
        defaulted.reserve_gib = None;
        let plan = server
            .plan_library_batch(Parameters(defaulted))
            .expect("plan with the default floor")
            .0;
        assert!(
            plan["protected_free_space_gib"].as_f64().unwrap() >= 10.0 || {
                // A scratch volume smaller than 40 GiB is capped at a quarter of itself.
                plan["volumes"][0]["total_gib"].as_f64().unwrap() < 40.0
            }
        );
        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn backups_in_old_are_never_planned_even_when_a_caller_passes_other_exclusions() {
    with_isolated_state("old-state", || {
        let root = scratch("old-excluded");
        fs::create_dir_all(root.join("Show").join("old")).unwrap();
        fs::write(root.join("Show").join("Ep1.mkv"), b"new").unwrap();
        fs::write(root.join("Show").join("old").join("Ep1.mkv"), b"original").unwrap();
        fs::write(root.join("Show").join("Ep2.mkv"), b"not yet rebuilt").unwrap();
        let mut params = plan_params(&root, 1.0);
        // A caller that supplies its own filters still cannot opt back in to old/.
        params.filename_exclude_patterns = Some(vec!["sample".into()]);
        let server = McpServer::new();
        let plan = server
            .plan_library_batch(Parameters(params))
            .expect("plan")
            .0;
        // Ep1 has a backup (already processed); Ep2 is the only real work. The
        // backup copy of Ep1 inside old/ must not appear as a third candidate.
        assert_eq!(plan["file_count"], 1, "{plan}");
        assert_eq!(plan["already_processed_skipped"], 1, "{plan}");
        assert!(Path::new(plan["first_files"][0]["path"].as_str().unwrap())
            .ends_with(Path::new("Show").join("Ep2.mkv")));
        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn a_full_rebuild_over_mcp_never_runs_with_a_zero_disk_reserve() {
    with_isolated_state("floor-default-state", || {
        let root = scratch("floor-default");
        fs::write(root.join("Ep1.mkv"), b"x").unwrap();
        let stop = root.join("stop-request");
        let reserve = |settings: Option<&str>, mode: &str| -> f64 {
            let params = StartRunParams {
                root: root.to_string_lossy().into_owned(),
                mode: Some(mode.into()),
                confirm_full_rebuild: Some(true),
                confirm_test_output: Some(true),
                settings_json: settings.map(str::to_owned),
                ..StartRunParams::default()
            };
            let args = prepare_media_run(params, &stop).expect("prepare").args;
            let index = args
                .iter()
                .position(|argument| argument == "--min-free-disk-gib")
                .expect("the engine always receives a reserve");
            args[index + 1].parse().unwrap()
        };
        assert!(
            reserve(None, "full") >= 0.1,
            "default settings carry no reserve"
        );
        let with_reserve = |gib: f64| {
            let mut settings = crate::settings::default_settings();
            settings["resources"]["min_free_disk_gib"] = json!(gib);
            settings.to_string()
        };
        assert!(
            reserve(Some(&with_reserve(0.0)), "full") > 0.0,
            "an explicit zero must not disable the guard"
        );
        assert_eq!(
            reserve(Some(&with_reserve(123.0)), "full"),
            123.0,
            "a larger configured floor is kept"
        );
        // Short tests write tiny clips beside the source, so the floor stays as configured.
        assert_eq!(reserve(Some(&with_reserve(0.0)), "test"), 0.0);
        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn other_runs_skip_old_unless_the_caller_gives_an_explicit_filter_list() {
    with_isolated_state("old-default-state", || {
        let root = scratch("old-default");
        fs::write(root.join("Ep1.mkv"), b"x").unwrap();
        let stop = root.join("stop-request");
        let run = |patterns: Option<Vec<String>>| {
            let params = StartRunParams {
                root: root.to_string_lossy().into_owned(),
                filename_exclude_patterns: patterns,
                ..StartRunParams::default()
            };
            prepare_media_run(params, &stop).expect("prepare").args
        };
        let excluded = |args: &[String]| {
            args.windows(2)
                .filter(|pair| pair[0] == "--exclude-filename-pattern")
                .map(|pair| pair[1].clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(excluded(&run(None)), vec!["old".to_string()]);
        assert!(
            excluded(&run(Some(Vec::new()))).is_empty(),
            "an explicit empty list opts out"
        );
        assert_eq!(
            excluded(&run(Some(vec!["sample".into()]))),
            vec!["sample".to_string()]
        );
        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn planning_refuses_an_empty_or_fully_processed_folder() {
    with_isolated_state("plan-empty-state", || {
        let root = scratch("plan-empty");
        let server = McpServer::new();
        assert!(server
            .plan_library_batch(Parameters(plan_params(&root, 1.0)))
            .is_err());
        fs::write(root.join("A.mkv"), b"x").unwrap();
        fs::create_dir(root.join("old")).unwrap();
        fs::write(root.join("old").join("A.mkv"), b"x").unwrap();
        let message = refusal(server.plan_library_batch(Parameters(plan_params(&root, 1.0))));
        assert!(message.contains("nothing left to rebuild"), "{message}");
        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn a_start_request_needs_a_real_plan_and_is_void_after_the_library_changes() {
    with_isolated_state("start-guard-state", || {
        let root = scratch("start-guard");
        fs::write(root.join("A.mkv"), b"first").unwrap();
        let server = McpServer::new();
        let invented = server.start_planned_batch(Parameters(StartPlannedParams {
            plan_id: "plan-made-up-by-the-model".into(),
        }));
        assert!(invented.is_err());

        let id = plan_id(&server, &root, 1.0);
        fs::write(root.join("A.mkv"), b"first, then edited").unwrap();
        let message = refusal(server.start_planned_batch(Parameters(StartPlannedParams {
            plan_id: id.clone(),
        })));
        assert!(
            message.contains("changed after it was planned"),
            "{message}"
        );
        // Void means void: the same ID must not work again.
        assert!(server
            .start_planned_batch(Parameters(StartPlannedParams { plan_id: id }))
            .is_err());
        assert!(
            server.jobs.lock().unwrap().is_empty(),
            "nothing may have started"
        );
        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn a_batch_will_not_start_below_its_floor_and_the_plan_survives_for_a_retry() {
    with_isolated_state("floor-state", || {
        let root = scratch("floor");
        fs::write(root.join("A.mkv"), b"x").unwrap();
        let server = McpServer::new();
        // No scratch disk has a million GiB free, so the floor is always breached.
        let id = plan_id(&server, &root, 1_000_000.0);
        for _ in 0..2 {
            let message = refusal(server.start_planned_batch(Parameters(StartPlannedParams {
                plan_id: id.clone(),
            })));
            assert!(message.contains("below the protected floor"), "{message}");
        }
        assert!(server.jobs.lock().unwrap().is_empty());
        let _ = fs::remove_dir_all(root);
    });
}

fn wait_for_job(server: &McpServer, job_id: &str) -> Value {
    let mut job = Value::Null;
    for _ in 0..600 {
        job = server
            .get_job(Parameters(JobParams {
                job_id: job_id.to_owned(),
                offset: None,
                limit: None,
            }))
            .unwrap()
            .0;
        if job["job"]["status"] != "running" {
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
    job
}

#[test]
fn one_bad_file_in_a_library_does_not_make_the_whole_batch_look_failed() {
    if !crate::media_tools_ready("mixed batch", &MEDIA_TOOLS) {
        return;
    }
    with_isolated_state("mixed-state", || {
        let root = scratch("mixed");
        let good = root.join("Good.mkv");
        let made = Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=320x180:rate=24:duration=2",
            ])
            .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=2"])
            .args(["-c:v", "libx264", "-preset", "ultrafast", "-c:a", "aac"])
            .args(["-metadata:s:a:0", "language=eng"])
            .arg(&good)
            .status()
            .expect("run ffmpeg");
        assert!(made.success());
        let original = fs::read(&good).unwrap();
        fs::write(root.join("Bad.mkv"), b"this is not media").unwrap();
        let server = McpServer::new();
        let id = plan_id(&server, &root, 0.1);
        let started = server
            .start_planned_batch(Parameters(StartPlannedParams { plan_id: id }))
            .expect("start")
            .0;
        let job = wait_for_job(&server, started["job_id"].as_str().unwrap());
        assert_eq!(job["job"]["status"], "completed_with_failures", "{job}");
        assert!(
            job["job"]["error"].is_null(),
            "per-file failures are not a job error"
        );
        let batch = &job["job"]["batch"];
        assert_eq!(batch["succeeded"], 1);
        assert_eq!(batch["failed"], 1);
        assert!(batch["halted"].is_null());
        assert_eq!(
            batch["originals_kept_bytes"].as_u64(),
            Some(original.len() as u64)
        );
        assert!(batch["output_bytes_written"].as_u64().unwrap() > 0);
        assert_eq!(
            fs::read(
                root.join(crate::backup_folder::BACKUP_FOLDER)
                    .join("Good.mkv")
            )
            .unwrap(),
            original,
            "the original is kept byte for byte"
        );
        assert_eq!(
            fs::read(root.join("Bad.mkv")).unwrap(),
            b"this is not media"
        );
        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn an_unattended_batch_pauses_after_a_failure_streak_and_reports_what_happened() {
    if !crate::media_tools_ready("unattended batch", &MEDIA_TOOLS) {
        return;
    }
    with_isolated_state("run-state", || {
        let root = scratch("run");
        let names = ["A.mkv", "B.mkv", "C.mkv", "D.mkv", "E.mkv"];
        for name in names {
            fs::write(root.join(name), b"this is not media").unwrap();
        }
        let server = McpServer::new();
        let id = plan_id(&server, &root, 0.1);
        let started = server
            .start_planned_batch(Parameters(StartPlannedParams {
                plan_id: id.clone(),
            }))
            .expect("start")
            .0;
        let job_id = started["job_id"].as_str().unwrap().to_owned();
        let mut job = Value::Null;
        for _ in 0..300 {
            job = server
                .get_job(Parameters(JobParams {
                    job_id: job_id.clone(),
                    offset: None,
                    limit: None,
                }))
                .unwrap()
                .0;
            if job["job"]["status"] != "running" {
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }
        assert_eq!(job["job"]["status"], "paused", "{job}");
        let batch = &job["job"]["batch"];
        assert_eq!(batch["halted"]["code"], "consecutive_failures");
        assert_eq!(batch["failed"], 3, "stops after the third failure in a row");
        assert_eq!(batch["failures"].as_array().unwrap().len(), 3);
        assert!(batch["report"].as_str().unwrap().contains("0 of 5"));
        for name in names {
            assert_eq!(fs::read(root.join(name)).unwrap(), b"this is not media");
        }
        assert!(!root.join(crate::backup_folder::BACKUP_FOLDER).exists());
        // The plan was consumed, and a finished job is listed for later lookup.
        assert!(server
            .start_planned_batch(Parameters(StartPlannedParams { plan_id: id }))
            .is_err());
        let listed = server.list_jobs().unwrap().0;
        assert_eq!(listed["jobs"][0]["job_id"], job_id.as_str());
        assert_eq!(listed["jobs"][0]["status"], "paused");
        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn the_server_reports_the_version_it_ships_as() {
    // The tool_handler attribute needs a string literal, so this fails the moment
    // a release bump leaves the MCP-reported version behind.
    let info = ServerHandler::get_info(&McpServer::new());
    assert_eq!(info.server_info.version, env!("CARGO_PKG_VERSION"));
}
