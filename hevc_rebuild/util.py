from __future__ import annotations

import csv
import hashlib
import json
import logging
import os
import shlex
import shutil
import subprocess
import sys
import threading
import time
from dataclasses import asdict, is_dataclass
from pathlib import Path
from typing import Any, Iterable


class RebuildError(RuntimeError):
    pass


class UserAbort(RebuildError):
    pass


def portable_data_dir() -> Path | None:
    """Return the writable data folder beside the running AppImage, if enabled."""
    value = os.environ.get("HEVC_REBUILD_PORTABLE_DATA_DIR")
    return Path(value).expanduser() if value else None


def user_log_directory() -> Path:
    portable = portable_data_dir()
    if portable is not None:
        return portable / "logs"
    state_home = Path(os.environ.get("XDG_STATE_HOME", Path.home() / ".local" / "state"))
    return state_home / "hevc-rebuild" / "logs"


def app_preferences_path() -> Path:
    portable = portable_data_dir()
    if portable is not None:
        return portable / "settings.json"
    config_home = Path(os.environ.get("XDG_CONFIG_HOME", Path.home() / ".config"))
    return config_home / "hevc-rebuild-studio" / "settings.json"


class ExternalCommandError(RebuildError):
    def __init__(self, command: list[str], returncode: int, stderr: str = "") -> None:
        super().__init__(f"Command failed ({returncode}): {format_command(command)}\n{stderr.strip()}")
        self.command = command
        self.returncode = returncode
        self.stderr = stderr


def format_command(args: Iterable[str]) -> str:
    return " ".join(shlex.quote(str(a)) for a in args)


def run(
    args: list[str],
    *,
    check: bool = True,
    capture: bool = True,
    text: bool = True,
    input_text: str | None = None,
    timeout: float | None = None,
    env: dict[str, str] | None = None,
) -> subprocess.CompletedProcess:
    logging.debug("exec: %s", format_command(args))
    cp = subprocess.run(
        args,
        input=input_text,
        stdout=subprocess.PIPE if capture else None,
        stderr=subprocess.PIPE if capture else None,
        text=text,
        timeout=timeout,
        env=env,
    )
    if check and cp.returncode != 0:
        raise ExternalCommandError(args, cp.returncode, cp.stderr or "")
    return cp


def run_json(args: list[str], *, timeout: float | None = None) -> dict[str, Any]:
    cp = run(args, timeout=timeout)
    try:
        return json.loads(cp.stdout)
    except json.JSONDecodeError as exc:
        raise RebuildError(f"Invalid JSON from {args[0]}: {exc}") from exc


def which(name: str) -> str | None:
    return shutil.which(name)


def prompt_enter(message: str, *, allow_skip: bool = False) -> str:
    if not sys.stdin.isatty():
        raise RebuildError(f"Interactive action required but stdin is not a TTY: {message}")
    while True:
        suffix = " [ENTER=continue, S=skip, Q=quit] " if allow_skip else " [ENTER=continue, Q=quit] "
        value = input(message + suffix).strip().lower()
        if value == "":
            return "continue"
        if value == "q":
            raise UserAbort("User aborted.")
        if allow_skip and value == "s":
            return "skip"
        print("Please press ENTER, S, or Q as shown.")


def human_bytes(value: int | float) -> str:
    size = float(value)
    for unit in ("B", "KiB", "MiB", "GiB", "TiB"):
        if abs(size) < 1024.0 or unit == "TiB":
            return f"{size:.2f} {unit}"
        size /= 1024.0
    return f"{size:.2f} TiB"


def duration_text(seconds: float | None) -> str:
    if seconds is None or seconds < 0:
        return "?"
    seconds = int(round(seconds))
    h, rem = divmod(seconds, 3600)
    m, s = divmod(rem, 60)
    return f"{h:02d}:{m:02d}:{s:02d}"


