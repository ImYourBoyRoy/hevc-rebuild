//! Live resource observations for native desktop and MCP media-run jobs.
//!
//! This module reports system and process-tree metrics while the compatibility
//! engine runs. It does not replace the engine's memory/disk guards or retries.

use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, VecDeque},
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
const MAX_PROCESS_TREE: usize = 256;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) struct RuntimeResourceSnapshot {
    pub(crate) sampled_at_unix_ms: u128,
    pub(crate) cpu_percent: Option<f64>,
    pub(crate) load1: Option<f64>,
    pub(crate) memory_total_gib: Option<f64>,
    pub(crate) memory_available_gib: Option<f64>,
    pub(crate) swap_total_gib: Option<f64>,
    pub(crate) swap_free_gib: Option<f64>,
    pub(crate) swap_used_gib: Option<f64>,
    pub(crate) swap_used_percent: Option<f64>,
    pub(crate) swap_in_bytes_per_second: Option<f64>,
    pub(crate) swap_out_bytes_per_second: Option<f64>,
    pub(crate) memory_some_avg10: Option<f64>,
    pub(crate) memory_full_avg10: Option<f64>,
    pub(crate) io_some_avg10: Option<f64>,
    pub(crate) io_full_avg10: Option<f64>,
    pub(crate) cpu_some_avg10: Option<f64>,
    pub(crate) disk_total_bytes: Option<u64>,
    pub(crate) disk_used_bytes: Option<u64>,
    pub(crate) disk_free_bytes: Option<u64>,
    pub(crate) disk_free_percent: Option<f64>,
    pub(crate) engine_process_rss_gib: Option<f64>,
    pub(crate) engine_process_swap_gib: Option<f64>,
    pub(crate) ffmpeg_process_rss_gib: Option<f64>,
    pub(crate) ffmpeg_process_swap_gib: Option<f64>,
    pub(crate) temperature_c: Option<f64>,
}

pub(crate) struct MonitorHandle {
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl MonitorHandle {
    pub(crate) fn stop(mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            worker.thread().unpark();
            let _ = worker.join();
        }
    }
}

impl Drop for MonitorHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            worker.thread().unpark();
            let _ = worker.join();
        }
    }
}

pub(crate) fn monitor_process<F>(
    pid: u32,
    disk_path: Option<PathBuf>,
    interval: Duration,
    mut on_snapshot: F,
) -> MonitorHandle
where
    F: FnMut(RuntimeResourceSnapshot) + Send + 'static,
{
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    let worker = thread::spawn(move || {
        let mut sampler = Sampler::new();
        while !thread_stop.load(Ordering::Acquire) {
            on_snapshot(sampler.sample(pid, disk_path.as_deref()));
            thread::park_timeout(interval.max(Duration::from_millis(250)));
        }
    });
    MonitorHandle {
        stop,
        worker: Some(worker),
    }
}

pub(crate) fn sample_current_host() -> RuntimeResourceSnapshot {
    Sampler::new().sample(std::process::id(), None)
}

#[derive(Default)]
pub(crate) struct Sampler {
    previous_cpu: Option<(u64, u64)>,
    previous_swap: Option<(u64, u64)>,
    previous_sample: Option<Instant>,
    page_size: u64,
}

impl Sampler {
    pub(crate) fn new() -> Self {
        Self {
            page_size: page_size(),
            ..Self::default()
        }
    }

