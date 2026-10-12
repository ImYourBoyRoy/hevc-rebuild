from __future__ import annotations

import os
import json
import math
import shutil
import sys
import time
from dataclasses import dataclass

from .model import MediaProfile
from .resources import ResourceSnapshot
from .util import ConsoleClosed, duration_text, human_bytes


_USE_COLOR = sys.stderr.isatty() and not os.environ.get("NO_COLOR")
_GUI_PROGRESS_PREFIX = "#HEVC_REBUILD_PROGRESS\t"


def _c(code: str, text: str) -> str:
    return f"\x1b[{code}m{text}\x1b[0m" if _USE_COLOR else text


def _write_stderr(text: str) -> None:
    """Write one progress line. A closed desktop window stops the encode cleanly."""
    try:
        print(text, file=sys.stderr, flush=True)
    except BrokenPipeError as exc:
        raise ConsoleClosed(
            "The desktop app closed while this file was encoding. The original was not changed."
        ) from exc


def _clip(text: str, width: int) -> str:
    if width <= 1:
        return text[:width]
    return text if len(text) <= width else text[: max(0, width - 1)] + "…"


def _hdr_label(profile: MediaProfile) -> str:
    labels: list[str] = []
    if profile.dolby_vision:
        labels.append(f"Dolby Vision P{profile.dovi_profile or '?'}")
    if profile.hdr10plus:
        labels.append("HDR10+")
    elif profile.hdr10:
        labels.append("HDR10")
    if profile.hlg:
        labels.append("HLG")
    return " + ".join(labels) if labels else "SDR"


def _box(title: str, rows: list[tuple[str, str]], *, width: int | None = None) -> None:
    width = width or max(76, min(118, shutil.get_terminal_size((100, 24)).columns))
    width = max(60, width)
    inner = width - 4
    title_text = f" {title} "
    bar = "─" * max(2, width - len(title_text) - 2)
    print(_c("1;36", "┌" + title_text + bar + "┐"), file=sys.stderr)
    label_width = max(7, min(10, max((len(k) for k, _ in rows), default=8)))
    for key, value in rows:
        label = _c("1", f"{key:<{label_width}}")
        text = _clip(value, inner - label_width - 1)
        print(f"│ {label} {text:<{inner-label_width}} │", file=sys.stderr)
    print(_c("1;36", "└" + "─" * (width - 2) + "┘"), file=sys.stderr)


def print_batch_card(
    *,
    total_files: int,
    total_input_bytes: int,
    disk_free_bytes: int | None,
    disk_total_bytes: int | None,
    cpu_threads: int,
    reserved_cpu_threads: int,
    mem_total_gib: float | None,
    mem_available_gib: float | None,
    memory_guard_gib: float,
    disk_guard_gib: float,
    mode: str,
) -> None:
    storage = "unknown"
    if disk_free_bytes is not None and disk_total_bytes is not None:
        storage = f"{human_bytes(disk_free_bytes)} free of {human_bytes(disk_total_bytes)}"
    memory = "unknown"
    if mem_available_gib is not None:
        memory = f"{mem_available_gib:.1f} GiB available"
        if mem_total_gib is not None:
            memory += f" / {mem_total_gib:.1f} GiB total"
    rows = [
        ("Queue", f"{total_files} file(s) • {human_bytes(total_input_bytes)} source media • one file at a time"),
        ("CPU", f"{cpu_threads} logical threads • auto uses up to {max(1, cpu_threads - max(0, reserved_cpu_threads))} for x265"),
        ("Memory", f"{memory} • dynamic reserve {memory_guard_gib:.1f} GiB"),
        ("Storage", f"{storage} • keep at least {disk_guard_gib:.1f} GiB free"),
        ("Safety", "Originals stay in place until each rebuilt file passes verification"),
    ]
    _box(f"HEVC Rebuild • {mode}", rows)


