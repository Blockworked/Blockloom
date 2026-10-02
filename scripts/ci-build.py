#!/usr/bin/env python3
"""Build one CI component and archive its outputs for packaging."""

import argparse
import os
import subprocess
import sys
import tarfile
from pathlib import Path

import replace


def archive_outputs(component):
    directory = Path("target/release")
    suffix = ".exe" if os.name == "nt" else ""
    if component == "editor":
        paths = [directory / (name + suffix) for name in
                 ("blockloom", "blockloom-hub", "blockloom-runtime", "blockloom-shell",
                  "blockloom-plugin-worker")]
        paths += [path for path in directory.iterdir()
                  if path.suffix in (".dll", ".dylib") or ".so" in path.name]
    else:
        target = replace.host_target() if component == "native" else "wasm32-unknown-unknown"
        paths = [directory / "players" / target]
        if not paths[0].is_dir() or not any(paths[0].iterdir()):
            raise ValueError(f"Missing staged player: {paths[0]}")
    # Tar keeps executable modes when Actions transfers files between runners.
    with tarfile.open("build-output.tar", "w") as archive:
        for path in paths:
            archive.add(path, arcname=path.as_posix())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--component", choices=("editor", "native", "web"), required=True)
    parser.add_argument("--shipping", action="store_true")
    args = parser.parse_args()
    profile = "dist" if args.shipping else "release"
    os.environ.setdefault("CARGO_BUILD_JOBS", str(os.cpu_count() or 1))
    os.environ.pop("BLOCKLOOM_INSTALL_DIR", None)
    subprocess.run([replace.just_exe(), "prepare-patched-deps"], check=True)
    target = replace.host_target()
    if args.component == "web":
        target = "wasm32-unknown-unknown"
        subprocess.run(["rustup", "target", "add", target], check=True)
        subprocess.run([replace.just_exe(), "web-tools"], check=True)
    subprocess.run(["cargo", "fetch", "--locked", "--target", target], check=True)
    os.environ["CARGO_NET_OFFLINE"] = "true"
    if args.component == "editor":
        command = [replace.just_exe(), "build"]
    elif args.component == "web":
        command = [replace.just_exe(), "web-player", profile]
    elif args.shipping:
        command = [replace.just_exe(), "player"]
    else:
        subprocess.run(["cargo", "build", "--locked", "--release", "-p", "blockloom-runtime"], check=True)
        command = [replace.just_exe(), "_stage-release-player"]
    subprocess.run(command, check=True)
    subprocess.run([sys.executable, str(Path(__file__).with_name("prune-target.py"))], check=True)
    archive_outputs(args.component)
    return 0


if __name__ == "__main__":
    sys.exit(main())
