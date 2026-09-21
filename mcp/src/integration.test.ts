//! End-to-end test against the real backend: spawn the same shell binary a
//! user drives, in a scratch data dir so the user's projects are untouched.

import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { resolveShell, ShellSession } from "./shell.js";

test("a shell session creates actors and reads state back", async (t) => {
  let shell: string;
  try {
    shell = resolveShell();
  } catch {
    t.skip("no blockloom-shell binary built; run `just build` to exercise this test");
    return;
  }

  const dataDir = mkdtempSync(join(tmpdir(), "blockloom-mcp-"));
  const savedDataDir = process.env.BLOCKLOOM_DATA_DIR;
  process.env.BLOCKLOOM_DATA_DIR = dataDir;
  const session = new ShellSession(shell, ["--no-state"]);
  try {
    const create = await session.run("create-project name=IntProj mode=TwoD");
    assert.equal(create.ok, true, create.error ?? "");

    const add = await session.run("add-actor name=Ball shape=Circle");
    assert.equal(add.ok, true, add.error ?? "");

    const state = await session.run("get-state");
    assert.equal(state.ok, true, state.error ?? "");
    const snapshot = state.result as {
      project: { name: string; actors: { name: string }[] };
    };
    assert.equal(snapshot.project.name, "IntProj", "commands share one session's state");
    assert.ok(
      snapshot.project.actors.some((actor) => actor.name === "Ball"),
      "the new actor is in the world",
    );

    // A second project replaces the first: still one open project per session.
    const replace = await session.run("create-project name=Second mode=TwoD");
    assert.equal(replace.ok, true, replace.error ?? "");
    const after = await session.run("get-state");
    const replaced = (after.result as { project: { name: string } }).project;
    assert.equal(replaced.name, "Second", "the second project took its place");

    const vocab = await session.run("block-vocabulary");
    assert.equal(vocab.ok, true, vocab.error ?? "");
    const parsed = vocab.result as { blocks: unknown[]; reporters: unknown[] };
    assert.ok(parsed.blocks.length > 10, "the block vocabulary is present");

    const bad = await session.run("add-actor name=A shape=Wibble");
    assert.equal(bad.ok, false, "a nonsense shape is an error, not a crash");
  } finally {
    session.close();
    if (savedDataDir === undefined) delete process.env.BLOCKLOOM_DATA_DIR;
    else process.env.BLOCKLOOM_DATA_DIR = savedDataDir;
    rmSync(dataDir, { recursive: true, force: true });
  }
});