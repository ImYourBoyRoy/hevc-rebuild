//! One-pass toolchain provisioning: a pure plan for "install and update everything" that needs
//! at most one administrator prompt. The plan is shared by the GUI, CLI and MCP so a person or a
//! model sees and approves exactly the same steps, and none of them ever handles the password.

use super::*;

/// What to do about the HDR10+ helper, which is installed in user space and never needs a password.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HelperAction {
    Install,
    Update { installed: String, latest: String },
    Current { installed: String },
    /// Installed, but the latest release could not be checked (offline, rate limited).
    Unchecked { installed: String },
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

/// Everything the privileged (password) step and the user-space step will do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProvisionPlan {
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

    pub(crate) fn nothing_to_do(&self) -> bool {
        !self.has_privileged_work() && !self.helper.needs_work()
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
            HelperAction::Update { installed: installed.to_string(), latest: latest.trim_start_matches('v').to_string() }
        }
        (Some(installed), Some(_)) => HelperAction::Current { installed: installed.to_string() },
        (Some(installed), None) => HelperAction::Unchecked { installed: installed.to_string() },
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
    let allowed = |name: &String| APT_ALLOWLIST.contains(&name.as_str());
    let mut install_packages = environment
        .get("missing_apt_packages")
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
    let apt_usable = environment.pointer("/os/package_manager").and_then(Value::as_str) == Some("apt")
        && environment.pointer("/os/package_manager_available").and_then(Value::as_bool) == Some(true);
    if !apt_usable {
        install_packages.clear();
        upgrade_packages.clear();
    }
    // The catalog is refreshed inside the same elevated step only when it is stale or packages are
    // missing; a fresh catalog with nothing to apply costs the user no prompt at all.
    let refresh_catalog = apt_usable && (catalog_stale || !install_packages.is_empty());
    ProvisionPlan {
        needs_password: refresh_catalog && !is_root,
        refresh_catalog,
        install_packages,
        upgrade_packages,
        helper: helper_action(helper_installed, helper_latest),
    }
}

pub(crate) fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// The fixed script run as root. Every argument is quoted; package names come from the allowlist.
pub(crate) fn privileged_script(plan: &ProvisionPlan) -> Option<String> {
    if !plan.has_privileged_work() {
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
            &["install", "--yes", "--no-upgrade", "--no-install-recommends", "--no-remove"],
            &plan.install_packages,
        ));
    }
    if !plan.upgrade_packages.is_empty() {
        lines.push(command(
            &["install", "--only-upgrade", "--yes", "--no-install-recommends", "--no-remove"],
            &plan.upgrade_packages,
        ));
    }
    Some(lines.join("\n"))
}

/// Gather the live state (environment, dpkg, GitHub release) and build the plan. A failed release
/// lookup is reported, not fatal: apt work can still proceed and an installed helper is left alone.
pub(crate) fn current_plan(tool_paths: ToolPaths) -> Result<(ProvisionPlan, Option<String>), String> {
    let environment = environment::inspect_environment(tool_paths, None)?;
    if environment.pointer("/os/supported").and_then(Value::as_bool) != Some(true) {
        return Err("One-pass setup is limited to supported Ubuntu, Linux Mint, Windows and macOS releases.".into());
    }
    let (upgradable, catalog_stale) = if cfg!(target_os = "linux") {
        match dependencies::apt_update_status() {
            Ok(status) => (
                status["packages"].as_array().into_iter().flatten()
                    .filter_map(|package| package["name"].as_str().map(str::to_string)).collect(),
                status["catalog_stale"].as_bool().unwrap_or(true),
            ),
            Err(_) => (Vec::new(), true),
        }
    } else {
        (Vec::new(), false)
    };
    let (latest, latest_error) = match dependencies::latest_hdr10plus_version() {
        Ok(version) => (Some(version), None),
        Err(error) => (None, Some(error)),
    };
    let installed = dependencies::installed_hdr10plus_version(&environment);
    let is_root = unsafe { libc_geteuid() } == 0;
    let mut plan = plan_provision(&environment, &upgradable, catalog_stale, installed.as_deref(), latest.as_deref(), is_root);
    if plan.needs_password && !is_root && !executable_in_path("pkexec") {
        return Err("Desktop administrator authentication (pkexec) is not installed.".into());
    }
    if !cfg!(target_os = "linux") {
        plan.needs_password = false;
    }
    Ok((plan, latest_error))
}

