from __future__ import annotations

import logging
import math
import re
import os
import subprocess
import sys
import time
import signal
from dataclasses import dataclass, replace
from pathlib import Path

from .model import MediaProfile, Toolchain
from .util import ExternalCommandError, RebuildError, duration_text, format_command, human_bytes, start_stderr_drain
from .ui import ProgressView
from .resources import CpuSampler, MemoryStats, PressureStats, ResourceSnapshot, SwapSampler, automatic_memory_reserve_gib, disk_stats, effective_cpu_threads, memory_stats, pressure_stats, process_rss_gib, snapshot


SWAP_IN_PRESSURE_BPS = 4 * 1024**2
SWAP_PRESSURE_MIN_SAMPLES = 5
MEMORY_FULL_PRESSURE_PERCENT = 1.0




@dataclass(frozen=True)
class MemoryForecast:
    """Profile-aware scheduling forecast; this is a budget heuristic, not peak-RSS prediction."""

    model: str
    level: str
    available_gib: float | None
    total_gib: float | None
    swap_free_gib: float | None
    swap_used_gib: float | None
    swap_used_percent: float | None
    reserve_gib: float
    planning_budget_gib: float | None
    profile_factor: float
    preset_factor: float
    per_frame_allowance_gib: float
    per_worker_allowance_gib: float
    modeled_plan_gib: float | None
    memory_worker_cap: int | None
    requested_workers: int | None
    requested_frame_threads: int | None
    planned_workers: int | None
    planned_frame_threads: int | None
    messages: tuple[str, ...]


@dataclass(frozen=True)
class ResourcePlan:
    worker_threads: int | None
    frame_threads: int | None
    mem_available_gib: float | None
    swap_free_gib: float | None
    cpu_threads: int
    reason: str
    memory_guard_gib: float = 0.0
    memory_forecast: MemoryForecast | None = None

    def reduced(self) -> "ResourcePlan":
        """Return a lower-memory retry plan without changing quality settings."""
        workers = self.worker_threads
        frames = self.frame_threads
        if workers is None:
            workers = max(1, self.cpu_threads - 1)
        # Frame encoders consume substantial memory. Reduce them first, then
        # step the worker pool down while still leaving useful parallelism.
        if frames is None or frames > 1:
            frames = 1
            workers = max(2, int(workers * 0.80)) if workers > 2 else workers
        else:
            workers = max(2, int(workers * 0.65)) if workers > 2 else 1
        forecast = self.memory_forecast
        if forecast is not None:
            modeled = (frames or 1) * forecast.per_frame_allowance_gib + (workers or 1) * forecast.per_worker_allowance_gib
            level = "high" if forecast.planning_budget_gib is not None and modeled > forecast.planning_budget_gib * 1.15 else "constrained"
            forecast = replace(
                forecast,
                level=level,
                modeled_plan_gib=round(modeled, 3),
                planned_workers=workers,
                planned_frame_threads=frames,
                messages=forecast.messages + ("A live memory-pressure event reduced x265 parallelism for this retry.",),
            )
        return ResourcePlan(
            worker_threads=workers,
            frame_threads=frames,
            mem_available_gib=self.mem_available_gib,
            swap_free_gib=self.swap_free_gib,
            cpu_threads=self.cpu_threads,
            reason="adaptive retry after memory pressure",
            memory_guard_gib=self.memory_guard_gib,
            memory_forecast=forecast,
        )


class MemoryPressureError(RebuildError):
    def __init__(
        self,
        message: str,
        *,
        evidence: str = "",
        available_gib: float | None = None,
        resource_summary: dict[str, float | None] | None = None,
    ) -> None:
        super().__init__(message)
        self.evidence = evidence
        self.available_gib = available_gib
        self.resource_summary = resource_summary or {}


class DiskPressureError(RebuildError):
    def __init__(self, message: str, *, free_bytes: int | None = None, projected_remaining_bytes: int | None = None) -> None:
        super().__init__(message)
        self.free_bytes = free_bytes
        self.projected_remaining_bytes = projected_remaining_bytes


def _meminfo_values() -> dict[str, int]:
    # Compatibility helper retained for older callers/tests.
    values: dict[str, int] = {}
    try:
        for line in Path("/proc/meminfo").read_text(encoding="utf-8").splitlines():
            if ":" not in line:
                continue
            key, rest = line.split(":", 1)
            token = rest.strip().split()[0]
            if token.isdigit():
                values[key] = int(token)
    except OSError:
        pass
    return values


def _cgroup_available_gib() -> float | None:
    # Kept as a compatibility alias; resources.memory_stats() handles cgroup-v2.
    stats = memory_stats()
    return stats.available_gib


def _meminfo_gib() -> tuple[float | None, float | None]:
    stats = memory_stats()
    return stats.available_gib, stats.swap_free_gib


def _proc_rss_gib(pid: int) -> float | None:
    return process_rss_gib(pid)


