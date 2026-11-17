//! The `json apt-refresh` and `json apt-upgrade` actions and their confirmation-gated apt runner.

use super::*;

fn parse_apt_upgrade_cli_args(args: &[String]) -> Result<Vec<String>, String> {
    let mut confirmed = false;
    let mut packages = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--confirm" if !confirmed => confirmed = true,
            "--confirm" => return Err("APT package upgrades accept --confirm only once.".into()),
            "--package" => {
                let Some(package) = args.get(index + 1) else {
                    return Err("APT package upgrades require a name after --package.".into());
                };
                packages.push(package.clone());
                index += 1;
            }
            option if option.starts_with("--package=") => {
                packages.push(option["--package=".len()..].to_string());
            }
            option => return Err(format!("Unknown apt-upgrade option {option}.")),
        }
        index += 1;
    }
    if !confirmed {
        return Err("APT package upgrades require --confirm.".into());
    }
    if packages.is_empty() {
        return Err("APT package upgrades require at least one --package NAME entry.".into());
    }
    Ok(packages)
}

fn run_apt_cli_command(arguments: &[String]) -> Result<(), String> {
    if arguments.is_empty() {
        return Err("APT command arguments are missing.".into());
    }
    run_elevated_cli_command("apt-get", arguments)
}

fn run_elevated_cli_command(program: &str, arguments: &[String]) -> Result<(), String> {
    if !cfg!(target_os = "linux") {
        return Err("APT package operations are available only on Linux hosts.".into());
    }
    let mut command = if unsafe { libc_geteuid() } == 0 {
        let mut command = Command::new(program);
        command.args(arguments);
        command
    } else {
        if !executable_in_path("sudo") {
            return Err("CLI administrator authentication (sudo) is not installed.".into());
        }
        let mut command = Command::new("sudo");
        command.arg(program).args(arguments);
        command
    };
    let mut child = command
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("Could not start the APT operation: {error}"))?;
    let process_id = child.id();
    let stdout_reader = child
        .stdout
        .take()
        .map(|reader| thread::spawn(move || forward_command_output(reader, process_id)));
    let stderr_reader = child
        .stderr
        .take()
        .map(|reader| thread::spawn(move || forward_command_output(reader, process_id)));
    let status = child.wait();
    if let Some(reader) = stdout_reader {
        reader
            .join()
            .map_err(|_| "Could not forward APT standard output.".to_string())??;
    }
    if let Some(reader) = stderr_reader {
        reader
            .join()
            .map_err(|_| "Could not forward APT diagnostics.".to_string())??;
    }
    let status =
        status.map_err(|error| format!("Could not wait for the APT operation: {error}"))?;
    if !status.success() {
        return Err(format!("APT operation exited with {status}."));
    }
    Ok(())
}

fn forward_command_output(mut reader: impl Read, process_id: u32) -> Result<(), String> {
    #[cfg(not(unix))]
    let _ = process_id;
    // Preserve raw CLI bytes and interactive sudo's foreground-terminal group.
    // Unlike noninteractive GUI/MCP jobs, this command must not create a group.
    let result = (|| {
        let mut bytes = [0_u8; 8192];
        loop {
            let count = reader
                .read(&mut bytes)
                .map_err(|error| format!("Could not read APT output: {error}"))?;
            if count == 0 {
                return Ok(());
            }
            let stderr = std::io::stderr();
            let mut stderr = stderr.lock();
            stderr
                .write_all(&bytes[..count])
                .and_then(|_| stderr.flush())
                .map_err(|error| format!("Could not forward APT output: {error}"))?;
        }
    })();
    if result.is_err() {
        #[cfg(unix)]
        unsafe {
            if let Ok(pid) = libc::pid_t::try_from(process_id) {
                libc::kill(pid, libc::SIGKILL);
            }
        }
    }
    result
}

pub(crate) fn run_json_cli_apt_refresh(args: &[String]) -> i32 {
    if args != ["--confirm"] {
        eprintln!("APT catalog refresh requires only the explicit --confirm option.");
        return 2;
    }
    if let Err(error) = dependencies::apt_update_status() {
        eprintln!("{error}");
        return 1;
    }
    if let Err(error) = run_apt_cli_command(&["update".into()]) {
        eprintln!("{error}");
        return 1;
    }
    println!("{{\"refreshed\":true,\"note\":\"Package indexes refreshed; no packages were installed or upgraded.\"}}");
    0
}

