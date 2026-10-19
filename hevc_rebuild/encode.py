from __future__ import annotations

import json
import logging
import math
import os
import re
import signal
import subprocess
import sys
import time
from dataclasses import dataclass, replace
from pathlib import Path

from . import resources
from .model import MediaProfile, StreamRef, Toolchain
from .util import ConsoleClosed, DiagnosticTail, ExternalCommandError, RebuildError, duration_text, format_command, human_bytes, native_json_action, safe_float, start_stderr_drain, terminate_process_group
from .ui import ProgressView
from .resources import CpuSampler, DiskStats, MemoryStats, PressureStats, ResourceSnapshot, SwapActivityStats, SwapSampler, automatic_memory_reserve_gib, effective_cpu_threads, memory_stats, pressure_stats, process_rss_gib, snapshot


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
        forecast = self.memory_forecast
        native_retry = _native_resource_retry(self, forecast) if forecast is not None else None
        if native_retry is not None:
            workers = int(native_retry["planned_workers"])
            frames = int(native_retry["planned_frame_threads"])
        else:
            if workers is None:
                workers = max(1, self.cpu_threads - 1)
            # Frame encoders consume substantial memory. Reduce them first, then
            # step the worker pool down while still leaving useful parallelism.
            if frames is None or frames > 1:
                frames = 1
                workers = max(2, int(workers * 0.80)) if workers > 2 else workers
            else:
                workers = max(2, int(workers * 0.65)) if workers > 2 else 1
        if forecast is not None:
            modeled = (frames or 1) * forecast.per_frame_allowance_gib + (workers or 1) * forecast.per_worker_allowance_gib
            if native_retry is not None:
                modeled = float(native_retry["modeled_plan_gib"])
                level = str(native_retry["level"])
            else:
                level = "high" if forecast.planning_budget_gib is not None and modeled > forecast.planning_budget_gib * 1.15 else "constrained"
            forecast = replace(
                forecast,
                level=level,
                modeled_plan_gib=round(modeled, 3),
                planned_workers=workers,
                planned_frame_threads=frames,
                messages=forecast.messages + ("A live memory-pressure event reduced encoder parallelism for this retry.",),
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


def _native_json_action(action: str, payload: dict[str, object]) -> dict[str, object] | None:
    return native_json_action(action, payload)


def _native_stream_plan_payload(stream: StreamRef) -> dict[str, object]:
    """Serialize only the stream facts used by Rust's encoder argument planner."""
    return {
        "index": stream.index,
        "type_ordinal": stream.type_ordinal,
        "codec_type": stream.codec_type,
        "codec_name": stream.codec_name,
        "width": stream.width,
        "height": stream.height,
        "default": stream.default,
        "attached_pic": stream.attached_pic,
        "disposition": stream.disposition,
    }


_NATIVE_PROFILE_PLAN_FIELDS = (
    "source_bit_depth", "source_chroma", "target_pix_fmt", "color_primaries",
    "transfer", "color_matrix", "color_range_x265", "color_range_ffmpeg",
    "chroma_location", "master_display", "max_cll", "max_fall", "hdr10",
    "hlg", "dolby_vision", "interlaced", "format_level",
)


def _native_media_plan_payload(profile: MediaProfile) -> dict[str, object]:
    return {
        "path": str(profile.path),
        "primary_video": _native_stream_plan_payload(profile.primary_video),
        "secondary_videos": [_native_stream_plan_payload(stream) for stream in profile.secondary_videos],
        "audio": [_native_stream_plan_payload(stream) for stream in profile.audio],
        "subtitles": [_native_stream_plan_payload(stream) for stream in profile.subtitles],
        "attachments": [_native_stream_plan_payload(stream) for stream in profile.attachments],
        "data": [_native_stream_plan_payload(stream) for stream in profile.data],
        **{name: getattr(profile, name) for name in _NATIVE_PROFILE_PLAN_FIELDS},
    }


def _native_encode_plan(
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
    resource_plan: ResourcePlan | None,
    selected_streams: list[StreamRef] | None,
    video_profiles: dict[int, MediaProfile] | None,
    encode_video_indices: set[int] | None,
    hdr10plus_jsons: dict[int, Path] | None,
    preserve_dolbyvision_indices: set[int] | None,
    strip_dolbyvision_indices: set[int] | None,
) -> tuple[list[str], dict[int, int]] | None:
    if not os.environ.get("HEVC_REBUILD_NATIVE_HELPER"):
        return None
    selected_profiles = video_profiles or {profile.primary_video.index: profile}
    selected_hdr_jsons = hdr10plus_jsons or (
        {profile.primary_video.index: hdr10plus_json} if hdr10plus_json else {}
    )
    request: dict[str, object] = {
        "tools": {
            "ffmpeg": tools.ffmpeg,
            "x265_pixel_formats": sorted(tools.x265_pixel_formats),
            "x265_dolbyvision": tools.x265_dolbyvision,
        },
        "profile": _native_media_plan_payload(profile),
        "video_profiles": {
            str(index): _native_media_plan_payload(item)
            for index, item in selected_profiles.items()
        },
        "hdr10plus_jsons": {str(index): str(path) for index, path in selected_hdr_jsons.items()},
        "preserve_dolbyvision_indices": sorted(preserve_dolbyvision_indices or ()),
        "strip_dolbyvision_indices": sorted(strip_dolbyvision_indices or ()),
        "resource_plan": {
            "worker_threads": resource_plan.worker_threads if resource_plan else None,
            "frame_threads": resource_plan.frame_threads if resource_plan else None,
        },
        "options": {
            "output": str(output),
            "crf": crf,
            "preset": preset,
            "tune": tune,
            "preserve_dolbyvision": preserve_dolbyvision,
            "strip_dolbyvision": strip_dolbyvision,
            "test_seconds": test_seconds,
            "preserve_source_level": preserve_source_level,
            "disable_hdr10_opt": disable_hdr10_opt,
            "secondary_video_policy": secondary_video_policy,
            "data_policy": data_policy,
            "force_deinterlace": force_deinterlace,
            "disable_deinterlace": disable_deinterlace,
        },
    }
    if selected_streams is not None:
        request["selected_streams"] = [_native_stream_plan_payload(stream) for stream in selected_streams]
    if encode_video_indices is not None:
        request["encode_video_indices"] = sorted(encode_video_indices)
    response = _native_json_action("encode-plan", request)
    if response is None:
        return None
    args = response.get("args")
    video_map = response.get("output_video_map")
    if not isinstance(args, list) or not all(isinstance(item, str) for item in args):
        raise RebuildError("The Rust encode-plan helper returned invalid FFmpeg arguments.")
    if not isinstance(video_map, dict):
        raise RebuildError("The Rust encode-plan helper returned an invalid video-track map.")
    try:
        mapped = {int(index): int(ordinal) for index, ordinal in video_map.items()}
    except (TypeError, ValueError) as exc:
        raise RebuildError("The Rust encode-plan helper returned an invalid video-track map.") from exc
    return args, mapped


def _native_resource_retry(
    plan: ResourcePlan, forecast: MemoryForecast
) -> dict[str, object] | None:
    response = _native_json_action(
        "resource-retry",
        {
            "worker_threads": plan.worker_threads,
            "frame_threads": plan.frame_threads,
            "per_frame_allowance_gib": forecast.per_frame_allowance_gib,
            "per_worker_allowance_gib": forecast.per_worker_allowance_gib,
            "planning_budget_gib": forecast.planning_budget_gib,
        },
    )
    if response is None:
        return None
    for key in ("planned_workers", "planned_frame_threads"):
        value = response.get(key)
        if not isinstance(value, int) or isinstance(value, bool) or value < 1:
            raise RebuildError(f"The Rust resource-retry helper returned an invalid {key} value.")
    modeled = response.get("modeled_plan_gib")
    level = response.get("level")
    if (
        not isinstance(modeled, (int, float))
        or isinstance(modeled, bool)
        or not math.isfinite(modeled)
        or modeled < 0
        or level not in {"high", "constrained"}
    ):
        raise RebuildError("The Rust resource-retry helper returned an invalid forecast.")
    return response


def _native_optional_number(value: object, field: str) -> float | None:
    if value is None:
        return None
    if not isinstance(value, (int, float)) or isinstance(value, bool) or not math.isfinite(value):
        raise RebuildError(f"The Rust resource-plan helper returned an invalid {field} value.")
    return float(value)


def _native_optional_integer(value: object, field: str) -> int | None:
    if value is None:
        return None
    if not isinstance(value, int) or isinstance(value, bool):
        raise RebuildError(f"The Rust resource-plan helper returned an invalid {field} value.")
    return value


def _native_memory_forecast(value: dict[str, object]) -> MemoryForecast:
    model = value.get("model")
    level = value.get("level")
    messages = value.get("messages")
    if not isinstance(model, str) or not isinstance(level, str) or not isinstance(messages, list):
        raise RebuildError("The Rust resource-plan helper returned an incomplete memory forecast.")
    if not all(isinstance(message, str) for message in messages):
        raise RebuildError("The Rust resource-plan helper returned invalid forecast messages.")
    return MemoryForecast(
        model=model,
        level=level,
        available_gib=_native_optional_number(value.get("available_gib"), "available_gib"),
        total_gib=_native_optional_number(value.get("total_gib"), "total_gib"),
        swap_free_gib=_native_optional_number(value.get("swap_free_gib"), "swap_free_gib"),
        swap_used_gib=_native_optional_number(value.get("swap_used_gib"), "swap_used_gib"),
        swap_used_percent=_native_optional_number(value.get("swap_used_percent"), "swap_used_percent"),
        reserve_gib=_native_optional_number(value.get("reserve_gib"), "reserve_gib") or 0.0,
        planning_budget_gib=_native_optional_number(value.get("planning_budget_gib"), "planning_budget_gib"),
        profile_factor=_native_optional_number(value.get("profile_factor"), "profile_factor") or 0.0,
        preset_factor=_native_optional_number(value.get("preset_factor"), "preset_factor") or 0.0,
        per_frame_allowance_gib=_native_optional_number(value.get("per_frame_allowance_gib"), "per_frame_allowance_gib") or 0.0,
        per_worker_allowance_gib=_native_optional_number(value.get("per_worker_allowance_gib"), "per_worker_allowance_gib") or 0.0,
        modeled_plan_gib=_native_optional_number(value.get("modeled_plan_gib"), "modeled_plan_gib"),
        memory_worker_cap=_native_optional_integer(value.get("memory_worker_cap"), "memory_worker_cap"),
        requested_workers=_native_optional_integer(value.get("requested_workers"), "requested_workers"),
        requested_frame_threads=_native_optional_integer(value.get("requested_frame_threads"), "requested_frame_threads"),
        planned_workers=_native_optional_integer(value.get("planned_workers"), "planned_workers"),
        planned_frame_threads=_native_optional_integer(value.get("planned_frame_threads"), "planned_frame_threads"),
        messages=tuple(messages),
    )


def _native_resource_plan(
    profile: MediaProfile,
    *,
    target_dimensions: tuple[int, int] | None,
    threading: str,
    reserved_cpu_threads: int,
    max_worker_threads: int,
    frame_threads: int,
    preset: str,
    min_free_memory_gib: float,
    memory_reserve_percent: float,
    mem_stats: MemoryStats | None,
    pressure: PressureStats | None,
    cpu_threads: int | None,
    concurrent_video_streams: int,
) -> ResourcePlan | None:
    width, height = target_dimensions or (
        profile.primary_video.width or 0,
        profile.primary_video.height or 0,
    )
    request: dict[str, object] = {
            "profile": {
                "video": {
                    "width": int(width),
                    "height": int(height),
                    "source_bit_depth": profile.source_bit_depth,
                    "source_chroma": profile.source_chroma,
                    "target_pixel_format": profile.target_pix_fmt,
                }
            },
            "settings": {
                "video": {"preset": preset},
                "resources": {
                    "threading": threading,
                    "reserved_cpu_threads": reserved_cpu_threads,
                    "max_worker_threads": max_worker_threads,
                    "frame_threads": frame_threads,
                    "min_free_memory_gib": min_free_memory_gib,
                    "memory_reserve_percent": memory_reserve_percent,
                },
            },
            "sample_live_memory_pressure": pressure is None,
            "full_memory_psi_avg10": pressure.memory_full_avg10 if pressure is not None else None,
            "target_dimensions": [int(width), int(height)] if target_dimensions is not None else None,
            "preset_override": preset,
            "frame_threads_override": frame_threads,
            "concurrent_video_streams": concurrent_video_streams,
        }
    if mem_stats is not None:
        hardware: dict[str, object] = {
            "memory_available_gib": mem_stats.available_gib,
            "memory_total_gib": mem_stats.total_gib,
            "swap_free_gib": mem_stats.swap_free_gib,
            "swap_used_gib": mem_stats.swap_used_gib,
            "swap_used_percent": mem_stats.swap_used_percent,
        }
        if cpu_threads is not None:
            hardware["logical_cpus"] = cpu_threads
        request["host"] = {"hardware": hardware}
    response = _native_json_action("resource-plan", request)
    if response is None:
        return None
    level = response.get("level")
    if level == "blocked":
        available = _native_optional_number(response.get("available_gib"), "available_gib")
        guard = _native_optional_number(response.get("reserve_gib"), "reserve_gib") or 0.0
        available_text = f"{available:.1f}" if available is not None else "unknown"
        raise RebuildError(
            f"Only {available_text} GiB RAM is currently available. HEVC Rebuild's dynamic safety reserve is "
            f"{guard:.1f} GiB. Close memory-heavy programs or adjust [resources] only if you intentionally accept that risk."
        )
    forecast = _native_memory_forecast(response)
    workers = forecast.planned_workers
    frames = forecast.planned_frame_threads
    reason = response.get("plan_reason")
    native_cpu_threads = _native_optional_integer(response.get("cpu_threads"), "cpu_threads")
    if (
        workers is None
        or workers < 1
        or frames is None
        or frames < 1
        or not isinstance(reason, str)
        or native_cpu_threads is None
        or native_cpu_threads < 1
    ):
        raise RebuildError("The Rust resource-plan helper returned an invalid encoder plan.")
    return ResourcePlan(
        worker_threads=workers,
        frame_threads=frames,
        mem_available_gib=forecast.available_gib,
        swap_free_gib=forecast.swap_free_gib,
        cpu_threads=native_cpu_threads,
        reason=reason,
        memory_guard_gib=forecast.reserve_gib,
        memory_forecast=forecast,
    )


def _native_resource_snapshot(value: object) -> ResourceSnapshot:
    if not isinstance(value, dict):
        raise RebuildError("The Rust runtime sampler returned an invalid resource snapshot.")

    def number(field: str) -> float | None:
        raw = value.get(field)
        if raw is None:
            return None
        if not isinstance(raw, (int, float)) or isinstance(raw, bool) or not math.isfinite(raw) or raw < 0:
            raise RebuildError(f"The Rust runtime sampler returned an invalid {field} value.")
        return float(raw)

    def integer(field: str) -> int | None:
        raw = value.get(field)
        if raw is None:
            return None
        if not isinstance(raw, int) or isinstance(raw, bool) or raw < 0:
            raise RebuildError(f"The Rust runtime sampler returned an invalid {field} value.")
        return raw

    disk_values = tuple(integer(field) for field in ("disk_total_bytes", "disk_used_bytes", "disk_free_bytes"))
    disk = DiskStats(*disk_values) if all(item is not None for item in disk_values) else None
    return ResourceSnapshot(
        cpu_percent=number("cpu_percent"),
        load1=number("load1"),
        memory=MemoryStats(
            total_gib=number("memory_total_gib"),
            available_gib=number("memory_available_gib"),
            swap_total_gib=number("swap_total_gib"),
            swap_free_gib=number("swap_free_gib"),
            swap_used_gib=number("swap_used_gib"),
            swap_used_percent=number("swap_used_percent"),
        ),
        disk=disk,
        pressure=PressureStats(
            cpu_some_avg10=number("cpu_some_avg10"),
            memory_some_avg10=number("memory_some_avg10"),
            memory_full_avg10=number("memory_full_avg10"),
            io_some_avg10=number("io_some_avg10"),
            io_full_avg10=number("io_full_avg10"),
        ),
        temperature_c=number("temperature_c"),
        process_rss_gib=number("ffmpeg_process_rss_gib") or number("engine_process_rss_gib"),
        swap_activity=SwapActivityStats(
            in_bytes_per_second=number("swap_in_bytes_per_second"),
            out_bytes_per_second=number("swap_out_bytes_per_second"),
        ),
        process_swap_gib=number("ffmpeg_process_swap_gib") or number("engine_process_swap_gib"),
    )


class _NativeRuntimeGuard:
    """Rust-owned live memory/disk policy for Rust-launched engine runs."""

    def __init__(self, executable: str) -> None:
        try:
            self.proc = subprocess.Popen(
                [executable, "internal", "runtime-guard"],
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                bufsize=1,
                start_new_session=os.name == "posix",
            )
        except OSError as exc:
            raise RebuildError(f"Could not start the Rust runtime guard: {exc}") from exc
        assert self.proc.stdin is not None
        assert self.proc.stdout is not None
        self._stderr_lines = DiagnosticTail()
        self._stderr_drain = start_stderr_drain(self.proc.stderr, self._stderr_lines)
        self._closed = False

    @classmethod
    def from_environment(cls) -> "_NativeRuntimeGuard | None":
        executable = os.environ.get("HEVC_REBUILD_NATIVE_HELPER")
        return cls(executable) if executable else None

    def decide(
        self,
        request: dict[str, int | float | str | None],
    ) -> tuple[dict[str, object], ResourceSnapshot]:
        if self._closed or self.proc.poll() is not None:
            raise RebuildError("The Rust runtime guard exited unexpectedly during encoding.")
        assert self.proc.stdin is not None
        assert self.proc.stdout is not None
        try:
            self.proc.stdin.write(json.dumps(request, separators=(",", ":"), allow_nan=False) + "\n")
            self.proc.stdin.flush()
            response = self.proc.stdout.readline()
        except (OSError, ValueError) as exc:
            raise RebuildError(f"Could not communicate with the Rust runtime guard: {exc}") from exc
        if not response:
            detail = self._stderr_lines.text(4000)
            suffix = f" Details: {detail.strip()}" if detail.strip() else ""
            raise RebuildError(f"The Rust runtime guard stopped responding.{suffix}")
        try:
            result = json.loads(response)
        except json.JSONDecodeError as exc:
            raise RebuildError("The Rust runtime guard returned malformed JSON.") from exc
        if not isinstance(result, dict):
            raise RebuildError("The Rust runtime guard returned an invalid response.")
        decision = result.get("decision")
        if not isinstance(decision, dict) or decision.get("action") not in {
            "continue",
            "retry_memory",
            "stop_disk",
        }:
            raise RebuildError("The Rust runtime guard returned an unsupported decision.")
        return decision, _native_resource_snapshot(result.get("snapshot"))

    def close(self, *, check_exit: bool = True) -> None:
        if self._closed:
            return
        self._closed = True
        if self.proc.stdin:
            try:
                self.proc.stdin.close()
            except OSError:
                pass
        try:
            return_code = self.proc.wait(timeout=3)
        except subprocess.TimeoutExpired:
            self.proc.terminate()
            try:
                return_code = self.proc.wait(timeout=1)
            except subprocess.TimeoutExpired:
                self.proc.kill()
                return_code = self.proc.wait(timeout=1)
        self._stderr_drain.join(timeout=2)
        if self._stderr_drain.is_alive():
            terminate_process_group(self.proc)
            self._stderr_drain.join(timeout=2)
        if self.proc.stdout:
            self.proc.stdout.close()
        if self.proc.stderr and not self._stderr_drain.is_alive():
            self.proc.stderr.close()
        if check_exit and self._stderr_drain.is_alive():
            raise RebuildError("Rust runtime guard diagnostic drain did not stop.")
        if check_exit and return_code != 0:
            detail = self._stderr_lines.text(4000)
            suffix = f" Details: {detail.strip()}" if detail.strip() else ""
            raise RebuildError(f"The Rust runtime guard exited with status {return_code}.{suffix}")


def _meminfo_values() -> dict[str, int]:
    # Compatibility adapter; canonical parser is shared with memory_stats().
    return resources._meminfo_values()


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
    target_dimensions: tuple[int, int] | None = None,
    threading: str,
    reserved_cpu_threads: int = 1,
    max_worker_threads: int = 0,
    frame_threads: int = 0,
    preset: str = "slow",
    min_free_memory_gib: float = 3.0,
    memory_reserve_percent: float = 8.0,
    mem_stats: MemoryStats | None = None,
    pressure: PressureStats | None = None,
    concurrent_video_streams: int = 1,
) -> ResourcePlan:
    concurrent_video_streams = max(1, min(int(concurrent_video_streams), 16))
    if os.environ.get("HEVC_REBUILD_NATIVE_HELPER"):
        native_plan = _native_resource_plan(
            profile,
            target_dimensions=target_dimensions,
            threading=threading,
            reserved_cpu_threads=reserved_cpu_threads,
            max_worker_threads=max_worker_threads,
            frame_threads=frame_threads,
            preset=preset,
            min_free_memory_gib=min_free_memory_gib,
            memory_reserve_percent=memory_reserve_percent,
            mem_stats=mem_stats,
            pressure=pressure,
            cpu_threads=effective_cpu_threads() if mem_stats is not None else None,
            concurrent_video_streams=concurrent_video_streams,
        )
        if native_plan is not None:
            return native_plan
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

        width, height = target_dimensions or (profile.primary_video.width or 0, profile.primary_video.height or 0)
        width, height = int(width), int(height)
        pixels = width * height
        if frame_threads:
            frames = frame_threads
        else:
            # Frame parallelism is expensive in RAM. Two frames is a conservative
            # modern default for HD/4K while WPP keeps the worker pool busy.
            frames = 2 if pixels >= 1280 * 720 else 1
        reason = f"all logical CPU threads minus {max(0, reserved_cpu_threads)}; dynamic memory/PSI guard"

    width, height = target_dimensions or (profile.primary_video.width or 0, profile.primary_video.height or 0)
    width, height = int(width), int(height)
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
    # FFmpeg may have several mapped libx265 encoders active at once. Budget
    # the largest selected stream's model once per encoder, and cap CPU workers
    # per encoder so aggregate thread count remains bounded.
    per_frame_allowance = profile_factor * concurrent_video_streams
    per_worker_allowance = profile_factor * 0.25 * concurrent_video_streams
    planning_budget: float | None = None
    memory_worker_cap: int | None = None
    messages: list[str] = [
        "Profile-scaled concurrency allowance is a conservative scheduling heuristic, not a prediction of exact peak encoder RAM.",
        "Current swap is reported for context but is not counted as fast memory headroom.",
    ]
    original_workers = workers
    workers = max(1, workers // concurrent_video_streams)
    if concurrent_video_streams > 1:
        reason += f"; shared across {concurrent_video_streams} selected video encoders"
        messages.append(
            f"The memory estimate reserves a conservative single-stream allowance for each of {concurrent_video_streams} selected video encoders."
        )
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
    selected_streams: list[StreamRef] | None = None,
    video_profiles: dict[int, MediaProfile] | None = None,
    encode_video_indices: set[int] | None = None,
    hdr10plus_jsons: dict[int, Path] | None = None,
    preserve_dolbyvision_indices: set[int] | None = None,
    strip_dolbyvision_indices: set[int] | None = None,
) -> tuple[list[str], dict[int, int]]:
    native_plan = _native_encode_plan(
        tools,
        profile,
        output,
        crf=crf,
        preset=preset,
        tune=tune,
        hdr10plus_json=hdr10plus_json,
        preserve_dolbyvision=preserve_dolbyvision,
        strip_dolbyvision=strip_dolbyvision,
        test_seconds=test_seconds,
        preserve_source_level=preserve_source_level,
        disable_hdr10_opt=disable_hdr10_opt,
        secondary_video_policy=secondary_video_policy,
        data_policy=data_policy,
        force_deinterlace=force_deinterlace,
        disable_deinterlace=disable_deinterlace,
        resource_plan=resource_plan,
        selected_streams=selected_streams,
        video_profiles=video_profiles,
        encode_video_indices=encode_video_indices,
        hdr10plus_jsons=hdr10plus_jsons,
        preserve_dolbyvision_indices=preserve_dolbyvision_indices,
        strip_dolbyvision_indices=strip_dolbyvision_indices,
    )
    if native_plan is not None:
        return native_plan

    video_profiles = video_profiles or {profile.primary_video.index: profile}
    hdr10plus_jsons = hdr10plus_jsons or ({profile.primary_video.index: hdr10plus_json} if hdr10plus_json else {})
    preserve_dolbyvision_indices = preserve_dolbyvision_indices or set()
    strip_dolbyvision_indices = strip_dolbyvision_indices or set()
    if selected_streams is None:
        videos = [profile.primary_video]
        if secondary_video_policy != "drop":
            videos.extend(profile.secondary_videos)
        selected_streams = [*videos, *profile.audio, *profile.subtitles, *profile.attachments]
        if data_policy == "preserve":
            selected_streams.extend(profile.data)
    selected_videos = [stream for stream in selected_streams if stream.codec_type == "video"]
    selected_real_videos = [stream for stream in selected_videos if not stream.attached_pic and stream.width > 0 and stream.height > 0]
    if not selected_real_videos:
        raise RebuildError("Select at least one usable video track to rebuild this file.")
    selected_real_videos.sort(key=lambda stream: (-stream.pixels, -int(stream.default), stream.index))
    selected_videos = [*selected_real_videos, *[stream for stream in selected_videos if stream.attached_pic]]
    if encode_video_indices is None:
        encode_video_indices = {profile.primary_video.index}
        if secondary_video_policy == "encode":
            encode_video_indices.update(stream.index for stream in selected_real_videos)
    selected_audio = [stream for stream in selected_streams if stream.codec_type == "audio"]
    selected_subtitles = [stream for stream in selected_streams if stream.codec_type == "subtitle"]
    selected_attachments = [stream for stream in selected_streams if stream.codec_type == "attachment"]
    selected_data = [stream for stream in selected_streams if stream.codec_type == "data"]
    selected_types = selected_videos + selected_audio + selected_subtitles + selected_attachments + selected_data
    if len({stream.index for stream in selected_types}) != len(selected_types):
        raise RebuildError("A source stream was selected more than once.")
    if encode_video_indices - {stream.index for stream in selected_real_videos}:
        raise RebuildError("An encode selection refers to a video track that is not selected.")
    for stream in selected_real_videos:
        if stream.index in encode_video_indices:
            track_profile = video_profiles.get(stream.index, profile)
            if track_profile.target_pix_fmt not in tools.x265_pixel_formats:
                raise RebuildError(
                    f"Installed FFmpeg/libx265 does not advertise required pixel format {track_profile.target_pix_fmt!r} for {stream.label}; refusing a silent chroma/bit-depth downgrade."
                )
    args = [
        tools.ffmpeg, "-hide_banner", "-nostdin", "-y", "-loglevel", "error", "-nostats", "-stats_period", "0.5",
        "-progress", "pipe:1",
    ]
    if test_seconds is None:
        # Preserve negative codec priming and copied-track offsets exactly.
        args += ["-copyts"]
    args += ["-i", str(profile.path),
    ]
    if test_seconds:
        args += ["-t", str(test_seconds)]

    output_video_map: dict[int, int] = {}
    for out_ord, stream in enumerate(selected_videos):
        args += ["-map", f"0:{stream.index}"]
        output_video_map[stream.index] = out_ord
    if test_seconds is None:
        for stream in selected_audio + selected_subtitles + selected_attachments + selected_data:
            args += ["-map", f"0:{stream.index}"]
        args += ["-map_metadata", "0", "-map_chapters", "0"]
    else:
        args += ["-map_metadata", "-1", "-map_chapters", "-1"]
    args += ["-c", "copy"]
    plan = resource_plan or choose_resource_plan(
        profile,
        threading=resource_threading,
        reserved_cpu_threads=reserved_cpu_threads,
        max_worker_threads=max_worker_threads,
        frame_threads=frame_threads,
        preset=preset,
        min_free_memory_gib=min_free_memory_gib,
        memory_reserve_percent=memory_reserve_percent,
        concurrent_video_streams=len(selected_real_videos),
    )
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
    for out_ord, stream in enumerate(selected_videos):
        if test_seconds is None:
            args += [f"-map_metadata:s:v:{out_ord}", f"0:s:{stream.index}"]
        if stream.attached_pic or stream.index not in encode_video_indices:
            args += [f"-c:v:{out_ord}", "copy"]
            continue
        track_profile = video_profiles.get(stream.index, profile)
        args += [f"-c:v:{out_ord}", "libx265", f"-pix_fmt:v:{out_ord}", track_profile.target_pix_fmt,
                 f"-preset:v:{out_ord}", preset, f"-crf:v:{out_ord}", str(crf)]
        if tune:
            args += [f"-tune:v:{out_ord}", tune]
        keep_dv = stream.index in preserve_dolbyvision_indices or (preserve_dolbyvision and track_profile.dolby_vision)
        drop_dv = stream.index in strip_dolbyvision_indices or (strip_dolbyvision and track_profile.dolby_vision)
        if keep_dv and drop_dv:
            raise RebuildError("Internal error: Dolby Vision cannot be preserved and stripped on the same video stream.")
        if keep_dv:
            args += [f"-dolbyvision:v:{out_ord}", "1"]
        elif drop_dv and tools.x265_dolbyvision:
            args += [f"-dolbyvision:v:{out_ord}", "0"]
        deint = force_deinterlace or (track_profile.interlaced and not disable_deinterlace)
        if track_profile.hlg and track_profile.interlaced and disable_deinterlace and not force_deinterlace:
            raise RebuildError("Interlaced HLG with deinterlacing disabled cannot be signaled safely.")
        if deint:
            args += [f"-filter:v:{out_ord}", "bwdif=mode=send_frame:parity=auto:deint=interlaced"]
        args += [f"-fps_mode:v:{out_ord}", "passthrough"]
        for option, value in (
            ("color_primaries", track_profile.color_primaries),
            ("color_trc", track_profile.transfer),
            ("colorspace", track_profile.color_matrix),
            ("color_range", track_profile.color_range_ffmpeg),
        ):
            if value:
                args += [f"-{option}:v:{out_ord}", value]
        params = x265_params(
            track_profile,
            hdr10plus_jsons.get(stream.index),
            preserve_source_level=preserve_source_level,
            disable_hdr10_opt=disable_hdr10_opt,
        )
        if plan.worker_threads:
            params.append(f"pools={plan.worker_threads}")
        if plan.frame_threads:
            params.append(f"frame-threads={plan.frame_threads}")
        if params:
            args += [f"-x265-params:v:{out_ord}", ":".join(params)]
        args += [f"-disposition:v:{out_ord}", _disposition_value(stream.disposition, force_default=(out_ord == 0))]

    type_short = {"audio": "a", "subtitle": "s", "attachment": "t", "data": "d"}
    for kind, streams in (("audio", selected_audio), ("subtitle", selected_subtitles), ("attachment", selected_attachments), ("data", selected_data)):
        short = type_short[kind]
        for out_ord, stream in enumerate(streams):
            if test_seconds is None:
                args += [f"-map_metadata:s:{short}:{out_ord}", f"0:s:{stream.index}"]
            if kind in {"audio", "subtitle"}:
                args += [f"-disposition:{short}:{out_ord}", _disposition_value(stream.disposition)]

    if test_seconds is None:
        args += ["-avoid_negative_ts", "disabled"]
    args += ["-f", "matroska", str(output)]
    return args, output_video_map


