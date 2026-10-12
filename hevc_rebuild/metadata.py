from __future__ import annotations

import json
import logging
import mimetypes
import re
import shutil
import tempfile
import xml.etree.ElementTree as ET
from pathlib import Path
from typing import Any, Sequence

from .model import MediaProfile, MetadataResult, Toolchain
from .util import RebuildError, native_json_action, run, run_json

TAG_MAP = {
    "title": "TITLE", "showtitle": "SHOWTITLE", "season": "SEASON", "episode": "EPISODE",
    "plot": "SYNOPSIS", "tagline": "SUMMARY", "mpaa": "LAW_RATING", "year": "DATE_RELEASED",
    "premiered": "DATE_RELEASED", "aired": "DATE_RELEASED", "studio": "PRODUCTION_STUDIO",
    "genre": "GENRE", "source": "SOURCE", "userrating": "RATING",
}

_RELEASE_GROUP_TAG_NAMES = {"RELEASE_GROUP", "RELEASEGROUP", "RLSGROUP"}
_RELEASE_MARKER_RE = re.compile(
    r"(?<![a-z0-9])(?:"
    r"\d{3,4}p|4k|8k|web[\s._-]?(?:dl|rip)|blu[\s._-]?ray|bdrip|brrip|"
    r"hdtv|dvd[\s._-]?rip|remux|x26[45]|h\.?26[45]|hevc|av1|hdr10\+?|hlg|uhd"
    r")(?![a-z0-9])",
    re.IGNORECASE,
)
_RELEASE_GROUP_TOKEN_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]{1,}$")
_TECHNICAL_RELEASE_TOKENS = {
    "2160P", "1080P", "720P", "576P", "480P", "4320P", "4K", "8K", "WEB", "DL", "RIP",
    "BLURAY", "BDRIP", "BRRIP", "HDTV", "DVDRIP", "REMUX", "X264", "X265", "H264", "H265",
    "HEVC", "AV1", "AAC", "AC3", "EAC3", "DDP", "DTS", "DTSHD", "TRUEHD", "ATMOS", "FLAC",
    "HDR", "HDR10", "HLG", "DV", "UHD", "SDR", "10BIT", "8BIT", "HI10P", "STEREO", "MONO",
    "HD", "SD",
}
_KNOWN_LOWERCASE_RELEASE_GROUPS = {
    "amiable", "cakes", "ctrlhd", "d3g", "eureka", "ettv", "evo", "eztv", "fgt", "flux",
    "framestor", "galaxyrg", "hdt", "ion10", "joy", "megusta", "ntb", "pahe", "psa",
    "rarbg", "sparks", "tigole", "yify", "yts", "ytsmx",
}
_NATIVE_TEXT_BYTES_LIMIT = 256 * 1024
_NATIVE_TEXT_BATCH_BYTES_LIMIT = 1024 * 1024
_NATIVE_TEXT_BATCH_COUNT_LIMIT = 10_000


def scrub_release_group_suffix(value: str) -> str:
    """Remove a clear release-group suffix without guessing at ordinary title text.

    A suffix is scrubbed only when a recognized release/codec marker appears before
    it. Technical codec, format, and audio tokens are never treated as group names.
    """
    trimmed = value.rstrip()
    trailing = value[len(trimmed):]
    if not trimmed:
        return value

    bracketed = re.search(r"\[(?P<group>[A-Za-z0-9][A-Za-z0-9._-]{1,})\]$", trimmed)
    if bracketed is None:
        bracketed = re.search(r"\((?P<group>[A-Za-z0-9][A-Za-z0-9._-]{1,})\)$", trimmed)
    if bracketed is not None:
        group = bracketed.group("group")
        prefix = trimmed[:bracketed.start()].rstrip()
        if _is_release_group(prefix, group):
            return prefix.rstrip(" ._-–—") + trailing

    for index in range(len(trimmed) - 1, -1, -1):
        if trimmed[index] not in "._-–—":
            continue
        group = trimmed[index + 1:]
        prefix = trimmed[:index].rstrip()
        if _is_release_group(prefix, group):
            return prefix.rstrip(" ._-–—") + trailing
    return value