def choose_resource_plan(
    profile: MediaProfile,
    *,
    threading: str,
    reserved_cpu_threads: int = 1,
    max_worker_threads: int = 0,
    frame_threads: int = 0,
    preset: str = "slow",
    min_free_memory_gib: float = 3.0,
    memory_reserve_percent: float = 8.0,
    mem_stats: MemoryStats | None = None,
    pressure: PressureStats | None = None,
) -> ResourcePlan:
    cpu = effective_cpu_threads()
    mem_stats = mem_stats or memory_stats()
    pressure = pressure or pressure_stats()
    mem = mem_stats.available_gib
    swap = mem_stats.swap_free_gib
    guard = automatic_memory_reserve_gib(
        mem_stats.total_gib, floor_gib=min_free_memory_gib, percent=memory_reserve_percent
    )

    if mem is not None and mem < guard:
        raise RebuildError(
            f"Only {mem:.1f} GiB RAM is currently available. "
            f"HEVC Rebuild's dynamic safety reserve is {guard:.1f} GiB. "
            "Close memory-heavy programs or adjust [resources] only if you intentionally accept that risk."
        )

    if threading == "unlimited":
        # "Unlimited" opts out of reserved CPU threads; the memory-aware cap still applies.
        workers = max_worker_threads or cpu
        frames = frame_threads or 0
        reason = "unlimited mode requested"
    else:
        # Default policy: use all logical CPUs except one. Memory pressure is
        # handled independently by the live guard/retry path.
        workers = max(1, cpu - max(0, reserved_cpu_threads))
        if max_worker_threads:
            workers = min(workers, max_worker_threads)

        width = int(profile.primary_video.width or 0)
        height = int(profile.primary_video.height or 0)
        pixels = width * height
        if frame_threads:
            frames = frame_threads
        else:
            # Frame parallelism is expensive in RAM. Two frames is a conservative
            # modern default for HD/4K while WPP keeps the worker pool busy.
            frames = 2 if pixels >= 1280 * 720 else 1
        reason = f"all logical CPU threads minus {max(0, reserved_cpu_threads)}; dynamic memory/PSI guard"

    width = int(profile.primary_video.width or 0)
    height = int(profile.primary_video.height or 0)
    pixels = max(1, width * height)
    target_depth = profile.source_bit_depth or 8
    match = re.search(r"p(10|12)le?$", profile.target_pix_fmt or "")
    if match:
        target_depth = max(target_depth, int(match.group(1)))
    chroma = profile.source_chroma or "4:2:0"
    chroma_factor = 1.5 if "4:4:4" in chroma else 1.25 if "4:2:2" in chroma else 1.0
    # These lookahead depths follow x265's documented presets. This is still a
    # scheduling allowance because x265 versions and encoder builds can differ.
    preset_lookahead = {
        "ultrafast": 5,
        "superfast": 10,
        "veryfast": 15,
        "faster": 15,
        "fast": 15,
        "medium": 20,
        "slow": 25,
        "slower": 40,
        "veryslow": 40,
    }
    preset_factor = preset_lookahead.get(preset, 25) / 25.0
    profile_factor = max(0.25, pixels / (1920 * 1080)) * max(1.0, target_depth / 8.0) * chroma_factor * preset_factor

    requested_workers = workers
    requested_frames = frames or (2 if pixels >= 1280 * 720 else 1)
    per_frame_allowance = profile_factor
    per_worker_allowance = profile_factor * 0.25
    planning_budget: float | None = None
    memory_worker_cap: int | None = None
    messages: list[str] = [
        "Profile-scaled concurrency allowance is a conservative scheduling heuristic, not a prediction of exact peak encoder RAM.",
        "Current swap is reported for context but is not counted as fast memory headroom.",
    ]
    original_workers = workers
    original_frames = requested_frames

    if mem is not None:
        # Keep one quarter of the post-reserve headroom unallocated for other
        # processes and model error. Existing live monitoring remains the final guard.
        pressure_factor = 0.5 if (pressure.memory_full_avg10 or 0.0) >= 1.0 else 0.75
        planning_budget = max(0.0, mem - guard) * pressure_factor
        frame_cap = max(1, min(requested_frames, int(planning_budget / per_frame_allowance)))
        if frame_threads == 0:
            frames = frame_cap
        else:
            frames = frame_threads
        worker_budget = max(0.0, planning_budget - (frames * per_frame_allowance))
        memory_worker_cap = max(1, int(worker_budget / per_worker_allowance))
        current_workers = workers if workers is not None else cpu
        workers = min(current_workers, memory_worker_cap)
        if frames != original_frames or workers != original_workers:
            reason += "; concurrency limited by profile-aware memory budget"
            messages.append(
                f"Automatic concurrency was adjusted to {workers} worker(s) and {frames} frame thread(s) using current available RAM."
            )
        if pressure_factor < 0.75:
            messages.append("Recent memory PSI indicates sustained pressure; the starting budget was reduced further.")
    else:
        frames = requested_frames
        if workers is None:
            workers = cpu
        messages.append("Available RAM could not be read; the CPU plan is used with live pressure monitoring.")

    modeled_plan = None
    if planning_budget is not None:
        modeled_plan = (frames * per_frame_allowance) + ((workers or cpu) * per_worker_allowance)
        minimum_plan = per_frame_allowance + per_worker_allowance
        if (pressure.memory_full_avg10 or 0.0) >= 1.0 or modeled_plan > planning_budget * 1.15 or planning_budget < minimum_plan:
            level = "high"
            messages.append("This file has little modeled memory headroom; the run may still be stopped safely if live memory pressure rises.")
        elif workers != original_workers or frames != original_frames:
            level = "constrained"
        else:
            level = "guarded"
    else:
        level = "unknown"

    forecast = MemoryForecast(
        model="profile-scaled-concurrency-v1",
        level=level,
        available_gib=mem,
        total_gib=mem_stats.total_gib,
        swap_free_gib=swap,
        swap_used_gib=mem_stats.swap_used_gib,
        swap_used_percent=mem_stats.swap_used_percent,
        reserve_gib=guard,
        planning_budget_gib=planning_budget,
        profile_factor=round(profile_factor, 3),
        preset_factor=round(preset_factor, 3),
        per_frame_allowance_gib=round(per_frame_allowance, 3),
        per_worker_allowance_gib=round(per_worker_allowance, 3),
        modeled_plan_gib=round(modeled_plan, 3) if modeled_plan is not None else None,
        memory_worker_cap=memory_worker_cap,
        requested_workers=requested_workers,
        requested_frame_threads=original_frames,
        planned_workers=workers,
        planned_frame_threads=frames,
        messages=tuple(messages),
    )

    return ResourcePlan(
        worker_threads=workers,
        frame_threads=frames,
        mem_available_gib=mem,
        swap_free_gib=swap,
        cpu_threads=cpu,
        reason=reason,
        memory_guard_gib=guard,
        memory_forecast=forecast,
    )


