"""Bounded HTTPS downloads and archive extraction for Hub installations."""

import hashlib
import http.client
import io
import json
import os
import platform
import posixpath
import socket
import ssl
from pathlib import Path, PurePosixPath
import re
import shutil
import stat
import sys
import tarfile
import tempfile
import time
import tomllib
import urllib.error
from urllib.parse import urlsplit
from urllib.request import HTTPRedirectHandler, Request, build_opener
import zipfile
import xml.etree.ElementTree as ET

from hub_process import Cancelled, checkpoint, copy_file

MAX_ARCHIVE = 8 * 1024**3
MAX_EXTRACTED = 24 * 1024**3
MAX_MEMBERS = 100000

READ_TIMEOUT = 30
DOWNLOAD_ATTEMPTS = 8
MAX_RETRY_DELAY = 30

TRANSIENT_ERRORS = (TimeoutError, ConnectionError, urllib.error.URLError,
                    http.client.HTTPException, ssl.SSLError, socket.gaierror)


class DownloadMismatch(ValueError):
    """A finished download failed its size/hash check; likely truncated, so retry."""


def sleep_interruptible(delay):
    steps = max(1, int(delay / 0.2))
    for _ in range(steps):
        time.sleep(delay / steps)
        checkpoint()


def transient_failure(action, url):
    for attempt in range(DOWNLOAD_ATTEMPTS):
        try:
            return action()
        except Cancelled:
            raise
        except (TRANSIENT_ERRORS + (DownloadMismatch,)) as error:
            checkpoint()
            if attempt + 1 >= DOWNLOAD_ATTEMPTS:
                raise ValueError(
                    f"Download failed after {DOWNLOAD_ATTEMPTS} attempts: {url} ({error}). "
                    "Check your connection and retry."
                ) from error
            delay = min(2 ** (attempt + 1), MAX_RETRY_DELAY)
            print(f"Download stalled ({error}); retrying {attempt + 2}/{DOWNLOAD_ATTEMPTS} in {delay}s...",
                  file=sys.stderr, flush=True)
            sleep_interruptible(delay)
            checkpoint()


def secure_url(url):
    if not isinstance(url, str):
        raise ValueError("Download URL must be a string")
    parsed = urlsplit(url)
    if parsed.scheme != "https" or not parsed.hostname or parsed.username or parsed.password or parsed.fragment:
        raise ValueError("Downloads require an HTTPS URL without credentials or fragments")
    return url


class SecureRedirect(HTTPRedirectHandler):
    def redirect_request(self, request, response, code, message, headers, new_url):
        secure_url(new_url)
        return super().redirect_request(request, response, code, message, headers, new_url)


def response(url, start=0):
    checkpoint()
    headers = {"User-Agent": "Blockloom-Hub", "Accept-Encoding": "identity"}
    if start:
        headers["Range"] = f"bytes={start}-"
    return build_opener(SecureRedirect()).open(Request(secure_url(url), headers=headers),
                                               timeout=READ_TIMEOUT)


def read_remote(url, limit=2 * 1024**2):
    def fetch():
        data = bytearray()
        with response(url) as stream:
            while True:
                checkpoint()
                chunk = stream.read(min(65536, limit + 1 - len(data)))
                if not chunk:
                    return bytes(data)
                data.extend(chunk)
                if len(data) > limit:
                    raise ValueError("Remote manifest exceeds the size limit")
    return transient_failure(fetch, url)


def version(value):
    number = r"(?:0|[1-9][0-9]*)"
    if not isinstance(value, str) or not re.fullmatch(rf"{number}\.{number}\.{number}(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?", value):
        raise ValueError("Release version must be canonical SemVer without build metadata")
    for part in value.split("-", 1)[1].split(".") if "-" in value else []:
        if part.isdigit() and len(part) > 1 and part.startswith("0"):
            raise ValueError("Numeric SemVer prerelease identifiers cannot start with zero")
    return value


def digest(value):
    if not isinstance(value, str) or not re.fullmatch(r"[0-9a-fA-F]{64}", value):
        raise ValueError("Archive must declare its SHA-256")
    return value.lower()


