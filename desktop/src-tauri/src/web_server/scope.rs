//! `--root` confinement for the browser API. When the server is started with one or more `--root DIR`, the
//! path browser lists only inside those folders and every command argument that names a media or output
//! path must resolve inside one of them (symlinks and `..` included). This is a guard rail for the browser
//! UI, not a sandbox against the token holder: tool paths, the queue document's data and the MCP tools
//! are not confined (the MCP endpoint is therefore not offered with `--root` unless asked for).

use serde_json::Value;
use std::path::{Component, Path, PathBuf};

/// The folders the server may touch. Empty means unrestricted (the default).
#[derive(Clone, Debug, Default)]
pub(crate) struct PathScope {
    roots: Vec<PathBuf>,
}

const OUTSIDE: &str = "That path is outside the folders this server may use (--root).";

impl PathScope {
    pub(crate) fn new(roots: &[PathBuf]) -> Result<PathScope, String> {
        let mut canonical = Vec::new();
        for root in roots {
            let resolved = root
                .canonicalize()
                .map_err(|error| format!("--root {} cannot be used: {error}", root.display()))?;
            if !resolved.is_dir() {
                return Err(format!("--root {} is not a folder.", root.display()));
            }
            canonical.push(resolved);
        }
        Ok(PathScope { roots: canonical })
    }

    pub(crate) fn is_restricted(&self) -> bool {
        !self.roots.is_empty()
    }

    pub(crate) fn first_root(&self) -> Option<&Path> {
        self.roots.first().map(PathBuf::as_path)
    }

    /// Resolve `path` as far as it exists (following symlinks), then keep the not-yet-existing tail as
    /// plain components, so an output file that will be created is judged by where it would land.
    fn resolve(path: &Path) -> Result<PathBuf, String> {
        if !path.is_absolute() {
            return Err("Use an absolute path.".into());
        }
        let mut existing = path.to_path_buf();
        let mut tail = Vec::new();
        loop {
            if let Ok(resolved) = existing.canonicalize() {
                let mut full = resolved;
                for part in tail.iter().rev() {
                    full.push(part);
                }
                return Ok(full);
            }
            match (
                existing.file_name().map(|name| name.to_owned()),
                existing.parent(),
            ) {
                (Some(name), Some(parent)) => {
                    if name == ".." {
                        return Err(OUTSIDE.into());
                    }
                    tail.push(name);
                    existing = parent.to_path_buf();
                }
                _ => return Err(OUTSIDE.into()),
            }
        }
    }

    /// Whether `path` (a string from a request) may be used.
    pub(crate) fn check(&self, path: &str) -> Result<(), String> {
        if !self.is_restricted() {
            return Ok(());
        }
        let resolved = Self::resolve(Path::new(path.trim()))?;
        if resolved
            .components()
            .any(|part| matches!(part, Component::ParentDir))
        {
            return Err(OUTSIDE.into());
        }
        if self.roots.iter().any(|root| resolved.starts_with(root)) {
            Ok(())
        } else {
            Err(OUTSIDE.into())
        }
    }

    pub(crate) fn contains(&self, path: &Path) -> bool {
        !self.is_restricted() || self.check(&path.to_string_lossy()).is_ok()
    }
}

/// Where each command carries media or output paths. Syntax: `a.b` nests, `a[]` is an array of paths,
/// `a{}` a map whose values are paths, `a{k}` a map whose keys are paths. Commands not listed carry none.
const PATH_ARGUMENTS: &[(&str, &[&str])] = &[
    ("list_directory", &["path"]),
    ("load_settings", &["root"]),
    ("resolve_work_directory_command", &["root", "workRoot"]),
    ("scan_folder", &["options.root"]),
    ("scan_organizer_folder", &["root"]),
    ("preview_library_rename", &["root"]),
    ("apply_library_rename", &["root"]),
    ("cache_local_library_artwork", &["sourcePath"]),
    (
        "save_library_queue_selection",
        &["selectedMediaPaths[]", "activeRoot"],
    ),
    ("export_tool_setup_log", &["destination"]),
    (
        "start_run",
        &[
            "request.root",
            "request.selectedFiles[]",
            "request.rebuildOutputDir",
            "request.youtubeOutputDir",
            "request.fileOutputDirs{k}",
            "request.fileOutputDirs{}",
            "request.filePosterOverrides{k}",
            "request.filePosterOverrides{}",
            "request.fileTuneOverrides{k}",
            "request.fileStreamOverrides{k}",
            "request.fileSettingsOverrides{k}",
            "request.fileMetadataOverrides{k}",
            "request.allowYoutubeHdr10plusLossPaths[]",
        ],
    ),
];

