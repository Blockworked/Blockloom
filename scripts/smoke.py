#!/usr/bin/env python3
"""End-to-end smoke flows through blockloom-shell.

Each flow makes a project, edits it, undoes and redoes, plays it in a real
runtime, checks that the world moved and logged no errors, stops, builds the
game for this machine and runs the built player for a few seconds.

    python3 scripts/smoke.py [--shell PATH] [--attach] [--keep] [flow ...]

Without --attach the shell hosts its own backend and starts the runtime as a
child (it needs a display: Xvfb works). With --attach it drives a running
editor instead, which is how the nightly run checks the editor's own Play.
Exits nonzero on the first failed check.
"""

import argparse
import json
import os
import pathlib
import shutil
import subprocess
import sys
import tempfile
import time

ROOT = pathlib.Path(__file__).resolve().parent.parent


class Failed(Exception):
    pass


class Shell:
    def __init__(self, exe, attach, env):
        args = [str(exe)] + (["--attach"] if attach else [])
        self.proc = subprocess.Popen(
            args, stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, env=env, bufsize=1
        )

    def run(self, line, ok=True):
        self.proc.stdin.write(line + "\n")
        self.proc.stdin.flush()
        out = self.proc.stdout.readline()
        if not out:
            raise Failed(f"shell exited during: {line}")
        reply = json.loads(out)
        if ok and not reply.get("ok"):
            raise Failed(f"{line} -> {reply.get('error')}")
        return reply

    def close(self):
        try:
            self.proc.stdin.write("exit\n")
            self.proc.stdin.flush()
            self.proc.wait(timeout=30)
        except Exception:
            self.proc.kill()


def check(cond, message):
    if not cond:
        raise Failed(message)


def actors(state):
    project = state.get("project") or {}
    found = project.get("actors")
    if found is None:
        scene = next((s for s in project.get("scenes", []) if s.get("actors") is not None), {})
        found = scene.get("actors", [])
    return found


def errors(state):
    return [line for line in state.get("log", []) if line.get("level") == "error"]


def pose(status):
    return {a["id"]: a["position"] + a["rotation"] for a in status.get("actors") or []}


def moved(first, now):
    return [i for i in first if i in now and max(abs(a - b) for a, b in zip(first[i], now[i])) > 0.05]


def play(shell, seconds, timeout):
    """Press Play and wait, within `timeout`, until an actor has moved and `seconds` more game time passed."""
    shell.run("run-project")
    first, since, deadline = None, None, time.monotonic() + timeout
    while time.monotonic() < deadline:
        state = shell.run("sync-status")["state"]
        status = state.get("status") or {}
        check(not errors(state), f"run logged errors: {errors(state)[:3]}")
        if first is None and status.get("actors"):
            first = pose(status)
        # A software renderer can spend a while warming up before the first tick.
        if first is not None and since is None and moved(first, pose(status)):
            since = status.get("time", 0)
        if since is not None and status.get("time", 0) >= since + seconds:
            shell.run("stop-project")
            return
        time.sleep(0.25)
    shell.run("stop-project", ok=False)
    what = "nothing moved" if since is None else f"the game did not run {seconds}s more"
    raise Failed(f"{what} within {timeout}s of pressing Play")


def run_player(folder, seconds, env):
    """Start a built game and check it is still alive after `seconds`."""
    exe = next(
        (p for p in folder.iterdir() if p.is_file() and os.access(p, os.X_OK) and p.suffix in ("", ".exe")),
        None,
    )
    check(exe is not None, f"no player in {folder}")
    log = folder / "smoke-player.log"
    with open(log, "w") as out:
        proc = subprocess.Popen([str(exe)], stdout=out, stderr=subprocess.STDOUT, env=env)
        time.sleep(seconds)
        alive = proc.poll() is None
        proc.kill()
        proc.wait()
    text = log.read_text(errors="replace")
    check(alive, f"the built game exited early:\n{text[-2000:]}")
    check("panicked" not in text, f"the built game panicked:\n{text[-2000:]}")


def flow_playground(shell, work, mode, env, args):
    name = f"Smoke {mode}"
    shell.run(f'create-project name="{name}" location="{work}" mode={mode} sample=physics-playground')
    state = shell.run("sync-status")["state"]
    before = actors(state)
    check(before, "the sample has no actors")

    # An edit, its undo and its redo.
    target = before[0]
    shell.run(f'rename-actor actorId="{target["id"]}" name="Renamed"')
    renamed = {a["id"]: a["name"] for a in actors(shell.run("sync-status")["state"])}
    check(renamed[target["id"]] == "Renamed", "rename did not land")
    shell.run("undo")
    undone = {a["id"]: a["name"] for a in actors(shell.run("sync-status")["state"])}
    check(undone[target["id"]] == target["name"], "undo did not restore the name")
    shell.run("redo")
    shell.run("undo")
    count = len(actors(shell.run(f'add-actor shape={"Cuboid" if mode == "ThreeD" else "Rect"} name=Extra')["state"]))
    check(count == len(before) + 1, "add-actor did not add one actor")
    shell.run("save-project")

    play(shell, args.seconds, args.timeout)

    out = work / f"build-{mode}"
    shell.run(f'build-game path="{out}"')
    folders = [p for p in out.iterdir() if p.is_dir()]
    check(folders, f"build wrote no folder in {out}")
    run_player(folders[0], args.player_seconds, env)
    shell.run("close-project")