pub(crate) fn run_json_cli_apt_upgrade(args: &[String]) -> i32 {
    let confirmed = match parse_apt_upgrade_cli_args(args) {
        Ok(confirmed) => confirmed,
        Err(error) => {
            eprintln!("{error}");
            return 2;
        }
    };
    let status = match dependencies::apt_update_status() {
        Ok(status) => status,
        Err(error) => {
            eprintln!("{error}");
            return 1;
        }
    };
    let packages = match validate_apt_update_confirmation(&status, confirmed) {
        Ok(packages) => packages,
        Err(error) => {
            eprintln!("{error}");
            return 1;
        }
    };
    if let Err(error) = validate_apt_upgrade_plan(&packages) {
        eprintln!("{error}");
        return 1;
    }
    let mut arguments = vec![
        "install".into(),
        "--only-upgrade".into(),
        "--no-install-recommends".into(),
        "--no-remove".into(),
        "--yes".into(),
        "--".into(),
    ];
    arguments.extend(packages.iter().cloned());
    if let Err(error) = run_apt_cli_command(&arguments) {
        eprintln!("{error}");
        return 1;
    }
    match serde_json::to_string(&json!({
        "upgraded": packages,
        "note": "The confirmed allowlisted package updates completed.",
    })) {
        Ok(payload) => {
            println!("{payload}");
            0
        }
        Err(error) => {
            eprintln!("Could not serialize the APT upgrade result: {error}");
            2
        }
    }
}

/// `json provision-plan` prints the one-pass plan; `json provision --confirm` runs it. The
/// administrator password is asked once, by sudo on the terminal; this process never sees it.
pub(crate) fn run_json_cli_provision(args: &[String], apply: bool) -> i32 {
    if apply && args != ["--confirm"] {
        eprintln!("One-pass setup requires only the explicit --confirm option. Run provision-plan first to review it.");
        return 2;
    }
    if !apply && !args.is_empty() {
        eprintln!("provision-plan does not accept additional options.");
        return 2;
    }
    let (plan, release_error) = match provision::current_plan(ToolPaths::default()) {
        Ok(result) => result,
        Err(error) => {
            eprintln!("{error}");
            return 1;
        }
    };
    let mut payload = provision::plan_json(&plan, release_error.as_deref());
    if !apply {
        println!("{payload}");
        return 0;
    }
    if plan.nothing_to_do() {
        payload["applied"] = json!(false);
        println!("{payload}");
        return 0;
    }
    if let Some(script) = provision::privileged_script(&plan) {
        if let Err(error) = run_elevated_cli_command("sh", &["-c".into(), script]) {
            eprintln!("{error}");
            return 1;
        }
    }
    if plan.helper.needs_work() {
        match dependencies::install_latest_hdr10plus_tool() {
            Ok(path) => payload["helper_installed_at"] = json!(path),
            Err(error) => {
                eprintln!("{error}");
                return 1;
            }
        }
    }
    payload["applied"] = json!(true);
    println!("{payload}");
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apt_upgrade_cli_parser_requires_confirmation_and_explicit_packages() {
        assert!(parse_apt_upgrade_cli_args(&["--package".into(), "ffmpeg".into()]).is_err());
        assert!(parse_apt_upgrade_cli_args(&["--confirm".into()]).is_err());
        assert!(parse_apt_upgrade_cli_args(&["--confirm".into(), "--confirm".into()]).is_err());
        assert!(parse_apt_upgrade_cli_args(&["--confirm".into(), "--unexpected".into()]).is_err());
    }

    #[test]
    fn apt_upgrade_cli_parser_accepts_repeated_exact_package_arguments() {
        let packages = parse_apt_upgrade_cli_args(&[
            "--confirm".into(),
            "--package".into(),
            "ffmpeg".into(),
            "--package=mediainfo".into(),
        ])
        .expect("parse explicit repeated package arguments");
        assert_eq!(packages, vec!["ffmpeg", "mediainfo"]);
    }
}