fn collect(value: &Value, expression: &str, found: &mut Vec<String>) {
    let (head, rest) = match expression.split_once('.') {
        Some((head, rest)) => (head, Some(rest)),
        None => (expression, None),
    };
    let (name, shape) = if let Some(name) = head.strip_suffix("[]") {
        (name, "[]")
    } else if let Some(name) = head.strip_suffix("{k}") {
        (name, "{k}")
    } else if let Some(name) = head.strip_suffix("{}") {
        (name, "{}")
    } else {
        (head, "")
    };
    let Some(child) = value.get(name) else {
        return;
    };
    if let Some(rest) = rest {
        collect(child, rest, found);
        return;
    }
    match shape {
        "[]" => {
            if let Some(items) = child.as_array() {
                found.extend(items.iter().filter_map(Value::as_str).map(str::to_string));
            }
        }
        "{k}" => {
            if let Some(map) = child.as_object() {
                found.extend(map.keys().cloned());
            }
        }
        "{}" => {
            if let Some(map) = child.as_object() {
                found.extend(map.values().filter_map(Value::as_str).map(str::to_string));
            }
        }
        _ => {
            if let Some(text) = child.as_str() {
                found.push(text.to_string());
            }
        }
    }
}

/// Every path a command's arguments name, per the table above.
pub(crate) fn command_paths(command: &str, args: &Value) -> Vec<String> {
    let mut found = Vec::new();
    if let Some((_, expressions)) = PATH_ARGUMENTS.iter().find(|(name, _)| *name == command) {
        for expression in *expressions {
            collect(args, expression, &mut found);
        }
    }
    found
}