def _recent_oom_evidence() -> str:
    """Best-effort kernel/cgroup OOM evidence; permissions may hide it."""
    patterns = re.compile(r"(out of memory|oom-kill|killed process|memory cgroup out of memory)", re.I)
    commands = [
        ["journalctl", "-k", "--since", "-5 min", "--no-pager", "-n", "160"],
        ["dmesg", "--ctime", "--level=err,warn"],
    ]
    for cmd in commands:
        try:
            result = subprocess.run(cmd, capture_output=True, text=True, timeout=5, check=False)
            matches = [line for line in result.stdout.splitlines() if patterns.search(line)]
            if matches:
                return "\n".join(matches[-12:])
        except (OSError, subprocess.SubprocessError):
            pass
    return ""


_DISPOSITION_ORDER = [
    "default", "dub", "original", "comment", "lyrics", "karaoke", "forced",
    "hearing_impaired", "visual_impaired", "clean_effects", "attached_pic",
    "timed_thumbnails", "captions", "descriptions", "metadata", "dependent",
    "still_image", "non_diegetic",
]


def _disposition_value(disposition: dict, *, force_default: bool | None = None) -> str:
    flags = [name for name in _DISPOSITION_ORDER if int(disposition.get(name, 0) or 0) == 1 and name != "attached_pic"]
    if force_default is True and "default" not in flags:
        flags.insert(0, "default")
    elif force_default is False and "default" in flags:
        flags.remove("default")
    return "+".join(flags) if flags else "0"


def x265_params(profile: MediaProfile, hdr10plus_json: Path | None, *, preserve_source_level: bool, disable_hdr10_opt: bool) -> list[str]:
    params: list[str] = []
    if preserve_source_level and profile.format_level:
        import re
        m = re.search(r"(\d+(?:\.\d+)?)", profile.format_level)
        if m and float(m.group(1)) >= 5.0:
            params.append(f"level-idc={m.group(1)}")
    if profile.color_primaries:
        params.append(f"colorprim={profile.color_primaries}")
    if profile.transfer:
        params.append(f"transfer={profile.transfer}")
    if profile.color_matrix:
        params.append(f"colormatrix={profile.color_matrix}")
    if profile.color_range_x265:
        params.append(f"range={profile.color_range_x265}")
    if profile.chroma_location is not None and "420" in profile.target_pix_fmt:
        # x265 documents chromaloc for 4:2:0 input only.
        params.append(f"chromaloc={profile.chroma_location}")
    if profile.master_display:
        params.append(f"master-display={profile.master_display}")
    if profile.max_cll is not None:
        params.append(f"max-cll={profile.max_cll},{profile.max_fall or 0}")
    if profile.hdr10:
        params.append("hdr10=1")
        # x265 documents hdr10-opt for 10-bit 4:2:0 BT.2020/PQ content. Do not
        # force that optimization onto 12-bit or 4:2:2/4:4:4 HDR sources.
        if (
            not disable_hdr10_opt
            and profile.target_pix_fmt == "yuv420p10le"
            and profile.color_primaries == "bt2020"
            and profile.transfer == "smpte2084"
        ):
            params.append("hdr10-opt=1")
    if profile.hlg:
        # x265 requires Alternative Transfer Characteristics SEI and picture
        # structure signaling for HLG. Output is progressive in the safe path.
        params.extend(["atc-sei=18", "pic-struct=0"])
    if hdr10plus_json:
        if ":" in str(hdr10plus_json):
            raise RebuildError(f"HDR10+ JSON path contains ':' and cannot be safely embedded in x265-params: {hdr10plus_json}")
        params.extend([f"dhdr10-info={hdr10plus_json}", "dhdr10-opt=1"])
    return params


