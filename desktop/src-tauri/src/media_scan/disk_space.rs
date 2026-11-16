//! Free and total space of the volume holding a path, on Linux, macOS and Windows.
//!
//! Every disk guard (rebuild preflight, live projection, batch planning) reads
//! space through this one function, so a platform gap cannot leave one guard
//! working and another blind. Unix uses `statvfs`; Windows uses
//! `GetDiskFreeSpaceExW`. A path that does not exist yet (for example an output
//! folder created later) is measured at its nearest existing ancestor.
//!
//! Notes: `available_bytes` is what this user can actually write (it honors
//! quotas and root-reserved blocks), not raw free space. On macOS `statvfs`
//! reports 32-bit block counts, so volumes above 16 TiB can read low.

use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DiskSpace {
    pub total_bytes: u64,
    pub used_bytes: u64,
    /// Space this user can still write.
    pub available_bytes: u64,
}

/// Space on the volume that holds `path`, or `None` when it cannot be read.
pub(crate) fn disk_space(path: &Path) -> Option<DiskSpace> {
    nearest_existing(path).and_then(|existing| platform_space(&existing))
}

/// The closest existing directory at or above `path`. A file resolves to its
/// folder: Windows' `GetDiskFreeSpaceExW` accepts only directories, and on Unix
/// the volume is the same either way.
fn nearest_existing(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .filter(|ancestor| !ancestor.as_os_str().is_empty())
        .find(|ancestor| ancestor.is_dir())
        .map(Path::to_path_buf)
}

/// statvfs field widths differ by OS (32-bit block counts on macOS, 64-bit on
/// Linux), so the conversion has to stay generic to compile cleanly on both.
#[cfg(unix)]
fn widen(value: impl TryInto<u64>) -> Option<u64> {
    value.try_into().ok()
}

#[cfg(unix)]
fn platform_space(path: &Path) -> Option<DiskSpace> {
    use std::os::unix::ffi::OsStrExt;
    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut stats = std::mem::MaybeUninit::<libc::statvfs>::zeroed();
    // SAFETY: c_path is NUL-terminated and stats points to writable storage.
    let code = unsafe { libc::statvfs(c_path.as_ptr(), stats.as_mut_ptr()) };
    if code != 0 {
        return None;
    }
    // SAFETY: statvfs initialized the structure on success.
    let stats = unsafe { stats.assume_init() };
    let block_size = widen(stats.f_frsize)?;
    let blocks = widen(stats.f_blocks)?;
    let free_blocks = widen(stats.f_bfree)?;
    let available_blocks = widen(stats.f_bavail)?;
    Some(DiskSpace {
        total_bytes: blocks.saturating_mul(block_size),
        used_bytes: blocks
            .saturating_sub(free_blocks)
            .saturating_mul(block_size),
        available_bytes: available_blocks.saturating_mul(block_size),
    })
}

#[cfg(windows)]
fn platform_space(path: &Path) -> Option<DiskSpace> {
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "kernel32")]
    extern "system" {
        fn GetDiskFreeSpaceExW(
            directory_name: *const u16,
            free_bytes_available_to_caller: *mut u64,
            total_number_of_bytes: *mut u64,
            total_number_of_free_bytes: *mut u64,
        ) -> i32;
    }

    let name = windows_directory_name(&path.to_string_lossy());
    let wide: Vec<u16> = std::ffi::OsStr::new(&name)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let (mut available, mut total, mut free) = (0u64, 0u64, 0u64);
    // SAFETY: `wide` is NUL-terminated and the three out-pointers refer to live u64s.
    let ok = unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut available, &mut total, &mut free) };
    if ok == 0 {
        return None;
    }
    Some(DiskSpace {
        total_bytes: total,
        used_bytes: total.saturating_sub(free),
        available_bytes: available,
    })
}

#[cfg(not(any(unix, windows)))]
fn platform_space(_path: &Path) -> Option<DiskSpace> {
    None
}

/// The directory name `GetDiskFreeSpaceExW` is documented to accept: no `\\?\`
/// verbatim prefix (canonicalized paths carry one), and a trailing backslash so
/// UNC shares resolve. Pure string work, so it is tested on every platform.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn windows_directory_name(raw: &str) -> String {
    let mut name = if let Some(unc) = raw.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc}")
    } else if let Some(rest) = raw.strip_prefix(r"\\?\") {
        rest.to_owned()
    } else {
        raw.to_owned()
    };
    if !name.ends_with('\\') {
        name.push('\\');
    }
    name
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_temp_volume_reports_sane_numbers_on_every_platform() {
        let space = disk_space(&std::env::temp_dir()).expect("read the temp volume");
        assert!(space.total_bytes > 0);
        assert!(space.available_bytes > 0, "a usable temp volume has room");
        assert!(space.available_bytes <= space.total_bytes);
        assert!(space.used_bytes <= space.total_bytes);
    }

    #[test]
    fn a_file_or_a_folder_that_does_not_exist_yet_is_measured_where_it_will_live() {
        let dir = std::env::temp_dir().join(format!("hevc-disk-space-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("clip.mkv");
        std::fs::write(&file, b"x").unwrap();
        let folder = disk_space(&dir).expect("existing folder");
        let beside_file = disk_space(&file).expect("an existing file measures its volume");
        let future = disk_space(&dir.join("not").join("created").join("yet"))
            .expect("a future output folder measures its nearest existing ancestor");
        assert_eq!(folder.total_bytes, beside_file.total_bytes);
        assert_eq!(folder.total_bytes, future.total_bytes);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn nothing_is_reported_for_a_path_with_no_existing_ancestor() {
        // A relative path with no existing component falls back to nothing rather
        // than to a guess that could disable a guard by being too generous.
        assert!(nearest_existing(Path::new("")).is_none());
    }

    #[test]
    fn windows_paths_lose_the_verbatim_prefix_and_gain_a_trailing_backslash() {
        assert_eq!(
            windows_directory_name(r"\\?\C:\Media\Movies"),
            r"C:\Media\Movies\"
        );
        assert_eq!(windows_directory_name(r"D:\Library\"), r"D:\Library\");
        assert_eq!(windows_directory_name(r"E:"), r"E:\");
        assert_eq!(
            windows_directory_name(r"\\?\UNC\nas\media\films"),
            r"\\nas\media\films\"
        );
        assert_eq!(
            windows_directory_name(r"\\nas\media"),
            r"\\nas\media\",
            "plain UNC shares need the trailing backslash"
        );
    }
}
