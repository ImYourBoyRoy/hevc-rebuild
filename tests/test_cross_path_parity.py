"""Equivalent synthetic fixtures for direct Python and rebuilt Rust-launched helpers."""
import csv
import json
import os
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path
from unittest.mock import patch

from hevc_rebuild.app import commit_transaction
from hevc_rebuild.model import Toolchain
from hevc_rebuild.probe import build_native_profile_set, build_profile
from hevc_rebuild.util import RebuildError, append_csv, terminate_process_group

HELPER = os.environ.get("HEVC_REBUILD_NATIVE_HELPER")


@unittest.skipUnless(HELPER, "Rebuilt native helper required for cross-path parity")
class CrossPathParityTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.environment = os.environ.copy()
        for name in ("HEVC_REBUILD_PORTABLE_DATA_DIR", "HEVC_REBUILD_NATIVE_HELPER"):
            self.environment.pop(name, None)
        for name in ("XDG_CONFIG_HOME", "XDG_STATE_HOME", "XDG_CACHE_HOME"):
            self.environment[name] = str(self.root / name.lower())

    def command(self, native, *args):
        prefix = [HELPER, "json"] if native else [sys.executable, "-m", "hevc_rebuild.bridge"]
        return subprocess.run(prefix + list(args), env=self.environment, capture_output=True, text=True, timeout=30, check=False)

    def test_defaults_layered_boolean_and_numeric_settings_match(self):
        library = self.root / "library"
        library.mkdir()
        user = Path(self.environment["XDG_CONFIG_HOME"]) / "hevc-rebuild" / "config.toml"
        user.parent.mkdir(parents=True)
        user.write_text('[video]\ncrf = 22\n[metadata]\nembed_artwork = false\n')
        (library / ".hevc-rebuild.toml").write_text('[video]\ncrf = 25\n[content]\ndetect_animation = false\n')
        direct = self.command(False, "defaults", "--root", str(library))
        native = self.command(True, "defaults", "--root", str(library))
        self.assertEqual((direct.returncode, native.returncode), (0, 0), (direct.stderr, native.stderr))
        expected, actual = json.loads(direct.stdout), json.loads(native.stdout)
        self.assertEqual(actual["settings"], expected["settings"])
        self.assertEqual(actual["sources"], expected["sources"])
        self.assertEqual(actual["settings"]["video"]["crf"], 25)
        self.assertFalse(actual["settings"]["metadata"]["embed_artwork"])
        self.assertFalse(actual["settings"]["content"]["detect_animation"])

    def test_invalid_settings_fail_both_paths(self):
        (self.root / ".hevc-rebuild.toml").write_text('[video]\ncrf = 99\n')
        for native in (False, True):
            result = self.command(native, "defaults", "--root", str(self.root))
            self.assertNotEqual(result.returncode, 0)
            self.assertTrue(result.stderr.strip())

    def test_report_rows_schema_and_privacy_match(self):
        rows = [{"input": "synthetic/source.mkv", "status": status, "message": 'Quoted, line\n"detail"'} for status in ("OK", "FAILED", "SKIPPED", "INTERRUPTED")]
        results = []
        for native in (False, True):
            log_dir = Path(self.environment["XDG_STATE_HOME"]) / "hevc-rebuild" / "logs"
            log_dir.mkdir(parents=True, mode=0o700, exist_ok=True)
            path = log_dir / f"hevc-rebuild-parity-{native}.csv"
            with patch.dict(os.environ, self.environment | {"HEVC_REBUILD_NATIVE_HELPER": HELPER if native else ""}):
                for row in rows:
                    append_csv(path, row)
            with path.open(newline="") as stream:
                reader = csv.DictReader(stream)
                results.append((reader.fieldnames, list(reader)))
            if os.name == "posix":
                self.assertEqual(path.stat().st_mode & 0o777, 0o600)
        self.assertEqual(results[0], results[1])

    def test_commit_success_collision_and_missing_temp_have_equivalent_outcomes(self):
        for scenario in ("success", "collision", "missing-temp"):
            outcomes = []
            for native in (False, True):
                root = self.root / f"{scenario}-{native}"
                root.mkdir()
                source, temp, final, backup = (root / name for name in ("source.mkv", "temp.mkv", "final.mkv", "old/source.mkv"))
                backup.parent.mkdir()
                source.write_bytes(b"source")
                if scenario != "missing-temp":
                    temp.write_bytes(b"validated")
                if scenario == "collision":
                    final.write_bytes(b"competitor")
                failed = False
                with patch.dict(os.environ, {"HEVC_REBUILD_NATIVE_HELPER": HELPER if native else ""}):
                    try:
                        commit_transaction(source, temp, final, backup)
                    except (OSError, RebuildError):
                        failed = True
                outcomes.append((failed, [path.read_bytes() if path.exists() else None for path in (source, temp, final, backup)]))
            self.assertEqual(outcomes[0], outcomes[1], scenario)

    def test_corrupt_report_recovery_rejects_without_overwriting_either_path(self):
        log_dir = Path(self.environment["XDG_STATE_HOME"]) / "hevc-rebuild" / "logs"
        log_dir.mkdir(parents=True, mode=0o700)
        for native in (False, True):
            path = log_dir / f"hevc-rebuild-corrupt-{native}.csv"
            original = b"wrong,schema\nretained,evidence\n"
            path.write_bytes(original)
            with patch.dict(os.environ, self.environment | {"HEVC_REBUILD_NATIVE_HELPER": HELPER if native else ""}):
                with self.assertRaises((OSError, RebuildError)):
                    append_csv(path, {"input": "synthetic.mkv", "status": "OK"})
            self.assertEqual(path.read_bytes(), original)

    @unittest.skipUnless(os.name == "posix", "Owned process cancellation requires Unix")
    def test_cancellation_reaps_same_synthetic_encoder_on_both_paths(self):
        for native in (False, True):
            pid_path = self.root / f"encoder-{native}.pid"
            encoder = [sys.executable, "-c", "import os,sys,time; open(sys.argv[1],'w').write(str(os.getpid())); time.sleep(30)", str(pid_path)]
            command = [HELPER, "internal", "encode-supervisor", "--", *encoder] if native else encoder
            proc = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, start_new_session=True)
            try:
                deadline = time.monotonic() + 5
                while not pid_path.exists() and time.monotonic() < deadline:
                    time.sleep(0.01)
                self.assertTrue(pid_path.exists(), "Synthetic encoder did not start")
                pid = int(pid_path.read_text())
                if native:
                    proc.stdin.write("cancel\n")
                    proc.stdin.flush()
                else:
                    terminate_process_group(proc)
                self.assertNotEqual(proc.wait(timeout=5), 0)
                with self.assertRaises(ProcessLookupError):
                    os.kill(pid, 0)
            finally:
                terminate_process_group(proc)
                for stream in (proc.stdin, proc.stdout, proc.stderr):
                    stream.close()

    def test_profile_primary_stream_and_sdr_pq_hlg_fields_match(self):
        for transfer in (None, "smpte2084", "arib-std-b67"):
            with self.subTest(transfer=transfer):
                source = self.root / "source.mkv"
                source.write_bytes(b"synthetic media boundary")
                video = {"index": 4, "codec_type": "video", "codec_name": "hevc", "width": 1920, "height": 1080, "pix_fmt": "yuv420p10le" if transfer else "yuv420p", "avg_frame_rate": "30/1", "duration": "10", "disposition": {"default": 1}}
                if transfer:
                    video.update(color_primaries="bt2020", color_transfer=transfer, color_space="bt2020nc")
                probe = {"format": {"duration": "10", "format_name": "matroska", "tags": {}}, "streams": [{"index": 0, "codec_type": "video", "codec_name": "mjpeg", "width": 2000, "height": 3000, "disposition": {"attached_pic": 1}}, {"index": 2, "codec_type": "video", "codec_name": "h264", "width": 640, "height": 360, "pix_fmt": "yuv420p", "disposition": {}}, video, {"index": 5, "codec_type": "audio", "codec_name": "aac", "channels": 2}], "chapters": []}
                ffprobe, mediainfo = self.root / "ffprobe-fixture", self.root / "mediainfo-fixture"
                ffprobe.write_text(f'#!{sys.executable}\nimport sys\nprint({json.dumps(json.dumps(probe))} if "-show_frames" not in sys.argv else \'{{"frames":[]}}\')\n')
                mediainfo.write_text(f'#!{sys.executable}\nprint(\'{{"media":{{"track":[]}}}}\')\n')
                ffprobe.chmod(0o700)
                mediainfo.chmod(0o700)
                tools = Toolchain("ffmpeg", str(ffprobe), str(mediainfo), "mkvpropedit", "mkvmerge", "mkvextract", None)
                with patch.dict(os.environ, {"HEVC_REBUILD_NATIVE_HELPER": ""}):
                    expected = build_profile(tools, source)
                with patch.dict(os.environ, {"HEVC_REBUILD_NATIVE_HELPER": HELPER}):
                    actual = build_native_profile_set(tools, source)
                self.assertIsNotNone(actual)
                for field in ("duration", "source_bit_depth", "source_chroma", "target_pix_fmt", "color_primaries", "color_matrix", "transfer", "hdr10", "hlg", "dolby_vision", "interlaced"):
                    self.assertEqual(getattr(actual, field), getattr(expected, field), field)
                self.assertEqual(actual.primary_video.index, expected.primary_video.index)
                self.assertEqual(actual.primary_video.index, 4)
                self.assertEqual([ref.index for ref in actual.secondary_videos], [ref.index for ref in expected.secondary_videos])


if __name__ == "__main__":
    unittest.main()
