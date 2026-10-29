# ./tests/test_metadata_scrub_native.py
"""Real-file parity: Rust `internal metadata-scrub` vs Python `_scrub_output_text_metadata`.

Needs the Rust helper plus ffmpeg and MKVToolNix; skipped otherwise.
    HEVC_REBUILD_NATIVE_HELPER=desktop/src-tauri/target/debug/hevc-rebuild-studio \
        python3 -m unittest tests.test_metadata_scrub_native -v
The real-clip pass uses tests/fixtures/local/clip15.mkv (gitignored; make it with
tools/extract_clip.py ... --duration 15s) or HEVC_REBUILD_TEST_CLIP. The clip is only ever copied; the same
dirty tags, chapters, and track names are applied to two copies, Python scrubs one
and Rust the other, and results plus untouched stream payload hashes must agree.
"""
from __future__ import annotations

import json
import os
import shutil
import subprocess
import tempfile
import unittest
import xml.etree.ElementTree as ET
from pathlib import Path
from unittest.mock import patch

from hevc_rebuild.metadata import _scrub_output_text_metadata
from hevc_rebuild.model import Toolchain

TOOLS = {name: shutil.which(name) for name in ("ffmpeg", "mkvmerge", "mkvextract", "mkvpropedit")}
BLACKLIST = ["Movie*Group"]

CHAPTERS = """<?xml version="1.0" encoding="UTF-8"?>
<Chapters><EditionEntry>
  <ChapterAtom><ChapterTimeStart>00:00:00.000000000</ChapterTimeStart>
    <ChapterDisplay><ChapterString>Intro MovieReleaseGroup</ChapterString><ChapterLanguage>eng</ChapterLanguage></ChapterDisplay></ChapterAtom>
  <ChapterAtom><ChapterTimeStart>00:00:01.000000000</ChapterTimeStart>
    <ChapterDisplay><ChapterString>Scene.1080p.WEB-DL.x265-GROUP</ChapterString><ChapterLanguage>eng</ChapterLanguage></ChapterDisplay></ChapterAtom>
  <ChapterAtom><ChapterTimeStart>00:00:02.000000000</ChapterTimeStart>
    <ChapterDisplay><ChapterString>Plain title &amp; more</ChapterString><ChapterLanguage>eng</ChapterLanguage></ChapterDisplay></ChapterAtom>
</EditionEntry></Chapters>"""

TAGS = """<?xml version="1.0" encoding="UTF-8"?>
<Tags><Tag><Targets/>
  <Simple><Name>COMMENT</Name><String>Keep Movieshowgroup text</String></Simple>
  <Simple><Name>RELEASE_GROUP</Name><String>GROUP</String></Simple>
  <Simple><Name>NOTE</Name><String>Film.1080p.WEB-DL.x265-GROUP</String>
    <Simple><Name>NESTED</Name><String>Nested &lt;ok&gt; MovieReleaseGroup</String></Simple></Simple>
</Tag></Tags>"""


def _real_clip() -> Path | None:
    explicit = os.environ.get("HEVC_REBUILD_TEST_CLIP")
    candidate = Path(explicit) if explicit else Path(__file__).parent / "fixtures" / "local" / "clip15.mkv"
    return candidate if candidate.is_file() else None


def _helper() -> str | None:
    value = os.environ.get("HEVC_REBUILD_NATIVE_HELPER")
    return value if value and os.name == "posix" else None


