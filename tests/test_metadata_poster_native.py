# ./tests/test_metadata_poster_native.py
"""Parity: Rust `internal metadata-poster` vs Python poster selection and attachment.

Run with the Rust helper plus ffmpeg and MKVToolNix (otherwise skipped):
    HEVC_REBUILD_NATIVE_HELPER=desktop/src-tauri/target/debug/hevc-rebuild-studio \
        python3 -m unittest tests.test_metadata_poster_native -v
Selection and description cases build throwaway library trees. The attach pass copies
tests/fixtures/local/clip15.mkv (gitignored; see tests/test_metadata_scrub_native.py)
or HEVC_REBUILD_TEST_CLIP, plants cover-like and unrelated attachments, and requires
Python and Rust to leave identical attachments and untouched stream payloads.
"""
from __future__ import annotations

import json
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

from hevc_rebuild.util import RebuildError
from hevc_rebuild.metadata import (
    _poster_attachment_description,
    attach_cover,
    choose_poster,
    poster_to_jpeg,
)
from hevc_rebuild.model import Toolchain

TOOLS = {name: shutil.which(name) for name in ("ffmpeg", "mkvmerge", "mkvextract", "mkvpropedit")}


def _helper() -> str | None:
    value = os.environ.get("HEVC_REBUILD_NATIVE_HELPER")
    return value if value and os.name == "posix" else None


def _real_clip() -> Path | None:
    explicit = os.environ.get("HEVC_REBUILD_TEST_CLIP")
    candidate = Path(explicit) if explicit else Path(__file__).parent / "fixtures" / "local" / "clip15.mkv"
    return candidate if candidate.is_file() else None