    pub(crate) fn sample(&mut self, pid: u32, disk_path: Option<&Path>) -> RuntimeResourceSnapshot {
        let now = Instant::now();
        let elapsed = self
            .previous_sample
            .replace(now)
            .map(|previous| now.duration_since(previous));
        let cpu_percent = sample_cpu(&mut self.previous_cpu);
        let (swap_in_bytes_per_second, swap_out_bytes_per_second) =
            sample_swap(&mut self.previous_swap, elapsed, self.page_size);
        let (memory_total_gib, memory_available_gib, swap_total_gib, swap_free_gib) =
            memory_stats();
        let swap_used_gib = swap_total_gib
            .zip(swap_free_gib)
            .map(|(total, free)| (total - free).max(0.0));
        let swap_used_percent = swap_used_gib.zip(swap_total_gib).map(|(used, total)| {
            if total > 0.0 {
                (used / total * 100.0).clamp(0.0, 100.0)
            } else {
                0.0
            }
        });
        let (
            engine_process_rss_gib,
            engine_process_swap_gib,
            ffmpeg_process_rss_gib,
            ffmpeg_process_swap_gib,
        ) = process_tree_memory(pid);
        let pressure_memory = read_psi("memory");
        let pressure_io = read_psi("io");
        let pressure_cpu = read_psi("cpu");
        let disk = disk_path.and_then(disk_stats);
        let disk_free_percent = disk
            .and_then(|(total, _, free)| (total > 0).then(|| free as f64 / total as f64 * 100.0));

        RuntimeResourceSnapshot {
            sampled_at_unix_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis(),
            cpu_percent,
            load1: fs::read_to_string("/proc/loadavg")
                .ok()
                .and_then(|text| text.split_whitespace().next()?.parse().ok()),
            memory_total_gib,
            memory_available_gib,
            swap_total_gib,
            swap_free_gib,
            swap_used_gib,
            swap_used_percent,
            swap_in_bytes_per_second,
            swap_out_bytes_per_second,
            memory_some_avg10: pressure_memory.get("some").copied(),
            memory_full_avg10: pressure_memory.get("full").copied(),
            io_some_avg10: pressure_io.get("some").copied(),
            io_full_avg10: pressure_io.get("full").copied(),
            cpu_some_avg10: pressure_cpu.get("some").copied(),
            disk_total_bytes: disk.map(|(total, _, _)| total),
            disk_used_bytes: disk.map(|(_, used, _)| used),
            disk_free_bytes: disk.map(|(_, _, free)| free),
            disk_free_percent,
            engine_process_rss_gib,
            engine_process_swap_gib,
            ffmpeg_process_rss_gib,
            ffmpeg_process_swap_gib,
            temperature_c: temperature_c(),
        }
    }
}

fn memory_stats() -> (Option<f64>, Option<f64>, Option<f64>, Option<f64>) {
    #[cfg(target_os = "linux")]
    {
        let Ok(meminfo) = fs::read_to_string("/proc/meminfo") else {
            return (None, None, None, None);
        };
        let values = parse_meminfo(&meminfo);
        let host_total = values
            .get("MemTotal")
            .copied()
            .map(|kib| kib as f64 / 1024.0 / 1024.0);
        let host_available = values
            .get("MemAvailable")
            .copied()
            .map(|kib| kib as f64 / 1024.0 / 1024.0);
        let (cgroup_total, cgroup_available) = cgroup_memory_gib();
        let total = min_available(host_total, cgroup_total);
        let available = min_available(host_available, cgroup_available);
        let swap_total = values
            .get("SwapTotal")
            .copied()
            .map(|kib| kib as f64 / 1024.0 / 1024.0);
        let swap_free = values
            .get("SwapFree")
            .copied()
            .map(|kib| kib as f64 / 1024.0 / 1024.0);
        (total, available, swap_total, swap_free)
    }
    #[cfg(not(target_os = "linux"))]
    {
        (None, None, None, None)
    }
}

fn min_available(left: Option<f64>, right: Option<f64>) -> Option<f64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

