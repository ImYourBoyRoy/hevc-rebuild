from __future__ import annotations

import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


FFMPEG = shutil.which("ffmpeg")
FFPROBE = shutil.which("ffprobe")
MKVMERGE = shutil.which("mkvmerge")
MKVPROPEDIT = shutil.which("mkvpropedit")
NATIVE_HELPER = os.environ.get("HEVC_REBUILD_NATIVE_HELPER")


def digest(path: Path) -> str:
    value = hashlib.sha256()
    with path.open("rb") as media:
        for block in iter(lambda: media.read(1024 * 1024), b""):
            value.update(block)
    return value.hexdigest()


@unittest.skipUnless(
    FFMPEG and FFPROBE and MKVMERGE and MKVPROPEDIT and NATIVE_HELPER and os.name == "posix",
    "FFmpeg, MKVToolNix, the Rust helper, and POSIX test scripts are required",
)
class MetadataOnlyIntegrationTests(unittest.TestCase):
    def test_copy_mode_keeps_tracks_chapters_attachments_hdr_and_original(self):
        with tempfile.TemporaryDirectory(prefix="hevc-metadata-only-") as temporary:
            root = Path(temporary)
            base = root / "base.mkv"
            source = root / "Example Film (2024).mkv"
            subtitle = root / "dialogue.srt"
            attachment = root / "font.bin"
            old_poster = root / "old-poster.jpg"
            selected_poster = root / "selected-poster.png"
            chapters = root / "chapters.xml"
            subtitle.write_text(
                "1\n00:00:00,000 --> 00:00:01,000\nFixture subtitle\n",
                encoding="utf-8",
            )
            attachment.write_bytes(b"fixture-font-payload")
            chapters.write_text(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                "<Chapters><EditionEntry>"
                "<ChapterAtom><ChapterTimeStart>00:00:00.000000000</ChapterTimeStart>"
                "<ChapterTimeEnd>00:00:01.000000000</ChapterTimeEnd>"
                "<ChapterDisplay><ChapterString>Opening</ChapterString>"
                "<ChapterLanguage>eng</ChapterLanguage></ChapterDisplay></ChapterAtom>"
                "<ChapterAtom><ChapterTimeStart>00:00:01.000000000</ChapterTimeStart>"
                "<ChapterTimeEnd>00:00:02.000000000</ChapterTimeEnd>"
                "<ChapterDisplay><ChapterString>Closing</ChapterString>"
                "<ChapterLanguage>eng</ChapterLanguage></ChapterDisplay></ChapterAtom>"
                "</EditionEntry></Chapters>",
                encoding="utf-8",
            )

            subprocess.run(
                [
                    FFMPEG,
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-f",
                    "lavfi",
                    "-i",
                    "testsrc2=size=96x64:rate=10:duration=2",
                    "-f",
                    "lavfi",
                    "-i",
                    "color=c=blue:size=80x46:rate=10:duration=2",
                    "-f",
                    "lavfi",
                    "-i",
                    "sine=frequency=440:duration=2",
                    "-map",
                    "0:v:0",
                    "-map",
                    "1:v:0",
                    "-map",
                    "2:a:0",
                    "-c:v:0",
                    "libx265",
                    "-preset:v:0",
                    "ultrafast",
                    "-pix_fmt:v:0",
                    "yuv420p10le",
                    "-x265-params:v:0",
                    "colorprim=9:transfer=16:colormatrix=9:"
                    "master-display=G(13250,34500)B(7500,3000)R(34000,16000)"
                    "WP(15635,16450)L(10000000,1):max-cll=1000,400",
                    "-color_primaries:v:0",
                    "bt2020",
                    "-color_trc:v:0",
                    "smpte2084",
                    "-colorspace:v:0",
                    "bt2020nc",
                    "-c:v:1",
                    "libx264",
                    "-preset:v:1",
                    "ultrafast",
                    "-pix_fmt:v:1",
                    "yuv420p",
                    "-c:a",
                    "aac",
                    "-shortest",
                    str(base),
                ],
                check=True,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            subprocess.run(
                [
                    FFMPEG,
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-f",
                    "lavfi",
                    "-i",
                    "color=c=black:size=64x96",
                    "-frames:v",
                    "1",
                    str(old_poster),
                ],
                check=True,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            subprocess.run(
                [
                    FFMPEG,
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-f",
                    "lavfi",
                    "-i",
                    "color=c=red:size=64x96",
                    "-frames:v",
                    "1",
                    str(selected_poster),
                ],
                check=True,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            subprocess.run(
                [
                    MKVMERGE,
                    "--output",
                    str(source),
                    str(base),
                    "--language",
                    "0:eng",
                    str(subtitle),
                    "--attachment-name",
                    "font.bin",
                    "--attachment-mime-type",
                    "application/octet-stream",
                    "--attach-file",
                    str(attachment),
                    "--attachment-name",
                    "cover.jpg",
                    "--attachment-mime-type",
                    "image/jpeg",
                    "--attach-file",
                    str(old_poster),
                    "--chapters",
                    str(chapters),
                ],
                check=True,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            subprocess.run(
                [MKVPROPEDIT, str(source), "--set", "title=Source Fixture"],
                check=True,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            source_digest = digest(source)
            metadata = {
                str(source): {
                    "title": "Reviewed Fixture Film",
                    "year": "2024",
                    "genres": ["Drama", "Test"],
                    "source_url": "https://example.invalid/title/1",
                }
            }
            posters = {str(source): str(selected_poster)}
            environment = dict(os.environ, HEVC_REBUILD_NATIVE_HELPER=NATIVE_HELPER or "")
            result = subprocess.run(
                [
                    sys.executable,
                    "-m",
                    "hevc_rebuild",
                    str(root),
                    "--metadata-only",
                    "--allow-unsupported-os",
                    "--no-dependency-repair",
                    "--skip-update-check",
                    "--min-size-mib",
                    "0",
                    "--selected-file",
                    str(source),
                    "--file-metadata-overrides-json",
                    json.dumps(metadata),
                    "--file-poster-overrides-json",
                    json.dumps(posters),
                ],
                env=environment,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                timeout=90,
            )
            self.assertEqual(result.returncode, 0, result.stdout)
            self.assertIn("1 metadata-only", result.stdout)

            output = root / "Example Film (2024).metadata.mkv"
            self.assertTrue(output.is_file(), result.stdout)
            self.assertTrue(source.is_file(), "Metadata-only output must retain its source.")
            self.assertEqual(digest(source), source_digest, "Source bytes changed during copy-only output.")
            probe = json.loads(
                subprocess.check_output(
                    [FFPROBE, "-v", "error", "-show_streams", "-show_format", "-show_chapters", "-of", "json", str(output)],
                    text=True,
                )
            )
            streams = probe["streams"]
            real_video = [
                stream
                for stream in streams
                if stream["codec_type"] == "video"
                and not int(stream.get("disposition", {}).get("attached_pic", 0))
            ]
            attached_pictures = [
                stream
                for stream in streams
                if stream["codec_type"] == "video"
                and int(stream.get("disposition", {}).get("attached_pic", 0))
            ]
            self.assertEqual(len(real_video), 2)
            self.assertEqual(len(attached_pictures), 1)
            self.assertEqual(sum(stream["codec_type"] == "audio" for stream in streams), 1)
            self.assertEqual(sum(stream["codec_type"] == "subtitle" for stream in streams), 1)
            self.assertEqual(
                (real_video[0].get("color_primaries"), real_video[0].get("color_transfer"), real_video[0].get("color_space")),
                ("bt2020", "smpte2084", "bt2020nc"),
            )
            tags = {str(key).casefold(): value for key, value in probe["format"].get("tags", {}).items()}
            self.assertEqual(tags.get("title"), "Reviewed Fixture Film")
            self.assertEqual([chapter["tags"]["title"] for chapter in probe["chapters"]], ["Opening", "Closing"])

            identify = json.loads(subprocess.check_output([MKVMERGE, "-J", str(output)], text=True))
            attachments = identify.get("attachments", [])
            self.assertCountEqual(
                [(item["file_name"], item["content_type"]) for item in attachments],
                [("font.bin", "application/octet-stream"), ("cover.jpg", "image/jpeg")],
            )


if __name__ == "__main__":
    unittest.main()
