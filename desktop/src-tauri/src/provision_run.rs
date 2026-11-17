//! Runs a `ProvisionPlan` for the GUI, CLI and MCP with one implementation: the package step for
//! the host's manager (apt under one administrator prompt, Homebrew as the user, winget under one
//! Windows elevation prompt), then the user-space HDR10+ helper. Callers only supply how lines of
//! output are shown; none of them ever sees or handles a password.

use super::provision::{
    brew_commands, privileged_script, windows_elevation_arguments, winget_script, Manager,
    ProvisionPlan,
};
use super::*;

/// How a Unix host is elevated for the single administrator step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Elevation {
    /// Desktop prompt (GUI and MCP).
    Pkexec,
    /// Terminal prompt (CLI).
    Sudo,
    /// No prompt possible (headless server): `sudo -n` succeeds or fails at once, and no password is
    /// ever read.
    SudoNonInteractive,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommandSpec {
    pub(crate) description: String,
    pub(crate) program: String,
    pub(crate) args: Vec<String>,
    pub(crate) env: Vec<(String, String)>,
    /// A file the command writes progress to (elevated Windows runs cannot pipe their output).
    pub(crate) log_file: Option<PathBuf>,
}

/// The ordered commands for the package step of a plan. Pure so every manager is tested on Linux.
pub(crate) fn package_commands(
    plan: &ProvisionPlan,
    elevation: Elevation,
    is_root: bool,
    brew_program: &str,
    windows_script: &Path,
    windows_log: &Path,
) -> Vec<CommandSpec> {
    match plan.manager {
        Manager::Apt => {
            let Some(script) = privileged_script(plan) else {
                return Vec::new();
            };
            let (program, mut args) = if is_root {
                ("sh".to_string(), Vec::new())
            } else {
                match elevation {
                    Elevation::Pkexec => ("pkexec".to_string(), vec!["sh".to_string()]),
                    Elevation::Sudo => ("sudo".to_string(), vec!["sh".to_string()]),
                    Elevation::SudoNonInteractive => {
                        ("sudo".to_string(), vec!["-n".to_string(), "sh".to_string()])
                    }
                }
            };
            args.extend(["-c".to_string(), script]);
            vec![CommandSpec {
                description:
                    "Administrator step · refresh catalog, install and update packages (one prompt)"
                        .into(),
                program,
                args,
                env: Vec::new(),
                log_file: None,
            }]
        }
        Manager::Brew => brew_commands(plan)
            .into_iter()
            .map(|args| CommandSpec {
                description: format!("Homebrew · brew {}", args.join(" ")),
                program: brew_program.to_string(),
                args,
                env: vec![
                    ("HOMEBREW_NO_AUTO_UPDATE".into(), "1".into()),
                    ("HOMEBREW_NO_INSTALL_CLEANUP".into(), "1".into()),
                    ("HOMEBREW_NO_ENV_HINTS".into(), "1".into()),
                    ("NONINTERACTIVE".into(), "1".into()),
                ],
                log_file: None,
            })
            .collect(),
        Manager::Winget => {
            if winget_script(plan, &windows_log.to_string_lossy()).is_none() {
                return Vec::new();
            }
            vec![CommandSpec {
                description: "Administrator step · install and update with winget (one prompt)"
                    .into(),
                program: "powershell".into(),
                args: windows_elevation_arguments(&windows_script.to_string_lossy()),
                env: Vec::new(),
                log_file: Some(windows_log.to_path_buf()),
            }]
        }
        Manager::None => Vec::new(),
    }
}

/// What a finished plan produced.
#[derive(Debug, Default)]
pub(crate) struct ProvisionOutcome {
    pub(crate) packages_applied: bool,
    pub(crate) helper_installed_at: Option<String>,
}

pub(crate) type LineSink = Arc<dyn Fn(String) + Send + Sync>;

fn brew_program() -> String {
    environment::find_tool_path("brew").unwrap_or_else(|| "brew".into())
}

pub(crate) fn run_plan(
    plan: &ProvisionPlan,
    elevation: Elevation,
    sink: LineSink,
) -> Result<ProvisionOutcome, String> {
    let mut outcome = ProvisionOutcome::default();
    let (script_path, log_path) = match plan.manager {
        Manager::Winget if plan.has_privileged_work() => {
            let directory = tool_setup_log_directory()?;
            fs::create_dir_all(&directory)
                .map_err(|error| format!("Could not prepare the setup folder: {error}"))?;
            let stem = format!("winget-setup-{}", std::process::id());
            (
                directory.join(format!("{stem}.ps1")),
                directory.join(format!("{stem}.log")),
            )
        }
        _ => (PathBuf::new(), PathBuf::new()),
    };
    if plan.manager == Manager::Winget {
        if let Some(script) = winget_script(plan, &log_path.to_string_lossy()) {
            // Written as UTF-8 with a BOM so Windows PowerShell 5 reads it correctly.
            let mut bytes = vec![0xEF, 0xBB, 0xBF];
            bytes.extend(script.as_bytes());
            fs::write(&script_path, bytes)
                .map_err(|error| format!("Could not write the setup script: {error}"))?;
            let _ = fs::remove_file(&log_path);
        }
    }
    let is_root = unsafe { libc_geteuid() } == 0;
    let commands = package_commands(
        plan,
        elevation,
        is_root,
        &brew_program(),
        &script_path,
        &log_path,
    );
    for command in &commands {
        sink(command.description.clone());
        run_command(command, sink.clone())?;
        outcome.packages_applied = true;
    }
    if plan.manager == Manager::Winget {
        let _ = fs::remove_file(&script_path);
    }
    if plan.helper.needs_work() {
        sink("HDR10+ helper · download, verify digest and install (no password)".into());
        let path = dependencies::install_latest_hdr10plus_tool()?;
        sink(format!("HDR10+ helper installed at {path}."));
        outcome.helper_installed_at = Some(path);
    }
    Ok(outcome)
}

