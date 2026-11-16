//! Tauri commands that check, refresh, upgrade and repair Ubuntu packages and install HDR10+ tooling.

use super::*;

fn emit_repair_line(
    app: &AppHandle,
    logger: &tool_diagnostics::ToolSetupLogger,
    job_id: &str,
    line: String,
) {
    logger.record(&line);
    let _ = app.emit(
        "repair-line",
        RunLine {
            run_id: job_id.to_string(),
            line,
        },
    );
}

fn forward_repair_lines<R: Read + Send + 'static>(
    app: AppHandle,
    job_id: String,
    logger: Arc<tool_diagnostics::ToolSetupLogger>,
    stream: R,
    process_id: u32,
) -> thread::JoinHandle<Result<(), String>> {
    thread::spawn(move || {
        crate::process_output::read_process_lines(stream, process_id, |line| {
            emit_repair_line(&app, &logger, &job_id, line);
        })
    })
}

#[tauri::command]
pub(crate) async fn check_ubuntu_package_updates() -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(dependencies::apt_update_status)
        .await
        .map_err(|error| format!("Ubuntu package update check failed: {error}"))?
}

fn start_apt_command_job(
    app: AppHandle,
    state: &RunState,
    command_parts: Vec<String>,
) -> Result<String, String> {
    start_apt_commands_job(app, state, vec![command_parts])
}

fn start_apt_commands_job(
    app: AppHandle,
    state: &RunState,
    commands: Vec<Vec<String>>,
) -> Result<String, String> {
    if !cfg!(target_os = "linux") {
        return Err("APT package operations are available only on Linux hosts.".into());
    }
    if commands.is_empty() || commands.iter().any(Vec::is_empty) {
        return Err("The Ubuntu package operation has no valid commands.".into());
    }
    let id = new_job_id();
    claim_job(state, new_active_job(id.clone(), None))?;
    let run_state = state.clone();
    let worker_id = id.clone();
    thread::spawn(move || {
        let app_handle = app;
        let logger = Arc::new(match tool_setup_log_directory() {
            Ok(directory) => tool_diagnostics::ToolSetupLogger::create(
                &directory,
                &worker_id,
                "Ubuntu package setup",
            ),
            Err(error) => tool_diagnostics::ToolSetupLogger::unavailable(error),
        });
        let mut success = false;
        let mut error_message = None;
        for (index, mut command_parts) in commands.into_iter().enumerate() {
            if unsafe { libc_geteuid() } != 0 {
                command_parts.insert(0, "pkexec".into());
            }
            let Some(program) = command_parts.first().cloned() else {
                error_message = Some("The Ubuntu package operation has no executable.".into());
                break;
            };
            let stage = format!(
                "Package setup step {} · {}",
                index + 1,
                command_parts.join(" ")
            );
            emit_repair_line(&app_handle, &logger, &worker_id, stage);
            match crate::process_output::spawn_owned(
                Command::new(program)
                    .args(command_parts.iter().skip(1))
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped()),
            ) {
                Ok(mut child) => {
                    let stderr_thread = child.stderr.take().map(|stream| {
                        forward_repair_lines(
                            app_handle.clone(),
                            worker_id.clone(),
                            logger.clone(),
                            stream,
                            child.id(),
                        )
                    });
                    let mut output_result = Ok(());
                    if let Some(stdout) = child.stdout.take() {
                        output_result =
                            crate::process_output::read_process_lines(stdout, child.id(), |line| {
                                emit_repair_line(&app_handle, &logger, &worker_id, line);
                            });
                    }
                    let diagnostic_result = match stderr_thread {
                        Some(handle) => handle
                            .join()
                            .unwrap_or_else(|_| Err("Package diagnostic reader panicked.".into())),
                        None => Ok(()),
                    };
                    let reader_result = output_result.and(diagnostic_result);
                    match child.wait() {
                        Ok(_) if reader_result.is_err() => {
                            success = false;
                            error_message = reader_result.err();
                        }
                        Ok(status) if status.success() => success = true,
                        Ok(status) => {
                            error_message =
                                Some(format!("APT package operation exited with {status}."));
                        }
                        Err(error) => {
                            error_message = Some(format!("Could not wait for APT: {error}"));
                        }
                    }
                }
                Err(error) => {
                    error_message = Some(format!(
                        "Could not start the APT package operation: {error}"
                    ));
                }
            }
            if error_message.is_some() {
                success = false;
                break;
            }
        }
        logger.finish(success, error_message.as_deref());
        let _ = app_handle.emit(
            "repair-finished",
            json!({"jobId": worker_id, "success": success, "error": error_message, "logWarning": logger.warning()}),
        );
        release_job(&run_state, &worker_id);
    });
    Ok(id)
}

fn start_apt_update_job(
    app: AppHandle,
    state: &RunState,
    packages: Vec<String>,
) -> Result<String, String> {
    let mut operation = vec![
        "apt-get".to_string(),
        "install".into(),
        "--only-upgrade".into(),
        "--no-install-recommends".into(),
        "--no-remove".into(),
        "--yes".into(),
        "--".into(),
    ];
    operation.extend(packages);
    start_apt_command_job(app, state, operation)
}