def safe_float(value: Any) -> float | None:
    if value is None:
        return None
    try:
        if isinstance(value, str) and "/" in value:
            a, b = value.split("/", 1)
            b_f = float(b)
            return float(a) / b_f if b_f else None
        return float(str(value).strip())
    except (TypeError, ValueError, ZeroDivisionError):
        return None


def safe_int(value: Any) -> int | None:
    if value is None:
        return None
    try:
        return int(str(value).strip())
    except (TypeError, ValueError):
        return None


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def fsync_file(path: Path) -> None:
    with path.open("rb") as f:
        os.fsync(f.fileno())


def fsync_dir(path: Path) -> None:
    try:
        fd = os.open(path, os.O_DIRECTORY)
    except (AttributeError, OSError):
        return
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def unique_backup(path: Path) -> Path:
    if not path.exists():
        return path
    stem, suffix = path.stem, path.suffix
    for n in range(1, 10000):
        candidate = path.with_name(f"{stem}.{n}{suffix}")
        if not candidate.exists():
            return candidate
    raise RebuildError(f"Unable to allocate a unique backup path under {path.parent}")


def configure_logging(log_dir: Path, verbose: bool = False) -> tuple[Path, Path]:
    log_dir.mkdir(parents=True, exist_ok=True)
    stamp = time.strftime("%Y%m%d-%H%M%S")
    log_path = log_dir / f"hevc-rebuild-{stamp}.log"
    csv_path = log_dir / f"hevc-rebuild-{stamp}.csv"
    root = logging.getLogger()
    root.handlers.clear()
    root.setLevel(logging.DEBUG)
    file_handler = logging.FileHandler(log_path, encoding="utf-8")
    file_handler.setLevel(logging.DEBUG)
    file_handler.setFormatter(logging.Formatter("%(asctime)s %(levelname)s %(message)s"))
    root.addHandler(file_handler)
    console = logging.StreamHandler(sys.stderr)
    console.setLevel(logging.DEBUG if verbose else logging.WARNING)
    console.setFormatter(logging.Formatter("%(levelname)s: %(message)s"))
    root.addHandler(console)
    return log_path, csv_path


def append_csv(csv_path: Path, row: dict[str, Any]) -> None:
    fields = [
        "input", "output", "status", "message", "primary_video", "resolution",
        "audio_count", "subtitle_count", "hdr10", "hdr10plus", "dolby_vision",
        "dovi_profile", "dovi_source", "dolby_vision_action",
        "memory_forecast", "memory_available_gib", "memory_total_gib", "memory_swap_used_percent", "memory_budget_gib",
        "memory_worker_cap", "planned_workers", "planned_frame_threads", "memory_forecast_message",
        "calibration_source", "calibration_workers", "calibration_frame_threads", "calibration_candidate_count", "calibration_note",
        "min_memory_available_gib", "max_encoder_rss_gib", "max_swap_used_percent", "max_encoder_swap_gib", "max_swap_in_mib_s", "max_swap_out_mib_s", "max_memory_full_psi_percent",
        "input_bytes", "output_bytes", "elapsed_seconds",
    ]
    exists = csv_path.exists()
    with csv_path.open("a", newline="", encoding="utf-8") as f:
        w = csv.DictWriter(f, fieldnames=fields, extrasaction="ignore")
        if not exists:
            w.writeheader()
        w.writerow(row)


def dataclass_json(value: Any) -> str:
    if is_dataclass(value):
        value = asdict(value)
    return json.dumps(value, indent=2, default=str, ensure_ascii=False)


def drain_pipe(pipe, sink: list[str]) -> None:
    if pipe is None:
        return
    for line in iter(pipe.readline, ""):
        sink.append(line)
        logging.debug("child-stderr: %s", line.rstrip())
    pipe.close()


def start_stderr_drain(pipe, sink: list[str]) -> threading.Thread:
    t = threading.Thread(target=drain_pipe, args=(pipe, sink), daemon=True)
    t.start()
    return t
