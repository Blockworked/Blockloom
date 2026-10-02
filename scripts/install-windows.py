#!/usr/bin/env python3
"""Install Blockloom for the current Windows user, or remove it again.

Mirrors what `just install` does on Linux: the editor and the runtime it
starts live in one directory, with the staged players beside them, plus a
Start Menu shortcut and a PATH entry. No admin rights needed; everything
sits under %LOCALAPPDATA%. `uninstall` removes all three.
"""

import os
import shutil
import subprocess
import sys
import winreg
from pathlib import Path


def repo_root():
    return Path(__file__).resolve().parent.parent


def release_dir():
    # CARGO_TARGET_DIR is honored the way Cargo honors it, which also lets a
    # test point the installer at dummy binaries.
    target = os.environ.get("CARGO_TARGET_DIR", "target")
    path = Path(target)
    if not path.is_absolute():
        path = repo_root() / path
    return path / "release"


def install_root(name="Blockloom"):
    local = os.environ.get("LOCALAPPDATA")
    if not local:
        sys.exit("install: LOCALAPPDATA is not set")
    return Path(local) / "Programs" / name


def shortcut_path(name="Blockloom"):
    appdata = os.environ.get("APPDATA")
    if not appdata:
        sys.exit("install: APPDATA is not set")
    return Path(appdata) / "Microsoft" / "Windows" / "Start Menu" / "Programs" / (name + ".lnk")


def remove_tree(path):
    def readonly(function, name, excinfo):
        os.chmod(name, 0o666)
        function(name)

    shutil.rmtree(path, onerror=readonly)


def install_file(src, dst):
    dst.parent.mkdir(parents=True, exist_ok=True)
    # A backup a previous install could not delete while its program ran.
    backup = dst.with_name(dst.name + ".old")
    try:
        backup.unlink(missing_ok=True)
    except OSError:
        pass
    try:
        shutil.copy2(src, dst)
        return
    except PermissionError:
        pass
    # A running executable cannot be overwritten, but it can be renamed away.
    try:
        os.replace(dst, backup)
    except OSError as error:
        sys.exit(f"install: cannot replace {dst} (close Blockloom first?): {error}")
    shutil.copy2(src, dst)
    try:
        backup.unlink()
    except OSError:
        # Still held by the running program; the next install clears it.
        pass


def create_shortcut(target, shortcut):
    # The installed editor carries its icon embedded, so the shortcut points
    # at it rather than at a separate icon file.
    script = (
        "$ws = New-Object -ComObject WScript.Shell; "
        f"$sc = $ws.CreateShortcut('{shortcut}'); "
        f"$sc.TargetPath = '{target}'; "
        f"$sc.WorkingDirectory = '{target.parent}'; "
        f"$sc.IconLocation = '{target},0'; "
        "$sc.Save()"
    )
    try:
        subprocess.run(
            ["powershell", "-NoProfile", "-NonInteractive", "-Command", script],
            check=True,
            stdout=subprocess.DEVNULL,
        )
    except (OSError, subprocess.CalledProcessError) as error:
        sys.exit(f"install: cannot create the Start Menu shortcut: {error}")


def read_user_path():
    try:
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, "Environment", 0, winreg.KEY_READ) as key:
            return winreg.QueryValueEx(key, "Path")
    except FileNotFoundError:
        return "", winreg.REG_EXPAND_SZ


def write_user_path(value, kind):
    with winreg.OpenKey(winreg.HKEY_CURRENT_USER, "Environment", 0, winreg.KEY_SET_VALUE) as key:
        winreg.SetValueEx(key, "Path", 0, kind, value)
    broadcast_env_change()


def broadcast_env_change():
    try:
        import ctypes

        ctypes.windll.user32.SendMessageTimeoutW(0xFFFF, 0x1A, 0, "Environment", 0x0002, 5000, None)
    except Exception:
        pass


def entry_present(path, value):
    wanted = str(path).rstrip("\\").lower()
    return any(part.rstrip("\\").lower() == wanted for part in value.split(";") if part)


def add_path_entry(path):
    value, kind = read_user_path()
    if entry_present(path, value):
        return False
    separator = "" if not value or value.endswith(";") else ";"
    write_user_path(f"{value}{separator}{path}", kind)
    return True


def remove_path_entry(path):
    value, kind = read_user_path()
    wanted = str(path).rstrip("\\").lower()
    kept = [part for part in value.split(";") if part and part.rstrip("\\").lower() != wanted]
    if len(kept) == len([part for part in value.split(";") if part]):
        return False
    write_user_path(";".join(kept), kind)
    return True


def install():
    release = release_dir()
    editor = release / "blockloom.exe"
    runtime = release / "blockloom-runtime.exe"
    for binary in (editor, runtime):
        if not binary.is_file():
            sys.exit(f"install: {binary} is missing - run just build first")
    root = install_root()
    install_file(editor, root / "blockloom.exe")
    install_file(runtime, root / "blockloom-runtime.exe")
    players = release / "players"
    if players.is_dir():
        shutil.copytree(players, root / "players", dirs_exist_ok=True)
    create_shortcut(root / "blockloom.exe", shortcut_path())
    on_path = add_path_entry(root)
    print(f"Installed Blockloom to {root}")
    print("Start Menu shortcut and PATH entry are ready." if on_path else "Start Menu shortcut is ready (PATH entry was already there).")


def uninstall():
    root = install_root()
    shortcut = shortcut_path()
    try:
        shortcut.unlink()
    except FileNotFoundError:
        pass
    except OSError as error:
        sys.exit(f"uninstall: cannot remove {shortcut}: {error}")
    if remove_path_entry(root):
        print("Removed the PATH entry.")
    if not root.exists():
        print("Nothing else to remove.")
        return
    try:
        remove_tree(root)
    except PermissionError:
        sys.exit(f"uninstall: cannot remove {root} - close Blockloom first and try again")
    print(f"Removed {root}")


def install_hub():
    binary = release_dir() / "blockloom-hub.exe"
    if not binary.is_file():
        sys.exit(f"install: {binary} is missing - run just hub-build first")
    root = install_root("Blockloom Hub")
    install_file(binary, root / binary.name)
    create_shortcut(root / binary.name, shortcut_path("Blockloom Hub"))
    add_path_entry(root)
    print(f"Installed Blockloom Hub to {root}")


def main(argv):
    commands = {"install": install, "uninstall": uninstall, "hub-install": install_hub}
    if len(argv) != 1 or argv[0] not in commands:
        print("usage: install-windows.py install|uninstall|hub-install", file=sys.stderr)
        return 2
    if os.name != "nt":
        sys.exit("install-windows.py only runs on Windows")
    commands[argv[0]]()
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main(sys.argv[1:]))
    except OSError as error:
        sys.exit(f"install-windows: {error}")