fn run_command(spec: &CommandSpec, sink: LineSink) -> Result<(), String> {
    let mut command = Command::new(&spec.program);
    command
        .args(&spec.args)
        .envs(spec.env.iter().cloned())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = crate::process_output::spawn_owned(&mut command)
        .map_err(|error| format!("Could not start {}: {error}", spec.program))?;
    let process_id = child.id();
    let stderr_thread = child.stderr.take().map(|stream| {
        let sink = sink.clone();
        thread::spawn(move || {
            crate::process_output::read_process_lines(stream, process_id, |line| sink(line))
        })
    });
    let tail = spec.log_file.clone().map(|path| {
        let sink = sink.clone();
        let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = done.clone();
        let handle = thread::spawn(move || tail_log(&path, &flag, &*sink));
        (done, handle)
    });
    let mut read_result = Ok(());
    if let Some(stdout) = child.stdout.take() {
        read_result =
            crate::process_output::read_process_lines(stdout, process_id, |line| sink(line));
    }
    let diagnostic = match stderr_thread {
        Some(handle) => handle
            .join()
            .unwrap_or_else(|_| Err("Setup diagnostic reader panicked.".into())),
        None => Ok(()),
    };
    let status = child
        .wait()
        .map_err(|error| format!("Could not wait for {}: {error}", spec.program));
    if let Some((done, handle)) = tail {
        done.store(true, std::sync::atomic::Ordering::SeqCst);
        let _ = handle.join();
    }
    let status = status?;
    read_result.and(diagnostic)?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{} exited with {status}.", spec.description))
    }
}

