#!/usr/bin/env python3
"""Exercise cache cleanup on small fixtures without compiling Rust."""

from contextlib import redirect_stdout, redirect_stderr
import fcntl
import importlib.util
import io
import os
from pathlib import Path
import tempfile
import subprocess
import sys
import unittest
from unittest.mock import patch


sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("prune_target", Path(__file__).with_name("prune-target.py"))
cleaner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cleaner)


class CleanupTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.target = Path(self.temp.name) / "target"
        self.profile = self.target / "release"
        self.env = patch.dict(os.environ, {}, clear=True)
        self.env.start()
        self.addCleanup(self.env.stop)

    def write(self, path, age=100):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(b"x" * 8192)
        os.utime(path, (age, age))
        return path

    def prune(self, limit, **kwargs):
        output = io.StringIO()
        with redirect_stdout(output), redirect_stderr(output):
            cleaner.prune(self.target, limit, **kwargs)
        return output.getvalue()

    def test_under_budget_keeps_caches(self):
        file = self.write(self.profile / "incremental/new/data")
        self.prune(1_000_000)
        self.assertTrue(file.exists())

    def test_evicts_oldest_incremental_first(self):
        old = self.write(self.profile / "incremental/old/data", 10)
        new = self.write(self.profile / "incremental/new/data", 20)
        binary = self.write(self.profile / "blockloom")
        limit = cleaner.usage(self.target)[0] - cleaner.usage(old.parent)[0]
        self.prune(limit)
        self.assertFalse(old.exists())
        self.assertTrue(new.exists())
        self.assertEqual(binary.read_bytes(), b"x" * 8192)

    def test_profile_eviction_preserves_binary_and_players(self):
        deps = self.write(self.profile / "deps/old.rlib")
        self.write(self.profile / ".fingerprint/old/hash")
        binary = self.write(self.profile / "blockloom")
        player = self.write(self.profile / "players/native/blockloom-runtime")
        limit = cleaner.usage(self.target)[0] - cleaner.usage(deps.parent)[0] - cleaner.usage(self.profile / ".fingerprint")[0]
        self.prune(limit)
        self.assertFalse(deps.exists())
        self.assertFalse((self.profile / ".fingerprint").exists())
        self.assertTrue(binary.exists())
        self.assertTrue(player.exists())
        self.assertLessEqual(cleaner.usage(self.target)[0], limit)

    def test_busy_profile_is_untouched(self):
        file = self.write(self.profile / "incremental/old/data")
        with (self.profile / ".cargo-build-lock").open("a+b") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            output = self.prune(1)
        self.assertTrue(file.exists())
        self.assertIn("active builds", output)

    def test_legacy_cargo_lock_is_respected(self):
        file = self.write(self.profile / "incremental/old/data")
        with (self.profile / ".cargo-lock").open("a+b") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            self.prune(1)
        self.assertTrue(file.exists())

    def test_dry_run_preserves_data(self):
        file = self.write(self.profile / "incremental/old/data")
        self.assertIn("Would remove", self.prune(1, dry_run=True))
        self.assertTrue(file.exists())

    def test_web_and_external_build_caches_count(self):
        web = self.write(self.target / "web-build/dist/incremental/old/data")
        external = Path(self.temp.name) / "intermediates"
        cache = self.write(external / "debug/incremental/old/data")
        os.environ["CARGO_BUILD_BUILD_DIR"] = str(external)
        self.prune(1)
        self.assertFalse(web.exists())
        self.assertFalse(cache.exists())

    def test_preview_keeps_resource_manifests(self):
        manifest = self.write(self.profile / "build/qt/out/module.qrc")
        output = self.prune(1, keep_profiles=["release"])
        self.assertTrue(manifest.exists())
        self.assertIn("protected outputs", output)

    def test_symlink_is_not_followed(self):
        file = self.write(Path(self.temp.name) / "outside/data")
        (self.target).mkdir()
        (self.target / "linked").symlink_to(file.parent, target_is_directory=True)
        self.prune(1)
        self.assertTrue(file.exists())

    def test_hard_links_are_counted_once(self):
        binary = self.write(self.profile / "blockloom")
        alias = self.profile / "deps/blockloom"
        alias.parent.mkdir()
        os.link(binary, alias)
        expected = binary.stat().st_blocks * 512
        expected += sum(path.stat().st_blocks * 512 for path in (self.target, self.profile, alias.parent))
        self.assertEqual(cleaner.usage(self.target)[0], expected)
        self.prune(1)
        self.assertTrue(binary.exists())

    def test_command_failure_still_cleans_up(self):
        command = "from pathlib import Path; import sys; p=Path(sys.argv[1]); p.parent.mkdir(parents=True); p.write_bytes(b'x'*8192); sys.exit(7)"
        cache = self.profile / "incremental/new/data"
        result = subprocess.run([
            sys.executable, str(Path(__file__).with_name("prune-target.py")),
            "--target-dir", str(self.target), "--limit-gib", "0.000001",
            "--run", sys.executable, "-c", command, str(cache),
        ], capture_output=True, text=True)
        self.assertEqual(result.returncode, 7, result.stderr)
        self.assertFalse(cache.exists())


if __name__ == "__main__":
    unittest.main()
