#!/usr/bin/env python3
"""Run a bash script where `bash` may not be on PATH.

On Windows, Git Bash is usually installed but its `usr/bin` is not on the
system PATH, so recipes cannot just call `bash`. This finds it - PATH first,
then the default Git for Windows locations - and execs the script with any
extra arguments, propagating its exit code.
"""

import os
import shutil
import subprocess
import sys


def find_bash():
    found = shutil.which("bash")
    if found:
        return found
    if os.name == "nt":
        program_files = os.environ.get("ProgramFiles", r"C:\Program Files")
        program_files_x86 = os.environ.get("ProgramFiles(x86)", r"C:\Program Files (x86)")
        for candidate in (
            os.path.join(program_files, "Git", "bin", "bash.exe"),
            os.path.join(program_files, "Git", "usr", "bin", "bash.exe"),
            os.path.join(program_files_x86, "Git", "bin", "bash.exe"),
        ):
            if os.path.isfile(candidate):
                return candidate
    sys.exit(
        "Could not find bash. On Windows, install Git for Windows "
        "(https://git-scm.com/downloads/win) or put bash on PATH."
    )


def main(argv):
    if not argv or argv[0] in ("-h", "--help"):
        print("usage: run-bash.py script [args ...]", file=sys.stderr)
        return 2
    return subprocess.run([find_bash(), *argv]).returncode


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
