from __future__ import annotations

import json
import os
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

from hevc_rebuild.model import StreamRef, Toolchain
from hevc_rebuild.util import RebuildError
from hevc_rebuild.youtube import (
    _hdr10plus_loss_decision,
    _install_youtube_output,
    _upload_args,
    _validate_upload,
    _video_rate_kbps,
    youtube_output_estimate,
)


def profile(path: Path, *, width: int = 1280, height: int = 720, duration: float = 1000.0):
    return SimpleNamespace(
        path=path,
        duration=duration,
        audio=[object()],
        primary_video=StreamRef(index=0, type_ordinal=0, codec_type="video", width=width, height=height),
        probe={"streams": [{"index": 0, "avg_frame_rate": "30/1", "side_data_list": []}]},
        transfer=None,
        hdr10=False,
        hdr10plus_hint=False,
        hlg=False,
        dolby_vision=False,
        source_bit_depth=8,
        color_primaries=None,
        color_matrix=None,
        color_range_x265=None,
        color_range_ffmpeg=None,
        master_display=None,
        max_cll=None,
        max_fall=None,
        interlaced=False,
    )


class YouTubeBitrateTests(unittest.TestCase):
    def test_balanced_multiplier_changes_video_rate_and_estimate(self):
        with tempfile.TemporaryDirectory() as td:
            source = Path(td) / "source.mkv"
            with source.open("wb") as handle:
                handle.truncate(100_000_000)
            media = profile(source)

            low = _video_rate_kbps(media, "1440p", "balanced", 1.5)
            high = _video_rate_kbps(media, "1440p", "balanced", 2.0)
            estimate = youtube_output_estimate(media, "1440p", "balanced", 2.0)

            self.assertEqual(low, 816)
            self.assertEqual(high, 1216)
            self.assertEqual(estimate["video_bitrate_kbps"], high)
            self.assertEqual((estimate["output_width"], estimate["output_height"]), (2560, 1440))

    def test_balanced_rate_keeps_minimum_floor_and_recommendation_ceiling(self):
        with tempfile.TemporaryDirectory() as td:
            source = Path(td) / "source.mkv"
            with source.open("wb") as handle:
                handle.truncate(10_000_000)
            media = profile(source)
            self.assertEqual(_video_rate_kbps(media, "1440p", "balanced", 0.5), 250)

            with source.open("wb") as handle:
                handle.truncate(100_000_000)
            media = profile(source, width=2560, height=1440, duration=100)
            self.assertEqual(_video_rate_kbps(media, "1440p", "balanced", 4.0), 16_000)

    def test_recommended_mode_ignores_balanced_multiplier(self):
        with tempfile.TemporaryDirectory() as td:
            source = Path(td) / "source.mkv"
            source.touch()
            media = profile(source)
            self.assertEqual(_video_rate_kbps(media, "1440p", "recommended", 0.5), 16_000)

    def test_invalid_balanced_multiplier_is_rejected(self):
        with tempfile.TemporaryDirectory() as td:
            source = Path(td) / "source.mkv"
            source.touch()
            with self.assertRaises(RebuildError):
                _video_rate_kbps(profile(source), "1440p", "balanced", 4.01)


class YouTubeHdr10PlusConsentTests(unittest.TestCase):
    def test_hdr10plus_source_is_skipped_without_file_specific_consent(self):
        with tempfile.TemporaryDirectory() as td:
            media = profile(Path(td) / "hdr10plus.mkv")
            media.hdr10plus_hint = True
            allowed, note = _hdr10plus_loss_decision(media, accepted=False)
            self.assertFalse(allowed)
            self.assertIn("No copy was written", note)
            self.assertIn("individual file", note)

    def test_explicit_consent_names_the_dynamic_metadata_loss(self):
        with tempfile.TemporaryDirectory() as td:
            media = profile(Path(td) / "hdr10plus.mkv")
            media.hdr10plus_hint = True
            allowed, note = _hdr10plus_loss_decision(media, accepted=True)
            self.assertTrue(allowed)
            self.assertIn("explicit per-file approval", note)
            self.assertIn("static HDR10/PQ", note)

    def test_sdr_or_hdr10_source_needs_no_hdr10plus_consent(self):
        with tempfile.TemporaryDirectory() as td:
            allowed, note = _hdr10plus_loss_decision(profile(Path(td) / "source.mkv"), accepted=False)
            self.assertTrue(allowed)
            self.assertIsNone(note)


