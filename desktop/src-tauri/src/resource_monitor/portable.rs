//! Memory, swap, CPU and process-tree sampling for macOS and Windows.
//!
//! Linux reads `/proc` and cgroups directly (see the parent module). Everywhere
//! else the `sysinfo` crate supplies the same numbers, so the live memory guard
//! has real data on every supported OS instead of silently seeing `None`.
//!
//! What this does not provide, by design: kernel pressure-stall (PSI) averages
//! and swap-in rates exist only on Linux, so the guard's *swap-pressure* rule
//! (which needs both) stays Linux-only. The *memory-reserve* rule needs only
//! available memory and therefore works on all three systems.

// Linux walks /proc itself; these helpers are what the other platforms (and the
// tests, on every platform) use.
#![cfg_attr(target_os = "linux", allow(dead_code))]

use std::collections::{BTreeMap, BTreeSet, VecDeque};

const GIB: f64 = 1024.0 * 1024.0 * 1024.0;

/// One running process, reduced to the fields the tree walk needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProcessRow {
    pub(crate) pid: u32,
    pub(crate) parent: Option<u32>,
    pub(crate) name: String,
    pub(crate) rss_bytes: u64,
}

/// Resident memory of the engine process and of every ffmpeg in its tree.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct TreeMemory {
    pub(crate) engine_rss_gib: Option<f64>,
    pub(crate) ffmpeg_rss_gib: Option<f64>,
}

/// True for `ffmpeg`, `ffmpeg.exe`, `FFmpeg` and managed names such as `ffmpeg-8.1`.
pub(crate) fn is_ffmpeg_name(name: &str) -> bool {
    name.to_ascii_lowercase().contains("ffmpeg")
}

/// Walks `root` and its descendants. The engine figure is the root process
/// itself; the ffmpeg figure sums every ffmpeg in the tree (the engine may run
/// several at once) and is `None` when none is running. A pid that the table
/// repeats, a parent loop, or more than `limit` processes cannot hang the walk.
pub(crate) fn tree_memory(rows: &[ProcessRow], root: u32, limit: usize) -> TreeMemory {
    let mut children: BTreeMap<u32, Vec<&ProcessRow>> = BTreeMap::new();
    for row in rows {
        if let Some(parent) = row.parent {
            children.entry(parent).or_default().push(row);
        }
    }
    let by_pid: BTreeMap<u32, &ProcessRow> = rows.iter().map(|row| (row.pid, row)).collect();

    let mut queue = VecDeque::from([root]);
    let mut visited = BTreeSet::new();
    let mut engine = None;
    let mut ffmpeg_bytes = 0u64;
    let mut ffmpeg_found = false;
    while let Some(pid) = queue.pop_front() {
        if visited.len() >= limit || !visited.insert(pid) {
            continue;
        }
        if let Some(row) = by_pid.get(&pid) {
            if pid == root {
                engine = Some(row.rss_bytes);
            }
            if is_ffmpeg_name(&row.name) {
                ffmpeg_found = true;
                ffmpeg_bytes = ffmpeg_bytes.saturating_add(row.rss_bytes);
            }
        }
        if let Some(kids) = children.get(&pid) {
            queue.extend(kids.iter().map(|kid| kid.pid));
        }
    }
    TreeMemory {
        engine_rss_gib: engine.map(|bytes| bytes as f64 / GIB),
        ffmpeg_rss_gib: ffmpeg_found.then(|| ffmpeg_bytes as f64 / GIB),
    }
}

/// Whole-machine CPU percent between two readings of cumulative `(total, idle)`
/// time. `None` when no time passed (or the counters went backwards), so a
/// stalled clock never reads as "0% busy" or "100% busy".
// Only the Windows sampler calls this outside the tests.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn cpu_percent_between(previous: (u64, u64), current: (u64, u64)) -> Option<f64> {
    let total = current.0.checked_sub(previous.0)?;
    let idle = current.1.checked_sub(previous.1)?;
    (total > 0)
        .then(|| (total.saturating_sub(idle) as f64 / total as f64 * 100.0).clamp(0.0, 100.0))
}

#[cfg(not(target_os = "linux"))]
pub(crate) use probe::Probe;

#[cfg(not(target_os = "linux"))]
mod probe {
    use super::{ProcessRow, GIB};
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

