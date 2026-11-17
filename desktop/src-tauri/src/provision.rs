//! One-pass toolchain provisioning: a pure plan for "install and update everything" that needs
//! at most one administrator prompt. The plan is shared by the GUI, CLI and MCP so a person or a
//! model sees and approves exactly the same steps, and none of them ever handles the password.

use super::*;

/// What to do about the HDR10+ helper, which is installed in user space and never needs a password.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HelperAction {
    Install,
    Update {
        installed: String,
        latest: String,
    },
    Current {
        installed: String,
    },
    /// Installed, but the latest release could not be checked (offline, rate limited).
    Unchecked {
        installed: String,
    },
}

impl HelperAction {
    pub(crate) fn key(&self) -> &'static str {
        match self {
            Self::Install => "install",
            Self::Update { .. } => "update",
            Self::Current { .. } => "current",
            Self::Unchecked { .. } => "unchecked",
        }
    }

    pub(crate) fn needs_work(&self) -> bool {
        matches!(self, Self::Install | Self::Update { .. })
    }
}

/// Which package manager the host uses to install the media tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Manager {
    Apt,
    Brew,
    Winget,
    None,
}

impl Manager {
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Apt => "apt",
            Self::Brew => "brew",
            Self::Winget => "winget",
            Self::None => "none",
        }
    }

    fn from_environment(environment: &Value) -> Self {
        match environment
            .pointer("/os/package_manager")
            .and_then(Value::as_str)
        {
            Some("apt") => Self::Apt,
            Some("brew") => Self::Brew,
            Some("winget") => Self::Winget,
            _ => Self::None,
        }
    }

    /// Logical package names the app may install or upgrade with this manager.
    fn allowed(self, name: &str) -> bool {
        match self {
            Self::Apt => APT_ALLOWLIST.contains(&name),
            Self::Brew | Self::Winget => ["ffmpeg", "mediainfo", "mkvtoolnix"].contains(&name),
            Self::None => false,
        }
    }
}

/// Something that must exist before the package manager can work, with the exact remedy to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Prerequisite {
    pub(crate) key: String,
    pub(crate) label: String,
    pub(crate) remedy: String,
    pub(crate) command: Option<String>,
}

/// Everything the privileged (password) step and the user-space step will do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProvisionPlan {
    pub(crate) manager: Manager,
    pub(crate) blockers: Vec<Prerequisite>,
    pub(crate) advisories: Vec<String>,
    pub(crate) refresh_catalog: bool,
    pub(crate) install_packages: Vec<String>,
    pub(crate) upgrade_packages: Vec<String>,
    pub(crate) helper: HelperAction,
    pub(crate) needs_password: bool,
}

impl ProvisionPlan {
    pub(crate) fn has_privileged_work(&self) -> bool {
        self.refresh_catalog
            || !self.install_packages.is_empty()
            || !self.upgrade_packages.is_empty()
    }

    /// The part of this plan that needs no password: the HDR10+ helper everywhere, and Homebrew
    /// (which runs as the user). Used by "keep tools up to date automatically", which must never
    /// raise an authentication prompt on its own.
    pub(crate) fn user_space_only(&self) -> ProvisionPlan {
        let mut plan = self.clone();
        if plan.manager != Manager::Brew {
            plan.refresh_catalog = false;
            plan.install_packages.clear();
            plan.upgrade_packages.clear();
            plan.needs_password = false;
        }
        plan.blockers.clear();
        plan
    }

    pub(crate) fn nothing_to_do(&self) -> bool {
        !self.has_privileged_work() && !self.helper.needs_work() && self.blockers.is_empty()
    }
}

/// Compare dotted numeric versions ("1.7.0" < "1.7.1"). Unparseable input is never "older".
pub(crate) fn version_is_older(installed: &str, latest: &str) -> bool {
    fn parts(value: &str) -> Option<Vec<u64>> {
        value
            .trim()
            .trim_start_matches('v')
            .split('.')
            .map(|part| part.parse::<u64>().ok())
            .collect()
    }
    match (parts(installed), parts(latest)) {
        (Some(mut a), Some(mut b)) => {
            let width = a.len().max(b.len());
            a.resize(width, 0);
            b.resize(width, 0);
            a < b
        }
        _ => false,
    }
}

pub(crate) fn helper_action(installed: Option<&str>, latest: Option<&str>) -> HelperAction {
    match (installed, latest) {
        (None, _) => HelperAction::Install,
        (Some(installed), Some(latest)) if version_is_older(installed, latest) => {
            HelperAction::Update {
                installed: installed.to_string(),
                latest: latest.trim_start_matches('v').to_string(),
            }
        }
        (Some(installed), Some(_)) => HelperAction::Current {
            installed: installed.to_string(),
        },
        (Some(installed), None) => HelperAction::Unchecked {
            installed: installed.to_string(),
        },
    }
}

