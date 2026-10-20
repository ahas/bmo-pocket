import { expect, test } from "bun:test";
import { existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

// Whether the installed Claude Code reaches the tool host through the configuration a harness is handed
// (docs/mcp-host.md, 「Transport」): an `http` server whose `Authorization` header reads the token out of
// `${BMO_MCP_TOKEN}`, and the tools fetched over the protocol revision it negotiates. No CLI is named in the
// suite, so this skips there; `BMO_LIVE_CLAUDE` names the installed one to run it. It runs in a configuration
// directory of its own with no login, so the program connects its servers and stops before asking any model
// anything. The host is the workspace's build, or the one `MCP_HOST_BINARY` names.

const cli = process.env.BMO_LIVE_CLAUDE ?? "";
const binary = process.env.MCP_HOST_BINARY ?? resolve(import.meta.dir, "../../target/debug/mcp-host");

// A placed tool standing in: one read-only entry, its namespace `fixture`, which a record holding its capability
// and showing it by name is listed.
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
  writeFileSync(
    join(placed, "echo.py"),
    `import json, sys
handed = json.load(sys.stdin)
print(json.dumps({"content": [{"type": "text", "text": handed["arguments"].get("text", "")}], "isError": False}))
`,
  );
}

test.skipIf(cli === "" || !existsSync(cli))("Claude Code hands the token its header reads from the environment, and loads the host's tools", async () => {
  const scratch = mkdtempSync(join(tmpdir(), "mcp-header-"));
  const sockets = mkdtempSync("/tmp/mh-");
  const config = join(scratch, "config");
  const tools = join(scratch, "tools");

  placeFixture(tools);

  const host = Bun.spawn(
    [binary, "serve", "--port", "0", "--data-root", join(scratch, "data"), "--control", join(sockets, "c.sock"), "--tools", tools],
    { stdout: "pipe", stderr: "ignore" },
  );

  try {
    const said = new TextDecoder().decode((await host.stdout.getReader().read()).value);
    const port = Number(/127\.0\.0\.1:(\d+)/.exec(said)?.[1]);
    const issuer = readFileSync(join(scratch, "data", "tool-host", "issuer-token"), "utf8").trim();
    const issued = await fetch("http://localhost/tool-sessions", {
      unix: join(sockets, "c.sock"),
      method: "POST",
      headers: { Authorization: `Bearer ${issuer}`, "Content-Type": "application/json" },
      body: JSON.stringify({ holderName: "tester", namespaces: ["fixture"], capabilities: ["fixture.read"], grants: [], shown: ["fixture/echo_back"] }),
    });

    expect(issued.status).toBe(201);

    const { token } = (await issued.json()) as { token: string };
    const file = join(scratch, "fixture.mcp.json");

    mkdirSync(config, { recursive: true });
    writeFileSync(file, JSON.stringify({ mcpServers: { fixture: { type: "http", url: `http://127.0.0.1:${port}/mcp/fixture`, headers: { Authorization: "Bearer ${BMO_MCP_TOKEN}" } } } }));

    const status = async (given: string): Promise<string> => {
      const run = Bun.spawn([cli, "-p", "OK", "--output-format", "stream-json", "--verbose", "--debug", "mcp", "--strict-mcp-config", "--mcp-config", file, "--max-turns", "1"], {
        cwd: scratch,
        env: { PATH: process.env.PATH ?? "", HOME: process.env.HOME ?? scratch, CLAUDE_CONFIG_DIR: config, ANTHROPIC_API_KEY: "", BMO_MCP_TOKEN: given },
        stdin: "ignore",
        stdout: "pipe",
        stderr: "ignore",
      });
      const opening = JSON.parse((await new Response(run.stdout).text()).split("\n")[0] ?? "{}") as { mcp_servers?: { name: string; status: string }[] };

      await run.exited;

      return opening.mcp_servers?.find((server) => server.name === "fixture")?.status ?? "absent";
    };

    expect(await status(token)).toBe("connected");
    expect(await status("not-the-token")).toBe("failed");

    // What the program fetched of the host is said in its debug log, and none of it failed.
    const debug = join(config, "debug");
    const logged = readdirSync(debug).map((name) => readFileSync(join(debug, name), "utf8")).join("\n");

    expect(logged).toContain('MCP server "fixture": Connection established');
    expect(logged).not.toContain('MCP server "fixture" Failed to fetch tools');
  } finally {
    host.kill();
    await host.exited;
    rmSync(scratch, { recursive: true, force: true });
    rmSync(sockets, { recursive: true, force: true });
  }
}, 120_000);
