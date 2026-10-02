#!/usr/bin/env python3
"""Run the compiled Qt Hub against isolated data and capture its main views."""

import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import zipfile

sys.dont_write_bytecode = True
import hub


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path("target/release") / (
        "blockloom-hub.exe" if os.name == "nt" else "blockloom-hub"))
    parser.add_argument("--screenshots", type=Path, default=Path("target/hub-ui-smoke"))
    args = parser.parse_args()
    screenshots = args.screenshots.resolve()
    screenshots.mkdir(parents=True, exist_ok=True)
    binary = args.binary.resolve(strict=True)
    with tempfile.TemporaryDirectory(prefix="hub-ui-") as temporary:
        base = Path(temporary).resolve()
        project = base / "A project with spaces"
        hub.write_json(project / "project.blockloom", {
            "name": "My first game", "world": {"mode": "TwoD"}})
        root = base / "Hub"
        hub.write_json(root / "projects.json", {"projects": [{"path": str(project)}]})
        identity = "dev-0123456789abcdef"
        hub.write_json(root / "installations" / identity / hub.MANIFEST, {
            "id": identity, "kind": "dev", "name": "blockloom",
            "version": "0.0.1", "target": hub.host_target(), "status": "unbuilt",
            "repo": str(base / "Local repository")})
        # A ready release exercises the version chooser without running fake binaries.
        hub.write_json(root / "installations" / "release-0.1.0" / hub.MANIFEST, {
            "id": "release-0.1.0", "kind": "release", "version": "0.1.0",
            "target": hub.host_target(), "status": "ready",
            "tools": {"rust": {"channel": "1.98.1"}}})
        release = root / "installations" / "release-0.1.0"
        suffix = ".exe" if os.name == "nt" else ""
        for name in ("blockloom", "blockloom-runtime", "tools/rust/bin/rustc", "tools/rust/bin/cargo"):
            path = release / (name + suffix)
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("test fixture", encoding="utf-8")
        (release / "tools/rust/lib/rustlib" / hub.host_target() / "lib").mkdir(parents=True)
        import hub_install
        dev = hub.read_json(root / "installations" / identity / hub.MANIFEST)
        hub_install.stage(release, root / "pending" / identity, {
            **dev, "status": "ready", "tools": {"rust": {"channel": "1.98.1"}},
            "sources": {}, "android_targets_available": False})
        env = {**os.environ, "BLOCKLOOM_HUB_DIR": str(root),
               "BLOCKLOOM_DATA_DIR": str(base / "Editor data"),
               "BLOCKLOOM_HUB_PYTHON": sys.executable, "QT_QPA_PLATFORM": "offscreen",
               "QT_QUICK_BACKEND": "software"}
        if os.name == "nt":
            env["BLOCKLOOM_HUB_SMOKE_FONT"] = str(Path(env["WINDIR"]) / "Fonts" / "segoeui.ttf")
            env["BLOCKLOOM_HUB_SMOKE_MONO"] = str(Path(env["WINDIR"]) / "Fonts" / "consola.ttf")
        if qmake := env.get("QMAKE"):
            kit = Path(qmake).parent.parent
            env["PATH"] = str(kit / "bin") + os.pathsep + env.get("PATH", "")
            env["QT_PLUGIN_PATH"] = str(kit / "plugins")
            env["QML_IMPORT_PATH"] = str(kit / "qml")
        for page in ("projects", "projects-scroll", "installations", "install", "version", "bind", "log", "log-scroll", "releases", "upgrade", "upgrade-bind", "dev-options", "checkbox-hover", "checkbox-checked", "dev-install"):
            projects = [{"path": str(project)}]
            if page == "projects-scroll":
                for index in range(12):
                    extra = base / ("Example project " + str(index + 1))
                    hub.write_json(extra / "project.blockloom", {"name": extra.name, "world": {"mode": "TwoD"}})
                    hub.write_json(extra / ".blockloom/hub.json", {"schema": 1, "installation": "release-0.1.0"})
                    projects.append({"path": str(extra)})
            hub.write_json(root / "projects.json", {"projects": projects})
            if page in ("upgrade", "upgrade-bind"):
                hub.write_json(project / ".blockloom/hub.json", {"schema": 1, "installation": "release-0.0.1"})
            result = subprocess.run([str(binary), "--smoke-test", "--smoke-page", page,
                                     "--smoke-output", str(screenshots / (page + ".png"))],
                                    env=env, capture_output=True, text=True, encoding="utf-8", timeout=30)
            if result.returncode or "Error" in result.stderr or "ReferenceError" in result.stderr or "TypeError" in result.stderr:
                raise RuntimeError(f"Qt smoke test failed for {page} ({result.returncode}):\n{result.stdout}\n{result.stderr}")
            if json.loads(result.stdout) != {"projects": len(projects), "installations": 2}:
                raise RuntimeError("The Qt window did not load the fixture data")
            print(f"PASS: {page}")
        if hub.read_json(project / ".blockloom" / "hub.json")["installation"] != "release-0.1.0":
            raise RuntimeError("The UI did not persist the selected installation")
        backups = list((root / "backups").glob("*/*.zip"))
        if len(backups) != 1:
            raise RuntimeError("The UI did not create the requested upgrade backup")
        with zipfile.ZipFile(backups[0]) as backup:
            if json.loads(backup.read(".blockloom/hub.json"))["installation"] != "release-0.0.1":
                raise RuntimeError("The backup did not retain the previous editor selection")
        installed = hub.read_json(root / "installations" / identity / hub.MANIFEST)
        if installed["status"] != "ready" or "sources" in installed:
            raise RuntimeError("The UI did not install the prepared development editor")
    return 0


if __name__ == "__main__":
    sys.exit(main())
