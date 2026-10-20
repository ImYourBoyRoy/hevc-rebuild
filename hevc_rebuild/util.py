from __future__ import annotations

import csv
import ctypes
import hashlib
import io
import json
import logging
import math
import os
import platform
import shlex
import shutil
import signal
import stat
import subprocess
import sys
import threading
import time
import uuid
from collections import deque
from dataclasses import asdict, is_dataclass
from pathlib import Path
from typing import Any, Iterable


class RebuildError(RuntimeError):
    pass


class ConsoleClosed(RebuildError):
    """The desktop progress pipe closed, so the encode stops and the original stays."""
    pass


class UserAbort(RebuildError):
    pass


MAX_HELPER_TIMEOUT_SECONDS = 24 * 60 * 60
DEFAULT_INSPECTION_TIMEOUT_SECONDS = 300


class StopRequest:
    """Scope direct CLI markers to this run; native controls use their private job directory."""
    def __init__(self, path: str | Path | None):
        self.path = Path(path) if path else None
        self.since_ns = time.time_ns()
        self.native_parent = None
        if self.path and self.path.parent.name.startswith("hevc-rebuild-control-"):
            parent = self.path.parent.lstat()
            if not stat.S_ISDIR(parent.st_mode) or parent.st_mode & 0o077 or (
                hasattr(os, "geteuid") and parent.st_uid != os.geteuid()
            ):
                raise RebuildError("Stop controls require an owned private directory")
            self.native_parent = (parent.st_dev, parent.st_ino)
            self.since_ns = parent.st_ctime_ns

    def requested(self) -> bool:
        if self.path is None:
            return False
        descriptor = None
        try:
            if self.native_parent is not None:
                parent = self.path.parent.lstat()
                if (parent.st_dev, parent.st_ino) != self.native_parent or not stat.S_ISDIR(parent.st_mode) or parent.st_mode & 0o077:
                    return False
            descriptor = os.open(self.path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0) | getattr(os, "O_NONBLOCK", 0))
            opened = os.fstat(descriptor)
            current = self.path.lstat()
            return (stat.S_ISREG(opened.st_mode) and stat.S_ISREG(current.st_mode)
                    and (opened.st_dev, opened.st_ino) == (current.st_dev, current.st_ino)
                    and (not hasattr(os, "geteuid") or opened.st_uid == os.geteuid())
                    and not opened.st_mode & 0o022
                    and opened.st_ctime_ns >= self.since_ns)
        except OSError:
            return False
        finally:
            if descriptor is not None:
                os.close(descriptor)


def rename_no_replace(source: Path, destination: Path) -> None:
    """Atomically move a path without replacing any destination, including symlinks."""
    src, dst = os.fsencode(source), os.fsencode(destination)
    if b"\0" in src or b"\0" in dst:
        raise ValueError("Transaction paths must not contain NUL bytes")
    if sys.platform.startswith("linux"):
        library = ctypes.CDLL(None, use_errno=True)
        rename = getattr(library, "renameat2", None)
        if rename is None:
            raise RebuildError("Atomic no-overwrite rename is unavailable; original retained")
        rename.argtypes = [ctypes.c_int, ctypes.c_char_p, ctypes.c_int, ctypes.c_char_p, ctypes.c_uint]
        rename.restype = ctypes.c_int
        status = rename(-100, src, -100, dst, 1)  # AT_FDCWD, RENAME_NOREPLACE
    elif sys.platform == "darwin":
        library = ctypes.CDLL(None, use_errno=True)
        rename = library.renamex_np
        rename.argtypes = [ctypes.c_char_p, ctypes.c_char_p, ctypes.c_uint]
        rename.restype = ctypes.c_int
        status = rename(src, dst, 4)  # RENAME_EXCL
    elif os.name == "nt":
        os.rename(source, destination)  # Windows refuses an existing destination.
        return
    else:
        raise RebuildError("Atomic no-overwrite rename is unsupported on this platform")
    if status:
        error = ctypes.get_errno()
        raise OSError(error, os.strerror(error), str(destination))


def terminate_process_group(proc: subprocess.Popen, *, timeout: float = 1.0) -> None:
    """Stop this owned session, including descendants retaining its pipes."""
    def send(sig: int) -> None:
        try:
            if os.name == "posix":
                os.killpg(proc.pid, sig)
            elif proc.poll() is None:
                proc.terminate() if sig == signal.SIGTERM else proc.kill()
        except ProcessLookupError:
            pass
    send(signal.SIGTERM)
    try:
        proc.wait(timeout=timeout)
    except subprocess.TimeoutExpired:
        pass
    # The parent may have exited before a descendant; signal the owned group
    # regardless of parent poll state, then reap the parent.
    send(signal.SIGKILL)
    proc.wait(timeout=timeout)


