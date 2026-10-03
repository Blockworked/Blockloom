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
                    ignore=shutil.ignore_patterns("*.a", "*.lib", "*.o", "*.obj", "*.pdb", "*.prl", "*.debug"))


def elf_type(path):
    """Return the ELF e_type of a file, or None if it has no ELF header."""
    with open(path, "rb") as stream:
        header = stream.read(20)
    if len(header) < 20 or header[:4] != b"\x7fELF":
        return None
    if header[5] == 1:
        order = "little"
    elif header[5] == 2:
        order = "big"
    else:
        return None
    return int.from_bytes(header[16:18], order)


def macho_filetypes(path):
    """Return the Mach-O filetype of each slice, or [] if not a Mach-O binary."""
    import struct
    with open(path, "rb") as stream:
        magic = stream.read(4)
        if magic in (b"\xca\xfe\xba\xbe", b"\xbe\xba\xfe\xca"):
            count = stream.read(4)
            if len(count) < 4:
                return []
            arches = []
            for _ in range(struct.unpack(">I", count)[0]):
                entry = stream.read(20)
                if len(entry) < 20:
                    break
                _, _, offset, _, _ = struct.unpack(">5I", entry)
                arches.append(offset)
            types = []
            for offset in arches:
                stream.seek(offset)
                header = stream.read(16)
                if len(header) < 16:
                    continue
                kind = header[:4]
                rest = header[12:16]
                if kind in (b"\xcf\xfa\xed\xfe", b"\xce\xfa\xed\xfe"):
                    types.append(struct.unpack("<I", rest)[0])
                elif kind in (b"\xfe\xed\xfa\xcf", b"\xfe\xed\xfa\xce"):
                    types.append(struct.unpack(">I", rest)[0])
            return types
        if magic in (b"\xcf\xfa\xed\xfe", b"\xce\xfa\xed\xfe"):
            rest = stream.read(12)
            return [struct.unpack("<I", rest[8:12])[0]] if len(rest) == 12 else []
        if magic in (b"\xfe\xed\xfa\xcf", b"\xfe\xed\xfa\xce"):
            rest = stream.read(12)
            return [struct.unpack(">I", rest[8:12])[0]] if len(rest) == 12 else []
    return []


def macho_needs_rpath(path):
    """Check whether a Mach-O file is linked output, not a relocatable object."""
    types = macho_filetypes(path)
    return any(kind in (2, 6, 8) for kind in types)


def existing_rpaths(path):
    """List the LC_RPATH entries already present in a Mach-O binary."""
    output = subprocess.check_output(["otool", "-l", str(path)], text=True)
    found = []
    lines = output.splitlines()
    for index, line in enumerate(lines):
        if "cmd LC_RPATH" in line:
            for following in lines[index + 1:index + 5]:
                text = following.strip()
                if text.startswith("path "):
                    found.append(text.split()[1])
                    break
    return found


def framework_root(path, directory):
    """Return the outermost .framework bundle containing path, or None.

    Codesign refuses to sign a framework's inner binary on its own (the
    bundle format is ambiguous), so callers sign the bundle directory
    instead once its binaries are patched.
    """
    root = None
    for parent in Path(path).parents:
        try:
            parent.relative_to(directory)
        except ValueError:
            break
        if parent.suffix == ".framework":
            root = parent
    return root


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
        frameworks = set()
        for file in directory.rglob("*"):
            if not file.is_file() or file.is_symlink():
                continue
            # The bundled Rust toolchain is self-contained and its deep
            # binaries lack room for longer load commands, so leave it alone.
            try:
                top = file.relative_to(directory).parts[0]
            except (ValueError, IndexError):
                top = ""
            if top == "tools":
                continue
            with file.open("rb") as stream:
                magic = stream.read(4)
            relative = os.path.relpath(output, file.parent).replace(os.sep, "/")
            if sys.platform == "linux" and magic == b"\x7fELF":
                # Qt ships intermediate relocatable objects next to its QML
                # files; patchelf only handles executables and shared objects.
                if elf_type(file) not in (2, 3):
                    continue
                subprocess.run(["patchelf", "--set-rpath", f"$ORIGIN:$ORIGIN/{relative}", str(file)], check=True)
            elif sys.platform == "darwin" and magic in (b"\xcf\xfa\xed\xfe", b"\xfe\xed\xfa\xcf", b"\xca\xfe\xba\xbe"):
                if not macho_needs_rpath(file):
                    continue
                dependencies = subprocess.check_output(["otool", "-L", str(file)], text=True).splitlines()[1:]
                for line in dependencies:
                    name = line.strip().split(" (", 1)[0]
                    if name.startswith(str(libraries) + "/"):
                        new = "@rpath/" + name[len(str(libraries)) + 1:]
                        subprocess.run(["install_name_tool", "-change", name, new, str(file)], check=True)
                wanted = f"@loader_path/{relative}"
                if wanted not in existing_rpaths(file):
                    subprocess.run(["install_name_tool", "-add_rpath", wanted, str(file)], check=True)
                root = framework_root(file, directory)
                if root is not None:
                    # Signing the inner binary directly fails with
                    # "bundle format is ambiguous"; the bundle is signed below.
                    frameworks.add(root)
                    continue
                subprocess.run(["codesign", "--force", "--sign", "-", str(file)], check=True)
        for framework in sorted(frameworks):
            subprocess.run(["codesign", "--force", "--sign", "-", str(framework)], check=True)
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
    tag = args.tag or version
    if tag != version:
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
