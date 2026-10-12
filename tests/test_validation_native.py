from __future__ import annotations

import json
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from hevc_rebuild.model import MediaProfile, StreamRef, Toolchain
from hevc_rebuild.probe import choose_primary_video, stream_refs
from hevc_rebuild.util import RebuildError
from hevc_rebuild.validate import _is_replaceable_cover, validate_output


FFMPEG = shutil.which("ffmpeg")
FFPROBE = shutil.which("ffprobe")
NATIVE_HELPER = os.environ.get("HEVC_REBUILD_NATIVE_HELPER")


class ReplaceableCoverPolicyTests(unittest.TestCase):
    def test_identifies_cover_attachments_and_attached_picture_streams(self):
        def ref(codec_type: str, *, attached_pic: bool = False, filename: str, mimetype: str) -> StreamRef:
            return StreamRef(
                index=0,
                type_ordinal=0,
                codec_type=codec_type,
                attached_pic=attached_pic,
                tags={"filename": filename, "mimetype": mimetype},
            )

        self.assertTrue(_is_replaceable_cover(ref("video", attached_pic=True, filename="cover.jpg", mimetype="image/jpeg")))
        self.assertTrue(_is_replaceable_cover(ref("attachment", filename="poster.png", mimetype="image/png")))
        self.assertFalse(_is_replaceable_cover(ref("video", attached_pic=True, filename="backdrop.jpg", mimetype="image/jpeg")))
        self.assertFalse(_is_replaceable_cover(ref("video", filename="cover.jpg", mimetype="image/jpeg")))
        self.assertFalse(_is_replaceable_cover(ref("attachment", filename="font.ttf", mimetype="application/x-truetype-font")))


@unittest.skipUnless(FFMPEG and FFPROBE and NATIVE_HELPER, "FFmpeg/FFprobe or Rust helper is unavailable")
class NativeOutputValidationTests(unittest.TestCase):
    def test_rust_validates_video_audio_and_attachment_before_python_commit(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            attachment = directory / "payload.bin"
            attachment.write_bytes(os.urandom(1_100_000))
            source = directory / "source.mkv"
            output = directory / "output.mkv"
            subprocess.run(
                [
                    FFMPEG,
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-f",
                    "lavfi",
                    "-i",
                    "testsrc2=size=320x180:rate=10:duration=2",
                    "-f",
                    "lavfi",
                    "-i",
                    "sine=frequency=440:duration=2",
                    "-map",
                    "0:v:0",
                    "-map",
                    "1:a:0",
                    "-c:v",
                    "mpeg4",
                    "-q:v",
                    "3",
                    "-c:a",
                    "flac",
                    "-attach",
                    str(attachment),
                    "-metadata:s:t:0",
                    "mimetype=application/octet-stream",
                    str(source),
                ],
                check=True,
            )
            subprocess.run(
                [
                    FFMPEG,
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-i",
                    str(source),
                    "-map",
                    "0:v:0",
                    "-map",
                    "0:a:0",
                    "-map",
                    "0:t:0",
                    "-c:v",
                    "libx265",
                    "-preset",
                    "ultrafast",
                    "-crf",
                    "30",
                    "-pix_fmt",
                    "yuv420p",
                    "-c:a",
                    "copy",
                    "-c:t",
                    "copy",
                    "-map_metadata",
                    "0",
                    "-map_chapters",
                    "0",
                    "-disposition:v:0",
                    "default",
                    str(output),
                ],
                check=True,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            source_probe = json.loads(
                subprocess.run(
                    [
                        FFPROBE,
                        "-v",
                        "error",
                        "-show_streams",
                        "-show_format",
                        "-show_chapters",
                        "-show_data_hash",
                        "sha256",
                        "-of",
                        "json",
                        str(source),
                    ],
                    check=True,
                    stdout=subprocess.PIPE,
                    text=True,
                ).stdout
            )
            refs = stream_refs(source_probe)
            primary, secondary = choose_primary_video(refs)
            profile = MediaProfile(
                path=source,
                probe=source_probe,
                mediainfo={},
                primary_video=primary,
                secondary_videos=secondary,
                audio=[stream for stream in refs if stream.codec_type == "audio"],
                subtitles=[stream for stream in refs if stream.codec_type == "subtitle"],
                attachments=[stream for stream in refs if stream.codec_type == "attachment"],
                data=[stream for stream in refs if stream.codec_type == "data"],
                chapters_count=len(source_probe.get("chapters", [])),
                duration=float(source_probe["format"]["duration"]),
                source_bit_depth=8,
                source_chroma="4:2:0",
                target_pix_fmt="yuv420p",
                color_primaries=None,
                transfer=None,
                color_matrix=None,
                color_range_x265=None,
                color_range_ffmpeg=None,
                chroma_location=None,
                master_display=None,
                master_display_source=None,
                max_cll=None,
                max_fall=None,
                hdr10=False,
                hdr10plus=False,
                hdr10plus_hint=False,
                hlg=False,
                dolby_vision=False,
                dovi_profile=None,
                dovi_level=None,
                dovi_rpu_present=False,
                dovi_el_present=False,
                dovi_bl_present=False,
                dovi_compatibility_id=None,
                dovi_source=None,
                interlaced=False,
                format_level=None,
            )
            tools = Toolchain(
                FFMPEG,
                FFPROBE,
                "mediainfo",
                "mkvpropedit",
                "mkvmerge",
                "mkvextract",
                "hdr10plus_tool",
            )
            with patch.dict(os.environ, {"HEVC_REBUILD_NATIVE_HELPER": NATIVE_HELPER}), patch(
                "hevc_rebuild.validate.build_profile", return_value=profile
            ):
                result = validate_output(
                    tools,
                    profile,
                    output,
                    secondary_video_policy="drop",
                    verify_copy_hashes=True,
                    expect_poster=False,
                    test_mode=False,
                )
            self.assertTrue(result.copied_hashes_verified)
            output_refs = stream_refs(result.output_probe)
            self.assertEqual([stream.codec_name for stream in output_refs if stream.codec_type == "video"], ["hevc"])
            self.assertEqual(len([stream for stream in output_refs if stream.codec_type == "attachment"]), 1)

            damaged = directory / "damaged-audio.mkv"
            subprocess.run(
                [
                    FFMPEG,
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-i",
                    str(source),
                    "-map",
                    "0:v:0",
                    "-map",
                    "0:a:0",
                    "-map",
                    "0:t:0",
                    "-c:v",
                    "libx265",
                    "-preset",
                    "ultrafast",
                    "-crf",
                    "30",
                    "-pix_fmt",
                    "yuv420p",
                    "-c:a",
                    "flac",
                    "-af",
                    "volume=0.2",
                    "-c:t",
                    "copy",
                    "-map_metadata",
                    "0",
                    "-map_chapters",
                    "0",
                    "-disposition:v:0",
                    "default",
                    str(damaged),
                ],
                check=True,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            with patch.dict(os.environ, {"HEVC_REBUILD_NATIVE_HELPER": NATIVE_HELPER}), patch(
                "hevc_rebuild.validate.build_profile", return_value=profile
            ), self.assertRaisesRegex(RebuildError, "compressed packet sequence/extradata differs"):
                validate_output(
                    tools,
                    profile,
                    damaged,
                    secondary_video_policy="drop",
                    verify_copy_hashes=True,
                    expect_poster=False,
                    test_mode=False,
                )


if __name__ == "__main__":
    unittest.main()
