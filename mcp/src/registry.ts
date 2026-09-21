//! Turning the shell's command registry into MCP tool schemas.
//!
//! The registry's arg types are human prose (what `help` shows); this maps
//! every one of them onto a zod schema. The map is exhaustive on purpose: a
//! new or renamed prose type fails loudly at startup rather than silently
//! sending the agent a string-typed slot it cannot fill.
//!
//! Strictness ends at the field boundary - the shell re-validates every
//! argument and answers with its own errors, so an object-shaped arg accepts
//! any object and the shell's `{ok, error}` decides what happens. The schema
//! is here to keep the agent from inventing fields, not to second-guess the
//! backend.

import { z } from "zod";
import type { ShellCommandSpec } from "./shell.js";

export type ToolShape = Record<string, z.ZodType>;

const STRING_KINDS = new Set([
  "id",
  "component name",
  "kind name",
  "target triple (default: this machine)",
  "folder path",
  "folder, default the root",
  "parent folder",
  "file path",
  "file name",
  "asset path",
  "image asset path (blank clears)",
  "new name",
  "string",
]);

const OPEN_OBJECT = z.record(z.string(), z.unknown());

function proseToSchema(ty: string): z.ZodType {
  // A "Road|Sky" prose type is a dropdown - an enum of the choices.
  if (ty.includes("|") && !ty.startsWith("[") && !ty.startsWith("{")) {
    return z.enum(ty.split("|").map((choice) => choice.trim()) as [string, ...string[]]);
  }
  if (STRING_KINDS.has(ty)) return z.string();
  if (ty === "number") return z.number();
  if (ty === "bool" || ty === "boolean (default: on when available)") return z.boolean();
  if (ty === "#RRGGBB") return z.string().regex(/^#[0-9a-fA-F]{6}$/);
  if (ty === "[x, y, z]") return z.array(z.number());
  if (ty === "[source paths]") return z.array(z.string());
  if (ty.startsWith("[")) return z.array(OPEN_OBJECT);
  if (ty.includes("object") || ty.startsWith("{")) return OPEN_OBJECT;
  throw new Error(`unknown arg type "${ty}"; teach mcp/src/registry.ts about it`);
}

/// The zod shape the MCP client's arguments are validated against. Missing
/// keys stay unset (the shell fills defaults); a `required` arg is a required
/// property, so a client that leaves it out gets a schema error, not a shell error.
export function buildToolSchema(spec: ShellCommandSpec): ToolShape {
  const shape: ToolShape = {};
  for (const arg of spec.args) {
    let schema = proseToSchema(arg.ty).describe(arg.ty);
    if (!arg.required) schema = schema.optional();
    shape[arg.name] = schema;
  }
  return shape;
}

/// The tool description handed to the agent: the command's one-liner, plus a
/// nudge to read state back when a result isn't self-explanatory.
export function toolDescription(spec: ShellCommandSpec): string {
  return spec.name === "get-state"
    ? spec.summary
    : `${spec.summary} Returns the command's own result; run get-state to read the world back.`;
}