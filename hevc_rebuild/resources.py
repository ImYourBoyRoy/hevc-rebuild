from __future__ import annotations

import os
import shutil
import time
from dataclasses import dataclass
from pathlib import Path


GIB = 1024 ** 3


@dataclass(frozen=True)
class MemoryStats:
    total_gib: float | None
    available_gib: float | None
    swap_total_gib: float | None
    swap_free_gib: float | None
    swap_used_gib: float | None = None
    swap_used_percent: float | None = None


@dataclass(frozen=True)
class DiskStats:
    total_bytes: int
    used_bytes: int
    free_bytes: int

    @property
    def free_gib(self) -> float:
        return self.free_bytes / GIB

    @property
    def free_percent(self) -> float:
        return (self.free_bytes / self.total_bytes * 100.0) if self.total_bytes else 0.0


@dataclass(frozen=True)
class PressureStats:
    cpu_some_avg10: float | None = None
    memory_some_avg10: float | None = None
    memory_full_avg10: float | None = None
    io_some_avg10: float | None = None
    io_full_avg10: float | None = None


@dataclass(frozen=True)
class ResourceSnapshot:
    cpu_percent: float | None
    load1: float | None
    memory: MemoryStats
    disk: DiskStats | None
    pressure: PressureStats
    temperature_c: float | None
    process_rss_gib: float | None
    swap_activity: "SwapActivityStats | None" = None
    process_swap_gib: float | None = None


@dataclass(frozen=True)
class SwapActivityStats:
    """System-wide swap-in/out rates sampled from Linux /proc/vmstat."""

    in_bytes_per_second: float | None = None
    out_bytes_per_second: float | None = None


class SwapSampler:
    """Best-effort swap traffic sampler; allocated swap alone is not pressure."""

    def __init__(self) -> None:
        try:
            self._page_size = int(os.sysconf("SC_PAGE_SIZE"))
        except (AttributeError, OSError, ValueError):
            self._page_size = 0
        self._previous = self._read()
        self._sampled_at = time.monotonic()

    @staticmethod
    def _read() -> tuple[int, int] | None:
        try:
            values: dict[str, int] = {}
            for line in Path("/proc/vmstat").read_text(encoding="utf-8").splitlines():
                key, _, raw = line.partition(" ")
                if key in {"pswpin", "pswpout"}:
                    values[key] = int(raw.strip())
            if "pswpin" not in values or "pswpout" not in values:
                return None
            return values["pswpin"], values["pswpout"]
        except (OSError, ValueError):
            return None

    def sample(self) -> SwapActivityStats:
        current = self._read()
        sampled_at = time.monotonic()
        previous = self._previous
        elapsed = sampled_at - self._sampled_at
        self._previous = current
        self._sampled_at = sampled_at
        if current is None or previous is None or elapsed <= 0 or self._page_size <= 0:
            return SwapActivityStats()
        swap_in_pages = max(0, current[0] - previous[0])
        swap_out_pages = max(0, current[1] - previous[1])
        return SwapActivityStats(
            in_bytes_per_second=swap_in_pages * self._page_size / elapsed,
            out_bytes_per_second=swap_out_pages * self._page_size / elapsed,
        )


class CpuSampler:
    """Best-effort aggregate Linux CPU utilization sampler using /proc/stat."""

    def __init__(self) -> None:
        self._previous: tuple[int, int] | None = self._read()

    @staticmethod
    def _read() -> tuple[int, int] | None:
        try:
            line = Path('/proc/stat').read_text(encoding='utf-8').splitlines()[0]
            parts = line.split()
            if not parts or parts[0] != 'cpu':
                return None
            vals = [int(v) for v in parts[1:]]
            if len(vals) < 4:
                return None
            idle = vals[3] + (vals[4] if len(vals) > 4 else 0)
            total = sum(vals)
            return total, idle
        except (OSError, ValueError, IndexError):
            return None

    def sample(self) -> float | None:
        current = self._read()
        previous = self._previous
        self._previous = current
        if current is None or previous is None:
            return None
        total_delta = current[0] - previous[0]
        idle_delta = current[1] - previous[1]
        if total_delta <= 0:
            return None
        busy = max(0, total_delta - max(0, idle_delta))
        return min(100.0, max(0.0, busy / total_delta * 100.0))


def _meminfo_values() -> dict[str, int]:
    values: dict[str, int] = {}
    try:
        for line in Path('/proc/meminfo').read_text(encoding='utf-8').splitlines():
            if ':' not in line:
                continue
            key, rest = line.split(':', 1)
            token = rest.strip().split()[0]
            if token.isdigit():
                values[key] = int(token)  # kB
    except OSError:
        pass
    return values


def _cgroup_memory_gib() -> tuple[float | None, float | None]:
    """Finite cgroup-v2 memory limit and remaining allowance, when applicable."""
    try:
        line = next(
            (line for line in Path('/proc/self/cgroup').read_text(encoding='utf-8').splitlines() if '::' in line),
            None,
        )
        if line is None:
            return None, None
        cg = line.split('::', 1)[1]
        base = Path('/sys/fs/cgroup') / cg.lstrip('/')
        maximum = (base / 'memory.max').read_text(encoding='utf-8').strip()
        if maximum == 'max':
            return None, None
        limit = int(maximum)
        current = int((base / 'memory.current').read_text(encoding='utf-8').strip())
        return limit / GIB, max(0, limit - current) / GIB
    except (OSError, ValueError):
        return None, None


def _cgroup_available_gib() -> float | None:
    # Compatibility helper for callers/tests from earlier revisions.
    return _cgroup_memory_gib()[1]


