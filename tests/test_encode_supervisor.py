from __future__ import annotations

import os
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path


HELPER = os.environ.get("HEVC_REBUILD_NATIVE_HELPER")


@unittest.skipUnless(HELPER, "Rust helper path not configured")
class NativeEncodeSupervisorTests(unittest.TestCase):
    def start_fake_encoder(self, source: str, *args: str) -> subprocess.Popen[str]:
        assert HELPER is not None
        return subprocess.Popen(
            [HELPER, "internal", "encode-supervisor", "--", sys.executable, "-c", source, *args],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            bufsize=1,
        )

    def test_forwards_progress_and_child_stderr_without_changing_status(self):
        proc = self.start_fake_encoder(
            "import sys; print('frame=2'); print('progress=end'); print('fixture diagnostic', file=sys.stderr)"
        )
        self.assertEqual(proc.wait(timeout=5), 0)
        stdout, stderr = proc.communicate()
        self.assertEqual(stdout, "frame=2\nprogress=end\n")
        self.assertIn("fixture diagnostic", stderr)

    def test_preserves_nonzero_encoder_exit_code(self):
        proc = self.start_fake_encoder("raise SystemExit(17)")
        self.assertEqual(proc.wait(timeout=5), 17)
        _, stderr = proc.communicate()
        self.assertEqual(stderr, "")

    @unittest.skipUnless(os.name == "posix", "child signal status is Unix-specific")
    def test_reports_signal_termination_for_oom_diagnostics(self):
        proc = self.start_fake_encoder(
            "import os, signal; os.kill(os.getpid(), signal.SIGKILL)"
        )
        self.assertEqual(proc.wait(timeout=5), 255)
        _, stderr = proc.communicate()
        self.assertIn("__HEVC_NATIVE_CHILD_SIGNAL__=9", stderr)

    @unittest.skipUnless(os.name == "posix", "process lookup is Unix-specific")
    def test_cancel_request_stops_and_reaps_encoder_child(self):
        with tempfile.TemporaryDirectory(prefix="hevc-supervisor-") as temp_dir:
            pid_file = Path(temp_dir) / "child.pid"
            proc = self.start_fake_encoder(
                "import os, sys, time; open(sys.argv[1], 'w').write(str(os.getpid())); time.sleep(30)",
                str(pid_file),
            )
            deadline = time.monotonic() + 5
            while not pid_file.exists() and time.monotonic() < deadline:
                time.sleep(0.01)
            self.assertTrue(pid_file.exists(), "fake encoder did not start")
            child_pid = int(pid_file.read_text())
            assert proc.stdin is not None
            proc.stdin.write("cancel\n")
            proc.stdin.flush()
            self.assertNotEqual(proc.wait(timeout=5), 0)
            proc.stdin.close()
            proc.stdout.close()
            proc.stderr.close()
            with self.assertRaises(ProcessLookupError):
                os.kill(child_pid, 0)

    @unittest.skipUnless(os.name == "posix", "process lookup is Unix-specific")
    def test_adapter_pipe_eof_stops_and_reaps_encoder_child(self):
        with tempfile.TemporaryDirectory(prefix="hevc-supervisor-") as temp_dir:
            pid_file = Path(temp_dir) / "child.pid"
            proc = self.start_fake_encoder(
                "import os, sys, time; open(sys.argv[1], 'w').write(str(os.getpid())); time.sleep(30)",
                str(pid_file),
            )
            deadline = time.monotonic() + 5
            while not pid_file.exists() and time.monotonic() < deadline:
                time.sleep(0.01)
            self.assertTrue(pid_file.exists(), "fake encoder did not start")
            child_pid = int(pid_file.read_text())
            assert proc.stdin is not None
            proc.stdin.close()
            self.assertNotEqual(proc.wait(timeout=5), 0)
            proc.stdout.close()
            proc.stderr.close()
            with self.assertRaises(ProcessLookupError):
                os.kill(child_pid, 0)


if __name__ == "__main__":
    unittest.main()
