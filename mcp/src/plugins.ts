//! Plugin commands as MCP tools.
//!
//! The shell's `plugin-commands` lists what the open project's installed
//! plugins contribute, with typed arguments. Each becomes a tool of its own,
//! so installing a plugin makes its commands available without this server
//! knowing about it. A tool is run as the shell line `plugin-id/name {json}`.

import { z } from "zod";

/// One plugin command argument, as the plugin's schema serializes it.
export interface PluginField {
  name: string;
  type: string;
  min?: number;
  max?: number;
  max_len?: number;
  options?: string[];
  item?: PluginField;
  default?: unknown;
  description?: string;
  /// A struct's fields.
  fields?: PluginField[];
  /// A map's value type.
  value?: PluginField;
  /// An optional's inner type.
  inner?: PluginField;
  /// An enum's tag key (empty for serde's external form) and its variants.
  tag?: string;
  variants?: PluginVariant[];
}

/// One shape of an `enum` field.
export interface PluginVariant {
  name: string;
  fields?: PluginField[];
  inner?: PluginField;
}

/// One entry of `plugin-commands`.
export interface PluginCommand {
  name: string;
  plugin: string;
  summary: string;
  args: PluginField[];
}

/// MCP tool names allow letters, digits, `_`, `-` and `.`; a command's
/// `plugin-id/name` becomes `plugin-id__name`.
export function pluginToolName(command: string): string {
  return command.replace("/", "__").replace(/[^A-Za-z0-9_.-]/g, "_");
}

/// A struct's fields as an object; a field with a default may be left out.
/// Unknown keys pass through, since the backend keeps them.
function objectSchema(fields: PluginField[]): z.ZodType {
  const shape: Record<string, z.ZodType> = {};
  for (const field of fields) {
    const schema = typeSchema(field);
    shape[field.name] = field.default !== undefined ? schema.optional() : schema;
  }
  return z.object(shape).passthrough();
}

function variantSchema(variant: PluginVariant, tag: string): z.ZodType {
  const fields = variant.fields ?? [];
  if (tag) {
    const body = objectSchema(fields) as z.ZodObject<Record<string, z.ZodType>>;
    return body.extend({ [tag]: z.literal(variant.name) });
  }
  if (!fields.length && !variant.inner) return z.literal(variant.name);
  const payload = variant.inner ? typeSchema(variant.inner) : objectSchema(fields);
  return z.object({ [variant.name]: payload });
}

/// The zod schema for one field type: plugin schemas and the built-in type
/// registry (`type-registry`) speak the same field types.
export function typeSchema(field: PluginField): z.ZodType {
  switch (field.type) {
    case "bool":
      return z.boolean();
    case "int": {
      let schema = z.number().int();
      if (field.min !== undefined) schema = schema.min(field.min);
      if (field.max !== undefined) schema = schema.max(field.max);
      return schema;
    }
    case "number": {
      let schema = z.number();
      if (field.min !== undefined) schema = schema.min(field.min);
      if (field.max !== undefined) schema = schema.max(field.max);
      return schema;
    }
    case "text": {
      const schema = z.string();
      return field.max_len === undefined ? schema : schema.max(field.max_len);
    }
    case "color":
    case "asset":
    case "actor":
      return z.string();
    case "vec3":
      return z.array(z.number()).length(3);
    case "choice":
      return z.enum((field.options ?? []) as [string, ...string[]]);
    case "list": {
      const schema = z.array(field.item ? typeSchema(field.item) : z.unknown());
      return field.max_len === undefined ? schema : schema.max(field.max_len);
    }
    case "struct":
      return objectSchema(field.fields ?? []);
    case "map":
      return z.record(z.string(), field.value ? typeSchema(field.value) : z.unknown());
    case "optional":
      return (field.inner ? typeSchema(field.inner) : z.unknown()).nullable();
    case "enum": {
      const options = (field.variants ?? []).map((v) => variantSchema(v, field.tag ?? ""));
      if (options.length === 0) return z.unknown();
      if (options.length === 1) return options[0];
      return z.union(options as [z.ZodType, z.ZodType, ...z.ZodType[]]);
    }
    case "opaque":
      return z.unknown();
    default:
      throw new Error(`unknown plugin field type "${field.type}"; teach mcp/src/plugins.ts about it`);
  }
}

/// The zod shape a plugin command's tool validates against. An argument with a
/// default may be left out; the host fills it in.
export function pluginToolSchema(command: PluginCommand): Record<string, z.ZodType> {
  const shape: Record<string, z.ZodType> = {};
  for (const field of command.args) {
    let schema = typeSchema(field);
    if (field.description) schema = schema.describe(field.description);
    if (field.default !== undefined) schema = schema.optional();
    shape[field.name] = schema;
  }
  return shape;
}

export function pluginToolDescription(command: PluginCommand): string {
  return `${command.summary} (command ${command.name} of plugin ${command.plugin}.)`;
}

/// Commands that can change which plugin commands exist.
export function changesPluginCommands(tool: string): boolean {
  return (
    tool.startsWith("plugin-") ||
    ["open-project", "create-project", "close-project", "undo", "redo"].includes(tool)
  );
}
