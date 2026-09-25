from __future__ import annotations

import os
import re
from pathlib import Path

MEDIA_EXTENSIONS = {".mkv", ".mp4", ".m4v", ".mov", ".avi", ".ts", ".m2ts", ".mts", ".webm"}
EXCLUDED_DIRS = {"old", ".hevc-rebuild", ".git", "node_modules"}


def candidate_file(path: Path, min_size: int) -> bool:
    try:
        return path.is_file() and path.suffix.lower() in MEDIA_EXTENSIONS and path.stat().st_size >= min_size and not path.name.startswith("Test_") and ".hevc-rebuild." not in path.name
    except OSError:
        return False


def discover(root: Path, *, recursive: bool, min_size: int) -> list[Path]:
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
    return found
