import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

import { type Pocket, PocketError, pocketClient } from "../index";

// The client against a real host (docs/api.md, 「The control socket」): the workspace's build of `mcp-host`, or the
// one `MCP_HOST_BINARY` names, started on a scratch data root with one placed tool, each route asked through the
// client as a caller asks it.

const binary = process.env.MCP_HOST_BINARY ?? resolve(import.meta.dir, "../../target/debug/mcp-host");

const MANIFEST = `[tool]
name = "almanac"
protocol = 1
run = ["python3", "almanac.py"]
timeout = 20

[[entry]]
namespace = "notes"
name = "read_note"
description = "Says the note back."
input = "note.json"
capabilities = ["notes.read"]
hints = { read_only = true }

[entry.facts]
icon = "i-lucide-notebook"

[[entry.schedule]]
cron = "0 7 * * *"
arguments = { text = "morning" }

[[entry]]
namespace = "notes"
name = "write_note"
description = "Keeps a note."
input = "note.json"
capabilities = ["notes.read"]
grant = "notes.keep"

[[entry]]
namespace = "pages"
name = "read_page"
description = "Reads a page of example.org."
input = "note.json"
capabilities = ["notes.read"]
hints = { read_only = true }

[entry.claims]
hosts = ["example.org"]
`;

const SCRIPT = `import json, sys
handed = json.load(sys.stdin)
if handed["kind"] == "claim":
    print(json.dumps({"arguments": {"page": handed["address"]}}))
else:
    print(json.dumps({"content": [{"type": "text", "text": "noted:" + handed["arguments"].get("text", "")}], "isError": False}))
`;

let scratch = "";
let sockets = "";
let host: ReturnType<typeof Bun.spawn> | undefined;
let issuer: Pocket;

beforeAll(async () => {
  if (!existsSync(binary)) {
    throw new Error(`${binary} is not built: cargo build -p mcp-host`);
  }

  scratch = mkdtempSync(join(tmpdir(), "pocket-client-"));
  sockets = mkdtempSync("/tmp/pc-");

  const placed = join(scratch, "tools", "almanac", ".placed");

  mkdirSync(placed, { recursive: true });
  writeFileSync(join(placed, "tool.toml"), MANIFEST);
  writeFileSync(join(placed, "note.json"), JSON.stringify({ type: "object", properties: { text: { type: "string" } } }));
  writeFileSync(join(placed, "almanac.py"), SCRIPT);

  const socket = join(sockets, "c.sock");

  host = Bun.spawn([binary, "serve", "--port", "0", "--data-root", join(scratch, "data"), "--control", socket, "--tools", join(scratch, "tools")], {
    stdout: "pipe",
    stderr: "ignore",
  });

  const said = new TextDecoder().decode((await (host.stdout as ReadableStream<Uint8Array>).getReader().read()).value);

  expect(said).toContain("listening");
  issuer = pocketClient({ socket, token: readFileSync(join(scratch, "data", "tool-host", "issuer-token"), "utf8").trim() });
});

afterAll(async () => {
  host?.kill();
  await host?.exited;
  rmSync(scratch, { recursive: true, force: true });
  rmSync(sockets, { recursive: true, force: true });
});

// A tool session issued for the namespaces the placed tool declares, under these grants.
async function session(grants: string[] = []): Promise<Pocket> {
  const issued = await issuer.issue({
    holderName: "tester",
    namespaces: ["notes", "pages"],
    capabilities: ["notes.read"],
    grants: grants.map((resource) => ({ resource })),
    ttlMs: 60_000,
  });

  expect(issued.expiresAt).toBe(issued.createdAt + 60_000);
  return pocketClient({ socket: issuer.socket, token: issued.token });
}

// The refusal a call was answered with.
async function refusal(call: Promise<unknown>): Promise<PocketError> {
  try {
    await call;
  } catch (thrown) {
    if (thrown instanceof PocketError) return thrown;
    throw thrown;
  }

  throw new Error("the call was not refused");
}

