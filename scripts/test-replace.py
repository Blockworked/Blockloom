#!/usr/bin/env python3
"""Exercise the concurrent replacement runner without compiling Rust."""

import importlib.util
import io
import os
from pathlib import Path
import queue
import sys
import tempfile
import time
import unittest
from unittest.mock import patch


sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("replace", Path(__file__).with_name("replace.py"))
replace = importlib.util.module_from_spec(spec)
spec.loader.exec_module(replace)


OK = "import sys; print('built ok'); print('trailer', end=''); sys.stdout.flush()"
FAIL = "import sys; print('boom'); sys.exit(3)"
SLEEP = "import time; time.sleep(60)"


class RunnerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.marker = Path(self.temp.name) / "staged"
        self.commands = {}
        command = patch.object(replace, "build_command", self.commands.__getitem__)
        command.start()
        self.addCleanup(command.stop)
        self.statuses = []

        statuses = self.statuses

        class FakeStatus:
            def __init__(self, output=sys.stdout):
                self.interactive = False
                self.states = ["building"] * len(replace.BUILDS)
                statuses.append(self)

            def draw(self, changed=False):
                pass

            def log(self, text):
                pass

            def finish(self):
                pass

        status = patch.object(replace, "Status", FakeStatus)
        status.start()
        self.addCleanup(status.stop)

    def stub(self, recipe, code=OK):
        self.commands[recipe] = [sys.executable, "-c", code]

    def test_all_builds_pass(self):
        for _, recipe in replace.BUILDS:
            self.stub(recipe)
        with patch.dict(os.environ, {"BLOCKLOOM_NATIVE_PROFILE": "dist"}):
            self.assertEqual(replace.run_builds("dist"), 0)

    def test_a_failed_build_fails_the_run(self):
        for _, recipe in replace.BUILDS:
            self.stub(recipe)
        self.stub("player", FAIL)
        with patch.dict(os.environ, {"BLOCKLOOM_NATIVE_PROFILE": "dist"}):
            self.assertNotEqual(replace.run_builds("dist"), 0)

    def test_release_profile_stages_after_the_editor(self):
        marker = str(self.marker)
        self.stub("build")
        self.stub("web-player")
        self.stub("_stage-release-player", f"from pathlib import Path; Path({marker!r}).write_text('staged')")
        with patch.dict(os.environ, {"BLOCKLOOM_NATIVE_PROFILE": "release"}):
            self.assertEqual(replace.run_builds("release"), 0)
        self.assertTrue(self.marker.exists())

    def test_editor_failure_skips_staging(self):
        self.stub("build", FAIL)
        self.stub("web-player")
        self.stub("_stage-release-player", "import sys; sys.exit(0)")
        with patch.dict(os.environ, {"BLOCKLOOM_NATIVE_PROFILE": "release"}):
            self.assertNotEqual(replace.run_builds("release"), 0)

    def test_stop_kills_a_running_build(self):
        self.stub("sleep", SLEEP)
        processes, buffers = [None], {}
        replace.launch(0, "sleep", processes, buffers, queue.Queue(), dict(os.environ))
        try:
            self.assertIsNone(processes[0].poll())
            replace.stop(processes)
            self.assertIsNotNone(processes[0].wait(timeout=30))
        finally:
            try:
                processes[0].kill()
            except OSError:
                pass

    def test_cancel_marks_builds_cancelled(self):
        with patch.object(replace, "stop", lambda processes: None):
            with patch.object(replace, "_run_threaded", side_effect=KeyboardInterrupt):
                with self.assertRaises(KeyboardInterrupt):
                    replace.run_builds("release")
        self.assertEqual(self.statuses[-1].states, ["cancelled", "cancelled", "cancelled"])

    def test_host_target_matches_the_exporter(self):
        triple = replace.host_target()
        if sys.platform == "win32":
            self.assertTrue(triple.endswith("-pc-windows-msvc"))
        elif sys.platform == "darwin":
            self.assertTrue(triple.endswith("-apple-darwin"))
        else:
            self.assertTrue(triple.endswith("-unknown-linux-gnu"))

    def test_orchestration_fetches_prunes_and_reinstalls(self):
        calls = []

        def check(cmd):
            calls.append(cmd)

        def run_builds(profile):
            calls.append(("builds", profile))
            return 0

        env = {
            "BLOCKLOOM_NATIVE_PROFILE": "dist",
            "CARGO_BUILD_JOBS": "",
            "BLOCKLOOM_WEB_PROFILE": "release",
        }
        with (
            patch.object(replace, "check", check),
            patch.object(replace, "run_builds", run_builds),
            patch.object(replace, "just_exe", lambda: "just"),
            patch.dict(os.environ, env, clear=False),
        ):
            # CARGO_BUILD_JOBS is resolved, never inherited empty.
            os.environ.pop("CARGO_BUILD_JOBS", None)
            self.assertEqual(replace.main([]), 0)
            fetch = ["cargo", "fetch", "--locked", "--target", replace.host_target(),
                     "--target", "wasm32-unknown-unknown"]
            self.assertIn(fetch, calls)
            self.assertEqual(calls.count(("builds", "dist")), 1)
            self.assertEqual(os.environ["CARGO_NET_OFFLINE"], "true")
            self.assertTrue(int(os.environ["CARGO_BUILD_JOBS"]) >= 1)

    def test_orchestration_rejects_bad_profiles_and_jobs(self):
        with patch.dict(os.environ, {"BLOCKLOOM_NATIVE_PROFILE": "lto"}):
            self.assertEqual(replace.main([]), 1)
        with patch.dict(os.environ, {"BLOCKLOOM_NATIVE_PROFILE": "release"}):
            self.assertEqual(replace.main(["0"]), 1)
            self.assertEqual(replace.main(["lots"]), 1)

    def test_failed_builds_skip_reinstall(self):
        with (
            patch.object(replace, "check", lambda cmd: None),
            patch.object(replace, "run_builds", lambda profile: 1),
            patch.dict(os.environ, {"BLOCKLOOM_NATIVE_PROFILE": "dist"}),
        ):
            self.assertEqual(replace.main([]), 1)


if __name__ == "__main__":
    unittest.main()
