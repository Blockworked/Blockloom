#!/usr/bin/env python3
"""The playable corpus: real projects played to a fixed tick and compared.

Each entry is built for this machine through blockloom-shell, then the built
player runs with BLOCKLOOM_RECORD_TICKS, which makes it write every actor's
settled pose and the global variables after that many fixed ticks and quit.
The snapshot is compared with the blessed one in tests/corpus/ within a
tolerance, so a refactor that changes how a game plays fails here.

    python3 scripts/corpus.py [--bless] [--project NAME=FOLDER ...] [entry ...]

Built-in entries are made from the samples (the golden fixtures name scripts
they don't carry, so they can't build); --project
adds a project folder from elsewhere (Emberwatch, Voxelvale) for the nightly
run, compared against tests/corpus/NAME.json when it exists.
"""

import argparse
import json
import os
import pathlib
import shutil
import subprocess
import sys
import tempfile

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from smoke import Failed, Shell, actors, find_shell  # noqa: E402

ROOT = pathlib.Path(__file__).resolve().parent.parent
BLESSED = ROOT / "tests" / "corpus"

# Name -> how to get the project open: a sample in a dimension, or a folder to copy.
ENTRIES = {
    "playground-3d": ("sample", ("physics-playground", "ThreeD")),
    "playground-2d": ("sample", ("physics-playground", "TwoD")),
    "platformer": ("sample", ("platformer", "TwoD")),
    "top-down": ("sample", ("top-down", "TwoD")),
    "first-person": ("sample", ("first-person", "ThreeD")),
    "ui-puzzle": ("sample", ("ui-puzzle", "TwoD")),
}


def open_entry(shell, work, name, how, what):
    """Open the entry and answer a stable label per actor id: a sample makes fresh ids each time."""
    if how == "sample":
        sample, mode = what
        reply = shell.run(f'create-project name="Corpus {name}" location="{work}" mode={mode} sample={sample}')
    else:
        copy = work / name
        shutil.copytree(what, copy)
        reply = shell.run(f'open-project path="{copy}" other_editor=true')
    labels, seen = {}, {}
    for actor in actors(reply["state"]):
        n = seen[actor["name"]] = seen.get(actor["name"], 0) + 1
        labels[actor["id"]] = actor["name"] if n == 1 else f"{actor['name']} #{n}"
    return labels


def record(shell, work, name, ticks, env, timeout):
    out = work / f"build-{name}"
    shell.run(f'build-game path="{out}" fast=false')
    folder = next(p for p in out.iterdir() if p.is_dir())
    exe = next(p for p in folder.iterdir() if p.is_file() and os.access(p, os.X_OK) and p.suffix in ("", ".exe"))
    snapshot = work / f"{name}.json"
    run_env = dict(env, BLOCKLOOM_RECORD_TICKS=str(ticks), BLOCKLOOM_RECORD_OUT=str(snapshot))
    log = work / f"{name}.log"
    with open(log, "w") as out_log:
        try:
            code = subprocess.run([str(exe)], env=run_env, stdout=out_log, stderr=subprocess.STDOUT, timeout=timeout).returncode
        except subprocess.TimeoutExpired:
            raise Failed(f"{name}: no snapshot after {timeout}s\n{log.read_text(errors='replace')[-2000:]}")
    if code != 0 or not snapshot.exists():
        raise Failed(f"{name}: player exited {code} without a snapshot\n{log.read_text(errors='replace')[-2000:]}")
    return json.loads(snapshot.read_text())


def compare(name, got, want, tolerance, degrees):
    problems = []
    want_actors = {a["id"]: a for a in want["actors"]}
    got_actors = {a["id"]: a for a in got["actors"]}
    for missing in sorted(set(want_actors) - set(got_actors)):
        problems.append(f"actor {missing} is gone")
    for extra in sorted(set(got_actors) - set(want_actors)):
        problems.append(f"actor {extra} is new")
    worst = 0.0
    for id_, a in want_actors.items():
        b = got_actors.get(id_)
        if not b:
            continue
        moved = max(abs(x - y) for x, y in zip(a["position"], b["position"]))
        turned = max(min(abs(x - y) % 360, 360 - abs(x - y) % 360) for x, y in zip(a["rotation"], b["rotation"]))
        worst = max(worst, moved)
        if moved > tolerance:
            problems.append(f"{id_} is {moved:.4f} from {a['position']} (at {b['position']})")
        if turned > degrees:
            problems.append(f"{id_} is turned {turned:.2f} degrees from {a['rotation']}")
    if got.get("globals") != want.get("globals"):
        problems.append(f"globals differ: {want.get('globals')} -> {got.get('globals')}")
    if problems:
        raise Failed(f"{name}:\n  " + "\n  ".join(problems[:20]))
    return worst


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("entries", nargs="*")
    parser.add_argument("--project", action="append", default=[], metavar="NAME=FOLDER")
    parser.add_argument("--bless", action="store_true", help="write the snapshots as the new expectation")
    parser.add_argument("--ticks", type=int, default=240)
    parser.add_argument("--tolerance", type=float, default=0.01, help="position, in world units")
    parser.add_argument("--degrees", type=float, default=0.5)
    parser.add_argument("--timeout", type=float, default=300.0)
    parser.add_argument("--shell", type=pathlib.Path, default=None)
    parser.add_argument("--keep", action="store_true")
    args = parser.parse_args()

    entries = dict(ENTRIES)
    for spec in args.project:
        name, _, folder = spec.partition("=")
        entries[name] = ("folder", pathlib.Path(folder))
    chosen = args.entries or list(entries)
    exe = args.shell or find_shell()
    if exe is None:
        sys.exit("No blockloom-shell; build the workspace or pass --shell.")
    work = pathlib.Path(tempfile.mkdtemp(prefix="blockloom-corpus-"))
    env = dict(os.environ)
    env.setdefault("BLOCKLOOM_DATA_DIR", str(work / "data"))
    env.setdefault("BLOCKLOOM_RUNTIME", "process")

    failures = 0
    for name in chosen:
        if name not in entries:
            sys.exit(f"Unknown entry {name}; known: {', '.join(entries)}")
        how, what = entries[name]
        shell = Shell(exe, False, env)
        try:
            labels = open_entry(shell, work, name, how, what)
            got = record(shell, work, name, args.ticks, env, args.timeout)
            for actor in got["actors"]:
                actor["id"] = labels.get(actor["id"], actor["id"])
            got["actors"].sort(key=lambda a: a["id"])
            blessed = BLESSED / f"{name}.json"
            if args.bless:
                BLESSED.mkdir(parents=True, exist_ok=True)
                blessed.write_text(json.dumps(got, indent=2, sort_keys=True) + "\n")
                print(f"bless {name}: {len(got['actors'])} actors")
            elif blessed.exists():
                worst = compare(name, got, json.loads(blessed.read_text()), args.tolerance, args.degrees)
                print(f"ok    {name} ({len(got['actors'])} actors, worst drift {worst:.5f})")
            else:
                print(f"new   {name}: nothing blessed yet, played to tick {got['ticks']}")
        except Failed as e:
            failures += 1
            print(f"FAIL  {e}")
        finally:
            shell.close()
    if args.keep or failures:
        print(f"work folder: {work}")
    else:
        shutil.rmtree(work, ignore_errors=True)
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
