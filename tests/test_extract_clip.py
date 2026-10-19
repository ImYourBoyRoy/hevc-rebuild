from __future__ import annotations

import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

from tools.extract_clip import ClipError, ClipRequest, build_command, parse_duration


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "tools" / "extract_clip.py"
FFMPEG = shutil.which("ffmpeg")
FFPROBE = shutil.which("ffprobe")


def _has_encoder(name: str) -> bool:
    if not FFMPEG:
        return False
    result = subprocess.run(
        [FFMPEG, "-hide_banner", "-h", f"encoder={name}"],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        check=False,
    )
    return result.returncode == 0 and f"Encoder {name}" in result.stdout


class ExtractClipTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        if not FFMPEG or not FFPROBE:
            raise unittest.SkipTest("FFmpeg and FFprobe are required for clip integration tests")

    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory(prefix="hevc-extract-clip-")
        self.root = Path(self.temp.name)
        self.source = self.root / "source.mkv"
        subprocess.run(
            [
                FFMPEG,
                "-hide_banner",
                "-loglevel",
                "error",
                "-nostdin",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=320x180:rate=10:duration=4",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=48000:duration=4",
                "-map",
                "0:v",
                "-map",
                "1:a",
                "-c:v",
                "mpeg4",
                "-q:v",
                "5",
                "-c:a",
                "aac",
                "-t",
                "4",
                str(self.source),
            ],
            check=True,
        )

    def tearDown(self) -> None:
        self.temp.cleanup()

    def _run(self, *args: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, str(SCRIPT), *args],
            cwd=ROOT,
            capture_output=True,
            text=True,
            check=False,
        )

    def _probe(self, path: Path) -> dict:
        result = subprocess.run(
            [
                FFPROBE,
                "-hide_banner",
                "-loglevel",
                "error",
                "-show_entries",
                "format=duration:stream=codec_type",
                "-of",
                "json",
                str(path),
            ],
            capture_output=True,
            text=True,
            check=True,
        )
        return json.loads(result.stdout)

    def test_duration_parser_and_command_seek_contract(self) -> None:
        self.assertEqual(parse_duration("30s"), 30.0)
        self.assertEqual(parse_duration("1.5m"), 90.0)
        self.assertEqual(parse_duration("2h"), 7200.0)
        self.assertEqual(parse_duration("0s", name="start", allow_zero=True), 0.0)
        with self.assertRaises(ClipError):
            parse_duration("nan")
        with self.assertRaises(ClipError):
            parse_duration("25h")

        copy_request = ClipRequest(self.source, self.root / "copy.mkv", 2.0, 3.0, "copy", FFMPEG, 60.0)
        copy_command = build_command(copy_request, self.root / ".copy.part.mkv")
        self.assertLess(copy_command.index("-ss"), copy_command.index("-i"))
        self.assertIn(["-map", "0"], [copy_command[i : i + 2] for i in range(len(copy_command) - 1)])
        self.assertIn("-c", copy_command)
        transcode_request = copy_request.__class__(
            self.source, self.root / "transcode.mkv", 2.0, 3.0, "transcode", FFMPEG, 60.0
        )
        transcode_command = build_command(transcode_request, self.root / ".transcode.part.mkv")
        self.assertGreater(transcode_command.index("-ss"), transcode_command.index("-i"))
        self.assertIn("libx264", transcode_command)

    def test_copy_keeps_source_unchanged_and_refuses_overwrite(self) -> None:
        before = hashlib.sha256(self.source.read_bytes()).hexdigest()
        output = self.root / "copy.mkv"
        result = self._run(
            str(self.source),
            "--start",
            "1s",
            "--duration",
            "1.5s",
            "--output",
            str(output),
            "--json",
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        payload = json.loads(result.stdout)
        self.assertTrue(payload["ok"])
        self.assertEqual(payload["mode"], "copy")
        self.assertTrue(output.is_file())
        self.assertEqual(hashlib.sha256(self.source.read_bytes()).hexdigest(), before)
        probe = self._probe(output)
        self.assertEqual([stream["codec_type"] for stream in probe["streams"]], ["video", "audio"])
        self.assertGreater(float(probe["format"]["duration"]), 0.4)
        # Stream-copy seeking starts on a nearby keyframe; the requested
        # duration is a bound for the fast fixture, not a frame-accurate cut.
        self.assertLessEqual(float(probe["format"]["duration"]), 3.0)

        collision = self._run(str(self.source), "--duration", "1s", "--output", str(output), "--json")
        self.assertEqual(collision.returncode, 4)
        self.assertIn("Refusing to overwrite", collision.stdout)

    @unittest.skipUnless(_has_encoder("libx264"), "FFmpeg/libx264 is required for accurate transcode mode")
    def test_transcode_mode_has_accurate_short_duration(self) -> None:
        output = self.root / "transcode.mkv"
        result = self._run(
            str(self.source),
            "--start",
            "1.1s",
            "--duration",
            "0.75s",
            "--mode",
            "transcode",
            "--output",
            str(output),
            "--json",
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(output.is_file())
        duration = float(self._probe(output)["format"]["duration"])
        self.assertGreater(duration, 0.5)
        self.assertLess(duration, 1.1)


if __name__ == "__main__":
    unittest.main()
