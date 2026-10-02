#!/usr/bin/env python3
"""Build the workspace and players concurrently without a system install."""

import argparse
import os
import subprocess
import sys
from pathlib import Path

import replace


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--shipping", action="store_true")
    args = parser.parse_args()
    profile = "dist" if args.shipping else "release"
    count = 3 if args.shipping else 2
    os.environ["CARGO_BUILD_JOBS"] = str(max(1, (os.cpu_count() or 1) // count))
    os.environ["BLOCKLOOM_WEB_PROFILE"] = profile
    os.environ.pop("BLOCKLOOM_INSTALL_DIR", None)
    subprocess.run([replace.just_exe(), "prepare-patched-deps"], check=True)
    subprocess.run(["rustup", "target", "add", "wasm32-unknown-unknown"], check=True)
    subprocess.run([replace.just_exe(), "web-tools"], check=True)
    subprocess.run(["cargo", "fetch", "--locked", "--target", replace.host_target(),
                    "--target", "wasm32-unknown-unknown"], check=True)
    os.environ["CARGO_NET_OFFLINE"] = "true"
    if replace.run_builds(profile):
        return 1
    subprocess.run([sys.executable, str(Path(__file__).with_name("prune-target.py"))], check=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
