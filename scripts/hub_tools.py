"""Shared tool cache for Hub installations.

If a specific Java, Android SDK/NDK or Rust version was already downloaded
for another editor, reuse the cached copy instead of downloading it again.
The cache lives under <Hub>/tool-cache/<key>/, one directory per tool
version. Installations keep their own self-contained copies (copied from the
cache on a hit, or from the bundle/download on a miss and then saved to the
cache). A cache entry is deleted once no installed editor references its
version.
"""

import hashlib
import json
import os
import platform
import re
import shutil
import sys
import tempfile
from pathlib import Path


CACHE_DIR_NAME = "tool-cache"

ANDROID_TARGETS = ("aarch64-linux-android", "x86_64-linux-android")


def host_target():
    arm = platform.machine().lower() in ("aarch64", "arm64")
    if sys.platform == "win32":
        return "aarch64-pc-windows-msvc" if arm else "x86_64-pc-windows-msvc"
    if sys.platform == "darwin":
        return "aarch64-apple-darwin" if arm else "x86_64-apple-darwin"
    return "aarch64-unknown-linux-gnu" if arm else "x86_64-unknown-linux-gnu"


def host_tag():
    host = {"win32": "windows", "darwin": "macosx"}.get(sys.platform, "linux")
    arch = "aarch64" if platform.machine().lower() in ("arm64", "aarch64") else "x64"
    return host, arch


def sanitize(value):
    return re.sub(r"[^A-Za-z0-9._-]", "_", str(value))[:100]


def cache_dir(hub_root):
    return Path(hub_root) / CACHE_DIR_NAME


def rust_key(channel):
    return f"rust-{sanitize(channel)}-{host_target()}"


def rust_target_key(channel, target):
    return f"rust-std-{sanitize(channel)}-{sanitize(target)}"


def java_key(version):
    host, arch = host_tag()
    return f"java-{sanitize(version)}-{host}-{arch}"


def ndk_key(version):
    host, arch = host_tag()
    return f"android-ndk-{sanitize(version)}-{host}-{arch}"


def sdk_key(packages, version):
    host, arch = host_tag()
    canonical = json.dumps(packages or {}, sort_keys=True, separators=(",", ":"))
    fingerprint = hashlib.sha256(canonical.encode()).hexdigest()[:12]
    return f"android-sdk-{sanitize(version)}-{host}-{arch}-{fingerprint}"


def live_keys(installations):
    """Collect cache keys referenced by installed editors."""
    live = set()
    for manifest in installations:
        tools = manifest.get("tools") if isinstance(manifest, dict) else None
        if not isinstance(tools, dict):
            continue
        rust = tools.get("rust")
        channel = rust.get("channel") if isinstance(rust, dict) else None
        if isinstance(channel, str) and re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", channel):
            live.add(rust_key(channel))
            targets = tools.get("android-rust-targets", [])
            if isinstance(targets, list):
                for target in targets:
                    if target in ANDROID_TARGETS:
                        live.add(rust_target_key(channel, target))
        java = tools.get("java")
        version = java.get("version") if isinstance(java, dict) else None
        if isinstance(version, str) and version:
            live.add(java_key(version))
        sdk = tools.get("android-sdk")
        if isinstance(sdk, dict) and isinstance(sdk.get("version"), str) and sdk["version"]:
            packages = sdk.get("packages") if isinstance(sdk.get("packages"), dict) else {}
            live.add(sdk_key(packages, sdk["version"]))
        ndk = tools.get("android-ndk")
        version = ndk.get("version") if isinstance(ndk, dict) else None
        if isinstance(version, str) and version:
            live.add(ndk_key(version))
    return live


def _suffix():
    return ".exe" if os.name == "nt" else ""


def valid_rust_base(path, channel=None):
    path = Path(path)
    suffix = _suffix()
    if not (path / "bin" / ("rustc" + suffix)).is_file():
        return False
    if not (path / "bin" / ("cargo" + suffix)).is_file():
        return False
    return (path / "lib" / "rustlib" / host_target() / "lib").is_dir()


