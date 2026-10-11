"""Shared setup for the Hub's Python tests: the Rust state tool they call."""

import json
import os
from pathlib import Path
import subprocess


def ensure_hubctl():
    """Point BLOCKLOOM_HUBCTL at a built `blockloom-hubctl`, building it once."""
    if os.environ.get("BLOCKLOOM_HUBCTL"):
        return
    repo = Path(__file__).resolve().parent.parent
    # CI has the Hub built already and may not have a toolchain to hand.
    prebuilt = repo / "target" / "release" / ("blockloom-hub.exe" if os.name == "nt" else "blockloom-hub")
    if os.environ.get("CI") and prebuilt.is_file():
        os.environ["BLOCKLOOM_HUBCTL"] = str(prebuilt)
        os.environ["BLOCKLOOM_HUBCTL_ARGS"] = "--hubctl"
        return
    built = subprocess.run(["cargo", "build", "-q", "-p", "blockloom-hub-state", "--bin", "blockloom-hubctl",
                            "--message-format=json"], cwd=repo, capture_output=True, text=True, check=True)
    for line in built.stdout.splitlines():
        message = json.loads(line)
        if message.get("executable"):
            os.environ["BLOCKLOOM_HUBCTL"] = message["executable"]
            os.environ.pop("BLOCKLOOM_HUBCTL_ARGS", None)
