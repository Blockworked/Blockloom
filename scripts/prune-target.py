#!/usr/bin/env python3
"""Keep Cargo's target tree within a disk budget without touching active builds."""

import argparse
from contextlib import ExitStack
import math
import os
from pathlib import Path
import shutil
import subprocess
import sys

try:
    import fcntl
except ImportError:
    fcntl = None
if fcntl is None and os.name == "nt":
    import msvcrt


GIB = 1024 ** 3
CACHES = {"incremental", "deps", "build", ".fingerprint", "examples"}


def _remove_tree(path):
    # Windows refuses to delete read-only files; Cargo caches shouldn't have
    # any, but a stray one must not abort the cleanup.
    if os.name == "nt":
        shutil.rmtree(path, onerror=_readonly_handler)
    else:
        shutil.rmtree(path)


def _readonly_handler(function, path, excinfo):
    os.chmod(path, 0o666)
    function(path)


def allocated(info):
    # st_blocks is Unix-only; elsewhere the logical size is close enough for
    # a cache budget.
    return info.st_blocks * 512 if hasattr(info, "st_blocks") else info.st_size


def usage(path):
    size = 0
    newest = 0
    seen = set()
    for directory, dirs, files in os.walk(path):
        if os.name != "nt":
            # NTFS reports a directory's entry list as its size, which shifts
            # whenever files come and go; it is not cache cost worth counting.
            try:
                size += allocated(Path(directory).stat())
            except FileNotFoundError:
                continue
        dirs[:] = [name for name in dirs if not (Path(directory) / name).is_symlink()]
        for name in files:
            try:
                info = (Path(directory) / name).lstat()
            except FileNotFoundError:
                continue
            key = (info.st_dev, info.st_ino)
            if key not in seen:
                size += allocated(info)
                seen.add(key)
            newest = max(newest, info.st_mtime)
    return size, newest


def profiles(roots):
    found = set()
    for root in roots:
        for directory, dirs, _ in os.walk(root):
            dirs[:] = [name for name in dirs if not (Path(directory) / name).is_symlink()]
            if "incremental" in dirs or ".fingerprint" in dirs:
                found.add(Path(directory))
            dirs[:] = [name for name in dirs if name not in CACHES | {"players", "cxxqt"}]
    return sorted(found)


def lock_profile(profile, stack):
    handles = []
    try:
        # Current Cargo uses the build lock; older Cargo uses .cargo-lock.
        for name in (".cargo-lock", ".cargo-build-lock"):
            handle = (profile / name).open("a+b")
            handles.append(handle)
            if fcntl is not None:
                fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
            else:
                # msvcrt locks a byte range; Cargo's whole-file lock overlaps
                # it, and so does another probe, so a held lock reads as busy.
                handle.seek(0)
                msvcrt.locking(handle.fileno(), msvcrt.LK_NBLCK, 1)
    except BlockingIOError:
        for handle in handles:
            handle.close()
        return False
    except OSError:
        for handle in handles:
            handle.close()
        if fcntl is None:
            # Windows reports lock contention as OSError, not BlockingIOError.
            return False
        raise
    except BaseException:
        for handle in handles:
            handle.close()
        raise
    for handle in handles:
        stack.enter_context(handle)
    return True


def prune(target, limit, dry_run=False, keep_profiles=()):
    roots = {target}
    if os.environ.get("CARGO_BUILD_BUILD_DIR"):
        roots.add(Path(os.environ["CARGO_BUILD_BUILD_DIR"]).resolve())
    # A custom build directory outside target gets its own share of the budget.
    roots = {root for root in roots if not any(root != other and root.is_relative_to(other) for other in roots)}
    total = sum(usage(root)[0] for root in roots)
    if total <= limit:
        if dry_run:
            print(f"Target cache: {total / GIB:.2f} GiB (limit {limit / GIB:g} GiB); no cleanup needed.")
        return
    busy = []
    with ExitStack() as stack:
        idle = []
        for profile in profiles(roots):
            if lock_profile(profile, stack):
                idle.append(profile)
            else:
                busy.append(profile)
        incremental = []
        for profile in idle:
            directory = profile / "incremental"
            if not directory.is_dir() or directory.is_symlink():
                continue
            for crate in directory.iterdir():
                if crate.is_dir() and not crate.is_symlink():
                    size, newest = usage(crate)
                    incremental.append((newest, crate, size))
        remaining = total
        for _, crate, size in sorted(incremental):
            if remaining <= limit:
                break
            print(f"{'Would remove' if dry_run else 'Removing'} incremental cache: {crate}")
            if not dry_run:
                _remove_tree(crate)
            remaining = remaining - size if dry_run else sum(usage(root)[0] for root in roots)
        if remaining > limit:
            old_profiles = []
            for profile in idle:
                if profile.name in keep_profiles:
                    continue
                caches = [profile / name for name in CACHES - {"incremental"}
                          if (profile / name).is_dir() and not (profile / name).is_symlink()]
                measured = [usage(cache) for cache in caches]
                old_profiles.append((max((stamp for _, stamp in measured), default=0), profile, caches))
            for _, profile, caches in sorted(old_profiles):
                if remaining <= limit:
                    break
                print(f"{'Would remove' if dry_run else 'Removing'} profile caches: {profile}")
                for cache in caches:
                    if not dry_run:
                        _remove_tree(cache)
                remaining = (remaining - sum(usage(cache)[0] for cache in caches)) if dry_run else sum(usage(root)[0] for root in roots)
        if not dry_run:
            remaining = sum(usage(root)[0] for root in roots)
        print(f"Target cache: {total / GIB:.2f} GiB -> {remaining / GIB:.2f} GiB (limit {limit / GIB:g} GiB)")
        if remaining > limit:
            reason = "active builds" if busy else "protected outputs"
            print(f"Still above budget because of {reason}; cleanup will retry next time.", file=sys.stderr)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--limit-gib", type=float, default=os.environ.get("BLOCKLOOM_TARGET_LIMIT_GIB", "20"))
    parser.add_argument("--target-dir", type=Path)
    parser.add_argument("--keep-profile", action="append", default=[])
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--run", nargs=argparse.REMAINDER, help="Run a command between cleanups")
    args = parser.parse_args()
    if not math.isfinite(args.limit_gib) or args.limit_gib <= 0:
        parser.error("--limit-gib must be a positive finite number")
    if args.run is not None and (not args.run or args.dry_run):
        parser.error("--run needs a command and cannot be combined with --dry-run")
    target = (args.target_dir or Path(os.environ.get("CARGO_TARGET_DIR", "target"))).resolve()
    if args.run and args.target_dir is None:
        for index, arg in enumerate(args.run):
            if arg == "--target-dir" and index + 1 < len(args.run):
                target = Path(args.run[index + 1]).resolve()
            elif arg.startswith("--target-dir="):
                target = Path(arg.split("=", 1)[1]).resolve()
    limit = int(args.limit_gib * GIB)
    prune(target, limit, args.dry_run, args.keep_profile)
    if args.run:
        try:
            status = subprocess.run(args.run).returncode
            return status if status >= 0 else 128 - status
        finally:
            prune(target, limit, keep_profiles=args.keep_profile)
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        sys.exit(130)
    except OSError as error:
        sys.exit(f"prune-target: {error}")
