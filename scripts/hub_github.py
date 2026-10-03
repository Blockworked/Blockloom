"""Use the user's GitHub CLI session for private release catalogs and assets."""

import json
from pathlib import Path
import re
import shutil
import sys
import tempfile
import threading

from hub_process import run, checkpoint

DEFAULT_REPO = "Blockworked/Blockloom"
CATALOG_ASSET = "blockloom-catalog.json"


def repository(value):
    if not isinstance(value, str) or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]*/[A-Za-z0-9][A-Za-z0-9_.-]*", value):
        raise ValueError("GitHub repository must be owner/repo")
    return value


def cli():
    executable = shutil.which("gh")
    if not executable:
        raise ValueError("Install GitHub CLI and run gh auth login before checking private releases")
    return executable


def capture(arguments, limit=4 * 1024**2):
    with tempfile.TemporaryFile() as output:
        run([cli(), *arguments], stdout=output)
        output.seek(0)
        data = output.read(limit + 1)
    if len(data) > limit:
        raise ValueError("GitHub response exceeds the size limit")
    return json.loads(data)


def asset(repo, tag, name, destination, size=None):
    repository(repo)
    if not isinstance(tag, str) or not tag or tag.startswith("-"):
        raise ValueError("Invalid GitHub release tag")
    if not isinstance(name, str) or not re.fullmatch(r"[A-Za-z0-9_.-]+", name):
        raise ValueError("Invalid GitHub asset name")
    if destination.exists():
        raise ValueError("Asset download destination already exists")
    if size is not None:
        print(f"Downloading {name} via GitHub CLI ({size / 1024**2:.0f} MiB expected)...",
              file=sys.stderr, flush=True)
    else:
        print(f"Downloading {name} via GitHub CLI...", file=sys.stderr, flush=True)
    # `gh release download` reports no byte progress itself, so poll the
    # growing file while it runs. Without this a 693 MiB editor download
    # leaves the Hub operation log empty for minutes.
    done = threading.Event()

    def watch():
        last = 0
        while not done.wait(1.0):
            try:
                current = destination.stat().st_size if destination.exists() else 0
            except OSError:
                current = 0
            if current != last and current > 0:
                if size:
                    print(f"Downloading: {current / 1024**2:.1f} MiB / {size / 1024**2:.1f} MiB",
                          file=sys.stderr, flush=True)
                else:
                    print(f"Downloading: {current / 1024**2:.1f} MiB",
                          file=sys.stderr, flush=True)
                last = current

    watcher = threading.Thread(target=watch, daemon=True)
    watcher.start()
    try:
        run([cli(), "release", "download", tag, "--repo", repo, "--pattern", name,
             "--output", str(destination)])
    finally:
        done.set()
        watcher.join(timeout=5)
    checkpoint()
    try:
        final = destination.stat().st_size
    except OSError:
        final = 0
    print(f"Downloaded {name} ({final / 1024**2:.1f} MiB).", file=sys.stderr, flush=True)


def catalog(repo, target):
    import hub_download
    repo = repository(repo)
    releases = capture(["api", f"repos/{repo}/releases?per_page=100"])
    if not isinstance(releases, list):
        raise ValueError("Unexpected GitHub releases response")
    entries, seen = [], set()
    for release in releases:
        checkpoint()
        if release.get("draft"):
            continue
        catalogs = [a for a in release.get("assets", []) if a.get("name") == CATALOG_ASSET]
        if not catalogs:
            continue
        if catalogs[0].get("size", 0) > 2 * 1024**2:
            raise ValueError("GitHub release catalog exceeds the size limit")
        with tempfile.TemporaryDirectory(prefix="hub-catalog-") as temporary:
            path = Path(temporary) / CATALOG_ASSET
            asset(repo, release["tag_name"], CATALOG_ASSET, path)
            if path.stat().st_size > 2 * 1024**2:
                raise ValueError("GitHub release catalog exceeds the size limit")
            selected = hub_download.catalog_data(json.loads(path.read_bytes()), target)
        assets = {a["name"]: a for a in release.get("assets", [])}
        for entry in selected:
            name = entry.get("asset")
            if name not in assets or assets[name].get("size") != entry["size"]:
                raise ValueError("Catalog asset is missing or has a different size")
            identity = entry["version"]
            if identity in seen:
                raise ValueError("GitHub releases contain duplicate Blockloom versions")
            seen.add(identity)
            entries.append({**entry, "github": {"repo": repo, "tag": release["tag_name"], "asset": name}})
    return entries
