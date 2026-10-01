#!/usr/bin/env python3
"""Run replacement builds with one status row below their combined output."""

import os
import selectors
import shutil
import signal
import subprocess
import sys
import time


BUILDS = (("Editor", "build"), ("Native", "player"), ("Web", "web-player"))


class Status:
    def __init__(self, output=sys.stdout):
        self.output = output
        self.interactive = output.isatty() and os.environ.get("TERM") != "dumb"
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
            self.output.write("\r\033[2K" + self.row())
        elif changed or now - self.last_report >= 10:
            self.output.write(self.row() + "\n")
            self.last_report = now
        self.output.flush()

    def log(self, text):
        if self.interactive:
            self.output.write("\r\033[2K")
        self.output.write(text)
        if not text.endswith("\n"):
            self.output.write("\n")
        if self.interactive:
            self.draw()
        self.output.flush()

    def finish(self):
        if self.interactive:
            self.draw()
            self.output.write("\n")
            self.output.flush()


def stop(processes):
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


def main():
    native_profile = os.environ.get("BLOCKLOOM_NATIVE_PROFILE", "release")
    if native_profile not in ("release", "dist"):
        sys.exit("BLOCKLOOM_NATIVE_PROFILE must be release or dist")
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
        with selectors.DefaultSelector() as selector:
            def launch(index, recipe):
                command = ["just", "--no-deps", recipe]
                if recipe == "web-player":
                    command.append(env.get("BLOCKLOOM_WEB_PROFILE", "release"))
                process = subprocess.Popen(
                    command, stdout=subprocess.PIPE,
                    stderr=subprocess.STDOUT, env=env, start_new_session=True,
                )
                processes[index] = process
                buffers[index] = b""
                selector.register(process.stdout, selectors.EVENT_READ, index)

            for index, (_, recipe) in enumerate(BUILDS):
                if index != 1 or native_profile == "dist":
                    launch(index, recipe)
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
        return int(any(process and process.returncode != 0 for process in processes))
    except BaseException:
        stop([process for process in processes if process is not None])
        status.states = ["cancelled" if state in ("building", "waiting", "staging") else state for state in status.states]
        raise
    finally:
        status.finish()


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        sys.exit(130)
    except OSError as error:
        sys.exit(f"replace-builds: {error}")