def native_json_action(
    action: str,
    payload: dict[str, object],
    *,
    timeout: float | None = 15.0,
) -> dict[str, object] | None:
    """Call an optional Rust helper, returning None for direct Python runs.

    A timeout of None selects the bounded full-media helper budget (24 hours).
    The active encoder lifecycle is separate and retains its live safety guards.
    """
    executable = os.environ.get("HEVC_REBUILD_NATIVE_HELPER")
    if not executable:
        return None
    try:
        result = run(
            [executable, "internal", action],
            input_text=json.dumps(payload, separators=(",", ":"), allow_nan=False),
            timeout=MAX_HELPER_TIMEOUT_SECONDS if timeout is None else timeout,
            check=False,
        )
    except (OSError, subprocess.SubprocessError, ValueError) as exc:
        raise RebuildError(f"Could not run the Rust {action} helper: {exc}") from exc
    if result.returncode != 0:
        detail = result.stderr.strip()[-4000:]
        suffix = f" Details: {detail}" if detail else ""
        raise RebuildError(f"The Rust {action} helper exited with status {result.returncode}.{suffix}")
    try:
        value = json.loads(result.stdout)
    except json.JSONDecodeError as exc:
        raise RebuildError(f"The Rust {action} helper returned malformed JSON.") from exc
    if not isinstance(value, dict):
        raise RebuildError(f"The Rust {action} helper returned an invalid response.")
    return value


def portable_data_dir() -> Path | None:
    """Return the writable data folder beside the running AppImage, if enabled."""
    value = os.environ.get("HEVC_REBUILD_PORTABLE_DATA_DIR")
    return Path(value).expanduser() if value else None


def _user_home_directory() -> Path:
    for key in ("USERPROFILE", "HOME"):
        value = os.environ.get(key)
        if value:
            return Path(value).expanduser()
    return Path.home()


def app_config_directory() -> Path:
    """Return the native per-user application configuration directory."""
    portable = portable_data_dir()
    if portable is not None:
        return portable
    home = _user_home_directory()
    system = platform.system().lower()
    if os.name == "nt" or system == "windows":
        root = Path(os.environ.get("APPDATA", home / "AppData" / "Roaming"))
    elif system == "darwin":
        root = home / "Library" / "Application Support"
    else:
        root = Path(os.environ.get("XDG_CONFIG_HOME", home / ".config"))
    return root / "hevc-rebuild-studio"


def app_state_directory() -> Path:
    """Return the native per-user application state/log directory."""
    portable = portable_data_dir()
    if portable is not None:
        return portable
    home = _user_home_directory()
    system = platform.system().lower()
    if os.name == "nt" or system == "windows":
        root = Path(os.environ.get("LOCALAPPDATA", home / "AppData" / "Local"))
        return root / "hevc-rebuild-studio"
    if system == "darwin":
        return home / "Library" / "Logs" / "hevc-rebuild-studio"
    root = Path(os.environ.get("XDG_STATE_HOME", home / ".local" / "state"))
    return root / "hevc-rebuild"


def app_cache_directory() -> Path:
    """Return the native per-user application cache directory."""
    portable = portable_data_dir()
    if portable is not None:
        return portable / "cache"
    home = _user_home_directory()
    system = platform.system().lower()
    if os.name == "nt" or system == "windows":
        root = Path(os.environ.get("LOCALAPPDATA", home / "AppData" / "Local"))
        return root / "hevc-rebuild-studio" / "cache"
    if system == "darwin":
        return home / "Library" / "Caches" / "hevc-rebuild-studio"
    root = Path(os.environ.get("XDG_CACHE_HOME", home / ".cache"))
    return root / "hevc-rebuild"


def user_log_directory() -> Path:
    return app_state_directory() / "logs"


