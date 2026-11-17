//! The single active-job slot behind the desktop run controls, the `Events` sink job runners emit into
//! (the desktop window or the headless server's bus), and the event payload types.

use super::*;

#[derive(Clone, Default)]
pub(crate) struct RunState(pub(crate) Arc<Mutex<Option<ActiveJob>>>);

#[derive(Clone)]
pub(crate) struct ActiveJob {
    pub(crate) id: String,
    /// What the job is, when a client other than the GUI started it (`integrity_check`, `media_run`, ...);
    /// empty for jobs the GUI starts itself.
    pub(crate) kind: String,
    pub(crate) stop_file: Option<PathBuf>,
    pub(crate) child_pid: Arc<Mutex<Option<u32>>>,
}

/// The job kind a run mode is announced under (the same names the MCP server uses), so a window that did
/// not start the run knows how to show it.
pub(crate) fn kind_for_run_mode(mode: &str) -> &'static str {
    if mode == "integrity" {
        "integrity_check"
    } else {
        "media_run"
    }
}

/// A job with a kind, for clients that follow jobs they did not start.
pub(crate) fn new_active_job_of(id: String, stop_file: Option<PathBuf>, kind: &str) -> ActiveJob {
    let mut job = new_active_job(id, stop_file);
    job.kind = kind.to_string();
    job
}

pub(crate) fn new_active_job(id: String, stop_file: Option<PathBuf>) -> ActiveJob {
    ActiveJob {
        id,
        kind: String::new(),
        stop_file,
        child_pid: Arc::new(Mutex::new(None)),
    }
}

/// The job that holds the single slot, for a client that wants to follow it (`active_job`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct ActiveJobInfo {
    pub(crate) id: String,
    pub(crate) kind: String,
}

pub(crate) fn active_job_info(state: &RunState) -> Option<ActiveJobInfo> {
    let current = state.0.lock().ok()?;
    current.as_ref().map(|job| ActiveJobInfo {
        id: job.id.clone(),
        kind: job.kind.clone(),
    })
}

fn remember_job_child(state: &RunState, run_id: &str, process_id: u32) {
    let child_pid = {
        let Ok(current) = state.0.lock() else {
            return;
        };
        let Some(job) = current.as_ref() else {
            return;
        };
        if job.id != run_id {
            return;
        }
        Arc::clone(&job.child_pid)
    };
    let Ok(mut child) = child_pid.lock() else {
        return;
    };
    *child = Some(process_id);
}

fn forget_job_child(state: &RunState, run_id: &str) {
    let Ok(current) = state.0.lock() else {
        return;
    };
    let Some(job) = current.as_ref().filter(|job| job.id == run_id) else {
        return;
    };
    let child_pid = Arc::clone(&job.child_pid);
    drop(current);
    let cleared = child_pid.lock().map(|mut child| *child = None);
    drop(cleared);
}

/// Where a job's events go: the desktop window (Tauri emit) or, in `serve` mode, the server's event
/// bus. Job runners only know this type, so one implementation serves both front ends.
#[derive(Clone)]
pub(crate) struct Events(Arc<EventSink>);

type EventSink = dyn Fn(&str, Value) + Send + Sync;

impl Events {
    #[cfg(feature = "desktop")]
    pub(crate) fn from_tauri(app: AppHandle) -> Events {
        Events::from_fn(move |event, payload| {
            let _ = app.emit(event, payload);
        })
    }

    pub(crate) fn from_fn(sink: impl Fn(&str, Value) + Send + Sync + 'static) -> Events {
        Events(Arc::new(sink))
    }

