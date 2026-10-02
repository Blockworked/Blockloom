"""Use the user's GitHub CLI session for private release catalogs and assets."""

import json
from pathlib import Path
import re
import shutil
import tempfile

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


def asset(repo, tag, name, destination):
    repository(repo)
    if not isinstance(tag, str) or not tag or tag.startswith("-"):
        raise ValueError("Invalid GitHub release tag")
    if not isinstance(name, str) or not re.fullmatch(r"[A-Za-z0-9_.-]+", name):
        raise ValueError("Invalid GitHub asset name")
    if destination.exists():
        raise ValueError("Asset download destination already exists")
    run([cli(), "release", "download", tag, "--repo", repo, "--pattern", name,
         "--output", str(destination)])
    checkpoint()


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
