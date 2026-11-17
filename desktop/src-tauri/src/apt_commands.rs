//! Tauri commands that check, refresh, upgrade and repair Ubuntu packages and install HDR10+ tooling.

use super::*;

fn emit_repair_line(
    app: &Events,
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

#[cfg_attr(feature = "desktop", tauri::command)]
pub(crate) async fn check_ubuntu_package_updates() -> Result<Value, String> {
    crate::runtime::spawn_blocking(dependencies::apt_update_status)
        .await
        .map_err(|error| format!("Ubuntu package update check failed: {error}"))?
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

#[cfg_attr(feature = "desktop", tauri::command)]
#[cfg_attr(not(feature = "desktop"), allow(dead_code))]
pub(crate) async fn provision_plan(tool_paths: ToolPaths) -> Result<Value, String> {
    provision_plan_for(tool_paths, provision::Privilege::Desktop).await
}

/// A headless plan also says which privilege route it will use (`root`, `sudo-n`, `none`) and asks for
/// no password prompt at all, since none can be answered.
pub(crate) fn annotate_plan_json(value: &mut Value, privilege: provision::Privilege) {
    if let (provision::Privilege::Headless(route), Some(object)) =
        (privilege, value.as_object_mut())
    {
        object.insert("elevation".into(), json!(route.key()));
        object.insert("password_prompts".into(), json!(0));
    }
}

/// The plan JSON for a front end. A headless plan also reports the privilege route it will use.
pub(crate) async fn provision_plan_for(
    tool_paths: ToolPaths,
    privilege: provision::Privilege,
) -> Result<Value, String> {
    crate::runtime::spawn_blocking(move || {
        provision::current_plan_for(tool_paths, privilege).map(|(plan, error)| {
            let mut value = provision::plan_json(&plan, error.as_deref());
            annotate_plan_json(&mut value, privilege);
            value
        })
    })
    .await
    .map_err(|error| format!("Setup plan failed: {error}"))?
}

/// Install and update everything in one pass: a single administrator prompt covers the catalog
/// refresh, missing packages and package updates; the HDR10+ helper is then installed or updated
/// in user space. The plan is recomputed here so the job never runs a stale or caller-supplied list.
#[cfg(feature = "desktop")]
#[tauri::command]
pub(crate) async fn provision_toolchain(
    app: AppHandle,
    state: State<'_, RunState>,
    tool_paths: ToolPaths,
    user_space_only: Option<bool>,
) -> Result<String, String> {
    provision_toolchain_with(
        Events::from_tauri(app),
        state.inner(),
        tool_paths,
        user_space_only,
        provision::Privilege::Desktop,
    )
    .await
}

/// Runtime-free body of `provision_toolchain`, shared with the headless server, which passes the
/// privilege route it detected instead of the desktop prompt.
pub(crate) async fn provision_toolchain_with(
    app: Events,
    state: &RunState,
    tool_paths: ToolPaths,
    user_space_only: Option<bool>,
    privilege: provision::Privilege,
) -> Result<String, String> {
    let elevation = match privilege {
        provision::Privilege::Desktop => provision_run::Elevation::Pkexec,
        provision::Privilege::Headless(_) => provision_run::Elevation::SudoNonInteractive,
    };
    let (plan, _) =
        crate::runtime::spawn_blocking(move || provision::current_plan_for(tool_paths, privilege))
            .await
            .map_err(|error| format!("Setup plan failed: {error}"))??;
    // Automatic updates pass `userSpaceOnly`: the backend itself drops anything that needs a password.
    let plan = if user_space_only == Some(true) {
        plan.user_space_only()
    } else {
        plan
    };
    if plan.nothing_to_do() {
        return Err("Everything is already installed and up to date.".into());
    }
    if !plan.has_privileged_work() && !plan.helper.needs_work() {
        return Err("Finish the listed prerequisites first, then run setup again.".into());
    }
    let id = new_job_id();
    claim_job(
        state,
        new_active_job_of(id.clone(), None, "system_provision"),
    )?;
    let run_state = state.clone();
    let worker_id = id.clone();
    thread::spawn(move || {
        let app_handle = app;
        let logger = Arc::new(match tool_setup_log_directory() {
            Ok(directory) => tool_diagnostics::ToolSetupLogger::create(
                &directory,
                &worker_id,
                "One-pass toolchain setup",
            ),
            Err(error) => tool_diagnostics::ToolSetupLogger::unavailable(error),
        });
        let sink_app = app_handle.clone();
        let sink_logger = logger.clone();
        let sink_id = worker_id.clone();
        let sink: provision_run::LineSink =
            Arc::new(move |line| emit_repair_line(&sink_app, &sink_logger, &sink_id, line));
        let outcome = provision_run::run_plan(&plan, elevation, sink);
        let error_message = outcome.err();
        logger.finish(error_message.is_none(), error_message.as_deref());
        let _ = app_handle.emit(
            "repair-finished",
            json!({"jobId": worker_id, "success": error_message.is_none(), "error": error_message, "logWarning": logger.warning()}),
        );
        release_job(&run_state, &worker_id);
    });
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headless_plans_report_their_route_and_never_a_password_prompt() {
        let mut plan = json!({"needs_password": true, "password_prompts": 1});
        annotate_plan_json(
            &mut plan,
            provision::Privilege::Headless(provision::HeadlessRoute::SudoNoPassword),
        );
        assert_eq!(plan["elevation"], "sudo-n");
        assert_eq!(plan["password_prompts"], 0);
        let mut none = json!({"password_prompts": 1});
        annotate_plan_json(
            &mut none,
            provision::Privilege::Headless(provision::HeadlessRoute::Unavailable),
        );
        assert_eq!(none["elevation"], "none");
    }

    #[test]
    fn desktop_plans_are_left_exactly_as_they_were() {
        let mut plan = json!({"needs_password": true, "password_prompts": 1});
        annotate_plan_json(&mut plan, provision::Privilege::Desktop);
        assert_eq!(plan, json!({"needs_password": true, "password_prompts": 1}));
    }
}
