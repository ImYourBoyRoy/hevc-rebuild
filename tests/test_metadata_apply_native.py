# ./tests/test_metadata_apply_native.py
"""Parity: Rust `internal metadata-apply` vs Python `apply_metadata` on real files.

Run with the Rust helper plus ffmpeg and MKVToolNix (otherwise skipped):
    HEVC_REBUILD_NATIVE_HELPER=desktop/src-tauri/target/debug/hevc-rebuild-studio \
        python3 -m unittest tests.test_metadata_apply_native -v
A throwaway library with Unicode series/episode names, ignored sidecar NFOs,
season poster art with attribution, and a copy of tests/fixtures/local/clip15.mkv
(gitignored) or a synthetic clip is built per case. The same pre-dirtied copy goes
through Python and Rust; MetadataResult fields, written tags/chapters/track names,
attachments, and untouched stream payloads must agree.
"""
from __future__ import annotations

import hashlib
import json
import os
import shutil
import subprocess
import tempfile
import unittest
import xml.etree.ElementTree as ET
from dataclasses import asdict
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

from hevc_rebuild.metadata import apply_metadata
from hevc_rebuild.model import Toolchain

TOOLS = {name: shutil.which(name) for name in ("ffmpeg", "mkvmerge", "mkvextract", "mkvpropedit")}
SERIES = "Série Ñandú ✨"
EPISODE = "S04E03 – Café Ünïcode.mkv"
BLACKLIST = ["Movie*Group"]

SERIES_NFO = f"""<?xml version="1.0" encoding="UTF-8"?>
<tvshow><title>Ignored sidecar</title><genre>Ignored</genre></tvshow>"""
EPISODE_NFO = """<?xml version="1.0" encoding="UTF-8"?>
<episodedetails><title>Ignored episode sidecar</title><userrating>1</userrating></episodedetails>"""
REVIEWED_EPISODE = {
    "title": "Café – The Piper.1080p.WEB-DL.x265-GROUP",
    "show_title": SERIES,
    "plot": "Plot MovieReleaseGroup text “quoted” 😀",
    "short_description": "Override tagline",
    "genres": ["Drama", "Fantasy"],
    "certification": "TV-14",
    "rating": "7.5",
    "airdate": "1987-05-01",
    "season": "4",
    "episode": "3",
}
CHAPTERS = """<?xml version="1.0" encoding="UTF-8"?>
<Chapters><EditionEntry>
  <ChapterAtom><ChapterTimeStart>00:00:00.000000000</ChapterTimeStart>
    <ChapterDisplay><ChapterString>Intro MovieReleaseGroup</ChapterString><ChapterLanguage>eng</ChapterLanguage></ChapterDisplay></ChapterAtom>
  <ChapterAtom><ChapterTimeStart>00:00:01.000000000</ChapterTimeStart>
    <ChapterDisplay><ChapterString>Café 1080p.WEB-DL.x265-GROUP</ChapterString><ChapterLanguage>eng</ChapterLanguage></ChapterDisplay></ChapterAtom>
</EditionEntry></Chapters>"""
SOURCE_TAGS = """<?xml version="1.0" encoding="UTF-8"?>
<Tags><Tag><Targets/>
  <Simple><Name>ENCODER_NOTE</Name><String>Keep me</String></Simple>
  <Simple><Name>TITLE</Name><String>Old title to replace</String></Simple>
  <Simple><Name>NOTE</Name><String>Film.1080p.WEB-DL.x265-GROUP</String></Simple>
</Tag></Tags>"""


def _helper() -> str | None:
    value = os.environ.get("HEVC_REBUILD_NATIVE_HELPER")
    return value if value and os.name == "posix" else None


def _real_clip() -> Path | None:
    explicit = os.environ.get("HEVC_REBUILD_TEST_CLIP")
    candidate = Path(explicit) if explicit else Path(__file__).parent / "fixtures" / "local" / "clip15.mkv"
    return candidate if candidate.is_file() else None