#[tauri::command]
pub(crate) async fn refresh_ubuntu_package_catalog(
    app: AppHandle,
    state: State<'_, RunState>,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(|| -> Result<(), String> {
        let environment = environment::inspect_environment(ToolPaths::default(), None)?;
        if environment
            .pointer("/os/supported")
            .and_then(Value::as_bool)
            != Some(true)
        {
            return Err(
                "APT catalog refresh is limited to supported Ubuntu and Linux Mint releases."
                    .into(),
            );
        }
        if environment
            .pointer("/os/package_manager")
            .and_then(Value::as_str)
            != Some("apt")
            || environment
                .pointer("/os/package_manager_available")
                .and_then(Value::as_bool)
                != Some(true)
        {
            return Err(
                "APT catalog refresh is available only when APT is installed on Linux.".into(),
            );
        }
        if unsafe { libc_geteuid() } != 0 && !executable_in_path("pkexec") {
            return Err("Desktop administrator authentication (pkexec) is not installed.".into());
        }
        Ok(())
    })
    .await
    .map_err(|error| format!("APT catalog refresh preflight failed: {error}"))??;
    start_apt_command_job(app, state.inner(), vec!["apt-get".into(), "update".into()])
}

#[tauri::command]
pub(crate) async fn upgrade_ubuntu_packages(
    app: AppHandle,
    state: State<'_, RunState>,
    confirmed_package_names: Vec<String>,
) -> Result<String, String> {
    let packages = tauri::async_runtime::spawn_blocking(move || -> Result<Vec<String>, String> {
        let status = dependencies::apt_update_status()?;
        let packages = validate_apt_update_confirmation(&status, confirmed_package_names)?;
        validate_apt_upgrade_plan(&packages)?;
        if unsafe { libc_geteuid() } != 0 && !executable_in_path("pkexec") {
            return Err("Desktop administrator authentication (pkexec) is not installed.".into());
        }
        Ok(packages)
    })
    .await
    .map_err(|error| format!("APT update preflight failed: {error}"))??;
    start_apt_update_job(app, state.inner(), packages)
}

#[tauri::command]
pub(crate) async fn install_ubuntu_toolchain(
    app: AppHandle,
    state: State<'_, RunState>,
    tool_paths: ToolPaths,
    confirmed_missing_packages: Vec<String>,
    confirmed_update_packages: Vec<String>,
) -> Result<String, String> {
    let (missing, updates) = tauri::async_runtime::spawn_blocking(move || -> Result<_, String> {
        let environment = environment::inspect_environment(tool_paths, None)?;
        if environment
            .pointer("/os/supported")
            .and_then(Value::as_bool)
            != Some(true)
        {
            return Err(
                "Toolchain package repair is limited to supported Ubuntu and Linux Mint releases."
                    .into(),
            );
        }
        let status = dependencies::apt_update_status()?;
        let (missing, updates) = validate_apt_toolchain_confirmation(
            &environment,
            &status,
            confirmed_missing_packages,
            confirmed_update_packages,
        )?;
        if unsafe { libc_geteuid() } != 0 && !executable_in_path("pkexec") {
            return Err("Desktop administrator authentication (pkexec) is not installed.".into());
        }
        Ok((missing, updates))
    })
    .await
    .map_err(|error| format!("Ubuntu toolchain setup preflight failed: {error}"))??;

    let mut commands = Vec::new();
    if !missing.is_empty() {
        let mut install = vec![
            "apt-get".into(),
            "install".into(),
            "--yes".into(),
            "--no-upgrade".into(),
            "--no-install-recommends".into(),
            "--no-remove".into(),
            "--".into(),
        ];
        install.extend(missing);
        commands.push(install);
    }
    if !updates.is_empty() {
        let mut upgrade = vec![
            "apt-get".into(),
            "install".into(),
            "--only-upgrade".into(),
            "--no-install-recommends".into(),
            "--no-remove".into(),
            "--yes".into(),
            "--".into(),
        ];
        upgrade.extend(updates);
        commands.push(upgrade);
    }
    start_apt_commands_job(app, state.inner(), commands)
}

