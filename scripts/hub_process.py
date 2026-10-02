"""Process identity and cooperative cancellation for Hub workers."""

import ctypes
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time
import threading
import tempfile


class Cancelled(Exception):
    pass


class WindowsJob:
    """Own the build's descendants without enumerating unrelated processes."""
    def __init__(self):
        from ctypes import wintypes
        class Limits(ctypes.Structure):
            _fields_ = [("user_time", ctypes.c_longlong), ("job_time", ctypes.c_longlong),
                        ("flags", wintypes.DWORD), ("min_working", ctypes.c_size_t),
                        ("max_working", ctypes.c_size_t), ("active", wintypes.DWORD),
                        ("affinity", ctypes.c_size_t), ("priority", wintypes.DWORD),
                        ("scheduling", wintypes.DWORD)]
        class Extended(ctypes.Structure):
            _fields_ = [("limits", Limits), ("io", ctypes.c_ulonglong * 6),
                        ("process_memory", ctypes.c_size_t), ("job_memory", ctypes.c_size_t),
                        ("peak_process", ctypes.c_size_t), ("peak_job", ctypes.c_size_t)]
        self.kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        self.kernel.CreateJobObjectW.argtypes = [ctypes.c_void_p, wintypes.LPCWSTR]
        self.kernel.CreateJobObjectW.restype = wintypes.HANDLE
        self.kernel.SetInformationJobObject.argtypes = [wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p, wintypes.DWORD]
        self.kernel.AssignProcessToJobObject.argtypes = [wintypes.HANDLE, wintypes.HANDLE]
        self.kernel.TerminateJobObject.argtypes = [wintypes.HANDLE, wintypes.UINT]
        self.kernel.CloseHandle.argtypes = [wintypes.HANDLE]
        self.handle = self.kernel.CreateJobObjectW(None, None)
        if not self.handle:
            raise ctypes.WinError(ctypes.get_last_error())
        limits = Extended()
        limits.limits.flags = 0x2000  # Kill descendants when the owning handle closes.
        if not self.kernel.SetInformationJobObject(self.handle, 9, ctypes.byref(limits), ctypes.sizeof(limits)):
            error = ctypes.get_last_error()
            self.close()
            raise ctypes.WinError(error)

    def assign(self, process):
        if not self.kernel.AssignProcessToJobObject(self.handle, int(process._handle)):
            raise ctypes.WinError(ctypes.get_last_error())

    def terminate(self):
        if not self.kernel.TerminateJobObject(self.handle, 130):
            raise ctypes.WinError(ctypes.get_last_error())

    def close(self):
        if self.handle:
            self.kernel.CloseHandle(self.handle)
            self.handle = None


def checkpoint():
    if path := os.environ.get("BLOCKLOOM_HUB_CANCEL_FILE"):
        if Path(path).exists():
            raise Cancelled("Operation cancelled; previous installation kept")


def process_token(pid):
    """Return a process's birth identifier, None if exited, unknown if inaccessible."""
    if not isinstance(pid, int) or pid <= 0:
        raise ValueError("Invalid process ID")
    if os.name == "nt":
        from ctypes import wintypes
        kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        kernel.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
        kernel.OpenProcess.restype = wintypes.HANDLE
        kernel.CloseHandle.argtypes = [wintypes.HANDLE]
        kernel.GetExitCodeProcess.argtypes = [wintypes.HANDLE, ctypes.POINTER(wintypes.DWORD)]
        kernel.GetProcessTimes.argtypes = [wintypes.HANDLE] + [ctypes.POINTER(wintypes.FILETIME)] * 4
        handle = kernel.OpenProcess(0x1000, False, pid)
        if not handle:
            return None if ctypes.get_last_error() == 87 else "unknown"
        try:
            code = wintypes.DWORD()
            if not kernel.GetExitCodeProcess(handle, ctypes.byref(code)):
                return "unknown"
            if code.value != 259:
                return None
            times = [wintypes.FILETIME() for _ in range(4)]
            if not kernel.GetProcessTimes(handle, *(ctypes.byref(t) for t in times)):
                return "unknown"
            return str((times[0].dwHighDateTime << 32) | times[0].dwLowDateTime)
        finally:
            kernel.CloseHandle(handle)
    if sys.platform.startswith("linux"):
        try:
            fields = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
            boot = Path("/proc/sys/kernel/random/boot_id").read_text().strip()
            return None if fields[0] in ("Z", "X") else boot + ":" + fields[19]
        except FileNotFoundError:
            return None
        except PermissionError:
            return "unknown"
    result = subprocess.run(["ps", "-p", str(pid), "-o", "lstart=", "-o", "stat="],
                            capture_output=True, text=True, timeout=5)
    value = result.stdout.strip()
    if not value:
        return None
    fields = value.rsplit(None, 1)
    return None if fields[-1].startswith("Z") else fields[0]


def running(record):
    current = process_token(record["pid"])
    return current is not None and (current == record.get("token")
                                   or "unknown" in (current, record.get("token")))


