//! blockloom-mcp - a Model Context Protocol server standing on the
//! `blockloom-shell` binary.
//!
//! Every shell command becomes an MCP tool; the agent calls it the same way
//! it would type it, and reads the world back through `get-state` or the
//! `blockloom://state` resource. Each server process owns one shell child,
//! which owns one backend - unless started with `--attach`, which drives the
//! editor's own backend over its attach socket instead, so there is ever one
//! copy of the project. Without `--attach`, don't edit a project that the
//! editor window has open at the same time: the shell attaches to the live
//! owner's files and follows their saves, but two writers still take turns
//! through the revision counter rather than truly sharing one copy.

import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import type { CallToolResult, ReadResourceResult } from "@modelcontextprotocol/sdk/types.js";

import { buildToolSchema, toolDescription } from "./registry.js";
import {
  ShellSession,
  loadSpecs,
  resolveShell,
  stageShell,
  unstageShell,
  type ShellCommandSpec,
} from "./shell.js";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

function packageVersion(): string {
  try {
    const manifest = JSON.parse(
      readFileSync(fileURLToPath(new URL("../package.json", import.meta.url)), "utf8"),
    ) as { version: string };
    return manifest.version;
  } catch {
    return "0.0.0";
  }
}

function usage(): string {
  return [
    "blockloom-mcp - drive Blockloom through the Model Context Protocol",
    "",
    "Launch it from an MCP client as a stdio server; there is nothing to",
    "type at it. Configuration it understands:",
    "",
    "  --shell <path>          Which blockloom-shell to drive (or set",
    "                          BLOCKLOOM_MCP_SHELL).",
    "  --attach                Drive the editor's own backend over its",
    "                          attach socket instead of a private copy.",
    "  --help                  Show this help.",
    "",
  ].join("\n");
}

async function main(): Promise<void> {
  let explicitShell: string | undefined;
  let attach = false;
  const args = process.argv.slice(2);
  for (let i = 0; i < args.length; i++) {
    if (args[i] === "--shell") {
      explicitShell = args[++i];
      if (!explicitShell) {
        console.error("--shell needs a path");
        process.exit(2);
      }
    } else if (args[i] === "--attach") {
      attach = true;
    } else if (args[i] === "--help" || args[i] === "-h") {
      process.stdout.write(usage());
      return;
    } else {
      console.error(`Unknown argument: ${args[i]}`);
      process.stderr.write(usage());
      process.exit(2);
    }
  }

  let shell: string;
  try {
    shell = resolveShell(explicitShell);
  } catch (error) {
    console.error(String(error));
    process.exit(1);
  }

  // A staged copy, not the build tree's binary: a running executable locks
  // its own file on Windows, and driving it in place would fail every cargo
  // build for as long as this server lives.
  const staged = stageShell(shell);
  const specs = await loadSpecs(staged);
  const sessionArgs = attach ? ["--no-state", "--attach"] : ["--no-state"];
  const session = new ShellSession(staged, sessionArgs);
  // stderr only: stdout is the protocol and must stay quiet.
  console.error(
    "blockloom-mcp: driving " +
      staged +
      " (staged copy of " +
      shell +
      ", so cargo builds stay unlocked) with " +
      specs.length +
      " commands" +
      (attach ? " attached to the editor's backend" : " on a private copy") +
      ". " +
      (attach
        ? "One copy of any project it opens; the window and the agent share it."
        : "This session has its own copy of any project it opens; " +
          "don't edit the same project from the window or another agent at once."),
  );

  const server = new McpServer({ name: "blockloom", version: packageVersion() });

  const registerTools = () => {
    for (const spec of specs) {
      const description = toolDescription(spec);
      const run = async (args: Record<string, unknown>) =>
        invoke(session, spec, args ?? {});
      if (spec.args.length === 0) {
        server.registerTool(spec.name, { title: spec.name, description }, () =>
          run({}),
        );
      } else {
        const inputSchema = buildToolSchema(spec);
        server.registerTool(
          spec.name,
          { title: spec.name, description, inputSchema },
          (args) => run(args ?? {}),
        );
      }
    }
  };

  const registerResources = () => {
    server.resource("state", "blockloom://state", async () => {
      const response = await session.run("get-state");
      return readResource(response, "blockloom://state");
    });
    server.resource("blocks", "blockloom://blocks", async () => {
      const response = await session.run("block-vocabulary");
      return readResource(response, "blockloom://blocks");
    });
  };

  registerTools();
  registerResources();

  const transport = new StdioServerTransport();
  await server.connect(transport);

  const shutdown = () => {
    session.close();
    unstageShell();
    process.exit(0);
  };
  process.on("SIGINT", shutdown);
  process.on("SIGTERM", shutdown);
  process.on("exit", () => unstageShell());
}

function readResource(
  response: { ok: boolean; result: unknown; error: string | null },
  uri: string,
): ReadResourceResult {
  const text = response.ok ? JSON.stringify(response.result) : `error: ${response.error}`;
  return { contents: [{ uri, text }] };
}

/// Runs one command and turns the shell's `{ok, result, error}` into an MCP
/// tool result. JSON in the content field, the parsed object in
/// `structuredContent` for clients that can use it directly.
async function invoke(
  session: ShellSession,
  spec: ShellCommandSpec,
  args: Record<string, unknown>,
): Promise<CallToolResult> {
  const line = spec.name + " " + JSON.stringify(args ?? {});
  try {
    const response = await session.run(line);
    if (response.ok) {
      const structuredContent =
        response.result !== null &&
        typeof response.result === "object" &&
        !Array.isArray(response.result)
          ? (response.result as Record<string, unknown>)
          : undefined;
      return {
        content: [{ type: "text", text: JSON.stringify(response.result) }],
        isError: false,
        structuredContent,
      };
    }
    return {
      content: [{ type: "text", text: response.error ?? "error" }],
      isError: true,
    };
  } catch (error) {
    return {
      content: [{ type: "text", text: String(error) }],
      isError: true,
    };
  }
}

main().catch((error) => {
  console.error(`blockloom-mcp: ${error}`);
  process.exit(1);
});