def encode_args(
    tools: Toolchain,
    profile: MediaProfile,
    output: Path,
    *,
    crf: int,
    preset: str,
    tune: str | None,
    hdr10plus_json: Path | None,
    preserve_dolbyvision: bool,
    strip_dolbyvision: bool,
    test_seconds: int | None,
    preserve_source_level: bool,
    disable_hdr10_opt: bool,
    secondary_video_policy: str,
    data_policy: str,
    force_deinterlace: bool,
    disable_deinterlace: bool,
    resource_threading: str = "auto",
    reserved_cpu_threads: int = 1,
    max_worker_threads: int = 0,
    frame_threads: int = 0,
    min_free_memory_gib: float = 0.0,
    memory_reserve_percent: float = 8.0,
    resource_plan: ResourcePlan | None = None,
) -> tuple[list[str], dict[int, int]]:
    if profile.target_pix_fmt not in tools.x265_pixel_formats:
        raise RebuildError(
            f"Installed FFmpeg/libx265 does not advertise required pixel format {profile.target_pix_fmt!r}; refusing a silent chroma/bit-depth downgrade."
        )
    args = [
        tools.ffmpeg, "-hide_banner", "-nostdin", "-y", "-loglevel", "error", "-nostats", "-stats_period", "0.5",
        "-progress", "pipe:1", "-analyzeduration", "200M", "-probesize", "200M", "-i", str(profile.path),
    ]
    if test_seconds:
        args += ["-t", str(test_seconds)]

    # Deterministic stream order: selected highest-resolution video first, optional copied secondary videos, then all audio/sub/attachments/data.
    output_video_map: dict[int, int] = {}
    args += ["-map", f"0:{profile.primary_video.index}"]
    output_video_map[profile.primary_video.index] = 0
    if secondary_video_policy == "copy":
        for out_ord, stream in enumerate(profile.secondary_videos, start=1):
            args += ["-map", f"0:{stream.index}"]
            output_video_map[stream.index] = out_ord
    args += ["-map", "0:a?", "-map", "0:s?", "-map", "0:t?"]
    if data_policy == "preserve":
        args += ["-map", "0:d?"]
    args += ["-map_metadata", "0", "-map_chapters", "0", "-c", "copy"]

    # Hard invariant: only v:0 is encoded. Audio and subtitles remain under global -c copy.
    args += ["-c:v:0", "libx265", "-pix_fmt:v:0", profile.target_pix_fmt, "-preset:v:0", preset, "-crf:v:0", str(crf)]
    if tune:
        args += ["-tune:v:0", tune]
    if preserve_dolbyvision and strip_dolbyvision:
        raise RebuildError("Internal error: Dolby Vision cannot be preserved and stripped in the same encode.")
    if preserve_dolbyvision:
        # Modern FFmpeg/libx265 can carry decoded per-frame Dolby Vision RPU side
        # data directly into the HEVC encoder. Force it on so a missing metadata
        # frame becomes a hard encode error instead of a silent DV loss.
        args += ["-dolbyvision:v:0", "1"]
    elif strip_dolbyvision and tools.x265_dolbyvision:
        # libx265's FFmpeg wrapper defaults this option to auto. An explicit HDR10
        # fallback must therefore force it OFF or decoded RPU side data could be
        # re-emitted accidentally on capable builds. Older builds lack the option,
        # in which case omission is the only valid form and no DV coding occurs.
        args += ["-dolbyvision:v:0", "0"]
    deint = force_deinterlace or (profile.interlaced and not disable_deinterlace)
    if profile.hlg and profile.interlaced and disable_deinterlace and not force_deinterlace:
        raise RebuildError(
            "Interlaced HLG with --disable-deinterlace cannot be signaled safely by this workflow. "
            "Use the default bwdif path or explicitly redesign picture-structure signaling."
        )
    if deint:
        args += ["-filter:v:0", "bwdif=mode=send_frame:parity=auto:deint=interlaced"]
    # Do not let FFmpeg duplicate/drop primary frames. This is especially important
    # for frame-associated HDR10+ metadata.
    args += ["-fps_mode:v:0", "passthrough"]
    if profile.color_primaries:
        args += ["-color_primaries:v:0", profile.color_primaries]
    if profile.transfer:
        args += ["-color_trc:v:0", profile.transfer]
    if profile.color_matrix:
        args += ["-colorspace:v:0", profile.color_matrix]
    if profile.color_range_ffmpeg:
        args += ["-color_range:v:0", profile.color_range_ffmpeg]
    params = x265_params(profile, hdr10plus_json, preserve_source_level=preserve_source_level, disable_hdr10_opt=disable_hdr10_opt)
    plan = resource_plan or choose_resource_plan(
        profile,
        threading=resource_threading,
        reserved_cpu_threads=reserved_cpu_threads,
        max_worker_threads=max_worker_threads,
        frame_threads=frame_threads,
        preset=preset,
        min_free_memory_gib=min_free_memory_gib,
        memory_reserve_percent=memory_reserve_percent,
    )
    if plan.worker_threads:
        params.append(f"pools={plan.worker_threads}")
    if plan.frame_threads:
        params.append(f"frame-threads={plan.frame_threads}")
    logging.debug(
        "x265 resources: workers=%s frame_threads=%s CPU_threads=%d RAM_available=%s GiB swap_free=%s GiB guard=%s GiB (%s)",
        plan.worker_threads or "x265-auto",
        plan.frame_threads or "x265-auto",
        plan.cpu_threads,
        f"{plan.mem_available_gib:.1f}" if plan.mem_available_gib is not None else "unknown",
        f"{plan.swap_free_gib:.1f}" if plan.swap_free_gib is not None else "unknown",
        f"{plan.memory_guard_gib:.1f}",
        plan.reason,
    )
    if params:
        args += ["-x265-params:v:0", ":".join(params)]

    # Make selected/highest-resolution video the default track while preserving every other disposition flag.
    args += ["-disposition:v:0", _disposition_value(profile.primary_video.disposition, force_default=True)]
    if secondary_video_policy == "copy":
        for out_ord, stream in enumerate(profile.secondary_videos, start=1):
            args += [f"-disposition:v:{out_ord}", _disposition_value(stream.disposition, force_default=False)]

    # FFmpeg may synthesize default dispositions when none exist. Pin audio/subtitle dispositions explicitly
    # so the rebuilt file preserves them rather than merely preserving packet payloads.
    for stream in profile.audio:
        args += [f"-disposition:a:{stream.type_ordinal}", _disposition_value(stream.disposition)]
    for stream in profile.subtitles:
        args += [f"-disposition:s:{stream.type_ordinal}", _disposition_value(stream.disposition)]

    args += ["-max_muxing_queue_size", "9999", "-f", "matroska", str(output)]
    return args, output_video_map