fn parse_meminfo(input: &str) -> BTreeMap<String, u64> {
    input
        .lines()
        .filter_map(|line| {
            let (key, value) = line.split_once(':')?;
            let value = value.split_whitespace().next()?.parse().ok()?;
            Some((key.to_string(), value))
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn cgroup_memory_gib() -> (Option<f64>, Option<f64>) {
    let Ok(cgroup) = fs::read_to_string("/proc/self/cgroup") else {
        return (None, None);
    };
    let Some(relative) = cgroup.lines().find_map(|line| line.strip_prefix("0::")) else {
        return (None, None);
    };
    let base = Path::new("/sys/fs/cgroup").join(relative.trim_start_matches('/'));
    let Ok(limit_text) = fs::read_to_string(base.join("memory.max")) else {
        return (None, None);
    };
    let limit_text = limit_text.trim();
    if limit_text == "max" {
        return (None, None);
    }
    let Ok(limit) = limit_text.parse::<u64>() else {
        return (None, None);
    };
    let Ok(current) = fs::read_to_string(base.join("memory.current")) else {
        return (None, None);
    };
    let Ok(current) = current.trim().parse::<u64>() else {
        return (None, None);
    };
    (
        Some(limit as f64 / GIB),
        Some(limit.saturating_sub(current) as f64 / GIB),
    )
}

#[cfg(not(target_os = "linux"))]
fn cgroup_memory_gib() -> (Option<f64>, Option<f64>) {
    (None, None)
}

#[cfg(target_os = "linux")]
fn sample_cpu(previous: &mut Option<(u64, u64)>) -> Option<f64> {
    let input = fs::read_to_string("/proc/stat").ok()?;
    let current = parse_cpu_counters(input.lines().next()?);
    let old = previous.replace(current)?;
    let total = current.0.saturating_sub(old.0);
    let idle = current.1.saturating_sub(old.1);
    (total > 0)
        .then(|| (total.saturating_sub(idle) as f64 / total as f64 * 100.0).clamp(0.0, 100.0))
}

#[cfg(not(target_os = "linux"))]
fn sample_cpu(_previous: &mut Option<(u64, u64)>) -> Option<f64> {
    None
}

#[cfg(target_os = "linux")]
fn parse_cpu_counters(line: &str) -> (u64, u64) {
    let mut values = line
        .split_whitespace()
        .skip(1)
        .filter_map(|value| value.parse::<u64>().ok());
    let values = values.by_ref().collect::<Vec<_>>();
    let idle = values.get(3).copied().unwrap_or(0) + values.get(4).copied().unwrap_or(0);
    (values.iter().sum(), idle)
}

#[cfg(target_os = "linux")]
fn sample_swap(
    previous: &mut Option<(u64, u64)>,
    elapsed: Option<Duration>,
    page_size: u64,
) -> (Option<f64>, Option<f64>) {
    let Ok(vmstat) = fs::read_to_string("/proc/vmstat") else {
        return (None, None);
    };
    let values = parse_vmstat(&vmstat);
    let (Some(&pages_in), Some(&pages_out)) = (values.get("pswpin"), values.get("pswpout")) else {
        return (None, None);
    };
    let current = (pages_in, pages_out);
    let old = previous.replace(current);
    if page_size == 0 {
        return (None, None);
    }
    let elapsed = elapsed.filter(|elapsed| !elapsed.is_zero());
    let Some((old_in, old_out)) = old else {
        return (None, None);
    };
    let Some(elapsed) = elapsed else {
        return (None, None);
    };
    let seconds = elapsed.as_secs_f64();
    (
        Some(current.0.saturating_sub(old_in) as f64 * page_size as f64 / seconds),
        Some(current.1.saturating_sub(old_out) as f64 * page_size as f64 / seconds),
    )
}

#[cfg(not(target_os = "linux"))]
fn sample_swap(
    _previous: &mut Option<(u64, u64)>,
    _elapsed: Option<Duration>,
    _page_size: u64,
) -> (Option<f64>, Option<f64>) {
    (None, None)
}

#[cfg(target_os = "linux")]
fn parse_vmstat(input: &str) -> BTreeMap<String, u64> {
    input
        .lines()
        .filter_map(|line| {
            let (key, value) = line.split_once(char::is_whitespace)?;
            if !matches!(key, "pswpin" | "pswpout") {
                return None;
            }
            Some((key.to_string(), value.trim().parse().ok()?))
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn read_psi(resource: &str) -> BTreeMap<String, f64> {
    fs::read_to_string(format!("/proc/pressure/{resource}"))
        .ok()
        .map(|input| parse_psi(&input))
        .unwrap_or_default()
}

#[cfg(not(target_os = "linux"))]
fn read_psi(_resource: &str) -> BTreeMap<String, f64> {
    BTreeMap::new()
}

fn parse_psi(input: &str) -> BTreeMap<String, f64> {
    input
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let kind = fields.next()?;
            let avg10 = fields
                .find_map(|field| field.strip_prefix("avg10="))?
                .parse()
                .ok()?;
            Some((kind.to_string(), avg10))
        })
        .collect()
}

fn process_tree_memory(root: u32) -> (Option<f64>, Option<f64>, Option<f64>, Option<f64>) {
    #[cfg(target_os = "linux")]
    {
        let mut queue = VecDeque::from([root]);
        let mut visited = std::collections::BTreeSet::new();
        let mut engine = None;
        let mut ffmpeg_rss = 0.0;
        let mut ffmpeg_swap = 0.0;
        let mut ffmpeg_found = false;
        while let Some(pid) = queue.pop_front() {
            if visited.len() >= MAX_PROCESS_TREE || !visited.insert(pid) {
                continue;
            }
            let process_path = PathBuf::from(format!("/proc/{pid}"));
            if process_path.exists() {
                let (rss, swap) = process_memory(pid);
                if pid == root {
                    engine = rss.zip(swap);
                }
                if is_ffmpeg_process(pid) {
                    ffmpeg_found = true;
                    ffmpeg_rss += rss.unwrap_or(0.0);
                    ffmpeg_swap += swap.unwrap_or(0.0);
                }
                if let Ok(children) = fs::read_to_string(
                    process_path
                        .join("task")
                        .join(pid.to_string())
                        .join("children"),
                ) {
                    queue.extend(
                        children
                            .split_whitespace()
                            .filter_map(|value| value.parse::<u32>().ok()),
                    );
                }
            }
        }
        let (engine_rss, engine_swap) =
            engine.map_or((None, None), |(rss, swap)| (Some(rss), Some(swap)));
        (
            engine_rss,
            engine_swap,
            ffmpeg_found.then_some(ffmpeg_rss),
            ffmpeg_found.then_some(ffmpeg_swap),
        )
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = root;
        (None, None, None, None)
    }
}

#[cfg(target_os = "linux")]
fn process_memory(pid: u32) -> (Option<f64>, Option<f64>) {
    let Ok(status) = fs::read_to_string(format!("/proc/{pid}/status")) else {
        return (None, None);
    };
    let mut rss = None;
    let mut swap = None;
    for line in status.lines() {
        let target = if line.starts_with("VmRSS:") {
            &mut rss
        } else if line.starts_with("VmSwap:") {
            &mut swap
        } else {
            continue;
        };
        *target = line
            .split_whitespace()
            .nth(1)
            .and_then(|value| value.parse::<u64>().ok())
            .map(|kib| kib as f64 / 1024.0 / 1024.0);
    }
    (rss, swap)
}

#[cfg(target_os = "linux")]
fn is_ffmpeg_process(pid: u32) -> bool {
    fs::read(format!("/proc/{pid}/cmdline"))
        .ok()
        .and_then(|cmdline| cmdline.split(|byte| *byte == 0).next().map(Vec::from))
        .is_some_and(|program| {
            let program = String::from_utf8_lossy(&program);
            Path::new(program.as_ref()).file_name().is_some_and(|name| {
                name.to_string_lossy()
                    .to_ascii_lowercase()
                    .contains("ffmpeg")
            })
        })
}

#[cfg(target_os = "linux")]
fn disk_stats(path: &Path) -> Option<(u64, u64, u64)> {
    use std::os::unix::ffi::OsStrExt;
    for existing in path
        .ancestors()
        .filter(|ancestor| !ancestor.as_os_str().is_empty())
    {
        let Ok(path) = std::ffi::CString::new(existing.as_os_str().as_bytes()) else {
            continue;
        };
        let mut stats = std::mem::MaybeUninit::<libc::statvfs>::zeroed();
        // SAFETY: the C path is NUL-terminated and stats points to writable memory.
        let code = unsafe { libc::statvfs(path.as_ptr(), stats.as_mut_ptr()) };
        if code != 0 {
            continue;
        }
        // SAFETY: statvfs initialized the structure on success.
        let stats = unsafe { stats.assume_init() };
        let block_size = stats.f_frsize as u64;
        let total = (stats.f_blocks as u64).saturating_mul(block_size);
        let used = (stats.f_blocks as u64)
            .saturating_sub(stats.f_bfree as u64)
            .saturating_mul(block_size);
        let free = (stats.f_bavail as u64).saturating_mul(block_size);
        return Some((total, used, free));
    }
    None
}

#[cfg(not(target_os = "linux"))]
fn disk_stats(_path: &Path) -> Option<(u64, u64, u64)> {
    None
}

#[cfg(target_os = "linux")]
fn temperature_c() -> Option<f64> {
    let mut temperatures = Vec::new();
    for root in [
        Path::new("/sys/class/thermal"),
        Path::new("/sys/class/hwmon"),
    ] {
        let Ok(entries) = fs::read_dir(root) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(children) = fs::read_dir(entry.path()) else {
                continue;
            };
            for child in children.flatten() {
                let name = child.file_name().to_string_lossy().to_string();
                let thermal_temp = root.ends_with("thermal") && name == "temp";
                let hwmon_temp =
                    root.ends_with("hwmon") && name.starts_with("temp") && name.ends_with("_input");
                if !thermal_temp && !hwmon_temp {
                    continue;
                }
                let Ok(raw) = fs::read_to_string(child.path()) else {
                    continue;
                };
                let Ok(raw) = raw.trim().parse::<f64>() else {
                    continue;
                };
                let celsius = if raw > 1000.0 { raw / 1000.0 } else { raw };
                if (10.0..=125.0).contains(&celsius) {
                    temperatures.push(celsius);
                }
            }
        }
    }
    temperatures.into_iter().reduce(f64::max)
}

#[cfg(not(target_os = "linux"))]
fn temperature_c() -> Option<f64> {
    None
}

#[cfg(target_os = "linux")]
fn page_size() -> u64 {
    // SAFETY: sysconf has no pointer arguments and returns the configured page size.
    let result = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    u64::try_from(result)
        .ok()
        .filter(|value| *value > 0)
        .unwrap_or(0)
}

#[cfg(not(target_os = "linux"))]
fn page_size() -> u64 {
    0
}
