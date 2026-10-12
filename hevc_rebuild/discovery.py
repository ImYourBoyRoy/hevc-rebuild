from __future__ import annotations

import os
import re
import logging
from pathlib import Path
from collections.abc import Sequence

from .util import RebuildError, native_json_action

MEDIA_EXTENSIONS = {".mkv", ".mp4", ".m4v", ".mov", ".avi", ".ts", ".m2ts", ".mts", ".webm"}
EXCLUDED_DIRS = {"old", ".hevc-rebuild", ".git", "node_modules", "youtube uploads"}


def candidate_file(path: Path, min_size: int) -> bool:
    try:
        return path.is_file() and path.suffix.lower() in MEDIA_EXTENSIONS and path.stat().st_size >= min_size and not path.name.startswith("Test_") and ".hevc-rebuild." not in path.name
    except OSError:
        return False


def validate_filename_patterns(patterns: Sequence[str], label: str) -> None:
    if len(patterns) > 100:
        raise RebuildError(f"Filename {label} list is limited to 100 patterns.")
    seen: set[str] = set()
    for index, pattern in enumerate(patterns, start=1):
        if not isinstance(pattern, str):
            raise RebuildError(f"Filename {label} pattern {index} must be text.")
        if not pattern or pattern.strip() != pattern:
            raise RebuildError(f"Filename {label} pattern {index} must not be empty or padded with spaces.")
        if len(pattern.encode("utf-8")) > 128:
            raise RebuildError(f"Filename {label} pattern {index} is limited to 128 UTF-8 bytes.")
        if any(ord(character) < 32 or character in "/\\" for character in pattern):
            raise RebuildError("Filename patterns cannot contain control characters or path separators.")
        if all(character in "*?" for character in pattern):
            raise RebuildError(f"Filename {label} pattern {index} must include at least one filename character.")
        normalized = pattern.lower()
        if normalized in seen:
            raise RebuildError(f"Filename {label} patterns cannot contain case-insensitive duplicates.")
        seen.add(normalized)


def _filename_glob_matches(name: str, pattern: str) -> bool:
    name_chars = list(name.lower())
    pattern_chars = list(pattern.lower())
    name_index = pattern_index = 0
    last_star: int | None = None
    retry_name_index = 0
    while name_index < len(name_chars):
        if pattern_index < len(pattern_chars) and (
            pattern_chars[pattern_index] == "?" or pattern_chars[pattern_index] == name_chars[name_index]
        ):
            name_index += 1
            pattern_index += 1
        elif pattern_index < len(pattern_chars) and pattern_chars[pattern_index] == "*":
            last_star = pattern_index
            pattern_index += 1
            retry_name_index = name_index
        elif last_star is not None:
            retry_name_index += 1
            name_index = retry_name_index
            pattern_index = last_star + 1
        else:
            return False
    while pattern_index < len(pattern_chars) and pattern_chars[pattern_index] == "*":
        pattern_index += 1
    return pattern_index == len(pattern_chars)


def filter_by_filename_patterns(
    files: Sequence[Path], *, include: Sequence[str] = (), exclude: Sequence[str] = ()
) -> list[Path]:
    validate_filename_patterns(include, "include")
    validate_filename_patterns(exclude, "exclude")
    return [
        path for path in files
        if (not include or any(_filename_glob_matches(path.name, pattern) for pattern in include))
        and not any(_filename_glob_matches(path.name, pattern) for pattern in exclude)
    ]


