#!/usr/bin/env python3
"""Make the mini-survival project: a voxel world and a first-person player.

    python3 plugins/voxel/samples/mini-survival/setup.py [projects folder]

Needs `target/debug/blockloom-shell` (or BLOCKLOOM_SHELL) and the sealed
voxel plugin in `target/plugins/com.blockworked.voxel` (`just voxel-plugin`).
Build the project runtime afterwards from the Plugin Manager, or run this
with --build-runtime.
"""

import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
PLUGIN = "com.blockworked.voxel"


def num(n):
    return {"kind": "Number", "value": n}


def text(t):
    return {"kind": "Text", "value": t}


def var(name):
    return {"kind": "Var", "name": name}


def op(name, *args):
    return {"kind": "Op", "op": name, "args": list(args), "saved": num(0)}


def read(block, *slots):
    return op("PluginRead", text(PLUGIN), text(block), *slots)


def run(block, *args):
    return {"type": "PluginBlock", "plugin": PLUGIN, "block": block, "args": list(args)}


def aim():
    """The camera's position and direction, then a reach of six cells."""
    pos = [op("CameraPosition", text(a)) for a in "XYZ"]
    look = [op("CameraDirection", text(a)) for a in "XYZ"]
    return pos + look + [num(6)]


class Shell:
    def __init__(self, data):
        exe = os.environ.get("BLOCKLOOM_SHELL", str(ROOT / "target/debug/blockloom-shell"))
        env = dict(os.environ, BLOCKLOOM_DATA_DIR=data)
        self.p = subprocess.Popen(
            [exe, "--no-state"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, env=env
        )

    def __call__(self, command, **args):
        line = command + "".join(f" {k}={json.dumps(v)}" for k, v in args.items())
        self.p.stdin.write(line + "\n")
        self.p.stdin.flush()
        answer = json.loads(self.p.stdout.readline())
        if not answer["ok"]:
            sys.exit(f"{command}: {answer['error']}")
        return answer["result"]


def main():
    argv = [a for a in sys.argv[1:] if not a.startswith("--")]
    location = Path(argv[0] if argv else Path.home() / "Blockloom/projects").resolve()
    plugin = ROOT / "target/plugins" / PLUGIN
    if not (plugin / "plugin.json").exists():
        sys.exit(f"seal the voxel plugin into {plugin} first (just voxel-plugin)")
    data = os.environ.get("BLOCKLOOM_DATA_DIR", str(location / ".blockloom-data"))
    sh = Shell(data)

    project = sh("create-project", name="Mini Survival", location=str(location), mode="ThreeD")
    folder = Path(project["path"] if isinstance(project, dict) and "path" in project else location / "Mini Survival")
    shutil.copytree(HERE / "assets", folder / "assets", dirs_exist_ok=True)
    sh("plugin-install", id=PLUGIN, source=f"path:{plugin}")
    sh("set-plugin-resource", resource=f"{PLUGIN}/world", payload=json.loads((HERE / "world.json").read_text()))

    player = sh("add-actor", shape="Capsule", name="Explorer")
    actor = player["id"] if isinstance(player, dict) else player
    # The voxel world is the ground, so the starter actors go.
    sh("save-project")
    for scene in (folder / "assets/scenes").glob("*.blockscene"):
        for other in json.loads(scene.read_text())["actors"]:
            if other["id"] != actor:
                sh("remove-actor", actorId=other["id"])
    sh("rename-actor", actorId=actor, name="Player")
    sh("apply-player-preset", actorId=actor, preset="first-person-3d")
    sh("create-input-action", name="Build")
    sh("add-input-binding", name="Build", binding="mouse:right")
    sh("select-actor", actorId=actor)
    sh("create-variable", name="day")

    def strand(x, y, *instructions):
        made = sh("add-strand", x=x, y=y, instruction=instructions[0])
        sid = made["id"] if isinstance(made, dict) else made
        for i, ins in enumerate(instructions[1:], start=1):
            sh("add-instruction", strandId=sid, path=[{"index": i, "slot": None}], instruction=ins)

    # Left click mines, right click builds planks, E puts down a chest.
    strand(0, 0, {"type": "WhenActionPressed", "action": "Fire"}, run("break_voxel", *aim()))
    strand(0, 160, {"type": "WhenActionPressed", "action": "Build"}, run("place_voxel", *aim(), text("planks")))
    strand(0, 320, {"type": "WhenActionPressed", "action": "Interact"}, run("place_voxel", *aim(), text("chest")))
    strand(
        0, 480,
        {"type": "WhenPlugin", "plugin": PLUGIN, "block": "when_entity_broken", "event": "entity_broken", "args": ["chest"]},
        {"type": "Say", "text": op("Join", text("The chest held "), read("last_broken_items"))},
    )
    # Stand on the ground, then let the sky go from day to night and back.
    step = {"type": "Wait", "duration": num(2)}
    strand(
        500, 0,
        {"type": "WhenStarted"},
        {"type": "GoTo", "x": num(48), "y": op("Add", read("ground_height", num(48), num(48)), num(3)), "z": num(48)},
        {"type": "SetVariable", "name": "day", "value": num(1)},
        {"type": "Forever", "body": [
            {"type": "Repeat", "count": num(20), "body": [
                {"type": "ChangeVariable", "name": "day", "value": num(-0.045)},
                run("set_daylight", var("day")), step]},
            {"type": "Repeat", "count": num(20), "body": [
                {"type": "ChangeVariable", "name": "day", "value": num(0.045)},
                run("set_daylight", var("day")), step]},
        ]},
    )
    sh("save-project")
    if "--build-runtime" in sys.argv:
        sh("build-project-runtime")
    print(f"made {folder}")


if __name__ == "__main__":
    main()