#[tauri::command]
pub(crate) fn install_apt_tools(
    app: AppHandle,
    state: State<'_, RunState>,
    packages: Vec<String>,
) -> Result<String, String> {
    if !cfg!(target_os = "linux") {
        return Err("APT package operations are available only on Linux hosts.".into());
    }
    if packages.is_empty()
        || packages
            .iter()
            .any(|package| !APT_ALLOWLIST.contains(&package.as_str()))
    {
        return Err("APT repair accepts only the application’s fixed Ubuntu package list.".into());
    }
    let id = new_job_id();
    claim_job(state.inner(), new_active_job(id.clone(), None))?;
    let run_state = state.inner().clone();
    let worker_id = id.clone();
    thread::spawn(move || {
        let app_handle = app;
        let logger = Arc::new(match tool_setup_log_directory() {
            Ok(directory) => tool_diagnostics::ToolSetupLogger::create(
                &directory,
                &worker_id,
                "Ubuntu package repair",
            ),
            Err(error) => tool_diagnostics::ToolSetupLogger::unavailable(error),
        });
        let mut last_error = None;
        let prefix = if cfg!(target_os = "linux") && unsafe { libc_geteuid() } != 0 {
            Some("pkexec")
        } else {
            None
        };
        let commands = [vec!["apt-get".to_string(), "update".to_string()], {
            let mut command = vec![
                "apt-get".into(),
                "install".into(),
                "--yes".into(),
                "--no-upgrade".into(),
                "--".into(),
            ];
            command.extend(packages);
            command
        }];
        let mut success = true;
        for mut parts in commands {
            if let Some(elevation) = prefix {
                parts.insert(0, elevation.to_string());
            }
            let Some(program) = parts.first().cloned() else {
                continue;
            };
            emit_repair_line(
                &app_handle,
                &logger,
                &worker_id,
                format!("Running package repair command: {}", parts.join(" ")),
            );
            let mut command = Command::new(program);
            command
                .args(parts.iter().skip(1))
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            match crate::process_output::spawn_owned(&mut command) {
                Ok(mut child) => {
                    let stderr_thread = child.stderr.take().map(|stream| {
                        forward_repair_lines(
                            app_handle.clone(),
                            worker_id.clone(),
                            logger.clone(),
                            stream,
                            child.id(),
                        )
                    });
                    let mut output_result = Ok(());
                    if let Some(stdout) = child.stdout.take() {
                        output_result =
                            crate::process_output::read_process_lines(stdout, child.id(), |line| {
                                emit_repair_line(&app_handle, &logger, &worker_id, line);
                            });
                    }
                    let diagnostic_result = match stderr_thread {
                        Some(handle) => handle
                            .join()
                            .unwrap_or_else(|_| Err("Package diagnostic reader panicked.".into())),
                        None => Ok(()),
                    };
                    let reader_result = output_result.and(diagnostic_result);
                    match child.wait() {
                        Ok(_) if reader_result.is_err() => {
                            success = false;
                            last_error = reader_result.err();
                        }
                        Ok(status) if status.success() => {}
                        Ok(status) => {
                            success = false;
                            last_error = Some(format!(
                                "APT exited with status {status}; inspect the repair output."
                            ));
                            break;
                        }
                        Err(error) => {
                            success = false;
                            last_error = Some(format!("Could not wait for APT: {error}"));
                            break;
                        }
                    }
                }
                Err(error) => {
                    success = false;
                    last_error = Some(format!("Could not start the package manager: {error}"));
                    break;
                }
            }
        }
        logger.finish(success, last_error.as_deref());
        let _ = app_handle.emit(
            "repair-finished",
            json!({"jobId": worker_id, "success": success, "error": last_error, "logWarning": logger.warning()}),
        );
        release_job(&run_state, &worker_id);
    });
    Ok(id)
}

#[cfg(target_os = "linux")]
pub(crate) unsafe fn libc_geteuid() -> u32 {
    unsafe extern "C" {
        fn geteuid() -> u32;
    }
    geteuid()
}

#[cfg(not(target_os = "linux"))]
pub(crate) unsafe fn libc_geteuid() -> u32 {
    1
}

#[tauri::command]
pub(crate) async fn install_hdr10plus(
    app: AppHandle,
    state: State<'_, RunState>,
    job_id: String,
) -> Result<String, String> {
    tool_diagnostics::validate_job_id(&job_id)?;
    let log_directory = tool_setup_log_directory();
    let id = new_job_id();
    claim_job(state.inner(), new_active_job(id.clone(), None))?;
    let run_state = state.inner().clone();
    let logger = Arc::new(match log_directory {
        Ok(directory) => tool_diagnostics::ToolSetupLogger::create(
            &directory,
            &job_id,
            "HDR10+ helper installation",
        ),
        Err(error) => tool_diagnostics::ToolSetupLogger::unavailable(error),
    });
    logger.record("Downloading the helper release and verifying its published digest.");
    let task =
        tauri::async_runtime::spawn_blocking(dependencies::install_latest_hdr10plus_tool).await;
    let result = match task {
        Ok(result) => result,
        Err(error) => Err(format!("HDR10+ helper setup task failed: {error}")),
    };
    match &result {
        Ok(path) => logger.record(&format!("HDR10+ helper installed at {path}.")),
        Err(error) => logger.record(&format!("ERROR: {error}")),
    }
    logger.finish(result.is_ok(), result.as_ref().err().map(String::as_str));
    if let Some(warning) = logger.warning() {
        let _ = app.emit(
            "repair-log-warning",
            RunLine {
                run_id: job_id,
                line: warning,
            },
        );
    }
    release_job(&run_state, &id);
    result
}
