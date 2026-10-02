"""Bounded HTTPS downloads and archive extraction for Hub installations."""

import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import stat
import sys
import tarfile
import tempfile
import time
import tomllib
from urllib.parse import urlsplit
from urllib.request import HTTPRedirectHandler, Request, build_opener
import zipfile

from hub_process import checkpoint, copy_file

MAX_ARCHIVE = 8 * 1024**3
MAX_EXTRACTED = 24 * 1024**3
MAX_MEMBERS = 100000


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


def response(url):
    checkpoint()
    return build_opener(SecureRedirect()).open(Request(secure_url(url), headers={
        "User-Agent": "Blockloom-Hub", "Accept-Encoding": "identity"}), timeout=5)


def read_remote(url, limit=2 * 1024**2):
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
    return catalog_data(json.loads(read_remote(url)), target)


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


def download(url, destination, sha256, size=None, limit=MAX_ARCHIVE):
    expected = digest(sha256)
    amount, last, hash_value = 0, 0, hashlib.sha256()
    created = False
    try:
        with response(url) as stream, destination.open("xb") as output:
            created = True
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
            raise ValueError("Download size or SHA-256 does not match the manifest")
        checkpoint()
    except BaseException:
        if created:
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


def extract(archive, destination, format, limit=MAX_EXTRACTED):
    destination.mkdir(parents=True)
    names, total, count, last = set(), 0, 0, 0

    def path_for(name, size):
        nonlocal count, total, last
        checkpoint()
        parts = PurePosixPath(name).parts
        if not parts or name.startswith("/") or "\\" in name or ":" in name or ".." in parts:
            raise ValueError("Archive contains an unsafe path")
        if any(p.endswith((".", " ")) or p.split(".")[0].upper() in ("CON", "PRN", "AUX", "NUL", *[f"COM{i}" for i in range(1, 10)], *[f"LPT{i}" for i in range(1, 10)]) for p in parts):
            raise ValueError("Archive contains a nonportable path")
        key = "/".join(parts).casefold()
        if key in names:
            raise ValueError("Archive contains duplicate paths")
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
                if not info.isfile() and not info.isdir():
                    raise ValueError("Archive links and special files are not supported")
                path = path_for(info.name, info.size)
                if info.isdir():
                    path.mkdir(parents=True, exist_ok=True)
                else:
                    with bundle.extractfile(info) as stream:
                        write(stream, path, info.size, info.mode)
    checkpoint()


def install_rust_targets(rust, channel, targets, temporary):
    missing = [target for target in targets if not (rust / "lib/rustlib" / target / "lib").is_dir()]
    if not missing:
        return
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", channel):
        raise ValueError("Android targets require a pinned Rust version")
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
