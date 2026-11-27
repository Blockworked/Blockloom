#!/usr/bin/env python3
"""Frame times of the templates on this machine's GPU.

Each entry is built like the corpus does, then the built player runs with
BLOCKLOOM_PROFILE_SECONDS, which makes it sample every frame after a warm-up
and write a summary (frame, GPU, per-pass and fixed-step times, the adapter)
before quitting. Use a release build: a debug runtime measures the debug
build, not the game.

    just build && python3 scripts/perf.py [--label NAME] [--seconds 10] [entry ...]

Summaries land in tests/perf/<label>/<entry>.json, with a table printed.
"""

import argparse
import json
import os
import pathlib
import platform
import shutil
import subprocess
import sys
import tempfile

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from corpus import ENTRIES, open_entry  # noqa: E402
from smoke import Failed, Shell  # noqa: E402

ROOT = pathlib.Path(__file__).resolve().parent.parent
RESULTS = ROOT / "tests" / "perf"

PERF_ENTRIES = dict(ENTRIES)
PERF_ENTRIES["third-person"] = ("sample", ("third-person", "ThreeD"))


def release_shell():
    target = pathlib.Path(os.environ.get("CARGO_TARGET_DIR") or ROOT / "target")
    for profile in ("release", "debug"):
        exe = target / profile / ("blockloom-shell.exe" if os.name == "nt" else "blockloom-shell")
        if exe.exists():
            return exe, profile
    return None, None


def profile(shell, work, name, seconds, warmup, env, timeout):
    out = work / f"build-{name}"
    shell.run(f'build-game path="{out}" fast=false')
    folder = next(p for p in out.iterdir() if p.is_dir())
    exe = next(p for p in folder.iterdir() if p.is_file() and os.access(p, os.X_OK) and p.suffix in ("", ".exe"))
    summary = work / f"{name}.json"
    run_env = dict(env, BLOCKLOOM_PROFILE_SECONDS=str(seconds), BLOCKLOOM_PROFILE_WARMUP=str(warmup),
                   BLOCKLOOM_PROFILE_OUT=str(summary))
    log = work / f"{name}.log"
    with open(log, "w") as out_log:
        try:
            code = subprocess.run([str(exe)], env=run_env, stdout=out_log, stderr=subprocess.STDOUT,
                                  timeout=timeout).returncode
        except subprocess.TimeoutExpired:
            raise Failed(f"{name}: no summary after {timeout}s\n{log.read_text(errors='replace')[-2000:]}")
    if code != 0 or not summary.exists():
        raise Failed(f"{name}: player exited {code} without a summary\n{log.read_text(errors='replace')[-2000:]}")
    return json.loads(summary.read_text())


def cell(stats, key):
    return f"{stats[key]:.2f}" if stats else "-"


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("entries", nargs="*")
    parser.add_argument("--label", default=platform.node() or "machine")
    parser.add_argument("--seconds", type=float, default=10.0)
    parser.add_argument("--warmup", type=float, default=3.0)
    parser.add_argument("--timeout", type=float, default=600.0)
    parser.add_argument("--shell", type=pathlib.Path, default=None)
    parser.add_argument("--adapter", help="GPU to use, by part of its name (WGPU_ADAPTER_NAME), e.g. intel")
    parser.add_argument("--vsync", action="store_true", help="keep vsync on (frame times then show the display)")
    parser.add_argument("--keep", action="store_true")
    args = parser.parse_args()

    exe, build = (args.shell, "given") if args.shell else release_shell()
    if exe is None:
        sys.exit("No blockloom-shell: run `just build` first, or pass --shell.")
    if build == "debug":
        print("warning: only a debug shell was found, so this measures the debug runtime")
    chosen = args.entries or list(PERF_ENTRIES)
    work = pathlib.Path(tempfile.mkdtemp(prefix="blockloom-perf-"))
    env = dict(os.environ)
    env.setdefault("BLOCKLOOM_DATA_DIR", str(work / "data"))
    env.setdefault("BLOCKLOOM_RUNTIME", "process")
    if args.adapter:
        env["WGPU_ADAPTER_NAME"] = args.adapter
    if args.vsync:
        env["BLOCKLOOM_PROFILE_VSYNC"] = "1"
    target = RESULTS / args.label
    target.mkdir(parents=True, exist_ok=True)

    rows, failures = [], 0
    for name in chosen:
        if name not in PERF_ENTRIES:
            sys.exit(f"Unknown entry {name}: pick from {', '.join(PERF_ENTRIES)}")
        how, what = PERF_ENTRIES[name]
        shell = Shell(exe, False, env)
        try:
            open_entry(shell, work, name, how, what)
            got = profile(shell, work, name, args.seconds, args.warmup, env, args.timeout)
            got["build"] = build
            (target / f"{name}.json").write_text(json.dumps(got, indent=2, sort_keys=True) + "\n")
            rows.append((name, got))
        except Failed as e:
            failures += 1
            print(f"FAIL  {e}")
        finally:
            shell.close()
            # Each build carries a copy of the runtime; drop it before the next.
            shutil.rmtree(work / f"build-{name}", ignore_errors=True)

    if rows:
        adapter = rows[0][1].get("adapter") or {}
        print(f"\n{adapter.get('name', 'unknown GPU')} ({adapter.get('backend', '?')}), "
              f"{rows[0][1].get('resolution')}, {build} build, vsync {'on' if args.vsync else 'off'}\n")
        print("| entry | fps | frame mean | p95 | p99 | GPU mean | GPU p95 | fixed mean |")
        print("|---|---|---|---|---|---|---|---|")
        for name, got in rows:
            f, g, s = got.get("frame_ms"), got.get("gpu_ms"), got.get("fixed_ms")
            print(f"| {name} | {got['fps']:.0f} | {cell(f, 'mean')} | {cell(f, 'p95')} | {cell(f, 'p99')} | "
                  f"{cell(g, 'mean')} | {cell(g, 'p95')} | {cell(s, 'mean')} |")
        print(f"\nsummaries: {target}")
    if args.keep or failures:
        print(f"work folder: {work}")
    else:
        shutil.rmtree(work, ignore_errors=True)
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
