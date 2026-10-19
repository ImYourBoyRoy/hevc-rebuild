import csv
import logging
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from hevc_rebuild import util


class ReportIdentityTests(unittest.TestCase):
    def test_reconfiguration_closes_old_handlers_and_creates_private_artifacts(self):
        root = logging.getLogger()
        saved = root.handlers[:]
        saved_level = root.level
        root.handlers = []
        try:
            with tempfile.TemporaryDirectory() as directory, patch.dict(os.environ, {"HEVC_REBUILD_NATIVE_HELPER": ""}):
                log, report = util.configure_logging(Path(directory) / "logs")
                previous = root.handlers[:]
                logging.info("first diagnostic")
                util.append_csv(report, {"status": "OK"})
                second_log, _ = util.configure_logging(log.parent)
                self.assertIsNone(previous[0].stream)
                self.assertEqual(len(root.handlers), 2)
                logging.info("second diagnostic")
                root.handlers[0].flush()
                self.assertIn("first diagnostic", log.read_text())
                self.assertNotIn("second diagnostic", log.read_text())
                self.assertIn("second diagnostic", second_log.read_text())
                if os.name == "posix":
                    self.assertEqual(log.parent.stat().st_mode & 0o777, 0o700)
                    self.assertEqual(log.stat().st_mode & 0o777, 0o600)
                    self.assertEqual(report.stat().st_mode & 0o777, 0o600)
        finally:
            for handler in root.handlers:
                handler.close()
            root.handlers = saved
            root.setLevel(saved_level)

    def test_sequential_runs_in_one_second_keep_artifacts_and_rows_separate(self):
        root = logging.getLogger()
        saved = root.handlers[:]
        saved_level = root.level
        created = []
        root.handlers = []
        try:
            with tempfile.TemporaryDirectory() as directory, patch.object(
                util.time, "strftime", return_value="20261011-120000"
            ), patch.dict("os.environ", {"HEVC_REBUILD_NATIVE_HELPER": ""}):
                first = util.configure_logging(Path(directory))
                created.extend(root.handlers)
                util.append_csv(first[1], {"input": "shared.mkv", "status": "OK"})
                second = util.configure_logging(Path(directory))
                created.extend(root.handlers)
                util.append_csv(second[1], {"input": "shared.mkv", "status": "FAIL"})
                self.assertNotEqual(first[1], second[1])
                for log, report in (first, second):
                    self.assertEqual(log.stem, report.stem)
                    self.assertTrue(log.exists())
                for pair, status in ((first, "OK"), (second, "FAIL")):
                    with pair[1].open(newline="") as stream:
                        rows = list(csv.DictReader(stream))
                    self.assertEqual(len(rows), 1)
                    self.assertEqual(rows[0]["status"], status)
        finally:
            for handler in created:
                handler.close()
            root.handlers = saved
            root.setLevel(saved_level)


class ReportWriterTests(unittest.TestCase):
    def test_sync_failures_retain_written_row_and_report_unconfirmed_durability(self):
        for boundary in ("os.fsync", "fsync_dir"):
            with self.subTest(boundary=boundary), tempfile.TemporaryDirectory() as directory, patch.dict(os.environ, {"HEVC_REBUILD_NATIVE_HELPER": ""}):
                report = Path(directory) / "hevc-rebuild-sync.csv"
                with patch("hevc_rebuild.util." + boundary, side_effect=OSError("synthetic sync failure")):
                    with self.assertRaisesRegex(util.RebuildError, "durability is unconfirmed"):
                        util.append_csv(report, {"status": "OK"})
                with report.open(newline="") as stream:
                    rows = list(csv.DictReader(stream))
                self.assertEqual(len(rows), 1)
                self.assertEqual(rows[0]["status"], "OK")

    def test_concurrent_process_writers_share_one_header(self):
        with tempfile.TemporaryDirectory() as directory:
            report = Path(directory) / "hevc-rebuild-concurrent.csv"
            env = {**os.environ, "HEVC_REBUILD_NATIVE_HELPER": ""}
            workers = [subprocess.Popen([
                sys.executable, "-c",
                "from pathlib import Path; from hevc_rebuild.util import append_csv; "
                "import sys; append_csv(Path(sys.argv[1]), {'input':sys.argv[2], 'status':'OK', 'message':'quoted,\\ntext'})",
                str(report), str(index),
            ], env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE) for index in range(12)]
            for worker in workers:
                stdout, stderr = worker.communicate(timeout=15)
                self.assertEqual(worker.returncode, 0, (stdout, stderr))
            with report.open(newline="") as stream:
                records = list(csv.reader(stream))
            self.assertEqual(records[0], util.REPORT_FIELDS)
            self.assertEqual(len(records), 13)
            self.assertEqual({row[0] for row in records[1:]}, {str(i) for i in range(12)})

    def test_empty_repeated_and_invalid_schema_reports(self):
        with tempfile.TemporaryDirectory() as directory, patch.dict(os.environ, {"HEVC_REBUILD_NATIVE_HELPER": ""}):
            report = Path(directory) / "hevc-rebuild-empty.csv"
            report.touch()
            for index in range(3):
                util.append_csv(report, {"input": index})
            with report.open(newline="") as stream:
                self.assertEqual(len(list(csv.DictReader(stream))), 3)
            for invalid in (b"wrong,header\n", b"input,status\nunfinished", ",".join(util.REPORT_FIELDS).encode()+b"\r\npartial"):
                report.write_bytes(invalid)
                with self.assertRaises(util.RebuildError):
                    util.append_csv(report, {"input": "new"})
                self.assertEqual(report.read_bytes(), invalid)

    def test_invalid_row_then_retry_does_not_leave_header(self):
        class InvalidValue:
            def __str__(self):
                raise ValueError("synthetic conversion failure")
        with tempfile.TemporaryDirectory() as directory, patch.dict(os.environ, {"HEVC_REBUILD_NATIVE_HELPER": ""}):
            report = Path(directory) / "hevc-rebuild-recovery.csv"
            with self.assertRaises(ValueError):
                util.append_csv(report, {"message": InvalidValue()})
            self.assertEqual(report.read_bytes(), b"")
            util.append_csv(report, {"status": "OK"})
            with report.open(newline="") as stream:
                self.assertEqual(len(list(csv.DictReader(stream))), 1)

    @unittest.skipUnless(os.environ.get("HEVC_REBUILD_NATIVE_HELPER"), "Rust helper not configured")
    def test_mixed_rust_python_processes_share_schema_and_lock(self):
        with tempfile.TemporaryDirectory() as directory:
            report = Path(directory) / "logs" / "hevc-rebuild-mixed.csv"
            report.parent.mkdir()
            workers = []
            for index in range(12):
                env = {**os.environ, "HEVC_REBUILD_PORTABLE_DATA_DIR": directory}
                if index % 2:
                    env["HEVC_REBUILD_NATIVE_HELPER"] = ""
                workers.append(subprocess.Popen([
                    sys.executable, "-c",
                    "from pathlib import Path; from hevc_rebuild.util import append_csv; "
                    "import sys; append_csv(Path(sys.argv[1]), {'input':sys.argv[2], 'status':'OK'})",
                    str(report), str(index),
                ], env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE))
            for worker in workers:
                stdout, stderr = worker.communicate(timeout=30)
                self.assertEqual(worker.returncode, 0, (stdout, stderr))
            with report.open(newline="") as stream:
                records = list(csv.reader(stream))
            self.assertEqual(records[0], util.REPORT_FIELDS)
            self.assertEqual(len(records), 13)
            self.assertEqual({row[0] for row in records[1:]}, {str(i) for i in range(12)})
