"""Create cancellable project snapshots before explicit editor changes."""

import os
from pathlib import Path
import stat
import sys
import time
import zipfile

from hub_process import checkpoint, CopyProgress

EXCLUDED = {".git", ".svn", ".blockloom/build", ".blockloom/lock.json"}


def inventory(project):
    def failed(error):
        raise error

    entries = {}
    for directory, folders, files in os.walk(project, followlinks=False, onerror=failed):
        checkpoint()
        parent = Path(directory)
        for name in [*folders, *files]:
            path = parent / name
            relative = path.relative_to(project).as_posix()
            if relative in EXCLUDED:
                if name in folders:
                    folders.remove(name)
                continue
            info = path.lstat()
            if stat.S_ISLNK(info.st_mode) or getattr(info, "st_file_attributes", 0) & 0x400:
                raise ValueError(f"Project backup does not follow links: {relative}")
            if not stat.S_ISREG(info.st_mode) and not stat.S_ISDIR(info.st_mode):
                raise ValueError(f"Project backup cannot include special files: {relative}")
            entries[relative] = (info.st_mode, info.st_size, info.st_mtime_ns, info.st_ino)
    return entries


def create(project, destination, idle):
    if destination.resolve().is_relative_to(project.resolve()):
        raise ValueError("Project backups must be stored outside the project folder")
    idle()
    entries = inventory(project)
    files = [name for name, info in entries.items() if stat.S_ISREG(info[0])]
    progress = CopyProgress("Backing up project", len(files), sum(entries[name][1] for name in files))
    created = False
    try:
        with zipfile.ZipFile(destination, "x", compression=zipfile.ZIP_DEFLATED, compresslevel=3) as archive:
            created = True
            for name in sorted(entries):
                checkpoint()
                path = project / name
                if stat.S_ISDIR(entries[name][0]):
                    archive.write(path, name)
                    continue
                info = zipfile.ZipInfo.from_file(path, name)
                info.compress_type = zipfile.ZIP_DEFLATED
                with path.open("rb") as reader, archive.open(info, "w", force_zip64=True) as output:
                    while chunk := reader.read(1024 * 1024):
                        checkpoint()
                        output.write(chunk)
                        progress(len(chunk), False)
                progress(0, True)
        checkpoint()
        idle()
        if inventory(project) != entries:
            raise ValueError("Project files changed during backup. Try again with the editor closed")
        print("Project backup complete.", file=sys.stderr, flush=True)
        return {"path": str(destination), "files": len(files), "created_at": int(time.time())}
    except BaseException:
        if created:
            destination.unlink(missing_ok=True)
        raise
