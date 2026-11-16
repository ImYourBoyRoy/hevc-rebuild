//! Allow-list and confirmation rules for Ubuntu package operations shared by the GUI, CLI and MCP.

use super::*;

pub(crate) const APT_ALLOWLIST: &[&str] = &[
    "ffmpeg",
    "mediainfo",
    "mkvtoolnix",
    "fontconfig",
    "ca-certificates",
];

pub(crate) fn validate_apt_update_confirmation(
    status: &Value,
    mut confirmed: Vec<String>,
) -> Result<Vec<String>, String> {
    if status.get("catalog_stale").and_then(Value::as_bool) == Some(true) {
        return Err("The local APT package catalog is stale. Refresh it explicitly, then check updates again.".into());
    }
    let mut available = status
        .get("packages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|package| package.get("name").and_then(Value::as_str))
        .map(str::to_string)
        .collect::<Vec<_>>();
    available.sort();
    available.dedup();
    if available
        .iter()
        .any(|package| !APT_ALLOWLIST.contains(&package.as_str()))
    {
        return Err(
            "The package update report contained a package outside the fixed allowlist.".into(),
        );
    }
    let provided_count = confirmed.len();
    confirmed.sort();
    confirmed.dedup();
    if confirmed.len() != provided_count {
        return Err("Each package can appear only once in the confirmation list.".into());
    }
    if available.is_empty() {
        return Err("The checked Ubuntu package catalog reports no available updates.".into());
    }
    if confirmed != available {
        return Err(format!(
            "Package update confirmation must exactly match the current checked list: {}.",
            available.join(", ")
        ));
    }
    Ok(available)
}

pub(crate) fn validate_apt_upgrade_plan(packages: &[String]) -> Result<(), String> {
    if !cfg!(target_os = "linux") {
        return Err("APT package operations are available only on Linux hosts.".into());
    }
    if packages.is_empty()
        || packages
            .iter()
            .any(|package| !APT_ALLOWLIST.contains(&package.as_str()))
    {
        return Err("APT upgrades accept only the fixed Ubuntu package allowlist.".into());
    }
    let output = Command::new("apt-get")
        .args([
            "-s",
            "install",
            "--only-upgrade",
            "--no-install-recommends",
            "--no-remove",
            "--",
        ])
        .args(packages)
        .output()
        .map_err(|error| format!("Could not simulate the APT upgrade plan: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "APT could not simulate this update plan: {}",
            String::from_utf8_lossy(&output.stderr)
                .chars()
                .take(2000)
                .collect::<String>()
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut planned = BTreeSet::new();
    let mut removals = Vec::new();
    for line in stdout.lines() {
        if let Some(package) = line
            .strip_prefix("Inst ")
            .and_then(|rest| rest.split_whitespace().next())
        {
            planned.insert(package.split(':').next().unwrap_or(package).to_string());
        }
        if let Some(package) = line
            .strip_prefix("Rem ")
            .and_then(|rest| rest.split_whitespace().next())
        {
            removals.push(package.to_string());
        }
    }
    let requested = packages.iter().cloned().collect::<BTreeSet<_>>();
    if planned != requested || !removals.is_empty() {
        let planned = planned.into_iter().collect::<Vec<_>>().join(", ");
        let removed = removals.join(", ");
        return Err(format!(
            "APT's simulated plan includes changes beyond the confirmed package list (upgrade plan: [{}]; removals: [{}]). No package changes were made.",
            planned, removed
        ));
    }
    Ok(())
}

fn validate_apt_install_plan(packages: &[String]) -> Result<(), String> {
    if !cfg!(target_os = "linux") {
        return Err("APT package operations are available only on Linux hosts.".into());
    }
    if packages.is_empty()
        || packages
            .iter()
            .any(|package| !APT_ALLOWLIST.contains(&package.as_str()))
    {
        return Err("APT installs accept only the fixed Ubuntu package allowlist.".into());
    }
    let output = Command::new("apt-get")
        .args([
            "-s",
            "install",
            "--no-upgrade",
            "--no-install-recommends",
            "--no-remove",
            "--",
        ])
        .args(packages)
        .output()
        .map_err(|error| format!("Could not simulate the APT install plan: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "APT could not simulate this install plan: {}",
            String::from_utf8_lossy(&output.stderr)
                .chars()
                .take(2000)
                .collect::<String>()
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    if stdout.lines().any(|line| line.starts_with("Rem ")) {
        return Err(
            "APT's simulated install plan would remove a package. No package changes were made."
                .into(),
        );
    }
    Ok(())
}

fn validate_apt_missing_confirmation(
    environment: &Value,
    mut confirmed: Vec<String>,
) -> Result<Vec<String>, String> {
    let mut missing = environment
        .get("missing_apt_packages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect::<Vec<_>>();
    missing.sort();
    missing.dedup();
    if missing
        .iter()
        .any(|package| !APT_ALLOWLIST.contains(&package.as_str()))
    {
        return Err(
            "The environment report included a package outside the fixed APT allowlist.".into(),
        );
    }
    let provided_count = confirmed.len();
    confirmed.sort();
    confirmed.dedup();
    if confirmed.len() != provided_count || confirmed != missing {
        return Err(format!(
            "Missing-package confirmation must exactly match the current environment report: {}.",
            missing.join(", ")
        ));
    }
    Ok(missing)
}

pub(crate) fn validate_apt_toolchain_confirmation(
    environment: &Value,
    update_status: &Value,
    confirmed_missing: Vec<String>,
    confirmed_updates: Vec<String>,
) -> Result<(Vec<String>, Vec<String>), String> {
    let missing = validate_apt_missing_confirmation(environment, confirmed_missing)?;
    let updates = if update_status
        .get("packages")
        .and_then(Value::as_array)
        .is_some_and(|packages| !packages.is_empty())
    {
        validate_apt_update_confirmation(update_status, confirmed_updates)?
    } else {
        if update_status.get("catalog_stale").and_then(Value::as_bool) == Some(true) {
            return Err("The local APT package catalog is stale. Refresh it, then review the tool setup again.".into());
        }
        if !confirmed_updates.is_empty() {
            return Err("The checked Ubuntu package catalog reports no available updates.".into());
        }
        Vec::new()
    };
    if missing.is_empty() && updates.is_empty() {
        return Err("No missing or outdated allowlisted Ubuntu packages need changes.".into());
    }
    if missing.iter().any(|package| updates.contains(package)) {
        return Err("A package cannot be confirmed as both missing and outdated.".into());
    }
    if !missing.is_empty() {
        validate_apt_install_plan(&missing)?;
    }
    if !updates.is_empty() {
        validate_apt_upgrade_plan(&updates)?;
    }
    Ok((missing, updates))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apt_upgrade_confirmation_must_match_the_fresh_exact_list() {
        let status = json!({
            "catalog_stale": false,
            "packages": [
                {"name": "ffmpeg"},
                {"name": "mediainfo"}
            ]
        });
        let approved =
            validate_apt_update_confirmation(&status, vec!["mediainfo".into(), "ffmpeg".into()])
                .expect("accept exact package list independent of order");
        assert_eq!(approved, vec!["ffmpeg", "mediainfo"]);
        assert!(validate_apt_update_confirmation(&status, vec!["ffmpeg".into()]).is_err());
        assert!(validate_apt_update_confirmation(
            &status,
            vec!["ffmpeg".into(), "ffmpeg".into(), "mediainfo".into()]
        )
        .is_err());
    }

    #[test]
    fn apt_toolchain_setup_requires_exact_missing_package_confirmation() {
        let environment = json!({"missing_apt_packages": ["mediainfo", "ffmpeg"]});
        let approved = validate_apt_missing_confirmation(
            &environment,
            vec!["ffmpeg".into(), "mediainfo".into()],
        )
        .expect("accept the exact missing package list independent of order");
        assert_eq!(approved, vec!["ffmpeg", "mediainfo"]);
        assert!(validate_apt_missing_confirmation(&environment, vec!["ffmpeg".into()]).is_err());
        assert!(validate_apt_missing_confirmation(
            &environment,
            vec!["ffmpeg".into(), "ffmpeg".into(), "mediainfo".into()]
        )
        .is_err());
        assert!(validate_apt_missing_confirmation(
            &json!({"missing_apt_packages": ["unlisted-package"]}),
            vec!["unlisted-package".into()]
        )
        .is_err());
    }

    #[test]
    fn apt_toolchain_setup_rejects_stale_catalog_before_any_package_plan() {
        let error = validate_apt_toolchain_confirmation(
            &json!({"missing_apt_packages": []}),
            &json!({"catalog_stale": true, "packages": []}),
            Vec::new(),
            Vec::new(),
        )
        .expect_err("stale package catalogs must be refreshed before changes are reviewed");
        assert!(error.contains("stale"));
    }

    #[test]
    fn apt_upgrade_confirmation_rejects_a_stale_catalog() {
        let status = json!({"catalog_stale": true, "packages": [{"name": "ffmpeg"}]});
        assert!(validate_apt_update_confirmation(&status, vec!["ffmpeg".into()]).is_err());
    }
}
