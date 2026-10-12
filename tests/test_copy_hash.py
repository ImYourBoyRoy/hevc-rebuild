from __future__ import annotations

import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from hevc_rebuild.model import Toolchain
from hevc_rebuild.util import RebuildError
from hevc_rebuild.validate import packet_fingerprints, stream_copy_hash


@unittest.skipUnless(shutil.which("ffmpeg"), "ffmpeg not installed")
class CopyHashTests(unittest.TestCase):
    @unittest.skipUnless(os.environ.get("HEVC_REBUILD_NATIVE_HELPER"), "Rust helper path not configured")
    def test_rust_stream_fingerprints_match_python_reference(self):
        ffmpeg = shutil.which("ffmpeg") or "ffmpeg"
        ffprobe = shutil.which("ffprobe") or "ffprobe"
        tools = Toolchain(ffmpeg, ffprobe, "mediainfo", "mkvpropedit", "mkvmerge", "mkvextract", "hdr10plus_tool")
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "source.mka"
            subprocess.run(
                [ffmpeg, "-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", "sine=frequency=660:duration=0.7", "-c:a", "flac", str(source)],
                check=True,
            )
            native_helper = os.environ["HEVC_REBUILD_NATIVE_HELPER"]
            with patch.dict(os.environ):
                os.environ.pop("HEVC_REBUILD_NATIVE_HELPER", None)
                python_fingerprints = packet_fingerprints(tools, source, ["a:0"])
                python_hash = stream_copy_hash(tools, source, "a:0")
            with patch.dict(os.environ, {"HEVC_REBUILD_NATIVE_HELPER": native_helper}):
                rust_fingerprints = packet_fingerprints(tools, source, ["a:0"])
                rust_hash = stream_copy_hash(tools, source, "a:0")
            self.assertEqual(rust_fingerprints, python_fingerprints)
            self.assertEqual(rust_hash, python_hash)

    def test_audio_streamcopy_hash_survives_remux(self):
        ffmpeg = shutil.which("ffmpeg") or "ffmpeg"
        ffprobe = shutil.which("ffprobe") or "ffprobe"
        tools = Toolchain(ffmpeg, ffprobe, "mediainfo", "mkvpropedit", "mkvmerge", "mkvextract", "hdr10plus_tool")
        with tempfile.TemporaryDirectory() as td:
            td = Path(td)
            src = td / "src.mka"
            dst = td / "dst.mka"
            subprocess.run([ffmpeg, "-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", "sine=frequency=440:duration=0.5", "-c:a", "flac", str(src)], check=True)
            subprocess.run([ffmpeg, "-hide_banner", "-loglevel", "error", "-i", str(src), "-map", "0:a", "-c", "copy", str(dst)], check=True)
            self.assertEqual(stream_copy_hash(tools, src, "a:0"), stream_copy_hash(tools, dst, "a:0"))
            self.assertEqual(packet_fingerprints(tools, src, ["a:0"]), packet_fingerprints(tools, dst, ["a:0"]))

    def test_subtitle_streamcopy_hash_survives_remux(self):
        ffmpeg = shutil.which("ffmpeg") or "ffmpeg"
        ffprobe = shutil.which("ffprobe") or "ffprobe"
        tools = Toolchain(ffmpeg, ffprobe, "mediainfo", "mkvpropedit", "mkvmerge", "mkvextract", "hdr10plus_tool")
        with tempfile.TemporaryDirectory() as td:
            td = Path(td)
            srt = td / "source.srt"
            srt.write_text("1\n00:00:00,000 --> 00:00:01,000\nExact subtitle payload\n", encoding="utf-8")
            src = td / "src.mkv"
            dst = td / "dst.mkv"
            subprocess.run([ffmpeg, "-hide_banner", "-loglevel", "error", "-i", str(srt), "-c:s", "srt", str(src)], check=True)
            subprocess.run([ffmpeg, "-hide_banner", "-loglevel", "error", "-i", str(src), "-map", "0:s", "-c", "copy", str(dst)], check=True)
            self.assertEqual(stream_copy_hash(tools, src, "s:0"), stream_copy_hash(tools, dst, "s:0"))
            self.assertEqual(packet_fingerprints(tools, src, ["s:0"]), packet_fingerprints(tools, dst, ["s:0"]))


class NativeVerifierDispatchTests(unittest.TestCase):
    def test_configured_but_missing_rust_helper_fails_closed(self):
        tools = Toolchain("ffmpeg", "ffprobe", "mediainfo", "mkvpropedit", "mkvmerge", "mkvextract", "hdr10plus_tool")
        with tempfile.TemporaryDirectory() as directory:
            missing = Path(directory) / "missing-rust-helper"
            with patch.dict(os.environ, {"HEVC_REBUILD_NATIVE_HELPER": str(missing)}):
                with self.assertRaisesRegex(RebuildError, "Could not start the Rust stream verifier"):
                    packet_fingerprints(tools, Path("not-opened.mkv"), ["a:0"])

    def test_incomplete_rust_fingerprint_payload_fails_closed(self):
        tools = Toolchain("ffmpeg", "ffprobe", "mediainfo", "mkvpropedit", "mkvmerge", "mkvextract", "hdr10plus_tool")
        completed = subprocess.CompletedProcess([], 0, '{"fingerprints":[]}', "")
        with patch.dict(os.environ, {"HEVC_REBUILD_NATIVE_HELPER": "configured-helper"}):
            with patch("hevc_rebuild.validate.subprocess.run", return_value=completed):
                with self.assertRaisesRegex(RebuildError, "incomplete stream fingerprint set"):
                    packet_fingerprints(tools, Path("not-opened.mkv"), ["a:0"])


if __name__ == "__main__":
    unittest.main()
