//! End-to-end test of the real MCP server: spawn the built `dist/index.js`
//! over stdio and call tools through an SDK client, so the full `invoke`
//! result path - text content and structuredContent - is exercised.
//! Regression guard: a command whose result is an array (list-assets) must
//! come back as a valid tool result, because arrays are not valid
//! structuredContent and used to break the call.

import { test } from "node:test";
import assert from "node:assert/strict";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StdioClientTransport } from "@modelcontextprotocol/sdk/client/stdio.js";
import { existsSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { repoRoot, resolveShell } from "./shell.js";

test("an array-returning command stays a valid tool result", async (t) => {
  let shell: string;
  try {
    shell = resolveShell();
  } catch {
    t.skip("no blockloom-shell binary built; run `just build` to exercise this test");
    return;
  }
  const serverScript = join(repoRoot(), "mcp", "dist", "index.js");
  if (!existsSync(serverScript)) {
    t.skip("mcp/dist/index.js not built; run `pnpm run build` in mcp/ to exercise this test");
    return;
  }

  const dataDir = mkdtempSync(join(tmpdir(), "blockloom-mcp-server-"));
  const savedDataDir = process.env.BLOCKLOOM_DATA_DIR;
  const savedShell = process.env.BLOCKLOOM_MCP_SHELL;
  process.env.BLOCKLOOM_DATA_DIR = dataDir;
  process.env.BLOCKLOOM_MCP_SHELL = shell;

  const transport = new StdioClientTransport({
    command: process.execPath,
    args: [serverScript],
    env: process.env as Record<string, string>,
  });
  const client = new Client({ name: "blockloom-mcp-test", version: "0.0.0" });
  try {
    await client.connect(transport);

    const create = await client.callTool({
      name: "create-project",
      arguments: { name: "ListProj", mode: "TwoD" },
    });
    assert.equal(create.isError, false, "a project must open before listing");

    const list = await client.callTool({ name: "list-assets", arguments: {} });
    assert.equal(
      list.isError,
      false,
      "list-assets returns an array; it must not be rejected as a result",
    );
    assert.ok(
      Array.isArray(list.content) && list.content.length > 0,
      "the result still carries text content",
    );
  } finally {
    await client.close();
    await transport.close();
    if (savedDataDir === undefined) delete process.env.BLOCKLOOM_DATA_DIR;
    else process.env.BLOCKLOOM_DATA_DIR = savedDataDir;
    if (savedShell === undefined) delete process.env.BLOCKLOOM_MCP_SHELL;
    else process.env.BLOCKLOOM_MCP_SHELL = savedShell;
    rmSync(dataDir, { recursive: true, force: true });
  }
});