def emit_gui_progress(
    *,
    file_name: str,
    file_path: str | None = None,
    phase: str,
    batch_number: int,
    batch_total: int,
    file_percent: float | None,
    queue_percent: float | None,
    speed: float | None = None,
    eta_seconds: float | None = None,
    elapsed_seconds: float | None = None,
    input_bytes: int | None = None,
    output_bytes: int | None = None,
    estimated_output_bytes: int | None = None,
) -> None:
    """Send a bounded machine-readable progress record to the Tauri host."""
    if os.environ.get("HEVC_REBUILD_GUI_EVENTS") != "1":
        return
    def finite(value: float | None, *, minimum: float = 0.0, maximum: float | None = None) -> float | None:
        if value is None or not math.isfinite(value):
            return None
        result = max(minimum, float(value))
        return min(maximum, result) if maximum is not None else result

    payload = {
        "fileName": file_name[:512],
        "filePath": file_path[:8192] if file_path else None,
        "phase": phase[:96],
        "batchNumber": max(1, int(batch_number)),
        "batchTotal": max(1, int(batch_total)),
        "filePercent": finite(file_percent, maximum=100.0),
        "queuePercent": finite(queue_percent, maximum=100.0),
        "speed": finite(speed),
        "etaSeconds": finite(eta_seconds),
        "elapsedSeconds": finite(elapsed_seconds),
        "inputBytes": max(0, int(input_bytes)) if input_bytes is not None else None,
        "outputBytes": max(0, int(output_bytes)) if output_bytes is not None else None,
        "estimatedOutputBytes": max(0, int(estimated_output_bytes)) if estimated_output_bytes is not None else None,
    }
    _write_stderr(
        _GUI_PROGRESS_PREFIX + json.dumps(payload, ensure_ascii=False, separators=(",", ":"), allow_nan=False)
    )


def print_encode_card(
    profile: MediaProfile,
    *,
    number: int,
    total: int,
    crf: int,
    preset: str,
    tune: str | None,
    worker_threads: int | None,
    frame_threads: int | None,
    cpu_threads: int,
    mem_available_gib: float | None,
    mem_total_gib: float | None,
    memory_guard_gib: float,
    memory_forecast_level: str,
    memory_forecast_budget_gib: float | None,
    disk_free_bytes: int | None,
    disk_required_bytes: int | None,
    disk_guard_gib: float,
    test_mode: bool,
) -> None:
    mode = "SHORT TEST" if test_mode else "FULL REBUILD"
    disk = "unknown"
    if disk_free_bytes is not None:
        disk = f"{human_bytes(disk_free_bytes)} free"
        if disk_required_bytes is not None:
            disk += f" • conservative budget {human_bytes(disk_required_bytes)}"
        disk += f" • reserve {disk_guard_gib:.1f} GiB"
    rows = [
        ("Source", profile.path.name),
        (
            "Video",
            f"{profile.primary_video.width}×{profile.primary_video.height} • "
            f"{(profile.primary_video.codec_name or '?').upper()} • "
            f"{profile.source_bit_depth}-bit {profile.source_chroma or '?'} • {_hdr_label(profile)}",
        ),
        (
            "Encode",
            f"x265 • CRF {crf} • {preset} • tune {tune or 'normal'} • {profile.target_pix_fmt}",
        ),
        (
            "Streams",
            f"{len(profile.audio)} audio COPY • {len(profile.subtitles)} subtitles COPY"
            + (f" • {len(profile.secondary_videos)} extra video COPY" if profile.secondary_videos else ""),
        ),
        (
            "CPU",
            f"{worker_threads or 'x265-auto'}/{cpu_threads} workers • {frame_threads or 'x265-auto'} frame threads",
        ),
        (
            "Memory",
            (
                f"{mem_available_gib:.1f} / {mem_total_gib:.1f} GiB available / effective total"
                if mem_available_gib is not None and mem_total_gib is not None
                else f"{mem_available_gib:.1f} GiB available" if mem_available_gib is not None else "available RAM unknown"
            )
            + f" • dynamic guard {memory_guard_gib:.1f} GiB",
        ),
        (
            "Forecast",
            f"{memory_forecast_level.upper()} profile-aware plan"
            + (f" • {memory_forecast_budget_gib:.1f} GiB planning budget" if memory_forecast_budget_gib is not None else "")
            + " • heuristic, not peak-RAM guarantee",
        ),
        ("Storage", disk),
        ("Safety", "Temporary output only; original changes only after full validation"),
    ]
    _box(f"HEVC Rebuild • {mode} • File {number}/{total}", rows)