    /// Host memory in GiB: (total, available, swap total, swap free).
    pub(crate) type HostMemory = (Option<f64>, Option<f64>, Option<f64>, Option<f64>);

    /// A long-lived `sysinfo` handle. CPU usage is a delta between two
    /// refreshes, so the handle must survive across samples.
    pub(crate) struct Probe {
        system: System,
        #[cfg(not(windows))]
        cpu_primed: bool,
        #[cfg(windows)]
        cpu_times: Option<(u64, u64)>,
    }

    impl Default for Probe {
        fn default() -> Self {
            Self {
                system: System::new(),
                #[cfg(not(windows))]
                cpu_primed: false,
                #[cfg(windows)]
                cpu_times: None,
            }
        }
    }

    /// Cumulative `(total, idle)` machine time in 100 ns units. Read straight
    /// from the kernel rather than through `sysinfo`, whose Windows CPU path
    /// needs the performance-counter service (PDH), which is slow to start and
    /// missing on some stripped-down systems and under Wine.
    #[cfg(windows)]
    fn windows_cpu_times() -> Option<(u64, u64)> {
        #[repr(C)]
        #[derive(Default, Clone, Copy)]
        struct FileTime {
            low: u32,
            high: u32,
        }
        impl FileTime {
            fn ticks(self) -> u64 {
                (u64::from(self.high) << 32) | u64::from(self.low)
            }
        }
        #[link(name = "kernel32")]
        extern "system" {
            fn GetSystemTimes(
                idle: *mut FileTime,
                kernel: *mut FileTime,
                user: *mut FileTime,
            ) -> i32;
        }
        let (mut idle, mut kernel, mut user) = (
            FileTime::default(),
            FileTime::default(),
            FileTime::default(),
        );
        // SAFETY: the three out-pointers refer to live, writable FILETIME-shaped structs.
        let ok = unsafe { GetSystemTimes(&mut idle, &mut kernel, &mut user) };
        // Kernel time already includes idle time.
        (ok != 0).then(|| (kernel.ticks().saturating_add(user.ticks()), idle.ticks()))
    }

    impl Probe {
        pub(crate) fn memory(&mut self) -> HostMemory {
            self.system.refresh_memory();
            let total = self.system.total_memory();
            if total == 0 {
                // The OS refused to say; "unknown" is honest, "0 GiB free" would
                // make the guard stop a healthy run.
                return (None, None, None, None);
            }
            (
                Some(total as f64 / GIB),
                Some(self.system.available_memory() as f64 / GIB),
                Some(self.system.total_swap() as f64 / GIB),
                Some(self.system.free_swap() as f64 / GIB),
            )
        }

        /// Whole-machine CPU percent since the previous call; `None` on the first
        /// call, which has nothing to compare against.
        pub(crate) fn cpu_percent(&mut self) -> Option<f64> {
            #[cfg(windows)]
            {
                let now = windows_cpu_times()?;
                let before = self.cpu_times.replace(now)?;
                super::cpu_percent_between(before, now)
            }
            #[cfg(not(windows))]
            {
                self.system.refresh_cpu_usage();
                let primed = std::mem::replace(&mut self.cpu_primed, true);
                primed.then(|| f64::from(self.system.global_cpu_usage()).clamp(0.0, 100.0))
            }
        }

