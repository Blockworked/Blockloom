#!/usr/bin/env python3
"""Exercise Hub cancellation with owned child processes and a real process tree."""

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
import hub_process


class ProcessTests(unittest.TestCase):
    def test_birth_identity_and_pid_reuse(self):
        token = hub_process.process_token(os.getpid())
        self.assertIsNotNone(token)
        self.assertNotEqual(token, "unknown")
        self.assertTrue(hub_process.running({"pid": os.getpid(), "token": token}))
        self.assertFalse(hub_process.running({"pid": os.getpid(), "token": "old process"}))

    def test_exited_process_is_not_running(self):
        process = subprocess.Popen([sys.executable, "-c", "pass"])
        process.wait(timeout=10)
        self.assertIsNone(hub_process.process_token(process.pid))

    def test_cancellation_stops_descendants_and_keeps_log(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            script = root / "build.py"
            markers = root / "pids.json"
            cancel = root / "cancel"
            script.write_text(
                "import json, os, subprocess, sys, time\n"
                "from pathlib import Path\n"
                "child = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)'])\n"
                "Path(sys.argv[1]).write_text(json.dumps([os.getpid(), child.pid]))\n"
                "print('Building a fixture', flush=True)\n"
                "time.sleep(60)\n", encoding="utf-8")

            def request():
                deadline = time.monotonic() + 10
                while not markers.exists() and time.monotonic() < deadline:
                    time.sleep(0.05)
                time.sleep(0.15)
                cancel.touch()

            watcher = threading.Thread(target=request)
            watcher.start()
            try:
                with patch.dict(os.environ, {"BLOCKLOOM_HUB_CANCEL_FILE": str(cancel)}):
                    with self.assertRaises(hub_process.Cancelled):
                        hub_process.run([sys.executable, str(script), str(markers)], log_file=root / "build.log")
                for pid in json.loads(markers.read_text()):
                    self.assertIsNone(hub_process.process_token(pid), f"Build process {pid} survived cancellation")
                self.assertIn("Building a fixture", (root / "build.log").read_text())
            finally:
                watcher.join(timeout=10)

    def test_cancel_before_launch(self):
        with tempfile.TemporaryDirectory() as temporary:
            cancel = Path(temporary) / "cancel"
            cancel.touch()
            with patch.dict(os.environ, {"BLOCKLOOM_HUB_CANCEL_FILE": str(cancel)}), patch.object(subprocess, "Popen") as launch:
                with self.assertRaises(hub_process.Cancelled):
                    hub_process.run([sys.executable, "-c", "pass"])
                launch.assert_not_called()


if __name__ == "__main__":
    unittest.main()