/// Refuse the call before it runs if any named path is outside the scope.
pub(crate) fn check_command(scope: &PathScope, command: &str, args: &Value) -> Result<(), String> {
    if !scope.is_restricted() {
        return Ok(());
    }
    for path in command_paths(command, args) {
        if path.trim().is_empty() {
            continue;
        }
        scope.check(&path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tree() -> (PathBuf, PathBuf, PathScope) {
        let base = crate::test_dir("scope");
        let media = base.join("media");
        std::fs::create_dir_all(media.join("films")).unwrap();
        std::fs::create_dir_all(base.join("media-evil")).unwrap();
        std::fs::create_dir_all(base.join("private")).unwrap();
        let scope = PathScope::new(std::slice::from_ref(&media)).unwrap();
        (base, media, scope)
    }

    #[test]
    fn roots_must_be_existing_folders() {
        let base = crate::test_dir("scope-new");
        assert!(PathScope::new(&[base.join("missing")]).is_err());
        let file = base.join("file.txt");
        std::fs::write(&file, "x").unwrap();
        assert!(PathScope::new(&[file])
            .unwrap_err()
            .contains("not a folder"));
        assert!(!PathScope::new(&[]).unwrap().is_restricted());
        assert!(PathScope::new(std::slice::from_ref(&base))
            .unwrap()
            .is_restricted());
        std::fs::remove_dir_all(base).ok();
    }

    #[test]
    fn paths_inside_a_root_are_allowed_and_everything_else_is_not() {
        let (base, media, scope) = tree();
        assert!(scope.check(media.to_str().unwrap()).is_ok());
        assert!(scope.check(media.join("films").to_str().unwrap()).is_ok());
        assert!(scope.check(base.join("private").to_str().unwrap()).is_err());
        assert!(
            scope.check(base.to_str().unwrap()).is_err(),
            "the parent of a root is outside"
        );
        assert!(scope.check("/etc/passwd").is_err());
        assert!(
            scope
                .check(base.join("media-evil").to_str().unwrap())
                .is_err(),
            "a sibling that shares the prefix is outside"
        );
        std::fs::remove_dir_all(base).ok();
    }

    #[test]
    fn dot_dot_and_symlinks_cannot_leave_the_root() {
        let (base, media, scope) = tree();
        let sneaky = format!("{}/films/../../private", media.display());
        assert!(scope.check(&sneaky).is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(base.join("private"), media.join("link-out")).unwrap();
            assert!(
                scope
                    .check(media.join("link-out").to_str().unwrap())
                    .is_err(),
                "a symlink to the outside resolves outside"
            );
            assert!(scope
                .check(media.join("link-out/new.mkv").to_str().unwrap())
                .is_err());
        }
        let missing_dotdot = format!("{}/does-not-exist/../../private/x", media.display());
        assert!(scope.check(&missing_dotdot).is_err());
        std::fs::remove_dir_all(base).ok();
    }

    #[test]
    fn a_file_that_will_be_created_is_judged_by_where_it_would_land() {
        let (base, media, scope) = tree();
        assert!(scope
            .check(media.join("films/new-output.mkv").to_str().unwrap())
            .is_ok());
        assert!(scope
            .check(media.join("not-yet/deeper/out.mkv").to_str().unwrap())
            .is_ok());
        assert!(scope
            .check(base.join("private/new.mkv").to_str().unwrap())
            .is_err());
        std::fs::remove_dir_all(base).ok();
    }

    #[test]
    fn relative_paths_are_refused_and_an_unrestricted_scope_allows_anything() {
        let (base, _, scope) = tree();
        assert!(scope.check("films/a.mkv").unwrap_err().contains("absolute"));
        assert!(PathScope::default().check("films/a.mkv").is_ok());
        assert!(PathScope::default().check("/etc/passwd").is_ok());
        std::fs::remove_dir_all(base).ok();
    }

    #[test]
    fn the_command_table_finds_paths_in_nested_arrays_and_maps() {
        let args = json!({"request": {
            "root": "/m",
            "selectedFiles": ["/m/a.mkv", "/m/b.mkv"],
            "rebuildOutputDir": "/out",
            "youtubeOutputDir": null,
            "fileOutputDirs": {"/m/a.mkv": "/out/a"},
            "filePosterOverrides": {"/m/a.mkv": "/posters/a.jpg"},
            "fileTuneOverrides": {"/m/b.mkv": "none"},
            "settings": {"video": {"crf": 21}},
        }});
        let mut paths = command_paths("start_run", &args);
        paths.sort();
        assert_eq!(
            paths,
            [
                "/m",
                "/m/a.mkv",
                "/m/a.mkv",
                "/m/a.mkv",
                "/m/b.mkv",
                "/m/b.mkv",
                "/out",
                "/out/a",
                "/posters/a.jpg"
            ]
        );
        assert_eq!(
            command_paths(
                "scan_folder",
                &json!({"options": {"root": "/r", "recursive": true}})
            ),
            ["/r"]
        );
        assert!(command_paths("default_media_root", &json!({})).is_empty());
        assert!(command_paths("start_run", &json!({})).is_empty());
    }

    #[test]
    fn a_command_with_any_outside_path_is_refused_before_it_runs() {
        let (base, media, scope) = tree();
        let inside = json!({"request": {"root": media.to_str().unwrap(), "selectedFiles": [media.join("films/a.mkv").to_str().unwrap()]}});
        assert!(check_command(&scope, "start_run", &inside).is_ok());
        let one_outside = json!({"request": {"root": media.to_str().unwrap(), "selectedFiles": [base.join("private/x.mkv").to_str().unwrap()]}});
        assert!(
            check_command(&scope, "start_run", &one_outside).is_err(),
            "one bad file refuses the run"
        );
        let output_outside = json!({"request": {"root": media.to_str().unwrap(), "rebuildOutputDir": base.join("private").to_str().unwrap()}});
        assert!(check_command(&scope, "start_run", &output_outside).is_err());
        let override_key_outside = json!({"request": {"root": media.to_str().unwrap(), "fileTuneOverrides": {(base.join("private/y.mkv").to_str().unwrap()): "none"}}});
        assert!(check_command(&scope, "start_run", &override_key_outside).is_err());
        assert!(
            check_command(&scope, "default_media_root", &json!({})).is_ok(),
            "commands without paths are unaffected"
        );
        assert!(check_command(&PathScope::default(), "start_run", &one_outside).is_ok());
        assert!(
            check_command(&scope, "list_directory", &json!({"path": null})).is_ok(),
            "no path given: the server picks the default inside the root"
        );
        std::fs::remove_dir_all(base).ok();
    }
}
