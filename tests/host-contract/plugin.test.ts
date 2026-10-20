import { expect, test } from "bun:test";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

// Whether the installed Claude Code loads the harness plugin (docs/api.md, 「The harness plugin」): `bmo-pocket mcp`
// started as a standard input and output server with a tool session's token, its four fixed tools and the hosted
// tools of the session's namespaces among the tools the program is handed. No CLI is named in the suite, so this
// skips there; `BMO_LIVE_CLAUDE` names the installed one to run it. It runs in a configuration directory of its own
// with no login, so the program connects its servers and stops before asking any model anything. The binaries are
// the workspace's build, or the ones `MCP_HOST_BINARY` and `POCKET_BINARY` name.

const cli = process.env.BMO_LIVE_CLAUDE ?? "";
const host = process.env.MCP_HOST_BINARY ?? resolve(import.meta.dir, "../../target/debug/mcp-host");
const pocket = process.env.POCKET_BINARY ?? resolve(import.meta.dir, "../../target/debug/bmo-pocket");

// A placed tool standing in: one read-only entry in the namespace `fixture`.
function placeFixture(tools: string): void {
  const placed = join(tools, "echo", ".placed");

  mkdirSync(placed, { recursive: true });
  writeFileSync(
    join(placed, "tool.toml"),
    `[tool]
name = "echo"
protocol = 1
run = ["python3", "echo.py"]
timeout = 20

[[entry]]
namespace = "fixture"
name = "echo_back"
description = "Says the text back."
input = "echo.json"
capabilities = ["fixture.read"]
hints = { read_only = true }
`,
  );
  writeFileSync(join(placed, "echo.json"), JSON.stringify({ type: "object", properties: { text: { type: "string" } } }));
  writeFileSync(join(placed, "echo.py"), `import json, sys\njson.load(sys.stdin)\nprint(json.dumps({"content": [], "isError": False}))\n`);
}

test.skipIf(cli === "" || !existsSync(cli))("Claude Code starts bmo-pocket mcp with a session's token and is handed its tools", async () => {
  const scratch = mkdtempSync(join(tmpdir(), "pocket-plugin-"));
  const sockets = mkdtempSync("/tmp/pp-");
  const socket = join(sockets, "c.sock");
  const config = join(scratch, "config");
  const tools = join(scratch, "tools");

  placeFixture(tools);

  const served = Bun.spawn([host, "serve", "--port", "0", "--data-root", join(scratch, "data"), "--control", socket, "--tools", tools], {
    stdout: "pipe",
    stderr: "ignore",
  });

  try {
    await served.stdout.getReader().read();

    const issuer = readFileSync(join(scratch, "data", "tool-host", "issuer-token"), "utf8").trim();
    const issued = await fetch("http://localhost/tool-sessions", {
      unix: socket,
      method: "POST",
      headers: { Authorization: `Bearer ${issuer}`, "Content-Type": "application/json" },
      body: JSON.stringify({ holderName: "tester", namespaces: ["fixture"], capabilities: ["fixture.read"], grants: [] }),
    });
    const { token } = (await issued.json()) as { token: string };
    const file = join(scratch, "pocket.mcp.json");

    mkdirSync(config, { recursive: true });
    writeFileSync(
      file,
      JSON.stringify({ mcpServers: { pocket: { type: "stdio", command: pocket, args: ["mcp", "--socket", socket], env: { BMO_POCKET_TOKEN: token } } } }),
    );

    const run = Bun.spawn([cli, "-p", "OK", "--output-format", "stream-json", "--verbose", "--strict-mcp-config", "--mcp-config", file, "--max-turns", "1"], {
      cwd: scratch,
      env: { PATH: process.env.PATH ?? "", HOME: process.env.HOME ?? scratch, CLAUDE_CONFIG_DIR: config, ANTHROPIC_API_KEY: "" },
      stdin: "ignore",
      stdout: "pipe",
      stderr: "ignore",
    });
    const opening = JSON.parse((await new Response(run.stdout).text()).split("\n")[0] ?? "{}") as {
      mcp_servers?: { name: string; status: string }[];
      tools?: string[];
    };

    await run.exited;

    expect(opening.mcp_servers?.find((server) => server.name === "pocket")?.status).toBe("connected");

    for (const tool of ["pocket_namespaces", "pocket_tools", "pocket_tool", "pocket_call", "fixture_echo_back"]) {
      expect(opening.tools ?? []).toContain(`mcp__pocket__${tool}`);
    }
  } finally {
    served.kill();
    await served.exited;
    rmSync(scratch, { recursive: true, force: true });
    rmSync(sockets, { recursive: true, force: true });
  }
}, 120_000);