describe("the TypeScript client", () => {
  test("answers health with the repository's version", async () => {
    const health = await issuer.health();

    expect(health.ready).toBe(true);
    expect(health.version).toBe(JSON.parse(readFileSync(resolve(import.meta.dir, "../package.json"), "utf8")).version);
  });

  test("answers the discipline as text to either token", async () => {
    const said = await issuer.discipline();

    expect(said.startsWith("# Finding and using tools\n")).toBe(true);
    expect(said).toBe(readFileSync(resolve(import.meta.dir, "../../prompts/tool-use.md"), "utf8"));
    expect(await (await session()).discipline()).toBe(said);
  });

  test("issues a session, and a session's token issues nothing", async () => {
    const reader = await session();
    const refused = await refusal(reader.issue({ holderName: "x", namespaces: [], capabilities: [], grants: [] }));

    expect([refused.status, refused.code]).toEqual([403, "forbidden"]);
  });

  test("pages the namespaces and the tools by name", async () => {
    const reader = await session();
    const first = await reader.namespaces({ limit: 1 });

    expect(first.items.map((namespace) => namespace.name)).toEqual(["notes"]);
    expect(first.more).toBe(true);
    expect((await reader.namespaces({ after: "notes" })).items.map((namespace) => namespace.name)).toEqual(["pages"]);
    expect((await reader.namespace("notes")).toolCount).toBe(2);

    const tools = await reader.tools("notes", { limit: 1 });

    expect([tools.items.map((tool) => tool.name), tools.more]).toEqual([["read_note"], true]);
    expect((await reader.tool("notes", "read_note")).icon).toBe("i-lucide-notebook");
    expect((await reader.tool("notes", "write_note")).grant).toBe("notes.keep");
  });

  test("calls a tool under the session's grants, and answers a refusal by its code", async () => {
    const ungranted = await session();
    const granted = await session(["notes.keep"]);
    const made = await granted.call({ namespace: "notes", name: "write_note", input: { text: "a" } });

    expect(made.result.content).toEqual([{ type: "text", text: "noted:a" }]);
    expect(typeof made.spentMs).toBe("number");

    const refused = await refusal(ungranted.call({ namespace: "notes", name: "write_note", input: { text: "a" } }));

    expect([refused.status, refused.code]).toEqual([403, "forbidden"]);
    expect(refused.message).toContain("notes.keep");
    expect((await refusal(ungranted.tool("notes", "nope"))).code).toBe("not-found");
  });

  test("claims an address by the entry that reads it", async () => {
    const reader = await session();
    const claimed = await reader.claim("https://www.example.org/a");

    expect(claimed).toEqual({
      resource: "https://www.example.org/a",
      granted: true,
      reason: null,
      namespace: "pages",
      toolName: "read_page",
      arguments: { page: "https://www.example.org/a" },
    });
    expect((await reader.claim("https://example.com/a")).granted).toBe(false);
  });

  test("lists the schedules the session sees", async () => {
    const reader = await session();
    const page = await reader.schedules();

    expect(page.items.map((schedule) => [schedule.namespace, schedule.toolName, schedule.cron])).toEqual([["notes", "read_note", "0 7 * * *"]]);
    expect((await reader.schedules({ after: page.items[0]!.nextAt })).items).toEqual([]);
  });

  test("lists the sessions standing and the external servers to the issuer alone", async () => {
    const reader = await session();
    const standing = await issuer.toolSessions();

    expect(standing.items.length).toBeGreaterThan(0);
    expect(standing.items.every((one) => one.holderName === "tester" && !("token" in one))).toBe(true);
    expect((await issuer.servers()).items).toEqual([]);
    expect((await refusal(reader.toolSessions())).code).toBe("forbidden");
    expect((await refusal(reader.servers())).code).toBe("forbidden");
  });

  test("reads one standing session by its token, with the servers its clients reached", async () => {
    const issued = await issuer.issue({ holderName: "tester", namespaces: ["notes"], capabilities: [], grants: [] });
    const read = await issuer.toolSession(issued.token);

    expect([read.holderName, read.createdAt, read.connected]).toEqual(["tester", issued.createdAt, []]);
    expect((await refusal(pocketClient({ socket: issuer.socket, token: issued.token }).toolSession(issued.token))).code).toBe("forbidden");
    await issuer.revoke(issued.token);
    expect((await refusal(issuer.toolSession(issued.token))).code).toBe("not-found");
  });

  test("tells a tool placed while the host runs as one event of the stream", async () => {
    const placed = join(scratch, "tools", "ledger", ".placed");

    mkdirSync(placed, { recursive: true });
    writeFileSync(join(placed, "note.json"), JSON.stringify({ type: "object" }));
    writeFileSync(join(placed, "almanac.py"), SCRIPT);
    writeFileSync(
      join(placed, "tool.toml"),
      MANIFEST.split("[[entry]]")[0]!.replace('name = "almanac"', 'name = "ledger"') +
        '[[entry]]\nnamespace = "ledger"\nname = "keep"\ndescription = "Keeps a line. Then says so."\ninput = "note.json"\ncapabilities = ["notes.read"]\n',
    );

    let page = await issuer.events();

    for (let tries = 0; page.items.length === 0 && tries < 50; tries += 1) {
      await Bun.sleep(100);
      page = await issuer.events();
    }

    expect(page.items.map((event) => [event.kind, event.data])).toEqual([
      ["tool.placed", { tool: "ledger", namespaces: ["ledger"], entries: [{ tool: "ledger/keep", reachedBy: "act", summary: "Keeps a line." }], replaced: false }],
    ]);
    expect((await issuer.events({ after: page.items[0]!.id })).items).toEqual([]);
    expect((await refusal(issuer.events({ after: 1 }))).code).toBe("gone");
    expect((await refusal((await session()).events())).code).toBe("forbidden");
  });

  test("revokes a session, after which its token is refused", async () => {
    const issued = await issuer.issue({ holderName: "tester", namespaces: ["notes"], capabilities: [], grants: [] });
    const revoked = pocketClient({ socket: issuer.socket, token: issued.token });

    await issuer.revoke(issued.token);
    expect((await refusal(revoked.health())).code).toBe("unauthorized");
    expect((await refusal(issuer.revoke(issued.token))).code).toBe("not-found");
  });
});
