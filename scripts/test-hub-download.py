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
import xml.etree.ElementTree as ET

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

    def test_android_repository_sha1_is_verified(self):
        value = b"official Android package"
        path = self.root / "android.zip"
        with patch.object(downloads, "response", return_value=io.BytesIO(value)):
            downloads.download("https://dl.google.com/android/repository/tools.zip", path,
                               hashlib.sha1(value).hexdigest(), len(value), algorithm="sha1")
        self.assertEqual(path.read_bytes(), value)
        path.unlink()
        with patch.object(downloads, "response", return_value=io.BytesIO(value)), self.assertRaises(ValueError):
            downloads.download("https://dl.google.com/android/repository/tools.zip", path, "0" * 40, algorithm="sha1")
        self.assertFalse(path.exists())

    def test_tool_file_links_are_materialized_and_bounded(self):
        info = zipfile.ZipInfo("tools/bin/alias")
        info.create_system = 3
        info.external_attr = (stat.S_IFLNK | 0o777) << 16
        archive = self.zip([(info, b"compiler"), ("tools/bin/compiler", b"exe")])
        destination = self.root / "tool-links"
        downloads.extract(archive, destination, "zip", allow_file_links=True)
        self.assertEqual((destination / "tools/bin/alias").read_bytes(), b"exe")
        self.assertFalse((destination / "tools/bin/alias").is_symlink())
        with self.assertRaisesRegex(ValueError, "limits"):
            downloads.extract(archive, self.root / "tool-budget", "zip", limit=12, allow_file_links=True)
        archive = self.zip([(info, b"../../../outside")])
        with self.assertRaisesRegex(ValueError, "escapes"):
            downloads.extract(archive, self.root / "tool-escape", "zip", allow_file_links=True)
        archive = self.zip([(info, b"alias")])
        with self.assertRaisesRegex(ValueError, "unresolved"):
            downloads.extract(archive, self.root / "tool-cycle", "zip", allow_file_links=True)

    def test_tool_tar_hardlinks_and_symlinks_are_regular_files(self):
        path = self.root / "jdk.tar.gz"
        with tarfile.open(path, "w:gz") as archive:
            for name, kind, target in (("jdk/bin/hardlink", tarfile.LNKTYPE, "jdk/bin/java"),
                                       ("jdk/bin/symlink", tarfile.SYMTYPE, "java")):
                info = tarfile.TarInfo(name)
                info.type, info.linkname = kind, target
                archive.addfile(info)
            info = tarfile.TarInfo("jdk/bin/java")
            info.size, info.mode = 3, 0o755
            archive.addfile(info, io.BytesIO(b"jdk"))
        output = self.root / "jdk-out"
        downloads.extract(path, output, "tar.gz", allow_file_links=True)
        for name in ("hardlink", "symlink"):
            self.assertEqual((output / "jdk/bin" / name).read_bytes(), b"jdk")

    def test_downloaded_android_tools_follow_pins_on_each_host(self):
        versions = {"JDK_MAJOR": "25", "PLATFORM": "android-35", "BUILD_TOOLS": "35.0.0", "NDK_MAJOR": "27"}

        def zipped(entries):
            data = io.BytesIO()
            with zipfile.ZipFile(data, "w") as archive:
                for name, value in entries.items():
                    archive.writestr(name, value)
            return data.getvalue()

        for host, python_os, machine in (("windows", "win32", "AMD64"), ("linux", "linux", "x86_64"), ("macosx", "darwin", "arm64")):
            with self.subTest(host=host):
                archives, packages = {}, []
                java_url = "https://github.com/adoptium/temurin25-binaries/releases/download/jdk/jdk.zip"
                home = "jdk/Contents/Home" if host == "macosx" else "jdk"
                java = zipped({home + "/bin/java": b"jdk", home + "/release": b'JAVA_VERSION="25.0.1"'})
                archives[java_url] = java
                assets = [{"binary": {"package": {"link": java_url, "checksum": hashlib.sha256(java).hexdigest(), "size": len(java)}},
                           "version": {"semver": "25.0.1+1"}}]
                for key, root, entries, major, minor in (
                        ("cmdline-tools;latest", "cmdline-tools", {"bin/sdkmanager": b"sdk"}, 19, 0),
                        ("platform-tools", "platform-tools", {"adb": b"adb"}, 36, 0),
                        ("platforms;android-35", "android-35", {"android.jar": b"jar"}, 2, 0),
                        ("build-tools;35.0.0", "android-15", {"aapt2": b"aapt", "lib/apksigner.jar": b"jar"}, 35, 0),
                        ("emulator", "emulator", {"emulator.exe" if host == "windows" else "emulator": b"emulator"}, 36, 0),
                        ("system-images;android-35;google_apis;x86_64", "x86_64", {"system.img": b"image"}, 9, 0),
                        ("ndk;27.1.1", "ndk-old", {"source.properties": b"old"}, 27, 1),
                        ("ndk;27.2.1", "ndk-pinned", {"source.properties": b"Pkg.Revision=27.2.1"}, 27, 2),
                        ("ndk;28.0.1", "ndk-new", {"source.properties": b"new"}, 28, 0)):
                    label = key.replace(";", "-")
                    value = zipped({root + "/" + name: data for name, data in entries.items()})
                    archive_base = "https://dl.google.com/android/repository/sys-img/google_apis/" if key.startswith("system-images;") else "https://dl.google.com/android/repository/"
                    archives[archive_base + label + ".zip"] = value
                    architecture = "<host-arch>aarch64</host-arch>" if host == "macosx" else ""
                    packages.append(f'''<remotePackage path="{key}"><type-details xsi:type="generic:genericDetailsType"/><revision><major>{major}</major><minor>{minor}</minor><micro>1</micro></revision>
                        <channelRef ref="channel-0"/><archives><archive><host-os>{host}</host-os>{architecture}<complete>
                        <url>{label}.zip</url><size>{len(value)}</size><checksum type="sha1">{hashlib.sha1(value).hexdigest()}</checksum>
                        </complete></archive></archives></remotePackage>''')
                repository = ('<ns0:sdk-repository xmlns:ns0="urn:sdk" xmlns:ns1="urn:common" xmlns:generic="urn:generic" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">'
                              + ''.join(packages) + '</ns0:sdk-repository>').encode()

                def metadata(url, *args):
                    if url.startswith("https://api.adoptium.net/"):
                        self.assertIn("/25/", url)
                        self.assertIn("architecture=" + ("aarch64" if host == "macosx" else "x64"), url)
                        return json.dumps(assets).encode()
                    return repository

                destination = self.root / host
                temporary = self.root / (host + "-temporary")
                temporary.mkdir()
                with patch.object(downloads, "read_remote", side_effect=metadata), \
                        patch.object(downloads, "response", side_effect=lambda url: io.BytesIO(archives[url])), \
                        patch.object(downloads.sys, "platform", python_os), \
                        patch.object(downloads.platform, "machine", return_value=machine):
                    tools = downloads.install_android_tools(destination, versions, hub.OPTIONAL_TOOLS, temporary)
                self.assertEqual(tools["android-ndk"]["version"], "27.2.1")
                self.assertEqual(tools["java"]["version"], "25.0.1+1")
                self.assertEqual((destination / "tools/java/bin/java").read_bytes(), b"jdk")
                self.assertTrue((destination / "tools/android-sdk/platforms/android-35/android.jar").is_file())
                self.assertTrue((destination / "tools/android-sdk/build-tools/35.0.0/lib/apksigner.jar").is_file())
                self.assertTrue((destination / "tools/android-sdk/emulator" / ("emulator.exe" if host == "windows" else "emulator")).is_file())
                self.assertEqual((destination / "tools/android-sdk/system-images/android-35/google_apis/x86_64/system.img").read_bytes(), b"image")
                self.assertIn("emulator", tools["android-sdk"]["packages"])
                self.assertIn("system-images;android-35;google_apis;x86_64", tools["android-sdk"]["packages"])
                self.assertTrue((destination / "tools/android-sdk/cmdline-tools/latest/package.xml").is_file())
                for package in destination.rglob("package.xml"):
                    ET.parse(package)
                self.assertEqual((destination / "tools/android-ndk/source.properties").read_bytes(), b"Pkg.Revision=27.2.1")

    def test_java_version_metadata_is_checked_before_downloading(self):
        with patch.object(downloads, "read_remote", return_value=b'[{"binary": {}}]'), \
                patch.object(downloads, "tool_archive") as archive:
            with self.assertRaisesRegex(ValueError, "missing the Java version"):
                downloads.install_android_tools(self.root, {"JDK_MAJOR": "25"}, ("java",), self.root)
        archive.assert_not_called()

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
