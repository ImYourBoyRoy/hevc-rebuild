import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from hevc_rebuild.util import StopRequest


class StopControlTests(unittest.TestCase):
    def test_stale_marker_ignored_new_request_and_unrelated_job(self):
        with tempfile.TemporaryDirectory() as directory:
            first = Path(directory) / "first"
            second = Path(directory) / "second"
            first.touch(mode=0o600)
            control = StopRequest(first)
            self.assertFalse(control.requested())
            other = StopRequest(second)
            first.unlink()
            first.touch(mode=0o600)
            self.assertTrue(control.requested())
            self.assertFalse(other.requested())

    def test_symlink_replacement_and_nonregular_files_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "stop"
            victim = Path(directory) / "victim"
            victim.write_text("untouched")
            control = StopRequest(path)
            path.symlink_to(victim)
            self.assertFalse(control.requested())
            self.assertEqual(victim.read_text(), "untouched")
            path.unlink()
            path.mkdir()
            self.assertFalse(control.requested())
            path.rmdir()
            path.touch(mode=0o600)
            actual_fstat = os.fstat
            def replace_after_open(fd):
                metadata = actual_fstat(fd)
                path.unlink()
                path.touch(mode=0o600)
                return metadata
            with patch("hevc_rebuild.util.os.fstat", side_effect=replace_after_open):
                self.assertFalse(control.requested())

    def test_private_native_marker_before_engine_start_and_parent_replacement(self):
        with tempfile.TemporaryDirectory(prefix="hevc-rebuild-control-") as directory:
            path = Path(directory) / "stop"
            path.touch(mode=0o600)
            control = StopRequest(path)
            self.assertTrue(control.requested())
            original = Path(directory)
            replacement = original.with_name(original.name + "-old")
            original.rename(replacement)
            try:
                original.mkdir(mode=0o700)
                path.touch(mode=0o600)
                self.assertFalse(control.requested())
                path.unlink()
                original.rmdir()
            finally:
                replacement.rename(original)

    def test_peer_writable_marker_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "stop"
            control = StopRequest(path)
            path.touch()
            path.chmod(0o666)
            self.assertFalse(control.requested())