def effective_cpu_threads() -> int:
    """Logical CPUs this process can actually schedule on (Linux affinity aware)."""
    try:
        allowed = len(os.sched_getaffinity(0))
        if allowed > 0:
            return allowed
    except (AttributeError, OSError):
        pass
    return max(1, os.cpu_count() or 1)


def memory_stats() -> MemoryStats:
    values = _meminfo_values()
    total_kib = values.get('MemTotal')
    available_kib = values.get('MemAvailable')
    swap_total_kib = values.get('SwapTotal')
    swap_free_kib = values.get('SwapFree')

    total = total_kib / 1024 / 1024 if total_kib is not None else None
    available = available_kib / 1024 / 1024 if available_kib is not None else None
    cgroup_limit, cgroup_available = _cgroup_memory_gib()
    if cgroup_limit is not None:
        total = min(total, cgroup_limit) if total is not None else cgroup_limit
    if cgroup_available is not None:
        available = min(available, cgroup_available) if available is not None else cgroup_available
    swap_total = swap_total_kib / 1024 / 1024 if swap_total_kib is not None else None
    swap_free = swap_free_kib / 1024 / 1024 if swap_free_kib is not None else None
    swap_used = max(0.0, swap_total - swap_free) if swap_total is not None and swap_free is not None else None
    swap_used_percent = (
        min(100.0, max(0.0, swap_used / swap_total * 100.0))
        if swap_used is not None and swap_total is not None and swap_total > 0
        else (0.0 if swap_total == 0 else None)
    )
    return MemoryStats(
        total_gib=total,
        available_gib=available,
        swap_total_gib=swap_total,
        swap_free_gib=swap_free,
        swap_used_gib=swap_used,
        swap_used_percent=swap_used_percent,
    )


def disk_stats(path: Path) -> DiskStats | None:
    try:
        usage = shutil.disk_usage(path)
        return DiskStats(usage.total, usage.used, usage.free)
    except OSError:
        return None


def _psi_file(name: str) -> dict[str, dict[str, float]]:
    result: dict[str, dict[str, float]] = {}
    try:
        for line in Path('/proc/pressure', name).read_text(encoding='utf-8').splitlines():
            parts = line.split()
            if not parts:
                continue
            row: dict[str, float] = {}
            for token in parts[1:]:
                if '=' not in token:
                    continue
                key, value = token.split('=', 1)
                try:
                    row[key] = float(value)
                except ValueError:
                    pass
            result[parts[0]] = row
    except OSError:
        pass
    return result


def pressure_stats() -> PressureStats:
    cpu = _psi_file('cpu')
    memory = _psi_file('memory')
    io = _psi_file('io')
    return PressureStats(
        cpu_some_avg10=cpu.get('some', {}).get('avg10'),
        memory_some_avg10=memory.get('some', {}).get('avg10'),
        memory_full_avg10=memory.get('full', {}).get('avg10'),
        io_some_avg10=io.get('some', {}).get('avg10'),
        io_full_avg10=io.get('full', {}).get('avg10'),
    )


def process_rss_gib(pid: int) -> float | None:
    try:
        for line in Path(f'/proc/{pid}/status').read_text(encoding='utf-8').splitlines():
            if line.startswith('VmRSS:'):
                return int(line.split()[1]) / 1024 / 1024
    except (OSError, ValueError, IndexError):
        pass
    return None


def process_swap_gib(pid: int) -> float | None:
    """Return Linux VmSwap for a process; shared-memory swap is not included."""
    try:
        for line in Path(f"/proc/{pid}/status").read_text(encoding="utf-8").splitlines():
            if line.startswith("VmSwap:"):
                return int(line.split()[1]) / 1024 / 1024
    except (OSError, ValueError, IndexError):
        pass
    return None


def _temperature_candidates() -> list[float]:
    temps: list[float] = []
    # thermal_zone is common on laptops/SoCs.
    for p in Path('/sys/class/thermal').glob('thermal_zone*/temp'):
        try:
            value = float(p.read_text(encoding='utf-8').strip())
            value = value / 1000.0 if value > 1000 else value
            if 10.0 <= value <= 125.0:
                temps.append(value)
        except (OSError, ValueError):
            pass
    # hwmon covers many desktop/laptop CPU package sensors.
    for p in Path('/sys/class/hwmon').glob('hwmon*/temp*_input'):
        try:
            value = float(p.read_text(encoding='utf-8').strip())
            value = value / 1000.0 if value > 1000 else value
            if 10.0 <= value <= 125.0:
                temps.append(value)
        except (OSError, ValueError):
            pass
    return temps


def temperature_c() -> float | None:
    temps = _temperature_candidates()
    return max(temps) if temps else None


def load1() -> float | None:
    try:
        return float(os.getloadavg()[0])
    except (OSError, AttributeError):
        return None


def snapshot(
    path: Path | None = None,
    *,
    pid: int | None = None,
    cpu_sampler: CpuSampler | None = None,
    swap_sampler: SwapSampler | None = None,
) -> ResourceSnapshot:
    return ResourceSnapshot(
        cpu_percent=cpu_sampler.sample() if cpu_sampler is not None else None,
        load1=load1(),
        memory=memory_stats(),
        disk=disk_stats(path) if path is not None else None,
        pressure=pressure_stats(),
        temperature_c=temperature_c(),
        process_rss_gib=process_rss_gib(pid) if pid is not None else None,
        swap_activity=swap_sampler.sample() if swap_sampler is not None else None,
        process_swap_gib=process_swap_gib(pid) if pid is not None else None,
    )


def automatic_memory_reserve_gib(total_gib: float | None, *, floor_gib: float, percent: float) -> float:
    """Dynamic reserve: configured floor plus a percentage of usable RAM."""
    if total_gib is None or percent <= 0:
        return max(0.0, float(floor_gib))
    return max(float(floor_gib), total_gib * float(percent) / 100.0)
