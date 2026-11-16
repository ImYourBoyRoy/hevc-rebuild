//! Per-user application config, state, cache and queue locations, including AppImage portable storage.

use super::*;

pub(crate) fn state_log_directory() -> Result<PathBuf, String> {
    Ok(app_state_directory()?.join("logs"))
}

pub(crate) fn tool_setup_log_directory() -> Result<PathBuf, String> {
    Ok(state_log_directory()?.join("tool-setup"))
}

pub(crate) fn user_home_directory() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        if let Some(home) = std::env::var_os("USERPROFILE") {
            return Some(PathBuf::from(home));
        }
        if let (Some(drive), Some(path)) =
            (std::env::var_os("HOMEDRIVE"), std::env::var_os("HOMEPATH"))
        {
            return Some(PathBuf::from(format!(
                "{}{}",
                drive.to_string_lossy(),
                path.to_string_lossy()
            )));
        }
    }
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

pub(crate) fn app_config_directory() -> Result<PathBuf, String> {
    if let Some(portable) = std::env::var_os("HEVC_REBUILD_PORTABLE_DATA_DIR") {
        let path = PathBuf::from(portable);
        return Ok(if path.is_absolute() {
            path
        } else {
            std::env::current_dir()
                .map_err(|error| format!("Could not resolve the portable data directory: {error}"))?
                .join(path)
        });
    }
    #[cfg(target_os = "windows")]
    {
        let root = std::env::var_os("APPDATA")
            .or_else(|| std::env::var_os("LOCALAPPDATA"))
            .map(PathBuf::from)
            .or_else(|| user_home_directory().map(|home| home.join("AppData/Roaming")))
            .ok_or("Could not locate the current user's Windows application-data folder.")?;
        Ok(root.join("hevc-rebuild-studio"))
    }
    #[cfg(target_os = "macos")]
    {
        let home = user_home_directory()
            .ok_or("Could not locate the current user's macOS home folder.")?;
        Ok(home.join("Library/Application Support/hevc-rebuild-studio"))
    }
    #[cfg(target_os = "linux")]
    {
        let root = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| user_home_directory().map(|home| home.join(".config")))
            .ok_or("Could not locate the current user's settings folder.")?;
        Ok(root.join("hevc-rebuild-studio"))
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        user_home_directory()
            .map(|home| home.join(".hevc-rebuild-studio"))
            .ok_or_else(|| {
                "Could not locate the current user's application-data folder.".to_string()
            })
    }
}

pub(crate) fn app_state_directory() -> Result<PathBuf, String> {
    if let Some(portable) = std::env::var_os("HEVC_REBUILD_PORTABLE_DATA_DIR") {
        let path = PathBuf::from(portable);
        return Ok(if path.is_absolute() {
            path
        } else {
            std::env::current_dir()
                .map_err(|error| format!("Could not resolve the portable data directory: {error}"))?
                .join(path)
        });
    }
    #[cfg(target_os = "windows")]
    {
        let root = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .or_else(|| user_home_directory().map(|home| home.join("AppData/Local")))
            .ok_or("Could not locate the current user's Windows local-data folder.")?;
        Ok(root.join("hevc-rebuild-studio"))
    }
    #[cfg(target_os = "macos")]
    {
        let home = user_home_directory()
            .ok_or("Could not locate the current user's macOS home folder.")?;
        Ok(home.join("Library/Logs/hevc-rebuild-studio"))
    }
    #[cfg(target_os = "linux")]
    {
        let root = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| user_home_directory().map(|home| home.join(".local/state")))
            .ok_or("Could not locate the current user's state folder.")?;
        Ok(root.join("hevc-rebuild"))
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        app_config_directory()
    }
}

pub(crate) fn app_cache_directory() -> Result<PathBuf, String> {
    if let Some(portable) = std::env::var_os("HEVC_REBUILD_PORTABLE_DATA_DIR") {
        let path = PathBuf::from(portable);
        let path = if path.is_absolute() {
            path
        } else {
            std::env::current_dir()
                .map_err(|error| format!("Could not resolve the portable data directory: {error}"))?
                .join(path)
        };
        return Ok(path.join("cache"));
    }
    #[cfg(target_os = "windows")]
    {
        let root = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .or_else(|| user_home_directory().map(|home| home.join("AppData/Local")))
            .ok_or("Could not locate the current user's Windows cache folder.")?;
        Ok(root.join("hevc-rebuild-studio/cache"))
    }
    #[cfg(target_os = "macos")]
    {
        let home = user_home_directory()
            .ok_or("Could not locate the current user's macOS home folder.")?;
        Ok(home.join("Library/Caches/hevc-rebuild-studio"))
    }
    #[cfg(target_os = "linux")]
    {
        let root = std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| user_home_directory().map(|home| home.join(".cache")))
            .ok_or("Could not locate the current user's cache folder.")?;
        Ok(root.join("hevc-rebuild"))
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        app_config_directory()
    }
}

pub(crate) fn media_queue_file() -> Result<PathBuf, String> {
    let preferences = app_preferences_file()?;
    let directory = preferences
        .parent()
        .ok_or("Could not locate the application profile folder.")?;
    Ok(directory.join("library-queue.json"))
}

pub fn initialize_portable_storage() {
    if std::env::var_os("HEVC_REBUILD_PORTABLE_DATA_DIR").is_some() {
        return;
    }
    let Some(appimage) = std::env::var_os("APPIMAGE") else {
        return;
    };
    let image_path = PathBuf::from(appimage.clone())
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(appimage));
    let mut sidecar_name = image_path.into_os_string();
    sidecar_name.push(".data");
    let sidecar = PathBuf::from(sidecar_name);
    let probe = sidecar.join(format!(".write-check-{}", std::process::id()));
    let available = (|| {
        fs::create_dir_all(&sidecar)?;
        #[cfg(unix)]
        fs::set_permissions(&sidecar, fs::Permissions::from_mode(0o700))?;
        fs::write(&probe, b"portable storage check")?;
        fs::remove_file(&probe)
    })();
    match available {
        Ok(()) => std::env::set_var("HEVC_REBUILD_PORTABLE_DATA_DIR", sidecar),
        Err(error) => std::env::set_var(
            "HEVC_REBUILD_PORTABLE_STORAGE_WARNING",
            format!(
                "The AppImage sidecar folder could not be written ({error}); settings will use this user's config folder."
            ),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_directories_follow_the_portable_override() {
        let _guard = TEST_STATE_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = std::env::var_os("HEVC_REBUILD_PORTABLE_DATA_DIR");
        let root = test_dir("app-directories");
        std::env::set_var("HEVC_REBUILD_PORTABLE_DATA_DIR", &root);

        assert_eq!(app_config_directory().unwrap(), root);
        assert_eq!(app_state_directory().unwrap(), root);
        assert_eq!(state_log_directory().unwrap(), root.join("logs"));
        assert_eq!(
            tool_setup_log_directory().unwrap(),
            root.join("logs").join("tool-setup")
        );
        assert_eq!(app_preferences_file().unwrap(), root.join("settings.json"));

        match previous {
            Some(value) => std::env::set_var("HEVC_REBUILD_PORTABLE_DATA_DIR", value),
            None => std::env::remove_var("HEVC_REBUILD_PORTABLE_DATA_DIR"),
        }
        fs::remove_dir_all(root).unwrap();
    }
}
