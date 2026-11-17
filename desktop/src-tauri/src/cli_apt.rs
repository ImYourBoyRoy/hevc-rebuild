//! The `json provision-plan` and `json provision --confirm` actions.

use super::*;

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
    if !plan.has_privileged_work() && !plan.helper.needs_work() {
        eprintln!("Finish the listed prerequisites first, then run provision again.");
        println!("{payload}");
        return 1;
    }
    // sudo asks for the password once on this terminal; output is forwarded as it arrives.
    let sink: provision_run::LineSink = Arc::new(|line| eprintln!("{line}"));
    match provision_run::run_plan(&plan, provision_run::Elevation::Sudo, sink) {
        Ok(outcome) => {
            if let Some(path) = outcome.helper_installed_at {
                payload["helper_installed_at"] = json!(path);
            }
        }
        Err(error) => {
            eprintln!("{error}");
            return 1;
        }
    }
    payload["applied"] = json!(true);
    println!("{payload}");
    0
}
