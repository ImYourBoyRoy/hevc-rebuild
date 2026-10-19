from __future__ import annotations

import hashlib
import json
import logging
import math
import os
import platform
import shutil
import signal
import subprocess
import tempfile
import time
from dataclasses import asdict, dataclass, replace
from functools import lru_cache
from pathlib import Path
from typing import Any

from .encode import (
    MEMORY_FULL_PRESSURE_PERCENT,
    SWAP_IN_PRESSURE_BPS,
    SWAP_PRESSURE_MIN_SAMPLES,
    ResourcePlan,
    x265_params,
)
from .model import MediaProfile, Toolchain
from .resources import SwapSampler, memory_stats, snapshot
from .util import DiagnosticTail, RebuildError, format_command, portable_data_dir, start_stderr_drain

GIB = 1024**3
MIB = 1024**2
MAX_CACHE_ENTRIES = 64
MAX_CACHE_AGE_HOURS = 720.0
_NATIVE_ENCODER_FINGERPRINTS: dict[str, str] = {}


@dataclass(frozen=True)
class CalibrationResult:
    worker_threads: int
    frame_threads: int
    elapsed_seconds: float
    media_seconds: float
    speed_x: float
    peak_rss_gib: float | None
    min_available_gib: float | None
    max_memory_psi_full: float | None
    max_swap_in_mib_s: float | None
    max_swap_out_mib_s: float | None
    healthy: bool
    reason: str = ""


@dataclass(frozen=True)
class CalibrationSelection:
    plan: ResourcePlan
    source: str  # benchmark | cache | base
    age_hours: float | None = None
    results: tuple[CalibrationResult, ...] = ()
    reason: str = ""


def cache_path() -> Path:
    portable = portable_data_dir()
    if portable is not None:
        return portable / "cache" / "calibration.json"
    cache_home = Path(os.environ.get("XDG_CACHE_HOME", Path.home() / ".cache"))
    return cache_home / "hevc-rebuild" / "calibration.json"


def is_demanding(profile: MediaProfile, source_size_bytes: int, *, large_file_gib: float) -> bool:
    if os.environ.get("HEVC_REBUILD_NATIVE_HELPER"):
        try:
            native = _native_calibration_action(
                "eligible",
                {
                    "width": int(profile.primary_video.width or 0),
                    "height": int(profile.primary_video.height or 0),
                    "source_size_bytes": max(0, int(source_size_bytes)),
                    "large_file_gib": large_file_gib,
                },
            )
            result = native.get("demanding") if native is not None else None
            if isinstance(result, bool):
                return result
            raise RebuildError("The Rust calibration-eligibility helper returned an invalid result.")
        except RebuildError as exc:
            logging.warning("Rust calibration eligibility failed; using compatibility logic: %s", exc)
    pixels = int(profile.primary_video.width or 0) * int(profile.primary_video.height or 0)
    return pixels >= 3840 * 2160 or source_size_bytes >= int(max(0.0, large_file_gib) * GIB)


def _resolution_class(profile: MediaProfile) -> str:
    pixels = int(profile.primary_video.width or 0) * int(profile.primary_video.height or 0)
    if pixels >= 3840 * 2160:
        return "2160p+"
    if pixels >= 2560 * 1440:
        return "1440p+"
    if pixels >= 1920 * 1080:
        return "1080p"
    return "sub1080p"


@lru_cache(maxsize=8)
def _encoder_fingerprint(ffmpeg: str) -> str:
    executable = shutil.which(ffmpeg) or ffmpeg
    parts = [str(Path(executable).expanduser())]
    try:
        stat = Path(executable).stat()
        parts.extend([str(stat.st_size), str(stat.st_mtime_ns)])
    except OSError:
        pass
    for args in ([executable, "-hide_banner", "-version"], [executable, "-hide_banner", "-h", "encoder=libx265"]):
        try:
            result = subprocess.run(args, capture_output=True, text=True, timeout=5, check=False)
            parts.append((result.stdout or "") + (result.stderr or ""))
            parts.append(str(result.returncode))
        except (OSError, subprocess.SubprocessError) as exc:
            parts.append(type(exc).__name__)
    return hashlib.sha256("\n".join(parts).encode("utf-8", "replace")).hexdigest()


