import os
import tempfile
import unittest
from pathlib import Path

from hevc_rebuild.app import _release_temporary_output
from hevc_rebuild.model import StreamRef
from hevc_rebuild.util import RebuildError
from hevc_rebuild.validate import _validate_timing


class RetainedOutputTests(unittest.TestCase):
    def _temp_encode(self, directory: Path, name: str = "chosen.mkv") -> tuple[Path, Path]:
        final = directory / name
        temporary = directory / f".{final.stem}.hevc-rebuild.fixture.tmp.mkv"
        temporary.write_bytes(b"completed-but-rejected-output")
        return final, temporary

    def test_late_failure_uses_final_name_and_adjacent_error_log(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "source.mkv"
            source.write_bytes(b"original")
            final, temporary = self._temp_encode(root)
            row = {"status": "FAILED", "message": "subtitle stream 0 shifted by 0.031000s"}

            _release_temporary_output(
                temporary,
                "output validation",
                row,
                False,
                final_output=final,
                source=source,
                failure_detail="RebuildError: subtitle timing drift exceeded tolerance",
            )

            error_log = root / "chosen+error.log"
            self.assertFalse(temporary.exists())
            self.assertEqual(final.read_bytes(), b"completed-but-rejected-output")
            self.assertEqual(source.read_bytes(), b"original")
            self.assertTrue(error_log.exists())
            self.assertIn("subtitle timing drift exceeded tolerance", error_log.read_text(encoding="utf-8"))
            self.assertIn(str(final), error_log.read_text(encoding="utf-8"))
            self.assertEqual(row["output"], str(final))
            self.assertIn("remains unvalidated", row["message"])
            if os.name == "posix":
                self.assertEqual(error_log.stat().st_mode & 0o777, 0o600)

    def test_in_place_failure_never_replaces_source(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "The chosen.mkv"
            source.write_bytes(b"original")
            final, temporary = self._temp_encode(root, source.name)
            row = {"status": "FAILED", "message": "mkvpropedit rejected metadata"}

            _release_temporary_output(
                temporary,
                "metadata embedding",
                row,
                False,
                final_output=final,
                source=source,
                failure_detail="RebuildError: metadata edit failed",
            )

            retained = root / "The chosen.hevc-rebuild.kept.mkv"
            error_log = root / "The chosen+error.log"
            self.assertEqual(source.read_bytes(), b"original")
            self.assertEqual(retained.read_bytes(), b"completed-but-rejected-output")
            self.assertTrue(error_log.exists())
            self.assertEqual(row["output"], str(retained))
            self.assertIn(str(retained), error_log.read_text(encoding="utf-8"))

    def test_keep_work_does_not_drop_late_completed_output_or_log(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "source.mkv"
            source.write_bytes(b"original")
            final, temporary = self._temp_encode(root)
            row = {"status": "FAILED", "message": "late validation failure"}

            _release_temporary_output(
                temporary,
                "lossless output validation",
                row,
                True,
                final_output=final,
                source=source,
                failure_detail="RebuildError: copied subtitle payload mismatch",
            )

            self.assertFalse(temporary.exists())
            self.assertTrue(final.exists())
            self.assertTrue((root / "chosen+error.log").exists())

    def test_early_failure_still_removes_unfinished_temp_output(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            final, temporary = self._temp_encode(root)
            row = {"status": "FAILED", "message": "encoder exited early"}

            _release_temporary_output(
                temporary,
                "encoding",
                row,
                False,
                final_output=final,
                source=root / "source.mkv",
                failure_detail="ExternalCommandError: encoder exited with status 1",
            )

            self.assertFalse(temporary.exists())
            self.assertFalse(final.exists())
            self.assertFalse((root / "chosen+error.log").exists())

    def test_existing_error_log_is_preserved(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "source.mkv"
            source.write_bytes(b"original")
            final, temporary = self._temp_encode(root)
            first_log = root / "chosen+error.log"
            first_log.write_text("previous review", encoding="utf-8")
            row = {"status": "FAILED", "message": "second late failure"}

            _release_temporary_output(
                temporary,
                "output validation",
                row,
                False,
                final_output=final,
                source=source,
                failure_detail="RebuildError: second validation failure",
            )

            self.assertEqual(first_log.read_text(encoding="utf-8"), "previous review")
            numbered = root / "chosen+error.1.log"
            self.assertTrue(numbered.exists())
            self.assertIn("second validation failure", numbered.read_text(encoding="utf-8"))

    def test_subtitle_timing_drift_is_a_controlled_validation_error(self):
        subtitle = StreamRef(index=2, type_ordinal=0, codec_type="subtitle", codec_name="subrip")
        source_probe = {"streams": [{"index": 2, "start_time": "0.000", "duration": "10.000"}]}
        output_probe = {"streams": [{"index": 2, "start_time": "0.031", "duration": "10.000"}]}

        with self.assertRaisesRegex(RebuildError, "subtitle stream 0 start_time shifted by 0.031000s"):
            _validate_timing(source_probe, output_probe, "subtitle", [subtitle], [subtitle], tolerance=0.020)

        output_probe["streams"][0]["start_time"] = "0.019"
        _validate_timing(source_probe, output_probe, "subtitle", [subtitle], [subtitle], tolerance=0.020)


if __name__ == "__main__":
    unittest.main()