def _clean_metadata_globs(value: str, patterns: Sequence[str]) -> tuple[str, list[str]]:
    result = value
    matched: list[str] = []
    for pattern in patterns:
        expression = re.compile(re.escape(pattern).replace(r"\*", ".*?"), re.IGNORECASE | re.DOTALL)
        matches = [match for match in expression.finditer(result) if match.end() > match.start()]
        if matches:
            matched.append(pattern)
            for match in reversed(matches):
                left = result[:match.start()]
                right = result[match.end():]
                left_gap = len(left) - len(left.rstrip())
                right_gap = len(right) - len(right.lstrip())
                if not left:
                    result = right.lstrip()
                elif not right:
                    result = left.rstrip()
                elif left_gap and right_gap:
                    result = left[:-left_gap] + " " + right[right_gap:]
                else:
                    result = left + right
    return result, matched


def clean_metadata_texts(
    values: Sequence[str],
    patterns: Sequence[str] = (),
) -> list[tuple[str, list[str]]]:
    """Clean multiple text fields in one native request, then apply suffix policy."""
    originals = list(values)
    glob_results: list[tuple[str, list[str]]]
    if originals:
        native = _native_clean_texts(originals, patterns)
    else:
        native = None
    native_suffix_policy = native is not None and native.get("suffix_policy") == "recognized-release-group-v1"
    if native is not None:
        results = native.get("results")
        if not isinstance(results, list) or len(results) != len(originals):
            raise RebuildError("The Rust metadata text helper returned an invalid result list.")
        glob_results = []
        for item in results:
            if not isinstance(item, dict):
                raise RebuildError("The Rust metadata text helper returned an invalid result.")
            cleaned = item.get("text")
            matched = item.get("matched")
            if not isinstance(cleaned, str) or not isinstance(matched, list) or not all(
                isinstance(pattern, str) for pattern in matched
            ):
                raise RebuildError("The Rust metadata text helper returned invalid text or rules.")
            glob_results.append((cleaned, matched))
    else:
        glob_results = [_clean_metadata_globs(value, patterns) for value in originals]

    output: list[tuple[str, list[str]]] = []
    for result, matched in glob_results:
        if native_suffix_policy:
            output.append((result, matched))
            continue
        without_suffix = scrub_release_group_suffix(result)
        if without_suffix != result:
            matched.append("release-group suffix")
        output.append((without_suffix, matched))
    return output


def _native_clean_texts(
    values: Sequence[str],
    patterns: Sequence[str],
) -> dict[str, object] | None:
    results: list[dict[str, object]] = []
    start = 0
    while start < len(values):
        texts: list[str] = []
        byte_count = 0
        while start < len(values) and len(texts) < _NATIVE_TEXT_BATCH_COUNT_LIMIT:
            text = values[start]
            size = len(text.encode("utf-8"))
            if size > _NATIVE_TEXT_BYTES_LIMIT:
                raise RebuildError("A metadata text value exceeds the Rust helper's 256 KiB limit.")
            if texts and byte_count + size > _NATIVE_TEXT_BATCH_BYTES_LIMIT:
                break
            texts.append(text)
            byte_count += size
            start += 1
        response = native_json_action(
            "metadata-text",
            {"texts": texts, "patterns": list(patterns)},
        )
        if response is None:
            return None
        batch = response.get("results")
        if not isinstance(batch, list) or len(batch) != len(texts) or not all(
            isinstance(item, dict) for item in batch
        ):
            raise RebuildError("The Rust metadata text helper returned an invalid result list.")
        results.extend(batch)
    return {"results": results}


