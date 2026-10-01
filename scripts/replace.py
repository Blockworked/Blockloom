#!/usr/bin/env python3
"""Rebuild the editor and players concurrently, then reinstall on success.

Runs the Editor, Native and Web builds side by side with one status row
below their combined output, reusing the workspace runtime as the native
player unless BLOCKLOOM_NATIVE_PROFILE=dist asks for the fat-LTO one. The
optional argument sets each build's Cargo job limit.
"""

import os
import platform
import queue
import re
import selectors
import shutil
import signal
import subprocess
import sys
import threading
import time
from pathlib import Path


BUILDS = (("Editor", "build"), ("Native", "player"), ("Web", "web-player"))

WINDOWS = os.name == "nt"


def host_target():
    # Spelled the way `blockloom_core::build` spells it - the two have to
    # agree for the exporter to find a staged player.
    arm = platform.machine().lower() in ("aarch64", "arm64")
    if sys.platform == "win32":
        return "aarch64-pc-windows-msvc" if arm else "x86_64-pc-windows-msvc"
    if sys.platform == "darwin":
        return "aarch64-apple-darwin" if arm else "x86_64-apple-darwin"
    return "aarch64-unknown-linux-gnu" if arm else "x86_64-unknown-linux-gnu"


def just_exe():
    # `just` launched this script, so it is on PATH; prefer its full path so
    # Windows process creation never depends on PATHEXT probing.
    return shutil.which("just") or "just"


class Status:
    def __init__(self, output=sys.stdout):
        self.output = output
        self.lock = threading.Lock()
        self.interactive = output.isatty() and os.environ.get("TERM") != "dumb"
        if WINDOWS and self.interactive:
            _enable_vt()
        self.color = self.interactive and "NO_COLOR" not in os.environ and all(
            os.environ.get(name) != "never" for name in ("JUST_COLOR", "CARGO_TERM_COLOR")
        )
        self.started = time.monotonic()
        self.last_report = float("-inf")
        self.states = ["building"] * len(BUILDS)

    def row(self):
        elapsed = int(time.monotonic() - self.started)
        prefix = f"[replace {elapsed // 60:02d}:{elapsed % 60:02d}] "
        labels = [label for label, _ in BUILDS]
        width = None
        if self.interactive:
            width = max(1, shutil.get_terminal_size().columns - 1)
            size = len(prefix) + sum(len(label) + len(state) + 2 for label, state in zip(labels, self.states)) + 6
            if size > width:
                prefix = f"[{elapsed}s] "
                labels = [label[0] for label in labels]
        parts = [(prefix, "2")]
        for index, (label, state) in enumerate(zip(labels, self.states)):
            if index:
                parts.append((" | ", "2"))
            parts.extend([(label, ("36", "35", "34")[index]), (": ", "0")])
            color = "32" if state == "done" else "31" if state.startswith("failed") else "33" if state in ("building", "staging") else "2"
            parts.append((state, color))
        rendered = []
        for text, color in parts:
            if width is not None:
                text = text[:width]
                width -= len(text)
            if text:
                rendered.append(f"\033[{color}m{text}\033[0m" if self.color else text)
        return "".join(rendered)

    def draw(self, changed=False):
        now = time.monotonic()
        if self.interactive:
            with self.lock:
                self.output.write("\r\033[2K" + self.row())
        elif changed or now - self.last_report >= 10:
            self.log(self.row() + "\n")
            self.last_report = now
            return
        self.output.flush()

    def log(self, text):
        with self.lock:
            if self.interactive:
                self.output.write("\r\033[2K")
            self.output.write(text)
            if not text.endswith("\n"):
                self.output.write("\n")
            if self.interactive:
                self.output.write("\033[2K" + self.row())
            self.output.flush()

    def finish(self):
        if self.interactive:
            self.draw()
            self.output.write("\n")
            self.output.flush()


def _enable_vt():
    # conhost needs virtual-terminal processing turned on for the status row's
    # escape sequences; Windows Terminal already has it. Best effort.
    try:
        import ctypes

        kernel32 = ctypes.windll.kernel32
        handle = kernel32.GetStdHandle(-11)
        mode = ctypes.c_ulong()
        if kernel32.GetConsoleMode(handle, ctypes.byref(mode)):
            kernel32.SetConsoleMode(handle, mode.value | 0x0004)
    except Exception:
        pass


