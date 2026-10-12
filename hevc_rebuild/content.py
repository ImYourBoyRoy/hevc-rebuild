from __future__ import annotations

import re
import xml.etree.ElementTree as ET
from pathlib import Path
from typing import Any

from .metadata import nfo_paths_for
from .util import RebuildError, native_json_action


def _split_genres(value: str) -> list[str]:
    return [x.strip() for x in re.split(r"[,;/|]", value) if x.strip()]


def metadata_genres(path: Path, mediainfo: dict[str, Any]) -> set[str]:
    genres: set[str] = set()
    for nfo in nfo_paths_for(path):
        try:
            root = ET.parse(nfo).getroot()
            for node in root.findall("genre"):
                if node.text:
                    genres.update(x.lower() for x in _split_genres(node.text))
        except (ET.ParseError, OSError):
            # NFO parsing for naming/tags remains strict later. Tune detection itself
            # must never make the encode fail or guess from malformed metadata.
            pass

    for track in ((mediainfo.get("media") or {}).get("track") or []):
        if str(track.get("@type", "")).lower() != "general":
            continue
        for key in ("Genre", "Genres"):
            value = track.get(key)
            if value:
                genres.update(x.lower() for x in _split_genres(str(value)))
    return genres


def resolve_tune(path: Path, mediainfo: dict[str, Any], requested: str, detect_animation: bool) -> str | None:
    requested = (requested or "auto").lower()
    if requested == "none":
        return None
    if requested in {"animation", "grain"}:
        return requested
    if requested != "auto":
        return None
    if detect_animation and "animation" in metadata_genres(path, mediainfo):
        return "animation"
    return None


def resolve_run_tune(
    path: Path,
    mediainfo: dict[str, Any],
    requested: str,
    detect_animation: bool,
    file_override: str | None = None,
) -> str | None:
    """Use Rust's run-time tune decision for app-launched jobs, with direct fallback."""
    general_tracks = []
    tracks = ((mediainfo.get("media") or {}).get("track") or []) if isinstance(mediainfo, dict) else []
    for track in tracks:
        if not isinstance(track, dict) or str(track.get("@type", "")).lower() != "general":
            continue
        summary = {"@type": "General"}
        for key in ("Genre", "Genres"):
            if key in track:
                summary[key] = track[key]
        general_tracks.append(summary)
    response = native_json_action(
        "content-tune",
        {
            "path": str(path.resolve()),
            "mediainfo": {"media": {"track": general_tracks}},
            "requested_tune": requested or "auto",
            "detect_animation": detect_animation,
            "file_override": file_override,
        },
        timeout=15.0,
    )
    if response is None:
        return resolve_tune(
            path,
            mediainfo,
            file_override if file_override is not None else requested,
            False if file_override is not None else detect_animation,
        )
    tune = response.get("tune")
    if tune is None or tune == "none":
        return None
    if tune in {"animation", "grain"}:
        return str(tune)
    raise RebuildError("The Rust content-tune helper returned an unsupported tune.")
