#!/usr/bin/env python3
"""Blockloom Hub's installation service and initial CLI."""

import argparse
from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import tomllib

from hub_install import MANIFEST, editor_name, stage, validate_payload
from replace import host_target, just_exe


ANDROID_TARGETS = ("aarch64-linux-android", "x86_64-linux-android")
OPTIONAL_TOOLS = ("java", "android-sdk", "android-ndk")


def tool_path(directory, name):
    return directory / "tools" / name


def validate_tools(directory, tools):
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
    for name in OPTIONAL_TOOLS:
        if name in tools:
            metadata = tools[name]
            if not isinstance(metadata, dict) or not metadata.get("version"):
                raise ValueError(f"Component must declare its pinned version: {name}")
            root = tool_path(directory, name)
            if not root.is_dir():
                raise ValueError(f"Bundle is missing selected component: {name}")
            if name == "java" and not (root / "bin" / ("java" + suffix)).is_file():
                raise ValueError("Bundled Java is missing its executable")
            if name == "android-sdk" and not (root / "platform-tools" / ("adb" + suffix)).is_file():
                raise ValueError("Bundled Android SDK is missing platform-tools")
            if name == "android-ndk" and not (root / "source.properties").is_file():
                raise ValueError("Bundled Android NDK is missing its revision metadata")
    for target in tools.get("android-rust-targets", []):
        if target not in ANDROID_TARGETS or not (tool_path(directory, "rust") / "lib" / "rustlib" / target / "lib").is_dir():
            raise ValueError(f"Bundle is missing Android Rust target: {target}")


def hub_root():
    if override := os.environ.get("BLOCKLOOM_HUB_DIR"):
        return Path(override).expanduser().resolve()
    if sys.platform == "win32":
        return Path(os.environ["LOCALAPPDATA"]) / "Blockloom" / "Hub"
    if sys.platform == "darwin":
        return Path.home() / "Library" / "Application Support" / "Blockloom" / "Hub"
    return Path(os.environ.get("XDG_DATA_HOME", Path.home() / ".local" / "share")) / "blockloom-hub"


def project_registry():
    if override := os.environ.get("BLOCKLOOM_DATA_DIR"):
        base = Path(override)
    elif sys.platform == "win32":
        base = Path(os.environ["APPDATA"])
    elif sys.platform == "darwin":
        base = Path.home() / "Library" / "Application Support"
    else:
        base = Path(os.environ.get("XDG_DATA_HOME", Path.home() / ".local" / "share"))
    return base / "blockloom" / "projects.json"


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