/// Build the plan from the environment report, the allowlisted packages the local index already
/// shows as upgradable, whether that index is stale, and the helper's installed/latest versions.
pub(crate) fn plan_provision(
    environment: &Value,
    upgradable: &[String],
    catalog_stale: bool,
    helper_installed: Option<&str>,
    helper_latest: Option<&str>,
    is_root: bool,
) -> ProvisionPlan {
    let manager = Manager::from_environment(environment);
    let allowed = |name: &String| manager.allowed(name);
    let mut install_packages = environment
        .get("missing_packages")
        .or_else(|| environment.get("missing_apt_packages"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .filter(allowed)
        .collect::<Vec<_>>();
    install_packages.sort();
    install_packages.dedup();
    let mut upgrade_packages = upgradable
        .iter()
        .filter(|name| allowed(name) && !install_packages.contains(name))
        .cloned()
        .collect::<Vec<_>>();
    upgrade_packages.sort();
    upgrade_packages.dedup();
    let manager_available = environment
        .pointer("/os/package_manager_available")
        .and_then(Value::as_bool)
        == Some(true);
    let blockers = prerequisites(manager, environment);
    // Without the manager (or something it needs) no package work can run; the user-space helper still can.
    let can_run_packages = manager != Manager::None && manager_available && blockers.is_empty();
    if !can_run_packages {
        install_packages.clear();
        upgrade_packages.clear();
    }
    let has_packages = !install_packages.is_empty() || !upgrade_packages.is_empty();
    // apt: refresh inside the same elevated step only when stale or packages are missing, so a fresh
    // catalog with nothing pending costs no prompt. brew: `brew update` before any change.
    let refresh_catalog = match manager {
        Manager::Apt => can_run_packages && (catalog_stale || !install_packages.is_empty()),
        Manager::Brew => has_packages,
        _ => false,
    };
    let needs_password = match manager {
        Manager::Apt => refresh_catalog && !is_root,
        Manager::Winget => has_packages,
        _ => false,
    };
    ProvisionPlan {
        manager,
        blockers,
        advisories: ffmpeg_advisories(environment),
        needs_password,
        refresh_catalog,
        install_packages,
        upgrade_packages,
        helper: helper_action(helper_installed, helper_latest),
    }
}

fn check_available(environment: &Value, key: &str) -> bool {
    environment
        .get("checks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .any(|check| {
            check.get("key").and_then(Value::as_str) == Some(key)
                && check.get("available").and_then(Value::as_bool) == Some(true)
        })
}

/// What must be installed before the package manager works. On macOS Homebrew needs the Xcode
/// Command Line Tools first, so they are listed first; the app never runs the Homebrew installer
/// script itself because it needs an interactive administrator session.
fn prerequisites(manager: Manager, environment: &Value) -> Vec<Prerequisite> {
    let mut blockers = Vec::new();
    match manager {
        Manager::Brew => {
            if !check_available(environment, "xcode_clt") {
                blockers.push(Prerequisite {
                    key: "xcode_clt".into(),
                    label: "Xcode Command Line Tools".into(),
                    remedy: "Install Apple's command line developer tools (Homebrew needs them). A system window will guide you; then check again.".into(),
                    command: Some("xcode-select --install".into()),
                });
            }
            if !check_available(environment, "brew") {
                blockers.push(Prerequisite {
                    key: "brew".into(),
                    label: "Homebrew".into(),
                    remedy: "Install Homebrew, the macOS package manager, by pasting this into Terminal, then check again.".into(),
                    command: Some("/bin/bash -c \"$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)\"".into()),
                });
            }
        }
        Manager::Winget if !check_available(environment, "winget") => {
            blockers.push(Prerequisite {
                key: "winget".into(),
                label: "Windows Package Manager (winget)".into(),
                remedy: "Install or update \"App Installer\" from the Microsoft Store, which provides winget, then check again.".into(),
                command: None,
            });
        }
        _ => {}
    }
    blockers
}

/// A stripped-down FFmpeg build can lack encoders the app uses; say so in plain words.
fn ffmpeg_advisories(environment: &Value) -> Vec<String> {
    let path = environment
        .get("checks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|check| check.get("key").and_then(Value::as_str) == Some("ffmpeg"))
        .and_then(|check| check.get("path").and_then(Value::as_str))
        .unwrap_or_default()
        .to_lowercase();
    if path.contains("essentials") {
        return vec!["Your FFmpeg is the \"Essentials\" build, which leaves out some libraries. Install the full build (winget package Gyan.FFmpeg) for the widest support; Install & update everything uses the full build.".into()];
    }
    Vec::new()
}

/// winget package ids. FFmpeg is the full build (not `Gyan.FFmpeg.Essentials`). MediaInfo's CLI is `MediaArea.MediaInfo`.
pub(crate) fn winget_id(name: &str) -> Option<&'static str> {
    match name {
        "ffmpeg" => Some("Gyan.FFmpeg"),
        "mediainfo" => Some("MediaArea.MediaInfo"),
        "mkvtoolnix" => Some("MoritzBunkus.MKVToolNix"),
        _ => None,
    }
}

/// Homebrew commands (arguments after `brew`), run as the user: no password is ever needed.
pub(crate) fn brew_commands(plan: &ProvisionPlan) -> Vec<Vec<String>> {
    let mut commands = Vec::new();
    if plan.manager != Manager::Brew || !plan.has_privileged_work() {
        return commands;
    }
    if plan.refresh_catalog {
        commands.push(vec!["update".to_string()]);
    }
    if !plan.install_packages.is_empty() {
        let mut command = vec!["install".to_string()];
        command.extend(plan.install_packages.iter().cloned());
        commands.push(command);
    }
    if !plan.upgrade_packages.is_empty() {
        let mut command = vec!["upgrade".to_string()];
        command.extend(plan.upgrade_packages.iter().cloned());
        commands.push(command);
    }
    commands
}

/// One PowerShell script that runs every winget change, writes a log, and exits non-zero on a real
/// failure. "No applicable update" and "already installed" are success.
pub(crate) fn winget_script(plan: &ProvisionPlan, log_path: &str) -> Option<String> {
    if plan.manager != Manager::Winget || !plan.has_privileged_work() {
        return None;
    }
    let mut lines = vec![
        "$ErrorActionPreference = 'Continue'".to_string(),
        "$failed = 0".to_string(),
        format!("$log = {}", powershell_quote(log_path)),
    ];
    let mut add = |verb: &str, name: &str| {
        let Some(id) = winget_id(name) else { return };
        let arguments = [
            verb,
            "--exact",
            "--id",
            id,
            "--silent",
            "--accept-source-agreements",
            "--accept-package-agreements",
        ]
        .iter()
        .map(|argument| powershell_quote(argument))
        .collect::<Vec<_>>();
        lines.push(format!("& {} {} *>&1 | ForEach-Object {{ Add-Content -LiteralPath $log -Value ([string]$_) -Encoding UTF8 }}", powershell_quote("winget"), arguments.join(" ")));
        // 0x8A15002B: no applicable update. 0x8A150061: already installed.
        lines.push("if ($LASTEXITCODE -ne 0 -and $LASTEXITCODE -ne -1978335189 -and $LASTEXITCODE -ne -1978335135) { $failed = 1 }".to_string());
    };
    for name in &plan.install_packages {
        add("install", name);
    }
    for name in &plan.upgrade_packages {
        add("upgrade", name);
    }
    lines.push("exit $failed".to_string());
    Some(lines.join("\n"))
}

/// Arguments for `powershell` that start the script elevated (one UAC prompt) and wait for it.
pub(crate) fn windows_elevation_arguments(script_path: &str) -> Vec<String> {
    let command = format!(
        "$p = Start-Process -FilePath powershell -Verb RunAs -Wait -PassThru -ArgumentList '-NoProfile','-ExecutionPolicy','Bypass','-File',{}; exit $p.ExitCode",
        powershell_quote(script_path)
    );
    vec!["-NoProfile".into(), "-Command".into(), command]
}

/// Names of our Homebrew formulae that `brew outdated --json=v2` reports as outdated.
pub(crate) fn parse_brew_outdated(json_text: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<Value>(json_text) else {
        return Vec::new();
    };
    let mut names = value
        .get("formulae")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| item.get("name").and_then(Value::as_str))
        .filter(|name| Manager::Brew.allowed(name))
        .map(str::to_string)
        .collect::<Vec<_>>();
    names.sort();
    names.dedup();
    names
}