class NativeYouTubePlanTests(unittest.TestCase):
    helper = os.environ.get("HEVC_REBUILD_NATIVE_HELPER")

    @unittest.skipUnless(helper, "Rust helper path not configured")
    def test_rust_policy_matches_python_size_estimates(self):
        with tempfile.TemporaryDirectory() as td:
            source = Path(td) / "source.mkv"
            with source.open("wb") as handle:
                handle.truncate(80_000_000)
            media = profile(source)
            with patch.dict(os.environ, {"HEVC_REBUILD_NATIVE_HELPER": self.helper}):
                native = youtube_output_estimate(media, "1440p", "balanced", 2.0)
            with patch.dict(os.environ):
                os.environ.pop("HEVC_REBUILD_NATIVE_HELPER", None)
                python_reference = youtube_output_estimate(media, "1440p", "balanced", 2.0)
            self.assertEqual(native, python_reference)

    @unittest.skipUnless(helper, "Rust helper path not configured")
    def test_rust_builds_source_aware_sdr_ffmpeg_arguments(self):
        with tempfile.TemporaryDirectory() as td:
            source = Path(td) / "source.mkv"
            source.touch()
            media = profile(source)
            tools = Toolchain(
                ffmpeg="ffmpeg-custom",
                ffprobe="ffprobe-custom",
                mediainfo="mediainfo",
                mkvpropedit="mkvpropedit",
                mkvmerge="mkvmerge",
                mkvextract="mkvextract",
                hdr10plus_tool="hdr10plus_tool",
                x265_pixel_formats=set(),
            )
            with patch.dict(os.environ, {"HEVC_REBUILD_NATIVE_HELPER": self.helper}):
                command = _upload_args(
                    tools,
                    media,
                    Path(td) / "output.mp4",
                    resolution="1440p",
                    bitrate_kbps=750,
                    worker_threads=4,
                    frame_threads=0,
                    color_mode="sdr",
                )
            self.assertEqual(command[0], "ffmpeg-custom")
            self.assertIn("0:0", command)
            self.assertIn("-c:v:0", command)
            self.assertEqual(command[command.index("-c:v:0") + 1], "libx264")
            self.assertIn("scale=2560:1440:force_original_aspect_ratio=decrease:flags=lanczos", command)

    @unittest.skipUnless(helper, "Rust helper path not configured")
    def test_rust_validator_uses_configured_ffprobe_and_checks_mp4_output(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            output = root / "output.mp4"
            with output.open("wb") as handle:
                handle.truncate(2048)
            ffprobe = root / "ffprobe-fixture"
            probe = {
                "streams": [
                    {"codec_type": "video", "codec_name": "h264", "width": 2560, "height": 1440, "pix_fmt": "yuv420p"},
                    {"codec_type": "audio", "codec_name": "aac"},
                ],
                "format": {"duration": "1000", "format_name": "mov,mp4,m4a"},
            }
            ffprobe.write_text(
                "#!/usr/bin/env python3\nimport json\nprint(json.dumps(" + repr(probe) + "))\n",
                encoding="utf-8",
            )
            ffprobe.chmod(0o755)
            media = profile(root / "source.mkv", width=1280, height=720, duration=1000)
            tools = Toolchain(
                ffmpeg="ffmpeg",
                ffprobe=str(ffprobe),
                mediainfo="mediainfo",
                mkvpropedit="mkvpropedit",
                mkvmerge="mkvmerge",
                mkvextract="mkvextract",
                hdr10plus_tool="hdr10plus_tool",
            )
            with patch.dict(os.environ, {"HEVC_REBUILD_NATIVE_HELPER": self.helper}):
                self.assertEqual(_validate_upload(tools, media, output, "1440p"), (2560, 1440))

    @unittest.skipUnless(helper, "Rust helper path not configured")
    def test_rust_installs_validated_copy_without_overwriting(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            temporary = root / "temporary.mp4"
            output = root / "output.mp4"
            temporary.write_bytes(b"validated")
            with patch.dict(os.environ, {"HEVC_REBUILD_NATIVE_HELPER": self.helper}):
                _install_youtube_output(temporary, output)
            self.assertFalse(temporary.exists())
            self.assertEqual(output.read_bytes(), b"validated")
            second = root / "second.mp4"
            second.write_bytes(b"must remain temporary")
            with patch.dict(os.environ, {"HEVC_REBUILD_NATIVE_HELPER": self.helper}):
                with self.assertRaises(RebuildError):
                    _install_youtube_output(second, output)
            self.assertTrue(second.exists())
            self.assertEqual(output.read_bytes(), b"validated")


if __name__ == "__main__":
    unittest.main()