def _terminate_child(
    proc: subprocess.Popen,
    *,
    timeout: float = 8.0,
    native_supervisor: bool = False,
) -> None:
    if proc.poll() is not None:
        terminate_process_group(proc, timeout=timeout)
        return
    if native_supervisor and proc.stdin is not None:
        try:
            proc.stdin.write("cancel\n")
            proc.stdin.flush()
        except (BrokenPipeError, OSError, ValueError):
            pass
        try:
            proc.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            terminate_process_group(proc, timeout=5)
        finally:
            try:
                proc.stdin.close()
            except OSError:
                pass
        terminate_process_group(proc, timeout=5)
        return
    terminate_process_group(proc, timeout=timeout)


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
    file_path: str | None = None,
    elapsed_seconds_before: float = 0.0,
    memory_guard_gib: float = 0.0,
    output_path: Path | None = None,
    disk_guard_gib: float = 0.0,
    disk_projection_margin_percent: float = 15.0,
    batch_number: int = 1,
    batch_total: int = 1,
) -> dict[str, float | None]:
    logging.debug("FFmpeg command: %s", format_command(args))
    native_guard = _NativeRuntimeGuard.from_environment()
    native_helper = os.environ.get("HEVC_REBUILD_NATIVE_HELPER")
    native_supervisor = bool(native_helper)
    process_args = (
        [native_helper, "internal", "encode-supervisor", "--", *args]
        if native_helper
        else args
    )
    try:
        proc = subprocess.Popen(
            process_args,
            stdin=subprocess.PIPE if native_supervisor else None,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            bufsize=1,
            start_new_session=os.name == "posix",
        )
    except BaseException:
        if native_guard is not None:
            native_guard.close(check_exit=False)
        raise
    stderr_lines = DiagnosticTail()
    drain = start_stderr_drain(proc.stderr, stderr_lines)
    state: dict[str, str] = {}
    start = time.monotonic()
    view = ProgressView(
        label=label,
        file_path=file_path,
        elapsed_seconds_before=elapsed_seconds_before,
        expected_duration=expected_duration,
        memory_guard_gib=memory_guard_gib,
        input_size=input_size,
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
            speed = max(0.0, safe_float(speed_text) or 0.0)
            try:
                size = int(state.get("total_size", "0"))
            except ValueError:
                size = 0

            native_decision: dict[str, object] | None = None
            if native_guard is not None:
                try:
                    native_decision, snap = native_guard.decide(
                        {
                            "pid": proc.pid,
                            "disk_path": str(monitor_path),
                            "memory_guard_gib": memory_guard_gib,
                            "disk_guard_bytes": disk_guard_bytes,
                            "projection_margin_percent": max(0.0, disk_projection_margin_percent),
                            "progress_seconds": done,
                            "expected_duration": expected_duration,
                            "output_bytes": max(0, size),
                        }
                    )
                except RebuildError:
                    _terminate_child(proc, native_supervisor=native_supervisor)
                    raise
            else:
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

            try:
                view.render(done=done, speed=speed, size=size, resources=snap)
            except ConsoleClosed:
                _terminate_child(proc, native_supervisor=native_supervisor)
                raise

            if native_guard is not None:
                assert native_decision is not None
                decision = native_decision
                action = decision["action"]
                reason = decision.get("reason")
                if action == "retry_memory":
                    if reason not in {"memory_reserve", "swap_pressure"}:
                        _terminate_child(proc, native_supervisor=native_supervisor)
                        raise RebuildError("The Rust runtime guard returned an invalid memory-retry reason.")
                    if reason == "memory_reserve":
                        available = decision.get("available_gib")
                        if not isinstance(available, (int, float)) or isinstance(available, bool):
                            _terminate_child(proc, native_supervisor=native_supervisor)
                            raise RebuildError("The Rust runtime guard omitted the measured memory level.")
                        psi = snap.pressure.memory_full_avg10
                        psi_text = f" Memory PSI full/10s={psi:.1f}%." if psi is not None else ""
                        view.note(
                            f"Memory reserve crossed ({available:.1f} GiB < {memory_guard_gib:.1f} GiB)."
                            f"{psi_text} Retrying with lower encoder parallelism; quality stays identical."
                        )
                        message = (
                            f"Available RAM fell to {available:.1f} GiB during encode, below the "
                            f"{memory_guard_gib:.1f} GiB dynamic safety reserve."
                        )
                    else:
                        view.note(
                            "Sustained swap-in traffic coincides with full-memory stalls. "
                            "Retrying the temporary encode with lower encoder parallelism; quality stays identical."
                        )
                        message = "Sustained swap-in pressure and memory PSI stalls detected during encode."
                        available = mem
                    _terminate_child(proc, native_supervisor=native_supervisor)
                    raise MemoryPressureError(
                        message,
                        evidence=_resource_detail(monitor_path, proc.pid, snap),
                        available_gib=float(available) if available is not None else None,
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
                if action == "stop_disk":
                    if reason not in {"disk_reserve", "projected_output"}:
                        _terminate_child(proc, native_supervisor=native_supervisor)
                        raise RebuildError("The Rust runtime guard returned an invalid disk-stop reason.")
                    free_bytes = decision.get("free_bytes")
                    if not isinstance(free_bytes, int) or isinstance(free_bytes, bool) or free_bytes < 0:
                        _terminate_child(proc, native_supervisor=native_supervisor)
                        raise RebuildError("The Rust runtime guard omitted the measured free-space value.")
                    if reason == "disk_reserve":
                        view.note(
                            f"Storage reserve crossed ({human_bytes(free_bytes)} free). "
                            "Stopping safely before the filesystem fills."
                        )
                        message = (
                            f"Free storage fell below the {disk_guard_gib:.1f} GiB safety reserve during encode."
                        )
                        projected_remaining = None
                    else:
                        view.note(
                            "Projected output would consume the disk safety reserve. "
                            "Stopping temporary encode before ENOSPC."
                        )
                        message = "Projected remaining output no longer fits while preserving the configured disk reserve."
                        projected_remaining = decision.get("projected_remaining_bytes")
                        if not isinstance(projected_remaining, int) or projected_remaining < 0:
                            _terminate_child(proc, native_supervisor=native_supervisor)
                            raise RebuildError("The Rust runtime guard omitted the projected output size.")
                    _terminate_child(proc, native_supervisor=native_supervisor)
                    raise DiskPressureError(
                        message,
                        free_bytes=free_bytes,
                        projected_remaining_bytes=projected_remaining,
                    )
            else:
                # Direct Python runs retain the reference policy until the Rust
                # launcher is present and the helper can make the same decision.
                if memory_guard_gib > 0 and mem is not None and mem < memory_guard_gib:
                    low_memory_samples += 1
                else:
                    low_memory_samples = 0
                if low_memory_samples >= 3:
                    psi = snap.pressure.memory_full_avg10
                    psi_text = f" Memory PSI full/10s={psi:.1f}%." if psi is not None else ""
                    view.note(
                        f"Memory reserve crossed ({mem:.1f} GiB < {memory_guard_gib:.1f} GiB)."
                        f"{psi_text} Retrying with lower encoder parallelism; quality stays identical."
                    )
                    _terminate_child(proc, native_supervisor=native_supervisor)
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
                        "Retrying the temporary encode with lower encoder parallelism; quality stays identical."
                    )
                    _terminate_child(proc, native_supervisor=native_supervisor)
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

                if snap.disk is not None:
                    if disk_guard_bytes > 0:
                        if snap.disk.free_bytes < disk_guard_bytes:
                            low_disk_samples += 1
                        else:
                            low_disk_samples = 0
                        if low_disk_samples >= 2:
                            view.note(
                                f"Storage reserve crossed ({human_bytes(snap.disk.free_bytes)} free). Stopping safely before the filesystem fills."
                            )
                            _terminate_child(proc, native_supervisor=native_supervisor)
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
                            _terminate_child(proc, native_supervisor=native_supervisor)
                            raise DiskPressureError(
                                "Projected remaining output no longer fits while preserving the configured disk reserve.",
                                free_bytes=snap.disk.free_bytes,
                                projected_remaining_bytes=projected_remaining,
                            )

            if value == "end":
                view.render(
                    done=max(done, expected_duration), speed=speed, size=size, resources=snap,
                    phase="Finalizing / muxing",
                )
                view.note("Progress reached 100%; FFmpeg is finalizing/muxing the output container.")
                logging.info("Encoding progress reached 100%%; FFmpeg is finalizing/muxing the output container.")
    except BaseException:
        _terminate_child(proc, timeout=5, native_supervisor=native_supervisor)
        raise
    finally:
        if proc.stdout:
            proc.stdout.close()
        if native_guard is not None:
            native_guard.close(check_exit=sys.exc_info()[0] is None)
    rc = proc.wait()
    drain.join(timeout=5)
    if proc.stdin is not None:
        proc.stdin.close()
    if native_supervisor:
        visible_stderr = DiagnosticTail()
        visible_stderr.truncated = stderr_lines.truncated
        visible_stderr.error = stderr_lines.error
        child_signal: int | None = None
        for line in stderr_lines:
            if line.startswith("__HEVC_NATIVE_CHILD_SIGNAL__="):
                try:
                    child_signal = int(line.split("=", 1)[1].strip())
                except ValueError:
                    visible_stderr.append(line)
            else:
                visible_stderr.append(line)
        stderr_lines = visible_stderr
        if child_signal is not None and child_signal > 0:
            rc = -child_signal
    if rc != 0:
        stderr_tail = stderr_lines.text(16000)
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
        "Encode health: elapsed=%s min_RAM=%sGiB max_FFmpeg_process_RSS=%sGiB max_swap_used=%s%% max_FFmpeg_process_swap=%sGiB max_swap_in=%s/s max_swap_out=%s/s min_disk_free=%s max_CPU=%s%% max_PSI_memory_full10=%s%% max_PSI_io_full10=%s%%",
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
