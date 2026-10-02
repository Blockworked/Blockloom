#!/usr/bin/env python3
"""Package editor, Hub, players, Qt and pinned Rust for a release catalog."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import tomllib
from urllib.parse import quote
import zipfile

import hub
import hub_download
import hub_install
from hub_process import bundle_rust


def copy_tree(source, destination):
    shutil.copytree(source, destination, dirs_exist_ok=True,
                    ignore=shutil.ignore_patterns("*.a", "*.lib", "*.pdb", "*.prl", "*.debug"))


def deploy_qt(directory, qmake):
    def query(name):
        return Path(subprocess.check_output([qmake, "-query", name], text=True).strip())
    copy_tree(query("QT_INSTALL_PLUGINS"), directory / "plugins")
    copy_tree(query("QT_INSTALL_QML"), directory / "qml")
    libraries = query("QT_INSTALL_BINS" if os.name == "nt" else "QT_INSTALL_LIBS")
    if os.name == "nt":
        for file in libraries.glob("*.dll"):
            shutil.copy2(file, directory / file.name)
        subprocess.run([str(query("QT_INSTALL_BINS") / "windeployqt.exe"), "--release",
                        "--no-translations", str(directory / ("blockloom.exe" if (directory / "blockloom.exe").exists() else "blockloom-hub.exe"))], check=True)
    else:
        output = directory / "lib"
        output.mkdir()
        for file in libraries.iterdir():
            if file.name.endswith(".framework"):
                copy_tree(file, output / file.name)
            elif ".so" in file.name or file.suffix == ".dylib":
                shutil.copy2(file, output / file.name)
        for file in directory.rglob("*"):
            if not file.is_file():
                continue
            with file.open("rb") as stream:
                magic = stream.read(4)
            relative = os.path.relpath(output, file.parent).replace(os.sep, "/")
            if sys.platform == "linux" and magic == b"\x7fELF":
                subprocess.run(["patchelf", "--set-rpath", f"$ORIGIN:$ORIGIN/{relative}", str(file)], check=True)
            elif sys.platform == "darwin" and magic in (b"\xcf\xfa\xed\xfe", b"\xfe\xed\xfa\xcf", b"\xca\xfe\xba\xbe"):
                dependencies = subprocess.check_output(["otool", "-L", str(file)], text=True).splitlines()[1:]
                for line in dependencies:
                    name = line.strip().split(" (", 1)[0]
                    if name.startswith(str(libraries) + "/"):
                        new = "@rpath/" + name[len(str(libraries)) + 1:]
                        subprocess.run(["install_name_tool", "-change", name, new, str(file)], check=True)
                subprocess.run(["install_name_tool", "-add_rpath", f"@loader_path/{relative}", str(file)], check=True)
                subprocess.run(["codesign", "--force", "--sign", "-", str(file)], check=True)
    (directory / "qt.conf").write_text("[Paths]\nPrefix=.\nLibraries=lib\nPlugins=plugins\nQmlImports=qml\n", encoding="utf-8")


def archive(directory, destination):
    size = 0
    with zipfile.ZipFile(destination, "x", compression=zipfile.ZIP_DEFLATED, compresslevel=6) as bundle:
        for file in sorted(directory.rglob("*")):
            if file.is_file():
                size += file.stat().st_size
                bundle.write(file, file.relative_to(directory).as_posix())
    checksum = hashlib.sha256()
    if destination.stat().st_size >= 2 * 1024**3:
        raise ValueError("Release asset exceeds GitHub's 2 GiB per-file limit")
    with destination.open("rb") as stream:
        while chunk := stream.read(1024 * 1024):
            checksum.update(chunk)
    return {"size": destination.stat().st_size, "unpacked_size": size, "sha256": checksum.hexdigest(), "format": "zip"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=Path("artifacts"))
    parser.add_argument("--tag")
    parser.add_argument("--repo", default="Blockworked/Blockloom")
    parser.add_argument("--qmake", default=os.environ.get("QMAKE") or shutil.which("qmake6") or "qmake")
    args = parser.parse_args()
    version = tomllib.loads(Path("Cargo.toml").read_text())["workspace"]["package"]["version"]
    hub_download.version(version)
    tag = args.tag or "v" + version
    if tag != "v" + version:
        raise ValueError("Release tag must match the editor package version")
    args.output.mkdir(parents=True, exist_ok=True)
    target = hub.host_target()
    with tempfile.TemporaryDirectory(prefix="blockloom-package-") as temporary:
        directory = Path(temporary) / "editor"
        hub_install.stage(Path("target/release"), directory, build_output=True)
        rust = Path(subprocess.check_output(["rustc", "--print", "sysroot"], text=True).strip())
        channel = tomllib.loads(Path("rust-toolchain.toml").read_text())["toolchain"]["channel"]
        bundle_rust(rust, directory / "tools/rust", channel)
        manifest = {"kind": "release", "version": version, "target": target,
                    "tools": {"rust": {"channel": channel}, "android-rust-targets": list(hub.ANDROID_TARGETS)}}
        hub.validate_tools(directory, manifest["tools"])
        hub.write_json(directory / hub.MANIFEST, manifest)
        deploy_qt(directory, args.qmake)
        name = f"blockloom-{version}-{target}.zip"
        entry = {**archive(directory, args.output / name), **manifest, "asset": name,
                 "url": f"https://github.com/{args.repo}/releases/download/{quote(tag, safe='')}/{name}"}
        hub.write_json(args.output / f"catalog-{target}.json", {"schema": 1, "releases": [entry]})
        launcher = Path(temporary) / "hub"
        launcher.mkdir()
        suffix = ".exe" if os.name == "nt" else ""
        shutil.copy2(Path("target/release") / ("blockloom-hub" + suffix), launcher)
        deploy_qt(launcher, args.qmake)
        # Hub's Python service is embedded; Python 3.11+ must be on PATH.
        (launcher / "README.txt").write_text("Blockloom Hub requires Python 3.11+ on PATH.\nPrivate releases use gh auth login.\n")
        archive(launcher, args.output / f"blockloom-hub-{version}-{target}.zip")


if __name__ == "__main__":
    main()
