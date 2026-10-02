#!/usr/bin/env python3
"""Check packaged archive integrity and isolated CI build components."""

import importlib.util
import json
import os
from pathlib import Path
import tempfile
import tarfile
import unittest
from unittest.mock import patch
import zipfile

import hub_download


def load(name):
    spec = importlib.util.spec_from_file_location(name.replace("-", "_"), Path(__file__).with_name(name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


packaging = load("package-release")
build = load("ci-build")
catalogs = load("release-catalog")


class PackageTests(unittest.TestCase):
    def test_archive_integrity_and_no_directory_prefix(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            directory = root / "bundle"
            (directory / "players").mkdir(parents=True)
            (directory / "editor").write_bytes(b"exe")
            (directory / "players/web.wasm").write_bytes(b"wasm")
            archive = root / "archive.zip"
            entry = packaging.archive(directory, archive)
            self.assertEqual(entry["unpacked_size"], 7)
            hub_download.verify_file(archive, entry["sha256"], entry["size"])
            hub_download.extract(archive, root / "out", "zip", entry["unpacked_size"])
            self.assertEqual((root / "out/players/web.wasm").read_bytes(), b"wasm")
            with zipfile.ZipFile(archive) as data:
                self.assertEqual(data.namelist(), ["editor", "players/web.wasm"])

    def test_ci_profiles_and_no_system_install(self):
        for shipping in (False, True):
            for component in ("editor", "native", "web"):
                with self.subTest(shipping=shipping, component=component), \
                        patch.dict(os.environ, {"BLOCKLOOM_INSTALL_DIR": "system", "CARGO_BUILD_JOBS": "4"}), \
                        patch("sys.argv", ["ci-build", "--component", component, *(["--shipping"] if shipping else [])]), \
                        patch.object(build.subprocess, "run") as commands, \
                        patch.object(build.replace, "just_exe", return_value="just"), \
                        patch.object(build.replace, "host_target", return_value="fixture"), \
                        patch.object(build, "archive_outputs") as archive:
                    self.assertEqual(build.main(), 0)
                    self.assertEqual(os.environ["CARGO_BUILD_JOBS"], "4")
                    self.assertNotIn("BLOCKLOOM_INSTALL_DIR", os.environ)
                    archive.assert_called_once_with(component)
                    calls = [call.args[0] for call in commands.call_args_list]
                    expected = {"editor": ["just", "build"],
                                "native": ["just", "player" if shipping else "_stage-release-player"],
                                "web": ["just", "web-player", "dist" if shipping else "release"]}
                    self.assertIn(expected[component], calls)
                    self.assertEqual(any("web-tools" in call for call in calls), component == "web")
                    self.assertEqual(any("-p" in call for call in calls), component == "native" and not shipping)
                    self.assertFalse(any("install" in call or "uninstall" in call for call in calls))

    def test_component_artifacts_preserve_paths_and_modes(self):
        with tempfile.TemporaryDirectory() as temporary, patch.object(build.replace, "host_target", return_value="fixture"):
            previous = Path.cwd()
            try:
                os.chdir(temporary)
                directory = Path("target/release")
                directory.mkdir(parents=True)
                suffix = ".exe" if os.name == "nt" else ""
                for name in ("blockloom", "blockloom-hub", "blockloom-runtime", "blockloom-shell",
                             "blockloom-plugin-worker"):
                    binary = directory / (name + suffix)
                    binary.write_bytes(b"exe")
                    binary.chmod(0o755)
                (directory / "unused.rlib").write_bytes(b"cache")
                for target in ("fixture", "wasm32-unknown-unknown"):
                    player = directory / "players" / target
                    player.mkdir(parents=True)
                    (player / "payload").write_bytes(b"player")
                for component in ("editor", "native", "web"):
                    build.archive_outputs(component)
                    with tarfile.open("build-output.tar") as archive:
                        names = archive.getnames()
                        if component == "editor":
                            self.assertEqual(len(names), 5)
                            self.assertEqual(archive.getmember("target/release/blockloom" + suffix).mode & 0o111, 0o111)
                        else:
                            target = "fixture" if component == "native" else "wasm32-unknown-unknown"
                            self.assertEqual(names, [f"target/release/players/{target}", f"target/release/players/{target}/payload"])
            finally:
                os.chdir(previous)

    def test_catalog_merge_requires_all_platforms_and_checksums(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            content = root / "content"
            content.mkdir()
            (content / "editor").write_bytes(b"exe")
            for target in ("windows", "linux", "macos"):
                name = target + ".zip"
                entry = {**packaging.archive(content, root / name), "version": "0.1.0", "target": target,
                         "asset": name, "url": "https://example.org/" + name}
                (root / ("catalog-" + target + ".json")).write_text(json.dumps({"schema": 1, "releases": [entry]}))
            catalogs.combine(root)
            self.assertEqual(len(json.loads((root / "blockloom-catalog.json").read_bytes())["releases"]), 3)
            (root / "linux.zip").write_bytes(b"tampered")
            with self.assertRaisesRegex(ValueError, "size"):
                catalogs.combine(root)
            (root / "catalog-linux.json").unlink()
            with self.assertRaisesRegex(ValueError, "each desktop platform"):
                catalogs.combine(root)


if __name__ == "__main__":
    unittest.main()