pub(crate) fn plan_json(plan: &ProvisionPlan, latest_error: Option<&str>) -> Value {
    let mut steps = Vec::new();
    if plan.refresh_catalog {
        steps.push("Refresh the Ubuntu package catalog".to_string());
    }
    if !plan.install_packages.is_empty() {
        steps.push(format!("Install missing packages: {}", plan.install_packages.join(", ")));
    }
    if !plan.upgrade_packages.is_empty() {
        steps.push(format!("Update installed packages: {}", plan.upgrade_packages.join(", ")));
    }
    match &plan.helper {
        HelperAction::Install => steps.push("Install the HDR10+ helper (hdr10plus_tool) without a password".to_string()),
        HelperAction::Update { installed, latest } => steps.push(format!("Update the HDR10+ helper {installed} to {latest} without a password")),
        _ => {}
    }
    json!({
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
            HelperAction::Update { installed: "1.7.0".into(), latest: "1.7.1".into() }
        );
        assert_eq!(
            helper_action(Some("1.7.1"), Some("1.7.1")),
            HelperAction::Current { installed: "1.7.1".into() }
        );
        assert_eq!(
            helper_action(Some("1.7.1"), None),
            HelperAction::Unchecked { installed: "1.7.1".into() }
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
        assert!(plan.refresh_catalog, "missing packages refresh the catalog inside the same elevated step");
        assert_eq!(plan.install_packages, names(&["mediainfo"]));
        assert_eq!(plan.upgrade_packages, names(&["ffmpeg", "mkvtoolnix"]));
        assert_eq!(plan.helper, HelperAction::Install);
        assert!(plan.needs_password);
        assert!(!plan.nothing_to_do());
    }

    #[test]
    fn a_root_session_never_needs_a_password_and_helper_only_work_never_does() {
        let root = plan_provision(&env(&[]), &names(&["ffmpeg"]), false, Some("1.7.1"), Some("1.7.1"), true);
        assert!(!root.needs_password);
        // No apt package is installed or missing, so the only work is the user-space helper.
        let helper_only = plan_provision(&env(&[]), &[], false, None, Some("1.7.1"), false);
        assert!(helper_only.install_packages.is_empty());
        assert!(!helper_only.needs_password, "helper-only work is user space: no password");
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
        assert!(lines[2].contains("'install'") && lines[2].contains("'--no-upgrade'")
            && lines[2].contains("'--no-remove'") && lines[2].ends_with("'--' 'mediainfo'"));
        assert!(lines[3].contains("'--only-upgrade'") && lines[3].ends_with("'--' 'ffmpeg'"));
        assert!(lines.iter().skip(1).all(|line| line.starts_with("DEBIAN_FRONTEND=noninteractive ")));
        assert_eq!(lines.len(), 4);
    }

    #[test]
    fn a_stale_catalog_alone_is_refreshed_but_a_fresh_one_costs_no_prompt() {
        let stale = plan_provision(&env(&[]), &[], true, Some("1.7.1"), Some("1.7.1"), false);
        assert!(stale.refresh_catalog && stale.needs_password);
        let script = privileged_script(&stale).unwrap();
        assert_eq!(script.split('\n').count(), 2, "only the catalog refresh runs");
        let fresh = plan_provision(&env(&[]), &[], false, Some("1.7.1"), Some("1.7.1"), false);
        assert!(!fresh.needs_password && fresh.nothing_to_do());
    }

    #[test]
    fn a_host_without_apt_plans_no_package_work() {
        let environment = json!({"missing_apt_packages": ["ffmpeg"], "os": {"package_manager": "brew", "package_manager_available": true}});
        let plan = plan_provision(&environment, &names(&["ffmpeg"]), true, None, Some("1.7.1"), false);
        assert!(!plan.has_privileged_work() && !plan.needs_password);
    }

    #[test]
    fn helper_only_plans_have_no_privileged_script() {
        let plan = plan_provision(&env(&[]), &[], false, None, Some("1.7.1"), false);
        assert!(privileged_script(&plan).is_none());
    }
}