def valid_rust_target(path):
    return (Path(path) / "lib").is_dir()


def valid_java(path):
    return (Path(path) / "bin" / ("java" + _suffix())).is_file()


def valid_sdk(path):
    return (Path(path) / "platform-tools" / ("adb" + _suffix())).is_file()


def valid_ndk(path):
    return (Path(path) / "source.properties").is_file()


VALIDATORS = {
    "rust-": valid_rust_base,
    "rust-std-": valid_rust_target,
    "java-": valid_java,
    "android-sdk-": valid_sdk,
    "android-ndk-": valid_ndk,
}


def valid_entry(key, path):
    for prefix in sorted(VALIDATORS, key=len, reverse=True):
        if key.startswith(prefix):
            try:
                return bool(VALIDATORS[prefix](path))
            except OSError:
                return False
    return Path(path).is_dir()


def cached_path(cache_root, key):
    return Path(cache_root) / key


def is_cached(cache_root, key):
    path = cached_path(cache_root, key)
    return path.is_dir() and valid_entry(key, path)


def _copy_tree(source, destination, checkpoint=None):
    from hub_process import copy_file

    def copy(source_file, destination_file):
        return copy_file(source_file, destination_file)

    def ignore(directory, names):
        if checkpoint is not None:
            checkpoint()
        return []

    if checkpoint is not None:
        checkpoint()
    shutil.copytree(source, destination, copy_function=copy, ignore=ignore)


def reuse(cache_root, key, destination, checkpoint=None):
    """Copy a cached tool version into a fresh destination. Return True on hit."""
    source = cached_path(cache_root, key)
    if not source.is_dir() or not valid_entry(key, source):
        return False
    if Path(destination).exists():
        raise ValueError(f"Tool destination already exists: {destination}")
    if checkpoint is not None:
        checkpoint()
    print(f"Reusing cached {key}...", file=sys.stderr, flush=True)
    _copy_tree(source, destination, checkpoint)
    if not valid_entry(key, destination):
        shutil.rmtree(destination, ignore_errors=True)
        raise ValueError(f"Cached tool failed validation: {key}")
    return True


def store(cache_root, key, source, checkpoint=None):
    """Save an installed tool directory into the cache for future editors."""
    source = Path(source)
    if not source.is_dir():
        return False
    if is_cached(cache_root, key):
        return True
    if not valid_entry(key, source):
        return False
    Path(cache_root).mkdir(parents=True, exist_ok=True)
    if checkpoint is not None:
        checkpoint()
    print(f"Caching {key} for future editors...", file=sys.stderr, flush=True)
    with tempfile.TemporaryDirectory(prefix=".tool-cache-", dir=cache_root) as temporary:
        staging = Path(temporary) / "entry"
        _copy_tree(source, staging, checkpoint)
        if not valid_entry(key, staging):
            raise ValueError(f"Tool failed validation before caching: {key}")
        if checkpoint is not None:
            checkpoint()
        destination = cached_path(cache_root, key)
        try:
            staging.rename(destination)
        except (FileExistsError, OSError):
            if is_cached(cache_root, key):
                # Another operation populated the same version first.
                pass
            else:
                shutil.rmtree(destination, ignore_errors=True)
                staging.rename(destination)
    return True