def app_preferences_path() -> Path:
    return app_config_directory() / "settings.json"


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
    if timeout is None:
        timeout = MAX_HELPER_TIMEOUT_SECONDS if Path(args[0]).name in {"mkvmerge", "mkvextract"} else DEFAULT_INSPECTION_TIMEOUT_SECONDS
    if not isinstance(timeout, (int, float)) or not math.isfinite(timeout) or timeout <= 0:
        raise RebuildError(f"Invalid timeout for {args[0]}.")
    timeout = min(timeout, MAX_HELPER_TIMEOUT_SECONDS)
    try:
        proc = subprocess.Popen(args, stdin=subprocess.PIPE if input_text is not None else None,
            stdout=subprocess.PIPE if capture else None, stderr=subprocess.PIPE if capture else None,
            text=text, env=env, start_new_session=os.name == "posix")
    except OSError as exc:
        raise RebuildError(f"Could not start external command {args[0]}: {exc}") from exc
    try:
        stdout, stderr = proc.communicate(input=input_text, timeout=timeout)
    except subprocess.TimeoutExpired as exc:
        terminate_process_group(proc)
        stdout, stderr = proc.communicate(timeout=2)
        detail = stderr.decode(errors="replace") if isinstance(stderr, bytes) else (stderr or "")
        raise ExternalCommandError(args, -1, f"Timed out after {timeout:g} seconds. {detail[-16000:]}") from exc
    except BaseException:
        terminate_process_group(proc)
        proc.communicate(timeout=2)
        raise
    cp = subprocess.CompletedProcess(args, proc.returncode, stdout, stderr)
    if check and cp.returncode != 0:
        details = []
        for value in (cp.stdout, cp.stderr):
            if isinstance(value, bytes):
                value = value.decode("utf-8", errors="replace")
            if isinstance(value, str) and value.strip():
                details.append(value.strip())
        raise ExternalCommandError(args, cp.returncode, "\n".join(details))
    return cp


def run_json(args: list[str], *, timeout: float | None = None) -> Any:
    """Compatibility helper for arbitrary JSON; object consumers validate below."""
    cp = run(args, timeout=timeout)
    try:
        return json.loads(cp.stdout)
    except json.JSONDecodeError as exc:
        raise RebuildError(f"Invalid JSON from {args[0]}: {exc}") from exc


def require_json_object(value: Any, context: str, *, required_keys: Iterable[str] = ()) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise RebuildError(f"{context} returned JSON {type(value).__name__}; expected an object.")
    missing = [key for key in required_keys if key not in value]
    if missing:
        raise RebuildError(f"{context} JSON is missing required fields: {', '.join(missing)}")
    return value


def run_json_object(args: list[str], *, timeout: float | None = None, required_keys: Iterable[str] = ()) -> dict[str, Any]:
    return require_json_object(run_json(args, timeout=timeout), args[0], required_keys=required_keys)


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
    size = safe_float(value)
    if size is None:
        return "unknown"
    for unit in ("B", "KiB", "MiB", "GiB", "TiB"):
        if abs(size) < 1024.0 or unit == "TiB":
            return f"{size:.2f} {unit}"
        size /= 1024.0
    return f"{size:.2f} TiB"


def duration_text(seconds: float | None) -> str:
    seconds = safe_float(seconds)
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
            result = float(a) / b_f if b_f else None
        else:
            result = float(str(value).strip())
        return result if result is not None and math.isfinite(result) else None
    except (TypeError, ValueError, OverflowError, ZeroDivisionError):
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
    # Windows requires a writable handle for FlushFileBuffers/fsync; Linux
    # accepts a read-only descriptor. Keep the least-privileged read handle on
    # Unix and use read/write only for the Windows durability probe.
    mode = "r+b" if os.name == "nt" else "rb"
    with path.open(mode) as f:
        os.fsync(f.fileno())


