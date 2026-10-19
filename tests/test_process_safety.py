import os
import io
import sys
import tempfile
import time
import subprocess
import unittest
from pathlib import Path

from hevc_rebuild import util


class ProcessSafetyTests(unittest.TestCase):
    @unittest.skipUnless(os.name == "posix", "Owned groups require Unix")
    def test_encode_cleanup_stops_descendant_after_parent_already_exited(self):
        from hevc_rebuild.encode import _terminate_child
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "descendant.pid"
            source = "import subprocess,sys; p=subprocess.Popen([sys.executable,'-c','import time; time.sleep(30)'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL); open(sys.argv[1],'w').write(str(p.pid))"
            proc = subprocess.Popen([sys.executable, "-c", source, str(path)], start_new_session=True)
            proc.wait(timeout=5)
            _terminate_child(proc, timeout=0.2)
            stat = Path(f"/proc/{int(path.read_text())}/stat")
            deadline = time.monotonic()+3
            while stat.exists() and stat.read_text().split()[2] != "Z" and time.monotonic()<deadline:
                time.sleep(0.01)
            if stat.exists():
                self.assertEqual(stat.read_text().split()[2], "Z")

    def test_verbose_diagnostic_drain_retains_bounded_tail_and_labels_truncation(self):
        tail = util.DiagnosticTail()
        pipe = io.StringIO("x" * 2_000_000 + "\nlast useful diagnostic\n")
        thread = util.start_stderr_drain(pipe, tail)
        thread.join(timeout=5)
        self.assertFalse(thread.is_alive())
        self.assertTrue(pipe.closed)
        self.assertLessEqual(len("".join(tail)), 65536)
        self.assertIn("truncated", tail.text())
        self.assertIn("last useful diagnostic", tail.text())

    def test_unexpected_stream_close_is_observable_and_drain_stops(self):
        pipe = io.StringIO("fixture")
        pipe.close()
        tail = util.DiagnosticTail()
        thread = util.start_stderr_drain(pipe, tail)
        thread.join(timeout=2)
        self.assertFalse(thread.is_alive())
        self.assertIsNotNone(tail.error)
        self.assertIn("drain failed", tail.text())

    @unittest.skipUnless(os.environ.get("HEVC_REBUILD_NATIVE_HELPER"), "Rust helper not configured")
    def test_runtime_guard_joins_drain_before_closing_pipe(self):
        from hevc_rebuild.encode import _NativeRuntimeGuard
        guard = _NativeRuntimeGuard(os.environ["HEVC_REBUILD_NATIVE_HELPER"])
        guard.close()
        self.assertFalse(guard._stderr_drain.is_alive())
        self.assertTrue(guard.proc.stdout.closed)
        self.assertTrue(guard.proc.stderr.closed)

    def test_timeout_preserves_diagnostic_and_command_context(self):
        started = time.monotonic()
        with self.assertRaises(util.ExternalCommandError) as caught:
            util.run([sys.executable, "-c", "import sys,time; print('fixture tail', file=sys.stderr, flush=True); time.sleep(20)"], timeout=0.15)
        self.assertIn("Timed out", str(caught.exception))
        self.assertIn("fixture tail", str(caught.exception))
        self.assertEqual(caught.exception.command[0], sys.executable)
        self.assertLess(time.monotonic()-started, 4)

    def test_missing_command_has_actionable_context(self):
        with self.assertRaisesRegex(util.RebuildError, "fixture-missing-executable"):
            util.run(["/fixture-missing-executable"])

    @unittest.skipUnless(os.name == "posix", "Owned sessions require Unix")
    def test_timeout_kills_descendant_that_holds_output_pipe(self):
        with tempfile.TemporaryDirectory() as directory:
            pid_file = Path(directory) / "descendant.pid"
            child_code = "import signal,time; signal.signal(signal.SIGTERM,signal.SIG_IGN); time.sleep(30)"
            parent_code = "import subprocess,sys,time; p=subprocess.Popen([sys.executable,'-c',sys.argv[2]]); open(sys.argv[1],'w').write(str(p.pid)); print('spawned',flush=True); time.sleep(30)"
            with self.assertRaises(util.ExternalCommandError):
                util.run([sys.executable, "-c", parent_code, str(pid_file), child_code], timeout=0.25)
            pid = int(pid_file.read_text())
            proc_stat = Path(f"/proc/{pid}/stat")
            if proc_stat.exists():
                # An adopted zombie has terminated and cannot hold pipes/run work.
                self.assertEqual(proc_stat.read_text().split()[2], "Z")

    def test_success_and_nonzero_status_keep_completed_process_contract(self):
        result = util.run([sys.executable, "-c", "print('ordinary output')"])
        self.assertEqual(result.stdout, "ordinary output\n")
        self.assertEqual(result.returncode, 0)
        result = util.run([sys.executable, "-c", "raise SystemExit(17)"], check=False)
        self.assertEqual(result.returncode, 17)
