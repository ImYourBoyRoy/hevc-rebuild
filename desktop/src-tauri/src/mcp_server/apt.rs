//! The one-pass setup job: the host's package step (one prompt at most), then the user-space helper.

use super::*;

pub(super) fn run_provision_job(
    jobs: Jobs,
    id: String,
    plan: super::super::provision::ProvisionPlan,
    privilege: super::super::provision::Privilege,
) {
    let elevation = match privilege {
        super::super::provision::Privilege::Desktop => {
            super::super::provision_run::Elevation::Pkexec
        }
        super::super::provision::Privilege::Headless(_) => {
            super::super::provision_run::Elevation::SudoNonInteractive
        }
    };
    let sink_jobs = jobs.clone();
    let sink_id = id.clone();
    let sink: super::super::provision_run::LineSink =
        Arc::new(move |line| record_line(&sink_jobs, &sink_id, line));
    match super::super::provision_run::run_plan(&plan, elevation, sink) {
        Ok(outcome) => {
            let mut result = json!({"packages_applied": outcome.packages_applied});
            if let Some(path) = outcome.helper_installed_at {
                result["helper_installed_at"] = json!(path);
            }
            if let Ok(mut jobs) = jobs.lock() {
                if let Some(job) = jobs.get_mut(&id) {
                    job.status = "completed".into();
                    job.result = Some(result);
                }
            }
        }
        Err(error) => update_failed(&jobs, &id, error),
    }
}