class Hub:
    def __init__(self, root=None, registry=None):
        self.root = Path(root or hub_root()).resolve()
        self.registry = Path(registry or project_registry())

    @contextmanager
    def mutation(self):
        self.root.mkdir(parents=True, exist_ok=True)
        lock = self.root / ".operation-lock"
        try:
            lock.mkdir()
        except FileExistsError:
            raise ValueError(f"Another Hub operation is active. If it crashed, remove {lock} after checking no Hub operation is running.") from None
        try:
            yield
        finally:
            lock.rmdir()

    def slot(self, identity):
        if not re.fullmatch(r"(?:release-[0-9A-Za-z.+-]+|dev-[0-9a-f]{16})", identity):
            raise ValueError("Invalid installation ID")
        return self.root / "installations" / identity

    def installation(self, identity, ready=False):
        directory = self.slot(identity)
        if directory.is_symlink():
            raise ValueError("Installation directory cannot be a symlink")
        result = read_json(directory / MANIFEST)
        if result.get("id") != identity or result.get("target") != host_target():
            raise ValueError(f"Invalid installation manifest: {directory}")
        if result.get("kind") not in ("release", "dev"):
            raise ValueError("Unknown installation kind")
        if ready:
            if result.get("status") != "ready":
                raise ValueError("Build this developer installation first")
            validate_payload(directory)
            validate_tools(directory, result.get("tools", {}))
        return result

    def installations(self):
        directory = self.root / "installations"
        if not directory.exists():
            return []
        return [self.installation(p.name) for p in sorted(directory.iterdir())
                if p.is_dir() and not p.name.startswith(".")]

    def project(self, path):
        path = Path(path).expanduser().resolve()
        meta = read_json(path / "project.blockloom")
        scenes = meta.get("scenes", [])
        scene = next((s for s in scenes if s.get("id") == meta.get("active_scene")),
                     scenes[0] if scenes else {})
        mode = scene.get("mode", scene.get("world", {}).get("mode",
                        meta.get("world", {}).get("mode", "TwoD")))
        binding = path / ".blockloom" / "hub.json"
        return {"path": str(path), "name": meta["name"], "mode": mode,
                "installation": read_json(binding).get("installation") if binding.exists() else None}

    def projects(self):
        if not self.registry.exists():
            return []
        entries = []
        for remembered in read_json(self.registry).get("projects", []):
            try:
                entry = self.project(remembered["path"])
            except (OSError, ValueError, KeyError) as error:
                entry = {"path": remembered["path"], "error": str(error)}
            entry["opened_at"] = remembered.get("opened_at", 0)
            entries.append(entry)
        return sorted(entries, key=lambda p: (-p["opened_at"], p.get("name", "").lower()))

    def add_dev(self, repo):
        repo = Path(repo).expanduser().resolve(strict=True)
        cargo = tomllib.loads((repo / "Cargo.toml").read_text(encoding="utf-8"))
        if "blockloom-qt" not in cargo.get("workspace", {}).get("members", []) or not (repo / "justfile").is_file():
            raise ValueError("Select a Blockloom repository containing its workspace and justfile")
        key = os.path.normcase(str(repo))
        identity = "dev-" + hashlib.sha256(key.encode()).hexdigest()[:16]
        with self.mutation():
            directory = self.slot(identity)
            if directory.exists():
                return self.installation(identity)
            result = {"id": identity, "kind": "dev", "repo": str(repo),
                      "name": repo.name, "version": cargo["workspace"]["package"]["version"],
                      "target": host_target(), "status": "unbuilt"}
            write_json(directory / MANIFEST, result)
            return result

    def install(self, bundle, components=(), android_rust_targets=False):
        bundle = Path(bundle).expanduser().resolve(strict=True)
        manifest = read_json(bundle / MANIFEST)
        version = manifest.get("version", "")
        # Require canonical SemVer core numbers and path-safe prerelease names.
        number = r"(?:0|[1-9][0-9]*)"
        if not isinstance(version, str) or not re.fullmatch(rf"{number}\.{number}\.{number}(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?", version):
            raise ValueError("Release version must be canonical SemVer without build metadata")
        if "-" in version:
            for part in version.split("-", 1)[1].split("."):
                if part.isdigit() and len(part) > 1 and part.startswith("0"):
                    raise ValueError("Numeric SemVer prerelease identifiers cannot start with zero")
        if manifest.get("kind") != "release" or manifest.get("target") != host_target():
            raise ValueError("Bundle must be a release for this host target")
        identity = "release-" + version
        available = manifest.get("tools", {})
        if not isinstance(available, dict):
            raise ValueError("Installation tools must be an object")
        selected = {"rust": available.get("rust", {})}
        for name in components:
            if name not in OPTIONAL_TOOLS or name not in available:
                raise ValueError(f"Component unavailable in this bundle: {name}")
            selected[name] = available[name]
        if android_rust_targets:
            targets = available.get("android-rust-targets", [])
            if set(targets) != set(ANDROID_TARGETS):
                raise ValueError("Bundle must include both supported Android Rust targets")
            selected["android-rust-targets"] = list(ANDROID_TARGETS)
        validate_tools(bundle, selected)
        excluded = [f"tools/{name}" for name in OPTIONAL_TOOLS if name not in selected]
        if not android_rust_targets:
            excluded.extend(f"tools/rust/lib/rustlib/{target}" for target in ANDROID_TARGETS)
        result = {"id": identity, "kind": "release", "version": version,
                  "target": host_target(), "status": "ready", "tools": selected}
        with self.mutation():
            stage(bundle, self.slot(identity), result, excluded=excluded)
        return result

    def bind(self, project, identity):
        with self.mutation():
            self.installation(identity, ready=True)
            result = self.project(project)
            write_json(Path(result["path"]) / ".blockloom" / "hub.json",
                       {"schema": 1, "installation": identity})
            result["installation"] = identity
            return result

    def rebuild(self, identity, jobs=None):
        with self.mutation():
            manifest = self.installation(identity)
            if manifest["kind"] != "dev":
                raise ValueError("Released installations cannot be rebuilt")
            repo = Path(manifest["repo"])
            version = tomllib.loads((repo / "Cargo.toml").read_text(encoding="utf-8"))["workspace"]["package"]["version"]
            manifest.update(version=version, status="ready")
            # Isolate build staging from the registered slot until it succeeds.
            with tempfile.TemporaryDirectory(prefix=".build-", dir=self.root) as temporary:
                output = Path(temporary) / "installation"
                env = {**os.environ, "BLOCKLOOM_INSTALL_DIR": str(output)}
                command = [just_exe(), "replace"] + ([str(jobs)] if jobs else [])
                subprocess.run(command, cwd=repo, env=env, check=True,
                               stdout=sys.stderr, stderr=sys.stderr)
                channel = tomllib.loads((repo / "rust-toolchain.toml").read_text(encoding="utf-8"))["toolchain"]["channel"]
                rustc = subprocess.check_output(["rustup", "which", "--toolchain", channel, "rustc"],
                                               cwd=repo, text=True).strip()
                shutil.copytree(Path(rustc).parent.parent, tool_path(output, "rust"))
                manifest["tools"] = {"rust": {"channel": channel}}
                validate_tools(output, manifest["tools"])
                stage(output, self.slot(identity), manifest, replace=True)
            return manifest

    def launch(self, project):
        with self.mutation():
            result = self.project(project)
            identity = result["installation"]
            if not identity:
                raise ValueError("Assign an installation to this project first")
            installation = self.installation(identity, ready=True)
            directory = self.slot(identity)
            env = dict(os.environ)
            rust = tool_path(directory, "rust")
            bins = [str(rust / "bin")]
            env["RUSTUP_TOOLCHAIN"] = str(rust)
            for name, variable in (("java", "JAVA_HOME"),
                                   ("android-sdk", "BLOCKLOOM_HUB_ANDROID_SDK"),
                                   ("android-ndk", "BLOCKLOOM_HUB_ANDROID_NDK")):
                # Clear inherited Hub paths when this installation omits a tool.
                if variable.startswith("BLOCKLOOM_HUB_"):
                    env.pop(variable, None)
                if name in installation["tools"]:
                    env[variable] = str(tool_path(directory, name))
                    if name == "java":
                        bins.append(str(tool_path(directory, name) / "bin"))
            env["PATH"] = os.pathsep.join(bins + [env.get("PATH", "")])
            process = subprocess.Popen([str(directory / editor_name()), "--project", result["path"]],
                                       cwd=directory, env=env)
            return {"pid": process.pid, "installation": identity, "path": result["path"]}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, help="Override the per-user Hub directory")
    commands = parser.add_subparsers(dest="command", required=True)
    for name in ("projects", "installations"):
        commands.add_parser(name)
    for name, argument in (("add-dev", "repo"), ("open", "project")):
        commands.add_parser(name).add_argument(argument)
    install = commands.add_parser("install")
    install.add_argument("bundle")
    for component in OPTIONAL_TOOLS:
        install.add_argument("--" + component, action="store_true")
    install.add_argument("--android-rust-targets", action="store_true")
    bind = commands.add_parser("bind", help="Explicitly select an editor; does not migrate project data")
    bind.add_argument("project")
    bind.add_argument("installation")
    rebuild = commands.add_parser("rebuild")
    rebuild.add_argument("installation")
    rebuild.add_argument("--jobs", type=int)
    args = parser.parse_args(argv)
    if args.command == "rebuild" and args.jobs is not None and args.jobs < 1:
        parser.error("--jobs must be positive")
    hub = Hub(root=args.root)
    try:
        if args.command == "projects":
            result = hub.projects()
        elif args.command == "installations":
            result = hub.installations()
        elif args.command == "add-dev":
            result = hub.add_dev(args.repo)
        elif args.command == "install":
            components = [name for name in OPTIONAL_TOOLS if getattr(args, name.replace("-", "_"))]
            result = hub.install(args.bundle, components, args.android_rust_targets)
        elif args.command == "bind":
            result = hub.bind(args.project, args.installation)
        elif args.command == "rebuild":
            result = hub.rebuild(args.installation, args.jobs)
        else:
            result = hub.launch(args.project)
        print(json.dumps(result, indent=2))
        return 0
    except (OSError, ValueError, KeyError, subprocess.CalledProcessError) as error:
        print(f"hub: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
