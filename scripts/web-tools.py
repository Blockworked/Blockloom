#!/usr/bin/env python3
"""Install the wasm-bindgen CLI matching Cargo.lock's wasm-bindgen.

The glue generator has to match the locked version exactly, so the version
is read off `cargo metadata` rather than pinned here.
"""

import json
import subprocess
import sys


def main():
    metadata = subprocess.run(
        ["cargo", "metadata", "--format-version", "1",
         "--filter-platform", "wasm32-unknown-unknown"],
        capture_output=True,
        text=True,
        encoding="utf-8",
        check=True,
    )
    version = next(
        package["version"]
        for package in json.loads(metadata.stdout)["packages"]
        if package["name"] == "wasm-bindgen"
    )
    return subprocess.run(
        ["cargo", "install", "wasm-bindgen-cli", "--locked", "--version", version]
    ).returncode


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, subprocess.CalledProcessError) as error:
        sys.exit(f"web-tools: {error}")
    except StopIteration:
        sys.exit("web-tools: no wasm-bindgen in Cargo.lock")
