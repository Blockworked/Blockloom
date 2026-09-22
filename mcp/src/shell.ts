//! The blockloom-shell child: how the host talks to a backend.
//!
//! One shell command per line in, one JSON response per line out, both on
//! stdio. The shell is the same binary an agent drives by hand, so the MCP
//! tools are exactly its commands: no second backend to keep in step.

import { execFile, spawn, type ChildProcessByStdio } from "node:child_process";
import { chmodSync, copyFileSync, existsSync, mkdirSync, readdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { delimiter, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export interface ShellArgSpec {
  name: string;
  ty: string;
  required: boolean;
}

export interface ShellCommandSpec {
  name: string;
  cmd: string;
  aliases: string[];
  summary: string;
  args: ShellArgSpec[];
}

export interface ShellResponse {
  ok: boolean;
  result: unknown;
  error: string | null;
  state: unknown;
}

function shellFileName(): string {
  return process.platform === "win32" ? "blockloom-shell.exe" : "blockloom-shell";
}

/// The workspace root, three levels up from this file (dist/or src -> mcp -> workspace).
export function repoRoot(): string {
  return resolve(fileURLToPath(import.meta.url), "..", "..", "..");
}

/// Where to find the shell binary, in the order an install would try it.
export function resolveShell(explicit?: string): string {
  if (explicit) return explicit;
  if (process.env.BLOCKLOOM_MCP_SHELL) return process.env.BLOCKLOOM_MCP_SHELL;

  const fromRepo = (profile: string) => join(repoRoot(), "target", profile, shellFileName());
  const candidates = [fromRepo("debug"), fromRepo("release")];
  for (const dir of (process.env.PATH ?? "").split(delimiter).filter(Boolean)) {
    candidates.push(join(dir, shellFileName()));
  }
  for (const candidate of candidates) {
    if (existsSync(candidate)) return candidate;
  }
  throw new Error(
    "couldn't find blockloom-shell. Build it with `just build`, or point " +
      "BLOCKLOOM_MCP_SHELL at the binary.",
  );
}

/// Where this process's staged shell copy lives: one dir per server process,
/// so a fresh start refreshes its own copy and never touches another's.
export function stageDir(): string {
  return join(tmpdir(), `blockloom-mcp-shell-${process.pid}`);
}

/// Copy the shell binary to a scratch file and return the copy's path.
/// A running executable locks its own file against replacement on Windows,
/// so driving the build tree's binary in place would fail every `cargo
/// build` for as long as the child lives. The copy is as fresh as the last
/// server start; rebuilding mid-session still needs a server restart before
/// the agent sees the new binary, but the build itself is never blocked.
export function stageShell(source: string): string {
  sweepStaleStages();
  const dir = stageDir();
  mkdirSync(dir, { recursive: true });
  const staged = join(dir, shellFileName());
  try {
    copyFileSync(source, staged);
  } catch (error) {
    throw new Error(
      `couldn't stage blockloom-shell (is a build writing it right now?): ${error}`,
    );
  }
  if (process.platform !== "win32") chmodSync(staged, 0o755);
  return staged;
}

/// Remove this process's staged copy. Best effort: exit paths must not throw.
export function unstageShell(): void {
  try {
    rmSync(stageDir(), { recursive: true, force: true });
  } catch {
    // Gone already, or never staged.
  }
}

/// Drop stage dirs whose owner is dead, so crashed servers don't accumulate
/// copies. Anything ambiguous is left alone.
function sweepStaleStages(): void {
  let entries: string[];
  try {
    entries = readdirSync(tmpdir());
  } catch {
    return;
  }
  const prefix = "blockloom-mcp-shell-";
  for (const entry of entries) {
    if (!entry.startsWith(prefix)) continue;
    const pid = Number(entry.slice(prefix.length));
    if (!Number.isInteger(pid) || pid === process.pid) continue;
    if (pidAlive(pid)) continue;
    try {
      rmSync(join(tmpdir(), entry), { recursive: true, force: true });
    } catch {
      // Someone else's mess, or still in use; leave it.
    }
  }
}

/// True when a process with this pid exists. EPERM means "alive but not
/// ours", which for sweeping purposes counts as alive.
function pidAlive(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    return (error as NodeJS.ErrnoException)?.code === "EPERM";
  }
}

/// The whole command registry, from `blockloom-shell --specs`. Run once at
/// startup so the tools can't drift from the commands they drive.
export function loadSpecs(shell: string): Promise<ShellCommandSpec[]> {
  return new Promise((resolveSpecs, reject) => {
    execFile(
      shell,
      ["--specs"],
      { encoding: "utf8", maxBuffer: 1 << 24 },
      (error, stdout, stderr) => {
        if (error) {
          reject(new Error(`blockloom-shell --specs failed: ${stderr || error}`));
          return;
        }
        try {
          const parsed = JSON.parse(stdout) as { commands: ShellCommandSpec[] };
          resolveSpecs(parsed.commands);
        } catch (e) {
          reject(new Error(`blockloom-shell --specs returned unreadable JSON: ${e}`));
        }
      },
    );
  });
}

export type ShellChildProcess = ChildProcessByStdio<
  import("node:stream").Writable,
  import("node:stream").Readable,
  null
>;

/// A persistent shell process. Each session owns one backend, so state lives
/// between calls and commands run strictly one at a time.
export class ShellSession {
  private child: ShellChildProcess;
  private buffer = "";
  private lines: string[] = [];
  private chain: Promise<unknown> = Promise.resolve();
  private exited = false;
  private exitReason = "";

  constructor(shell: string, args: string[] = []) {
    this.child = spawn(shell, args, {
      stdio: ["pipe", "pipe", "inherit"],
      windowsHide: true,
    });
    this.child.stdout.setEncoding("utf8");
    this.child.stdout.on("data", (chunk: string) => {
      this.buffer += chunk;
      let newline = this.buffer.indexOf("\n");
      while (newline >= 0) {
        this.lines.push(this.buffer.slice(0, newline).replace(/\r$/, ""));
        this.buffer = this.buffer.slice(newline + 1);
        newline = this.buffer.indexOf("\n");
      }
    });
    this.child.on("exit", (code, signal) => {
      this.exited = true;
      if (this.buffer !== "" || this.lines.length > 0) {
        this.lines.push(this.buffer); // a final un-terminated line
        this.buffer = "";
      }
      this.exitReason = `blockloom-shell exited (${code ?? signal})`;
    });
    this.child.on("error", (error) => {
      this.exited = true;
      this.exitReason = `couldn't start blockloom-shell: ${error.message}`;
    });
  }

  /// Serialized behind `chain` so two tool calls never interleave on the wire.
  run(line: string): Promise<ShellResponse> {
    const task = this.chain.then(() => this.runNow(line));
    this.chain = task.catch(() => undefined);
    return task;
  }

  private runNow(line: string): Promise<ShellResponse> {
    if (this.exited) {
      return Promise.reject(new Error(`${this.exitReason}; retry after a fresh session`));
    }
    this.child.stdin.write(line + "\n", "utf8");
    return this.readResponse().then((text) => {
      try {
        const parsed = JSON.parse(text) as ShellResponse;
        if (!parsed || typeof parsed !== "object" || !("ok" in parsed)) {
          throw new Error(`blockloom-shell answered something unreadable: ${text}`);
        }
        return parsed;
      } catch (e) {
        if (e instanceof SyntaxError) {
          // A stray WARN line on stdout, not a response; keep reading.
          return this.runNow(line);
        }
        throw e;
      }
    });
  }

  /// The next line that parses as a `{ok, ...}` response. Any other line is
  /// shell noise and is skipped on the way.
  private readResponse(): Promise<string> {
    return new Promise((resolveResponse, reject) => {
      const tryRead = () => {
        while (this.lines.length > 0) {
          const line = this.lines.shift()!;
          try {
            const parsed: unknown = JSON.parse(line);
            if (parsed && typeof parsed === "object" && "ok" in (parsed as object)) {
              resolveResponse(line);
              return;
            }
          } catch {
            // Not JSON: noise, skip it.
          }
        }
        const onData = () => {
          cleanup();
          tryRead();
        };
        const onExit = () => {
          cleanup();
          if (this.lines.length > 0) {
            tryRead();
            return;
          }
          reject(new Error(`${this.exitReason}; retry after a fresh session`));
        };
        const cleanup = () => {
          this.child.stdout.off("data", onData);
          this.child.off("exit", onExit);
        };
        this.child.stdout.once("data", onData);
        this.child.once("exit", onExit);
      };
      tryRead();
    });
  }

  close() {
    try {
      this.child.kill();
    } catch {
      // Already gone.
    }
  }
}