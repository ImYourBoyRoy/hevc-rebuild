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
            // A symlink that does not resolve (dangling) is a way to create files elsewhere later: refuse it.
            if std::fs::symlink_metadata(&existing).is_ok_and(|meta| meta.file_type().is_symlink())
            {
                return Err(OUTSIDE.into());
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
        let resolved = Self::resolve(Path::new(path))?;
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

fn looks_like_path(text: &str) -> bool {
    let bytes = text.as_bytes();
    text.starts_with('/')
        || text.starts_with("\\\\")
        || (bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && matches!(bytes[2], b'\\' | b'/'))
}

/// Keys that hold executables the user picked. They are never media paths and are neutralised by
/// `strip_tool_paths` where they would run, so the scan leaves them out.
const TOOL_KEYS: &[&str] = &["toolPaths", "ffmpegPath"];

fn collect_deep(value: &Value, found: &mut Vec<String>) {
    match value {
        Value::String(text) if looks_like_path(text) => found.push(text.clone()),
        Value::Array(items) => items.iter().for_each(|item| collect_deep(item, found)),
        Value::Object(map) => {
            for (key, item) in map {
                if TOOL_KEYS.contains(&key.as_str()) {
                    continue;
                }
                if looks_like_path(key) {
                    found.push(key.clone());
                }
                collect_deep(item, found);
            }
        }
        _ => {}
    }
}

/// Every absolute-looking path string or map key anywhere in the arguments, whichever command it is for. This
/// is what keeps a command that is missing from the table (or a nested setting such as a work folder) inside
/// the roots.
pub(crate) fn deep_paths(args: &Value) -> Vec<String> {
    let mut found = Vec::new();
    collect_deep(args, &mut found);
    found
}

/// Commands whose tool-path overrides would be executed (to print a version, or to run a job).
const RUNS_TOOLS: &[&str] = &[
    "inspect_environment",
    "provision_plan",
    "provision_toolchain",
    "scan_folder",
    "start_run",
];

/// Replace the override with its "none given" value, keeping the argument present so commands that require it
/// still accept the call.
fn neutralise_tool_keys(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, item) in map.iter_mut() {
                match key.as_str() {
                    "toolPaths" => *item = Value::Object(Default::default()),
                    "ffmpegPath" => *item = Value::Null,
                    _ => neutralise_tool_keys(item),
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(neutralise_tool_keys),
        _ => {}
    }
}

/// In read-only or `--root` mode a client may not choose which program runs: tool-path overrides are emptied
/// and the tools found on the server's own PATH are used. Commands that only store them are left alone.
pub(crate) fn strip_tool_paths(command: &str, args: &mut Value) {
    if RUNS_TOOLS.contains(&command) {
        neutralise_tool_keys(args);
    }
}

/// Refuse the call before it runs if any path it names, in the table or anywhere in its arguments, is
/// outside the scope.
pub(crate) fn check_command(scope: &PathScope, command: &str, args: &Value) -> Result<(), String> {
    if !scope.is_restricted() {
        return Ok(());
    }
    let mut paths = command_paths(command, args);
    paths.extend(deep_paths(args));
    for path in paths {
        if path.is_empty() {
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
    fn the_deep_scan_finds_absolute_paths_anywhere_in_the_arguments_and_ignores_the_rest() {
        let args = json!({
            "request": {
                "root": "/m",
                "settings": {"paths": {"work_root": "/scratch"}, "video": {"crf": 21}, "text": "plain words"},
                "targets": [{"directory": "/posters/a"}, {"directory": "relative/dir"}],
                "byFile": {"/m/a.mkv": {"note": "x"}},
                "link": "https://image.tmdb.org/t/p/original/abc.jpg",
                "win": ["C:\\Media\\a.mkv", "\\\\server\\share\\b.mkv", "c:/lower/c.mkv"],
                "toolPaths": {"ffmpeg": "/usr/bin/ffmpeg"},
                "ffmpegPath": "/usr/bin/ffmpeg",
            }
        });
        let mut found = deep_paths(&args);
        found.sort();
        assert_eq!(
            found,
            [
                "/m",
                "/m/a.mkv",
                "/posters/a",
                "/scratch",
                "C:\\Media\\a.mkv",
                "\\\\server\\share\\b.mkv",
                "c:/lower/c.mkv"
            ]
        );
    }

    #[test]
    fn commands_that_were_missing_from_the_table_are_still_confined_by_the_deep_scan() {
        let (base, media, scope) = tree();
        let outside = base.join("private");
        let outside = outside.to_str().unwrap();
        let inside = media.to_str().unwrap();
        let cases = [
            (
                "read_local_library_artwork_preview",
                json!({"request": {"root": outside, "mediaPath": format!("{inside}/a.mkv")}}),
            ),
            (
                "scan_local_library_assets",
                json!({"request": {"root": "/", "mediaPaths": [format!("{inside}/a.mkv")]}}),
            ),
            (
                "save_library_artwork",
                json!({"request": {"root": inside, "targets": [{"directory": outside}]}}),
            ),
            (
                "save_local_library_artwork",
                json!({"request": {"root": inside, "sourcePath": format!("{outside}/p.jpg")}}),
            ),
            (
                "preview_queue_output_targets",
                json!({"request": {"outputDir": outside}}),
            ),
            (
                "start_run",
                json!({"request": {"root": inside, "settings": {"paths": {"work_root": outside}}}}),
            ),
            (
                "start_run",
                json!({"request": {"root": inside, "fileSettingsOverrides": {(format!("{inside}/a.mkv")): {"paths": {"work_root": outside}}}}}),
            ),
        ];
        for (command, args) in cases {
            assert!(
                check_command(&scope, command, &args).is_err(),
                "{command} must be refused: {args}"
            );
        }
        let fine = json!({"request": {"root": inside, "targets": [{"directory": format!("{inside}/art")}], "settings": {"paths": {"work_root": "auto"}}}});
        assert!(check_command(&scope, "save_library_artwork", &fine).is_ok());
        std::fs::remove_dir_all(base).ok();
    }

    #[test]
    fn tool_path_overrides_do_not_trip_the_scan_because_they_are_neutralised_separately() {
        let (base, media, scope) = tree();
        let args = json!({"request": {"root": media.to_str().unwrap(), "toolPaths": {"ffmpeg": "/usr/bin/ffmpeg"}, "ffmpegPath": "/usr/bin/ffmpeg"}});
        assert!(check_command(&scope, "start_run", &args).is_ok());
        let preferences = json!({"preferences": {"toolPaths": {"ffmpeg": "/usr/bin/ffmpeg"}, "root": media.to_str().unwrap()}});
        assert!(
            check_command(&scope, "save_app_preferences", &preferences).is_ok(),
            "saving preferences keeps working"
        );
        std::fs::remove_dir_all(base).ok();
    }

    #[test]
    fn executable_overrides_are_emptied_only_where_they_would_run_something() {
        let mut args = json!({"toolPaths": {"ffmpeg": "/bin/sh"}, "request": {"toolPaths": {"ffprobe": "/bin/sh"}, "ffmpegPath": "/bin/sh", "root": "/m"}});
        strip_tool_paths("start_run", &mut args);
        assert_eq!(
            args,
            json!({"toolPaths": {}, "request": {"toolPaths": {}, "ffmpegPath": null, "root": "/m"}}),
            "overrides are emptied, the arguments stay present, nothing else changes"
        );
        // Saving preferences stores the user's tool paths; it never runs them, so they are left alone.
        let mut preferences = json!({"preferences": {"toolPaths": {"ffmpeg": "/usr/bin/ffmpeg"}}});
        strip_tool_paths("save_app_preferences", &mut preferences);
        assert_eq!(
            preferences["preferences"]["toolPaths"]["ffmpeg"],
            "/usr/bin/ffmpeg"
        );
        for command in [
            "inspect_environment",
            "provision_plan",
            "provision_toolchain",
            "scan_folder",
        ] {
            let mut value = json!({"toolPaths": {"ffmpeg": "/bin/sh"}, "options": {"toolPaths": {"ffmpeg": "/bin/sh"}}});
            strip_tool_paths(command, &mut value);
            assert!(
                value["toolPaths"].as_object().unwrap().is_empty()
                    && value["options"]["toolPaths"]
                        .as_object()
                        .unwrap()
                        .is_empty(),
                "{command}"
            );
        }
    }

    #[test]
    fn a_path_with_surrounding_whitespace_is_judged_as_written_not_trimmed() {
        let (base, media, scope) = tree();
        let padded = format!(" {}", media.display());
        assert!(
            scope.check(&padded).is_err(),
            "the command would run on the untrimmed string"
        );
        std::fs::remove_dir_all(base).ok();
    }

    #[cfg(unix)]
    #[test]
    fn a_dangling_symlink_that_points_outside_is_refused() {
        let (base, media, scope) = tree();
        std::os::unix::fs::symlink(base.join("private/will-be-created"), media.join("dangling"))
            .unwrap();
        assert!(scope
            .check(media.join("dangling").to_str().unwrap())
            .is_err());
        assert!(scope
            .check(media.join("dangling/inside.mkv").to_str().unwrap())
            .is_err());
        std::fs::remove_dir_all(base).ok();
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
