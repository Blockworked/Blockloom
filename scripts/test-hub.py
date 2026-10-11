#!/usr/bin/env python3
"""Exercise Hub install and launch behavior without building an editor."""

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
import hub
import hub_install
import hub_testing


def setUpModule():
    hub_testing.ensure_hubctl()


class HubTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name).resolve()
        self.service = hub.Hub(self.base / "Hub", self.base / "data")
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
        self.assertEqual(self.service.ctl("installations"),
                         [{**first, "path": str(self.service.slot(first["id"]))}])

    def test_prepared_payload_promotes_without_copying(self):
        destination = self.base / "promoted"
        with patch.object(hub_install.shutil, "copytree", side_effect=AssertionError("Unexpected copy")):
            hub_install.promote(self.bundle, destination, {"kind": "dev"})
        self.assertFalse(self.bundle.exists())
        self.assertTrue((destination / hub_install.editor_name()).exists())

    def test_failed_promotion_restores_previous_installation(self):
        destination = self.base / "promoted"
        hub_install.stage(self.bundle, destination, {"kind": "dev", "version": "old"})
        rename = Path.rename

        def fail_source(path, target):
            if path == self.bundle:
                raise OSError("Fixture rename failure")
            return rename(path, target)

        with patch.object(Path, "rename", fail_source), self.assertRaises(OSError):
            hub_install.promote(self.bundle, destination, {"kind": "dev"}, replace=True)
        self.assertEqual(hub.read_json(destination / hub.MANIFEST)["version"], "old")
        self.assertTrue(self.bundle.exists())

    def test_rust_bundle_keeps_runtime_and_skips_documentation(self):
        import hub_process
        source = hub.tool_path(self.bundle, "rust")
        for name in ("share/doc/rust/html/index.html", "lib/rustlib/src/rust/library/std.rs", "libexec/rust-analyzer-proc-macro-srv"):
            path = source / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("fixture")
        target = self.base / "bundled-rust"
        hub_process.bundle_rust(source, target, "1.98.1")
        self.assertTrue((target / "bin" / ("rustc.exe" if os.name == "nt" else "rustc")).exists())
        self.assertTrue((target / "libexec/rust-analyzer-proc-macro-srv").exists())
        self.assertTrue((target / "lib/rustlib" / hub.host_target() / "lib").exists())
        self.assertFalse((target / "share").exists())
        self.assertFalse((target / "lib/rustlib/src").exists())

    def test_new_version_does_not_change_project_binding(self):
        self.service.install(self.bundle)
        self.service.ctl("bind", self.project, "release-0.1.0")
        self.manifest["version"] = "0.2.0"
        self.save_manifest()
        self.service.install(self.bundle)
        self.assertEqual(self.service.ctl("remember", self.project)["installation"], "release-0.1.0")
        before = (self.project / "project.blockloom").read_bytes()
        self.service.ctl("bind", self.project, "release-0.2.0")
        self.assertEqual((self.project / "project.blockloom").read_bytes(), before)

    def test_private_release_download_verifies_and_keeps_project_binding(self):
        import hashlib
        import zipfile
        self.service.install(self.bundle)
        self.service.ctl("bind", self.project, "release-0.1.0")
        self.manifest["version"] = "0.2.0"
        self.save_manifest()
        archive = self.base / "remote.zip"
        (hub.tool_path(self.bundle, "rust") / "lib/rustlib" / hub.host_target() / "lib/libstd.rlib").write_bytes(b"std")
        with zipfile.ZipFile(archive, "w") as data:
            for file in self.bundle.rglob("*"):
                if file.is_file():
                    data.write(file, file.relative_to(self.bundle).as_posix())
        checksum = hashlib.sha256(archive.read_bytes()).hexdigest()
        entry = {"version": "0.2.0", "size": archive.stat().st_size, "unpacked_size": 10000,
                 "format": "zip", "sha256": checksum, "github": {"repo": "Blockworked/Blockloom", "tag": "0.2.0", "asset": "editor.zip"}}

        def asset(repo, tag, name, destination, size=None):
            destination.write_bytes(archive.read_bytes())

        self.service.release_settings("github-cli", repo="Blockworked/Blockloom")
        self.assertEqual(self.service.release_settings()["source"], "github-cli")
        with patch.object(hub.hub_github, "catalog", return_value=[entry]), patch.object(hub.hub_github, "asset", side_effect=asset):
            with self.assertRaisesRegex(ValueError, "catalog changed"):
                self.service.download_release("0.2.0", sha256="0" * 64)
            installed = self.service.download_release("0.2.0", sha256=checksum)
        self.assertEqual(installed["id"], "release-0.2.0")
        self.assertEqual(self.service.ctl("remember", self.project)["installation"], "release-0.1.0")

    def test_release_source_defaults_to_github_cli(self):
        settings = self.service.release_settings()
        self.assertEqual(settings["source"], "github-cli")
        self.assertEqual(settings["repo"], "Blockworked/Blockloom")

    def test_installations_list_slot_paths(self):
        installed = self.service.install(self.bundle)
        entries = self.service.ctl("installations")
        self.assertEqual(entries[0]["path"], str(self.service.slot(installed["id"])))

    def test_uninstall_refuses_installation_used_by_a_project(self):
        self.service.install(self.bundle)
        self.service.ctl("remember", self.project)
        self.service.ctl("bind", self.project, "release-0.1.0")
        with self.assertRaisesRegex(ValueError, "used by projects"):
            self.service.uninstall("release-0.1.0")
        self.assertTrue(self.service.slot("release-0.1.0").exists())

    def test_add_tools_downloads_missing_components_into_place(self):
        (hub.tool_path(self.bundle, "rust") / "lib/rustlib" / hub.host_target() / "lib/libstd.rlib").write_bytes(b"std")
        self.service.install(self.bundle)

        def tools(directory, versions, components, temporary, cache_dir=None):
            self.assertEqual(list(components), ["java"])
            self.assertEqual(versions["JDK_MAJOR"], "25")
            java = hub.tool_path(Path(directory), "java") / "bin"
            java.mkdir(parents=True)
            (java / ("java.exe" if os.name == "nt" else "java")).write_text("fake java", encoding="utf-8")
            return {"java": {"version": "25.0.1"}}

        with patch.object(hub.hub_download, "install_android_tools", side_effect=tools) as fetch:
            result = self.service.add_tools("release-0.1.0", ["java"])
        fetch.assert_called_once()
        self.assertEqual(result["added"], ["java"])
        self.assertEqual(result["tools"]["java"], {"version": "25.0.1"})
        slot = self.service.slot("release-0.1.0")
        self.assertTrue((slot / "tools/java/bin" / ("java.exe" if os.name == "nt" else "java")).is_file())
        self.assertTrue((slot / ("blockloom.exe" if os.name == "nt" else "blockloom")).exists())
        # A second call is a no-op and downloads nothing.
        with patch.object(hub.hub_download, "install_android_tools",
                           side_effect=AssertionError("already installed")):
            again = self.service.add_tools("release-0.1.0", ["java"])
        self.assertEqual(again["added"], [])

    def test_add_tools_downloads_missing_android_rust_targets(self):
        (hub.tool_path(self.bundle, "rust") / "lib/rustlib" / hub.host_target() / "lib/libstd.rlib").write_bytes(b"std")
        self.service.install(self.bundle)

        def targets(rust, channel, wanted, temporary, cache_dir=None):
            for target in wanted:
                (Path(rust) / "lib" / "rustlib" / target / "lib").mkdir(parents=True)

        with patch.object(hub.hub_download, "install_rust_targets", side_effect=targets) as fetch:
            result = self.service.add_tools("release-0.1.0", [], android_rust_targets=True)
        fetch.assert_called_once()
        self.assertEqual(result["added"], ["android-rust-targets"])
        self.assertEqual(set(result["tools"]["android-rust-targets"]), set(hub.ANDROID_TARGETS))

    def test_add_tools_rejects_unknown_components(self):
        self.service.install(self.bundle)
        with self.assertRaisesRegex(ValueError, "Unknown component"):
            self.service.add_tools("release-0.1.0", ["not-a-tool"])

    def test_add_tools_refuses_installation_in_use(self):
        self.service.install(self.bundle)
        self.service.ctl("remember", self.project)
        self.service.ctl("bind", self.project, "release-0.1.0")
        hub.write_json(self.project / ".blockloom/lock.json", {"pid": os.getpid(), "heartbeat": time.time()})
        with self.assertRaisesRegex(ValueError, "Close projects"):
            self.service.add_tools("release-0.1.0", ["java"])
        with self.assertRaisesRegex(ValueError, "Close projects"):
            self.service.uninstall("release-0.1.0")

    def test_release_download_fetches_optional_tools_missing_from_bundle(self):
        import hashlib
        import zipfile
        self.manifest["version"] = "0.2.0"
        self.save_manifest()
        archive = self.base / "remote.zip"
        (hub.tool_path(self.bundle, "rust") / "lib/rustlib" / hub.host_target() / "lib/libstd.rlib").write_bytes(b"std")
        with zipfile.ZipFile(archive, "w") as data:
            for file in self.bundle.rglob("*"):
                if file.is_file():
                    data.write(file, file.relative_to(self.bundle).as_posix())
        checksum = hashlib.sha256(archive.read_bytes()).hexdigest()
        entry = {"version": "0.2.0", "size": archive.stat().st_size, "unpacked_size": 10000,
                 "format": "zip", "sha256": checksum, "github": {"repo": "Blockworked/Blockloom", "tag": "0.2.0", "asset": "editor.zip"}}

        def asset(repo, tag, name, destination, size=None):
            destination.write_bytes(archive.read_bytes())

        def tools(directory, versions, components, temporary, cache_dir=None):
            self.assertEqual(list(components), ["java"])
            self.assertEqual(versions["JDK_MAJOR"], "25")
            java = hub.tool_path(Path(directory), "java") / "bin"
            java.mkdir(parents=True)
            (java / ("java.exe" if os.name == "nt" else "java")).write_text("fake java", encoding="utf-8")
            return {"java": {"version": "25.0.1"}}

        self.service.release_settings("github-cli", repo="Blockworked/Blockloom")
        with patch.object(hub.hub_github, "catalog", return_value=[entry]), patch.object(hub.hub_github, "asset", side_effect=asset), \
                patch.object(hub.hub_download, "install_android_tools", side_effect=tools):
            installed = self.service.download_release("0.2.0", ["java"], False, checksum)
        self.assertEqual(installed["tools"]["java"], {"version": "25.0.1"})
        self.assertTrue((self.service.slot("release-0.2.0") / "tools/java/bin" / ("java.exe" if os.name == "nt" else "java")).is_file())

    def prepare_upgrade(self):
        self.service.install(self.bundle)
        self.service.ctl("bind", self.project, "release-0.1.0")
        self.manifest["version"] = "0.2.0"
        self.save_manifest()
        self.service.install(self.bundle)

    def test_editor_upgrade_backs_up_before_binding(self):
        import zipfile
        self.prepare_upgrade()
        assets = self.project / "assets"
        assets.mkdir()
        (assets / "picture.png").write_bytes(b"game asset")
        (assets / "empty").mkdir()
        cache = self.project / ".blockloom/build"
        cache.mkdir()
        (cache / "cache.dll").write_bytes(b"generated")
        before = (self.project / "project.blockloom").read_bytes()
        result = self.service.ctl("bind", self.project, "release-0.2.0", "--backup")
        with zipfile.ZipFile(result["backup"]["path"]) as archive:
            self.assertEqual(archive.read("project.blockloom"), before)
            self.assertEqual(archive.read("assets/picture.png"), b"game asset")
            self.assertEqual(json.loads(archive.read(".blockloom/hub.json"))["installation"], "release-0.1.0")
            self.assertIn("assets/empty/", archive.namelist())
            self.assertFalse(any(name.startswith(".blockloom/build") for name in archive.namelist()))
        self.assertEqual(self.service.ctl("remember", self.project)["installation"], "release-0.2.0")
        self.assertEqual((self.project / "project.blockloom").read_bytes(), before)

    def test_missing_required_rust_is_rejected(self):
        self.manifest["tools"] = {}
        self.save_manifest()
        with self.assertRaisesRegex(ValueError, "pinned Rust"):
            self.service.install(self.bundle)

    def test_modules_list_versions_sizes_and_default_rule(self):
        result = self.service.modules()
        ids = [m["id"] for m in result["modules"]]
        self.assertEqual(ids, ["java", "android-sdk", "android-ndk", "android-rust-targets"])
        self.assertIn("Java 25", result["modules"][0]["label"])
        self.assertTrue(all(m["approx_mib"] > 0 for m in result["modules"]))
        self.assertEqual(result["defaults"]["release"], [])
        self.assertEqual(result["defaults"]["dev"], ids)

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
        android = repo / "blockloom-core/src/android.rs"
        android.parent.mkdir(parents=True)
        android.write_text((Path(__file__).resolve().parent.parent / "blockloom-core/src/android.rs").read_text())
        return repo

    def test_repo_registration_is_idempotent(self):
        repo = self.repo()
        first = self.service.add_dev(repo)
        self.assertEqual(first, self.service.add_dev(repo / "."))
        self.assertEqual(first["status"], "unbuilt")
        with self.assertRaisesRegex(ValueError, "Build this"):
            self.service.ctl("bind", self.project, first["id"])

    def test_failed_rebuild_preserves_previous_installation(self):
        dev = self.service.add_dev(self.repo())
        directory = self.service.slot(dev["id"])
        hub_install.stage(self.bundle, directory, {**dev, "status": "ready", "tools": self.manifest["tools"]}, replace=True)
        before = (directory / hub.MANIFEST).read_bytes()
        with patch.object(hub, "run", side_effect=subprocess.CalledProcessError(3, "just")):
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
        for target in hub.ANDROID_TARGETS:
            (hub.tool_path(self.bundle, "rust") / "lib/rustlib" / target / "lib").mkdir(parents=True)
        with patch.object(hub, "run", build), patch.object(hub.subprocess, "check_output", return_value=str(rustc)):
            prepared = self.service.prepare_dev(dev["id"], 2)
        for target in hub.ANDROID_TARGETS:
            self.assertFalse((self.service.candidate_path(dev["id"]) / "tools/rust/lib/rustlib" / target).exists())
        self.assertEqual(self.service.installation(dev["id"])["status"], "unbuilt")
        result = self.service.install_dev(dev["id"], build_id=prepared["build_id"])
        self.assertEqual(result["tools"]["rust"]["channel"], "1.98.1")
        self.service.installation(dev["id"], ready=True)

    def prepare_fake_dev(self):
        dev = self.service.add_dev(self.repo())
        manifest = {**dev, "status": "ready", "tools": self.manifest["tools"],
                    "sources": {}, "android_targets_available": False}
        hub_install.stage(self.bundle, self.service.candidate_path(dev["id"]), manifest)
        return dev

    def test_prepared_build_requires_explicit_install_and_can_retry(self):
        dev = self.prepare_fake_dev()
        self.assertEqual(self.service.installation(dev["id"])["status"], "unbuilt")
        self.assertTrue(self.service.ctl("installations")[0]["prepared"])
        result = self.service.install_dev(dev["id"])
        self.assertEqual(result["status"], "ready")
        self.assertNotIn("sources", result)
        self.service.installation(dev["id"], ready=True)
        self.assertEqual(self.service.install_dev(dev["id"]), result)

    def test_dev_optional_tools_are_downloaded_into_private_installation(self):
        dev = self.prepare_fake_dev()

        def download(directory, versions, components, temporary, cache_dir=None):
            self.assertNotEqual(directory, self.service.slot(dev["id"]))
            self.assertEqual(versions["JDK_MAJOR"], "25")
            self.assertEqual(versions["NDK_MAJOR"], "27")
            self.assertEqual(components, hub.OPTIONAL_TOOLS)
            self.binary(hub.tool_path(directory, "java") / "bin", "java")
            self.binary(hub.tool_path(directory, "android-sdk") / "platform-tools", "adb")
            ndk = hub.tool_path(directory, "android-ndk")
            ndk.mkdir()
            (ndk / "source.properties").write_text("Pkg.Revision=27.2.12479018\n")
            return {"java": {"version": "25.0.1"}, "android-sdk": {"version": "36.0.0"},
                    "android-ndk": {"version": "27.2.12479018"}}

        with patch.object(hub.hub_download, "install_android_tools", side_effect=download) as fetch, \
                patch.dict(os.environ, {"JAVA_HOME": "local-java-must-not-be-used", "ANDROID_HOME": "local-sdk-must-not-be-used"}):
            result = self.service.install_dev(dev["id"], hub.OPTIONAL_TOOLS)
        fetch.assert_called_once()
        self.assertEqual(result["tools"]["java"]["version"], "25.0.1")
        self.assertEqual(result["tools"]["android-ndk"]["version"], "27.2.12479018")
        self.assertFalse((self.service.slot(dev["id"]) / "tools/android-sdk/ndk").exists())
        self.service.install_dev(dev["id"])
        self.assertFalse((self.service.slot(dev["id"]) / "tools/java").exists())

    def test_failed_dev_configuration_keeps_previous_install(self):
        dev = self.prepare_fake_dev()
        self.service.install_dev(dev["id"])
        before = (self.service.slot(dev["id"]) / hub.MANIFEST).read_bytes()
        with patch.object(hub.hub_download, "install_android_tools", side_effect=ValueError("Download failed")):
            with self.assertRaisesRegex(ValueError, "Download failed"):
                self.service.install_dev(dev["id"], ["java"])
        self.assertEqual((self.service.slot(dev["id"]) / hub.MANIFEST).read_bytes(), before)
        self.assertFalse((self.service.root / ".operation-lock").exists())

    def test_dev_local_tool_sources_are_rejected(self):
        dev = self.prepare_fake_dev()
        with self.assertRaisesRegex(ValueError, "local tool sources are not supported"):
            self.service.install_dev(dev["id"], ["java"], sources={"java": str(self.base)})

    def test_rebuild_includes_all_android_tools_by_default(self):
        with patch.object(self.service, "prepare_dev"), patch.object(self.service, "install_dev") as install:
            self.service.rebuild("development", 2)
        install.assert_called_once_with("development", hub.OPTIONAL_TOOLS, android_rust_targets=True)

    def test_changed_candidate_requires_reviewing_new_build(self):
        dev = self.prepare_fake_dev()
        with self.assertRaisesRegex(ValueError, "newer development build"):
            self.service.install_dev(dev["id"], build_id="previous-build")
        self.assertEqual(self.service.installation(dev["id"])["status"], "unbuilt")

    def test_dev_android_targets_are_selected_independently(self):
        dev = self.prepare_fake_dev()
        candidate = self.service.candidate_path(dev["id"])
        for target in hub.ANDROID_TARGETS:
            (candidate / "tools/rust/lib/rustlib" / target / "lib").mkdir(parents=True)
        manifest = hub.read_json(candidate / hub.MANIFEST)
        manifest["android_targets_available"] = True
        hub.write_json(candidate / hub.MANIFEST, manifest)
        selected = self.service.install_dev(dev["id"], android_rust_targets=True)
        self.assertEqual(set(selected["tools"]["android-rust-targets"]), set(hub.ANDROID_TARGETS))
        for target in hub.ANDROID_TARGETS:
            self.assertTrue((self.service.slot(dev["id"]) / "tools/rust/lib/rustlib" / target / "lib").is_dir())
        self.service.install_dev(dev["id"])
        for target in hub.ANDROID_TARGETS:
            self.assertFalse((self.service.slot(dev["id"]) / "tools/rust/lib/rustlib" / target / "lib").exists())

    def test_unavailable_android_targets_keep_current_install(self):
        dev = self.prepare_fake_dev()
        with patch.object(hub.hub_download, "install_rust_targets", side_effect=ValueError("Download failed")):
            with self.assertRaisesRegex(ValueError, "Download failed"):
                self.service.install_dev(dev["id"], android_rust_targets=True)
        self.assertEqual(self.service.installation(dev["id"])["status"], "unbuilt")

    def test_cancelled_dev_install_preserves_slot_and_releases_lock(self):
        dev = self.prepare_fake_dev()
        self.service.install_dev(dev["id"])
        before = (self.service.slot(dev["id"]) / hub.MANIFEST).read_bytes()
        cancel = self.base / "cancel"
        cancel.touch()
        with patch.dict(os.environ, {"BLOCKLOOM_HUB_CANCEL_FILE": str(cancel)}):
            with self.assertRaises(hub.Cancelled):
                self.service.install_dev(dev["id"])
        self.assertEqual((self.service.slot(dev["id"]) / hub.MANIFEST).read_bytes(), before)
        self.assertFalse((self.service.root / ".operation-lock").exists())

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

    def test_a_crashed_operation_does_not_lock_the_hub_forever(self):
        lock = self.service.root / ".operation-lock"
        lock.mkdir(parents=True)
        hub.write_json(lock / "owner.json", {"pid": 2 ** 22 + 7})
        with patch.object(hub, "process_token", return_value=None):
            self.service.install(self.bundle)
        self.assertFalse(lock.exists())
        # A lock whose owner is alive still blocks.
        lock.mkdir()
        hub.write_json(lock / "owner.json", {"pid": os.getpid()})
        with self.assertRaisesRegex(ValueError, "Another Hub operation"):
            self.service.install(self.bundle)


    CONTRACT = Path(__file__).resolve().parent.parent / "blockloom-core" / "tests" / "fixtures" / "hub-contract"

    def contract(self, name):
        return json.loads((self.CONTRACT / name).read_text(encoding="utf-8"))


if __name__ == "__main__":
    unittest.main()