def filter_by_selected_files(files: Sequence[Path], selected_paths: Sequence[str] = ()) -> list[Path]:
    if not selected_paths:
        return list(files)
    if len(selected_paths) > 10_000:
        raise RebuildError("A job can select at most 10,000 media files.")
    requested: set[Path] = set()
    for index, raw_path in enumerate(selected_paths, start=1):
        if not isinstance(raw_path, str) or not raw_path or len(raw_path.encode("utf-8")) > 8192:
            raise RebuildError(f"Selected media path {index} is invalid or too long.")
        path = Path(raw_path).expanduser()
        if not path.is_absolute():
            raise RebuildError("Selected media paths must be absolute paths.")
        try:
            resolved = path.resolve(strict=True)
        except OSError as exc:
            raise RebuildError(f"A selected media file is no longer available: {path}") from exc
        if not resolved.is_file():
            raise RebuildError("Selected media paths must point to files.")
        if resolved in requested:
            raise RebuildError("Selected media paths must be unique.")
        requested.add(resolved)
    available = {path.resolve() for path in files}
    missing = requested - available
    if missing:
        raise RebuildError("A selected file is no longer in the analyzed input or does not match the active filename filters. Analyze again.")
    return [path for path in files if path.resolve() in requested]


def discover(
    root: Path,
    *,
    recursive: bool,
    min_size: int,
    include_name_patterns: Sequence[str] = (),
    exclude_name_patterns: Sequence[str] = (),
    selected_paths: Sequence[str] = (),
) -> list[Path]:
    validate_filename_patterns(include_name_patterns, "include")
    validate_filename_patterns(exclude_name_patterns, "exclude")
    native = native_json_action(
        "discover",
        {
            "root": str(root.expanduser()),
            "recursive": recursive,
            "minimum_size_bytes": min_size,
        },
        timeout=60.0,
    )
    if native is not None:
        files = native.get("files")
        warnings = native.get("warnings", [])
        if not isinstance(files, list) or any(not isinstance(path, str) for path in files):
            raise RebuildError("The Rust discovery helper returned an invalid file list.")
        if not isinstance(warnings, list) or any(not isinstance(item, str) for item in warnings):
            raise RebuildError("The Rust discovery helper returned invalid warnings.")
        for warning in warnings:
            logging.warning("Media discovery: %s", warning)
        if native.get("limit_reached") is True:
            raise RebuildError(
                "Rust discovery reached its 10,000-file safety limit. Narrow the selected folder before running the job."
            )
        filtered = filter_by_filename_patterns(
            [Path(path) for path in files], include=include_name_patterns, exclude=exclude_name_patterns
        )
        return filter_by_selected_files(filtered, selected_paths)

    if root.is_file():
        files = [root] if candidate_file(root, min_size) else []
        filtered = filter_by_filename_patterns(files, include=include_name_patterns, exclude=exclude_name_patterns)
        return filter_by_selected_files(filtered, selected_paths)
    seen: set[Path] = set()
    found: list[Path] = []

    def add_dir(directory: Path) -> None:
        try:
            entries = sorted(directory.iterdir(), key=lambda p: p.name.lower())
        except OSError:
            return
        for p in entries:
            if candidate_file(p, min_size) and p.resolve() not in seen:
                seen.add(p.resolve())
                found.append(p)

    if recursive:
        for current, dirs, files in os.walk(root):
            dirs[:] = [d for d in dirs if d.lower() not in EXCLUDED_DIRS and not d.startswith(".")]
            cur = Path(current)
            for name in sorted(files):
                p = cur / name
                if candidate_file(p, min_size) and p.resolve() not in seen:
                    seen.add(p.resolve())
                    found.append(p)
    else:
        # Legacy-compatible: direct movies plus Season* directories. Enhanced to find Show/Season XX one level below the supplied root too.
        add_dir(root)
        season_dirs: list[Path] = []
        for p in root.iterdir():
            if p.is_dir() and p.name.lower() not in EXCLUDED_DIRS:
                if re.match(r"(?i)^season\s*\d+", p.name):
                    season_dirs.append(p)
                else:
                    try:
                        season_dirs.extend(q for q in p.iterdir() if q.is_dir() and re.match(r"(?i)^season\s*\d+", q.name))
                    except OSError:
                        pass
        for d in sorted(season_dirs):
            add_dir(d)
    filtered = filter_by_filename_patterns(found, include=include_name_patterns, exclude=exclude_name_patterns)
    return filter_by_selected_files(filtered, selected_paths)