def stop_tree(process, job=None):
    if process.poll() is not None:
        return
    if os.name == "nt":
        job.terminate()
    else:
        # Replacement builds use their own sessions, so include descendants too.
        snapshot = subprocess.run(["ps", "-axo", "pid=,ppid="], capture_output=True,
                                  text=True, check=True, timeout=5)
        parents = {int(pid): int(parent) for pid, parent in
                   (line.split() for line in snapshot.stdout.splitlines() if line.strip())}
        descendants = [process.pid]
        for parent in descendants:
            descendants.extend(pid for pid, ppid in parents.items()
                               if ppid == parent and pid not in descendants)
        tokens = {pid: process_token(pid) for pid in descendants}
        for sig in (signal.SIGTERM, signal.SIGKILL):
            for pid in reversed(descendants):
                if tokens[pid] is not None and process_token(pid) == tokens[pid]:
                    try:
                        os.kill(pid, sig)
                    except ProcessLookupError:
                        pass
            if sig == signal.SIGTERM:
                time.sleep(0.25)
    process.wait(timeout=15)


def run(command, log_file=None, **kwargs):
    checkpoint()
    log = None
    if log_file:
        log_file.parent.mkdir(parents=True, exist_ok=True)
        log = log_file.open("wb")
        kwargs.update(stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    if os.name == "nt":
        kwargs.setdefault("creationflags", subprocess.CREATE_NO_WINDOW)
    try:
        _run(command, log, kwargs)
    finally:
        if log:
            log.close()


def _run(command, log, kwargs):
    if os.name == "nt":
        job = WindowsJob()
        try:
            with tempfile.TemporaryDirectory(prefix="hub-build-gate-") as temporary:
                gate = Path(temporary) / "start"
                # Assign the waiting bootstrap before it can create any descendants.
                bootstrap = [sys.executable, "-B", __file__, "--launch", json.dumps([str(c) for c in command]), str(gate)]
                with subprocess.Popen(bootstrap, **kwargs) as process:
                    try:
                        job.assign(process)
                        gate.touch()
                    except BaseException:
                        process.kill()
                        process.wait(timeout=10)
                        raise
                    _monitor(process, command, log, job)
        finally:
            job.close()
    else:
        with subprocess.Popen(command, **kwargs) as process:
            _monitor(process, command, log)


def _monitor(process, command, log, job=None):
    reader = None
    if log:
        def pump():
            while chunk := process.stdout.read1(65536):
                log.write(chunk)
                log.flush()
                sys.stderr.write(chunk.decode("utf-8", errors="replace"))
                sys.stderr.flush()
        reader = threading.Thread(target=pump, daemon=True)
        reader.start()
    try:
        while process.poll() is None:
            checkpoint()
            time.sleep(0.1)
        checkpoint()
        if process.returncode:
            raise subprocess.CalledProcessError(process.returncode, command)
    except BaseException:
        stop_tree(process, job)
        raise
    finally:
        if reader:
            reader.join(timeout=15)


def copy_file(source, destination, progress=None):
    """Check between bounded copy chunks, then preserve file metadata."""
    import shutil
    with open(source, "rb") as reader, open(destination, "wb") as writer:
        while True:
            checkpoint()
            chunk = reader.read(1024 * 1024)
            if not chunk:
                break
            writer.write(chunk)
            if progress:
                progress(len(chunk), False)
    shutil.copystat(source, destination)
    if progress:
        progress(0, True)
    return destination


class CopyProgress:
    def __init__(self, label, files, size):
        self.label, self.files, self.size = label, files, size
        self.copied, self.finished, self.last = 0, 0, 0
        self(0, False)

    def __call__(self, size, finished):
        self.copied += size
        self.finished += int(finished)
        now = time.monotonic()
        if now - self.last >= 1 or self.finished == self.files:
            print(f"{self.label}: {self.finished}/{self.files} files, "
                  f"{self.copied / 1024**2:.0f}/{self.size / 1024**2:.0f} MiB", file=sys.stderr, flush=True)
            self.last = now


def bundle_rust(source, destination, channel):
    import shutil
    roots = [source / name for name in ("bin", "lib", "libexec", "etc") if (source / name).is_dir()]

    def ignore(directory, names):
        checkpoint()
        return ["src"] if Path(directory) == source / "lib" / "rustlib" and "src" in names else []

    files = []
    for root in roots:
        for directory, directories, names in os.walk(root):
            checkpoint()
            directories[:] = [name for name in directories if name not in ignore(directory, directories)]
            files.extend(Path(directory) / name for name in names)
    progress = CopyProgress(f"Bundling Rust {channel}", len(files), sum(p.stat().st_size for p in files))
    destination.mkdir(parents=True)
    for root in roots:
        shutil.copytree(root, destination / root.name, ignore=ignore,
                        copy_function=lambda src, dst: copy_file(src, dst, progress))


if __name__ == "__main__":
    if len(sys.argv) != 4 or sys.argv[1] != "--launch":
        sys.exit("This helper is launched by the Hub build worker")
    while not Path(sys.argv[3]).exists():
        time.sleep(0.01)
    sys.exit(subprocess.run(json.loads(sys.argv[2]), creationflags=subprocess.CREATE_NO_WINDOW,
                            stdin=subprocess.DEVNULL, stdout=sys.stdout, stderr=sys.stderr).returncode)