def clean_metadata_text(value: str, patterns: Sequence[str] = ()) -> tuple[str, list[str]]:
    """Remove configured case-insensitive substring globs, then known release suffixes."""
    return clean_metadata_texts([value], patterns)[0]


def _is_release_group(prefix: str, group: str) -> bool:
    if not _RELEASE_GROUP_TOKEN_RE.fullmatch(group) or not _RELEASE_MARKER_RE.search(prefix):
        return False
    normalized_group = re.sub(r"[^A-Za-z0-9]+", "", group).casefold()
    if not group.isupper() and normalized_group not in _KNOWN_LOWERCASE_RELEASE_GROUPS:
        return False
    tokens = [token.upper() for token in re.split(r"[^A-Za-z0-9]+", group) if token]
    if not tokens or any(token in _TECHNICAL_RELEASE_TOKENS for token in tokens):
        return False
    return True


def _normalized_tag_name(value: str) -> str:
    return re.sub(r"[^A-Z0-9]+", "_", value.upper()).strip("_")


def _tag_scope(tag: ET.Element) -> str:
    targets = tag.find("Targets")
    if targets is None:
        return "global"
    for field, scope in (
        ("TrackUID", "track_tag"), ("ChapterUID", "chapter_tag"),
        ("AttachmentUID", "attachment_tag"), ("EditionUID", "edition_tag"),
    ):
        if any((child.text or "").strip() for child in targets.findall(field)):
            return scope
    return "global"


def _scrub_tag_tree(root: ET.Element, text_blacklist: Sequence[str] = ()) -> list[dict[str, Any]]:
    events: list[tuple[ET.Element, ET.Element, str, str, str, bool]] = []

    def visit(parent: ET.Element, scope: str) -> None:
        for simple in list(parent.findall("Simple")):
            name = (simple.findtext("Name") or "").strip()
            text = simple.find("String")
            if _normalized_tag_name(name) in _RELEASE_GROUP_TAG_NAMES and text is not None:
                before = text.text or ""
                events.append((parent, simple, scope, name or "RELEASE_GROUP", before, True))
                continue
            if text is not None and text.text is not None:
                events.append((parent, simple, scope, name or "text", text.text, False))
            visit(simple, scope)

    for tag in root.findall("Tag"):
        visit(tag, _tag_scope(tag))

    editable = [event for event in events if not event[5]]
    cleaned = clean_metadata_texts([event[4] for event in editable], text_blacklist)
    cleaned_iter = iter(cleaned)
    changes: list[dict[str, Any]] = []
    for parent, simple, scope, field, before, remove in events:
        if remove:
            parent.remove(simple)
            changes.append({"scope": scope, "field": field, "before": before, "after": ""})
            continue
        after, rules = next(cleaned_iter)
        if after != before:
            text = simple.find("String")
            if text is not None:
                text.text = after
            changes.append({"scope": scope, "field": field, "before": before, "after": after, "rules": rules})
    return changes


def _scrub_chapter_tree(
    root: ET.Element,
    text_blacklist: Sequence[str] = (),
) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    parents = {child: parent for parent in root.iter() for child in parent}
    atoms = list(root.iter("ChapterAtom"))
    targets: list[tuple[int, str, ET.Element, str]] = []
    for chapter_index, atom in enumerate(atoms):
        start_ns = atom.findtext("ChapterTimeStart")
        for text in atom.findall(".//ChapterString"):
            nearest_atom = parents.get(text)
            while nearest_atom is not None and nearest_atom.tag != "ChapterAtom":
                nearest_atom = parents.get(nearest_atom)
            if nearest_atom is not atom or text.text is None:
                continue
            targets.append((chapter_index, start_ns or "", text, text.text))

    cleaned = clean_metadata_texts([target[3] for target in targets], text_blacklist)
    changes: list[dict[str, Any]] = []
    edits: list[dict[str, Any]] = []
    for (chapter_index, start_ns, text, before), (after, rules) in zip(targets, cleaned):
        if after == before:
            continue
        text.text = after
        changes.append({
            "scope": "chapter", "field": "title", "chapter_index": chapter_index,
            "chapter_start": start_ns, "before": before, "after": after, "rules": rules,
        })
        edits.append({"scope": "chapter", "chapter_index": chapter_index, "before": before, "after": after})
    return changes, edits


