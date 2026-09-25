from __future__ import annotations

import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

from hevc_rebuild.model import Toolchain
from hevc_rebuild.validate import packet_fingerprints, stream_copy_hash


@unittest.skipUnless(shutil.which("ffmpeg"), "ffmpeg not installed")
class CopyHashTests(unittest.TestCase):
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


if __name__ == "__main__":
    unittest.main()
