#!/usr/bin/env python3
"""Create a bounded test clip without modifying the source media.

The default ``copy`` mode is fast and keeps every source stream that the
selected output container can represent.  Because stream copying can only
start on a keyframe, use ``--mode transcode`` when the requested start point
must be frame-accurate.  Transcode mode uses broadly available H.264/AAC
settings and is intended for short test fixtures, not archival output.

Examples (Linux/macOS):
    python tools/extract_clip.py input.mkv --duration 30s
    python tools/extract_clip.py input.mkv --start 2m --duration 45s \
        --output ./fixtures/input-45s.mkv

Windows uses the same arguments with ``py -3`` in place of ``python``.
"""

from __future__ import annotations

import argparse
from dataclasses import dataclass
from decimal import Decimal, InvalidOperation
import json
import math
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import sys
import tempfile
from typing import Sequence


MAX_SECONDS = 24 * 60 * 60
DEFAULT_TIMEOUT_SECONDS = 10 * 60
MAX_TIMEOUT_SECONDS = 6 * 60 * 60
DIAGNOSTIC_LIMIT = 16 * 1024

_DURATION_RE = re.compile(
    r"^\s*(?P<value>\d+(?:\.\d+)?)\s*"
    r"(?P<unit>s|sec(?:ond)?s?|m|min(?:ute)?s?|h|hr(?:s)?|hour(?:s)?)?\s*$",
    re.IGNORECASE,
)


class ClipError(Exception):
    """A user-facing extraction failure with a stable process exit code."""

    def __init__(self, message: str, *, code: int = 1) -> None:
        super().__init__(message)
        self.code = code


@dataclass(frozen=True)
class ClipRequest:
    input_path: Path
    output_path: Path
    start_seconds: float
    duration_seconds: float
    mode: str
    ffmpeg: str
    timeout_seconds: float


def parse_duration(value: str, *, name: str = "duration", allow_zero: bool = False) -> float:
    """Parse seconds/minutes/hours into a finite bounded number of seconds."""

    match = _DURATION_RE.fullmatch(value)
    if not match:
        raise ClipError(
            f"Invalid {name} {value!r}; use a positive value such as 30s, 1.5m, or 2h."
        )
    try:
        amount = Decimal(match.group("value"))
    except InvalidOperation as exc:  # pragma: no cover - regex excludes this
        raise ClipError(f"Invalid {name} {value!r}.") from exc
    unit = (match.group("unit") or "s").lower()
    factor = 1
    if unit.startswith("m"):
        factor = 60
    elif unit.startswith("h"):
        factor = 60 * 60
    seconds = amount * factor
    if not seconds.is_finite():
        raise ClipError(f"{name.capitalize()} must be finite.")
    if seconds < 0 or (seconds == 0 and not allow_zero):
        qualifier = "non-negative" if allow_zero else "positive"
        raise ClipError(f"{name.capitalize()} must be {qualifier}.")
    if seconds > MAX_SECONDS:
        raise ClipError(f"{name.capitalize()} may not exceed 24 hours.")
    result = float(seconds)
    if not math.isfinite(result):  # defensive against an unusual float platform
        raise ClipError(f"{name.capitalize()} must be finite.")
    return result


def format_seconds(seconds: float) -> str:
    """Format seconds for FFmpeg and generated filenames without float noise."""

    if seconds.is_integer():
        return str(int(seconds))
    return f"{seconds:.6f}".rstrip("0").rstrip(".")


def duration_label(seconds: float) -> str:
    return f"{format_seconds(seconds)}s"


def _tail(text: str, limit: int = DIAGNOSTIC_LIMIT) -> str:
    if len(text) <= limit:
        return text.strip()
    return "[FFmpeg diagnostics truncated; showing tail]\n" + text[-limit:].strip()


def _resolve_input(path: Path) -> Path:
    path = path.expanduser()
    try:
        resolved = path.resolve(strict=True)
    except FileNotFoundError as exc:
        raise ClipError(f"Input does not exist: {path}", code=2) from exc
    except OSError as exc:
        raise ClipError(f"Cannot inspect input {path}: {exc}", code=2) from exc
    if not resolved.is_file():
        raise ClipError(f"Input is not a regular file: {path}", code=2)
    return resolved


