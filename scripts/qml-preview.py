#!/usr/bin/env python3
"""Build Blockloom and preview its embedded QML with the selected Qt kit."""

import argparse
import json
import os
from pathlib import Path
import shutil
import shlex
import subprocess
import sys


def qt_tool():
    qmake = os.environ.get("QMAKE") or shutil.which("qmake6") or shutil.which("qmake")
    if not qmake:
        raise RuntimeError("Qt 6.12 is required. Set QMAKE to your Qt kit's qmake.")

    def query(key):
        return subprocess.check_output([qmake, "-query", key], text=True).strip()

    version = query("QT_VERSION")
    if tuple(int(part) for part in version.split(".")[:2]) < (6, 12):
        raise RuntimeError(f"QMAKE selects Qt {version}; Qt 6.12 or newer is required.")
    suffix = ".exe" if os.name == "nt" else ""
    preview = Path(query("QT_HOST_BINS")) / f"qmlpreview{suffix}"
    if not preview.is_file():
        raise RuntimeError(f"Missing {preview}. Install QML tooling for this Qt kit.")
    help_text = subprocess.check_output([str(preview), "--help"], text=True)
    if "--resource" not in help_text or "--interactive" not in help_text:
        raise RuntimeError(f"{preview} lacks Qt 6.12 preview options; update this kit's tooling.")
    print(f"Using Qt {version}: {preview}", flush=True)
    return qmake, preview


def build(profile, qmake):
    command = [
        "cargo", "build", "--workspace", "--profile", profile,
        "--features", "blockloom/qml-preview", "--message-format=json-render-diagnostics",
    ]
    resources = set()
    executable = None
    # Use Cargo's active outputs, rather than stale manifests from other builds.
    with subprocess.Popen(
        command, stdout=subprocess.PIPE, text=True, env={**os.environ, "QMAKE": qmake}
    ) as process:
        for line in process.stdout:
            try:
                message = json.loads(line)
            except json.JSONDecodeError:
                print(line, end="")
                continue
            reason = message.get("reason")
            if reason == "compiler-message":
                rendered = message["message"].get("rendered")
                if rendered:
                    print(rendered, end="", file=sys.stderr)
            elif reason == "build-script-executed":
                out = Path(message["out_dir"])
                resources.update(out.glob("qt-build-utils/qml_modules/**/*.qrc"))
            elif reason == "compiler-artifact":
                if message["target"]["name"] == "blockloom" and message.get("executable"):
                    executable = Path(message["executable"])
        if process.wait():
            raise RuntimeError("Workspace build failed.")
    if not executable or not executable.is_file():
        raise RuntimeError("Cargo did not produce the Blockloom editor.")
    for module in ("Blockloom", "Blockstitch"):
        if not any(f"/com/blockworked/{module}/" in path.as_posix() for path in resources):
            raise RuntimeError(f"Missing active {module} QML resource manifest.")
    return executable, sorted(resources)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--profile", choices=("dev", "release"), default="release")
    parser.add_argument("--build-only", action="store_true", help="Build and print the launch command")
    parser.add_argument("--verbose", action="store_true")
    parser.add_argument("--output", help="Save input events to a .qtd file")
    parser.add_argument("--replay", help="Replay input events from a .qtd file")
    parser.add_argument("app_args", nargs=argparse.REMAINDER, help="Application arguments after --")
    args = parser.parse_args()
    os.chdir(Path(__file__).resolve().parent.parent)
    qmake, preview = qt_tool()
    if sys.platform == "linux":
        subprocess.run([sys.executable, "scripts/prune-target.py"], check=True)
    try:
        executable, resources = build(args.profile, qmake)
    finally:
        if sys.platform == "linux":
            profile = "debug" if args.profile == "dev" else args.profile
            subprocess.run([sys.executable, "scripts/prune-target.py", "--keep-profile", profile], check=True)
    command = [str(preview), "--interactive"]
    for resource in resources:
        command.extend(["--resource", str(resource)])
    if args.verbose:
        command.append("--verbose")
    for option in ("output", "replay"):
        if getattr(args, option):
            command.extend([f"--{option}", getattr(args, option)])
    app_args = args.app_args
    if app_args[:1] == ["--"]:
        app_args = app_args[1:]
    command.extend([str(executable), *app_args])
    print("Preview command:", flush=True)
    print(subprocess.list2cmdline(command) if os.name == "nt" else shlex.join(command), flush=True)
    if args.build_only:
        return 0
    return subprocess.call(command)


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, RuntimeError, subprocess.CalledProcessError) as error:
        sys.exit(f"qml-preview: {error}")
    except KeyboardInterrupt:
        sys.exit(130)