/// Our logical package names whose winget id appears in `winget upgrade` output (ids are not translated).
pub(crate) fn parse_winget_upgradable(text: &str) -> Vec<String> {
    let mut names = ["ffmpeg", "mediainfo", "mkvtoolnix"]
        .iter()
        .filter(|name| {
            winget_id(name).is_some_and(|id| {
                text.lines()
                    .any(|line| line.split_whitespace().any(|word| word == id))
            })
        })
        .map(|name| name.to_string())
        .collect::<Vec<_>>();
    names.sort();
    names
}

/// PowerShell single-quoted string: a quote is escaped by doubling it.
pub(crate) fn powershell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

pub(crate) fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// The fixed script run as root. Every argument is quoted; package names come from the allowlist.
pub(crate) fn privileged_script(plan: &ProvisionPlan) -> Option<String> {
    if plan.manager != Manager::Apt || !plan.has_privileged_work() {
        return None;
    }
    let command = |arguments: &[&str], packages: &[String]| {
        let mut parts = vec!["DEBIAN_FRONTEND=noninteractive".to_string()];
        parts.push(shell_quote("apt-get"));
        parts.extend(arguments.iter().map(|argument| shell_quote(argument)));
        if !packages.is_empty() {
            parts.push(shell_quote("--"));
            parts.extend(packages.iter().map(|package| shell_quote(package)));
        }
        parts.join(" ")
    };
    let mut lines = vec!["set -eu".to_string()];
    if plan.refresh_catalog {
        lines.push(command(&["update"], &[]));
    }
    if !plan.install_packages.is_empty() {
        lines.push(command(
            &[
                "install",
                "--yes",
                "--no-upgrade",
                "--no-install-recommends",
                "--no-remove",
            ],
            &plan.install_packages,
        ));
    }
    if !plan.upgrade_packages.is_empty() {
        lines.push(command(
            &[
                "install",
                "--only-upgrade",
                "--yes",
                "--no-install-recommends",
                "--no-remove",
            ],
            &plan.upgrade_packages,
        ));
    }
    Some(lines.join("\n"))
}

fn run_text(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    Some(format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    ))
}

/// Which allowlisted packages the host's manager reports as upgradable, and whether its package
/// index is stale (apt only; Homebrew and winget refresh as part of the step).
fn upgradable_packages(environment: &Value) -> (Vec<String>, bool) {
    match Manager::from_environment(environment) {
        Manager::Apt if cfg!(target_os = "linux") => match dependencies::apt_update_status() {
            Ok(status) => (
                status["packages"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|package| package["name"].as_str().map(str::to_string))
                    .collect(),
                status["catalog_stale"].as_bool().unwrap_or(true),
            ),
            Err(_) => (Vec::new(), true),
        },
        Manager::Brew => {
            let brew = environment::find_tool_path("brew").unwrap_or_else(|| "brew".into());
            let text = run_text(
                &brew,
                &["outdated", "--json=v2", "ffmpeg", "mediainfo", "mkvtoolnix"],
            );
            (
                text.map(|text| parse_brew_outdated(&text))
                    .unwrap_or_default(),
                false,
            )
        }
        Manager::Winget => {
            let text = run_text("winget", &["upgrade", "--accept-source-agreements"]);
            (
                text.map(|text| parse_winget_upgradable(&text))
                    .unwrap_or_default(),
                false,
            )
        }
        _ => (Vec::new(), false),
    }
}