def _run(args: list[str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(args, capture_output=True, text=True, check=True, timeout=120)


def _payload_hash(path: Path, selector: str) -> str:
    result = _run([str(TOOLS["ffmpeg"]), "-v", "error", "-i", str(path), "-map", selector, "-c", "copy", "-f", "md5", "-"])
    return result.stdout.strip()


def _identify(path: Path) -> dict:
    return json.loads(_run([str(TOOLS["mkvmerge"]), "-J", str(path)]).stdout)


def _extract(path: Path, mode: str, work: Path) -> ET.Element | None:
    target = work / f"{mode}-{path.stem}.xml"
    _run([str(TOOLS["mkvextract"]), str(path), mode, str(target)])
    return ET.parse(target).getroot() if target.exists() else None


def _shape(node: ET.Element | None) -> object:
    if node is None:
        return None
    children = [_shape(child) for child in node]
    text = node.text
    if children and (text is None or not text.strip()):
        text = None
    return node.tag, text, children


def _strip_uids(node: ET.Element | None) -> ET.Element | None:
    """Matroska generates random UIDs; they are identity, not content."""
    if node is None:
        return None
    for parent in list(node.iter()):
        for child in list(parent):
            if child.tag.endswith("UID"):
                parent.remove(child)
    return node


def _synthetic_clip(path: Path) -> None:
    raw = path.with_suffix(".raw.mkv")
    _run([
        str(TOOLS["ffmpeg"]), "-v", "error", "-y",
        "-f", "lavfi", "-i", "testsrc=size=160x90:rate=10:duration=3",
        "-f", "lavfi", "-i", "sine=frequency=440:duration=3",
        "-c:v", "libx265", "-preset", "ultrafast", "-x265-params", "log-level=none",
        "-c:a", "aac", "-shortest", str(raw),
    ])
    _run([str(TOOLS["mkvmerge"]), "-o", str(path), str(raw)])
    raw.unlink()


def _make_dirty(path: Path, work: Path) -> None:
    (work / "dirty-chapters.xml").write_text(CHAPTERS, encoding="utf-8")
    (work / "dirty-tags.xml").write_text(TAGS, encoding="utf-8")
    _run([
        str(TOOLS["mkvpropedit"]), str(path),
        "--chapters", str(work / "dirty-chapters.xml"),
        "--tags", f"global:{work / 'dirty-tags.xml'}",
        "--edit", "track:1", "--set", "name=Video.1080p.WEB-DL.x265-GROUP",
        "--edit", "track:2", "--set", "name=Stereo MovieReleaseGroup",
    ])


def _scrub_request(output: Path, work: Path) -> str:
    return json.dumps({
        "tools": {
            "mkvmerge": TOOLS["mkvmerge"], "mkvextract": TOOLS["mkvextract"],
            "mkvpropedit": TOOLS["mkvpropedit"],
        },
        "output": str(output), "work_dir": str(work), "text_blacklist": BLACKLIST,
    })


@unittest.skipUnless(
    _helper() and all(TOOLS.values()),
    "Rust helper path, POSIX, ffmpeg, and MKVToolNix are required",
)
class NativeOutputScrubParityTests(unittest.TestCase):
    def _native(self, output: Path, work: Path) -> dict:
        completed = subprocess.run(
            [str(_helper()), "internal", "metadata-scrub"],
            input=_scrub_request(output, work), capture_output=True, text=True, check=False, timeout=120,
        )
        self.assertEqual(completed.returncode, 0, completed.stderr)
        return json.loads(completed.stdout)

    def _compare(self, source: Path, label: str) -> None:
        with tempfile.TemporaryDirectory(prefix="hevc-scrub-parity-") as temporary:
            root = Path(temporary)
            python_file = root / f"python-{label}.mkv"
            rust_file = root / f"rust-{label}.mkv"
            shutil.copyfile(source, python_file)
            _make_dirty(python_file, root)
            shutil.copyfile(python_file, rust_file)
            before_hashes = {sel: _payload_hash(rust_file, sel) for sel in ("0:v:0", "0:a:0")}

            python_work = root / "py-work"
            rust_work = root / "rs-work"
            python_work.mkdir()
            rust_work.mkdir()
            toolchain = Toolchain(
                "ffmpeg", "ffprobe", "mediainfo", str(TOOLS["mkvpropedit"]),
                str(TOOLS["mkvmerge"]), str(TOOLS["mkvextract"]), "hdr10plus_tool",
            )
            with patch.dict(os.environ):
                os.environ.pop("HEVC_REBUILD_NATIVE_HELPER", None)
                py_changes, py_edits = _scrub_output_text_metadata(toolchain, python_file, python_work, BLACKLIST)
            native = self._native(rust_file, rust_work)

            self.assertTrue(py_changes, "fixture must exercise the scrub")
            self.assertEqual(native["changes"], py_changes)
            self.assertEqual(native["chapter_edits"], py_edits)
            scopes = {change["scope"] for change in py_changes}
            self.assertTrue({"global", "chapter", "track_header"} <= scopes, scopes)
            self.assertEqual(
                [(c["track_index"], c["track_type"]) for c in native["changes"] if c["scope"] == "track_header"],
                [(0, "video"), (1, "audio")],
            )

            py_tracks = [t["properties"].get("track_name") for t in _identify(python_file)["tracks"]]
            rs_tracks = [t["properties"].get("track_name") for t in _identify(rust_file)["tracks"]]
            self.assertEqual(rs_tracks, py_tracks)
            self.assertEqual(rs_tracks[:2], ["Video.1080p.WEB-DL.x265", "Stereo"])
            self.assertEqual(
                _shape(_strip_uids(_extract(rust_file, "chapters", root))),
                _shape(_strip_uids(_extract(python_file, "chapters", root))),
            )
            rs_tags = _extract(rust_file, "tags", root)
            self.assertEqual(_shape(_strip_uids(rs_tags)), _shape(_strip_uids(_extract(python_file, "tags", root))))
            assert rs_tags is not None
            texts = [node.text for node in rs_tags.iter("String")]
            self.assertNotIn("GROUP", texts)
            self.assertIn("Keep text", texts)
            self.assertIn("Nested <ok>", [text.strip() for text in texts if text])

            after_hashes = {sel: _payload_hash(rust_file, sel) for sel in before_hashes}
            self.assertEqual(after_hashes, before_hashes, "stream payload must be untouched")
            self.assertEqual(self._native(rust_file, rust_work)["changes"], [], "scrub must be idempotent")

    def test_synthetic_clip_matches_python(self):
        with tempfile.TemporaryDirectory(prefix="hevc-scrub-source-") as temporary:
            source = Path(temporary) / "synthetic.mkv"
            _synthetic_clip(source)
            self._compare(source, "synthetic")

    @unittest.skipUnless(_real_clip(), "put a short MKV at tests/fixtures/local/clip15.mkv or set HEVC_REBUILD_TEST_CLIP")
    def test_real_clip_matches_python(self):
        source = _real_clip()
        assert source is not None
        source_hash = _payload_hash(source, "0:v:0")
        self._compare(source, "real")
        self.assertEqual(_payload_hash(source, "0:v:0"), source_hash, "the original clip must never change")

    def test_rust_refuses_missing_output_without_touching_anything(self):
        with tempfile.TemporaryDirectory() as temporary:
            result = subprocess.run(
                [str(_helper()), "internal", "metadata-scrub"],
                input=json.dumps({
                    "tools": {"mkvmerge": "mkvmerge", "mkvextract": "mkvextract", "mkvpropedit": "mkvpropedit"},
                    "output": str(Path(temporary) / "missing.mkv"), "work_dir": temporary,
                }),
                capture_output=True, text=True, check=False, timeout=30,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("existing file", result.stderr)


if __name__ == "__main__":
    unittest.main()
