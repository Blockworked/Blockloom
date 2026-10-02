#!/usr/bin/env python3
"""Check packaged archive integrity and the concurrent CI build profiles."""

import importlib.util
import json
import os
from pathlib import Path
import tempfile
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
            with patch.dict(os.environ, {"BLOCKLOOM_INSTALL_DIR": "system"}), patch("sys.argv", ["ci-build", *( ["--shipping"] if shipping else [])]), \
                    patch.object(build.subprocess, "run") as commands, patch.object(build.replace, "host_target", return_value="fixture"), patch.object(build.replace, "run_builds", return_value=0) as concurrent:
                self.assertEqual(build.main(), 0)
                self.assertEqual(os.environ["BLOCKLOOM_WEB_PROFILE"], "dist" if shipping else "release")
                self.assertNotIn("BLOCKLOOM_INSTALL_DIR", os.environ)
                concurrent.assert_called_once_with("dist" if shipping else "release")
                self.assertFalse(any("install" in call.args[0] or "uninstall" in call.args[0] for call in commands.call_args_list))

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
