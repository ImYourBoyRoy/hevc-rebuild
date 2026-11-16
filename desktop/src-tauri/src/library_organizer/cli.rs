//! The `organizer` command-line mode: plan documents in, preview or applied results out.

use super::*;

pub(crate) fn read_plan_document(path: &Path) -> Result<RenamePlanDocument, String> {
    let metadata = fs::metadata(path)
        .map_err(|error| format!("Could not inspect the rename plan file: {error}"))?;
    if !metadata.is_file() || metadata.len() > MAX_PLAN_JSON_BYTES as u64 {
        return Err("Rename plan JSON must be a regular file no larger than 2 MiB.".into());
    }
    let bytes =
        fs::read(path).map_err(|error| format!("Could not read the rename plan file: {error}"))?;
    if bytes.len() > MAX_PLAN_JSON_BYTES {
        return Err("Rename plan JSON must be no larger than 2 MiB.".into());
    }
    serde_json::from_slice(&bytes).map_err(|error| format!("Rename plan JSON is invalid: {error}"))
}

fn plan_text_blacklist(plan: &RenamePlanDocument) -> Result<Vec<String>, String> {
    match &plan.text_blacklist_patterns {
        Some(patterns) => Ok(patterns.clone()),
        None => crate::settings::metadata_text_blacklist_for_root(&plan.root),
    }
}

pub(crate) fn organizer_cli_mode(args: &[String]) -> i32 {
    let usage = "Usage:\n  hevc-rebuild-studio organize scan ROOT\n  hevc-rebuild-studio organize preview PLAN.json\n  hevc-rebuild-studio organize apply PLAN.json --preview-id ID --confirm";
    let result = match args {
        [mode, root] if mode == "scan" => scan_organizer_folder(root.clone())
            .and_then(|value| serde_json::to_value(value).map_err(|error| error.to_string())),
        [mode, plan_path] if mode == "preview" => read_plan_document(Path::new(plan_path))
            .and_then(|plan| {
                let patterns = plan_text_blacklist(&plan)?;
                preview_rename_batch_with_patterns(plan.root, plan.items, patterns)
            })
            .and_then(|value| serde_json::to_value(value).map_err(|error| error.to_string())),
        [mode, plan_path, preview_flag, preview_id, confirm]
            if mode == "apply" && preview_flag == "--preview-id" && confirm == "--confirm" =>
        {
            read_plan_document(Path::new(plan_path))
                .and_then(|plan| {
                    let patterns = plan_text_blacklist(&plan)?;
                    apply_rename_batch_with_patterns(
                        plan.root,
                        plan.items,
                        patterns,
                        preview_id.clone(),
                        true,
                    )
                })
                .and_then(|value| serde_json::to_value(value).map_err(|error| error.to_string()))
        }
        _ => {
            eprintln!("{usage}");
            return 2;
        }
    };
    match result {
        Ok(value) => match serde_json::to_string_pretty(&value) {
            Ok(json) => {
                println!("{json}");
                0
            }
            Err(error) => {
                eprintln!("Could not format organizer JSON: {error}");
                2
            }
        },
        Err(error) => {
            eprintln!("Organizer operation failed: {error}");
            1
        }
    }
}
