#!/usr/bin/env python3
"""Exercise Hub install and launch behavior without building an editor."""

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
import hub
import hub_install


class HubTests(unittest.TestCase):
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
        self.project = self.base / "A project with spaces"
        self.project.mkdir()
        hub.write_json(self.project / "project.blockloom", {
            "name": "Demo", "scenes": [{"id": "main", "path": "main.blockscene", "mode": "ThreeD"}],
            "active_scene": "main"})

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

    def test_release_is_unique_and_immutable(self):
        first = self.service.install(self.bundle)
        with self.assertRaisesRegex(ValueError, "already exists"):
            self.service.install(self.bundle)
        with self.assertRaisesRegex(ValueError, "cannot be replaced"):
            hub_install.stage(self.bundle, self.service.slot(first["id"]), replace=True)
        self.assertEqual(self.service.installations(), [first])

    def test_new_version_does_not_change_project_binding(self):
        self.service.install(self.bundle)
        self.service.bind(self.project, "release-0.1.0")
        self.manifest["version"] = "0.2.0"
        self.save_manifest()
        self.service.install(self.bundle)
        self.assertEqual(self.service.project(self.project)["installation"], "release-0.1.0")
        before = (self.project / "project.blockloom").read_bytes()
        self.service.bind(self.project, "release-0.2.0")
        self.assertEqual((self.project / "project.blockloom").read_bytes(), before)

    def test_project_binding_survives_folder_move(self):
        self.service.install(self.bundle)
        self.service.bind(self.project, "release-0.1.0")
        renamed = self.base / "Renamed"
        self.project.rename(renamed)
        self.assertEqual(self.service.project(renamed)["installation"], "release-0.1.0")

    def test_registry_is_read_only_and_retains_missing_projects(self):
        hub.write_json(self.service.registry, {"projects": [
            {"path": str(self.project), "opened_at": 100},
            {"path": str(self.base / "missing"), "opened_at": 101}]})
        before = self.service.registry.read_bytes()
        entries = self.service.projects()
        self.assertIn("error", entries[0])
        self.assertEqual(entries[1]["mode"], "ThreeD")
        self.assertIsNone(entries[1]["installation"])
        self.assertEqual(self.service.registry.read_bytes(), before)

    def test_missing_required_rust_is_rejected(self):
        self.manifest["tools"] = {}
        self.save_manifest()
        with self.assertRaisesRegex(ValueError, "pinned Rust"):
            self.service.install(self.bundle)
        self.assertEqual(self.service.installations(), [])

    def test_components_are_optional_and_filter_payload(self):
        for name in hub.OPTIONAL_TOOLS:
            hub.tool_path(self.bundle, name).mkdir(parents=True)
            self.manifest["tools"][name] = {"version": "test"}
        self.binary(hub.tool_path(self.bundle, "java") / "bin", "java")
        self.binary(hub.tool_path(self.bundle, "android-sdk") / "platform-tools", "adb")
        (hub.tool_path(self.bundle, "android-ndk") / "source.properties").write_text("Pkg.Revision = 27.0.0")
        for target in hub.ANDROID_TARGETS:
            (hub.tool_path(self.bundle, "rust") / "lib" / "rustlib" / target / "lib").mkdir(parents=True)
        self.manifest["tools"]["android-rust-targets"] = list(hub.ANDROID_TARGETS)
        self.save_manifest()
        installed = self.service.install(self.bundle)
        directory = self.service.slot(installed["id"])
        self.assertEqual(set(installed["tools"]), {"rust"})
        for name in hub.OPTIONAL_TOOLS:
            self.assertFalse(hub.tool_path(directory, name).exists())
        for target in hub.ANDROID_TARGETS:
            self.assertFalse((hub.tool_path(directory, "rust") / "lib" / "rustlib" / target).exists())
        self.manifest["version"] = "0.2.0"
        self.save_manifest()
        installed = self.service.install(self.bundle, hub.OPTIONAL_TOOLS, True)
        self.assertEqual(set(installed["tools"]), {"rust", *hub.OPTIONAL_TOOLS, "android-rust-targets"})

    def test_unavailable_component_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "unavailable"):
            self.service.install(self.bundle, ["java"])

    def test_foreign_bundle_and_invalid_versions_are_rejected(self):
        for version in ("../../bad", "01.0.0", "0.1.0+build", "latest"):
            self.manifest["version"] = version
            self.save_manifest()
            with self.assertRaises(ValueError):
                self.service.install(self.bundle)
        self.manifest.update(version="0.1.0", target="foreign-target")
        self.save_manifest()
        with self.assertRaisesRegex(ValueError, "host target"):
            self.service.install(self.bundle)

    def test_invalid_identity_cannot_escape_installations(self):
        for identity in ("../release-1.0.0", "dev-../../outside", "C:/elsewhere"):
            with self.assertRaises(ValueError):
                self.service.slot(identity)

    def test_lock_serializes_mutations(self):
        with self.service.mutation():
            with self.assertRaisesRegex(ValueError, "Another Hub operation"):
                self.service.install(self.bundle)
        self.service.install(self.bundle)

    def repo(self):
        repo = self.base / "repo with spaces"
        repo.mkdir()
        (repo / "Cargo.toml").write_text('[workspace]\nmembers = ["blockloom-qt"]\n[workspace.package]\nversion = "0.0.1"\n')
        (repo / "rust-toolchain.toml").write_text('[toolchain]\nchannel = "1.98.1"\n')
        (repo / "justfile").touch()
        return repo

    def test_repo_registration_is_idempotent(self):
        repo = self.repo()
        first = self.service.add_dev(repo)
        self.assertEqual(first, self.service.add_dev(repo / "."))
        self.assertEqual(first["status"], "unbuilt")
        with self.assertRaisesRegex(ValueError, "Build this"):
            self.service.bind(self.project, first["id"])

    def test_failed_rebuild_preserves_previous_installation(self):
        dev = self.service.add_dev(self.repo())
        directory = self.service.slot(dev["id"])
        hub_install.stage(self.bundle, directory, {**dev, "status": "ready", "tools": self.manifest["tools"]}, replace=True)
        before = (directory / hub.MANIFEST).read_bytes()
        with patch.object(hub.subprocess, "run", side_effect=subprocess.CalledProcessError(3, "just")):
            with self.assertRaises(subprocess.CalledProcessError):
                self.service.rebuild(dev["id"])
        self.assertEqual((directory / hub.MANIFEST).read_bytes(), before)
        self.assertFalse((self.service.root / ".operation-lock").exists())

    def test_rebuild_uses_repo_and_private_destination(self):
        repo = self.repo()
        dev = self.service.add_dev(repo)

        def build(command, **kwargs):
            self.assertEqual(command[-2:], ["replace", "2"])
            self.assertEqual(kwargs["cwd"], repo)
            output = Path(kwargs["env"]["BLOCKLOOM_INSTALL_DIR"])
            self.binary(output, "blockloom")
            self.binary(output, "blockloom-runtime")

        rustc = hub.tool_path(self.bundle, "rust") / "bin" / ("rustc.exe" if os.name == "nt" else "rustc")
        with patch.object(hub.subprocess, "run", build), patch.object(hub.subprocess, "check_output", return_value=str(rustc)):
            result = self.service.rebuild(dev["id"], 2)
        self.assertEqual(result["tools"]["rust"]["channel"], "1.98.1")
        self.service.installation(dev["id"], ready=True)

    def test_launch_passes_path_and_bundled_tools_without_shell(self):
        self.service.install(self.bundle)
        self.service.bind(self.project, "release-0.1.0")
        with patch.object(hub.subprocess, "Popen") as launch:
            launch.return_value.pid = 42
            result = self.service.launch(self.project)
        args, kwargs = launch.call_args
        self.assertEqual(args[0][-2:], ["--project", str(self.project)])
        self.assertEqual(result["pid"], 42)
        self.assertTrue(kwargs["env"]["PATH"].startswith(str(self.service.slot("release-0.1.0") / "tools" / "rust" / "bin")))
        self.assertNotIn("shell", kwargs)

    def test_stage_copy_failure_preserves_existing_payload(self):
        destination = self.base / "dev-payload"
        hub_install.stage(self.bundle, destination, {"kind": "dev"})
        before = (destination / hub.MANIFEST).read_bytes()
        with patch.object(hub_install.shutil, "copy2", side_effect=OSError("disk full")):
            with self.assertRaises(OSError):
                hub_install.stage(self.bundle, destination, replace=True)
        self.assertEqual((destination / hub.MANIFEST).read_bytes(), before)

    def test_stage_rename_failure_rolls_back(self):
        destination = self.base / "dev-payload"
        hub_install.stage(self.bundle, destination, {"kind": "dev"})
        original = Path.rename

        def rename(path, target):
            if path.name == "payload":
                raise OSError("rename failed")
            return original(path, target)

        with patch.object(Path, "rename", rename):
            with self.assertRaises(OSError):
                hub_install.stage(self.bundle, destination, {"kind": "dev", "changed": True}, replace=True)
        self.assertEqual(hub.read_json(destination / hub.MANIFEST), {"kind": "dev"})


if __name__ == "__main__":
    unittest.main()
