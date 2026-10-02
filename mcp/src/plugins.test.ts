//! Plugin commands as tools: names, schemas and when the tool list is stale.

import { test } from "node:test";
import assert from "node:assert/strict";
import { z } from "zod";

import {
  changesPluginCommands,
  pluginToolName,
  pluginToolSchema,
  type PluginCommand,
} from "./plugins.js";

const command: PluginCommand = {
  name: "com.example.health/set_hp",
  plugin: "com.example.health",
  summary: "Set hit points.",
  args: [
    { name: "actor", type: "actor" },
    { name: "value", type: "int", min: 0, max: 100 },
    { name: "mode", type: "choice", options: ["set", "add"], default: "set" },
    { name: "tint", type: "color", default: "#ffffff" },
    { name: "pos", type: "vec3", default: [0, 0, 0] },
    { name: "tags", type: "list", item: { name: "tag", type: "text", max_len: 4 }, max_len: 2, default: [] },
  ],
};

test("a plugin command's tool name keeps the id and is a valid MCP name", () => {
  assert.equal(pluginToolName("com.example.health/set_hp"), "com.example.health__set_hp");
  assert.match(pluginToolName("a.b/c d"), /^[A-Za-z0-9_.-]+$/);
});

test("the schema follows the plugin's field types and bounds", () => {
  const schema = z.object(pluginToolSchema(command));
  assert.ok(schema.safeParse({ actor: "a", value: 50 }).success, "defaults may be left out");
  assert.ok(
    schema.safeParse({
      actor: "a",
      value: 5,
      mode: "add",
      tint: "#00ff00",
      pos: [1, 2, 3],
      tags: ["ab", "cd"],
    }).success,
  );
  assert.ok(!schema.safeParse({ value: 5 }).success, "a field with no default is required");
  assert.ok(!schema.safeParse({ actor: "a", value: 101 }).success, "above max");
  assert.ok(!schema.safeParse({ actor: "a", value: 1.5 }).success, "an int is whole");
  assert.ok(!schema.safeParse({ actor: "a", value: 1, mode: "mul" }).success, "not an option");
  assert.ok(!schema.safeParse({ actor: "a", value: 1, pos: [1, 2] }).success, "vec3 has three");
  assert.ok(!schema.safeParse({ actor: "a", value: 1, tags: ["a", "b", "c"] }).success);
  assert.ok(!schema.safeParse({ actor: "a", value: 1, tags: ["abcde"] }).success);
});

test("an unknown field type fails loudly", () => {
  assert.throws(
    () => pluginToolSchema({ ...command, args: [{ name: "x", type: "hologram" }] }),
    /hologram/,
  );
});

test("only project and plugin changes make the tool list stale", () => {
  for (const tool of ["plugin-install", "plugin-remove", "open-project", "undo"]) {
    assert.ok(changesPluginCommands(tool), tool);
  }
  for (const tool of ["get-state", "add-actor"]) {
    assert.ok(!changesPluginCommands(tool), tool);
  }
});
