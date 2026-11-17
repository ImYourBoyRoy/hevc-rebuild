//! Sharing the MCP server's jobs with the headless web server. When the MCP endpoint is mounted inside
//! `serve`, an agent's job must be visible in the browser and must not collide with a run the browser
//! starts. This module holds the two halves: the single job slot (`Shared::claim`, so the GUI and MCP
//! take turns exactly as the GUI alone does) and a bridge that turns changes in the MCP job registry
//! into the desktop events the GUI already renders (`run-line`, `run-progress`, `run-finished`, ...)
//! on the server's event bus. The job executors are untouched: the bridge only reads the registry.

use super::*;
use crate::{release_job, Events, RunFinished, RunLine, RunProgress, RunState};
use std::collections::HashMap;

/// What the web server hands the MCP server so both front ends share one job slot and one event bus.
#[derive(Clone)]
pub(crate) struct Shared {
    pub(crate) run_state: RunState,
    pub(crate) events: Events,
    /// How privileged setup is authorised here (the server probes root / `sudo -n`).
    pub(crate) privilege: Arc<dyn Fn() -> crate::provision::Privilege + Send + Sync>,
}

fn is_active(job: &Job) -> bool {
    matches!(job.status.as_str(), "queued" | "running")
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Family {
    /// A media run the GUI renders on the Run page.
    Run,
    /// System setup or helper install: the GUI's tool-repair log.
    Repair,
    /// Folder analysis: the GUI's scan progress bar.
    Scan,
    /// Short bookkeeping operations with no live view.
    Quiet,
}

fn family(kind: &str) -> Family {
    match kind {
        "media_run" | "media_queue_run" | "library_batch" | "integrity_check" => Family::Run,
        "system_provision" | "helper_install" => Family::Repair,
        "folder_analysis" => Family::Scan,
        _ => Family::Quiet,
    }
}

impl Shared {
    /// Take the single job slot for `job`, called by `insert_job` with the registry locked. Two folder
    /// analyses may overlap (as in stdio MCP); anything else needs the slot free, so a run the GUI
    /// started refuses an agent's job and the reverse.
    pub(super) fn claim(&self, jobs: &HashMap<String, Job>, job: &Job) -> Result<(), String> {
        let mut slot = self
            .run_state
            .0
            .lock()
            .map_err(|_| "Could not lock the shared job state.".to_string())?;
        match slot.as_ref() {
            None => {
                let mut active = crate::new_active_job(job.id.clone(), None);
                active.kind = job.kind.clone();
                *slot = Some(active);
                Ok(())
            }
            Some(held) => {
                let shares = job.kind == "folder_analysis"
                    && jobs
                        .get(&held.id)
                        .is_some_and(|other| other.kind == "folder_analysis" && is_active(other));
                if shares {
                    Ok(())
                } else {
                    Err("Another run, scan, or repair is already active (started from the desktop window or another client).".into())
                }
            }
        }
    }

    /// Point the slot at the stop file once a job has one, so the GUI's stop buttons work on it.
    fn adopt_stop_file(&self, job: &Job) {
        let Ok(mut slot) = self.run_state.0.lock() else {
            return;
        };
        if let Some(held) = slot.as_mut().filter(|held| held.id == job.id) {
            if held.stop_file.is_none() {
                held.stop_file = job.stop_file.clone();
            }
        }
    }

    /// Release a slot held by a finished MCP job; hand it to another active MCP job if one remains.
    fn reconcile_slot(&self, snapshot: &[Job]) {
        let held = self
            .run_state
            .0
            .lock()
            .ok()
            .and_then(|slot| slot.as_ref().map(|held| held.id.clone()));
        if let Some(held) = held {
            if snapshot.iter().any(|job| job.id == held && !is_active(job)) {
                release_job(&self.run_state, &held);
            }
        }
        if let Some(next) = snapshot.iter().find(|job| is_active(job)) {
            if let Ok(mut slot) = self.run_state.0.lock() {
                if slot.is_none() {
                    let mut active = crate::new_active_job(next.id.clone(), next.stop_file.clone());
                    active.kind = next.kind.clone();
                    *slot = Some(active);
                }
            }
        }
    }
}

#[derive(Default)]
struct Seen {
    started: bool,
    lines: u64,
    progress: Option<(usize, usize, Option<String>, Option<ProgressDetail>)>,
    resources: Option<String>,
    finished: bool,
}

/// Turns registry changes into events. `tick` is called a few times a second by `spawn`, and directly by
/// the tests.
#[derive(Default)]
pub(super) struct Bridge {
    seen: HashMap<String, Seen>,
}

impl Bridge {
    pub(super) fn tick(&mut self, jobs: &Jobs, shared: &Shared) {
        let mut snapshot: Vec<Job> = match jobs.lock() {
            Ok(registry) => registry.values().cloned().collect(),
            Err(_) => return,
        };
        snapshot.sort_by(|a, b| a.id.cmp(&b.id));
        let events = &shared.events;
        for job in &snapshot {
            shared.adopt_stop_file(job);
            let seen = self.seen.entry(job.id.clone()).or_default();
            let family = family(&job.kind);
            if !seen.started {
                seen.started = true;
                let _ = events.emit("job-started", json!({"runId": job.id, "kind": job.kind}));
            }
            match family {
                Family::Run | Family::Repair => {
                    let fresh = (job.lines_total.saturating_sub(seen.lines) as usize)
                        .min(job.last_lines.len());
                    let name = if family == Family::Run {
                        "run-line"
                    } else {
                        "repair-line"
                    };
                    for line in &job.last_lines[job.last_lines.len() - fresh..] {
                        let _ = events.emit(
                            name,
                            RunLine {
                                run_id: job.id.clone(),
                                line: line.clone(),
                            },
                        );
                    }
                    seen.lines = job.lines_total;
                }
                _ => {}
            }
            let progress = (
                job.current,
                job.total,
                job.last_file.clone(),
                job.progress_detail.clone(),
            );
            if seen.progress.as_ref() != Some(&progress)
                && (job.total > 0 || job.last_file.is_some())
            {
                seen.progress = Some(progress);
                match family {
                    Family::Run => {
                        let path = job.last_file.clone();
                        let detail = job.progress_detail.clone().unwrap_or_default();
                        let _ = events.emit(
                            "run-progress",
                            RunProgress {
                                run_id: job.id.clone(),
                                file_name: path
                                    .as_deref()
                                    .and_then(|path| Path::new(path).file_name())
                                    .map(|name| name.to_string_lossy().into_owned())
                                    .unwrap_or_default(),
                                file_path: path,
                                phase: if detail.phase.is_empty() {
                                    "Working".into()
                                } else {
                                    detail.phase
                                },
                                batch_number: job.current as u32,
                                batch_total: job.total as u32,
                                file_percent: detail.file_percent,
                                queue_percent: detail.queue_percent.or_else(|| {
                                    (job.total > 0).then(|| {
                                        job.current.saturating_sub(1) as f64 * 100.0
                                            / job.total as f64
                                    })
                                }),
                                speed: detail.speed,
                                eta_seconds: detail.eta_seconds,
                                elapsed_seconds: detail.elapsed_seconds,
                                input_bytes: detail.input_bytes,
                                output_bytes: detail.output_bytes,
                                estimated_output_bytes: detail.estimated_output_bytes,
                            },
                        );
                    }
                    Family::Scan => {
                        let _ = events.emit(
                            "scan-progress",
                            json!({"current": job.current, "total": job.total, "file": job.last_file.as_deref().and_then(|path| Path::new(path).file_name()).map(|name| name.to_string_lossy().into_owned())}),
                        );
                    }
                    _ => {}
                }
            }
            if family == Family::Run {
                if let Some(snapshot) = job.resource_snapshot.as_ref() {
                    let encoded = serde_json::to_string(snapshot).unwrap_or_default();
                    if seen.resources.as_deref() != Some(encoded.as_str()) {
                        seen.resources = Some(encoded);
                        let _ = events.emit(
                            "run-resources",
                            crate::RunResourceEvent {
                                run_id: job.id.clone(),
                                snapshot: snapshot.clone(),
                            },
                        );
                    }
                }
            }
            if !is_active(job) && !seen.finished {
                seen.finished = true;
                match family {
                    Family::Run => {
                        let _ =
                            events.emit(
                                "run-finished",
                                RunFinished {
                                    run_id: job.id.clone(),
                                    exit_code: job
                                        .exit_code
                                        .or(Some(if job.status == "completed" { 0 } else { 1 })),
                                    stop_requested: job.status == "stopped",
                                    log_path: job.log_path.clone(),
                                    report_path: job.report_path.clone(),
                                    error: job.error.clone(),
                                },
                            );
                    }
                    Family::Repair => {
                        let _ = events.emit(
                            "repair-finished",
                            json!({"jobId": job.id, "success": job.status == "completed", "error": job.error, "logWarning": null}),
                        );
                    }
                    _ => {}
                }
            }
        }
        shared.reconcile_slot(&snapshot);
        self.seen
            .retain(|id, _| snapshot.iter().any(|job| &job.id == id));
    }
}

/// Poll the registry for the life of the process.
pub(super) fn spawn(jobs: Jobs, shared: Shared) {
    thread::spawn(move || {
        let mut bridge = Bridge::default();
        loop {
            bridge.tick(&jobs, &shared);
            thread::sleep(Duration::from_millis(200));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provision::{HeadlessRoute, Privilege};

    type Captured = Arc<Mutex<Vec<(String, Value)>>>;

    fn shared() -> (Shared, Captured) {
        let captured: Captured = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&captured);
        let shared = Shared {
            run_state: RunState::default(),
            events: Events::from_fn(move |name, payload| {
                sink.lock().unwrap().push((name.to_string(), payload))
            }),
            privilege: Arc::new(|| Privilege::Headless(HeadlessRoute::Unavailable)),
        };
        (shared, captured)
    }

    fn names(captured: &Captured) -> Vec<String> {
        captured
            .lock()
            .unwrap()
            .iter()
            .map(|(name, _)| name.clone())
            .collect()
    }

    fn running(server: &McpServer, id: &str, kind: &str) {
        let mut job = Job::new(id.into(), kind);
        job.status = "running".into();
        server.insert_job(job).expect("insert");
    }

    fn with_job(server: &McpServer, id: &str, change: impl FnOnce(&mut Job)) {
        let mut registry = server.jobs.lock().unwrap();
        change(registry.get_mut(id).expect("job"));
    }

    #[test]
    fn an_agent_job_takes_the_shared_slot_and_a_second_run_is_refused_until_it_finishes() {
        let (shared, _) = shared();
        let server = McpServer::new_shared(shared.clone());
        running(&server, "job-1", "integrity_check");
        assert_eq!(
            crate::active_job_info(&shared.run_state),
            Some(crate::ActiveJobInfo {
                id: "job-1".into(),
                kind: "integrity_check".into()
            })
        );
        let mut second = Job::new("job-2".into(), "media_run");
        second.status = "running".into();
        assert!(server.insert_job(second).unwrap_err().contains("active"));
        with_job(&server, "job-1", |job| job.status = "completed".into());
        let mut bridge = Bridge::default();
        bridge.tick(&server.jobs, &shared);
        assert_eq!(
            crate::active_job_info(&shared.run_state),
            None,
            "the slot is released when the job ends"
        );
        running(&server, "job-3", "media_run");
    }

    #[test]
    fn a_run_the_gui_started_blocks_an_agent_job() {
        let (shared, _) = shared();
        crate::claim_job(
            &shared.run_state,
            crate::new_active_job("gui-run".into(), None),
        )
        .unwrap();
        let server = McpServer::new_shared(shared.clone());
        let mut job = Job::new("job-1".into(), "media_run");
        job.status = "running".into();
        let error = server.insert_job(job).unwrap_err();
        assert!(error.contains("already active"), "{error}");
        assert!(
            server.jobs.lock().unwrap().is_empty(),
            "a refused job is not registered"
        );
        assert_eq!(
            crate::active_job_info(&shared.run_state).unwrap().id,
            "gui-run"
        );
    }

    #[test]
    fn two_folder_analyses_share_the_slot_and_it_passes_on_when_the_first_ends() {
        let (shared, _) = shared();
        let server = McpServer::new_shared(shared.clone());
        running(&server, "a-1", "folder_analysis");
        running(&server, "a-2", "folder_analysis");
        with_job(&server, "a-1", |job| job.status = "completed".into());
        let mut bridge = Bridge::default();
        bridge.tick(&server.jobs, &shared);
        assert_eq!(
            crate::active_job_info(&shared.run_state).map(|info| info.id),
            Some("a-2".into()),
            "still busy while one analysis runs"
        );
        with_job(&server, "a-2", |job| job.status = "completed".into());
        bridge.tick(&server.jobs, &shared);
        assert_eq!(crate::active_job_info(&shared.run_state), None);
    }

    #[test]
    fn the_stop_file_reaches_the_slot_so_the_gui_can_stop_an_agent_job() {
        let (shared, _) = shared();
        let server = McpServer::new_shared(shared.clone());
        running(&server, "job-1", "media_run");
        let stop = std::env::temp_dir().join("hevc-bridge-stop-file");
        with_job(&server, "job-1", |job| job.stop_file = Some(stop.clone()));
        Bridge::default().tick(&server.jobs, &shared);
        let held = shared.run_state.0.lock().unwrap().clone().unwrap();
        assert_eq!(held.stop_file, Some(stop));
    }

    #[test]
    fn registry_changes_become_the_desktop_events_the_gui_renders() {
        let (shared, captured) = shared();
        let server = McpServer::new_shared(shared.clone());
        let mut bridge = Bridge::default();
        running(&server, "job-1", "integrity_check");
        bridge.tick(&server.jobs, &shared);
        assert_eq!(names(&captured), ["job-started"]);
        assert_eq!(
            captured.lock().unwrap()[0].1,
            json!({"runId": "job-1", "kind": "integrity_check"})
        );

        record_line(&server.jobs, "job-1", "Checking a.mkv (1/2)".into());
        record_line(&server.jobs, "job-1", "a.mkv: INTEGRITY_OK".into());
        with_job(&server, "job-1", |job| {
            job.current = 1;
            job.total = 2;
            job.last_file = Some("/media/a.mkv".into());
        });
        bridge.tick(&server.jobs, &shared);
        bridge.tick(&server.jobs, &shared);
        let seen = captured.lock().unwrap().clone();
        let lines: Vec<_> = seen
            .iter()
            .filter(|(name, _)| name == "run-line")
            .map(|(_, payload)| payload["line"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(
            lines,
            ["Checking a.mkv (1/2)", "a.mkv: INTEGRITY_OK"],
            "each line once, in order, across ticks"
        );
        let progress: Vec<_> = seen
            .iter()
            .filter(|(name, _)| name == "run-progress")
            .collect();
        assert_eq!(progress.len(), 1, "an unchanged position is not re-sent");
        assert_eq!(progress[0].1["runId"], "job-1");
        assert_eq!(progress[0].1["fileName"], "a.mkv");
        assert_eq!(progress[0].1["batchNumber"], 1);
        assert_eq!(progress[0].1["batchTotal"], 2);

        with_job(&server, "job-1", |job| {
            job.status = "completed".into();
            job.exit_code = Some(0);
            job.log_path = Some("/logs/x.log".into());
            job.report_path = Some("/logs/x.csv".into());
        });
        bridge.tick(&server.jobs, &shared);
        bridge.tick(&server.jobs, &shared);
        let seen = captured.lock().unwrap().clone();
        let finished: Vec<_> = seen
            .iter()
            .filter(|(name, _)| name == "run-finished")
            .collect();
        assert_eq!(finished.len(), 1, "finished exactly once");
        assert_eq!(
            finished[0].1,
            json!({"runId": "job-1", "exitCode": 0, "stopRequested": false, "logPath": "/logs/x.log", "reportPath": "/logs/x.csv", "error": null})
        );
    }

    #[test]
    fn per_file_progress_detail_reaches_the_run_progress_event() {
        let (shared, captured) = shared();
        let server = McpServer::new_shared(shared.clone());
        running(&server, "job-1", "media_run");
        with_job(&server, "job-1", |job| {
            job.set_progress(
                2,
                5,
                "/media/b.mkv".into(),
                ProgressDetail {
                    phase: "Encoding".into(),
                    file_percent: Some(42.5),
                    queue_percent: Some(48.5),
                    speed: Some(1.25),
                    eta_seconds: Some(310.0),
                    elapsed_seconds: Some(95.0),
                    input_bytes: Some(1_000),
                    output_bytes: Some(400),
                    estimated_output_bytes: Some(900),
                },
            );
        });
        Bridge::default().tick(&server.jobs, &shared);
        let seen = captured.lock().unwrap().clone();
        let progress = seen
            .iter()
            .find(|(name, _)| name == "run-progress")
            .expect("a progress event")
            .1
            .clone();
        assert_eq!(progress["phase"], "Encoding");
        assert_eq!(progress["filePercent"], 42.5);
        assert_eq!(progress["queuePercent"], 48.5);
        assert_eq!(progress["speed"], 1.25);
        assert_eq!(progress["etaSeconds"], 310.0);
        assert_eq!(progress["elapsedSeconds"], 95.0);
        assert_eq!(
            (
                progress["inputBytes"].clone(),
                progress["outputBytes"].clone(),
                progress["estimatedOutputBytes"].clone()
            ),
            (json!(1000), json!(400), json!(900))
        );
        assert_eq!(
            (
                progress["batchNumber"].clone(),
                progress["batchTotal"].clone()
            ),
            (json!(2), json!(5))
        );
    }

    #[test]
    fn a_change_in_percent_alone_is_a_new_progress_event_and_an_unchanged_one_is_not() {
        let (shared, captured) = shared();
        let server = McpServer::new_shared(shared.clone());
        let mut bridge = Bridge::default();
        running(&server, "job-1", "media_run");
        let detail = |percent: f64| ProgressDetail {
            phase: "Encoding".into(),
            file_percent: Some(percent),
            ..ProgressDetail::default()
        };
        with_job(&server, "job-1", |job| {
            job.set_progress(1, 1, "/m/a.mkv".into(), detail(10.0))
        });
        bridge.tick(&server.jobs, &shared);
        bridge.tick(&server.jobs, &shared);
        with_job(&server, "job-1", |job| {
            job.set_progress(1, 1, "/m/a.mkv".into(), detail(20.0))
        });
        bridge.tick(&server.jobs, &shared);
        let percents: Vec<_> = captured
            .lock()
            .unwrap()
            .iter()
            .filter(|(name, _)| name == "run-progress")
            .map(|(_, payload)| payload["filePercent"].clone())
            .collect();
        assert_eq!(percents, [json!(10.0), json!(20.0)]);
    }

    #[test]
    fn the_job_json_an_agent_polls_carries_the_same_detail() {
        let mut job = Job::new("j".into(), "media_run");
        job.set_progress(
            1,
            3,
            "/m/a.mkv".into(),
            ProgressDetail {
                phase: "Encoding".into(),
                file_percent: Some(5.0),
                eta_seconds: Some(60.0),
                ..ProgressDetail::default()
            },
        );
        let value = serde_json::to_value(&job).unwrap();
        assert_eq!(value["progress_detail"]["file_percent"], 5.0);
        assert_eq!(value["progress_detail"]["eta_seconds"], 60.0);
        assert_eq!(
            (
                value["current"].clone(),
                value["total"].clone(),
                value["last_file"].clone()
            ),
            (json!(1), json!(3), json!("/m/a.mkv"))
        );
        let idle = serde_json::to_value(Job::new("k".into(), "media_run")).unwrap();
        assert!(
            idle.get("progress_detail").is_none(),
            "absent until a run reports progress"
        );
    }

    #[test]
    fn failed_and_stopped_runs_report_exit_codes_and_stop_state() {
        let (shared, captured) = shared();
        let server = McpServer::new_shared(shared.clone());
        let mut bridge = Bridge::default();
        running(&server, "bad", "media_run");
        with_job(&server, "bad", |job| {
            job.status = "failed".into();
            job.error = Some("ffmpeg missing".into());
        });
        bridge.tick(&server.jobs, &shared);
        running(&server, "halt", "media_run");
        with_job(&server, "halt", |job| job.status = "stopped".into());
        bridge.tick(&server.jobs, &shared);
        let seen = captured.lock().unwrap().clone();
        let finished: Vec<_> = seen
            .iter()
            .filter(|(name, _)| name == "run-finished")
            .map(|(_, payload)| payload.clone())
            .collect();
        assert_eq!(
            (
                finished[0]["exitCode"].clone(),
                finished[0]["error"].clone()
            ),
            (json!(1), json!("ffmpeg missing"))
        );
        assert_eq!(
            (
                finished[1]["stopRequested"].clone(),
                finished[1]["runId"].clone()
            ),
            (json!(true), json!("halt"))
        );
    }

    #[test]
    fn setup_and_analysis_jobs_use_the_repair_and_scan_events() {
        let (shared, captured) = shared();
        let server = McpServer::new_shared(shared.clone());
        let mut bridge = Bridge::default();
        running(&server, "setup", "system_provision");
        record_line(&server.jobs, "setup", "Administrator step".into());
        with_job(&server, "setup", |job| job.status = "completed".into());
        bridge.tick(&server.jobs, &shared);
        running(&server, "scan", "folder_analysis");
        with_job(&server, "scan", |job| {
            job.current = 3;
            job.total = 9;
            job.last_file = Some("/m/c.mkv".into());
        });
        bridge.tick(&server.jobs, &shared);
        let seen = captured.lock().unwrap().clone();
        assert!(seen.iter().any(|(name, payload)| name == "repair-line"
            && payload["line"] == "Administrator step"
            && payload["runId"] == "setup"));
        assert!(seen.iter().any(|(name, payload)| name == "repair-finished"
            && payload["jobId"] == "setup"
            && payload["success"] == true));
        assert!(seen.iter().any(|(name, payload)| name == "scan-progress"
            && payload == &json!({"current": 3, "total": 9, "file": "c.mkv"})));
        assert!(
            !seen
                .iter()
                .any(|(name, _)| name == "run-line" || name == "run-finished"),
            "no run events for setup or analysis"
        );
    }

    #[test]
    fn plain_stdio_servers_have_no_shared_state_and_behave_as_before() {
        let server = McpServer::new();
        running(&server, "job-1", "media_run");
        let mut second = Job::new("job-2".into(), "media_run");
        second.status = "running".into();
        assert!(
            server.insert_job(second).is_err(),
            "the registry's own one-job rule still applies"
        );
    }
}