def _terminate_child(proc: subprocess.Popen, *, timeout: float = 8.0) -> None:
    if proc.poll() is not None:
        return
    proc.terminate()
    try:
        proc.wait(timeout=timeout)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait(timeout=5)


def _resource_detail(path: Path, pid: int | None = None, resources: ResourceSnapshot | None = None) -> str:
    snap = resources or snapshot(path, pid=pid)
    parts: list[str] = []
    if snap.cpu_percent is not None:
        parts.append(f"CPU={snap.cpu_percent:.1f}%")
    if snap.load1 is not None:
        parts.append(f"load1={snap.load1:.2f}")
    if snap.memory.available_gib is not None:
        parts.append(f"RAM_available={snap.memory.available_gib:.2f}GiB")
    if snap.memory.swap_free_gib is not None:
        parts.append(f"swap_free={snap.memory.swap_free_gib:.2f}GiB")
    if snap.memory.swap_used_gib is not None:
        parts.append(f"swap_used={snap.memory.swap_used_gib:.2f}GiB ({snap.memory.swap_used_percent:.1f}%)" if snap.memory.swap_used_percent is not None else f"swap_used={snap.memory.swap_used_gib:.2f}GiB")
    if snap.swap_activity is not None:
        if snap.swap_activity.in_bytes_per_second is not None:
            parts.append(f"swap_in={human_bytes(int(snap.swap_activity.in_bytes_per_second))}/s")
        if snap.swap_activity.out_bytes_per_second is not None:
            parts.append(f"swap_out={human_bytes(int(snap.swap_activity.out_bytes_per_second))}/s")
    if snap.disk is not None:
        parts.append(f"disk_free={human_bytes(snap.disk.free_bytes)}")
    if snap.process_rss_gib is not None:
        parts.append(f"encoder_RSS={snap.process_rss_gib:.2f}GiB")
    if snap.process_swap_gib is not None:
        parts.append(f"encoder_swap={snap.process_swap_gib:.2f}GiB")
    p = snap.pressure
    if p.memory_some_avg10 is not None:
        parts.append(f"PSI_mem_some10={p.memory_some_avg10:.2f}%")
    if p.memory_full_avg10 is not None:
        parts.append(f"PSI_mem_full10={p.memory_full_avg10:.2f}%")
    if p.io_some_avg10 is not None:
        parts.append(f"PSI_io_some10={p.io_some_avg10:.2f}%")
    if p.io_full_avg10 is not None:
        parts.append(f"PSI_io_full10={p.io_full_avg10:.2f}%")
    if snap.temperature_c is not None:
        parts.append(f"temperature={snap.temperature_c:.1f}C")
    return ", ".join(parts) if parts else "resource metrics unavailable"