def _flatten_text(node: ET.Element | None) -> str | None:
    if node is None:
        return None
    values: list[str] = []
    if node.text and node.text.strip():
        values.append(node.text.strip())
    for child in node:
        if child.text and child.text.strip():
            values.append(child.text.strip())
    value = ", ".join(dict.fromkeys(values)).strip()
    return value or None


def nfo_paths_for(path: Path) -> list[Path]:
    """Return nearest series metadata, then adjacent NFOs in override order."""
    candidates: list[Path] = []
    parent = path.parent
    for _ in range(24):
        series = [parent / "tvshow.nfo", parent / "tvshow.hevc-rebuild.nfo"]
        found = [candidate for candidate in series if candidate.is_file()]
        if found:
            candidates.extend(found)
            break
        if parent.parent == parent:
            break
        parent = parent.parent
    candidates.extend([
        path.with_suffix(".nfo"),
        path.with_name(f"{path.stem}.hevc-rebuild.nfo"),
    ])
    return list(dict.fromkeys(candidate for candidate in candidates if candidate.is_file()))


def nfo_path_for(path: Path) -> Path | None:
    """Compatibility helper returning the conventional sidecar when present."""
    candidates = nfo_paths_for(path)
    return candidates[-1] if candidates else None


def _is_global_tag(tag: ET.Element) -> bool:
    targets = tag.find("Targets")
    if targets is None:
        return True
    targeted_ids = {"TrackUID", "ChapterUID", "AttachmentUID", "EditionUID"}
    return not any(child.tag in targeted_ids and (child.text or "").strip() for child in targets)


def _read_existing_global_root(tools: Toolchain, path: Path, work_dir: Path) -> ET.Element:
    """Extract true global Matroska tags so rebuilding does not discard source metadata."""
    root = ET.Element("Tags")
    if path.suffix.lower() != ".mkv":
        return root
    extracted = work_dir / "source-tags.xml"
    cp = run([tools.mkvextract, str(path), "tags", str(extracted)], check=False)
    if cp.returncode != 0 or not extracted.exists() or extracted.stat().st_size == 0:
        return root
    try:
        parsed = ET.parse(extracted).getroot()
    except (ET.ParseError, OSError) as exc:
        logging.warning("Existing Matroska tags could not be parsed and will not be merged: %s", exc)
        return root
    for tag in list(parsed.findall("Tag")):
        if _is_global_tag(tag):
            root.append(tag)
    return root


def _remove_conflicting_simple_tags(root: ET.Element, names: set[str]) -> None:
    wanted = {n.upper() for n in names}
    for tag in root.findall("Tag"):
        if not _is_global_tag(tag):
            continue
        for simple in list(tag.findall("Simple")):
            name = simple.findtext("Name", default="").strip().upper()
            if name in wanted:
                tag.remove(simple)
        if tag.find("Simple") is None:
            root.remove(tag)