def _run(args: list[str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(args, capture_output=True, text=True, check=True, timeout=180)


def _toolchain() -> Toolchain:
    return Toolchain(
        str(TOOLS["ffmpeg"]), "ffprobe", "mediainfo", str(TOOLS["mkvpropedit"]),
        str(TOOLS["mkvmerge"]), str(TOOLS["mkvextract"]), "hdr10plus_tool",
    )


def _tool_paths() -> dict:
    return {
        "mkvmerge": TOOLS["mkvmerge"], "mkvextract": TOOLS["mkvextract"], "mkvpropedit": TOOLS["mkvpropedit"],
    }


def _synthetic(path: Path) -> None:
    raw = path.with_suffix(".raw.mkv")
    _run([
        str(TOOLS["ffmpeg"]), "-v", "error", "-y", "-f", "lavfi", "-i", "testsrc=size=160x90:rate=10:duration=3",
        "-f", "lavfi", "-i", "sine=frequency=440:duration=3", "-c:v", "libx265", "-preset", "ultrafast",
        "-x265-params", "log-level=none", "-c:a", "aac", "-shortest", str(raw),
    ])
    _run([str(TOOLS["mkvmerge"]), "-o", str(path), str(raw)])
    raw.unlink()


def _payload(path: Path, selector: str) -> str:
    result = _run([str(TOOLS["ffmpeg"]), "-v", "error", "-i", str(path), "-map", selector, "-c", "copy", "-f", "md5", "-"])
    return result.stdout.strip()


def _strip_uids(node: ET.Element | None) -> ET.Element | None:
    if node is None:
        return None
    for parent in list(node.iter()):
        for child in list(parent):
            if child.tag.endswith("UID"):
                parent.remove(child)
    return node


def _shape(node: ET.Element | None) -> object:
    if node is None:
        return None
    children = [_shape(child) for child in node]
    text = node.text
    if children and (text is None or not text.strip()):
        text = None
    return node.tag, text, children


def _snapshot(path: Path, work: Path) -> dict:
    identify = json.loads(_run([str(TOOLS["mkvmerge"]), "-J", str(path)]).stdout)
    state: dict = {
        "title": identify["container"]["properties"].get("title"),
        "tracks": [(t["type"], t["properties"].get("track_name")) for t in identify["tracks"]],
        "attachments": [
            (a["file_name"], a["content_type"], a.get("description"), a.get("size")) for a in identify.get("attachments", [])
        ],
    }
    for mode in ("tags", "chapters"):
        target = work / f"{mode}.xml"
        target.unlink(missing_ok=True)
        _run([str(TOOLS["mkvextract"]), str(path), mode, str(target)])
        state[mode] = _shape(_strip_uids(ET.parse(target).getroot())) if target.exists() else None
    state["video"] = _payload(path, "0:v:0")
    state["audio"] = _payload(path, "0:a:0") if any(t[0] == "audio" for t in state["tracks"]) else None
    return state


def _normalize(value: object, replacements: list[tuple[str, str]]) -> object:
    text = json.dumps(value, sort_keys=True, ensure_ascii=False, default=str)
    for before, after in replacements:
        text = text.replace(before, after)
    return json.loads(text)


@unittest.skipUnless(_helper() and all(TOOLS.values()), "Rust helper path, POSIX, ffmpeg, and MKVToolNix are required")
class NativeApplyParityTests(unittest.TestCase):
    def _library(self, root: Path, clip: Path, *, with_art: bool = True) -> Path:
        season = root / SERIES / "Season 04"
        season.mkdir(parents=True)
        media = season / EPISODE
        shutil.copyfile(clip, media)
        (root / SERIES / "tvshow.nfo").write_text(SERIES_NFO, encoding="utf-8")
        (season / EPISODE.replace(".mkv", ".nfo")).write_text(EPISODE_NFO, encoding="utf-8")
        if with_art:
            _run([
                str(TOOLS["ffmpeg"]), "-v", "error", "-y", "-f", "lavfi", "-i", "testsrc=size=200x300:rate=1:duration=1",
                "-frames:v", "1", str(season / "poster.png"),
            ])
            (season / "poster.png.attribution.json").write_text(
                json.dumps({"title": "Carte d’essai", "creator": "Éloïse", "license": "CC0 1.0"}), encoding="utf-8",
            )
        return media

    def _dirty_output(self, media: Path, root: Path) -> Path:
        work = root / "dirty"
        work.mkdir()
        (work / "c.xml").write_text(CHAPTERS, encoding="utf-8")
        (work / "t.xml").write_text(SOURCE_TAGS, encoding="utf-8")
        _run([
            str(TOOLS["mkvpropedit"]), str(media), "--chapters", str(work / "c.xml"), "--tags", f"global:{work / 't.xml'}",
            "--edit", "track:1", "--set", "name=Video.1080p.WEB-DL.x265-GROUP",
        ])
        for name, mime, body in (("Cover.JPG", "image/jpeg", b"old"), ("banner.png", "image/png", b"keep")):
            (work / name).write_bytes(body)
            _run([
                str(TOOLS["mkvpropedit"]), str(media), "--attachment-name", name, "--attachment-mime-type", mime,
                "--add-attachment", str(work / name),
            ])
        return media

    def _run_both(self, source: Path, root: Path, **options: object) -> tuple[dict, dict, dict, dict]:
        python_out, rust_out = root / "python.mkv", root / "rust.mkv"
        shutil.copyfile(source, python_out)
        shutil.copyfile(source, rust_out)
        python_work, rust_work = root / "py-work", root / "rs-work"
        python_work.mkdir()
        rust_work.mkdir()
        before = {"video": _payload(rust_out, "0:v:0")}
        comment = str(options.get("comment", "Processed by HEVC Rebuild"))
        override = options.get("metadata_override", REVIEWED_EPISODE)
        embed = bool(options.get("embed_artwork", True))
        profile = SimpleNamespace(path=source)
        with patch.dict(os.environ):
            os.environ.pop("HEVC_REBUILD_NATIVE_HELPER", None)
            python_result = apply_metadata(
                _toolchain(), python_out, profile, python_work,  # type: ignore[arg-type]
                comment=comment, text_blacklist=BLACKLIST, metadata_override=override,  # type: ignore[arg-type]
                embed_artwork=embed,
            )
        completed = subprocess.run(
            [str(_helper()), "internal", "metadata-apply"],
            input=json.dumps({
                "media_path": str(source), "output": str(rust_out), "work_dir": str(rust_work),
                "ffmpeg": TOOLS["ffmpeg"], "tools": _tool_paths(), "comment": comment,
                "text_blacklist": BLACKLIST, "metadata_override": override, "embed_artwork": embed,
            }),
            capture_output=True, text=True, check=False, timeout=300,
        )
        self.assertEqual(completed.returncode, 0, completed.stderr)
        native = json.loads(completed.stdout)
        python_dict = asdict(python_result)
        replacements = [(str(python_work), "<WORK>"), (str(rust_work), "<WORK>")]
        python_view = _normalize(python_dict, replacements)
        native_view = _normalize({k: v for k, v in native.items() if k != "warnings"}, replacements)
        self.assertEqual(before["video"], _payload(rust_out, "0:v:0"))
        return (
            python_view, native_view,
            _snapshot(python_out, python_work), _snapshot(rust_out, rust_work),
        )  # type: ignore[return-value]

    def _compare(self, clip: Path, label: str, **options: object) -> dict:
        with tempfile.TemporaryDirectory(prefix=f"hevc-apply-{label}-") as temporary:
            root = Path(temporary)
            media = self._library(root / "lib", clip, with_art=bool(options.pop("with_art", True)))
            source_before = hashlib.sha256(media.read_bytes()).hexdigest()
            self._dirty_output(media, root)
            dirty_before = hashlib.sha256(media.read_bytes()).hexdigest()
            py_result, rs_result, py_state, rs_state = self._run_both(media, root, **options)
            self.assertEqual(rs_result, py_result)
            self.assertEqual(rs_state, py_state)
            self.assertEqual(hashlib.sha256(media.read_bytes()).hexdigest(), dirty_before, "source copy is read-only")
            self.assertNotEqual(source_before, dirty_before)
            return {"result": rs_result, "state": rs_state}

    def test_synthetic_clip_with_unicode_library_matches_python(self):
        with tempfile.TemporaryDirectory(prefix="hevc-apply-source-") as temporary:
            clip = Path(temporary) / "synthetic.mkv"
            _synthetic(clip)
            outcome = self._compare(clip, "synthetic")
        self.assertEqual(outcome["result"]["title"], "Café – The Piper.1080p.WEB-DL.x265")
        self.assertEqual(outcome["result"]["expected_global_tags"]["RATING"], "3.75")
        self.assertNotIn("nfo_path", outcome["result"])
        names = [item[0] for item in outcome["state"]["attachments"]]
        self.assertEqual(sorted(names), ["banner.png", "cover.jpg"])

    @unittest.skipUnless(_real_clip(), "put a short MKV at tests/fixtures/local/clip15.mkv or set HEVC_REBUILD_TEST_CLIP")
    def test_real_clip_with_unicode_library_matches_python(self):
        clip = _real_clip()
        assert clip is not None
        outcome = self._compare(clip, "real")
        self.assertEqual(outcome["result"]["title"], "Café – The Piper.1080p.WEB-DL.x265")
        self.assertIn(SERIES, json.dumps(outcome["state"]["tags"], ensure_ascii=False))

    def test_reviewed_override_and_no_artwork_match_python(self):
        with tempfile.TemporaryDirectory(prefix="hevc-apply-override-") as temporary:
            clip = Path(temporary) / "synthetic.mkv"
            _synthetic(clip)
            outcome = self._compare(
                clip, "override", embed_artwork=False,
                metadata_override={
                    "title": "Reviewed Título", "genres": ["Drama", "Drama", " Fantasía "], "rating": "9",
                    "year": "1999", "comment": "Reviewer note",
                },
            )
        self.assertEqual(outcome["result"]["title"], "Reviewed Título")
        self.assertIsNone(outcome["result"]["poster_path"])
        names = [item[0] for item in outcome["state"]["attachments"]]
        self.assertIn("Cover.JPG", names, "embed_artwork=False must leave attachments untouched")

    def test_missing_art_leaves_existing_attachments_like_python(self):
        with tempfile.TemporaryDirectory(prefix="hevc-apply-noart-") as temporary:
            clip = Path(temporary) / "synthetic.mkv"
            _synthetic(clip)
            outcome = self._compare(clip, "noart", with_art=False)
        self.assertIsNone(outcome["result"]["poster_path"])
        self.assertIn("Cover.JPG", [item[0] for item in outcome["state"]["attachments"]])

    def test_python_adapter_routes_apply_and_verify_through_the_helper(self):
        from hevc_rebuild.util import RebuildError
        from hevc_rebuild.validate import verify_expected_output

        with tempfile.TemporaryDirectory(prefix="hevc-apply-adapter-") as temporary:
            root = Path(temporary)
            clip = root / "synthetic.mkv"
            _synthetic(clip)
            media = self._dirty_output(self._library(root / "lib", clip), root)
            outputs, results, work_dirs = {}, {}, {}
            for label, helper in (("python", None), ("native", _helper())):
                outputs[label] = root / f"{label}.mkv"
                work_dirs[label] = root / f"{label}-work"
                work_dirs[label].mkdir()
                shutil.copyfile(media, outputs[label])
                with patch.dict(os.environ):
                    os.environ.pop("HEVC_REBUILD_NATIVE_HELPER", None)
                    if helper:
                        os.environ["HEVC_REBUILD_NATIVE_HELPER"] = helper
                    results[label] = apply_metadata(
                        _toolchain(), outputs[label], SimpleNamespace(path=media),  # type: ignore[arg-type]
                        work_dirs[label], comment="adapter", text_blacklist=BLACKLIST,
                        metadata_override=REVIEWED_EPISODE,
                    )
                    verify_expected_output(
                        _toolchain(), outputs[label], expect_poster=True,
                        expected_global_tags=results[label].expected_global_tags,
                    )
                    with self.assertRaises(RebuildError) as raised:
                        verify_expected_output(
                            _toolchain(), outputs[label], expect_poster=False, expected_global_tags={"TITLE": "Nope ’x’"},
                        )
                    results[label + "-error"] = str(raised.exception)
            self.assertEqual(results["native-error"], results["python-error"])
            self.assertEqual(
                _normalize(asdict(results["native"]), [(str(work_dirs["native"]), "<W>")]),
                _normalize(asdict(results["python"]), [(str(work_dirs["python"]), "<W>")]),
            )
            self.assertEqual(
                _snapshot(outputs["native"], work_dirs["native"]), _snapshot(outputs["python"], work_dirs["python"]),
            )

    def test_malformed_nfo_sidecar_is_ignored_and_never_blocks_the_output(self):
        with tempfile.TemporaryDirectory(prefix="hevc-apply-badnfo-") as temporary:
            root = Path(temporary)
            clip = root / "synthetic.mkv"
            _synthetic(clip)
            media = self._library(root / "lib", clip)
            (media.with_suffix(".nfo")).write_text("<episodedetails><title>broken", encoding="utf-8")
            output = root / "out.mkv"
            shutil.copyfile(media, output)
            work = root / "work"
            work.mkdir()
            completed = subprocess.run(
                [str(_helper()), "internal", "metadata-apply"],
                input=json.dumps({
                    "media_path": str(media), "output": str(output), "work_dir": str(work),
                    "ffmpeg": TOOLS["ffmpeg"], "tools": _tool_paths(),
                    "metadata_override": {"title": "Reviewed title"},
                }),
                capture_output=True, text=True, check=False, timeout=120,
            )
            self.assertEqual(completed.returncode, 0, completed.stderr)
            self.assertEqual(json.loads(completed.stdout)["title"], "Reviewed title")

    def test_native_rejects_unknown_fields(self):
        completed = subprocess.run(
            [str(_helper()), "internal", "metadata-apply"], input='{"surprise":1}',
            capture_output=True, text=True, check=False, timeout=30,
        )
        self.assertNotEqual(completed.returncode, 0)


if __name__ == "__main__":
    unittest.main()