def _resolve_output(
    input_path: Path,
    *,
    output: Path | None,
    output_dir: Path | None,
    duration_seconds: float,
) -> Path:
    if output is not None and output_dir is not None:
        raise ClipError("Use either --output or --output-dir, not both.", code=2)

    if output is None:
        directory = (output_dir or Path.cwd()).expanduser()
        try:
            directory = directory.resolve(strict=True)
        except FileNotFoundError as exc:
            raise ClipError(f"Output directory does not exist: {directory}", code=2) from exc
        if not directory.is_dir():
            raise ClipError(f"Output directory is not a directory: {directory}", code=2)
        output = directory / f"{input_path.stem}-clip-{duration_label(duration_seconds)}.mkv"
    else:
        output = output.expanduser()
        if output.suffix == "":
            output = output.with_suffix(".mkv")
        parent = output.parent
        try:
            parent = parent.resolve(strict=True)
        except FileNotFoundError as exc:
            raise ClipError(f"Output directory does not exist: {output.parent}", code=2) from exc
        if not parent.is_dir():
            raise ClipError(f"Output parent is not a directory: {parent}", code=2)
        output = parent / output.name

    # lexists also rejects a dangling symlink rather than allowing FFmpeg to
    # follow or replace an unexpected target.
    if os.path.lexists(output):
        raise ClipError(f"Refusing to overwrite existing output: {output}", code=4)
    if output.resolve(strict=False) == input_path:
        raise ClipError("Output must be different from the input.", code=2)
    return output


def _resolve_ffmpeg(value: str | None) -> str:
    candidate = value or os.environ.get("HEVC_REBUILD_FFMPEG") or shutil.which("ffmpeg")
    if not candidate:
        raise ClipError(
            "FFmpeg was not found on PATH. Install FFmpeg or pass --ffmpeg PATH.",
            code=2,
        )
    if any(sep in candidate for sep in (os.sep, os.altsep) if sep):
        path = Path(candidate).expanduser()
        if not path.is_file():
            raise ClipError(f"FFmpeg executable does not exist: {path}", code=2)
        if os.name != "nt" and not os.access(path, os.X_OK):
            raise ClipError(f"FFmpeg is not executable: {path}", code=2)
        return str(path)
    return candidate


def build_command(request: ClipRequest, temporary_output: Path) -> list[str]:
    """Build an argv list; no shell quoting or interpolation is used."""

    command = [
        request.ffmpeg,
        "-hide_banner",
        "-nostdin",
        # The destination is a private, uniquely-created temporary path. The
        # final requested output is published with an atomic no-overwrite step.
        "-y",
        "-loglevel",
        "error",
    ]
    if request.mode == "copy":
        # Fast seeking is intentional here. Stream copying cannot make a
        # frame-accurate cut before the next keyframe; transcode mode below can.
        command += ["-ss", format_seconds(request.start_seconds), "-i", str(request.input_path)]
    else:
        command += ["-i", str(request.input_path), "-ss", format_seconds(request.start_seconds)]
    command += [
        "-t",
        format_seconds(request.duration_seconds),
        "-map",
        "0",
        "-map_metadata",
        "0",
        "-map_chapters",
        "-1",
    ]
    if request.mode == "copy":
        command += ["-c", "copy"]
    else:
        command += [
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "-crf",
            "23",
            "-c:a",
            "aac",
            "-b:a",
            "160k",
            "-c:s",
            "copy",
            "-c:d",
            "copy",
            "-c:t",
            "copy",
        ]
    command += ["-avoid_negative_ts", "make_zero", str(temporary_output)]
    return command


def _terminate_process(process: subprocess.Popen[str]) -> None:
    if process.poll() is not None:
        return
    if os.name == "nt":
        try:
            process.send_signal(signal.CTRL_BREAK_EVENT)
            process.wait(timeout=5)
            return
        except (AttributeError, OSError, subprocess.TimeoutExpired):
            process.kill()
            return
    try:
        os.killpg(process.pid, signal.SIGTERM)
        process.wait(timeout=5)
    except (OSError, subprocess.TimeoutExpired):
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except OSError:
            process.kill()


def _run_ffmpeg(command: Sequence[str], *, timeout_seconds: float) -> str:
    options: dict[str, object] = {
        "stdin": subprocess.DEVNULL,
        "stdout": subprocess.PIPE,
        "stderr": subprocess.PIPE,
        "text": True,
    }
    if os.name == "nt":
        options["creationflags"] = subprocess.CREATE_NEW_PROCESS_GROUP
    else:
        options["start_new_session"] = True
    process = subprocess.Popen(list(command), **options)  # type: ignore[arg-type]
    try:
        _, stderr = process.communicate(timeout=timeout_seconds)
    except subprocess.TimeoutExpired as exc:
        _terminate_process(process)
        raise ClipError(f"FFmpeg timed out after {timeout_seconds:g} seconds.", code=124) from exc
    if process.returncode != 0:
        detail = _tail(stderr or "")
        suffix = f"\n{detail}" if detail else ""
        raise ClipError(f"FFmpeg failed with exit code {process.returncode}.{suffix}", code=3)
    return _tail(stderr or "")