def fsync_dir(path: Path) -> None:
    # Directory durability is required on supported Linux. Non-Unix platforms
    # lacking O_DIRECTORY cannot claim the same guarantee.
    if not hasattr(os, "O_DIRECTORY"):
        return
    fd = os.open(path, os.O_DIRECTORY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def unique_backup(path: Path) -> Path:
    # This is only an advisory name choice. Commit must use rename_no_replace.
    if not os.path.lexists(path):
        return path
    stem, suffix = path.stem, path.suffix
    for n in range(1, 10000):
        candidate = path.with_name(f"{stem}.{n}{suffix}")
        if not os.path.lexists(candidate):
            return candidate
    raise RebuildError(f"Unable to allocate a unique backup path under {path.parent}")


def configure_logging(log_dir: Path, verbose: bool = False) -> tuple[Path, Path]:
    # New engine artifacts contain local paths and are private by policy. Keep
    # existing directory/file modes intact; exports remain explicit workflows.
    log_dir.mkdir(parents=True, exist_ok=True, mode=0o700)
    stamp = time.strftime("%Y%m%d-%H%M%S")
    # Allocate once for every artifact of this run, including sequential runs
    # within one clock tick. Native consumers associate the emitted exact path.
    run_id = uuid.uuid4().hex
    artifact_name = f"hevc-rebuild-{stamp}-{run_id}"
    log_path = log_dir / f"{artifact_name}.log"
    csv_path = log_dir / f"{artifact_name}.csv"
    root = logging.getLogger()
    for handler in root.handlers[:]:
        try:
            handler.flush()
            handler.close()
        finally:
            root.removeHandler(handler)
    root.setLevel(logging.DEBUG)
    fd = os.open(log_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    file_handler = logging.FileHandler(log_path, encoding="utf-8", delay=True)
    file_handler.stream = os.fdopen(fd, "w", encoding="utf-8")
    file_handler.setLevel(logging.DEBUG)
    file_handler.setFormatter(logging.Formatter("%(asctime)s %(levelname)s %(message)s"))
    root.addHandler(file_handler)
    console = logging.StreamHandler(sys.stderr)
    console.setLevel(logging.DEBUG if verbose else logging.WARNING)
    console.setFormatter(logging.Formatter("%(levelname)s: %(message)s"))
    root.addHandler(console)
    return log_path, csv_path


REPORT_FIELDS = [
    "input", "output", "status", "message", "primary_video", "resolution",
    "audio_count", "subtitle_count", "subtitle_note", "hdr10", "hdr10plus", "hdr10plus_note", "dolby_vision",
    "dovi_profile", "dovi_source", "dolby_vision_action",
    "memory_forecast", "memory_available_gib", "memory_total_gib", "memory_swap_used_percent", "memory_budget_gib",
    "memory_worker_cap", "planned_workers", "planned_frame_threads", "memory_forecast_message",
    "calibration_source", "calibration_workers", "calibration_frame_threads", "calibration_candidate_count", "calibration_note",
    "min_memory_available_gib", "max_encoder_rss_gib", "max_swap_used_percent", "max_encoder_swap_gib", "max_swap_in_mib_s", "max_swap_out_mib_s", "max_memory_full_psi_percent",
    "input_bytes", "output_bytes", "upload_resolution", "upload_quality_mode", "upload_source_multiplier", "output_resolution",
    "upload_video_bitrate_kbps", "youtube_recommended_video_bitrate_kbps", "estimated_output_bytes",
    "metadata_text_scrub_count", "metadata_text_scrub_details", "elapsed_seconds",
]


def append_csv(csv_path: Path, row: dict[str, Any]) -> None:
    native_helper = os.environ.get("HEVC_REBUILD_NATIVE_HELPER")
    if native_helper:
        command = [native_helper, "internal", "report-append"]
        try:
            report_row = {
                key: None if isinstance(value, float) and not math.isfinite(value) else value
                for key, value in row.items()
            }
            payload = json.dumps(
                {"path": str(csv_path), "row": report_row},
                ensure_ascii=False,
                default=str,
                allow_nan=False,
            )
            result = run(command, input_text=payload, timeout=30, check=False)
        except (OSError, subprocess.SubprocessError, TypeError, ValueError) as exc:
            raise RebuildError(f"Could not append the run report through Rust: {exc}") from exc
        if result.returncode != 0:
            detail = result.stderr.strip()[-4000:]
            suffix = f" Details: {detail}" if detail else ""
            raise RebuildError(
                f"The Rust report writer exited with status {result.returncode}.{suffix}"
            )
        return

    # Linux flock is shared with the Rust writer and covers header validation
    # plus the entire append, across processes. Windows has no fcntl module;
    # use a one-byte sibling lock file there so parallel CLI/native writers
    # retain the same serialization guarantee without adding a dependency.
    windows_lock_fd: int | None = None
    if os.name == "nt":
        import msvcrt

        lock_path = Path(str(csv_path) + ".lock")
        windows_lock_fd = os.open(lock_path, os.O_RDWR | os.O_CREAT, 0o600)
        if os.path.getsize(lock_path) == 0:
            os.write(windows_lock_fd, b"\0")
        os.lseek(windows_lock_fd, 0, os.SEEK_SET)
        msvcrt.locking(windows_lock_fd, msvcrt.LK_LOCK, 1)
        fcntl = None
    else:
        import fcntl

    fd = os.open(
        csv_path,
        os.O_RDWR | os.O_CREAT | os.O_APPEND | getattr(os, "O_NOFOLLOW", 0),
        0o600,
    )
    with os.fdopen(fd, "r+b", buffering=0) as stream:
        if fcntl is not None:
            fcntl.flock(stream, fcntl.LOCK_EX)
        try:
            stream.seek(0, os.SEEK_END)
            original_size = stream.tell()
            if original_size:
                stream.seek(0)
                header = next(csv.reader([stream.readline(16384).decode("utf-8")]), [])
                if header != REPORT_FIELDS:
                    raise RebuildError(f"Run report schema mismatch: {csv_path}")
                stream.seek(original_size - 1)
                if stream.read(1) != b"\n":
                    raise RebuildError(f"Run report has an incomplete write; review before retrying: {csv_path}")
            output = io.StringIO(newline="")
            writer = csv.DictWriter(output, fieldnames=REPORT_FIELDS, extrasaction="ignore")
            if not original_size:
                writer.writeheader()
            writer.writerow({key: None if isinstance(value, float) and not math.isfinite(value) else value for key, value in row.items()})
            stream.seek(0, os.SEEK_END)
            try:
                remaining = memoryview(output.getvalue().encode("utf-8"))
                while remaining:
                    written = stream.write(remaining)
                    if not written:
                        raise OSError("Run report writer made no progress")
                    remaining = remaining[written:]
                stream.flush()
            except OSError:
                # Restore this append on an ordinary writer failure. A crashed
                # writer's partial tail is rejected above, never guessed away.
                os.ftruncate(stream.fileno(), original_size)
                raise
            try:
                os.fsync(stream.fileno())
                if not original_size:
                    fsync_dir(csv_path.parent)
            except OSError as exc:
                raise RebuildError(f"Run report was written but durability is unconfirmed: {csv_path}: {exc}") from exc
        finally:
            if fcntl is not None:
                fcntl.flock(stream, fcntl.LOCK_UN)
            if windows_lock_fd is not None:
                msvcrt.locking(windows_lock_fd, msvcrt.LK_UNLCK, 1)
                os.close(windows_lock_fd)


def dataclass_json(value: Any) -> str:
    if is_dataclass(value):
        value = asdict(value)
    return json.dumps(value, indent=2, default=str, ensure_ascii=False, allow_nan=False)


class DiagnosticTail:
    """Thread-safe retained tail: at most 64K characters and 1024 chunks."""
    def __init__(self) -> None:
        self._lines: deque[str] = deque()
        self._characters = 0
        self._lock = threading.Lock()
        self.truncated = False
        self.error: str | None = None

    def append(self, line: str) -> None:
        with self._lock:
            if len(line) > 8192:
                line = line[-8192:]
                self.truncated = True
            self._lines.append(line)
            self._characters += len(line)
            while self._characters > 65536 or len(self._lines) > 1024:
                self._characters -= len(self._lines.popleft())
                self.truncated = True

    def __iter__(self):
        with self._lock:
            return iter(tuple(self._lines))

    def text(self, limit: int = 16000) -> str:
        with self._lock:
            text = "".join(self._lines)
            clipped = self.truncated or len(text) > limit
            error = self.error
        prefix = "[diagnostics truncated; showing retained tail]\n" if clipped else ""
        if error:
            prefix += f"[diagnostic drain failed: {error}]\n"
        return prefix + text[-limit:]


def drain_pipe(pipe, sink: DiagnosticTail | list[str]) -> None:
    if pipe is None:
        return
    try:
        for line in iter(lambda: pipe.readline(8192), ""):
            sink.append(line)
            if isinstance(sink, list):
                # Retain the previous list-sink contract for compatibility.
                clipped = False
                while len(sink) > 1024 or sum(map(len, sink)) > 65536:
                    sink.pop(0)
                    clipped = True
                if clipped:
                    sink.insert(0, "[diagnostics truncated; showing retained tail]\n")
            logging.debug("child-stderr: %s", line.rstrip())
    except (OSError, ValueError) as exc:
        if isinstance(sink, DiagnosticTail):
            sink.error = str(exc)
        else:
            sink.append(f"[diagnostic drain failed: {exc}]\n")
        logging.warning("Child diagnostic drain failed: %s", exc)
    finally:
        pipe.close()


def start_stderr_drain(pipe, sink: DiagnosticTail | list[str]) -> threading.Thread:
    t = threading.Thread(target=drain_pipe, args=(pipe, sink), daemon=True)
    t.start()
    return t
