#!/usr/bin/env python3
"""Blockloom Hub commands from a checkout, plus the helpers packaging uses.

The Hub itself is Rust: `blockloom-hub --hubctl <command>` (or the
`blockloom-hubctl` binary) installs, downloads and builds editors and manages
projects. This script only forwards to it.
"""

import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile

from hub_install import MANIFEST
from replace import host_target


ANDROID_TARGETS = ("aarch64-linux-android", "x86_64-linux-android")
OPTIONAL_TOOLS = ("java", "android-sdk", "android-ndk")


def tool_path(directory, name):
    return directory / "tools" / name


def validate_tools(directory, tools):
    """The same checks the Hub makes of an installation's bundled tools."""
    if not isinstance(tools, dict):
        raise ValueError("Installation tools must be an object")
    rust = tools.get("rust", {})
    if not isinstance(rust, dict) or not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", str(rust.get("channel", ""))):
        raise ValueError("Every installation must bundle its pinned Rust toolchain")
    suffix = ".exe" if os.name == "nt" else ""
    for binary in ("rustc", "cargo"):
        if not (tool_path(directory, "rust") / "bin" / (binary + suffix)).is_file():
            raise ValueError(f"Bundled Rust toolchain is missing {binary}")
    if not (tool_path(directory, "rust") / "lib" / "rustlib" / host_target() / "lib").is_dir():
        raise ValueError("Bundled Rust toolchain is missing host standard libraries")
    for target in tools.get("android-rust-targets", []):
        if target not in ANDROID_TARGETS or not (tool_path(directory, "rust") / "lib" / "rustlib" / target / "lib").is_dir():
            raise ValueError(f"Bundle is missing Android Rust target: {target}")


def read_json(path):
    result = json.loads(Path(path).read_text(encoding="utf-8"))
    if not isinstance(result, dict):
        raise ValueError(f"Expected a JSON object: {path}")
    return result


def write_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary = tempfile.mkstemp(prefix=".hub-", dir=path.parent)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8") as stream:
            json.dump(value, stream, indent=2)
            stream.write("\n")
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    finally:
        Path(temporary).unlink(missing_ok=True)


def hubctl_command():
    """BLOCKLOOM_HUBCTL if set, else the checkout's built `blockloom-hubctl`."""
    if explicit := os.environ.get("BLOCKLOOM_HUBCTL"):
        return [explicit, *os.environ.get("BLOCKLOOM_HUBCTL_ARGS", "").split()]
    suffix = ".exe" if os.name == "nt" else ""
    target = Path(__file__).resolve().parent.parent / "target"
    for profile in ("release", "debug"):
        candidate = target / profile / ("blockloom-hubctl" + suffix)
        if candidate.is_file():
            return [str(candidate)]
    raise ValueError("The Hub tool is missing. Build it with: cargo build --release -p blockloom-hub-state")


def main(argv=None):
    argv = list(sys.argv[1:] if argv is None else argv)
    try:
        return subprocess.run([*hubctl_command(), *argv]).returncode
    except ValueError as error:
        print(f"hub: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