def build_command(recipe):
    command = [just_exe(), "--no-deps", recipe]
    if recipe == "web-player":
        command.append(os.environ.get("BLOCKLOOM_WEB_PROFILE", "release"))
    return command


def launch(index, recipe, processes, buffers, outbox, env):
    process = subprocess.Popen(
        build_command(recipe),
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        env=env,
        **({} if not WINDOWS else {"creationflags": subprocess.CREATE_NEW_PROCESS_GROUP}),
        **({"start_new_session": True} if not WINDOWS else {}),
    )
    processes[index] = process
    buffers[index] = b""
    if WINDOWS:
        thread = threading.Thread(target=_pump, args=(process.stdout, outbox, index), daemon=True)
        thread.start()
    return process


def _pump(stream, outbox, index):
    # Selectors cannot wait on pipes on Windows, so each build gets a thread
    # that forwards its output to the main loop's queue.
    try:
        while True:
            chunk = stream.read(65536)
            if not chunk:
                break
            outbox.put((index, chunk))
    except OSError:
        pass
    finally:
        outbox.put((index, None))
        try:
            stream.close()
        except OSError:
            pass


def stop(processes):
    if WINDOWS:
        # No process groups to signal; taskkill takes the whole tree instead.
        for process in processes:
            try:
                subprocess.run(
                    ["taskkill", "/F", "/T", "/PID", str(process.pid)],
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL,
                    timeout=10,
                )
            except (ProcessLookupError, OSError, subprocess.SubprocessError):
                pass
        for process in processes:
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                try:
                    process.kill()
                except OSError:
                    pass
                process.wait()
        return
    for process in processes:
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
    for process in processes:
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait()


def run_builds(native_profile):
    status = Status()
    processes = [None] * len(BUILDS)
    if native_profile == "release":
        status.states[1] = "waiting"
    buffers = {}
    env = {**os.environ, "CARGO_TERM_PROGRESS_WHEN": "never"}
    # Child pipes hide the terminal, so restore colors unless explicitly set.
    color = "always" if status.interactive and "NO_COLOR" not in env else "never"
    env.setdefault("JUST_COLOR", color)
    env.setdefault("CARGO_TERM_COLOR", color)
    try:
        if WINDOWS:
            return _run_threaded(status, processes, buffers, env, native_profile)
        with selectors.DefaultSelector() as selector:
            def launch_selector(index, recipe):
                process = launch(index, recipe, processes, buffers, None, env)
                selector.register(process.stdout, selectors.EVENT_READ, index)

            for index, (_, recipe) in enumerate(BUILDS):
                if index != 1 or native_profile == "dist":
                    launch_selector(index, recipe)
            status.draw(changed=True)
            while selector.get_map() or any(process and process.poll() is None for process in processes):
                for key, _ in selector.select(timeout=0.5):
                    index = key.data
                    chunk = os.read(key.fileobj.fileno(), 65536)
                    if chunk:
                        buffers[index] += chunk
                        end = buffers[index].rfind(b"\n") + 1
                        if end:
                            status.log(buffers[index][:end].decode(errors="replace"))
                            buffers[index] = buffers[index][end:]
                    else:
                        if buffers[index]:
                            status.log(buffers[index].decode(errors="replace"))
                        selector.unregister(key.fileobj)
                        key.fileobj.close()
                _update(status, processes, native_profile, launch_selector)
        return int(any(process and process.returncode != 0 for process in processes))
    except BaseException:
        stop([process for process in processes if process is not None])
        status.states = ["cancelled" if state in ("building", "waiting", "staging") else state for state in status.states]
        raise
    finally:
        status.finish()


