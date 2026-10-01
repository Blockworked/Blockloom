#!/usr/bin/env python3
"""Render, resize and restart both worlds through the real Qt Game view."""

import argparse
import os
from pathlib import Path
import subprocess
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--backend", choices=("vulkan", "opengl"), default="vulkan")
    parser.add_argument("--exe", type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    exe = args.exe or root / "target/release/blockloom.exe"
    output = Path(tempfile.mkdtemp(prefix="game-view-", dir=root / "target"))
    environment = os.environ.copy()
    environment.update(
        BLOCKLOOM_DATA_DIR=str(output / "data"),
        BLOCKLOOM_QML_ENTRY=(root / "blockloom-qt/tests/GameViewSmoke.qml").as_uri(),
        QSG_RHI_BACKEND=args.backend,
        QSG_INFO="1",
    )
    print(f"Game view {args.backend}: {output}", flush=True)
    with (output / "stdout.log").open("w") as stdout, (output / "stderr.log").open("w") as stderr:
        process = subprocess.Popen(
            [str(exe.resolve()), output.as_posix()], cwd=root, env=environment,
            stdout=stdout, stderr=stderr,
            creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0,
        )
        try:
            code = process.wait(timeout=180)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
            print("Timed out. Build with --features blockloom/qml-preview first.")
            return 1
    log = (output / "stderr.log").read_text(errors="replace")
    screenshots = [output / f"{mode}-{phase}.png" for mode in ("TwoD", "ThreeD") for phase in (0, 1)]
    passed = code == 0 and "SMOKE PASSED" in log and all(path.is_file() for path in screenshots)
    if args.backend == "vulkan":
        passed &= log.count("imported 3 zero-copy Vulkan images") >= 4
    else:
        passed &= "zero-copy Vulkan images" not in log
    if not passed:
        print(log[-12000:])
        print(f"Failed with exit code {code}; logs and captures are in {output}")
        return 1
    print("Passed: 2D, 3D, resize, world restart, item destruction and clean exit.")
    print("Inspect the four PNG captures for rendering regressions.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
