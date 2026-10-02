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
import time
import tomllib
import uuid
import hub_download
import hub_github

from hub_install import MANIFEST, editor_name, promote, stage, validate_payload
from replace import host_target, just_exe
from hub_process import Cancelled, bundle_rust, checkpoint, copy_file, process_token, running, run


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


def local_tools(registry, overrides=None):
    settings = registry.parent / "android.json"
    config = read_json(settings) if settings.exists() else {}
    default_sdk = registry.parent / "android-sdk" if os.environ.get("BLOCKLOOM_DATA_DIR") else Path.home() / "Blockloom" / "android-sdk"
    sdk = Path(config.get("sdk_path") or os.environ.get("ANDROID_HOME") or default_sdk)
    ndk = config.get("ndk_path") or os.environ.get("ANDROID_NDK_HOME")
    if not ndk and (sdk / "ndk").is_dir():
        versions = sorted((sdk / "ndk").iterdir())
        ndk = str(versions[-1]) if versions else None
    sources = {"java": os.environ.get("JAVA_HOME"), "android-sdk": str(sdk), "android-ndk": ndk}
    sources.update(overrides or {})
    result = {}
    for name, source in sources.items():
        if not source:
            continue
        directory = Path(source).expanduser().resolve()
        metadata = directory / ("release" if name == "java" else
                                "platform-tools/source.properties" if name == "android-sdk" else "source.properties")
        if not metadata.is_file():
            continue
        text = metadata.read_text(encoding="utf-8")
        match = re.search(r'^JAVA_VERSION="([^"]+)"' if name == "java" else r"^Pkg.Revision\s*=\s*(\S+)", text, re.MULTILINE)
        if match:
            result[name] = {"source": str(directory), "version": match[1]}
    return result


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
        entries = [self.installation(p.name) for p in sorted(directory.iterdir())
                   if p.is_dir() and not p.name.startswith(".")]
        active = self.running_editors()
        for entry in entries:
            count = sum(p["installation"] == entry["id"] for p in active)
            if count:
                entry["running"] = count
            candidate = self.candidate_path(entry["id"]) / MANIFEST
            if entry["kind"] == "dev" and candidate.exists():
                entry["prepared"] = True
        return entries

    def running_editors(self):
        path = self.root / "running.json"
        records = read_json(path).get("editors", []) if path.exists() else []
        return [record for record in records if running(record)]

    def project_active(self, project):
        key = os.path.normcase(str(Path(project).resolve()))
        if any(os.path.normcase(p["path"]) == key for p in self.running_editors()):
            return True
        lock = Path(project) / ".blockloom" / "lock.json"
        if not lock.exists():
            return False
        owner = read_json(lock)
        return process_token(owner["pid"]) is not None or time.time() - owner["heartbeat"] < 30

    def require_idle(self, project):
        if self.project_active(project):
            raise ValueError("Close this project's editor or shell before changing its editor or opening it again")

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
        sources = [self.registry, self.root / "projects.json"]
        remembered_projects = {}
        for source in sources:
            if source.exists():
                for remembered in read_json(source).get("projects", []):
                    key = os.path.normcase(str(Path(remembered["path"]).expanduser().resolve()))
                    previous = remembered_projects.get(key)
                    if previous is None or remembered.get("opened_at", 0) > previous.get("opened_at", 0):
                        remembered_projects[key] = remembered
        entries = []
        for remembered in remembered_projects.values():
            entry = {"path": remembered["path"]}
            try:
                entry.update(self.project(remembered["path"]))
                entry["active"] = self.project_active(entry["path"])
            except (OSError, ValueError, KeyError) as error:
                entry.update(error=str(error), active=True)
            entry["opened_at"] = remembered.get("opened_at", 0)
            entries.append(entry)
        return sorted(entries, key=lambda p: (-p["opened_at"], p.get("name", "").lower()))

    def remember(self, project):
        with self.mutation():
            result = self.project(project)
            registry = self.root / "projects.json"
            saved = read_json(registry) if registry.exists() else {"projects": []}
            key = os.path.normcase(result["path"])
            if not any(os.path.normcase(str(Path(p["path"]).expanduser().resolve())) == key
                       for p in saved["projects"]):
                saved["projects"].append({"path": result["path"], "opened_at": 0})
                write_json(registry, saved)
            return result

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
            stage(bundle, self.slot(identity), result, excluded=excluded, checkpoint=checkpoint)
        return result

    def release_settings(self, source=None, url=None, repo=None):
        path = self.root / "catalog.json"
        config = read_json(path) if path.exists() else {}
        config = {"source": "https", "repo": hub_github.DEFAULT_REPO, "url": "", **config}
        if source is not None:
            if source not in ("https", "github-cli"):
                raise ValueError("Invalid release source")
            config.update(source=source, repo=hub_github.repository(repo or config["repo"]), url=url or "")
            if config["source"] == "https" and config["url"]:
                hub_download.secure_url(config["url"])
            with self.mutation():
                write_json(path, config)
        return config

    def check_releases(self, url=None):
        config_path = self.root / "catalog.json"
        config = self.release_settings()
        if config["source"] == "github-cli" and not url:
            entries = hub_github.catalog(config["repo"], host_target())
            for entry in entries:
                entry["installed"] = self.slot("release-" + entry["version"]).exists()
            return {**config, "releases": entries}
        url = url or os.environ.get("BLOCKLOOM_HUB_CATALOG_URL") or config.get("url")
        if not url:
            raise ValueError("Enter a release catalog URL to check for Blockloom versions")
        entries = hub_download.catalog(url, host_target())
        with self.mutation():
            write_json(config_path, {**config, "source": "https", "url": url})
        for entry in entries:
            entry["installed"] = self.slot("release-" + entry["version"]).exists()
        return {"url": url, "releases": entries}

    def download_release(self, version, components=(), android_rust_targets=False, sha256=None):
        entries = self.check_releases()["releases"]
        entry = next((item for item in entries if item["version"] == version), None)
        if entry is None:
            raise ValueError("Release is not available for this computer")
        if entry["installed"]:
            raise ValueError("This Blockloom version is already installed")
        if sha256 and hub_download.digest(sha256) != hub_download.digest(entry["sha256"]):
            raise ValueError("The release catalog changed. Check releases again before installing")
        with tempfile.TemporaryDirectory(prefix=".download-", dir=self.root) as temporary:
            temporary = Path(temporary)
            archive = temporary / "release.archive"
            if github := entry.get("github"):
                hub_github.asset(github["repo"], github["tag"], github["asset"], archive)
                hub_download.verify_file(archive, entry["sha256"], entry["size"])
            else:
                hub_download.download(entry["url"], archive, entry["sha256"], entry["size"])
            output = temporary / "release"
            hub_download.extract(archive, output, entry["format"], entry["unpacked_size"])
            bundles = [output] if (output / MANIFEST).is_file() else [p for p in output.iterdir() if p.is_dir() and (p / MANIFEST).is_file()]
            if len(bundles) != 1 or read_json(bundles[0] / MANIFEST).get("version") != version:
                raise ValueError("Downloaded bundle does not match the selected release")
            return self.install(bundles[0], components, android_rust_targets)

    def bind(self, project, identity):
        with self.mutation():
            self.installation(identity, ready=True)
            result = self.project(project)
            self.require_idle(result["path"])
            write_json(Path(result["path"]) / ".blockloom" / "hub.json",
                       {"schema": 1, "installation": identity})
            result["installation"] = identity
            return result

    def rebuild(self, identity, jobs=None):
        self.prepare_dev(identity, jobs)
        return self.install_dev(identity)

    def require_installation_idle(self, identity):
        if any(p["installation"] == identity for p in self.running_editors()):
            raise ValueError("Close editors using this installation before rebuilding it")
        if any(p.get("installation") == identity and p.get("active") for p in self.projects()):
            raise ValueError("Close projects using this installation before rebuilding it")

    def candidate_path(self, identity):
        self.slot(identity)
        return self.root / "pending" / identity

    def prepare_dev(self, identity, jobs=None):
        with self.mutation():
            manifest = self.installation(identity)
            if manifest["kind"] != "dev":
                raise ValueError("Released installations cannot be rebuilt")
            self.require_installation_idle(identity)
            repo = Path(manifest["repo"])
            version = tomllib.loads((repo / "Cargo.toml").read_text(encoding="utf-8"))["workspace"]["package"]["version"]
            manifest.update(version=version, status="ready")
            manifest["build_id"] = uuid.uuid4().hex
            # Isolate build staging from the registered slot until it succeeds.
            with tempfile.TemporaryDirectory(prefix=".build-", dir=self.root) as temporary:
                output = Path(temporary) / "installation"
                env = {**os.environ, "BLOCKLOOM_INSTALL_DIR": str(output)}
                command = [just_exe(), "replace"] + ([str(jobs)] if jobs else [])
                run(command, cwd=repo, env=env, stdout=sys.stderr, stderr=sys.stderr,
                    log_file=self.root / "logs" / f"{identity}-build.log")
                checkpoint()
                channel = tomllib.loads((repo / "rust-toolchain.toml").read_text(encoding="utf-8"))["toolchain"]["channel"]
                print(f"Bundling Rust {channel} for the prepared editor...", file=sys.stderr)
                rustc = subprocess.check_output(["rustup", "which", "--toolchain", channel, "rustc"],
                                               cwd=repo, text=True).strip()
                bundle_rust(Path(rustc).parent.parent, tool_path(output, "rust"), channel)
                manifest["tools"] = {"rust": {"channel": channel}}
                validate_tools(output, manifest["tools"])
                sources = local_tools(self.registry)
                targets = [target for target in ANDROID_TARGETS if
                           (tool_path(output, "rust") / "lib" / "rustlib" / target / "lib").is_dir()]
                manifest["sources"] = sources
                manifest["android_targets_available"] = len(targets) == len(ANDROID_TARGETS)
                checkpoint()
                promote(output, self.candidate_path(identity), manifest, replace=True, checkpoint=checkpoint)
                print("Build prepared. Choose installation options in the Hub.", file=sys.stderr)
            return manifest

    def dev_options(self, identity):
        if self.installation(identity)["kind"] != "dev":
            raise ValueError("Only developer installations have prepared builds")
        manifest = read_json(self.candidate_path(identity) / MANIFEST)
        if manifest.get("id") != identity or manifest.get("kind") != "dev":
            raise ValueError("Invalid prepared development installation")
        return manifest

    def install_dev(self, identity, components=(), android_rust_targets=False, sources=None, build_id=None):
        with self.mutation():
            current = self.installation(identity)
            if current["kind"] != "dev":
                raise ValueError("Only developer installations can use a prepared build")
            self.require_installation_idle(identity)
            candidate = self.candidate_path(identity)
            manifest = read_json(candidate / MANIFEST)
            if manifest.get("id") != identity or manifest.get("kind") != "dev" or manifest.get("target") != host_target():
                raise ValueError("Invalid prepared development installation")
            if build_id is not None and manifest.get("build_id") != build_id:
                raise ValueError("A newer development build was prepared. Reopen its installation options before installing")
            available = manifest.get("sources", {})
            if sources:
                discovered = local_tools(self.registry, sources)
                for name in sources:
                    if name not in discovered:
                        raise ValueError(f"Select a complete local {name} installation with version metadata")
                    available[name] = discovered[name]
            selected = {"rust": manifest["tools"]["rust"]}
            for name in components:
                if name not in OPTIONAL_TOOLS or name not in available:
                    raise ValueError(f"Select a local installation for {name}")
                selected[name] = {"version": available[name]["version"]}
            if android_rust_targets:
                selected["android-rust-targets"] = list(ANDROID_TARGETS)
            excluded = [] if android_rust_targets else [f"tools/rust/lib/rustlib/{target}" for target in ANDROID_TARGETS]
            # Build a final payload privately, keeping the prepared build for retry.
            with tempfile.TemporaryDirectory(prefix=".configure-", dir=self.root) as temporary:
                output = Path(temporary) / "installation"
                print("Preparing editor and Rust toolchain...", file=sys.stderr)
                stage(candidate, output, excluded=excluded, checkpoint=checkpoint)
                if android_rust_targets:
                    hub_download.install_rust_targets(tool_path(output, "rust"), selected["rust"]["channel"],
                                                      ANDROID_TARGETS, Path(temporary))
                for name in components:
                    checkpoint()
                    print(f"Including {name} {selected[name]['version']}...", file=sys.stderr)
                    shutil.copytree(available[name]["source"], tool_path(output, name), copy_function=copy_file,
                                    ignore=shutil.ignore_patterns("ndk") if name == "android-sdk" else None)
                manifest.pop("sources", None)
                manifest.pop("android_targets_available", None)
                manifest["tools"] = selected
                validate_tools(output, selected)
                checkpoint()
                print("Saving development installation...", file=sys.stderr)
                promote(output, self.slot(identity), manifest, replace=True, checkpoint=checkpoint)
            print("Development installation ready.", file=sys.stderr)
            return manifest

    def launch(self, project):
        with self.mutation():
            result = self.project(project)
            self.require_idle(result["path"])
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
            logs = self.root / "logs"
            logs.mkdir(exist_ok=True)
            # Keep editor output out of the service's JSON response pipe.
            with (logs / f"{identity}-editor.log").open("ab") as output:
                process = subprocess.Popen([str(directory / editor_name()), "--project", result["path"]],
                                           cwd=directory, env=env, stdin=subprocess.DEVNULL,
                                           stdout=output, stderr=subprocess.STDOUT)
            token = process_token(process.pid)
            if token is not None:
                records = self.running_editors()
                records.append({"pid": process.pid, "token": token, "installation": identity,
                                "path": result["path"], "launched_at": time.time()})
                try:
                    write_json(self.root / "running.json", {"editors": records})
                except BaseException:
                    process.terminate()
                    process.wait(timeout=10)
                    raise
            return {"pid": process.pid, "installation": identity, "path": result["path"]}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, help="Override the per-user Hub directory")
    commands = parser.add_subparsers(dest="command", required=True)
    for name in ("projects", "installations"):
        commands.add_parser(name)
    for name, argument in (("add-dev", "repo"), ("open", "project"), ("remember", "project")):
        commands.add_parser(name).add_argument(argument)
    commands.add_parser("dev-options").add_argument("installation")
    commands.add_parser("check-releases").add_argument("url", nargs="?")
    settings = commands.add_parser("release-settings")
    settings.add_argument("--source", choices=("https", "github-cli"))
    settings.add_argument("--url")
    settings.add_argument("--repo")
    install = commands.add_parser("install")
    install.add_argument("bundle")
    for component in OPTIONAL_TOOLS:
        install.add_argument("--" + component, action="store_true")
    install.add_argument("--android-rust-targets", action="store_true")
    remote = commands.add_parser("download-release")
    remote.add_argument("version")
    remote.add_argument("--sha256")
    for component in OPTIONAL_TOOLS:
        remote.add_argument("--" + component, action="store_true")
    remote.add_argument("--android-rust-targets", action="store_true")
    bind = commands.add_parser("bind", help="Explicitly select an editor; does not migrate project data")
    bind.add_argument("project")
    bind.add_argument("installation")
    for name in ("rebuild", "prepare-dev"):
        rebuild = commands.add_parser(name)
        rebuild.add_argument("installation")
        rebuild.add_argument("--jobs", type=int)
    developer = commands.add_parser("install-dev")
    developer.add_argument("installation")
    for component in OPTIONAL_TOOLS:
        developer.add_argument("--" + component, action="store_true")
        developer.add_argument("--" + component + "-source")
    developer.add_argument("--android-rust-targets", action="store_true")
    developer.add_argument("--build-id")
    args = parser.parse_args(argv)
    if args.command in ("rebuild", "prepare-dev") and args.jobs is not None and args.jobs < 1:
        parser.error("--jobs must be positive")
    hub = Hub(root=args.root)
    try:
        if args.command == "projects":
            result = hub.projects()
        elif args.command == "installations":
            result = hub.installations()
        elif args.command == "add-dev":
            result = hub.add_dev(args.repo)
        elif args.command == "remember":
            result = hub.remember(args.project)
        elif args.command == "install":
            components = [name for name in OPTIONAL_TOOLS if getattr(args, name.replace("-", "_"))]
            result = hub.install(args.bundle, components, args.android_rust_targets)
        elif args.command == "release-settings":
            result = hub.release_settings(args.source, args.url, args.repo)
        elif args.command == "check-releases":
            result = hub.check_releases(args.url)
        elif args.command == "download-release":
            components = [name for name in OPTIONAL_TOOLS if getattr(args, name.replace("-", "_"))]
            result = hub.download_release(args.version, components, args.android_rust_targets, args.sha256)
        elif args.command == "bind":
            result = hub.bind(args.project, args.installation)
        elif args.command == "rebuild":
            result = hub.rebuild(args.installation, args.jobs)
        elif args.command == "prepare-dev":
            result = hub.prepare_dev(args.installation, args.jobs)
        elif args.command == "dev-options":
            result = hub.dev_options(args.installation)
        elif args.command == "install-dev":
            components = [name for name in OPTIONAL_TOOLS if getattr(args, name.replace("-", "_"))]
            sources = {name: getattr(args, name.replace("-", "_") + "_source") for name in components
                       if getattr(args, name.replace("-", "_") + "_source")}
            result = hub.install_dev(args.installation, components, args.android_rust_targets, sources, args.build_id)
        else:
            result = hub.launch(args.project)
        print(json.dumps(result, indent=2))
        return 0
    except (OSError, ValueError, KeyError, subprocess.CalledProcessError,
            hub_download.zipfile.BadZipFile, hub_download.tarfile.TarError) as error:
        print(f"hub: {error}", file=sys.stderr)
        return 1
    except Cancelled as error:
        print(f"hub: {error}", file=sys.stderr)
        return 130


if __name__ == "__main__":
    sys.exit(main())