def catalog(url, target):
    raw = read_remote(url)
    try:
        data = json.loads(raw)
    except ValueError:
        # Pasting a repository page (https://github.com/owner/repo) is the
        # common mistake; it returns HTML, and the raw JSON error explains
        # nothing. Point at the setting that handles it instead.
        from urllib.parse import urlsplit as _split
        host = (_split(url).hostname or "").lower()
        if host in ("github.com", "www.github.com"):
            raise ValueError(
                "This looks like a GitHub repository page, not a release catalog. "
                "In Hub settings, enable 'Use GitHub CLI for private releases' instead."
            ) from None
        raise ValueError(
            "The catalog URL did not return a release catalog "
            "(JSON with schema 1). Check it in Hub settings."
        ) from None
    return catalog_data(data, target)


def catalog_data(data, target):
    if not isinstance(data, dict) or data.get("schema") != 1 or not isinstance(data.get("releases"), list):
        raise ValueError("Expected a schema 1 Blockloom release catalog")
    if len(data["releases"]) > 1000:
        raise ValueError("Release catalog has too many entries")
    entries, identities = [], set()
    for entry in data["releases"]:
        if not isinstance(entry, dict):
            raise ValueError("Invalid release catalog entry")
        version(entry.get("version"))
        secure_url(entry.get("url", ""))
        digest(entry.get("sha256"))
        if not isinstance(entry.get("size"), int) or not 0 < entry["size"] <= MAX_ARCHIVE:
            raise ValueError("Release must declare a bounded archive byte count")
        if not isinstance(entry.get("unpacked_size"), int) or not 0 < entry["unpacked_size"] <= MAX_EXTRACTED:
            raise ValueError("Release must declare a bounded extraction byte count")
        if entry.get("format") not in ("zip", "tar.xz", "tar.gz") or not isinstance(entry.get("target"), str):
            raise ValueError("Release archive format or target is invalid")
        identity = (entry["version"], entry["target"])
        if identity in identities:
            raise ValueError("Catalog contains duplicate releases for the same version and target")
        identities.add(identity)
        if entry["target"] == target:
            entries.append(entry)
    return entries


def download(url, destination, sha256, size=None, limit=MAX_ARCHIVE, algorithm="sha256"):
    if algorithm not in ("sha256", "sha1"):
        raise ValueError("Unsupported download checksum")
    length = 64 if algorithm == "sha256" else 40
    if not isinstance(sha256, str) or not re.fullmatch(rf"[0-9a-fA-F]{{{length}}}", sha256):
        raise ValueError("Archive must declare its checksum")
    expected = sha256.lower()

    def fetch():
        try:
            start = destination.stat().st_size if destination.exists() else 0
        except OSError:
            start = 0
        if size is not None and start > size:
            destination.unlink(missing_ok=True)
            start = 0
        hash_value = hashlib.new(algorithm)
        if start:
            try:
                with destination.open("rb") as existing:
                    while chunk := existing.read(1024 * 1024):
                        checkpoint()
                        hash_value.update(chunk)
            except OSError:
                destination.unlink(missing_ok=True)
                start = 0
                hash_value = hashlib.new(algorithm)
            else:
                print(f"Resuming download from {start / 1024**2:.1f} MiB...",
                      file=sys.stderr, flush=True)
        amount, last = start, 0
        try:
            try:
                stream = response(url, start)
            except urllib.error.HTTPError as error:
                if error.code == 416 and start:
                    destination.unlink(missing_ok=True)
                    hash_value = hashlib.new(algorithm)
                    amount, start = 0, 0
                    stream = response(url)
                else:
                    raise
            if start:
                status = getattr(stream, "status", 200)
                content_range = None
                headers = getattr(stream, "headers", None)
                if headers is not None:
                    content_range = headers.get("Content-Range")
                if status != 206 or (content_range is not None
                                     and not content_range.startswith(f"bytes {start}-")):
                    stream.close()
                    destination.unlink(missing_ok=True)
                    hash_value = hashlib.new(algorithm)
                    amount, start = 0, 0
                    stream = response(url)
            with stream, destination.open("ab" if start else "xb") as output:
                while True:
                    checkpoint()
                    chunk = stream.read(256 * 1024)
                    if not chunk:
                        break
                    amount += len(chunk)
                    if amount > limit or (size is not None and amount > size):
                        raise ValueError("Download exceeds the declared size")
                    output.write(chunk)
                    hash_value.update(chunk)
                    now = time.monotonic()
                    if now - last >= 1:
                        print(f"Downloading: {amount / 1024**2:.1f} MiB" +
                              (f" / {size / 1024**2:.1f} MiB" if size else ""), file=sys.stderr, flush=True)
                        last = now
            if (size is not None and amount != size) or hash_value.hexdigest() != expected:
                raise DownloadMismatch(
                    f"Download size or {algorithm} does not match the manifest: {url} "
                    f"(got {amount} bytes, expected {size} bytes; got {algorithm} "
                    f"{hash_value.hexdigest()}, expected {expected})")
            checkpoint()
        except (DownloadMismatch,) + TRANSIENT_ERRORS:
            raise
        except Cancelled:
            destination.unlink(missing_ok=True)
            raise
        except BaseException:
            destination.unlink(missing_ok=True)
            raise

    try:
        return transient_failure(fetch, url)
    except Cancelled:
        destination.unlink(missing_ok=True)
        raise
    except ValueError:
        destination.unlink(missing_ok=True)
        raise