        /// Every process with its parent and resident size.
        pub(crate) fn processes(&mut self) -> Vec<ProcessRow> {
            self.system.refresh_processes_specifics(
                ProcessesToUpdate::All,
                true,
                ProcessRefreshKind::nothing().with_memory(),
            );
            self.system
                .processes()
                .iter()
                .map(|(pid, process)| ProcessRow {
                    pid: pid.as_u32(),
                    parent: process.parent().map(sysinfo::Pid::as_u32),
                    name: process.name().to_string_lossy().into_owned(),
                    rss_bytes: process.memory(),
                })
                .collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(pid: u32, parent: Option<u32>, name: &str, rss_mib: u64) -> ProcessRow {
        ProcessRow {
            pid,
            parent,
            name: name.to_string(),
            rss_bytes: rss_mib * 1024 * 1024,
        }
    }

    #[test]
    fn the_engine_and_every_ffmpeg_below_it_are_counted_and_nothing_else() {
        let rows = [
            row(1, None, "init", 5),
            row(100, Some(1), "hevc-rebuild-studio", 512),
            row(101, Some(100), "ffmpeg", 2048),
            row(102, Some(100), "mkvmerge", 64),
            row(103, Some(102), "FFmpeg.exe", 1024),
            row(200, Some(1), "ffmpeg", 4096),
        ];
        let memory = tree_memory(&rows, 100, 256);
        assert_eq!(memory.engine_rss_gib, Some(0.5));
        assert_eq!(
            memory.ffmpeg_rss_gib,
            Some(3.0),
            "two ffmpegs in the tree; the unrelated one next to it is not ours"
        );
    }

    #[test]
    fn no_ffmpeg_means_unknown_not_zero() {
        let rows = [row(100, None, "hevc-rebuild-studio", 256)];
        let memory = tree_memory(&rows, 100, 256);
        assert_eq!(memory.engine_rss_gib, Some(0.25));
        assert_eq!(memory.ffmpeg_rss_gib, None);
    }

    #[test]
    fn a_missing_root_reports_nothing() {
        let rows = [row(1, None, "init", 5)];
        assert_eq!(tree_memory(&rows, 999, 256), TreeMemory::default());
    }

    #[test]
    fn a_parent_loop_or_duplicate_rows_cannot_hang_or_double_count() {
        let rows = [
            row(10, Some(11), "a", 100),
            row(11, Some(10), "ffmpeg", 100),
            row(11, Some(10), "ffmpeg", 100),
        ];
        let memory = tree_memory(&rows, 10, 256);
        assert_eq!(memory.engine_rss_gib, Some(100.0 / 1024.0));
        assert!(memory.ffmpeg_rss_gib.is_some());
    }

    #[test]
    fn a_huge_process_table_is_cut_off_at_the_limit() {
        let mut rows = vec![row(1, None, "engine", 1)];
        rows.extend((2..2000).map(|pid| row(pid, Some(pid - 1), "worker", 1)));
        // A 1,999-deep chain must stop at the limit rather than walk it all.
        let memory = tree_memory(&rows, 1, 8);
        assert_eq!(memory.engine_rss_gib, Some(1.0 / 1024.0));
    }

    #[test]
    fn cpu_percent_is_the_busy_share_of_elapsed_time() {
        assert_eq!(cpu_percent_between((0, 0), (1000, 750)), Some(25.0));
        assert_eq!(cpu_percent_between((1000, 900), (2000, 900)), Some(100.0));
        assert_eq!(cpu_percent_between((10, 10), (110, 110)), Some(0.0));
        assert_eq!(cpu_percent_between((5, 5), (5, 5)), None, "no time passed");
        assert_eq!(
            cpu_percent_between((100, 50), (90, 60)),
            None,
            "counters that go backwards are unknown, not a number"
        );
    }

    #[test]
    fn ffmpeg_is_recognized_across_platform_spellings() {
        for name in ["ffmpeg", "ffmpeg.exe", "FFmpeg", "ffmpeg-8.1", "FFMPEG.EXE"] {
            assert!(is_ffmpeg_name(name), "{name}");
        }
        for name in ["ffprobe", "mkvmerge", "ffm", ""] {
            assert!(!is_ffmpeg_name(name), "{name}");
        }
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn the_live_probe_reads_real_memory_cpu_and_this_process() {
        let mut probe = Probe::default();
        let (total, available, swap_total, swap_free) = probe.memory();
        let total = total.expect("this machine has memory");
        let available = available.expect("and some of it is available");
        assert!(total > 0.0 && available > 0.0 && available <= total);
        assert!(swap_total.is_some() && swap_free.is_some());

        assert_eq!(
            probe.cpu_percent(),
            None,
            "the first reading has no baseline"
        );
        std::thread::sleep(std::time::Duration::from_millis(300));
        let cpu = probe.cpu_percent().expect("the second reading has one");
        assert!((0.0..=100.0).contains(&cpu));

        let me = std::process::id();
        let rows = probe.processes();
        let tree = tree_memory(&rows, me, 256);
        assert!(
            tree.engine_rss_gib.is_some_and(|gib| gib > 0.0),
            "the test process must see its own resident memory"
        );
    }
}
