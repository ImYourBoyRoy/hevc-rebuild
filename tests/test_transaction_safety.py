"""Synthetic byte fixtures exercise commit safety without processing media."""
import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from hevc_rebuild.app import commit_transaction
from hevc_rebuild.util import RebuildError, rename_no_replace


class TransactionSafetyTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.source = self.root / "source.mkv"
        self.temporary = self.root / "temporary.mkv"
        self.final = self.root / "final.mkv"
        self.backup = self.root / "old" / "original.mkv"
        self.backup.parent.mkdir()
        self.source.write_bytes(b"original")
        self.temporary.write_bytes(b"validated")
        self.environment = patch.dict(os.environ, {}, clear=False)
        self.environment.start()
        os.environ.pop("HEVC_REBUILD_NATIVE_HELPER", None)
        self.addCleanup(self.environment.stop)

    def commit(self):
        commit_transaction(self.source, self.temporary, self.final, self.backup)

    def test_success_and_existing_destination_never_overwrite(self):
        self.final.write_bytes(b"competitor")
        with self.assertRaises(RebuildError):
            self.commit()
        self.assertEqual(self.source.read_bytes(), b"original")
        self.assertEqual(self.final.read_bytes(), b"competitor")
        self.final.unlink()
        self.commit()
        self.assertEqual(self.backup.read_bytes(), b"original")
        self.assertEqual(self.final.read_bytes(), b"validated")

    def test_backup_created_between_check_and_move_is_preserved(self):
        def race(source, destination):
            if destination == self.backup:
                destination.write_bytes(b"competing backup")
            rename_no_replace(source, destination)
        with patch("hevc_rebuild.transactions.rename_no_replace", side_effect=race):
            with self.assertRaises(FileExistsError):
                self.commit()
        self.assertEqual(self.source.read_bytes(), b"original")
        self.assertEqual(self.backup.read_bytes(), b"competing backup")
        self.assertEqual(self.temporary.read_bytes(), b"validated")

    def test_competing_output_restores_source(self):
        def race(source, destination):
            if destination == self.final:
                destination.write_bytes(b"competing output")
            rename_no_replace(source, destination)
        with patch("hevc_rebuild.transactions.rename_no_replace", side_effect=race):
            with self.assertRaises(FileExistsError):
                self.commit()
        self.assertEqual(self.source.read_bytes(), b"original")
        self.assertEqual(self.final.read_bytes(), b"competing output")

    def test_rollback_race_retains_backup_without_overwriting_source(self):
        def race(source, destination):
            if destination == self.final:
                self.source.write_bytes(b"new occupant")
                raise OSError("synthetic install failure")
            rename_no_replace(source, destination)
        with patch("hevc_rebuild.transactions.rename_no_replace", side_effect=race):
            with self.assertRaisesRegex(RebuildError, "original remains preserved"):
                self.commit()
        self.assertEqual(self.source.read_bytes(), b"new occupant")
        self.assertEqual(self.backup.read_bytes(), b"original")
        self.assertEqual(self.temporary.read_bytes(), b"validated")

    def test_directory_sync_failures_have_truthful_recovery_states(self):
        for fail_at in (1, 2, 3):
            with self.subTest(fail_at=fail_at):
                if self.final.exists():
                    self.final.unlink()
                self.source.write_bytes(b"original")
                self.temporary.write_bytes(b"validated")
                if self.backup.exists():
                    self.backup.unlink()
                calls = 0
                def sync(_, fail_at=fail_at):
                    nonlocal calls
                    calls += 1
                    if calls == fail_at:
                        raise OSError("synthetic sync failure")
                with patch("hevc_rebuild.transactions.fsync_dir", side_effect=sync):
                    with self.assertRaises((OSError, RebuildError)) as failure:
                        self.commit()
                if fail_at == 3:
                    self.assertIn("Output was installed", str(failure.exception))
                    self.assertEqual(self.final.read_bytes(), b"validated")
                    self.assertEqual(self.backup.read_bytes(), b"original")
                else:
                    self.assertEqual(self.source.read_bytes(), b"original")
                    self.assertEqual(self.temporary.read_bytes(), b"validated")

    def test_missing_source_and_file_sync_failure_preserve_temporary(self):
        with patch("hevc_rebuild.transactions.fsync_file", side_effect=OSError("synthetic file sync failure")):
            with self.assertRaises(OSError):
                self.commit()
        self.assertEqual(self.source.read_bytes(), b"original")
        self.source.unlink()
        with self.assertRaises(FileNotFoundError):
            self.commit()
        self.assertEqual(self.temporary.read_bytes(), b"validated")

    def test_existing_symlink_is_not_replaced(self):
        self.final.symlink_to(self.root / "missing")
        with self.assertRaises(RebuildError):
            self.commit()
        self.assertTrue(self.final.is_symlink())
        self.assertEqual(self.source.read_bytes(), b"original")