def build_tags(
    tools: Toolchain,
    path: Path,
    work_dir: Path,
    *,
    comment: str,
    text_blacklist: Sequence[str] = (),
    metadata_override: dict[str, object] | None = None,
) -> tuple[Path, str, Path | None, dict[str, str], list[dict[str, Any]]]:
    nfos = nfo_paths_for(path)
    nfo = nfos[-1] if nfos else None
    values: dict[str, str] = {}
    changes: list[dict[str, Any]] = []
    for candidate_nfo in nfos:
        try:
            root = ET.parse(candidate_nfo).getroot()
            for source, target in TAG_MAP.items():
                # Kodi may repeat elements such as <genre> and <studio>. Preserve all
                # values in source order instead of silently taking only the first.
                found = root.findall(source)
                parts = [v for node in found if (v := _flatten_text(node))]
                value = ", ".join(dict.fromkeys(parts)).strip() if parts else None
                if value:
                    if source == "mpaa":
                        value = re.sub(r"^Rated\s+", "", value, flags=re.I).strip()
                    elif source == "userrating":
                        try:
                            rating = float(value)
                        except ValueError as exc:
                            raise RebuildError(f"Kodi NFO contains an invalid audience rating ({candidate_nfo}).") from exc
                        if not 0.0 <= rating <= 10.0:
                            raise RebuildError(f"Kodi NFO audience rating must be between 0 and 10 ({candidate_nfo}).")
                        # Matroska RATING is a 0-5 scale; Kodi userrating is 0-10.
                        value = f"{rating / 2:.2f}".rstrip("0").rstrip(".")
                    values[target] = value
        except (ET.ParseError, OSError) as exc:
            raise RebuildError(f"Kodi NFO exists but cannot be parsed safely ({candidate_nfo}): {exc}") from exc

    override = metadata_override or {}
    override_tag_names = {
        "title": "TITLE",
        "show_title": "SHOWTITLE",
        "certification": "LAW_RATING",
        "short_description": "SUMMARY",
        "plot": "SYNOPSIS",
        "season": "SEASON",
        "episode": "EPISODE",
        "source_url": "SOURCE",
    }
    for source_name, target_name in override_tag_names.items():
        if source_name in override:
            value = override[source_name]
            if not isinstance(value, str):
                raise RebuildError(f"Reviewed metadata field {source_name!r} must be text.")
            values[target_name] = value.strip()
    if "year" in override or "airdate" in override:
        date_value = override.get("airdate") or override.get("year") or ""
        if not isinstance(date_value, str):
            raise RebuildError("Reviewed year and air date values must be text.")
        values["DATE_RELEASED"] = date_value.strip()
    if "genres" in override:
        genres = override["genres"]
        if not isinstance(genres, list) or any(not isinstance(item, str) for item in genres):
            raise RebuildError("Reviewed genres must be a list of text values.")
        values["GENRE"] = ", ".join(dict.fromkeys(item.strip() for item in genres if item.strip()))
    if "rating" in override:
        rating_value = override["rating"]
        if not isinstance(rating_value, str):
            raise RebuildError("Reviewed rating must be text.")
        if rating_value.strip():
            try:
                rating = float(rating_value)
            except ValueError as exc:
                raise RebuildError("Reviewed rating must be a number from 0 to 10.") from exc
            if not 0.0 <= rating <= 10.0:
                raise RebuildError("Reviewed rating must be a number from 0 to 10.")
            values["RATING"] = f"{rating / 2:.2f}".rstrip("0").rstrip(".")
        else:
            values["RATING"] = ""

    if "comment" in override:
        comment_value = override["comment"]
        if not isinstance(comment_value, str):
            raise RebuildError("Reviewed comment must be text.")
        values["COMMENT"] = comment_value.strip()
    elif comment:
        values["COMMENT"] = comment

    raw_title = values.get("TITLE", path.stem)
    non_title_values = [(name, value) for name, value in values.items() if name != "TITLE"]
    cleaned_fields = clean_metadata_texts(
        [value for _, value in non_title_values] + [raw_title],
        text_blacklist,
    )
    for (name, value), (after, rules) in zip(non_title_values, cleaned_fields):
        if after != value:
            values[name] = after
            changes.append({"scope": "global", "field": name, "before": value, "after": after, "rules": rules})
    title, title_rules = cleaned_fields[-1]
    if title != raw_title:
        changes.append({"scope": "global", "field": "TITLE", "before": raw_title, "after": title, "rules": title_rules})
        changes.append({"scope": "segment", "field": "title", "before": raw_title, "after": title, "rules": title_rules})
    values["TITLE"] = title

    # Better than v0.107: preserve pre-existing *global* Matroska tags, while Kodi
    # values intentionally win for the fields this utility manages.
    tags = _read_existing_global_root(tools, path, work_dir)
    _remove_conflicting_simple_tags(tags, set(values))
    tag = ET.SubElement(tags, "Tag")
    ET.SubElement(tag, "Targets")
    for name, value in values.items():
        if not value:
            continue
        simple = ET.SubElement(tag, "Simple")
        ET.SubElement(simple, "Name").text = name
        ET.SubElement(simple, "String").text = value

    changes.extend(_scrub_tag_tree(tags, text_blacklist))

    xml_path = work_dir / "merged-global-tags.xml"
    ET.ElementTree(tags).write(xml_path, encoding="utf-8", xml_declaration=True)
    expected_tags = {name: value for name, value in values.items() if value}
    return xml_path, title, nfo, expected_tags, changes


