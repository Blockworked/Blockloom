//! Schema-mapping tests: the registry turns prose command args into zod
//! schemas, and it must know every prose shape the shell actually emits.

import { test } from "node:test";
import assert from "node:assert/strict";
import { z } from "zod";

import { buildToolSchema } from "./registry.js";
import { loadSpecs, resolveShell, stageShell, unstageShell, type ShellCommandSpec } from "./shell.js";

function parseWith(spec: ShellCommandSpec, args: unknown) {
  const shape = buildToolSchema(spec);
  return z.object(shape).safeParse(args);
}

const fixtureSpec = {
  name: "fixture",
  cmd: "fixture",
  aliases: [],
  summary: "synthetic",
  args: [
    { name: "tone", ty: "say|error", required: true },
    { name: "pos", ty: "[x, y, z]", required: true },
    { name: "scene", ty: "block object", required: false },
    { name: "color", ty: "#RRGGBB", required: false },
    { name: "count", ty: "number", required: false },
    { name: "on", ty: "bool", required: false },
    { name: "tag", ty: "string", required: false },
  ],
};

test("every prose category maps to a schema that accepts its own kind of value", () => {
  const good = parseWith(fixtureSpec, {
    tone: "error",
    pos: [1, -2.5, 0],
    scene: { instruction: { kind: "Move", steps: { kind: "Number", value: 100 } } },
    color: "#0a1b2c",
    count: 3,
    on: true,
    tag: "x",
  });
  assert.equal(good.success, true, JSON.stringify(good));

  const bad = [
    { tone: "loud", pos: [1, 2, 3] }, // not a say|error choice
    { tone: "error", pos: [1, "x", 3] }, // xyz must be numbers
    { tone: "error", pos: [1, 2, 3], color: "red" }, // not a hex color
    { pos: [1, 2, 3] }, // tone is required
    { tone: "say", pos: [1, 2, 3], count: "lots" }, // count is a number
  ];
  for (const args of bad) {
    const result = parseWith(fixtureSpec, args);
    assert.equal(result.success, false, `accepted bad args: ${JSON.stringify(args)}`);
  }
});

test("an unknown prose type fails loudly, not silently", () => {
  assert.throws(() =>
    buildToolSchema({
      name: "n",
      cmd: "n",
      aliases: [],
      summary: "s",
      args: [{ name: "x", ty: "wibble", required: false }],
    }),
  );
});

test("every real command in the shell registry maps to a schema", async (t) => {
  let shell: string;
  try {
    shell = stageShell(resolveShell());
  } catch {
    t.skip("no blockloom-shell binary built; run `just build` to exercise this test");
    return;
  }
  t.after(() => unstageShell());
  const specs = await loadSpecs(shell);
  assert.ok(specs.length >= 50, `expected a real registry, got ${specs.length} commands`);
  for (const spec of specs) {
    assert.doesNotThrow(() => buildToolSchema(spec), `schema failed for ${spec.name}`);
  }

  // Spot-check a few well-known shapes against the real spec.
  const create = specs.find((s) => s.name === "create-project")!;
  const createShape = buildToolSchema(create);
  assert.ok("name" in createShape, "create-project takes a name");
  assert.ok("mode" in createShape, "create-project takes a mode");

  const strand = specs.find((s) => s.name === "add-strand")!;
  const strandShape = buildToolSchema(strand);
  assert.ok(
    "instruction" in strandShape && "x" in strandShape && "y" in strandShape,
    "add-strand takes x, y and an instruction",
  );
});