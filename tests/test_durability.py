import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from hevc_rebuild.util import fsync_dir


@unittest.skipUnless(hasattr(os, "O_DIRECTORY"), "Directory fsync requires Unix")
class DirectorySyncTests(unittest.TestCase):
    def test_existing_directory_syncs_and_missing_directory_raises(self):
        with tempfile.TemporaryDirectory() as directory:
            fsync_dir(Path(directory))
            with self.assertRaises(FileNotFoundError):
                fsync_dir(Path(directory) / "missing")

    def test_open_permission_error_is_not_success(self):
        with patch("hevc_rebuild.util.os.open", side_effect=PermissionError("synthetic denied")), patch("hevc_rebuild.util.os.fsync") as sync:
            with self.assertRaises(PermissionError):
                fsync_dir(Path("/synthetic"))
            sync.assert_not_called()

    def test_sync_failure_propagates_and_descriptor_closes(self):
        with patch("hevc_rebuild.util.os.open", return_value=42), patch("hevc_rebuild.util.os.fsync", side_effect=OSError("synthetic sync failure")), patch("hevc_rebuild.util.os.close") as close:
            with self.assertRaisesRegex(OSError, "sync failure"):
                fsync_dir(Path("/synthetic"))
            close.assert_called_once_with(42)
