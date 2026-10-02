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

function typeSchema(field: PluginField): z.ZodType {
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