def _encode_resource_summary(
    *,
    min_memory_available_gib: float | None,
    max_swap_used_percent: float | None,
    max_swap_in_bps: float | None,
    max_swap_out_bps: float | None,
    max_encoder_rss_gib: float | None,
    max_encoder_swap_gib: float | None,
    max_mem_psi_full: float | None,
) -> dict[str, float | None]:
    return {
        "min_memory_available_gib": min_memory_available_gib,
        "max_swap_used_percent": max_swap_used_percent,
        "max_swap_in_mib_s": max_swap_in_bps / (1024**2) if max_swap_in_bps is not None else None,
        "max_swap_out_mib_s": max_swap_out_bps / (1024**2) if max_swap_out_bps is not None else None,
        "max_encoder_rss_gib": max_encoder_rss_gib,
        "max_encoder_swap_gib": max_encoder_swap_gib,
        "max_memory_full_psi_percent": max_mem_psi_full,
    }


def run_encode(
    args: list[str],
    *,
    expected_duration: float,
    input_size: int,
    label: str,
    memory_guard_gib: float = 0.0,
    output_path: Path | None = None,
    disk_guard_gib: float = 0.0,
    disk_projection_margin_percent: float = 15.0,
    batch_number: int = 1,
    batch_total: int = 1,
) -> dict[str, float | None]:
    logging.debug("FFmpeg command: %s", format_command(args))
    proc = subprocess.Popen(args, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, bufsize=1)
    stderr_lines: list[str] = []
    drain = start_stderr_drain(proc.stderr, stderr_lines)
    state: dict[str, str] = {}
    start = time.monotonic()
    view = ProgressView(
        label=label,
        expected_duration=expected_duration,
        memory_guard_gib=memory_guard_gib,
        batch_number=batch_number,
        batch_total=batch_total,
    )
    cpu_sampler = CpuSampler()
    swap_sampler = SwapSampler()
    low_memory_samples = 0
    swap_pressure_samples = 0
    low_disk_samples = 0
    min_seen: float | None = None
    min_disk_free: int | None = None
    max_cpu: float | None = None
    max_mem_psi_full: float | None = None
    max_io_psi_full: float | None = None
    max_swap_in_bps: float | None = None
    max_swap_out_bps: float | None = None
    max_swap_used_percent: float | None = None
    max_encoder_rss_gib: float | None = None
    max_encoder_swap_gib: float | None = None
    last_snapshot: ResourceSnapshot | None = None
    monitor_path = (output_path or Path(args[-1])).parent
    disk_guard_bytes = int(max(0.0, disk_guard_gib) * 1024**3)
    projection_margin = 1.0 + max(0.0, disk_projection_margin_percent) / 100.0
    try:
        assert proc.stdout is not None
        for raw in proc.stdout:
            line = raw.strip()
            if "=" not in line:
                continue
            key, value = line.split("=", 1)
            state[key] = value
            if key != "progress":
                continue
            out_us = 0
            for candidate in ("out_time_us", "out_time_ms"):
                try:
                    out_us = int(state.get(candidate, "0"))
                    break
                except ValueError:
                    pass
            done = max(0.0, out_us / 1_000_000.0)
            speed_text = state.get("speed", "0x").rstrip("x")
            try:
                speed = float(speed_text)
            except ValueError:
                speed = 0.0
            try:
                size = int(state.get("total_size", "0"))
            except ValueError:
                size = 0

            snap = snapshot(monitor_path, pid=proc.pid, cpu_sampler=cpu_sampler, swap_sampler=swap_sampler)
            last_snapshot = snap
            mem = snap.memory.available_gib
            if mem is not None:
                min_seen = mem if min_seen is None else min(min_seen, mem)
            if snap.disk is not None:
                min_disk_free = snap.disk.free_bytes if min_disk_free is None else min(min_disk_free, snap.disk.free_bytes)
            if snap.cpu_percent is not None:
                max_cpu = snap.cpu_percent if max_cpu is None else max(max_cpu, snap.cpu_percent)
            if snap.pressure.memory_full_avg10 is not None:
                max_mem_psi_full = snap.pressure.memory_full_avg10 if max_mem_psi_full is None else max(max_mem_psi_full, snap.pressure.memory_full_avg10)
            if snap.pressure.io_full_avg10 is not None:
                max_io_psi_full = snap.pressure.io_full_avg10 if max_io_psi_full is None else max(max_io_psi_full, snap.pressure.io_full_avg10)
            if snap.swap_activity and snap.swap_activity.in_bytes_per_second is not None:
                rate = snap.swap_activity.in_bytes_per_second
                max_swap_in_bps = rate if max_swap_in_bps is None else max(max_swap_in_bps, rate)
            if snap.swap_activity and snap.swap_activity.out_bytes_per_second is not None:
                rate = snap.swap_activity.out_bytes_per_second
                max_swap_out_bps = rate if max_swap_out_bps is None else max(max_swap_out_bps, rate)
            if snap.memory.swap_used_percent is not None:
                max_swap_used_percent = snap.memory.swap_used_percent if max_swap_used_percent is None else max(max_swap_used_percent, snap.memory.swap_used_percent)
            if snap.process_swap_gib is not None:
                max_encoder_swap_gib = snap.process_swap_gib if max_encoder_swap_gib is None else max(max_encoder_swap_gib, snap.process_swap_gib)
            if snap.process_rss_gib is not None:
                max_encoder_rss_gib = snap.process_rss_gib if max_encoder_rss_gib is None else max(max_encoder_rss_gib, snap.process_rss_gib)

            view.render(done=done, speed=speed, size=size, resources=snap)

            # Memory: use the dynamic reserve calculated before the encode. A few
            # consecutive samples are required so a transient cache/reclaim event
            # cannot cause a needless restart.
            if memory_guard_gib > 0 and mem is not None and mem < memory_guard_gib:
                low_memory_samples += 1
            else:
                low_memory_samples = 0
            if low_memory_samples >= 3:
                psi = snap.pressure.memory_full_avg10
                psi_text = f" Memory PSI full/10s={psi:.1f}%." if psi is not None else ""
                view.note(
                    f"Memory reserve crossed ({mem:.1f} GiB < {memory_guard_gib:.1f} GiB)."
                    f"{psi_text} Retrying with lower x265 parallelism; quality stays identical."
                )
                _terminate_child(proc)
                raise MemoryPressureError(
                    f"Available RAM fell to {mem:.1f} GiB during encode, below the {memory_guard_gib:.1f} GiB dynamic safety reserve.",
                    evidence=_resource_detail(monitor_path, proc.pid, snap),
                    available_gib=mem,
                    resource_summary=_encode_resource_summary(
                        min_memory_available_gib=min_seen,
                        max_swap_used_percent=max_swap_used_percent,
                        max_swap_in_bps=max_swap_in_bps,
                        max_swap_out_bps=max_swap_out_bps,
                        max_encoder_rss_gib=max_encoder_rss_gib,
                        max_encoder_swap_gib=max_encoder_swap_gib,
                        max_mem_psi_full=max_mem_psi_full,
                    ),
                )

            # A large amount of allocated swap can be harmless cached state. Retry
            # only when active swap-in traffic coincides with full-memory stalls
            # for several consecutive progress samples.
            swap_in_bps = snap.swap_activity.in_bytes_per_second if snap.swap_activity else None
            memory_full_psi = snap.pressure.memory_full_avg10
            if (
                swap_in_bps is not None
                and swap_in_bps >= SWAP_IN_PRESSURE_BPS
                and memory_full_psi is not None
                and memory_full_psi >= MEMORY_FULL_PRESSURE_PERCENT
            ):
                swap_pressure_samples += 1
            else:
                swap_pressure_samples = 0
            if swap_pressure_samples >= SWAP_PRESSURE_MIN_SAMPLES:
                view.note(
                    "Sustained swap-in traffic coincides with full-memory stalls. "
                    "Retrying the temporary encode with lower x265 parallelism; quality stays identical."
                )
                _terminate_child(proc)
                raise MemoryPressureError(
                    "Sustained swap-in pressure and memory PSI stalls detected during encode.",
                    evidence=_resource_detail(monitor_path, proc.pid, snap),
                    available_gib=mem,
                    resource_summary=_encode_resource_summary(
                        min_memory_available_gib=min_seen,
                        max_swap_used_percent=max_swap_used_percent,
                        max_swap_in_bps=max_swap_in_bps,
                        max_swap_out_bps=max_swap_out_bps,
                        max_encoder_rss_gib=max_encoder_rss_gib,
                        max_encoder_swap_gib=max_encoder_swap_gib,
                        max_mem_psi_full=max_mem_psi_full,
                    ),
                )

            # Disk: protect a hard free-space reserve and, after enough progress to
            # make the estimate meaningful, ensure projected remaining output can
            # fit without consuming that reserve.
            if snap.disk is not None and disk_guard_bytes > 0:
                if snap.disk.free_bytes < disk_guard_bytes:
                    low_disk_samples += 1
                else:
                    low_disk_samples = 0
                if low_disk_samples >= 2:
                    view.note(
                        f"Storage reserve crossed ({human_bytes(snap.disk.free_bytes)} free). Stopping safely before the filesystem fills."
                    )
                    _terminate_child(proc)
                    raise DiskPressureError(
                        f"Free storage fell below the {disk_guard_gib:.1f} GiB safety reserve during encode.",
                        free_bytes=snap.disk.free_bytes,
                    )

                fraction = done / expected_duration if expected_duration > 0 else 0.0
                if fraction >= 0.10 and done >= 10.0 and size > 0:
                    projected_total = int(size / max(0.001, fraction))
                    projected_remaining = max(0, projected_total - size)
                    safe_remaining = max(0, snap.disk.free_bytes - disk_guard_bytes)
                    if projected_remaining * projection_margin > safe_remaining:
                        view.note(
                            "Projected output would consume the disk safety reserve. Stopping temporary encode before ENOSPC."
                        )
                        _terminate_child(proc)
                        raise DiskPressureError(
                            "Projected remaining output no longer fits while preserving the configured disk reserve.",
                            free_bytes=snap.disk.free_bytes,
                            projected_remaining_bytes=projected_remaining,
                        )

            if value == "end":
                break
    except KeyboardInterrupt:
        _terminate_child(proc, timeout=5)
        raise
    finally:
        if proc.stdout:
            proc.stdout.close()
    rc = proc.wait()
    drain.join(timeout=2)
    if rc != 0:
        stderr_tail = "".join(stderr_lines)[-16000:]
        resource_detail = _resource_detail(monitor_path, proc.pid, last_snapshot)
        logging.error("Encoder exit rc=%s; %s", rc, resource_detail)
        if rc == -signal.SIGKILL:
            mem_stats = memory_stats()
            oom = _recent_oom_evidence()
            if oom:
                detail = (
                    "\n\nKernel evidence indicates an out-of-memory kill while FFmpeg/x265 was running. "
                    + (f"RAM available now: {mem_stats.available_gib:.1f} GiB. " if mem_stats.available_gib is not None else "")
                    + (f"Swap free now: {mem_stats.swap_free_gib:.1f} GiB. " if mem_stats.swap_free_gib is not None else "")
                    + "HEVC Rebuild can retry the temporary encode with lower parallelism; video quality settings remain unchanged."
                    + "\nResource snapshot: " + resource_detail
                    + "\nRecent kernel OOM evidence:\n" + oom
                )
                raise MemoryPressureError(
                    "Linux OOM-killed FFmpeg/x265.",
                    evidence=detail,
                    available_gib=mem_stats.available_gib,
                    resource_summary=_encode_resource_summary(
                        min_memory_available_gib=min_seen,
                        max_swap_used_percent=max_swap_used_percent,
                        max_swap_in_bps=max_swap_in_bps,
                        max_swap_out_bps=max_swap_out_bps,
                        max_encoder_rss_gib=max_encoder_rss_gib,
                        max_encoder_swap_gib=max_encoder_swap_gib,
                        max_mem_psi_full=max_mem_psi_full,
                    ),
                )
            detail = (
                "\n\nFFmpeg/x265 received SIGKILL (signal 9) from outside the encoder. "
                "No kernel OOM evidence was available, so HEVC Rebuild will NOT claim this was definitely memory-related. "
                "HDR/x265 errors normally return their own diagnostic instead of an unexplained SIGKILL. "
                "\nResource snapshot: " + resource_detail
            )
            stderr_tail += detail
        raise ExternalCommandError(args, rc, stderr_tail)
    view.finish(f"Encode complete in {duration_text(time.monotonic() - start)}")
    logging.info(
        "Encode health: elapsed=%s min_RAM=%sGiB max_x265_RSS=%sGiB max_swap_used=%s%% max_x265_swap=%sGiB max_swap_in=%s/s max_swap_out=%s/s min_disk_free=%s max_CPU=%s%% max_PSI_memory_full10=%s%% max_PSI_io_full10=%s%%",
        duration_text(time.monotonic() - start),
        f"{min_seen:.1f}" if min_seen is not None else "unknown",
        f"{max_encoder_rss_gib:.2f}" if max_encoder_rss_gib is not None else "unknown",
        f"{max_swap_used_percent:.1f}" if max_swap_used_percent is not None else "unknown",
        f"{max_encoder_swap_gib:.2f}" if max_encoder_swap_gib is not None else "unknown",
        human_bytes(int(max_swap_in_bps)) if max_swap_in_bps is not None else "unknown",
        human_bytes(int(max_swap_out_bps)) if max_swap_out_bps is not None else "unknown",
        human_bytes(min_disk_free) if min_disk_free is not None else "unknown",
        f"{max_cpu:.1f}" if max_cpu is not None else "unknown",
        f"{max_mem_psi_full:.2f}" if max_mem_psi_full is not None else "unknown",
        f"{max_io_psi_full:.2f}" if max_io_psi_full is not None else "unknown",
    )
    return _encode_resource_summary(
        min_memory_available_gib=min_seen,
        max_swap_used_percent=max_swap_used_percent,
        max_swap_in_bps=max_swap_in_bps,
        max_swap_out_bps=max_swap_out_bps,
        max_encoder_rss_gib=max_encoder_rss_gib,
        max_encoder_swap_gib=max_encoder_swap_gib,
        max_mem_psi_full=max_mem_psi_full,
    )
