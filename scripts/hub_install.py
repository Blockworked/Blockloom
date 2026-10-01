"""Portable installation staging shared by the Hub and replacement builds."""

import json
import os
from pathlib import Path
import shutil
import tempfile


MANIFEST = "blockloom-installation.json"


def editor_name():
    return "blockloom.exe" if os.name == "nt" else "blockloom"


def validate_payload(directory):
    suffix = ".exe" if os.name == "nt" else ""
    for name in ("blockloom", "blockloom-runtime"):
        binary = directory / (name + suffix)
        if not binary.is_file() or binary.is_symlink():
            raise ValueError(f"Missing regular binary: {binary}")


def stage(source, destination, manifest=None, replace=False, build_output=False, excluded=()):
    source, destination = Path(source).resolve(), Path(destination).absolute()
    validate_payload(source)
    if (source == destination.resolve() or source in destination.resolve().parents
            or destination.resolve() in source.parents):
        raise ValueError("Installation source and destination cannot contain each other")
    if destination.is_symlink():
        raise ValueError("Installation destination cannot be a symlink")
    if destination.exists() and not replace:
        raise ValueError(f"Installation already exists: {destination}")
    if destination.exists() and (destination / MANIFEST).is_file():
        existing = json.loads((destination / MANIFEST).read_text(encoding="utf-8"))
        if existing.get("kind") == "release":
            raise ValueError("Released installations cannot be replaced")
        if existing.get("kind") != "dev":
            raise ValueError("Unrecognized installation manifest")
    elif destination.exists():
        validate_payload(destination)
    destination.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".stage-", dir=destination.parent) as temporary:
        payload = Path(temporary) / "payload"
        payload.mkdir()

        def ignore(directory, names):
            relative = Path(directory).relative_to(source)
            return [name for name in names if (relative / name).as_posix() in excluded]

        for item in source.iterdir():
            if build_output and not (
                item.name in ("blockloom", "blockloom.exe", "blockloom-runtime",
                              "blockloom-runtime.exe", "blockloom-shell", "blockloom-shell.exe",
                              "players", "plugins", "qml", "qt.conf")
                or item.suffix in (".dll", ".dylib") or ".so" in item.name
            ):
                continue
            if item.is_symlink() or (item.is_dir() and any(p.is_symlink() for p in item.rglob("*"))):
                raise ValueError(f"Bundle links are not supported: {item}")
            if item.is_dir():
                shutil.copytree(item, payload / item.name, ignore=ignore)
            else:
                shutil.copy2(item, payload / item.name)
        if manifest is not None:
            (payload / MANIFEST).write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
        validate_payload(payload)
        backup = Path(temporary) / "previous"
        if destination.exists():
            destination.rename(backup)
        try:
            payload.rename(destination)
        except BaseException:
            if backup.exists():
                backup.rename(destination)
            raise