/// How the privileged step runs where no desktop can prompt (the headless server).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HeadlessRoute {
    /// The server process already is root.
    Root,
    /// `sudo -n` works for this user: passwordless sudo.
    SudoNoPassword,
    /// Neither; the plan reports a blocker with the fix.
    Unavailable,
}

impl HeadlessRoute {
    pub(crate) fn key(self) -> &'static str {
        match self {
            HeadlessRoute::Root => "root",
            HeadlessRoute::SudoNoPassword => "sudo-n",
            HeadlessRoute::Unavailable => "none",
        }
    }
}

/// Pure choice of route; `sudo_works` is only asked when not root.
pub(crate) fn headless_route(is_root: bool, sudo_works: impl FnOnce() -> bool) -> HeadlessRoute {
    if is_root {
        HeadlessRoute::Root
    } else if sudo_works() {
        HeadlessRoute::SudoNoPassword
    } else {
        HeadlessRoute::Unavailable
    }
}

/// `whoami /groups` lists the token's mandatory integrity label; `S-1-16-12288` is High, an elevated
/// administrator token, which can run the elevated winget step without a UAC prompt.
#[cfg_attr(not(windows), allow(dead_code))] // read on Windows; tested everywhere
pub(crate) fn groups_show_elevation(whoami_groups: &str) -> bool {
    whoami_groups.contains("S-1-16-12288")
}

#[cfg(windows)]
fn windows_token_is_elevated() -> bool {
    Command::new("whoami")
        .arg("/groups")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .is_ok_and(|output| groups_show_elevation(&String::from_utf8_lossy(&output.stdout)))
}

#[cfg(not(windows))]
fn windows_token_is_elevated() -> bool {
    false
}

/// Whether this process already holds the rights the privileged step needs: root on Unix, an elevated
/// token on Windows.
fn process_is_elevated() -> bool {
    let is_root = unsafe { libc_geteuid() } == 0;
    is_root || windows_token_is_elevated()
}

