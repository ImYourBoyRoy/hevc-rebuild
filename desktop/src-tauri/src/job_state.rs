//! The single active-job slot behind the desktop run controls, plus the events a job emits to the window.

use super::*;

#[derive(Clone, Default)]
pub(crate) struct RunState(pub(crate) Arc<Mutex<Option<ActiveJob>>>);

#[derive(Clone)]
pub(crate) struct ActiveJob {
    pub(crate) id: String,
    pub(crate) stop_file: Option<PathBuf>,
    pub(crate) child_pid: Arc<Mutex<Option<u32>>>,
}

pub(crate) fn new_active_job(id: String, stop_file: Option<PathBuf>) -> ActiveJob {
    ActiveJob {
        id,
        stop_file,
        child_pid: Arc::new(Mutex::new(None)),
    }
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