def read_global_simple_tags(tools: Toolchain, path: Path) -> dict[str, list[str]]:
    """Read global Matroska Simple tags for post-write validation."""
    with tempfile.TemporaryDirectory(prefix="hevc-rebuild-tags-") as tmp:
        out = Path(tmp) / "tags.xml"
        cp = run([tools.mkvextract, str(path), "tags", str(out)], check=False)
        if cp.returncode != 0 or not out.exists():
            return {}
        try:
            root = ET.parse(out).getroot()
        except (ET.ParseError, OSError):
            return {}
        result: dict[str, list[str]] = {}
        for tag in root.findall("Tag"):
            if not _is_global_tag(tag):
                continue
            for simple in tag.findall("Simple"):
                name = simple.findtext("Name", default="").strip().upper()
                value = simple.findtext("String", default="").strip()
                if name and value:
                    result.setdefault(name, []).append(value)
        return result

def _poster_candidates(profile: MediaProfile) -> list[Path]:
    path = profile.path
    candidates: list[Path] = []
    for ext in (".jpg", ".jpeg", ".png", ".webp"):
        candidates.append(path.with_name(path.stem + "-poster" + ext))
    # TV/season conventions inherited from the original utility plus common modern names.
    parent = path.parent
    if re.match(r"(?i)^season\s*\d+", parent.name):
        series = parent.parent
        normalized = re.sub(r"\s+", "", parent.name).lower()
        for ext in (".jpg", ".jpeg", ".png", ".webp"):
            candidates.extend([
                parent / f"poster{ext}", parent / f"folder{ext}",
                series / f"{normalized}-poster{ext}",
                series / f"{parent.name}-poster{ext}",
            ])
        # A season-specific image takes precedence; use the series-level folder art
        # only when no season poster is present.
        for ext in (".jpg", ".jpeg", ".png", ".webp"):
            candidates.extend([
                series / f"folder{ext}", series / f"poster{ext}", series / f"cover{ext}",
                series / f"{series.name}-poster{ext}",
            ])
    return candidates


def _inside(path: Path, directory: Path) -> bool:
    try:
        path.resolve().relative_to(directory.resolve())
    except (OSError, ValueError):
        return False
    return True


def choose_poster(profile: MediaProfile, override: Path | None = None) -> Path | None:
    """Return only library artwork that explicitly matches the media/season.

    HEVC Rebuild never invents or supplies fallback artwork. If no recognized local
    poster exists, None is returned so any already-embedded attachment copied from
    the source remains untouched. An explicit poster choice wins over adjacent art
    for a movie. For an episode, poster art inside that season folder wins over a
    choice that points at the series folder.
    """
    present = [path for path in _poster_candidates(profile) if path.exists() and path.is_file()]
    parent = profile.path.parent
    if re.match(r"(?i)^season\s*\d+", parent.name):
        season_art = next((path for path in present if _inside(path, parent)), None)
        if season_art is not None and (override is None or not _inside(override, parent)):
            return season_art
    if override is not None:
        return override
    return present[0] if present else None