def verify_file(path, sha256, size):
    if path.stat().st_size != size or size > MAX_ARCHIVE:
        raise ValueError("Downloaded asset size does not match the catalog")
    checksum = hashlib.sha256()
    with path.open("rb") as stream:
        while chunk := stream.read(1024 * 1024):
            checkpoint()
            checksum.update(chunk)
    if checksum.hexdigest() != digest(sha256):
        raise ValueError("Downloaded asset SHA-256 does not match the catalog")


def filesystem_case_sensitive(directory):
    with tempfile.TemporaryDirectory(prefix=".hub-case-", dir=directory) as temporary:
        probe = Path(temporary) / "case"
        probe.touch()
        return not (probe.parent / "CASE").exists()


def extract(archive, destination, format, limit=MAX_EXTRACTED, allow_file_links=False,
            allow_case_sensitive_paths=False):
    destination.mkdir(parents=True)
    # Linux tool archives can contain headers whose names differ only by case.
    case_sensitive = allow_case_sensitive_paths and filesystem_case_sensitive(destination)
    names, total, count, last = set(), 0, 0, 0
    links = []

    def file_link(path, target, relative=True):
        if target.startswith("/") or "\\" in target or ":" in target:
            raise ValueError("Archive contains an unsafe link")
        name = posixpath.normpath(posixpath.join(path.parent.relative_to(destination).as_posix(), target)
                                 if relative else target)
        if name == ".." or name.startswith("../"):
            raise ValueError("Archive link escapes its destination")
        links.append((path, destination / name))

    def path_for(name, size):
        nonlocal count, total, last
        checkpoint()
        parts = PurePosixPath(name).parts
        if not parts or name.startswith("/") or "\\" in name or ":" in name or ".." in parts:
            raise ValueError("Archive contains an unsafe path")
        if any(p.endswith((".", " ")) or p.split(".")[0].upper() in ("CON", "PRN", "AUX", "NUL", *[f"COM{i}" for i in range(1, 10)], *[f"LPT{i}" for i in range(1, 10)]) for p in parts):
            raise ValueError("Archive contains a nonportable path")
        key = "/".join(parts)
        if not case_sensitive:
            key = key.casefold()
        if key in names:
            raise ValueError(f"Archive contains duplicate paths: {name}")
        names.add(key)
        count += 1
        total += size
        if count > MAX_MEMBERS or total > limit or size < 0:
            raise ValueError("Archive exceeds extraction limits")
        now = time.monotonic()
        if now - last >= 1:
            print(f"Extracting: {count} entries, {total / 1024**2:.0f} MiB", file=sys.stderr, flush=True)
            last = now
        return destination.joinpath(*parts)

    def write(stream, path, size, mode):
        path.parent.mkdir(parents=True, exist_ok=True)
        remaining = size
        with path.open("xb") as output:
            while remaining:
                checkpoint()
                chunk = stream.read(min(remaining, 1024 * 1024))
                if not chunk:
                    raise ValueError("Archive file is truncated")
                output.write(chunk)
                remaining -= len(chunk)
        if os.name != "nt":
            path.chmod(mode & 0o777 or 0o644)

    if format == "zip":
        with zipfile.ZipFile(archive) as bundle:
            for info in bundle.infolist():
                mode = info.external_attr >> 16
                if stat.S_ISLNK(mode) and allow_file_links:
                    path = path_for(info.orig_filename, info.file_size)
                    if info.file_size > 4096:
                        raise ValueError("Archive link target is too long")
                    file_link(path, bundle.read(info).decode("utf-8"))
                    continue
                if stat.S_ISLNK(mode) or (stat.S_IFMT(mode) not in (0, stat.S_IFREG, stat.S_IFDIR)):
                    raise ValueError("Archive links and special files are not supported")
                path = path_for(info.orig_filename, info.file_size)
                if info.is_dir():
                    path.mkdir(parents=True, exist_ok=True)
                else:
                    with bundle.open(info) as stream:
                        write(stream, path, info.file_size, mode)
    else:
        with tarfile.open(archive, "r|xz" if format == "tar.xz" else "r|gz") as bundle:
            for info in bundle:
                if (info.issym() or info.islnk()) and allow_file_links:
                    path = path_for(info.name, 0)
                    file_link(path, info.linkname, relative=info.issym())
                    continue
                if not info.isfile() and not info.isdir():
                    raise ValueError("Archive links and special files are not supported")
                path = path_for(info.name, info.size)
                if info.isdir():
                    path.mkdir(parents=True, exist_ok=True)
                else:
                    with bundle.extractfile(info) as stream:
                        write(stream, path, info.size, info.mode)
    # Tool archives contain compiler aliases. Store regular files in the bundle.
    while links:
        pending = []
        for path, target in links:
            checkpoint()
            if not target.is_file():
                pending.append((path, target))
                continue
            size = target.stat().st_size
            total += size
            if total > limit:
                raise ValueError("Archive exceeds extraction limits")
            path.parent.mkdir(parents=True, exist_ok=True)
            copy_file(target, path)
        if len(pending) == len(links):
            raise ValueError("Archive contains unresolved or non-file links")
        links = pending
    checkpoint()


