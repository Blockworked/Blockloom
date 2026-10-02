#!/usr/bin/env python3
"""Exercise remote manifests, archive validation and pinned Rust target downloads."""

import hashlib
import io
import json
import os
from pathlib import Path
import stat
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch
import zipfile

sys.dont_write_bytecode = True
import hub
import hub_download as downloads


class DownloadTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()

    def test_download_checks_hash_and_size(self):
        value = b"archive bytes"
        path = self.root / "archive"
        with patch.object(downloads, "response", return_value=io.BytesIO(value)):
            downloads.download("https://example.org/editor.zip", path, hashlib.sha256(value).hexdigest(), len(value))
        self.assertEqual(path.read_bytes(), value)
        for checksum, size in (("0" * 64, len(value)), (hashlib.sha256(value).hexdigest(), 1)):
            path.unlink()
            with patch.object(downloads, "response", return_value=io.BytesIO(value)):
                with self.assertRaises(ValueError):
                    downloads.download("https://example.org/editor.zip", path, checksum, size)
            self.assertFalse(path.exists())
            path.touch()

    def test_cancellation_removes_partial_download(self):
        path = self.root / "partial"
        with patch.object(downloads, "response", return_value=io.BytesIO(b"bytes")), patch.object(downloads, "checkpoint", side_effect=hub.Cancelled):
            with self.assertRaises(hub.Cancelled):
                downloads.download("https://example.org/editor.zip", path, "0" * 64)
        self.assertFalse(path.exists())

    def test_http_credentials_and_redirects_are_rejected(self):
        for url in ("http://example.org/file", "https://user:password@example.org/file", "https://example.org/file#fragment", "file:///file"):
            with self.assertRaises(ValueError):
                downloads.secure_url(url)
        with self.assertRaises(ValueError):
            downloads.SecureRedirect().redirect_request(None, None, 302, "", {}, "http://example.org/file")

    def zip(self, entries):
        path = self.root / "archive.zip"
        with zipfile.ZipFile(path, "w") as archive:
            for name, value in entries:
                archive.writestr(name, value)
        return path

    def test_archive_paths_and_aliases_cannot_escape(self):
        for index, name in enumerate(("../outside", "/absolute", "C:/outside", "folder\\outside", "CON", "trailing. ")):
            with self.subTest(name=name):
                archive = self.zip([(name.replace("\\", "/"), b"bad")])
                if "\\" in name:
                    archive.write_bytes(archive.read_bytes().replace(name.replace("\\", "/").encode(), name.encode()))
                with self.assertRaises(ValueError):
                    downloads.extract(archive, self.root / ("out-" + str(index)), "zip")
        self.assertFalse((self.root.parent / "outside").exists())
        archive = self.zip([("file", b"a"), ("FILE", b"b")])
        with self.assertRaisesRegex(ValueError, "duplicate"):
            downloads.extract(archive, self.root / "duplicate", "zip")

    def test_archive_links_and_oversize_content_are_rejected(self):
        info = zipfile.ZipInfo("link")
        info.create_system = 3
        info.external_attr = (stat.S_IFLNK | 0o777) << 16
        with self.assertRaisesRegex(ValueError, "links"):
            downloads.extract(self.zip([(info, b"outside")]), self.root / "links", "zip")
        with self.assertRaisesRegex(ValueError, "limits"):
            downloads.extract(self.zip([("large", b"0" * 2000)]), self.root / "large", "zip", limit=1000)
        path = self.root / "link.tar.gz"
        with tarfile.open(path, "w:gz") as archive:
            info = tarfile.TarInfo("link")
            info.type = tarfile.SYMTYPE
            info.linkname = "../outside"
            archive.addfile(info)
        with self.assertRaisesRegex(ValueError, "links"):
            downloads.extract(path, self.root / "tar-link", "tar.gz")

    def test_valid_archive_extracts_and_preserves_executable_mode(self):
        path = self.root / "editor.tar.gz"
        with tarfile.open(path, "w:gz") as archive:
            info = tarfile.TarInfo("bundle/editor")
            info.size, info.mode = 3, 0o755
            archive.addfile(info, io.BytesIO(b"exe"))
        output = self.root / "unpacked"
        downloads.extract(path, output, "tar.gz", limit=3)
        self.assertEqual((output / "bundle/editor").read_bytes(), b"exe")
        if os.name != "nt":
            self.assertTrue((output / "bundle/editor").stat().st_mode & 0o111)

    def test_catalog_filters_hosts_and_rejects_duplicates(self):
        entry = {"version": "0.1.0", "target": hub.host_target(), "url": "https://example.org/editor.zip",
                 "sha256": "0" * 64, "size": 100, "unpacked_size": 200, "format": "zip"}
        data = {"schema": 1, "releases": [entry, {**entry, "target": "another-host"}]}
        with patch.object(downloads, "read_remote", return_value=json.dumps(data).encode()):
            self.assertEqual(downloads.catalog("https://example.org/catalog.json", hub.host_target()), [entry])
        data["releases"].append(entry)
        with patch.object(downloads, "read_remote", return_value=json.dumps(data).encode()):
            with self.assertRaisesRegex(ValueError, "duplicate"):
                downloads.catalog("https://example.org/catalog.json", hub.host_target())

    def test_rust_targets_use_exact_release_and_checksum(self):
        artifacts, tables = {}, []
        for target in hub.ANDROID_TARGETS:
            output = io.BytesIO()
            with tarfile.open(fileobj=output, mode="w:xz") as archive:
                info = tarfile.TarInfo(f"rust-std-1.98.1-{target}/rust-std-{target}/lib/rustlib/{target}/lib/libstd.rlib")
                info.size = 3
                archive.addfile(info, io.BytesIO(b"std"))
            url = f"https://static.rust-lang.org/dist/rust-std-1.98.1-{target}.tar.xz"
            artifacts[url] = output.getvalue()
            tables.append(f'[pkg.rust-std.target.{target}]\navailable=true\nxz_url="{url}"\nxz_hash="{hashlib.sha256(artifacts[url]).hexdigest()}"\n')
        manifest = ('manifest-version="2"\n[pkg.rust-std]\nversion="1.98.1 (fixture)"\n' + "".join(tables)).encode()
        url = "https://static.rust-lang.org/dist/channel-rust-1.98.1.toml"
        artifacts[url] = manifest
        artifacts[url + ".sha256"] = hashlib.sha256(manifest).hexdigest().encode()
        rust = self.root / "rust"
        with patch.object(downloads, "response", side_effect=lambda address: io.BytesIO(artifacts[address])):
            downloads.install_rust_targets(rust, "1.98.1", hub.ANDROID_TARGETS, self.root)
        for target in hub.ANDROID_TARGETS:
            self.assertEqual((rust / "lib/rustlib" / target / "lib/libstd.rlib").read_bytes(), b"std")
        with patch.object(downloads, "read_remote") as remote:
            downloads.install_rust_targets(rust, "1.98.1", hub.ANDROID_TARGETS, self.root)
            remote.assert_not_called()


if __name__ == "__main__":
    unittest.main()
