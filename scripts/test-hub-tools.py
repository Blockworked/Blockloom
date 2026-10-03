#!/usr/bin/env python3
"""Exercise the shared Hub tool cache without downloading toolchains."""

import os
from pathlib import Path
import sys
import tempfile
import unittest

sys.dont_write_bytecode = True
import hub
import hub_tools


class ToolCacheTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name).resolve()
        self.service = hub.Hub(self.base / "Hub", self.base / "projects.json")
        self.bundle = self.base / "bundle"
        self.bundle.mkdir()
        self.binary(self.bundle, "blockloom")
        self.binary(self.bundle, "blockloom-runtime")
        self.rust(self.bundle)
        self.manifest = {"kind": "release", "version": "0.1.0", "target": hub.host_target(),
                         "tools": {"rust": {"channel": "1.98.1"}}}
        self.save_manifest()

    def binary(self, directory, name):
        directory.mkdir(parents=True, exist_ok=True)
        path = directory / (name + (".exe" if os.name == "nt" else ""))
        path.write_text("fake binary", encoding="utf-8")
        path.chmod(0o755)

    def rust(self, directory):
        rust = hub.tool_path(directory, "rust")
        self.binary(rust / "bin", "rustc")
        self.binary(rust / "bin", "cargo")
        (rust / "lib" / "rustlib" / hub.host_target() / "lib").mkdir(parents=True)

    def save_manifest(self):
        hub.write_json(self.bundle / hub.MANIFEST, self.manifest)

    def java_bundle(self, version="25.0.1+1"):
        self.binary(self.bundle / "tools/java/bin", "java")
        self.manifest["tools"]["java"] = {"version": version}
        self.save_manifest()

    def test_release_tools_are_cached_and_shared(self):
        self.java_bundle("25.0.1+1")
        first = self.service.install(self.bundle, ["java"])
        cache = self.service.tool_cache_dir()
        self.assertTrue((cache / hub_tools.java_key("25.0.1+1")).is_dir())
        self.assertTrue((cache / hub_tools.rust_key("1.98.1")).is_dir())
        status = self.service.tool_cache()
        self.assertTrue(all(entry["live"] for entry in status["entries"]))
        # A second editor with the same versions reuses the cache entries.
        self.manifest["version"] = "0.2.0"
        self.save_manifest()
        second = self.service.install(self.bundle, ["java"])
        self.assertEqual(first["tools"]["java"], second["tools"]["java"])
        entries = [entry["key"] for entry in self.service.tool_cache()["entries"]]
        self.assertEqual(len([key for key in entries if key.startswith("java-")]), 1)
        self.assertEqual(len([key for key in entries if key.startswith("rust-")]), 1)

    def test_reconfiguring_drops_unused_cached_versions(self):
        self.java_bundle("25.0.1+1")
        self.service.install(self.bundle, ["java"])
        java_key = hub_tools.java_key("25.0.1+1")
        self.assertTrue((self.service.tool_cache_dir() / java_key).is_dir())
        # A second release without Java still keeps the first editor's cache.
        self.manifest["version"] = "0.2.0"
        del self.manifest["tools"]["java"]
        self.save_manifest()
        self.service.install(self.bundle)
        self.assertTrue((self.service.tool_cache_dir() / java_key).is_dir())
        # Once no installed editor uses that Java, its cache goes away.
        self.service.uninstall("release-0.1.0")
        self.assertFalse((self.service.tool_cache_dir() / java_key).exists())
        # The Rust toolchain is still used by both editors, so it stays.
        self.assertTrue((self.service.tool_cache_dir() / hub_tools.rust_key("1.98.1")).is_dir())

    def test_uninstall_prunes_its_tool_versions(self):
        self.java_bundle("21.0.5+11")
        self.service.install(self.bundle, ["java"])
        java_key = hub_tools.java_key("21.0.5+11")
        self.assertTrue((self.service.tool_cache_dir() / java_key).is_dir())
        removed = self.service.uninstall("release-0.1.0")
        self.assertEqual(removed["id"], "release-0.1.0")
        self.assertFalse((self.service.tool_cache_dir() / java_key).exists())
        self.assertFalse(self.service.slot("release-0.1.0").exists())

    def test_uninstall_refuses_editors_in_use(self):
        self.service.install(self.bundle)
        project = self.base / "Demo"
        project.mkdir()
        hub.write_json(project / "project.blockloom", {
            "name": "Demo", "scenes": [{"id": "main", "path": "main.blockscene", "mode": "TwoD"}],
            "active_scene": "main"})
        self.service.remember(project)
        self.service.bind(project, "release-0.1.0")
        with self.assertRaisesRegex(ValueError, "used by projects"):
            self.service.uninstall("release-0.1.0")
        self.assertTrue(self.service.slot("release-0.1.0").exists())

    def test_prune_tools_reports_live_entries(self):
        self.service.install(self.bundle)
        status = self.service.prune_tools()
        self.assertEqual(status["removed"], [])
        self.assertTrue(any(entry["key"].startswith("rust-") for entry in status["entries"]))

    def test_rust_targets_are_reused_without_redownloading(self):
        import hub_download
        from unittest.mock import patch
        rust = hub.tool_path(self.bundle, "rust")
        cache = self.service.tool_cache_dir()
        target = hub.ANDROID_TARGETS[0]
        key = hub_tools.rust_target_key("1.98.1", target)
        cached_target = cache / key
        (cached_target / "lib").mkdir(parents=True)
        (cached_target / "lib" / "libstd.rlib").write_bytes(b"std")
        temporary = self.base / "temp"
        temporary.mkdir()
        # No network is needed: the cached target satisfies the install.
        hub_download.install_rust_targets(rust, "1.98.1", [target], temporary, cache)
        self.assertEqual((rust / "lib/rustlib" / target / "lib" / "libstd.rlib").read_bytes(), b"std")
        with patch.object(hub_download, "download", side_effect=AssertionError("must not redownload")):
            hub_download.install_rust_targets(rust, "1.98.1", [target], temporary, cache)


if __name__ == "__main__":
    unittest.main()