def store_rust_base(cache_root, rust_dir, channel, checkpoint=None):
    """Cache a Rust toolchain without its Android target subdirectories."""
    rust_dir = Path(rust_dir)
    if not valid_rust_base(rust_dir):
        return False
    key = rust_key(channel)
    if is_cached(cache_root, key):
        return True
    Path(cache_root).mkdir(parents=True, exist_ok=True)
    if checkpoint is not None:
        checkpoint()
    print(f"Caching {key} for future editors...", file=sys.stderr, flush=True)
    with tempfile.TemporaryDirectory(prefix=".tool-cache-", dir=cache_root) as temporary:
        staging = Path(temporary) / "entry"
        from hub_process import copy_file

        def ignore(directory, names):
            if checkpoint is not None:
                checkpoint()
            if Path(directory) == rust_dir / "lib" / "rustlib":
                return [name for name in names if name in ANDROID_TARGETS]
            return []

        shutil.copytree(rust_dir, staging, ignore=ignore,
                        copy_function=lambda src, dst: copy_file(src, dst))
        if not valid_rust_base(staging):
            raise ValueError(f"Rust toolchain failed validation before caching: {key}")
        if checkpoint is not None:
            checkpoint()
        destination = cached_path(cache_root, key)
        try:
            staging.rename(destination)
        except (FileExistsError, OSError):
            if is_cached(cache_root, key):
                pass
            else:
                shutil.rmtree(destination, ignore_errors=True)
                staging.rename(destination)
    return True


def store_installation(cache_root, installation_dir, tools, checkpoint=None):
    """Save every cacheable tool from a finished installation."""
    installation_dir = Path(installation_dir)
    rust = tools.get("rust", {}) if isinstance(tools, dict) else {}
    channel = rust.get("channel")
    if isinstance(channel, str) and re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", channel):
        rust_dir = installation_dir / "tools" / "rust"
        if rust_dir.is_dir():
            store_rust_base(cache_root, rust_dir, channel, checkpoint)
            targets = tools.get("android-rust-targets", [])
            if isinstance(targets, list):
                for target in targets:
                    if target in ANDROID_TARGETS:
                        source = rust_dir / "lib" / "rustlib" / target
                        if source.is_dir():
                            store(cache_root, rust_target_key(channel, target), source, checkpoint)
    for name, key_fn, valid in (
        ("java", lambda v: java_key(v), valid_java),
        ("android-ndk", lambda v: ndk_key(v), valid_ndk),
    ):
        entry = tools.get(name, {}) if isinstance(tools, dict) else {}
        version = entry.get("version") if isinstance(entry, dict) else None
        if isinstance(version, str) and version:
            source = installation_dir / "tools" / name
            if source.is_dir():
                try:
                    store(cache_root, key_fn(version), source, checkpoint)
                except ValueError:
                    pass
    sdk = tools.get("android-sdk", {}) if isinstance(tools, dict) else {}
    if isinstance(sdk, dict) and isinstance(sdk.get("version"), str) and sdk["version"]:
        source = installation_dir / "tools" / "android-sdk"
        if source.is_dir():
            packages = sdk.get("packages") if isinstance(sdk.get("packages"), dict) else {}
            try:
                store(cache_root, sdk_key(packages, sdk["version"]), source, checkpoint)
            except ValueError:
                pass


def collect_garbage(hub_root, installations):
    """Delete cached tool versions no installed editor references."""
    root = cache_dir(hub_root)
    if not root.is_dir():
        return []
    live = live_keys(installations)
    removed = []
    for entry in sorted(root.iterdir()):
        if entry.name.startswith(".") or not entry.is_dir():
            continue
        if entry.name not in live:
            print(f"Removing unused cached tool {entry.name}...", file=sys.stderr, flush=True)
            shutil.rmtree(entry, ignore_errors=True)
            removed.append(entry.name)
    return removed


def summary(hub_root, installations):
    """Describe cached tool versions and whether editors still use them."""
    root = cache_dir(hub_root)
    live = live_keys(installations)
    entries = []
    if root.is_dir():
        for entry in sorted(root.iterdir()):
            if entry.name.startswith(".") or not entry.is_dir():
                continue
            total = 0
            try:
                for path in entry.rglob("*"):
                    try:
                        if path.is_file() and not path.is_symlink():
                            total += path.stat().st_size
                    except OSError:
                        continue
            except OSError:
                continue
            entries.append({"key": entry.name, "size": total, "live": entry.name in live,
                            "valid": valid_entry(entry.name, entry)})
    return {"directory": str(root), "entries": entries,
            "live": sorted(live)}