@dataclass
class ProgressView:
    label: str
    expected_duration: float
    memory_guard_gib: float
    file_path: str | None = None
    elapsed_seconds_before: float = 0.0
    input_size: int = 0
    batch_number: int = 1
    batch_total: int = 1
    enabled: bool = True
    _last_bucket: int = -1
    _last_logged_percent: int = -1
    _last_update_at: float = 0.0
    _drawn_lines: int = 0
    _started_at: float = 0.0
    _last_gui_update_at: float = 0.0

    def __post_init__(self) -> None:
        self.enabled = bool(self.enabled and sys.stderr.isatty())
        self._started_at = time.monotonic()

    def _clear_live(self) -> None:
        if not self.enabled or self._drawn_lines <= 0:
            return
        # Cursor normally sits at the end of the last live line. Clear each
        # line and walk upward so notes/phases never leave stale HUD fragments.
        for idx in range(self._drawn_lines):
            sys.stderr.write("\r\x1b[2K")
            if idx < self._drawn_lines - 1:
                sys.stderr.write("\x1b[1A")
        sys.stderr.write("\r")
        sys.stderr.flush()
        self._drawn_lines = 0

    def render(
        self,
        *,
        done: float,
        speed: float,
        size: int,
        resources: ResourceSnapshot,
        phase: str = "Encoding",
    ) -> None:
        pct = min(100.0, max(0.0, done / self.expected_duration * 100.0)) if self.expected_duration else 0.0
        eta = (self.expected_duration - done) / speed if speed > 0 else None
        # Encoding is most of the job, but metadata writing, validation, and
        # installation still follow. Leave room for those explicit stages.
        queue_pct = ((self.batch_number - 1) + 0.85 * pct / 100.0) / max(1, self.batch_total) * 100.0
        now = time.monotonic()
        if now - self._last_gui_update_at >= 1.0 or pct >= 100.0:
            label = self.label
            if label.startswith("[") and "] " in label:
                label = label.split("] ", 1)[1]
            fraction = done / self.expected_duration if self.expected_duration > 0 else 0.0
            projected = int(max(0, size) / fraction) if fraction >= 0.10 and size > 0 else None
            emit_gui_progress(
                file_name=label,
                file_path=self.file_path,
                phase=phase,
                batch_number=self.batch_number,
                batch_total=self.batch_total,
                file_percent=pct,
                queue_percent=queue_pct,
                speed=speed if speed > 0 else None,
                eta_seconds=max(0.0, eta) if eta is not None else None,
                elapsed_seconds=max(0.0, self.elapsed_seconds_before + now - self._started_at),
                input_bytes=self.input_size,
                output_bytes=max(0, size),
                estimated_output_bytes=projected,
            )
            self._last_gui_update_at = now
        term = max(58, shutil.get_terminal_size((110, 24)).columns - 1)

        # Two compact live lines preserve the useful telemetry on normal
        # terminals without creating a wall of scrolling FFmpeg output.
        bar_width = max(8, min(28, term - 64))
        filled = int(round(bar_width * pct / 100.0))
        bar = "█" * filled + "·" * (bar_width - filled)
        line1 = _clip(
            f"  {phase} {self.batch_number}/{self.batch_total}  [{bar}] {pct:5.1f}%"
            f"  • Queue {queue_pct:5.1f}%  • {speed:4.2f}x  • ETA {duration_text(eta)}",
            term,
        )

        health: list[str] = []
        if self.input_size > 0:
            size_status = f"Input {human_bytes(self.input_size)} · Output {human_bytes(max(0, size))}"
            if pct >= 10.0 and size > 0:
                projected_bytes = int(size / max(0.001, done / self.expected_duration))
                size_status += f" · Live estimate {human_bytes(projected_bytes)}"
            else:
                size_status += " · Live estimate calculating"
            health.append(size_status)
        if resources.cpu_percent is not None:
            health.append(f"CPU {resources.cpu_percent:.0f}%")
        if resources.memory.available_gib is not None:
            health.append(f"RAM {resources.memory.available_gib:.1f}G")
        if resources.memory.swap_used_gib is not None:
            swap = f"Swap {resources.memory.swap_used_gib:.1f}G"
            if resources.memory.swap_total_gib is not None:
                swap += f"/{resources.memory.swap_total_gib:.1f}G"
            if resources.memory.swap_used_percent is not None:
                swap += f" {resources.memory.swap_used_percent:.0f}%"
            health.append(swap)
        if resources.swap_activity is not None:
            swap_in = resources.swap_activity.in_bytes_per_second
            swap_out = resources.swap_activity.out_bytes_per_second
            if swap_in is not None or swap_out is not None:
                incoming = human_bytes(int(swap_in or 0))
                outgoing = human_bytes(int(swap_out or 0))
                health.append(f"Swap I/O {incoming}/{outgoing}/s")
        if resources.process_rss_gib is not None:
            health.append(f"FFmpeg {resources.process_rss_gib:.1f}G")
        if resources.process_swap_gib is not None:
            health.append(f"FFmpeg swap {resources.process_swap_gib:.1f}G")
        if resources.disk is not None:
            health.append(f"Disk {human_bytes(resources.disk.free_bytes)} free")
        if resources.temperature_c is not None:
            health.append(f"Temp {resources.temperature_c:.0f}°C")
        io_full = resources.pressure.io_full_avg10
        if io_full is not None and io_full >= 1.0:
            health.append(f"I/O pressure {io_full:.1f}%")
        line2 = _clip("  " + "  • ".join(health), term)

        if self.enabled:
            try:
                self._clear_live()
                sys.stderr.write(line1 + "\n" + line2)
                sys.stderr.flush()
            except BrokenPipeError as exc:
                raise ConsoleClosed(
                    "The desktop app closed while this file was encoding. The original was not changed."
                ) from exc
            self._drawn_lines = 2
        elif os.environ.get("HEVC_REBUILD_GUI_EVENTS") == "1":
            # The desktop bar, ETA, and resource cards already update from the
            # structured event. The activity log only records percent milestones.
            whole = int(pct)
            if whole != self._last_logged_percent and whole % 5 == 0:
                self._last_logged_percent = whole
                _write_stderr(
                    f"  {phase} {self.batch_number}/{self.batch_total}  {pct:.0f}%"
                    f"  • {speed:.2f}x  • {duration_text(eta)} left"
                )
        else:
            bucket = min(20, int(pct // 5))
            now = time.monotonic()
            if bucket != self._last_bucket or now - self._last_update_at >= 2.0:
                self._last_bucket = bucket
                self._last_update_at = now
                # Logs/pipes receive readable progress and telemetry as separate
                # lines so longer size estimates are not clipped by terminal width.
                _write_stderr(line1)
                if health:
                    _write_stderr("  " + "  •  ".join(health[:6]))

    def note(self, text: str) -> None:
        self._clear_live()
        print(_c("1;33", "  ! " + text), file=sys.stderr)

    def finish(self, text: str = "Encode complete") -> None:
        self._clear_live()
        print(_c("1;32", f"  ✓ {text}"), file=sys.stderr)


def print_phase(text: str) -> None:
    _write_stderr(_c("1;36", f"  • {text}"))


def print_success(text: str) -> None:
    _write_stderr(_c("1;32", f"  ✓ {text}"))


def print_warning(text: str) -> None:
    _write_stderr(_c("1;33", f"  ! {text}"))


def print_batch_summary(*, ok: int, tests: int, info: int, skipped: int, failed: int, elapsed: float, metadata: int = 0) -> None:
    rows = [
        ("Result", f"{ok} rebuilt • {metadata} metadata-only • {tests} test(s) • {info} checked • {skipped} skipped • {failed} failed"),
        ("Elapsed", duration_text(elapsed)),
        ("Logs", "Detailed diagnostics were written to the run log/report"),
    ]
    _box("Run complete", rows)
