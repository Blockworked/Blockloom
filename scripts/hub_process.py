"""Cancellable file copies for release packaging (the Hub has its own in Rust)."""

import os
from pathlib import Path
import sys
import time


class Cancelled(Exception):
    pass


def checkpoint():
    if path := os.environ.get("BLOCKLOOM_HUB_CANCEL_FILE"):
        if Path(path).exists():
            raise Cancelled("Operation cancelled; previous installation kept")


def copy_file(source, destination, progress=None):
    """Check between bounded copy chunks, then preserve file metadata."""
    import shutil
    with open(source, "rb") as reader, open(destination, "wb") as writer:
        while True:
            checkpoint()
            chunk = reader.read(1024 * 1024)
            if not chunk:
                break
            writer.write(chunk)
            if progress:
                progress(len(chunk), False)
    shutil.copystat(source, destination)
    if progress:
        progress(0, True)
    return destination


class CopyProgress:
    def __init__(self, label, files, size):
        self.label, self.files, self.size = label, files, size
        self.copied, self.finished, self.last = 0, 0, 0
        self(0, False)

    def __call__(self, size, finished):
        self.copied += size
        self.finished += int(finished)
        now = time.monotonic()
        if now - self.last >= 1 or self.finished == self.files:
            print(f"{self.label}: {self.finished}/{self.files} files, "
                  f"{self.copied / 1024**2:.0f}/{self.size / 1024**2:.0f} MiB", file=sys.stderr, flush=True)
            self.last = now


def bundle_rust(source, destination, channel, exclude_targets=()):
    import shutil
    roots = [source / name for name in ("bin", "lib", "libexec", "etc") if (source / name).is_dir()]

    def ignore(directory, names):
        checkpoint()
        return [name for name in names if name == "src" or name in exclude_targets] if Path(directory) == source / "lib" / "rustlib" else []

    files = []
    for root in roots:
        for directory, directories, names in os.walk(root):
            checkpoint()
            directories[:] = [name for name in directories if name not in ignore(directory, directories)]
            files.extend(Path(directory) / name for name in names)
    progress = CopyProgress(f"Bundling Rust {channel}", len(files), sum(p.stat().st_size for p in files))
    destination.mkdir(parents=True)
    for root in roots:
        shutil.copytree(root, destination / root.name, ignore=ignore,
                        copy_function=lambda src, dst: copy_file(src, dst, progress))