def flow_scripted(shell, work, mode, env, args):
    """A project whose actor is driven by a Rust script: the editor compiles it
    on Play, so this flow needs a rustc. The world runs in its own process for
    a scripted project, which is what keeps a faulting script off the editor."""
    name = f"Smoke script {mode}"
    shell.run(f'create-project name="{name}" location="{work}" mode={mode}')
    shape = "Cuboid" if mode == "ThreeD" else "Rect"
    actor = actors(shell.run(f"add-actor shape={shape} name=Walker")["state"])[-1]
    step = 0.05 if mode == "ThreeD" else 5.0
    source = (
        "use blockloom::*;\n"
        "fn tick(me: &Actor, _dt: f32) {\n"
        f"    let pose = me.pose();\n"
        f"    me.go_to(pose.position[0] + {step}, pose.position[1], pose.position[2]);\n"
        "}\n"
        "blockloom::export!(tick = tick);\n"
    )
    shell.run(f'write-text-asset path=assets/scripts/walker.rs text={json.dumps(source)}')
    component = json.dumps({"component": "Script", "path": "assets/scripts/walker.rs"})
    shell.run(f'add-actor-component actorId={actor["id"]} component={component}')
    play(shell, args.seconds, args.timeout)
    shell.run("close-project")


def flow_lan(shell, work, env, args):
    """The LAN co-op template hosted from the shell's runtime, and its built
    player joining it as player 2 over loopback."""
    shell.run(f'create-project name="Smoke LAN" location="{work}" mode=TwoD sample=lan-co-op')
    out = work / "build-lan"
    shell.run(f'build-game path="{out}"')
    folders = [p for p in out.iterdir() if p.is_dir()]
    check(folders, f"build wrote no folder in {out}")
    shell.run("run-project")
    # A software renderer warms up before the run starts; ask until it has.
    invite, lan, asked, deadline = None, {}, 0.0, time.monotonic() + args.timeout
    while invite is None and time.monotonic() < deadline:
        if not lan.get("open") and time.monotonic() - asked > 2:
            shell.run('open-lan bind="127.0.0.1:0" max_guests=3')
            asked = time.monotonic()
        time.sleep(0.25)
        lan = shell.run("session-status")["result"] or {}
        invite = lan.get("invite")
    check(invite, f"LAN didn't open within {args.timeout}s: {lan.get('error')}")
    folder = folders[0]
    exe = next(
        (p for p in folder.iterdir() if p.is_file() and os.access(p, os.X_OK) and p.suffix in ("", ".exe")),
        None,
    )
    check(exe is not None, f"no player in {folder}")
    log = folder / "smoke-guest.log"
    with open(log, "w") as text:
        guest = subprocess.Popen(
            [str(exe), "--join", invite, "--trusted-build", invite.rsplit("|", 1)[-1]],
            stdout=text, stderr=subprocess.STDOUT, env=env,
        )
        try:
            joined, deadline = False, time.monotonic() + args.timeout
            while not joined and time.monotonic() < deadline:
                check(guest.poll() is None, f"the guest exited:\n{log.read_text(errors='replace')[-2000:]}")
                joined = bool((shell.run("session-status")["result"] or {}).get("guests"))
                time.sleep(0.25)
            check(joined, f"the guest never joined within {args.timeout}s")
            time.sleep(args.player_seconds)
            check(guest.poll() is None, f"the guest left:\n{log.read_text(errors='replace')[-2000:]}")
            state = shell.run("sync-status")["state"]
            check(not errors(state), f"the host logged errors: {errors(state)[:3]}")
        finally:
            guest.kill()
            guest.wait()
    text = log.read_text(errors="replace")
    check("panicked" not in text, f"the guest panicked:\n{text[-2000:]}")
    shell.run("close-lan")
    shell.run("stop-project")
    shell.run("close-project")


FLOWS = {
    "playground-3d": lambda s, w, e, a: flow_playground(s, w, "ThreeD", e, a),
    "playground-2d": lambda s, w, e, a: flow_playground(s, w, "TwoD", e, a),
    "scripted-3d": lambda s, w, e, a: flow_scripted(s, w, "ThreeD", e, a),
    "lan": flow_lan,
}


def find_shell():
    for profile in ("debug", "release"):
        exe = ROOT / "target" / profile / ("blockloom-shell.exe" if os.name == "nt" else "blockloom-shell")
        if exe.exists():
            return exe
    return None


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("flows", nargs="*", default=list(FLOWS))
    parser.add_argument("--shell", type=pathlib.Path, default=None)
    parser.add_argument("--attach", action="store_true")
    parser.add_argument("--keep", action="store_true", help="keep the work folder")
    parser.add_argument("--seconds", type=float, default=2.0, help="game time to play")
    parser.add_argument("--timeout", type=float, default=180.0, help="wall time to reach it")
    parser.add_argument("--player-seconds", type=float, default=5.0)
    args = parser.parse_args()

    exe = args.shell or find_shell()
    if exe is None:
        sys.exit("No blockloom-shell; build the workspace or pass --shell.")
    work = pathlib.Path(tempfile.mkdtemp(prefix="blockloom-smoke-"))
    env = dict(os.environ)
    if not args.attach:
        env.setdefault("BLOCKLOOM_DATA_DIR", str(work / "data"))
        env.setdefault("BLOCKLOOM_RUNTIME", "process")

    failures = 0
    for name in args.flows:
        if name not in FLOWS:
            sys.exit(f"Unknown flow {name}; known: {', '.join(FLOWS)}")
        shell = Shell(exe, args.attach, env)
        started = time.monotonic()
        try:
            FLOWS[name](shell, work, env, args)
            print(f"ok   {name} ({time.monotonic() - started:.0f}s)")
        except (Failed, json.JSONDecodeError) as e:
            failures += 1
            print(f"FAIL {name}: {e}")
        finally:
            shell.close()
    if args.keep or failures:
        print(f"work folder: {work}")
    else:
        shutil.rmtree(work, ignore_errors=True)
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