@lru_cache(maxsize=1)
def _cpu_identity() -> str:
    identity = [platform.machine(), platform.processor()]
    if sys_platform_linux():
        try:
            for line in Path("/proc/cpuinfo").read_text(encoding="utf-8", errors="replace").splitlines():
                key, separator, value = line.partition(":")
                if separator and key.strip().lower() in {"model name", "hardware"}:
                    identity.append(value.strip())
                    break
        except OSError:
            pass
    return "|".join(value for value in identity if value)


def sys_platform_linux() -> bool:
    return platform.system() == "Linux"


def _native_calibration_action(mode: str, payload: dict[str, Any]) -> dict[str, Any] | None:
    executable = os.environ.get("HEVC_REBUILD_NATIVE_HELPER")
    if not executable:
        return None
    timeout = 90 if mode == "run" else 15
    try:
        result = subprocess.run(
            [executable, "internal", "calibration", mode],
            input=json.dumps(payload, separators=(",", ":"), allow_nan=False),
            capture_output=True,
            text=True,
            timeout=timeout,
            check=False,
        )
    except (OSError, subprocess.SubprocessError, ValueError) as exc:
        raise RebuildError(f"Could not run the Rust calibration {mode} helper: {exc}") from exc
    if result.returncode != 0:
        detail = result.stderr.strip()[-4000:]
        suffix = f" Details: {detail}" if detail else ""
        raise RebuildError(
            f"The Rust calibration {mode} helper exited with status {result.returncode}.{suffix}"
        )
    try:
        value = json.loads(result.stdout)
    except json.JSONDecodeError as exc:
        raise RebuildError("The Rust calibration helper returned malformed JSON.") from exc
    if not isinstance(value, dict):
        raise RebuildError("The Rust calibration helper returned an invalid response.")
    return value


def _native_results(value: object) -> tuple[CalibrationResult, ...]:
    if not isinstance(value, list):
        raise RebuildError("The Rust calibration helper returned invalid candidate results.")
    results: list[CalibrationResult] = []
    for item in value:
        if not isinstance(item, dict):
            raise RebuildError("The Rust calibration helper returned an invalid candidate result.")
        try:
            result = CalibrationResult(**item)
        except TypeError as exc:
            raise RebuildError("The Rust calibration helper returned an incomplete candidate result.") from exc
        numeric = (
            result.elapsed_seconds,
            result.media_seconds,
            result.speed_x,
            result.peak_rss_gib,
            result.min_available_gib,
            result.max_memory_psi_full,
            result.max_swap_in_mib_s,
            result.max_swap_out_mib_s,
        )
        if (
            not isinstance(result.worker_threads, int)
            or not isinstance(result.frame_threads, int)
            or not isinstance(result.healthy, bool)
            or not isinstance(result.reason, str)
            or any(value is not None and (not isinstance(value, (int, float)) or isinstance(value, bool) or not math.isfinite(value)) for value in numeric)
        ):
            raise RebuildError("The Rust calibration helper returned invalid candidate values.")
        results.append(result)
    return tuple(results)


def _native_profile_payload(profile: MediaProfile) -> dict[str, Any]:
    return {
        "source_path": str(profile.path),
        "duration": profile.duration,
        "primary_video_index": profile.primary_video.index,
        "primary_video_width": profile.primary_video.width,
        "primary_video_height": profile.primary_video.height,
        "source_bit_depth": profile.source_bit_depth,
        "source_chroma": profile.source_chroma,
        "target_pix_fmt": profile.target_pix_fmt,
        "format_level": profile.format_level,
        "color_primaries": profile.color_primaries,
        "transfer": profile.transfer,
        "color_matrix": profile.color_matrix,
        "color_range_x265": profile.color_range_x265,
        "color_range_ffmpeg": profile.color_range_ffmpeg,
        "chroma_location": profile.chroma_location,
        "master_display": profile.master_display,
        "max_cll": profile.max_cll,
        "max_fall": profile.max_fall,
        "hdr10": profile.hdr10,
        "hdr10plus": profile.hdr10plus,
        "hlg": profile.hlg,
        "dolby_vision": profile.dolby_vision,
        "dovi_profile": profile.dovi_profile,
        "dovi_rpu_present": profile.dovi_rpu_present,
        "interlaced": profile.interlaced,
    }


def _native_toolchain_payload(tools: Toolchain) -> dict[str, Any]:
    return {
        "ffmpeg": tools.ffmpeg,
        "x265_pixel_formats": sorted(tools.x265_pixel_formats),
        "x265_dolbyvision": tools.x265_dolbyvision,
    }