def _publish_no_overwrite(temporary_output: Path, output: Path) -> None:
    if os.path.lexists(output):
        raise ClipError(f"Refusing to overwrite existing output: {output}", code=4)
    try:
        if os.name == "nt":
            # os.rename raises if the destination appears between the check
            # and this operation on Windows, preserving no-overwrite behavior.
            os.rename(temporary_output, output)
        else:
            # A hard-link create is atomic and fails if another process wins
            # the destination race. Both paths are in the same output folder.
            os.link(temporary_output, output)
            temporary_output.unlink()
    except FileExistsError as exc:
        raise ClipError(f"Refusing to overwrite existing output: {output}", code=4) from exc
    except OSError as exc:
        raise ClipError(f"Could not publish clip without overwriting: {exc}", code=1) from exc


def extract(request: ClipRequest) -> dict[str, object]:
    descriptor, temporary_name = tempfile.mkstemp(
        prefix=f".{request.output_path.name}.part-",
        suffix=request.output_path.suffix or ".mkv",
        dir=request.output_path.parent,
    )
    os.close(descriptor)
    temporary_output = Path(temporary_name)
    try:
        command = build_command(request, temporary_output)
        _run_ffmpeg(command, timeout_seconds=request.timeout_seconds)
        if not temporary_output.is_file() or temporary_output.stat().st_size == 0:
            raise ClipError("FFmpeg reported success but produced no clip.", code=3)
        if os.name != "nt":
            temporary_output.chmod(0o600)
        _publish_no_overwrite(temporary_output, request.output_path)
    except Exception:
        try:
            temporary_output.unlink(missing_ok=True)
        except OSError:
            pass
        raise

    return {
        "ok": True,
        "input": str(request.input_path),
        "output": str(request.output_path),
        "mode": request.mode,
        "start_seconds": request.start_seconds,
        "duration_seconds": request.duration_seconds,
        "output_bytes": request.output_path.stat().st_size,
        "ffmpeg": request.ffmpeg,
    }


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Create a bounded short video fixture without modifying the source file."
    )
    parser.add_argument("input", type=Path, help="source video path")
    parser.add_argument("-d", "--duration", required=True, help="clip length, for example 30s or 2m")
    parser.add_argument("--start", default="0s", help="start offset, for example 15s or 1m")
    parser.add_argument(
        "--mode",
        choices=("copy", "transcode"),
        default="copy",
        help="copy streams quickly (default) or transcode for a frame-accurate start",
    )
    output = parser.add_mutually_exclusive_group()
    output.add_argument("-o", "--output", type=Path, help="output path; .mkv is added when omitted")
    output.add_argument("--output-dir", type=Path, help="existing directory for the derived MKV name")
    parser.add_argument("--ffmpeg", help="FFmpeg executable path; otherwise HEVC_REBUILD_FFMPEG or PATH")
    parser.add_argument(
        "--timeout",
        type=float,
        default=DEFAULT_TIMEOUT_SECONDS,
        help=f"FFmpeg timeout in seconds (default {DEFAULT_TIMEOUT_SECONDS:g}; max {MAX_TIMEOUT_SECONDS:g})",
    )
    parser.add_argument("--json", action="store_true", help="print one machine-readable JSON result")
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    parser = _parser()
    args = parser.parse_args(argv)
    try:
        if not math.isfinite(args.timeout) or args.timeout <= 0 or args.timeout > MAX_TIMEOUT_SECONDS:
            raise ClipError(f"Timeout must be greater than zero and at most {MAX_TIMEOUT_SECONDS:g} seconds.", code=2)
        input_path = _resolve_input(args.input)
        duration_seconds = parse_duration(args.duration)
        start_seconds = parse_duration(args.start, name="start", allow_zero=True)
        output_path = _resolve_output(
            input_path,
            output=args.output,
            output_dir=args.output_dir,
            duration_seconds=duration_seconds,
        )
        request = ClipRequest(
            input_path=input_path,
            output_path=output_path,
            start_seconds=start_seconds,
            duration_seconds=duration_seconds,
            mode=args.mode,
            ffmpeg=_resolve_ffmpeg(args.ffmpeg),
            timeout_seconds=args.timeout,
        )
        result = extract(request)
    except ClipError as exc:
        if args.json:
            print(json.dumps({"ok": False, "error": str(exc), "exit_code": exc.code}, sort_keys=True))
        else:
            print(f"extract_clip: {exc}", file=sys.stderr)
        return exc.code

    if args.json:
        print(json.dumps(result, sort_keys=True))
    else:
        print(
            f"Created {result['output']} ({result['output_bytes']} bytes; "
            f"{result['mode']} mode, requested {duration_label(duration_seconds)} from "
            f"{format_seconds(start_seconds)}s)."
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