def install_rust_targets(rust, channel, targets, temporary, cache_dir=None):
    missing = [target for target in targets if not (rust / "lib/rustlib" / target / "lib").is_dir()]
    if not missing:
        return
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", channel):
        raise ValueError("Android targets require a pinned Rust version")
    if cache_dir is not None:
        import hub_tools
        still_missing = []
        for target in missing:
            key = hub_tools.rust_target_key(channel, target)
            cached = Path(cache_dir) / key
            if cached.is_dir() and hub_tools.valid_rust_target(cached):
                print(f"Reusing cached Rust target {target} for Rust {channel}...",
                      file=sys.stderr, flush=True)
                checkpoint()
                shutil.copytree(cached, rust / "lib/rustlib" / target, copy_function=copy_file)
                continue
            still_missing.append(target)
        missing = still_missing
        if not missing:
            return
    url = f"https://static.rust-lang.org/dist/channel-rust-{channel}.toml"
    print(f"Checking Android targets for Rust {channel}...", file=sys.stderr, flush=True)
    manifest = read_remote(url)
    checksum = read_remote(url + ".sha256", 1024).decode().split()[0]
    if hashlib.sha256(manifest).hexdigest() != digest(checksum):
        raise ValueError("Rust distribution manifest checksum mismatch")
    data = tomllib.loads(manifest.decode("utf-8"))
    std = data.get("pkg", {}).get("rust-std", {})
    if data.get("manifest-version") != "2" or std.get("version", "").split(" ")[0] != channel:
        raise ValueError("Rust distribution manifest does not match the pinned version")
    for target in missing:
        descriptor = std.get("target", {}).get(target, {})
        if not descriptor.get("available"):
            raise ValueError(f"Rust {channel} does not publish {target}")
        artifact_url = descriptor.get("xz_url", "")
        if urlsplit(artifact_url).hostname != "static.rust-lang.org":
            raise ValueError("Rust component must come from the official distribution host")
        print(f"Installing Rust target {target}...", file=sys.stderr, flush=True)
        archive = temporary / (target + ".tar.xz")
        download(artifact_url, archive, descriptor.get("xz_hash"), limit=256 * 1024**2)
        output = temporary / target
        extract(archive, output, "tar.xz", limit=1024**3)
        matches = [p for p in output.glob(f"*/rust-std-{target}/lib/rustlib/{target}") if (p / "lib").is_dir()]
        if len(matches) != 1 or not any((matches[0] / "lib").iterdir()):
            raise ValueError("Rust component archive has an unexpected layout")
        shutil.copytree(matches[0], rust / "lib/rustlib" / target, copy_function=copy_file)
        if cache_dir is not None:
            import hub_tools
            try:
                hub_tools.store(Path(cache_dir), hub_tools.rust_target_key(channel, target),
                                rust / "lib/rustlib" / target, checkpoint)
            except ValueError:
                pass


