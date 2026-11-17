//! PATH lookup used to decide whether a helper tool can be launched.

use super::*;

pub(crate) fn executable_in_path(program: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|directory| {
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
