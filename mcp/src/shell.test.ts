//! Staging tests: the server drives a scratch copy of blockloom-shell, never
//! the build tree's binary, so a running agent can't lock `cargo build` out.

import { test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { resolveShell, stageDir, stageShell, unstageShell } from "./shell.js";

function sourceOrSkip(t: { skip: (msg?: string) => void }): string | null {
  try {
    return resolveShell();
  } catch {
    t.skip("no blockloom-shell binary built; run `just build` to exercise this test");
    return null;
  }
}

test("staging copies the binary aside and cleans up after itself", (t) => {
  const source = sourceOrSkip(t);
  if (!source) return;
  t.after(() => unstageShell());

  const staged = stageShell(source);
  assert.notEqual(staged, source, "the child must not run the build tree's file");
  assert.ok(staged.startsWith(tmpdir()), "the copy lives in scratch space");
  assert.ok(existsSync(staged), "the copy exists");
  assert.deepEqual(
    readFileSync(staged),
    readFileSync(source),
    "the copy is the binary, byte for byte",
  );

  unstageShell();
  assert.ok(!existsSync(stageDir()), "unstaging removes the scratch dir");
});

test("staging twice refreshes one dir, and sweeping spares the living", (t) => {
  const source = sourceOrSkip(t);
  if (!source) return;
  t.after(() => unstageShell());

  // A stage dir for a pid that can never exist: swept on the next stage.
  const dead = join(tmpdir(), "blockloom-mcp-shell-2147483647");
  mkdirSync(dead, { recursive: true });
  const marker = join(dead, "marker");
  writeFileSync(marker, "stale");

  // Our own dir is never swept, even though its owner (this test) is alive:
  // the sweep explicitly skips it, and a second stage reuses it.
  const first = stageShell(source);
  const second = stageShell(source);
  assert.equal(second, first, "one server keeps one staged copy");
  assert.ok(!existsSync(marker), "a dead owner's stage dir is swept");
  assert.ok(existsSync(first), "our own staged copy survives sweeping");
});