def _run_threaded(status, processes, buffers, env, native_profile):
    outbox = queue.Queue()
    pending = set()

    def launch_threaded(index, recipe):
        launch(index, recipe, processes, buffers, outbox, env)
        pending.add(index)

    def handle(index, chunk):
        if chunk is None:
            return False
        buffers[index] += chunk
        end = buffers[index].rfind(b"\n") + 1
        if end:
            status.log(buffers[index][:end].decode(errors="replace"))
            buffers[index] = buffers[index][end:]
        return True

    def drained():
        # Process all queued output; answer whether any arrived. Slots free
        # up only here, so the loop below cannot end with an unseen frame.
        found = False
        try:
            while True:
                index, chunk = outbox.get_nowait()
                found = True
                if not handle(index, chunk):
                    pending.discard(index)
        except queue.Empty:
            pass
        return found

    for index, (_, recipe) in enumerate(BUILDS):
        if index != 1 or native_profile == "dist":
            launch_threaded(index, recipe)
    status.draw(changed=True)
    while True:
        drained()
        _update(status, processes, native_profile, launch_threaded)
        if pending or any(process and process.poll() is None for process in processes):
            try:
                index, chunk = outbox.get(timeout=0.5)
            except queue.Empty:
                continue
            if not handle(index, chunk):
                pending.discard(index)
            continue
        # Quiescent - but output may have landed during the last update.
        if not drained():
            break
    for index, leftover in buffers.items():
        if leftover:
            status.log(leftover.decode(errors="replace"))
    return int(any(process and process.returncode != 0 for process in processes))


def _update(status, processes, native_profile, launch):
    changed = False
    for index, process in enumerate(processes):
        if process is None:
            continue
        code = process.poll()
        if code is not None and status.states[index] in ("building", "staging"):
            status.states[index] = "done" if code == 0 else f"failed (exit {code})"
            changed = True
    if status.states[1] == "waiting" and processes[0].returncode is not None:
        if processes[0].returncode == 0:
            launch(1, "_stage-release-player")
            status.states[1] = "staging"
        else:
            status.states[1] = "skipped"
        changed = True
    status.draw(changed)


def check(cmd):
    code = subprocess.run(cmd).returncode
    if code != 0:
        sys.exit(code)


def main(argv):
    native_profile = os.environ.get("BLOCKLOOM_NATIVE_PROFILE", "release")
    if native_profile not in ("release", "dist"):
        print("BLOCKLOOM_NATIVE_PROFILE must be release or dist", file=sys.stderr)
        return 1
    build_count = 3 if native_profile == "dist" else 2
    jobs = argv[0] if argv else ""
    if not jobs:
        cpus = os.cpu_count() or 1
        jobs = os.environ.get("CARGO_BUILD_JOBS") or str((cpus + build_count - 1) // build_count)
    if not re.fullmatch(r"[1-9][0-9]*", jobs):
        print("jobs must be a positive integer (per build)", file=sys.stderr)
        return 1
    os.environ["CARGO_BUILD_JOBS"] = jobs
    os.environ["CARGO_TERM_PROGRESS_WHEN"] = "never"
    root = Path(__file__).resolve().parent
    prune = [sys.executable, str(root / "prune-target.py")]
    check(prune)
    # Resolve and download once before parallel builds contend for the cache.
    print("Fetching native and web dependencies before compiling.")
    check(["cargo", "fetch", "--locked", "--target", host_target(), "--target", "wasm32-unknown-unknown"])
    os.environ["CARGO_NET_OFFLINE"] = "true"
    print(f"Building {build_count} targets concurrently ({jobs} Cargo jobs each).")
    if native_profile == "release":
        print("Native player will reuse the editor's optimized runtime.")
    # Preparation already ran as a recipe dependency; the runner skips the
    # recipes' own dependencies.
    error = None
    try:
        failed = run_builds(native_profile) != 0
    except KeyboardInterrupt:
        raise
    except BaseException as exc:
        failed = True
        error = exc
    check(prune)
    if error is not None:
        raise error
    if failed:
        print("A build failed; skipping reinstall.", file=sys.stderr)
        return 1
    if sys.platform == "linux":
        check([just_exe(), "uninstall", "install"])
    else:
        # No system install elsewhere: the editor, runtime and staged players
        # are used from target/release as built.
        directory = Path(os.environ.get("CARGO_TARGET_DIR", "target")) / "release"
        print(f"Replacement builds are in {directory} - run the editor from there.")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main(sys.argv[1:]))
    except KeyboardInterrupt:
        sys.exit(130)
    except OSError as error:
        sys.exit(f"replace: {error}")