/// Follow a log file that an elevated process appends to, emitting each complete line.
fn tail_log(
    path: &Path,
    done: &std::sync::atomic::AtomicBool,
    sink: &(dyn Fn(String) + Send + Sync),
) {
    let mut offset = 0_u64;
    let mut pending = String::new();
    loop {
        let finished = done.load(std::sync::atomic::Ordering::SeqCst);
        if let Ok(mut file) = fs::File::open(path) {
            use std::io::{Seek, SeekFrom};
            if file.seek(SeekFrom::Start(offset)).is_ok() {
                let mut bytes = Vec::new();
                if file.read_to_end(&mut bytes).is_ok() && !bytes.is_empty() {
                    offset += bytes.len() as u64;
                    pending.push_str(&String::from_utf8_lossy(&bytes).replace('\u{feff}', ""));
                    while let Some(index) = pending.find('\n') {
                        let line = pending[..index].trim_end_matches('\r').to_string();
                        pending.drain(..=index);
                        if !line.trim().is_empty() {
                            sink(line);
                        }
                    }
                }
            }
        }
        if finished {
            if !pending.trim().is_empty() {
                sink(std::mem::take(&mut pending));
            }
            return;
        }
        thread::sleep(std::time::Duration::from_millis(400));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provision::{plan_provision, HelperAction};

    fn names(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    fn apt() -> Value {
        json!({"missing_packages": ["mediainfo"], "os": {"package_manager": "apt", "package_manager_available": true}})
    }

    #[test]
    fn apt_is_one_elevated_sh_batch_whichever_way_the_host_elevates() {
        let plan = plan_provision(
            &apt(),
            &names(&["ffmpeg"]),
            false,
            Some("1.7.1"),
            Some("1.7.1"),
            false,
        );
        let gui = package_commands(
            &plan,
            Elevation::Pkexec,
            false,
            "brew",
            Path::new("s.ps1"),
            Path::new("s.log"),
        );
        assert_eq!(gui.len(), 1, "exactly one prompt");
        assert_eq!(
            (
                gui[0].program.as_str(),
                gui[0].args[0].as_str(),
                gui[0].args[1].as_str()
            ),
            ("pkexec", "sh", "-c")
        );
        let cli = package_commands(
            &plan,
            Elevation::Sudo,
            false,
            "brew",
            Path::new("s.ps1"),
            Path::new("s.log"),
        );
        assert_eq!(cli[0].program, "sudo");
        let root = package_commands(
            &plan,
            Elevation::Pkexec,
            true,
            "brew",
            Path::new("s.ps1"),
            Path::new("s.log"),
        );
        assert_eq!(
            (root[0].program.as_str(), root[0].args[0].as_str()),
            ("sh", "-c"),
            "root needs no wrapper"
        );
    }

    #[test]
    fn headless_elevation_is_sudo_n_so_it_can_never_wait_for_a_password() {
        let plan = plan_provision(
            &apt(),
            &names(&["ffmpeg"]),
            false,
            Some("1.7.1"),
            Some("1.7.1"),
            false,
        );
        let headless = package_commands(
            &plan,
            Elevation::SudoNonInteractive,
            false,
            "brew",
            Path::new("s.ps1"),
            Path::new("s.log"),
        );
        assert_eq!(headless.len(), 1);
        assert_eq!(headless[0].program, "sudo");
        assert_eq!(
            headless[0].args[..3],
            ["-n".to_string(), "sh".to_string(), "-c".to_string()]
        );
        let root = package_commands(
            &plan,
            Elevation::SudoNonInteractive,
            true,
            "brew",
            Path::new("s.ps1"),
            Path::new("s.log"),
        );
        assert_eq!(
            (root[0].program.as_str(), root[0].args[0].as_str()),
            ("sh", "-c")
        );
    }

    #[test]
    fn homebrew_commands_run_unelevated_and_non_interactive_with_the_found_brew() {
        let environment = json!({"missing_packages": ["mediainfo"], "os": {"package_manager": "brew", "package_manager_available": true},
            "checks": [{"key":"brew","available":true},{"key":"xcode_clt","available":true}]});
        let plan = plan_provision(
            &environment,
            &names(&["ffmpeg"]),
            false,
            Some("1.7.1"),
            Some("1.7.1"),
            false,
        );
        let commands = package_commands(
            &plan,
            Elevation::Pkexec,
            false,
            "/opt/homebrew/bin/brew",
            Path::new("s"),
            Path::new("l"),
        );
        assert_eq!(commands.len(), 3);
        assert!(commands
            .iter()
            .all(|command| command.program == "/opt/homebrew/bin/brew"));
        assert!(commands.iter().all(|command| command
            .env
            .iter()
            .any(|(k, v)| k == "NONINTERACTIVE" && v == "1")));
        assert!(commands.iter().all(|command| command.log_file.is_none()));
    }

    #[test]
    fn winget_is_one_elevated_powershell_that_logs_to_a_file() {
        let environment = json!({"missing_packages": ["ffmpeg"], "os": {"package_manager": "winget", "package_manager_available": true},
            "checks": [{"key":"winget","available":true}]});
        let plan = plan_provision(
            &environment,
            &[],
            false,
            Some("1.7.1"),
            Some("1.7.1"),
            false,
        );
        let commands = package_commands(
            &plan,
            Elevation::Pkexec,
            false,
            "brew",
            Path::new("C:/t/s.ps1"),
            Path::new("C:/t/s.log"),
        );
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].program, "powershell");
        assert_eq!(
            commands[0].log_file.as_deref(),
            Some(Path::new("C:/t/s.log"))
        );
        assert!(commands[0].args.join(" ").contains("-Verb RunAs"));
    }

    #[test]
    fn helper_only_or_blocked_plans_have_no_package_commands() {
        let clean = plan_provision(
            &json!({"os": {"package_manager": "apt", "package_manager_available": true}}),
            &[],
            false,
            None,
            Some("1.7.1"),
            true,
        );
        assert!(package_commands(
            &clean,
            Elevation::Pkexec,
            true,
            "brew",
            Path::new("s"),
            Path::new("l")
        )
        .is_empty());
        assert_eq!(clean.helper, HelperAction::Install);
        let blocked = plan_provision(
            &json!({"missing_packages": ["ffmpeg"], "os": {"package_manager": "brew", "package_manager_available": false}, "checks": []}),
            &[],
            false,
            Some("1.7.1"),
            Some("1.7.1"),
            false,
        );
        assert!(package_commands(
            &blocked,
            Elevation::Pkexec,
            false,
            "brew",
            Path::new("s"),
            Path::new("l")
        )
        .is_empty());
    }

    #[test]
    fn log_tailing_emits_complete_lines_and_the_unfinished_last_line() {
        let directory = std::env::temp_dir().join(format!("hevc-tail-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let log = directory.join("x.log");
        fs::write(&log, "\u{feff}first\r\nsecond\nthird-no-newline").unwrap();
        let lines = Arc::new(std::sync::Mutex::new(Vec::new()));
        let collected = lines.clone();
        let done = std::sync::atomic::AtomicBool::new(true);
        tail_log(&log, &done, &move |line| {
            collected.lock().unwrap().push(line)
        });
        assert_eq!(
            *lines.lock().unwrap(),
            vec!["first", "second", "third-no-newline"]
        );
        let _ = fs::remove_dir_all(directory);
    }
}