def _native_options_payload(
    *,
    preset: str,
    tune: str | None,
    crf: int,
    sample_seconds: float,
    hdr10plus_json: Path | None,
    preserve_dolbyvision: bool,
    strip_dolbyvision: bool,
    preserve_source_level: bool,
    disable_hdr10_opt: bool,
    force_deinterlace: bool,
    disable_deinterlace: bool,
) -> dict[str, Any]:
    return {
        "preset": preset,
        "tune": tune,
        "crf": crf,
        "sample_seconds": sample_seconds,
        "hdr10plus_json": str(hdr10plus_json) if hdr10plus_json else None,
        "preserve_dolbyvision": preserve_dolbyvision,
        "strip_dolbyvision": strip_dolbyvision,
        "preserve_source_level": preserve_source_level,
        "disable_hdr10_opt": disable_hdr10_opt,
        "force_deinterlace": force_deinterlace,
        "disable_deinterlace": disable_deinterlace,
    }


def _native_plan_payload(plan: ResourcePlan) -> dict[str, Any]:
    return {
        "worker_threads": plan.worker_threads,
        "frame_threads": plan.frame_threads,
        "cpu_threads": plan.cpu_threads,
        "memory_guard_gib": plan.memory_guard_gib,
    }


def calibration_key(
    tools: Toolchain,
    profile: MediaProfile,
    base_plan: ResourcePlan,
    *,
    preset: str,
    tune: str | None,
    crf: int,
    total_memory_gib: float | None,
    sample_seconds: float,
    max_candidates: int,
    throughput_tolerance_percent: float,
    hdr10plus_enabled: bool,
    preserve_source_level: bool,
    disable_hdr10_opt: bool,
    force_deinterlace: bool,
    disable_deinterlace: bool,
    preserve_dolbyvision: bool,
    strip_dolbyvision: bool,
) -> str:
    if os.environ.get("HEVC_REBUILD_NATIVE_HELPER"):
        try:
            encoder_fingerprint = _NATIVE_ENCODER_FINGERPRINTS.get(tools.ffmpeg)
            native = _native_calibration_action(
                "key",
                {
                    "profile": _native_profile_payload(profile),
                    "toolchain": _native_toolchain_payload(tools),
                    "options": _native_options_payload(
                        preset=preset,
                        tune=tune,
                        crf=crf,
                        sample_seconds=sample_seconds,
                        hdr10plus_json=None,
                        preserve_dolbyvision=preserve_dolbyvision,
                        strip_dolbyvision=strip_dolbyvision,
                        preserve_source_level=preserve_source_level,
                        disable_hdr10_opt=disable_hdr10_opt,
                        force_deinterlace=force_deinterlace,
                        disable_deinterlace=disable_deinterlace,
                    ),
                    "base_plan": _native_plan_payload(base_plan),
                    "total_memory_gib": total_memory_gib,
                    "max_candidates": max_candidates,
                    "throughput_tolerance_percent": throughput_tolerance_percent,
                    "hdr10plus_enabled": hdr10plus_enabled,
                    "encoder_fingerprint": encoder_fingerprint,
                },
            )
            key = native.get("key") if native is not None else None
            fingerprint = native.get("encoder_fingerprint") if native is not None else None
            if (
                isinstance(key, str)
                and len(key) == 32
                and all(character in "0123456789abcdef" for character in key)
                and isinstance(fingerprint, str)
                and len(fingerprint) == 64
                and all(character in "0123456789abcdef" for character in fingerprint)
            ):
                _NATIVE_ENCODER_FINGERPRINTS[tools.ffmpeg] = fingerprint
                return key
            raise RebuildError("The Rust calibration-key helper returned an invalid workload key.")
        except RebuildError as exc:
            logging.warning("Rust calibration-key generation failed; using the compatibility key: %s", exc)

    payload = {
        "encoder": _encoder_fingerprint(tools.ffmpeg),
        "cpu": _cpu_identity(),
        "cpu_threads": int(base_plan.cpu_threads),
        "memory_gib": round(float(total_memory_gib or 0.0), 1),
        "base_workers": base_plan.worker_threads,
        "base_frames": base_plan.frame_threads,
        "memory_guard_gib": round(base_plan.memory_guard_gib, 2),
        "resolution": _resolution_class(profile),
        "bit_depth": int(profile.source_bit_depth),
        "pix_fmt": profile.target_pix_fmt,
        "chroma": profile.source_chroma,
        "preset": preset,
        "tune": tune or "none",
        "crf": int(crf),
        "sample_seconds": round(float(sample_seconds), 2),
        "max_candidates": int(max_candidates),
        "throughput_tolerance_percent": round(float(throughput_tolerance_percent), 2),
        "hdr10plus_enabled": bool(hdr10plus_enabled),
        "hdr10": bool(profile.hdr10),
        "hdr10plus": bool(profile.hdr10plus),
        "hlg": bool(profile.hlg),
        "dolby_vision": [bool(profile.dolby_vision), profile.dovi_profile, profile.dovi_rpu_present],
        "interlaced": bool(profile.interlaced),
        "preserve_source_level": bool(preserve_source_level),
        "disable_hdr10_opt": bool(disable_hdr10_opt),
        "force_deinterlace": bool(force_deinterlace),
        "disable_deinterlace": bool(disable_deinterlace),
        "preserve_dolbyvision": bool(preserve_dolbyvision),
        "strip_dolbyvision": bool(strip_dolbyvision),
    }
    encoded = json.dumps(payload, sort_keys=True, separators=(",", ":")).encode("utf-8")
    return hashlib.sha256(encoded).hexdigest()[:32]