/// Look at this process: root or an elevated Windows token, else whether `sudo -n true` succeeds (it never
/// prompts).
pub(crate) fn probe_headless_route() -> HeadlessRoute {
    headless_route(process_is_elevated(), || {
        Command::new("sudo")
            .args(["-n", "true"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    })
}

/// Fit a plan to a headless route. Where the plan needs a prompt (apt without root, winget UAC) and no
/// route exists, the privileged work is withheld and one blocker names it and the fix, so the plan says
/// what to do instead of failing mid-run. A route that exists means no prompt at all.
pub(crate) fn apply_headless_route(plan: &mut ProvisionPlan, route: HeadlessRoute) {
    if !plan.needs_password {
        return;
    }
    if route != HeadlessRoute::Unavailable {
        plan.needs_password = false;
        return;
    }
    let mut work = Vec::new();
    if !plan.install_packages.is_empty() {
        work.push(format!("install {}", plan.install_packages.join(", ")));
    }
    if !plan.upgrade_packages.is_empty() {
        work.push(format!("update {}", plan.upgrade_packages.join(", ")));
    }
    if plan.refresh_catalog {
        work.push("refresh the package catalog".to_string());
    }
    plan.blockers.push(Prerequisite {
        key: "headless-privilege".into(),
        label: format!("Administrator rights are needed to {}", work.join("; ")),
        remedy: "This server has no desktop to ask for a password, and the API never accepts one. Start the server as root, or allow this user to run the setup through `sudo -n` (passwordless sudo), then plan again.".into(),
        command: Some("sudo visudo -f /etc/sudoers.d/hevc-rebuild".into()),
    });
    plan.refresh_catalog = false;
    plan.install_packages.clear();
    plan.upgrade_packages.clear();
    plan.needs_password = false;
}

/// Which front end is asking for a plan: it decides how the privileged step could be authorised.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Privilege {
    /// The desktop app, CLI or stdio MCP: pkexec/sudo prompt on a real session.
    Desktop,
    /// The headless server with a probed route.
    Headless(HeadlessRoute),
}

pub(crate) fn current_plan(
    tool_paths: ToolPaths,
) -> Result<(ProvisionPlan, Option<String>), String> {
    current_plan_for(tool_paths, Privilege::Desktop)
}

/// Gather the live state (environment, package manager, GitHub release) and build the plan. A failed
/// release lookup is reported, not fatal: package work can still proceed and an installed helper is left alone.
pub(crate) fn current_plan_for(
    tool_paths: ToolPaths,
    privilege: Privilege,
) -> Result<(ProvisionPlan, Option<String>), String> {
    let environment = environment::inspect_environment(tool_paths, None)?;
    if environment
        .pointer("/os/supported")
        .and_then(Value::as_bool)
        != Some(true)
    {
        return Err("One-pass setup is limited to supported Ubuntu, Linux Mint, Windows and macOS releases.".into());
    }
    let (upgradable, catalog_stale) = upgradable_packages(&environment);
    let (latest, latest_error) = match dependencies::latest_hdr10plus_version() {
        Ok(version) => (Some(version), None),
        Err(error) => (None, Some(error)),
    };
    let installed = dependencies::installed_hdr10plus_version(&environment);
    let is_root = unsafe { libc_geteuid() } == 0;
    let mut plan = plan_provision(
        &environment,
        &upgradable,
        catalog_stale,
        installed.as_deref(),
        latest.as_deref(),
        is_root,
    );
    match privilege {
        Privilege::Desktop => {
            if plan.manager == Manager::Apt
                && plan.needs_password
                && !is_root
                && !executable_in_path("pkexec")
            {
                return Err(
                    "Desktop administrator authentication (pkexec) is not installed.".into(),
                );
            }
        }
        Privilege::Headless(route) => apply_headless_route(&mut plan, route),
    }
    Ok((plan, latest_error))
}

pub(crate) fn plan_json(plan: &ProvisionPlan, latest_error: Option<&str>) -> Value {
    let mut steps = Vec::new();
    if plan.refresh_catalog {
        steps.push(match plan.manager {
            Manager::Brew => "Update Homebrew's package list".to_string(),
            _ => "Refresh the Ubuntu package catalog".to_string(),
        });
    }
    if !plan.install_packages.is_empty() {
        steps.push(format!(
            "Install missing packages: {}",
            plan.install_packages.join(", ")
        ));
    }
    if !plan.upgrade_packages.is_empty() {
        steps.push(format!(
            "Update installed packages: {}",
            plan.upgrade_packages.join(", ")
        ));
    }
    match &plan.helper {
        HelperAction::Install => {
            steps.push("Install the HDR10+ helper (hdr10plus_tool) without a password".to_string())
        }
        HelperAction::Update { installed, latest } => steps.push(format!(
            "Update the HDR10+ helper {installed} to {latest} without a password"
        )),
        _ => {}
    }
    json!({
        "manager": plan.manager.key(),
        "blockers": plan.blockers.iter().map(|blocker| json!({"key": blocker.key, "label": blocker.label, "remedy": blocker.remedy, "command": blocker.command})).collect::<Vec<_>>(),
        "advisories": plan.advisories,
        "refresh_catalog": plan.refresh_catalog,
        "install_packages": plan.install_packages,
        "upgrade_packages": plan.upgrade_packages,
        "helper": {
            "action": plan.helper.key(),
            "installed": match &plan.helper { HelperAction::Update { installed, .. } | HelperAction::Current { installed } | HelperAction::Unchecked { installed } => Some(installed.clone()), _ => None },
            "latest": match &plan.helper { HelperAction::Update { latest, .. } => Some(latest.clone()), _ => None },
        },
        "needs_password": plan.needs_password,
        "password_prompts": if plan.needs_password { 1 } else { 0 },
        "nothing_to_do": plan.nothing_to_do(),
        "steps": steps,
        "release_check_error": latest_error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    fn env(missing: &[&str]) -> Value {
        json!({"missing_apt_packages": missing, "os": {"package_manager": "apt", "package_manager_available": true, "supported": true}})
    }

    fn mac_env(brew: bool, clt: bool, missing: &[&str]) -> Value {
        json!({"missing_packages": missing, "os": {"package_manager": "brew", "package_manager_available": brew},
               "checks": [{"key":"brew","available":brew},{"key":"xcode_clt","available":clt}]})
    }

    fn windows_env(winget: bool, missing: &[&str], ffmpeg_path: &str) -> Value {
        json!({"missing_packages": missing, "os": {"package_manager": "winget", "package_manager_available": winget},
               "checks": [{"key":"winget","available":winget},{"key":"ffmpeg","available":true,"path":ffmpeg_path}]})
    }

    const ELEVATED_GROUPS: &str = "GROUP INFORMATION\r\n-----------------\r\n\r\nGroup Name                                    Type             SID          Attributes\r\n============================================= ================ ============ ==================================================\r\nEveryone                                      Well-known group S-1-1-0      Mandatory group, Enabled by default, Enabled group\r\nBUILTIN\\Administrators                        Alias            S-1-5-32-544 Mandatory group, Enabled by default, Enabled group, Group owner\r\nMandatory Label\\High Mandatory Level         Label            S-1-16-12288\r\n";

    #[test]
    fn a_windows_token_with_the_high_integrity_label_is_elevated() {
        assert!(groups_show_elevation(ELEVATED_GROUPS));
        let medium = ELEVATED_GROUPS
            .replace("S-1-16-12288", "S-1-16-8192")
            .replace("High Mandatory Level", "Medium Mandatory Level");
        assert!(
            !groups_show_elevation(&medium),
            "an administrator who has not elevated still gets a UAC prompt"
        );
        assert!(!groups_show_elevation(""));
    }

    #[test]
    fn headless_route_prefers_root_then_passwordless_sudo_and_never_asks_for_a_password() {
        assert_eq!(
            headless_route(true, || panic!("root never probes sudo")),
            HeadlessRoute::Root
        );
        assert_eq!(
            headless_route(false, || true),
            HeadlessRoute::SudoNoPassword
        );
        assert_eq!(headless_route(false, || false), HeadlessRoute::Unavailable);
        assert_eq!(
            [
                HeadlessRoute::Root.key(),
                HeadlessRoute::SudoNoPassword.key(),
                HeadlessRoute::Unavailable.key()
            ],
            ["root", "sudo-n", "none"]
        );
    }

    #[test]
    fn headless_plan_without_a_privilege_route_is_a_blocker_that_names_the_work_and_the_fix() {
        let mut plan = plan_provision(
            &env(&["mediainfo"]),
            &names(&["ffmpeg"]),
            false,
            Some("1.7.1"),
            Some("1.7.1"),
            false,
        );
        assert!(plan.has_privileged_work() && plan.needs_password);
        apply_headless_route(&mut plan, HeadlessRoute::Unavailable);
        assert!(
            !plan.has_privileged_work() && !plan.needs_password,
            "nothing privileged can run without a route"
        );
        let blocker = plan
            .blockers
            .iter()
            .find(|blocker| blocker.key == "headless-privilege")
            .expect("blocker");
        assert!(
            blocker.label.contains("mediainfo") && blocker.label.contains("ffmpeg"),
            "{}",
            blocker.label
        );
        assert!(
            blocker.remedy.contains("root") && blocker.remedy.contains("sudo -n"),
            "{}",
            blocker.remedy
        );
        assert!(
            blocker.remedy.contains("password"),
            "the remedy must say no password is requested over the API"
        );
        assert!(
            !plan.nothing_to_do(),
            "a blocked plan is not 'everything is installed'"
        );
    }

    #[test]
    fn headless_plan_with_root_or_passwordless_sudo_keeps_its_work_and_asks_for_nothing() {
        for route in [HeadlessRoute::Root, HeadlessRoute::SudoNoPassword] {
            let mut plan = plan_provision(
                &env(&["mediainfo"]),
                &names(&["ffmpeg"]),
                false,
                Some("1.7.1"),
                Some("1.7.1"),
                false,
            );
            apply_headless_route(&mut plan, route);
            assert!(
                plan.has_privileged_work() && !plan.needs_password && plan.blockers.is_empty(),
                "{route:?}"
            );
        }
    }

    #[test]
    fn headless_route_ignores_plans_that_need_no_privileged_step() {
        let mut helper_only = plan_provision(
            &json!({"os": {"package_manager": "apt", "package_manager_available": true}}),
            &[],
            false,
            None,
            Some("1.7.1"),
            true,
        );
        apply_headless_route(&mut helper_only, HeadlessRoute::Unavailable);
        assert!(helper_only.blockers.is_empty() && helper_only.helper.needs_work());
        let mut brew = plan_provision(
            &mac_env(true, true, &["mediainfo"]),
            &[],
            false,
            Some("1.7.1"),
            Some("1.7.1"),
            false,
        );
        apply_headless_route(&mut brew, HeadlessRoute::Unavailable);
        assert!(
            brew.blockers.is_empty() && brew.has_privileged_work(),
            "Homebrew runs as the user"
        );
        let mut winget = plan_provision(
            &windows_env(true, &["ffmpeg"], "C:/ffmpeg.exe"),
            &[],
            false,
            Some("1.7.1"),
            Some("1.7.1"),
            false,
        );
        assert!(winget.needs_password);
        apply_headless_route(&mut winget, HeadlessRoute::Unavailable);
        assert!(
            winget
                .blockers
                .iter()
                .any(|blocker| blocker.key == "headless-privilege"),
            "a UAC prompt cannot be answered over HTTP"
        );
    }

    #[test]
    fn homebrew_installs_and_upgrades_run_as_the_user_with_no_password() {
        let plan = plan_provision(
            &mac_env(true, true, &["mediainfo"]),
            &names(&["ffmpeg"]),
            false,
            Some("1.7.1"),
            Some("1.7.1"),
            false,
        );
        assert_eq!(plan.manager, Manager::Brew);
        assert!(!plan.needs_password, "Homebrew never needs a password");
        assert!(plan.blockers.is_empty());
        assert_eq!(
            brew_commands(&plan),
            vec![
                names(&["update"]),
                names(&["install", "mediainfo"]),
                names(&["upgrade", "ffmpeg"]),
            ]
        );
    }

    #[test]
    fn a_mac_without_homebrew_or_command_line_tools_is_blocked_with_the_remedy_to_run() {
        let plan = plan_provision(
            &mac_env(false, false, &["ffmpeg"]),
            &[],
            false,
            Some("1.7.1"),
            Some("1.7.1"),
            false,
        );
        let keys: Vec<&str> = plan.blockers.iter().map(|b| b.key.as_str()).collect();
        assert_eq!(
            keys,
            vec!["xcode_clt", "brew"],
            "Command Line Tools come before Homebrew, which needs them"
        );
        assert!(
            plan.install_packages.is_empty() && !plan.has_privileged_work(),
            "package work waits for the prerequisites"
        );
        assert!(plan.blockers[0]
            .command
            .as_deref()
            .unwrap()
            .contains("xcode-select --install"));
        assert!(plan.blockers[1]
            .command
            .as_deref()
            .unwrap()
            .contains("raw.githubusercontent.com/Homebrew/install"));
        assert!(!plan.nothing_to_do(), "blockers are something to do");
    }

    #[test]
    fn a_blocked_mac_still_installs_the_user_space_helper() {
        let plan = plan_provision(
            &mac_env(false, true, &[]),
            &[],
            false,
            None,
            Some("1.7.1"),
            false,
        );
        assert_eq!(plan.helper, HelperAction::Install);
        assert_eq!(plan.blockers.len(), 1);
    }

    #[test]
    fn winget_uses_the_full_ffmpeg_build_and_one_elevated_script() {
        assert_eq!(
            winget_id("ffmpeg"),
            Some("Gyan.FFmpeg"),
            "the full build, not Gyan.FFmpeg.Essentials"
        );
        assert_eq!(winget_id("mediainfo"), Some("MediaArea.MediaInfo"));
        assert_eq!(winget_id("mkvtoolnix"), Some("MoritzBunkus.MKVToolNix"));
        assert_eq!(winget_id("fontconfig"), None);
        let plan = plan_provision(
            &windows_env(true, &["mediainfo"], "C:/ffmpeg/bin/ffmpeg.exe"),
            &names(&["ffmpeg"]),
            false,
            Some("1.7.1"),
            Some("1.7.1"),
            false,
        );
        assert_eq!(plan.manager, Manager::Winget);
        assert!(
            plan.needs_password,
            "one Windows administrator approval for the whole batch"
        );
        let script = winget_script(&plan, "C:\\Temp\\setup.log").unwrap();
        assert!(script.contains("& 'winget' 'install' '--exact' '--id' 'MediaArea.MediaInfo'"));
        assert!(script.contains("'winget' 'upgrade' '--exact' '--id' 'Gyan.FFmpeg'"));
        assert_eq!(powershell_quote("it's"), "'it''s'");
        assert!(script.contains("--accept-package-agreements") && script.contains("--silent"));
        assert!(
            script.contains("-1978335189"),
            "already-current is success, not failure"
        );
        assert!(script.contains("exit $failed"));
        assert!(script.contains("C:\\Temp\\setup.log"));
    }

    #[test]
    fn the_windows_elevation_wrapper_prompts_once_and_waits() {
        let args = windows_elevation_arguments("C:\\Temp\\setup.ps1");
        let joined = args.join(" ");
        assert!(
            joined.contains("-Verb RunAs")
                && joined.contains("-Wait")
                && joined.contains("-PassThru")
        );
        assert!(joined.contains("C:\\Temp\\setup.ps1"));
        assert!(joined.contains("exit $p.ExitCode"));
        assert!(args[0] == "-NoProfile");
    }

    #[test]
    fn winget_without_the_app_installer_is_blocked() {
        let plan = plan_provision(
            &windows_env(false, &["ffmpeg"], ""),
            &[],
            false,
            Some("1.7.1"),
            Some("1.7.1"),
            false,
        );
        assert_eq!(plan.blockers[0].key, "winget");
        assert!(plan.blockers[0].remedy.contains("App Installer"));
        assert!(winget_script(&plan, "x").is_none());
    }

    #[test]
    fn an_essentials_ffmpeg_gets_a_plain_advisory_to_use_the_full_build() {
        let plan = plan_provision(&windows_env(true, &[], "C:/Users/a/AppData/Local/Microsoft/WinGet/Packages/Gyan.FFmpeg.Essentials_x/ffmpeg.exe"), &[], false, Some("1.7.1"), Some("1.7.1"), false);
        assert!(plan
            .advisories
            .iter()
            .any(|text| text.contains("Essentials") && text.contains("full build")));
        let full = plan_provision(
            &windows_env(
                true,
                &[],
                "C:/Users/a/AppData/Local/Microsoft/WinGet/Packages/Gyan.FFmpeg_x/ffmpeg.exe",
            ),
            &[],
            false,
            Some("1.7.1"),
            Some("1.7.1"),
            false,
        );
        assert!(full.advisories.is_empty());
    }

    #[test]
    fn homebrew_outdated_json_is_parsed_and_limited_to_our_formulae() {
        let json = r#"{"formulae":[{"name":"ffmpeg","installed_versions":["7.0"],"current_version":"7.1"},{"name":"wget","current_version":"1"}],"casks":[]}"#;
        assert_eq!(parse_brew_outdated(json), names(&["ffmpeg"]));
        assert!(parse_brew_outdated("not json").is_empty());
    }

    #[test]
    fn winget_upgrade_listing_matches_ids_not_translated_text() {
        let text = "Name   Id   Version Available\n------\nFFmpeg Gyan.FFmpeg 7.0 7.1 winget\nSomething Else.App 1 2 winget\n";
        assert_eq!(parse_winget_upgradable(text), names(&["ffmpeg"]));
        assert!(
            parse_winget_upgradable("No installed package found matching input criteria.")
                .is_empty()
        );
    }

    #[test]
    fn automatic_mode_never_applies_work_that_needs_a_password() {
        let apt = plan_provision(
            &env(&["mediainfo"]),
            &names(&["ffmpeg"]),
            true,
            None,
            Some("1.7.1"),
            false,
        )
        .user_space_only();
        assert!(
            !apt.has_privileged_work() && !apt.needs_password,
            "apt work waits for the owner"
        );
        assert_eq!(
            apt.helper,
            HelperAction::Install,
            "the helper still updates silently"
        );
        assert!(!apt.nothing_to_do());

        let winget = plan_provision(
            &windows_env(true, &["ffmpeg"], "C:/x/ffmpeg.exe"),
            &[],
            false,
            Some("1.7.1"),
            Some("1.7.1"),
            false,
        )
        .user_space_only();
        assert!(
            !winget.has_privileged_work(),
            "a Windows elevation prompt is never raised automatically"
        );

        let brew = plan_provision(
            &mac_env(true, true, &["mediainfo"]),
            &names(&["ffmpeg"]),
            false,
            Some("1.7.1"),
            Some("1.7.1"),
            false,
        )
        .user_space_only();
        assert!(
            brew.has_privileged_work() && !brew.needs_password,
            "Homebrew changes run as the user, so they may be automatic"
        );

        let current = plan_provision(&env(&[]), &[], false, Some("1.7.1"), Some("1.7.1"), false)
            .user_space_only();
        assert!(current.nothing_to_do());
    }

    #[test]
    fn dotted_versions_compare_numerically_not_lexically() {
        assert!(version_is_older("1.7.0", "1.7.1"));
        assert!(version_is_older("1.9.0", "1.10.0"));
        assert!(version_is_older("1.7", "1.7.1"));
        assert!(!version_is_older("1.7.1", "1.7.1"));
        assert!(!version_is_older("2.0.0", "1.9.9"));
        assert!(!version_is_older("garbage", "1.0.0"));
        assert!(version_is_older("1.7.0", "v1.8.0"));
    }

    #[test]
    fn helper_action_covers_missing_outdated_current_and_unchecked() {
        assert_eq!(helper_action(None, Some("1.7.1")), HelperAction::Install);
        assert_eq!(helper_action(None, None), HelperAction::Install);
        assert_eq!(
            helper_action(Some("1.7.0"), Some("1.7.1")),
            HelperAction::Update {
                installed: "1.7.0".into(),
                latest: "1.7.1".into()
            }
        );
        assert_eq!(
            helper_action(Some("1.7.1"), Some("1.7.1")),
            HelperAction::Current {
                installed: "1.7.1".into()
            }
        );
        assert_eq!(
            helper_action(Some("1.7.1"), None),
            HelperAction::Unchecked {
                installed: "1.7.1".into()
            }
        );
    }

    #[test]
    fn one_plan_installs_missing_and_upgrades_installed_with_one_password() {
        let plan = plan_provision(
            &env(&["mediainfo"]),
            &names(&["ffmpeg", "mkvtoolnix"]),
            false,
            None,
            Some("1.7.1"),
            false,
        );
        assert!(
            plan.refresh_catalog,
            "missing packages refresh the catalog inside the same elevated step"
        );
        assert_eq!(plan.install_packages, names(&["mediainfo"]));
        assert_eq!(plan.upgrade_packages, names(&["ffmpeg", "mkvtoolnix"]));
        assert_eq!(plan.helper, HelperAction::Install);
        assert!(plan.needs_password);
        assert!(!plan.nothing_to_do());
    }

    #[test]
    fn a_root_session_never_needs_a_password_and_helper_only_work_never_does() {
        let root = plan_provision(
            &env(&[]),
            &names(&["ffmpeg"]),
            false,
            Some("1.7.1"),
            Some("1.7.1"),
            true,
        );
        assert!(!root.needs_password);
        // No apt package is installed or missing, so the only work is the user-space helper.
        let helper_only = plan_provision(&env(&[]), &[], false, None, Some("1.7.1"), false);
        assert!(helper_only.install_packages.is_empty());
        assert!(
            !helper_only.needs_password,
            "helper-only work is user space: no password"
        );
        assert_eq!(helper_only.helper, HelperAction::Install);
    }

    #[test]
    fn installed_packages_outside_the_allowlist_are_never_planned() {
        let plan = plan_provision(
            &env(&["not-allowed"]),
            &names(&["not-allowed-either", "ffmpeg"]),
            false,
            Some("1.7.1"),
            Some("1.7.1"),
            false,
        );
        assert!(plan.install_packages.is_empty());
        assert_eq!(plan.upgrade_packages, names(&["ffmpeg"]));
    }

    #[test]
    fn a_fully_current_host_has_nothing_to_do() {
        let plan = plan_provision(&env(&[]), &[], false, Some("1.7.1"), Some("1.7.1"), false);
        assert!(plan.nothing_to_do());
        assert!(!plan.needs_password);
    }

    #[test]
    fn shell_quoting_neutralises_every_metacharacter() {
        assert_eq!(shell_quote("ffmpeg"), "'ffmpeg'");
        assert_eq!(shell_quote("a b"), "'a b'");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        assert_eq!(shell_quote("$(rm -rf /);`x`"), "'$(rm -rf /);`x`'");
    }

    #[test]
    fn the_privileged_script_is_one_fail_fast_batch() {
        let plan = plan_provision(
            &env(&["mediainfo"]),
            &names(&["ffmpeg"]),
            false,
            Some("1.7.1"),
            Some("1.7.1"),
            false,
        );
        let script = privileged_script(&plan).expect("apt work exists");
        let lines: Vec<&str> = script.split('\n').collect();
        assert_eq!(lines[0], "set -eu");
        assert!(lines[1].contains("'apt-get' 'update'"));
        assert!(
            lines[2].contains("'install'")
                && lines[2].contains("'--no-upgrade'")
                && lines[2].contains("'--no-remove'")
                && lines[2].ends_with("'--' 'mediainfo'")
        );
        assert!(lines[3].contains("'--only-upgrade'") && lines[3].ends_with("'--' 'ffmpeg'"));
        assert!(lines
            .iter()
            .skip(1)
            .all(|line| line.starts_with("DEBIAN_FRONTEND=noninteractive ")));
        assert_eq!(lines.len(), 4);
    }

    #[test]
    fn a_stale_catalog_alone_is_refreshed_but_a_fresh_one_costs_no_prompt() {
        let stale = plan_provision(&env(&[]), &[], true, Some("1.7.1"), Some("1.7.1"), false);
        assert!(stale.refresh_catalog && stale.needs_password);
        let script = privileged_script(&stale).unwrap();
        assert_eq!(
            script.split('\n').count(),
            2,
            "only the catalog refresh runs"
        );
        let fresh = plan_provision(&env(&[]), &[], false, Some("1.7.1"), Some("1.7.1"), false);
        assert!(!fresh.needs_password && fresh.nothing_to_do());
    }

    #[test]
    fn a_host_without_apt_plans_no_package_work() {
        let environment = json!({"missing_apt_packages": ["ffmpeg"], "os": {"package_manager": "brew", "package_manager_available": true}});
        let plan = plan_provision(
            &environment,
            &names(&["ffmpeg"]),
            true,
            None,
            Some("1.7.1"),
            false,
        );
        assert!(!plan.has_privileged_work() && !plan.needs_password);
    }

    #[test]
    fn helper_only_plans_have_no_privileged_script() {
        let plan = plan_provision(&env(&[]), &[], false, None, Some("1.7.1"), false);
        assert!(privileged_script(&plan).is_none());
    }
}