def android_versions(repo):
    source = (repo / "blockloom-core/src/android.rs").read_text(encoding="utf-8")
    result = {}
    for name in ("JDK_MAJOR", "PLATFORM", "BUILD_TOOLS", "NDK_MAJOR", "EMULATOR_IMAGE"):
        match = re.search(rf'pub const {name}: [^=]+ = (?:"([^"]+)"|([0-9]+));', source)
        if not match:
            raise ValueError(f"Development repository is missing Android pin: {name}")
        result[name] = match[1] or match[2]
    return result


def default_android_versions():
    """Pinned Android versions for release catalogs that predate android_versions.

    Mirrors blockloom-core/src/android.rs; package-release embeds the same
    pins into new catalogs, so this is only the fallback for 0.0.1 entries.
    """
    return {"JDK_MAJOR": "25", "PLATFORM": "android-35", "BUILD_TOOLS": "35.0.0",
            "NDK_MAJOR": "27", "EMULATOR_IMAGE": "system-images;android-35;google_apis;x86_64"}


def tool_archive(url, checksum, size, temporary, name, algorithm="sha256"):
    format = "zip" if urlsplit(url).path.endswith(".zip") else "tar.gz"
    archive = temporary / (name + "." + format)
    download(url, archive, checksum, size=size, algorithm=algorithm)
    output = temporary / name
    extract(archive, output, format, allow_file_links=True, allow_case_sensitive_paths=True)
    roots = [path for path in output.iterdir() if path.is_dir() and path.name != "__MACOSX"]
    if len(roots) != 1:
        raise ValueError("Tool archive has an unexpected layout")
    return roots[0]


def _reuse_cached(cache, key, destination):
    import hub_tools
    try:
        return hub_tools.reuse(cache, key, destination, checkpoint)
    except ValueError:
        shutil.rmtree(Path(cache) / key, ignore_errors=True)
        return False