def _load_cache() -> dict[str, Any]:
    try:
        value = json.loads(cache_path().read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return {}
    return value if isinstance(value, dict) else {}


def _with_threads(base: ResourcePlan, workers: int, frames: int, reason: str) -> ResourcePlan:
    forecast = base.memory_forecast
    if forecast is not None:
        modeled = frames * forecast.per_frame_allowance_gib + workers * forecast.per_worker_allowance_gib
        forecast = replace(
            forecast,
            modeled_plan_gib=round(modeled, 3),
            planned_workers=workers,
            planned_frame_threads=frames,
            messages=forecast.messages + (reason,),
        )
    return replace(
        base,
        worker_threads=workers,
        frame_threads=frames,
        reason=reason,
        memory_forecast=forecast,
    )


def cached_plan(
    key: str,
    base_plan: ResourcePlan,
    *,
    cache_hours: float,
    now: float | None = None,
) -> CalibrationSelection | None:
    try:
        native = _native_calibration_action(
            "cache",
            {
                "key": key,
                "cache_hours": cache_hours,
                "base_worker_threads": int(base_plan.worker_threads or max(1, base_plan.cpu_threads - 1)),
                "base_frame_threads": int(base_plan.frame_threads or 1),
            },
        )
    except RebuildError as exc:
        logging.warning("Rust calibration-cache lookup failed; using the compatibility cache: %s", exc)
        native = None
    if native is not None:
        if native.get("found") is False:
            return None
        workers = native.get("worker_threads")
        frames = native.get("frame_threads")
        age_hours = native.get("age_hours")
        if (
            native.get("found") is not True
            or not isinstance(workers, int)
            or isinstance(workers, bool)
            or not isinstance(frames, int)
            or isinstance(frames, bool)
            or not isinstance(age_hours, (int, float))
            or isinstance(age_hours, bool)
            or not math.isfinite(age_hours)
        ):
            raise RebuildError("The Rust calibration-cache helper returned invalid cached values.")
        reason = f"cached hardware calibration ({age_hours:.1f}h old)"
        return CalibrationSelection(
            plan=_with_threads(base_plan, workers, frames, reason),
            source="cache",
            age_hours=float(age_hours),
        )

    data = _load_cache()
    entries = data.get("entries")
    entry = entries.get(key) if isinstance(entries, dict) else None
    if not isinstance(entry, dict):
        return None
    timestamp = entry.get("timestamp")
    workers = entry.get("worker_threads")
    frames = entry.get("frame_threads")
    if (
        not isinstance(timestamp, (int, float))
        or isinstance(timestamp, bool)
        or not isinstance(workers, int)
        or isinstance(workers, bool)
        or not isinstance(frames, int)
        or isinstance(frames, bool)
    ):
        return None
    age_hours = ((now if now is not None else time.time()) - float(timestamp)) / 3600.0
    if age_hours < 0 or age_hours > max(0.0, cache_hours):
        return None
    base_workers = int(base_plan.worker_threads or max(1, base_plan.cpu_threads - 1))
    base_frames = int(base_plan.frame_threads or 1)
    if workers < 1 or frames < 1 or workers > base_workers or frames > base_frames:
        return None
    reason = f"cached hardware calibration ({age_hours:.1f}h old)"
    plan = _with_threads(base_plan, workers, frames, reason)
    return CalibrationSelection(plan=plan, source="cache", age_hours=age_hours)


def _candidate_plans(base: ResourcePlan, *, max_candidates: int) -> list[ResourcePlan]:
    workers = int(base.worker_threads or max(1, base.cpu_threads - 1))
    frames = int(base.frame_threads or 1)
    candidates: list[tuple[int, int]] = [(workers, frames)]
    if frames > 1:
        candidates.append((workers, 1))
    if workers > 2:
        candidates.append((max(2, int(round(workers * 0.80))), 1))
        candidates.append((max(2, int(round(workers * 0.60))), 1))
    unique: list[ResourcePlan] = []
    seen: set[tuple[int, int]] = set()
    for worker_count, frame_count in candidates:
        pair = (worker_count, frame_count)
        if pair in seen:
            continue
        seen.add(pair)
        unique.append(_with_threads(base, worker_count, frame_count, "hardware calibration candidate"))
        if len(unique) >= max(1, max_candidates):
            break
    return unique


def choose_best_result(
    results: list[CalibrationResult],
    *,
    throughput_tolerance_percent: float = 3.0,
) -> CalibrationResult | None:
    healthy = [result for result in results if result.healthy and result.speed_x > 0]
    if not healthy:
        return None
    fastest = max(result.speed_x for result in healthy)
    threshold = fastest * (1.0 - max(0.0, throughput_tolerance_percent) / 100.0)
    near = [result for result in healthy if result.speed_x >= threshold]

    def resource_key(result: CalibrationResult) -> tuple[float, float, int, int, float]:
        peak = result.peak_rss_gib if result.peak_rss_gib is not None else float("inf")
        min_available = result.min_available_gib if result.min_available_gib is not None else -float("inf")
        return (peak, -min_available, result.frame_threads, result.worker_threads, -result.speed_x)

    return min(near, key=resource_key)


def _benchmark_args(
    tools: Toolchain,
    profile: MediaProfile,
    *,
    preset: str,
    tune: str | None,
    crf: int,
    plan: ResourcePlan,
    sample_seconds: float,
    hdr10plus_json: Path | None,
    preserve_dolbyvision: bool,
    strip_dolbyvision: bool,
    preserve_source_level: bool,
    disable_hdr10_opt: bool,
    force_deinterlace: bool,
    disable_deinterlace: bool,
) -> list[str]:
    duration = max(0.0, float(profile.duration or 0.0))
    sample = min(max(1.0, sample_seconds), max(1.0, duration - 0.5)) if duration else max(1.0, sample_seconds)
    seek = 0.0
    if duration > sample + 5.0:
        seek = min(duration * 0.25, max(0.0, duration - sample - 2.0))

    args = [tools.ffmpeg, "-hide_banner", "-nostdin", "-y", "-loglevel", "error", "-nostats"]
    if seek > 0:
        args += ["-ss", f"{seek:.3f}"]
    args += ["-i", str(profile.path), "-t", f"{sample:.3f}", "-map", f"0:{profile.primary_video.index}"]
    args += ["-an", "-sn", "-dn", "-c:v:0", "libx265", "-pix_fmt:v:0", profile.target_pix_fmt]
    args += ["-preset:v:0", preset, "-crf:v:0", str(crf), "-fps_mode:v:0", "passthrough"]
    if tune and tune != "none":
        args += ["-tune:v:0", tune]
    if preserve_dolbyvision:
        args += ["-dolbyvision:v:0", "1"]
    elif strip_dolbyvision and tools.x265_dolbyvision:
        args += ["-dolbyvision:v:0", "0"]
    deinterlace = force_deinterlace or (profile.interlaced and not disable_deinterlace)
    if deinterlace:
        args += ["-filter:v:0", "bwdif=mode=send_frame:parity=auto:deint=interlaced"]
    if profile.color_primaries:
        args += ["-color_primaries:v:0", profile.color_primaries]
    if profile.transfer:
        args += ["-color_trc:v:0", profile.transfer]
    if profile.color_matrix:
        args += ["-colorspace:v:0", profile.color_matrix]
    if profile.color_range_ffmpeg:
        args += ["-color_range:v:0", profile.color_range_ffmpeg]
    params = x265_params(
        profile,
        hdr10plus_json,
        preserve_source_level=preserve_source_level,
        disable_hdr10_opt=disable_hdr10_opt,
    )
    if plan.worker_threads:
        params.append(f"pools={plan.worker_threads}")
    if plan.frame_threads:
        params.append(f"frame-threads={plan.frame_threads}")
    if params:
        args += ["-x265-params:v:0", ":".join(params)]
    args += ["-f", "null", "-"]
    return args


def _terminate_candidate(proc: subprocess.Popen[str]) -> None:
    if proc.poll() is not None:
        return
    try:
        if os.name == "posix":
            os.killpg(proc.pid, signal.SIGTERM)
        else:
            proc.terminate()
    except OSError:
        pass
    try:
        proc.wait(timeout=4)
    except subprocess.TimeoutExpired:
        try:
            if os.name == "posix":
                os.killpg(proc.pid, signal.SIGKILL)
            else:
                proc.kill()
        except OSError:
            pass
        try:
            proc.wait(timeout=4)
        except subprocess.TimeoutExpired:
            logging.error("Could not stop a hardware calibration FFmpeg process promptly.")


def benchmark_candidate(
    tools: Toolchain,
    profile: MediaProfile,
    *,
    preset: str,
    tune: str | None,
    crf: int,
    plan: ResourcePlan,
    sample_seconds: float,
    hdr10plus_json: Path | None,
    preserve_dolbyvision: bool,
    strip_dolbyvision: bool,
    preserve_source_level: bool,
    disable_hdr10_opt: bool,
    force_deinterlace: bool,
    disable_deinterlace: bool,
) -> CalibrationResult:
    media_seconds = min(max(1.0, sample_seconds), max(1.0, float(profile.duration or sample_seconds)))
    workers = int(plan.worker_threads or 0)
    frames = int(plan.frame_threads or 0)
    try:
        args = _benchmark_args(
            tools,
            profile,
            preset=preset,
            tune=tune,
            crf=crf,
            plan=plan,
            sample_seconds=sample_seconds,
            hdr10plus_json=hdr10plus_json,
            preserve_dolbyvision=preserve_dolbyvision,
            strip_dolbyvision=strip_dolbyvision,
            preserve_source_level=preserve_source_level,
            disable_hdr10_opt=disable_hdr10_opt,
            force_deinterlace=force_deinterlace,
            disable_deinterlace=disable_deinterlace,
        )
    except (RebuildError, OSError, ValueError) as exc:
        return CalibrationResult(workers, frames, 0.0, media_seconds, 0.0, None, None, None, None, None, False, str(exc))

    logging.info("Hardware calibration command: %s", format_command(args))
    started = time.monotonic()
    try:
        proc = subprocess.Popen(
            args,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
            text=True,
            start_new_session=(os.name == "posix"),
        )
    except (OSError, subprocess.SubprocessError) as exc:
        return CalibrationResult(workers, frames, 0.0, media_seconds, 0.0, None, None, None, None, None, False, f"Could not start calibration: {exc}")

    stderr_lines = DiagnosticTail()
    drain = start_stderr_drain(proc.stderr, stderr_lines)
    swap_sampler = SwapSampler()
    peak_rss: float | None = None
    min_available: float | None = None
    max_psi: float | None = None
    max_swap_in: float | None = None
    max_swap_out: float | None = None
    low_memory_samples = 0
    swap_pressure_samples = 0
    severe_psi_samples = 0
    healthy = True
    reason = ""
    try:
        while proc.poll() is None:
            snap = snapshot(profile.path.parent, pid=proc.pid, swap_sampler=swap_sampler)
            available = snap.memory.available_gib
            psi = snap.pressure.memory_full_avg10
            swap_in = snap.swap_activity.in_bytes_per_second if snap.swap_activity else None
            swap_out = snap.swap_activity.out_bytes_per_second if snap.swap_activity else None
            if snap.process_rss_gib is not None:
                peak_rss = max(peak_rss or 0.0, snap.process_rss_gib)
            if available is not None:
                min_available = available if min_available is None else min(min_available, available)
                low_memory_samples = low_memory_samples + 1 if plan.memory_guard_gib > 0 and available < plan.memory_guard_gib else 0
            if psi is not None:
                max_psi = max(max_psi or 0.0, psi)
                severe_psi_samples = severe_psi_samples + 1 if psi >= 20.0 else 0
            if swap_in is not None:
                max_swap_in = max(max_swap_in or 0.0, swap_in)
            if swap_out is not None:
                max_swap_out = max(max_swap_out or 0.0, swap_out)
            if (
                swap_in is not None
                and swap_in >= SWAP_IN_PRESSURE_BPS
                and psi is not None
                and psi >= MEMORY_FULL_PRESSURE_PERCENT
            ):
                swap_pressure_samples += 1
            else:
                swap_pressure_samples = 0
            if low_memory_samples >= 3:
                healthy = False
                reason = f"available RAM stayed below the {plan.memory_guard_gib:.1f} GiB reserve"
            elif swap_pressure_samples >= SWAP_PRESSURE_MIN_SAMPLES:
                healthy = False
                reason = "sustained swap-in and memory PSI pressure"
            elif severe_psi_samples >= 3:
                healthy = False
                reason = "severe full-memory PSI pressure"
            if not healthy:
                _terminate_candidate(proc)
                break
            time.sleep(0.5)
        try:
            return_code = proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            _terminate_candidate(proc)
            return_code = proc.poll()
        if return_code != 0:
            healthy = False
            diagnostic = stderr_lines.text(3000).strip()
            reason = reason or f"FFmpeg calibration exited {return_code}" + (f": {diagnostic}" if diagnostic else "")
    finally:
        if proc.poll() is None:
            _terminate_candidate(proc)
        drain.join(timeout=2)
        if proc.stderr:
            proc.stderr.close()

    elapsed = max(0.001, time.monotonic() - started)
    if max_psi is not None and max_psi >= 20.0:
        healthy = False
        reason = reason or f"full-memory PSI peaked at {max_psi:.1f}%"
    if max_swap_out is not None and max_swap_out >= 64.0 * MIB:
        healthy = False
        reason = reason or f"heavy swap-out peaked at {max_swap_out / MIB:.0f} MiB/s"
    speed_x = media_seconds / elapsed if healthy else 0.0
    return CalibrationResult(
        worker_threads=workers,
        frame_threads=frames,
        elapsed_seconds=elapsed,
        media_seconds=media_seconds,
        speed_x=speed_x,
        peak_rss_gib=peak_rss,
        min_available_gib=min_available,
        max_memory_psi_full=max_psi,
        max_swap_in_mib_s=max_swap_in / MIB if max_swap_in is not None else None,
        max_swap_out_mib_s=max_swap_out / MIB if max_swap_out is not None else None,
        healthy=healthy,
        reason=reason,
    )


def run_calibration(
    tools: Toolchain,
    profile: MediaProfile,
    base_plan: ResourcePlan,
    *,
    preset: str,
    tune: str | None,
    crf: int,
    sample_seconds: float,
    max_candidates: int,
    throughput_tolerance_percent: float,
    hdr10plus_json: Path | None,
    preserve_dolbyvision: bool,
    strip_dolbyvision: bool,
    preserve_source_level: bool,
    disable_hdr10_opt: bool,
    force_deinterlace: bool,
    disable_deinterlace: bool,
    cache_key: str | None = None,
) -> CalibrationSelection:
    if os.environ.get("HEVC_REBUILD_NATIVE_HELPER") and cache_key:
        try:
            native = _native_calibration_action(
                "run",
                {
                    "key": cache_key,
                    "max_candidates": max_candidates,
                    "throughput_tolerance_percent": throughput_tolerance_percent,
                    "memory_guard_gib": base_plan.memory_guard_gib,
                    "profile": _native_profile_payload(profile),
                    "toolchain": _native_toolchain_payload(tools),
                    "options": _native_options_payload(
                        preset=preset,
                        tune=tune,
                        crf=crf,
                        sample_seconds=sample_seconds,
                        hdr10plus_json=hdr10plus_json,
                        preserve_dolbyvision=preserve_dolbyvision,
                        strip_dolbyvision=strip_dolbyvision,
                        preserve_source_level=preserve_source_level,
                        disable_hdr10_opt=disable_hdr10_opt,
                        force_deinterlace=force_deinterlace,
                        disable_deinterlace=disable_deinterlace,
                    ),
                    "base_plan": _native_plan_payload(base_plan),
                },
            )
            if native is not None:
                results = _native_results(native.get("results", []))
                source = native.get("source")
                reason = native.get("reason")
                if not isinstance(reason, str):
                    raise RebuildError("The Rust calibration helper returned an invalid selection reason.")
                if source == "base":
                    return CalibrationSelection(base_plan, "base", results=results, reason=reason)
                workers = native.get("worker_threads")
                frames = native.get("frame_threads")
                if (
                    source != "benchmark"
                    or not isinstance(workers, int)
                    or isinstance(workers, bool)
                    or not isinstance(frames, int)
                    or isinstance(frames, bool)
                ):
                    raise RebuildError("The Rust calibration helper returned an invalid plan selection.")
                return CalibrationSelection(
                    plan=_with_threads(base_plan, workers, frames, reason),
                    source="benchmark",
                    results=results,
                )
        except RebuildError as exc:
            logging.warning("Rust hardware calibration failed; using the compatibility path: %s", exc)

    if memory_stats().available_gib is None:
        return CalibrationSelection(base_plan, "base", reason="Memory telemetry is unavailable; calibration was skipped.")
    results: list[CalibrationResult] = []
    for plan in _candidate_plans(base_plan, max_candidates=max_candidates):
        try:
            result = benchmark_candidate(
                tools,
                profile,
                preset=preset,
                tune=tune,
                crf=crf,
                plan=plan,
                sample_seconds=sample_seconds,
                hdr10plus_json=hdr10plus_json,
                preserve_dolbyvision=preserve_dolbyvision,
                strip_dolbyvision=strip_dolbyvision,
                preserve_source_level=preserve_source_level,
                disable_hdr10_opt=disable_hdr10_opt,
                force_deinterlace=force_deinterlace,
                disable_deinterlace=disable_deinterlace,
            )
        except Exception as exc:
            logging.warning("Hardware calibration candidate failed; keeping the normal resource plan: %s", exc)
            result = CalibrationResult(
                int(plan.worker_threads or 0),
                int(plan.frame_threads or 0),
                0.0,
                max(1.0, sample_seconds),
                0.0,
                None,
                None,
                None,
                None,
                None,
                False,
                f"Calibration candidate failed: {type(exc).__name__}: {exc}",
            )
        results.append(result)
    best = choose_best_result(results, throughput_tolerance_percent=throughput_tolerance_percent)
    if best is None:
        return CalibrationSelection(
            base_plan,
            "base",
            results=tuple(results),
            reason="No calibration candidate completed with acceptable resource pressure; using the normal plan.",
        )
    reason = "selected by quick hardware calibration; live resource guards remain active"
    plan = _with_threads(base_plan, best.worker_threads, best.frame_threads, reason)
    return CalibrationSelection(plan=plan, source="benchmark", results=tuple(results))


def store_selection(key: str, selection: CalibrationSelection) -> None:
    if selection.source != "benchmark":
        return
    if os.environ.get("HEVC_REBUILD_NATIVE_HELPER"):
        try:
            _native_calibration_action(
                "store",
                {
                    "key": key,
                    "worker_threads": int(selection.plan.worker_threads or 1),
                    "frame_threads": int(selection.plan.frame_threads or 1),
                    "results": [asdict(result) for result in selection.results],
                },
            )
            return
        except RebuildError as exc:
            logging.warning("Rust could not store the hardware calibration cache; using the compatibility cache: %s", exc)
    path = cache_path()
    data = _load_cache()
    entries = data.get("entries")
    if not isinstance(entries, dict):
        entries = {}
    now = time.time()
    entries = {
        entry_key: entry
        for entry_key, entry in entries.items()
        if isinstance(entry, dict)
        and isinstance(entry.get("timestamp"), (int, float))
        and 0 <= (now - float(entry["timestamp"])) / 3600.0 <= MAX_CACHE_AGE_HOURS
    }
    entries[key] = {
        "timestamp": now,
        "worker_threads": int(selection.plan.worker_threads or 1),
        "frame_threads": int(selection.plan.frame_threads or 1),
        "results": [asdict(result) for result in selection.results],
    }
    if len(entries) > MAX_CACHE_ENTRIES:
        entries = dict(sorted(entries.items(), key=lambda item: float(item[1]["timestamp"]), reverse=True)[:MAX_CACHE_ENTRIES])
    data["schema"] = 1
    data["entries"] = entries
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    try:
        path.parent.chmod(0o700)
    except OSError:
        pass
    file_descriptor, temporary_name = tempfile.mkstemp(prefix="calibration-", suffix=".tmp", dir=path.parent)
    temporary = Path(temporary_name)
    try:
        with os.fdopen(file_descriptor, "w", encoding="utf-8") as stream:
            if hasattr(os, "fchmod"):
                os.fchmod(stream.fileno(), 0o600)
            json.dump(data, stream, indent=2, sort_keys=True)
            stream.write("\n")
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    finally:
        try:
            temporary.unlink()
        except FileNotFoundError:
            pass
