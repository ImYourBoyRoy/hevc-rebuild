//! Ubuntu package jobs (refresh, upgrade, repair) run through allow-listed apt commands.

use super::*;

pub(super) fn run_apt_job(jobs: Jobs, id: String, packages: Vec<String>) {
    if !cfg!(target_os = "linux") {
        update_failed(
            &jobs,
            &id,
            "APT package operations are available only on Linux hosts.".into(),
        );
        return;
    }
    let operations = [vec!["apt-get".to_string(), "update".to_string()], {
        let mut install = vec![
            "apt-get".into(),
            "install".into(),
            "--yes".into(),
            "--no-upgrade".into(),
            "--".into(),
        ];
        install.extend(packages);
        install
    }];
    for (index, mut operation) in operations.into_iter().enumerate() {
        if unsafe { geteuid() } != 0 {
            operation.insert(0, "pkexec".into());
        }
        let Some(program) = operation.first().cloned() else {
            continue;
        };
        let result = crate::process_output::spawn_owned(
            Command::new(program)
                .args(operation.iter().skip(1))
                .stdout(Stdio::piped())
                .stderr(Stdio::piped()),
        );
        let mut child = match result {
            Ok(child) => child,
            Err(error) => {
                update_failed(
                    &jobs,
                    &id,
                    format!("Could not start the package manager: {error}"),
                );
                return;
            }
        };
        let process_id = child.id();
        let stderr = match child.stderr.take() {
            Some(stream) => stream,
            None => {
                update_failed(&jobs, &id, "Could not read APT diagnostics.".into());
                return;
            }
        };
        let jobs_for_err = jobs.clone();
        let err_id = id.clone();
        let thread = thread::spawn(move || {
            crate::process_output::read_process_lines(stderr, process_id, |line| {
                record_line(&jobs_for_err, &err_id, line);
            })
        });
        let mut output_result = Ok(());
        if let Some(stdout) = child.stdout.take() {
            output_result = crate::process_output::read_process_lines(stdout, process_id, |line| {
                record_line(&jobs, &id, line);
            });
        }
        let diagnostic_result = thread
            .join()
            .unwrap_or_else(|_| Err("APT diagnostic reader panicked.".into()));
        let reader_result = output_result.and(diagnostic_result);
        match child.wait() {
            Ok(_) if reader_result.is_err() => {
                update_failed(&jobs, &id, reader_result.unwrap_err());
                return;
            }
            Ok(status) if status.success() => {}
            Ok(status) => {
                update_failed(
                    &jobs,
                    &id,
                    format!("APT operation {} exited with {status}.", index + 1),
                );
                return;
            }
            Err(error) => {
                update_failed(&jobs, &id, format!("Could not wait for APT: {error}"));
                return;
            }
        }
    }
    if let Ok(mut jobs) = jobs.lock() {
        if let Some(job) = jobs.get_mut(&id) {
            job.status = "completed".into();
        }
    }
}

pub(super) fn run_apt_command_job(jobs: Jobs, id: String, mut operation: Vec<String>) {
    if !cfg!(target_os = "linux") {
        update_failed(
            &jobs,
            &id,
            "APT package operations are available only on Linux hosts.".into(),
        );
        return;
    }
    if unsafe { geteuid() } != 0 {
        operation.insert(0, "pkexec".into());
    }
    let Some(program) = operation.first().cloned() else {
        update_failed(&jobs, &id, "The package update command was empty.".into());
        return;
    };
    let result = crate::process_output::spawn_owned(
        Command::new(program)
            .args(operation.iter().skip(1))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    );
    let mut child = match result {
        Ok(child) => child,
        Err(error) => {
            update_failed(
                &jobs,
                &id,
                format!("Could not start the APT package operation: {error}"),
            );
            return;
        }
    };
    let process_id = child.id();
    let Some(stderr) = child.stderr.take() else {
        update_failed(&jobs, &id, "Could not read APT updater diagnostics.".into());
        return;
    };
    let jobs_for_err = jobs.clone();
    let error_id = id.clone();
    let stderr_thread = thread::spawn(move || {
        crate::process_output::read_process_lines(stderr, process_id, |line| {
            record_line(&jobs_for_err, &error_id, line);
        })
    });
    let mut output_result = Ok(());
    if let Some(stdout) = child.stdout.take() {
        output_result = crate::process_output::read_process_lines(stdout, process_id, |line| {
            record_line(&jobs, &id, line);
        });
    }
    let diagnostic_result = stderr_thread
        .join()
        .unwrap_or_else(|_| Err("APT diagnostic reader panicked.".into()));
    let reader_result = output_result.and(diagnostic_result);
    match child.wait() {
        Ok(_) if reader_result.is_err() => {
            update_failed(&jobs, &id, reader_result.unwrap_err());
        }
        Ok(status) if status.success() => {
            if let Ok(mut jobs) = jobs.lock() {
                if let Some(job) = jobs.get_mut(&id) {
                    job.status = "completed".into();
                }
            }
        }
        Ok(status) => update_failed(
            &jobs,
            &id,
            format!("APT package operation exited with {status}."),
        ),
        Err(error) => update_failed(
            &jobs,
            &id,
            format!("Could not wait for the APT package operation: {error}"),
        ),
    }
}

pub(super) fn run_apt_update_job(jobs: Jobs, id: String, packages: Vec<String>) {
    let mut operation = vec![
        "apt-get".to_string(),
        "install".into(),
        "--only-upgrade".into(),
        "--no-install-recommends".into(),
        "--yes".into(),
        "--".into(),
    ];
    operation.extend(packages);
    run_apt_command_job(jobs, id, operation);
}

#[cfg(target_os = "linux")]
pub(super) unsafe fn geteuid() -> u32 {
    unsafe extern "C" {
        fn geteuid() -> u32;
    }
    geteuid()
}
#[cfg(not(target_os = "linux"))]
pub(super) unsafe fn geteuid() -> u32 {
    1
}