def install_android_tools(directory, versions, components, temporary, cache_dir=None):
    selected = {}
    host = {"win32": "windows", "darwin": "macosx"}.get(sys.platform, "linux")
    arch = "aarch64" if platform.machine().lower() in ("arm64", "aarch64") else "x64"
    cache = Path(cache_dir) if cache_dir is not None else None
    if "java" in components:
        java_os = "mac" if host == "macosx" else host
        url = (f"https://api.adoptium.net/v3/assets/latest/{versions['JDK_MAJOR']}/hotspot"
               f"?architecture={arch}&image_type=jdk&os={java_os}&vendor=eclipse")
        print(f"Downloading Java {versions['JDK_MAJOR']}...", file=sys.stderr, flush=True)
        assets = json.loads(read_remote(url))
        if not isinstance(assets, list) or len(assets) != 1:
            raise ValueError("Adoptium did not publish one matching JDK")
        java_version = assets[0].get("version", {}).get("semver")
        if not isinstance(java_version, str) or not java_version:
            raise ValueError("Adoptium package metadata is missing the Java version")
        package = assets[0]["binary"]["package"]
        if urlsplit(package["link"]).hostname != "github.com" or not urlsplit(package["link"]).path.startswith("/adoptium/"):
            raise ValueError("Java package must come from Adoptium")
        if cache is not None:
            import hub_tools
            key = hub_tools.java_key(java_version)
            destination = directory / "tools/java"
            if _reuse_cached(cache, key, destination):
                selected["java"] = {"version": java_version}
            else:
                root = tool_archive(package["link"], package["checksum"], package["size"], temporary, "java-download")
                if (root / "Contents/Home").is_dir():
                    root = root / "Contents/Home"
                shutil.copytree(root, destination, copy_function=copy_file)
                selected["java"] = {"version": java_version}
                try:
                    hub_tools.store(cache, key, destination, checkpoint)
                except ValueError:
                    pass
        else:
            root = tool_archive(package["link"], package["checksum"], package["size"], temporary, "java-download")
            if (root / "Contents/Home").is_dir():
                root = root / "Contents/Home"
            shutil.copytree(root, directory / "tools/java", copy_function=copy_file)
            selected["java"] = {"version": java_version}
    if not any(name in components for name in ("android-sdk", "android-ndk")):
        return selected
    print("Checking official Android packages...", file=sys.stderr, flush=True)
    def repository(base, manifest):
        raw = read_remote(base + manifest, 16 * 1024**2)
        data = ET.fromstring(raw)
        tag = data.tag
        namespaces = {"xmlns:" + prefix: url for _, (prefix, url) in ET.iterparse(io.BytesIO(raw), events=["start-ns"]) if prefix}
        for node in data.iter():
            node.tag = node.tag.rsplit("}", 1)[-1]
        packages = {node.attrib["path"]: node for node in data.findall("remotePackage")
                    if node.find("channelRef") is None or node.find("channelRef").get("ref") == "channel-0"}
        return base, data, tag, namespaces, packages

    main_repository = repository("https://dl.google.com/android/repository/", "repository2-3.xml")
    packages = main_repository[-1]

    def revision(node):
        return tuple(int(node.findtext("revision/" + field, "0")) for field in ("major", "minor", "micro"))

    def install_package(key, destination, label, catalog=main_repository):
        base, data, repository_tag, namespaces, catalog_packages = catalog
        node = catalog_packages.get(key)
        if node is None:
            raise ValueError(f"Android repository does not publish {key}")
        archives = [item for item in node.findall("archives/archive")
                    if item.findtext("host-os", host) == host and item.findtext("host-arch", arch) == arch]
        if len(archives) != 1:
            raise ValueError(f"Android package {key} does not support this host")
        complete = archives[0].find("complete")
        relative = complete.findtext("url", "")
        if not relative or "/" in relative or "\\" in relative or ":" in relative:
            raise ValueError("Android package must come from the official repository")
        checksum = complete.find("checksum")
        print(f"Downloading {key}...", file=sys.stderr, flush=True)
        root = tool_archive(base + relative, checksum.text,
                            int(complete.findtext("size")), temporary, label, checksum.get("type", "sha1"))
        shutil.copytree(root, destination, copy_function=copy_file)
        # SDK Manager needs package metadata to recognize the bundled packages.
        for index, uri in enumerate(dict.fromkeys(namespaces.values())):
            ET.register_namespace("hub_schema_" + str(index), uri)
        local = ET.Element("localPackage", {"path": key})
        for child in node:
            if child.tag not in ("archives", "channelRef"):
                local.append(ET.fromstring(ET.tostring(child)))
        repository = ET.Element(repository_tag, namespaces)
        license_ref = node.find("uses-license")
        if license_ref is not None:
            for license_node in data.findall("license"):
                if license_node.get("id") == license_ref.get("ref"):
                    repository.append(ET.fromstring(ET.tostring(license_node)))
        repository.append(local)
        ET.ElementTree(repository).write(destination / "package.xml", encoding="utf-8", xml_declaration=True)
        return ".".join(map(str, revision(node)))

    def package_revision(key, catalog=main_repository):
        catalog_packages = catalog[-1]
        node = catalog_packages.get(key)
        if node is None:
            raise ValueError(f"Android repository does not publish {key}")
        archives = [item for item in node.findall("archives/archive")
                    if item.findtext("host-os", host) == host and item.findtext("host-arch", arch) == arch]
        if len(archives) != 1:
            raise ValueError(f"Android package {key} does not support this host")
        return ".".join(map(str, revision(node)))

    if "android-sdk" in components:
        sdk = directory / "tools/android-sdk"
        image = versions.get("EMULATOR_IMAGE", f"system-images;{versions['PLATFORM']};google_apis;x86_64")
        image_parts = image.split(";")
        if len(image_parts) != 4 or image_parts[0] != "system-images" or any(not re.fullmatch(r"[a-zA-Z0-9_-]+", part) for part in image_parts):
            raise ValueError("Repository has an invalid emulator image pin")
        sdk_keys = (("cmdline-tools;latest", "cmdline-tools/latest", "command-tools"),
                    ("platform-tools", "platform-tools", "platform-tools"),
                    ("platforms;" + versions["PLATFORM"], "platforms/" + versions["PLATFORM"], "platform"),
                    ("build-tools;" + versions["BUILD_TOOLS"], "build-tools/" + versions["BUILD_TOOLS"], "build-tools"),
                    ("emulator", "emulator", "emulator"))
        sdk_key_value = None
        sdk_fingerprint = None
        image_repository = None
        if cache is not None:
            import hub_tools
            image_repository = repository(f"https://dl.google.com/android/repository/sys-img/{image_parts[2]}/", "sys-img2-3.xml")
            versions_preview = {}
            for key, _relative, _label in sdk_keys:
                versions_preview[key] = package_revision(key)
            versions_preview[image] = package_revision(image, image_repository)
            sdk_key_value = hub_tools.sdk_key(versions_preview, versions_preview["platform-tools"])
            if _reuse_cached(cache, sdk_key_value, sdk):
                selected["android-sdk"] = {"version": versions_preview["platform-tools"],
                                           "packages": versions_preview}
                sdk_fingerprint = versions_preview
        if sdk_fingerprint is None:
            versions_installed = {}
            for key, relative, label in sdk_keys:
                versions_installed[key] = install_package(key, sdk / relative, label)
            if image_repository is None:
                image_repository = repository(f"https://dl.google.com/android/repository/sys-img/{image_parts[2]}/", "sys-img2-3.xml")
            versions_installed[image] = install_package(image, sdk.joinpath(*image_parts), "system-image", image_repository)
            selected["android-sdk"] = {"version": versions_installed["platform-tools"], "packages": versions_installed}
            if cache is not None:
                import hub_tools
                try:
                    hub_tools.store(cache, hub_tools.sdk_key(versions_installed, versions_installed["platform-tools"]),
                                    sdk, checkpoint)
                except ValueError:
                    pass
    if "android-ndk" in components:
        ndks = [key for key in packages if key.startswith("ndk;" + versions["NDK_MAJOR"] + ".")]
        if not ndks:
            raise ValueError("Android repository does not publish the pinned NDK major")
        key = max(ndks, key=lambda item: revision(packages[item]))
        if cache is not None:
            import hub_tools
            preview = ".".join(map(str, revision(packages[key])))
            ndk_key_value = hub_tools.ndk_key(preview)
            destination = directory / "tools/android-ndk"
            if _reuse_cached(cache, ndk_key_value, destination):
                selected["android-ndk"] = {"version": preview}
            else:
                selected["android-ndk"] = {"version": install_package(key, destination, "ndk")}
                try:
                    hub_tools.store(cache, ndk_key_value, destination, checkpoint)
                except ValueError:
                    pass
        else:
            selected["android-ndk"] = {"version": install_package(key, directory / "tools/android-ndk", "ndk")}
    return selected
