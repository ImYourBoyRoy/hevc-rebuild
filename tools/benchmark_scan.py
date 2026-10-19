"""Benchmark native scans using exclusively generated, disposable media."""
from __future__ import annotations

import argparse
import json
import os
import resource
import shutil
import signal
import subprocess
import tempfile
import time
from pathlib import Path


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--helper", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--counts", type=int, nargs="+", default=[1, 20, 100])
    args = parser.parse_args()
    helper = args.helper.resolve(strict=True)
    if any(count < 1 or count > 1000 for count in args.counts):
        parser.error("Counts must be between 1 and 1000")
    results = []
    with tempfile.TemporaryDirectory(prefix="hevc-scan-benchmark-") as directory:
        root = Path(directory)
        environment = os.environ | {"HEVC_REBUILD_SCAN_TIMING": "1"}
        environment.pop("HEVC_REBUILD_PORTABLE_DATA_DIR", None)
        for name in ("XDG_CONFIG_HOME", "XDG_STATE_HOME", "XDG_CACHE_HOME"):
            environment[name] = str(root / name.lower())
        clip = root / "generated.mkv"
        subprocess.run(["ffmpeg", "-v", "error", "-f", "lavfi", "-i", "color=c=blue:s=320x180:r=24", "-f", "lavfi", "-i", "sine=frequency=440", "-t", "1", "-c:v", "libx264", "-threads", "1", "-c:a", "aac", str(clip)], check=True, timeout=30)
        for count in args.counts:
            queue = root / str(count)
            queue.mkdir()
            for index in range(count):
                shutil.copyfile(clip, queue / f"synthetic-{index:04}.mkv")
            command = [str(helper), "json", "scan", "--root", str(queue), "--min-size-mib", "0"]
            start = time.monotonic()
            usage_before = resource.getrusage(resource.RUSAGE_CHILDREN)
            result = subprocess.run(command, env=environment, capture_output=True, text=True, timeout=300, check=True)
            wall_ms = (time.monotonic() - start) * 1000
            usage_after = resource.getrusage(resource.RUSAGE_CHILDREN)
            payload = json.loads(result.stdout)
            if payload["candidate_count"] != count or any(entry["status"] != "Ready" for entry in payload["entries"]):
                raise RuntimeError("Synthetic queue did not analyze completely")
            results.append({"count": count, "wall_ms": wall_ms, "timings": payload["scan_timings"], "child_cpu_seconds": usage_after.ru_utime + usage_after.ru_stime - usage_before.ru_utime - usage_before.ru_stime, "cumulative_child_peak_rss_kib": usage_after.ru_maxrss, "json_bytes": len(result.stdout.encode()), "profile_dimensions": [320, 180]})
            # Kill only this owned synthetic scan session after its first handoff.
            proc = subprocess.Popen(command, env=environment, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True, start_new_session=True)
            try:
                assert proc.stderr is not None
                while True:
                    line = proc.stderr.readline()
                    if line.startswith("scan\t"):
                        break
                    if not line:
                        raise RuntimeError("Scan exited before cancellation measurement")
                cancel_start = time.monotonic()
                os.killpg(proc.pid, signal.SIGTERM)
                proc.wait(timeout=5)
                os.killpg(proc.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            finally:
                if proc.poll() is None:
                    os.killpg(proc.pid, signal.SIGKILL)
                    proc.wait(timeout=5)
                proc.stderr.close()
            results[-1]["cancel_to_exit_ms"] = (time.monotonic() - cancel_start) * 1000
    args.output.write_text(json.dumps({"helper": str(helper), "synthetic_only": True, "rss_note": "ru_maxrss is cumulative child high-water mark, not per-stage allocation", "results": results}, indent=2) + "\n")
    print(args.output)


if __name__ == "__main__":
    main()
