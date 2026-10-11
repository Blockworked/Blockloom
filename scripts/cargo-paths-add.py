#!/usr/bin/env python3
"""Add a path to .cargo/config.toml's `paths` override without dropping
entries other recipes (e.g. `just blockstitch-local`) wrote there.

Usage: cargo-paths-add.py <abs-or-rel-path>  (run from the workspace root)
"""

import re
import sys
from pathlib import Path

path = sys.argv[1].replace("\\", "/")
cfg = Path(".cargo/config.toml")
paths: list[str] = []
if cfg.is_file():
    m = re.search(r"paths\s*=\s*\[(.*?)\]", cfg.read_text(), re.S)
    if m:
        paths = re.findall(r'"([^"]+)"', m.group(1))
if path not in paths:
    paths.append(path)
cfg.parent.mkdir(parents=True, exist_ok=True)
cfg.write_text("paths = [" + ", ".join(f'"{p}"' for p in paths) + "]\n")
print("cargo paths override:", paths)
