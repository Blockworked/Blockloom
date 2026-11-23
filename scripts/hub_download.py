"""Release catalog checks for packaging. The Hub downloads in Rust
(`blockloom-hub-state/src/fetch.rs`), with the same rules."""

import hashlib
from pathlib import Path
import re
from urllib.parse import urlsplit

MAX_ARCHIVE = 8 * 1024**3
MAX_EXTRACTED = 24 * 1024**3


def checkpoint():
    pass

def secure_url(url):
    if not isinstance(url, str):
        raise ValueError("Download URL must be a string")
    parsed = urlsplit(url)
    if parsed.scheme != "https" or not parsed.hostname or parsed.username or parsed.password or parsed.fragment:
        raise ValueError("Downloads require an HTTPS URL without credentials or fragments")
    return url


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


def verify_file(path, sha256, size):
    if path.stat().st_size != size or size > MAX_ARCHIVE:
        raise ValueError("Downloaded asset size does not match the catalog")
    checksum = hashlib.sha256()
    with path.open("rb") as stream:
        while chunk := stream.read(1024 * 1024):
            checksum.update(chunk)
    if checksum.hexdigest() != digest(sha256):
        raise ValueError("Downloaded asset SHA-256 does not match the catalog")


def android_versions(repo):
    source = (repo / "blockloom-core/src/android.rs").read_text(encoding="utf-8")
    result = {}
    for name in ("JDK_MAJOR", "PLATFORM", "BUILD_TOOLS", "NDK_MAJOR", "EMULATOR_IMAGE"):
        match = re.search(rf'pub const {name}: [^=]+ = (?:"([^"]+)"|([0-9]+));', source)
        if not match:
            raise ValueError(f"Development repository is missing Android pin: {name}")
        result[name] = match[1] or match[2]
    return result