def _run(args: list[str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(args, capture_output=True, text=True, check=True, timeout=120)


def _native(request: dict) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [str(_helper()), "internal", "metadata-poster"],
        input=json.dumps(request), capture_output=True, text=True, check=False, timeout=120,
    )


def _touch(path: Path, content: bytes = b"art") -> Path:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(content)
    return path


def _error_prefix(message: str) -> str:
    """Python and serde word JSON syntax errors differently; compare up to the path."""
    marker = "): "
    return message.split(marker, 1)[0] + (")" if marker in message else "")


def _toolchain() -> Toolchain:
    return Toolchain(
        str(TOOLS["ffmpeg"]), "ffprobe", "mediainfo", str(TOOLS["mkvpropedit"]),
        str(TOOLS["mkvmerge"]), str(TOOLS["mkvextract"]), "hdr10plus_tool",
    )


def _tool_block(output: Path, work: Path) -> dict:
    return {
        "ffmpeg": TOOLS["ffmpeg"], "output": str(output), "work_dir": str(work),
        "tools": {
            "mkvmerge": TOOLS["mkvmerge"], "mkvextract": TOOLS["mkvextract"],
            "mkvpropedit": TOOLS["mkvpropedit"],
        },
    }


def _attachments(path: Path) -> list[dict]:
    identify = json.loads(_run([str(TOOLS["mkvmerge"]), "-J", str(path)]).stdout)
    records = []
    for item in identify.get("attachments", []):
        properties = {k: v for k, v in (item.get("properties") or {}).items() if k != "uid"}
        records.append({
            "id": item["id"], "file_name": item.get("file_name"), "content_type": item.get("content_type"),
            "description": item.get("description"), "size": item.get("size"), "properties": properties,
        })
    return records


def _payload_hash(path: Path, selector: str) -> str:
    result = _run([str(TOOLS["ffmpeg"]), "-v", "error", "-i", str(path), "-map", selector, "-c", "copy", "-f", "md5", "-"])
    return result.stdout.strip()


@unittest.skipUnless(_helper(), "Rust helper path on POSIX is required")
class NativePosterSelectionParityTests(unittest.TestCase):
    def _assert_selection(self, media: Path, override: Path | None = None) -> str | None:
        python_poster = choose_poster(SimpleNamespace(path=media), override)  # type: ignore[arg-type]
        python_description: str | None
        python_error: str | None = None
        try:
            python_description = _poster_attachment_description(python_poster) if python_poster else None
        except RebuildError as exc:
            python_description, python_error = None, str(exc)
        completed = _native({"media_path": str(media), "poster_override": str(override) if override else None})
        if python_error is not None:
            self.assertNotEqual(completed.returncode, 0, python_error)
            self.assertEqual(_error_prefix(completed.stderr.strip()), _error_prefix(python_error))
            return python_error
        self.assertEqual(completed.returncode, 0, completed.stderr)
        native = json.loads(completed.stdout)
        self.assertEqual(native["poster"], str(python_poster) if python_poster else None)
        self.assertEqual(native["description"], python_description)
        self.assertFalse(native["attached"])
        return python_description

    def test_movie_and_season_layouts_match_python(self):
        with tempfile.TemporaryDirectory(prefix="hevc-poster-select-") as temporary:
            root = Path(temporary)
            movie = _touch(root / "Movies" / "Heat (1995).mkv")
            self.assertIsNone(self._assert_selection(movie))
            for extension in (".webp", ".png", ".jpeg", ".jpg"):
                _touch(root / "Movies" / f"Heat (1995)-poster{extension}")
                self._assert_selection(movie)
            explicit = _touch(root / "elsewhere" / "chosen.png")
            self._assert_selection(movie, explicit)

            episode = _touch(root / "TV" / "Show Name" / "Season 02" / "S02E01.mkv")
            self.assertIsNone(self._assert_selection(episode))
            _touch(root / "TV" / "Show Name" / "Show Name-poster.png")
            self._assert_selection(episode)
            _touch(root / "TV" / "Show Name" / "folder.webp")
            self._assert_selection(episode)
            _touch(root / "TV" / "Show Name" / "season02-poster.jpeg")
            self._assert_selection(episode)
            series_choice = _touch(root / "TV" / "Show Name" / "chosen.jpg")
            self._assert_selection(episode, series_choice)
            _touch(root / "TV" / "Show Name" / "Season 02" / "folder.png")
            self._assert_selection(episode, series_choice)
            season_choice = _touch(root / "TV" / "Show Name" / "Season 02" / "mine.png")
            self._assert_selection(episode, season_choice)
            # A folder that merely starts with the word is not a season folder.
            lookalike = _touch(root / "TV" / "Other" / "Seasons 1" / "E1.mkv")
            _touch(root / "TV" / "Other" / "Seasons 1" / "poster.jpg")
            self.assertIsNone(self._assert_selection(lookalike))
            spaced = _touch(root / "TV" / "Spaced" / "SEASON  3" / "E1.mkv")
            _touch(root / "TV" / "Spaced" / "season3-poster.png")
            self._assert_selection(spaced)

    def test_attribution_descriptions_and_failures_match_python(self):
        with tempfile.TemporaryDirectory(prefix="hevc-poster-credit-") as temporary:
            root = Path(temporary)
            media = _touch(root / "Movie.mkv")
            poster = _touch(root / "Movie-poster.jpg")
            sidecar = root / "Movie-poster.jpg.attribution.json"
            self.assertEqual(self._assert_selection(media), "Cover art")
            cases: list[bytes | str] = [
                json.dumps({
                    "title": "  The   Pied\u00a0\u2003Piper ", "creator": "Éloïse “Q”", "attribution": "Public domain",
                    "license": "CC0 1.0", "sourceUrl": "https://example.org/a", "licenseUrl": " ",
                }),
                json.dumps({"title": "x" * 5000}),
                json.dumps({"title": "é" * 3000}),
                json.dumps({"title": 7, "license": "MIT"}),
                json.dumps({"title": "bad\u0007text"}),
                json.dumps({"title": "tab\tinside is control"}),
                json.dumps({"unrelated": "x", "title": "   "}),
                "{}",
                "[]",
                "not json",
                b"\xff\xfe\x00",
                b" " * (16 * 1024 + 1),
            ]
            for index, content in enumerate(cases):
                with self.subTest(case=index):
                    if isinstance(content, bytes):
                        sidecar.write_bytes(content)
                    else:
                        sidecar.write_text(content, encoding="utf-8")
                    self._assert_selection(media)
            self.assertTrue(poster.exists())

    def test_native_never_modifies_library_art(self):
        with tempfile.TemporaryDirectory(prefix="hevc-poster-readonly-") as temporary:
            root = Path(temporary)
            media = _touch(root / "Movie.mkv")
            poster = _touch(root / "Movie-poster.png", b"original-art")
            before = poster.read_bytes()
            completed = _native({"media_path": str(media)})
            self.assertEqual(completed.returncode, 0, completed.stderr)
            self.assertEqual(poster.read_bytes(), before)
            self.assertEqual(sorted(path.name for path in root.iterdir()), ["Movie-poster.png", "Movie.mkv"])

    def test_native_rejects_unknown_fields_and_bad_attach_target(self):
        with tempfile.TemporaryDirectory(prefix="hevc-poster-reject-") as temporary:
            root = Path(temporary)
            media = _touch(root / "Movie.mkv")
            _touch(root / "Movie-poster.jpg")
            self.assertNotEqual(_native({"media_path": str(media), "surprise": True}).returncode, 0)
            request = {"media_path": str(media), "attach": _tool_block(root / "missing.mkv", root)}
            result = _native(request)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("existing file", result.stderr)


@unittest.skipUnless(
    _helper() and all(TOOLS.values()) and _real_clip(),
    "Rust helper, ffmpeg, MKVToolNix, and tests/fixtures/local/clip15.mkv are required",
)
class NativePosterAttachParityTests(unittest.TestCase):
    def test_real_clip_attachment_replacement_matches_python(self):
        clip = _real_clip()
        assert clip is not None
        clip_hash = _payload_hash(clip, "0:v:0")
        with tempfile.TemporaryDirectory(prefix="hevc-poster-attach-") as temporary:
            root = Path(temporary)
            media = root / "Show" / "Season 04" / "S04E03.mkv"
            media.parent.mkdir(parents=True)
            shutil.copyfile(clip, media)
            poster = media.parent / "poster.png"
            _run([
                str(TOOLS["ffmpeg"]), "-v", "error", "-y", "-f", "lavfi", "-i", "testsrc=size=200x300:rate=1:duration=1",
                "-frames:v", "1", str(poster),
            ])
            poster_before = poster.read_bytes()
            (media.parent / "poster.png.attribution.json").write_text(
                json.dumps({"title": "Test Card", "creator": "Éloïse", "license": "CC0 1.0"}), encoding="utf-8",
            )
            planted = root / "plant"
            planted.mkdir()
            for name, mime, body in (
                ("Cover.JPG", "image/jpeg", b"old-cover-bytes"),
                ("front.png", "image/png", b"old-front-bytes"),
                ("banner.png", "image/png", b"keep-banner"),
                ("poster.txt", "text/plain", b"keep-text"),
            ):
                (planted / name).write_bytes(body)
                _run([
                    str(TOOLS["mkvpropedit"]), str(media), "--attachment-name", name,
                    "--attachment-mime-type", mime, "--add-attachment", str(planted / name),
                ])
            python_file = root / "python.mkv"
            rust_file = root / "rust.mkv"
            shutil.copyfile(media, python_file)
            shutil.copyfile(media, rust_file)
            planted_view = _attachments(rust_file)
            self.assertEqual(len(planted_view), 4)
            before = {sel: _payload_hash(rust_file, sel) for sel in ("0:v:0", "0:a:0")}

            python_work, rust_work = root / "py-work", root / "rs-work"
            python_work.mkdir()
            rust_work.mkdir()
            tools = _toolchain()
            with patch.dict(os.environ):
                os.environ.pop("HEVC_REBUILD_NATIVE_HELPER", None)
                chosen = choose_poster(SimpleNamespace(path=media), None)  # type: ignore[arg-type]
                assert chosen is not None
                description = _poster_attachment_description(chosen)
                cover = poster_to_jpeg(tools, chosen, python_work)
                deleted = attach_cover(tools, python_file, cover, description)

            completed = _native({"media_path": str(media), "attach": _tool_block(rust_file, rust_work)})
            self.assertEqual(completed.returncode, 0, completed.stderr)
            native = json.loads(completed.stdout)
            self.assertEqual(native["poster"], str(poster))
            self.assertEqual(native["description"], description)
            self.assertTrue(native["attached"])
            self.assertEqual(native["deleted_attachment_ids"], deleted)
            self.assertEqual(sorted(deleted), [1, 2], "cover-like image attachments only")
            self.assertEqual((rust_work / "cover.jpg").read_bytes(), (python_work / "cover.jpg").read_bytes())

            py_attachments, rs_attachments = _attachments(python_file), _attachments(rust_file)
            self.assertEqual(rs_attachments, py_attachments)
            names = [item["file_name"] for item in rs_attachments]
            self.assertEqual(sorted(names), ["banner.png", "cover.jpg", "poster.txt"])
            cover_record = next(item for item in rs_attachments if item["file_name"] == "cover.jpg")
            self.assertEqual(cover_record["content_type"], "image/jpeg")
            self.assertTrue(cover_record["description"].startswith("Cover art · Artwork: Test Card"))
            self.assertEqual({sel: _payload_hash(rust_file, sel) for sel in before}, before)
            self.assertEqual(_payload_hash(clip, "0:v:0"), clip_hash, "the original clip must never change")
            self.assertEqual(poster.read_bytes(), poster_before, "library art must never change")

    def test_unreadable_art_fails_before_touching_the_output(self):
        clip = _real_clip()
        assert clip is not None
        with tempfile.TemporaryDirectory(prefix="hevc-poster-corrupt-") as temporary:
            root = Path(temporary)
            media = root / "Movie.mkv"
            shutil.copyfile(clip, media)
            _touch(root / "Movie-poster.jpg", b"definitely not an image" * 20)
            before = media.read_bytes()
            work = root / "work"
            work.mkdir()
            completed = _native({"media_path": str(media), "attach": _tool_block(media, work)})
            self.assertNotEqual(completed.returncode, 0)
            self.assertEqual(media.read_bytes(), before)


if __name__ == "__main__":
    unittest.main()