def poster_to_jpeg(tools: Toolchain, poster: Path, work_dir: Path) -> Path:
    out = work_dir / "cover.jpg"
    # Decode and re-encode even JPEG input so corrupt/mislabeled artwork cannot be
    # attached under a false image/jpeg MIME type. This also normalizes PNG/WebP.
    run([
        tools.ffmpeg, "-hide_banner", "-loglevel", "error", "-y", "-i", str(poster),
        "-frames:v", "1", "-q:v", "2", str(out),
    ])
    if not out.exists() or out.stat().st_size < 128:
        raise RebuildError(f"Failed to prepare poster artwork from {poster}")
    return out


def _poster_attachment_description(poster: Path) -> str:
    """Keep selected artwork attribution with its eventual Matroska attachment."""
    attribution_path = poster.with_name(poster.name + ".attribution.json")
    if not attribution_path.exists():
        return "Cover art"
    try:
        if attribution_path.stat().st_size > 16 * 1024:
            raise RebuildError(f"Artwork attribution is too large to embed safely ({attribution_path}).")
        value = json.loads(attribution_path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        raise RebuildError(f"Selected poster attribution could not be read safely ({attribution_path}): {exc}") from exc
    if not isinstance(value, dict):
        raise RebuildError(f"Selected poster attribution has an invalid structure ({attribution_path}).")
    details: list[str] = []
    for key, prefix in (
        ("title", "Artwork"), ("creator", "Creator"), ("attribution", "Credit"),
        ("license", "License"), ("sourceUrl", "Source"), ("licenseUrl", "License details"),
    ):
        item = value.get(key)
        if isinstance(item, str) and item.strip():
            if any(ord(char) < 32 for char in item):
                raise RebuildError(f"Selected poster attribution contains control text ({attribution_path}).")
            clean = " ".join(item.split())
            details.append(f"{prefix}: {clean}")
    if not details:
        raise RebuildError(f"Selected poster attribution contains no credit or license information ({attribution_path}).")
    description = "Cover art · " + " · ".join(details)
    return description[:2048]


def mkv_identify(tools: Toolchain, path: Path) -> dict[str, Any]:
    return run_json([tools.mkvmerge, "-J", str(path)])


def _extract_xml(tools: Toolchain, path: Path, mode: str, destination: Path) -> ET.Element | None:
    cp = run([tools.mkvextract, str(path), mode, str(destination)], check=False)
    if cp.returncode != 0:
        raise RebuildError(f"Could not safely inspect Matroska {mode} metadata (mkvextract exit {cp.returncode}).")
    if not destination.exists():
        return None
    try:
        return ET.parse(destination).getroot()
    except (ET.ParseError, OSError) as exc:
        raise RebuildError(f"Could not safely parse Matroska {mode} metadata: {exc}") from exc


def _write_xml(root: ET.Element, destination: Path) -> None:
    ET.ElementTree(root).write(destination, encoding="utf-8", xml_declaration=True)


def _scrub_output_text_metadata(
    tools: Toolchain,
    output: Path,
    work_dir: Path,
    text_blacklist: Sequence[str] = (),
) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    changes: list[dict[str, Any]] = []
    chapter_edits: list[dict[str, Any]] = []

    tags_path = work_dir / "output-tags.xml"
    tags = _extract_xml(tools, output, "tags", tags_path)
    if tags is not None:
        tag_changes = _scrub_tag_tree(tags, text_blacklist)
        if tag_changes:
            sanitized_path = work_dir / "sanitized-output-tags.xml"
            _write_xml(tags, sanitized_path)
            run([tools.mkvpropedit, str(output), "--tags", f"all:{sanitized_path}"])
            changes.extend(tag_changes)

    chapters_path = work_dir / "output-chapters.xml"
    chapters = _extract_xml(tools, output, "chapters", chapters_path)
    if chapters is not None:
        chapter_changes, chapter_edits = _scrub_chapter_tree(chapters, text_blacklist)
        if chapter_changes:
            sanitized_path = work_dir / "sanitized-output-chapters.xml"
            _write_xml(chapters, sanitized_path)
            run([tools.mkvpropedit, str(output), "--chapters", str(sanitized_path)])
            changes.extend(chapter_changes)

    # Matroska track names live in track headers, not in the XML tag tree.
    identify = mkv_identify(tools, output)
    track_actions: list[str] = [tools.mkvpropedit, str(output)]
    track_targets: list[tuple[int, str]] = []
    for track_position, track in enumerate(identify.get("tracks", []) or [], start=1):
        properties = track.get("properties") or {}
        before = properties.get("track_name")
        if not isinstance(before, str):
            continue
        track_targets.append((track_position, before))

    cleaned_tracks = clean_metadata_texts([target[1] for target in track_targets], text_blacklist)
    for (track_position, before), (after, rules) in zip(track_targets, cleaned_tracks):
        if after == before:
            continue
        track_actions.extend(["--edit", f"track:{track_position}", "--set", f"name={after}"])
        changes.append({
            "scope": "track_header", "field": "title", "track_index": track_position - 1,
            "track_type": str(track.get("type") or "unknown"), "before": before, "after": after, "rules": rules,
        })
    if len(track_actions) > 2:
        run(track_actions)
    return changes, chapter_edits


def apply_metadata(
    tools: Toolchain,
    output: Path,
    profile: MediaProfile,
    work_dir: Path,
    *,
    comment: str,
    text_blacklist: Sequence[str] = (),
    poster_override: Path | None = None,
    metadata_override: dict[str, object] | None = None,
    embed_artwork: bool = True,
) -> MetadataResult:
    xml_path, title, nfo, expected_tags, changes = build_tags(
        tools, profile.path, work_dir, comment=comment, text_blacklist=text_blacklist,
        metadata_override=metadata_override,
    )
    poster = choose_poster(profile, poster_override) if embed_artwork else None
    cover = poster_to_jpeg(tools, poster, work_dir) if poster else None

    # Replace only global tags; preserve track-specific tags/statistics copied from the source.
    args = [tools.mkvpropedit, str(output), "--edit", "info", "--set", f"title={title}", "--tags", f"global:{xml_path}"]
    run(args)

    if cover:
        ident = mkv_identify(tools, output)
        # Better than v0.107: remove only cover/poster-like image attachments, not unrelated image attachments.
        delete_ids: list[int] = []
        for att in ident.get("attachments", []) or []:
            name = str(att.get("file_name") or "")
            mime = str(att.get("content_type") or "")
            if mime.startswith("image/") and re.search(r"(?i)(cover|poster|folder|front)", name):
                try:
                    delete_ids.append(int(att["id"]))
                except (KeyError, TypeError, ValueError):
                    pass
        if delete_ids:
            run([tools.mkvpropedit, str(output), *sum((["--delete-attachment", str(i)] for i in delete_ids), [])])
        run([
            tools.mkvpropedit, str(output), "--attachment-name", "cover.jpg",
            "--attachment-mime-type", "image/jpeg", "--attachment-description", _poster_attachment_description(poster),
            "--add-attachment", str(cover),
        ])
    output_changes, chapter_edits = _scrub_output_text_metadata(tools, output, work_dir, text_blacklist)
    changes.extend(output_changes)
    return MetadataResult(
        xml_path=xml_path,
        title=title,
        nfo_path=nfo,
        poster_path=poster,
        expected_global_tags=expected_tags,
        scrubbed_text_values=changes,
        chapter_text_edits=chapter_edits,
    )
