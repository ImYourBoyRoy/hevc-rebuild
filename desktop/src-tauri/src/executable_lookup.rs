//! PATH lookup used to decide whether a helper tool can be launched.

use super::*;

/// Directories where package managers put tools but a desktop-launched app's PATH often omits
/// them: a macOS app started from Finder sees only `/usr/bin:/bin:/usr/sbin:/sbin`, so Homebrew's
/// `/opt/homebrew/bin` (Apple silicon) and `/usr/local/bin` (Intel) would otherwise look empty, and a
/// Windows process started before `winget` installed FFmpeg never sees the new `WinGet\Links`.
pub(crate) fn package_manager_directories(
    os: &str,
    home: Option<&Path>,
    local_app_data: Option<&Path>,
) -> Vec<PathBuf> {
    match os {
        "macos" => vec![
            PathBuf::from("/opt/homebrew/bin"),
            PathBuf::from("/opt/homebrew/sbin"),
            PathBuf::from("/usr/local/bin"),
            PathBuf::from("/usr/local/sbin"),
        ],
        "windows" => local_app_data
            .map(|root| vec![root.join("Microsoft").join("WinGet").join("Links")])
            .unwrap_or_default(),
        "linux" => home
            .map(|home| vec![home.join(".local").join("bin")])
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// The process PATH followed by the package-manager directories above (without duplicates).
pub(crate) fn search_directories() -> Vec<PathBuf> {
    let mut directories = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .unwrap_or_default();
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    for extra in
        package_manager_directories(std::env::consts::OS, home.as_deref(), local.as_deref())
    {
        if !directories.contains(&extra) {
            directories.push(extra);
        }
    }
    directories
}

pub(crate) fn executable_in_path(program: &str) -> bool {
    search_directories().into_iter().any(|directory| {
        let candidates = {
            #[cfg(target_os = "windows")]
            {
                let mut candidates = vec![directory.join(program)];
                if Path::new(program).extension().is_none() {
                    if let Some(extensions) = std::env::var_os("PATHEXT") {
                        candidates.extend(
                            extensions
                                .to_string_lossy()
                                .split(';')
                                .filter(|extension| !extension.is_empty())
                                .map(|extension| directory.join(format!("{program}{extension}"))),
                        );
                    }
                }
                candidates
            }
            #[cfg(not(target_os = "windows"))]
            {
                vec![directory.join(program)]
            }
        };
        candidates.into_iter().any(|candidate| {
            let Ok(metadata) = candidate.metadata() else {
                return false;
            };
            if !metadata.is_file() {
                return false;
            }
            #[cfg(unix)]
            {
                metadata.permissions().mode() & 0o111 != 0
            }
            #[cfg(not(unix))]
            {
                true
            }
        })
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn executable_in_path_finds_system_tools_and_rejects_missing_names() {
        assert!(executable_in_path("sh"));
        assert!(!executable_in_path("hevc-rebuild-no-such-tool-0f3a"));
    }
}

#[cfg(test)]
mod directory_tests {
    use super::*;

    #[test]
    fn a_finder_launched_mac_app_still_searches_homebrew() {
        let dirs = package_manager_directories("macos", Some(Path::new("/Users/a")), None);
        assert!(
            dirs.contains(&PathBuf::from("/opt/homebrew/bin")),
            "Apple silicon Homebrew"
        );
        assert!(
            dirs.contains(&PathBuf::from("/usr/local/bin")),
            "Intel Homebrew"
        );
    }

    #[test]
    fn windows_searches_the_winget_links_folder_for_tools_installed_after_launch() {
        let dirs = package_manager_directories(
            "windows",
            None,
            Some(Path::new("C:/Users/a/AppData/Local")),
        );
        assert_eq!(
            dirs,
            vec![PathBuf::from(
                "C:/Users/a/AppData/Local/Microsoft/WinGet/Links"
            )]
        );
        assert!(package_manager_directories("windows", None, None).is_empty());
    }

    #[test]
    fn linux_and_unknown_hosts_add_only_the_users_local_bin() {
        assert_eq!(
            package_manager_directories("linux", Some(Path::new("/home/a")), None),
            vec![PathBuf::from("/home/a/.local/bin")]
        );
        assert!(
            package_manager_directories("freebsd", Some(Path::new("/home/a")), None).is_empty()
        );
    }
}