    /// Same shape as Tauri's `emit`, so runner code reads the same in both front ends.
    pub(crate) fn emit<S: Serialize>(&self, event: &str, payload: S) -> Result<(), String> {
        let value = serde_json::to_value(payload)
            .map_err(|error| format!("Could not encode `{event}`: {error}"))?;
        (self.0)(event, value);
        Ok(())
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RunLine {
    pub(crate) run_id: String,
    pub(crate) line: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RunProgress {
    pub(crate) run_id: String,
    pub(crate) file_name: String,
    pub(crate) file_path: Option<String>,
    pub(crate) phase: String,
    pub(crate) batch_number: u32,
    pub(crate) batch_total: u32,
    pub(crate) file_percent: Option<f64>,
    pub(crate) queue_percent: Option<f64>,
    pub(crate) speed: Option<f64>,
    pub(crate) eta_seconds: Option<f64>,
    pub(crate) elapsed_seconds: Option<f64>,
    pub(crate) input_bytes: Option<u64>,
    pub(crate) output_bytes: Option<u64>,
    pub(crate) estimated_output_bytes: Option<u64>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RunResourceEvent {
    pub(crate) run_id: String,
    pub(crate) snapshot: resource_monitor::RuntimeResourceSnapshot,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RunFinished {
    pub(crate) run_id: String,
    pub(crate) exit_code: Option<i32>,
    pub(crate) stop_requested: bool,
    pub(crate) log_path: Option<String>,
    pub(crate) report_path: Option<String>,
    pub(crate) error: Option<String>,
}

pub(crate) fn new_job_id() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{}-{now}", std::process::id())
}

pub(crate) fn claim_job(state: &RunState, job: ActiveJob) -> Result<(), String> {
    let mut current = state
        .0
        .lock()
        .map_err(|_| "Could not lock the desktop job state.")?;
    if current.is_some() {
        return Err("Another run, scan, or repair is already active.".into());
    }
    *current = Some(job);
    Ok(())
}

pub(crate) fn release_job(state: &RunState, id: &str) {
    if let Ok(mut current) = state.0.lock() {
        if current.as_ref().is_some_and(|job| job.id == id) {
            *current = None;
        }
    }
}

pub(crate) fn child_tracker<'a>(
    state: &'a RunState,
    run_id: &'a str,
) -> impl FnMut(Option<u32>) + 'a {
    move |process_id| match process_id {
        Some(process_id) => remember_job_child(state, run_id, process_id),
        None => forget_job_child(state, run_id),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_deliver_the_event_name_and_camel_case_payload_to_the_sink() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let events = {
            let seen = Arc::clone(&seen);
            Events::from_fn(move |name, payload| {
                seen.lock().unwrap().push((name.to_string(), payload))
            })
        };
        events
            .emit(
                "run-line",
                RunLine {
                    run_id: "r1".into(),
                    line: "frame=1".into(),
                },
            )
            .expect("emit");
        let clone = events.clone();
        clone
            .emit("run-finished", json!({"runId": "r1"}))
            .expect("emit from a clone");
        let seen = seen.lock().unwrap();
        assert_eq!(
            seen[0],
            (
                "run-line".to_string(),
                json!({"runId": "r1", "line": "frame=1"})
            )
        );
        assert_eq!(seen[1].0, "run-finished");
    }

    #[test]
    fn gui_started_jobs_carry_a_kind_so_another_window_can_follow_them() {
        assert_eq!(kind_for_run_mode("integrity"), "integrity_check");
        for mode in ["check", "metadata", "test", "full", "youtube"] {
            assert_eq!(kind_for_run_mode(mode), "media_run", "{mode}");
        }
        let state = RunState::default();
        claim_job(
            &state,
            new_active_job_of("scan-1".into(), None, "folder_analysis"),
        )
        .unwrap();
        assert_eq!(active_job_info(&state).unwrap().kind, "folder_analysis");
    }

    #[test]
    fn active_job_info_reports_the_slot_holder_and_nothing_when_idle() {
        let state = RunState::default();
        assert_eq!(active_job_info(&state), None);
        let mut job = new_active_job("run-a".into(), None);
        job.kind = "integrity_check".into();
        claim_job(&state, job).unwrap();
        assert_eq!(
            active_job_info(&state),
            Some(ActiveJobInfo {
                id: "run-a".into(),
                kind: "integrity_check".into()
            })
        );
        release_job(&state, "run-a");
        assert_eq!(active_job_info(&state), None);
    }

    #[test]
    fn job_state_allows_one_active_job_and_tracks_its_child_process() {
        let state = RunState::default();
        claim_job(&state, new_active_job("run-a".into(), None)).expect("claim the idle state");
        let second = claim_job(&state, new_active_job("run-b".into(), None));
        assert_eq!(
            second.unwrap_err(),
            "Another run, scan, or repair is already active."
        );

        remember_job_child(&state, "other", 11);
        remember_job_child(&state, "run-a", 42);
        let pid = |state: &RunState| {
            let current = state.0.lock().unwrap();
            let job = current.as_ref().expect("the job stays claimed");
            let child = *job.child_pid.lock().unwrap();
            child
        };
        assert_eq!(pid(&state), Some(42));
        forget_job_child(&state, "other");
        assert_eq!(pid(&state), Some(42));
        child_tracker(&state, "run-a")(None);
        assert_eq!(pid(&state), None);
        child_tracker(&state, "run-a")(Some(7));
        assert_eq!(pid(&state), Some(7));

        release_job(&state, "run-b");
        assert!(state.0.lock().unwrap().is_some());
        release_job(&state, "run-a");
        assert!(state.0.lock().unwrap().is_none());
        claim_job(&state, new_active_job("run-b".into(), None)).expect("claim after release");
    }

    #[test]
    fn job_ids_start_with_the_process_id_and_differ_between_calls() {
        let first = new_job_id();
        let second = new_job_id();
        assert!(first.starts_with(&format!("{}-", std::process::id())));
        assert_ne!(first, second);
    }
}
