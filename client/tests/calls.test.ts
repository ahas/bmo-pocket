import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

import { type Pocket, pocketClient } from "../index";

// The calls face (docs/api.md, 「Asynchronous calls」): a call asked of the host, the host keeping and running it, and
// the agent woken through the receiver. The host is the workspace's build, run on a scratch data root with one
// placed tool; the caller is this test, which writes nothing.

const binary = process.env.MCP_HOST_BINARY ?? resolve(import.meta.dir, "../../target/debug/mcp-host");

const MANIFEST = `[tool]
name = "notes"
protocol = 1
run = ["python3", "notes.py"]
timeout = 20

[[entry]]
namespace = "notes"
name = "keep_note"
description = "Keeps a note."
input = "note.json"
capabilities = ["notes.read"]
hints = { read_only = true }
`;

const SCRIPT = `import json, sys
handed = json.load(sys.stdin)
if handed["kind"] == "claim":
    print(json.dumps({"arguments": {}}))
else:
    text = "kept:" + str(handed["arguments"].get("text", ""))
    print(json.dumps({"content": [{"type": "text", "text": text}], "isError": False}))
`;

const RECEIVER_TOKEN = "receiver-token";

let scratch = "";
let sockets = "";
let host: ReturnType<typeof Bun.spawn> | undefined;
let receiver: ReturnType<typeof Bun.serve> | undefined;
let issuer: Pocket;
let port = 0;
const woken: any[] = [];

beforeAll(async () => {
  if (!existsSync(binary)) {
    throw new Error(`${binary} is not built: cargo build -p mcp-host`);
  }

  scratch = mkdtempSync(join(tmpdir(), "pocket-calls-"));
  sockets = mkdtempSync("/tmp/pk-");

  const placed = join(scratch, "tools", "notes", ".placed");

  mkdirSync(placed, { recursive: true });
  writeFileSync(join(placed, "tool.toml"), MANIFEST);
  writeFileSync(join(placed, "note.json"), JSON.stringify({ type: "object", properties: { text: { type: "string" } } }));
  writeFileSync(join(placed, "notes.py"), SCRIPT);

  receiver = Bun.serve({
    port: 0,
    async fetch(request) {
      woken.push(await request.json());
      return Response.json({ id: 7 }, { status: 201 });
    },
  });

  const socket = join(sockets, "c.sock");

  host = Bun.spawn([binary, "serve", "--port", "0", "--data-root", join(scratch, "data"), "--control", socket, "--tools", join(scratch, "tools")], {
    stdout: "pipe",
    stderr: "ignore",
    env: { ...process.env, RECEIVER_ADDRESS: `http://127.0.0.1:${receiver.port}`, RECEIVER_TOKEN },
  });

  const said = new TextDecoder().decode((await (host.stdout as ReadableStream<Uint8Array>).getReader().read()).value);
  port = Number(said.match(/listening on 127\.0\.0\.1:(\d+)/)?.[1]);

  expect(port).toBeGreaterThan(0);
  issuer = pocketClient({ socket, token: readFileSync(join(scratch, "data", "tool-host", "issuer-token"), "utf8").trim() });
});

afterAll(async () => {
  host?.kill();
  await host?.exited;
  receiver?.stop(true);
  rmSync(scratch, { recursive: true, force: true });
  rmSync(sockets, { recursive: true, force: true });
});

// A session token for the namespace the placed tool declares.
async function session(): Promise<string> {
  const issued = await issuer.issue({
    holderName: "tester",
    namespaces: ["notes"],
    capabilities: ["notes.read"],
    grants: [],
    ttlMs: 60_000,
  });

  return issued.token;
}

// One request of the calls face, its answer, and the status it came with.
async function ask(token: string, path: string, body?: unknown): Promise<{ status: number; answer: any }> {
  const response = await fetch(`http://127.0.0.1:${port}${path}`, {
    method: body === undefined ? "GET" : "POST",
    headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });

  return { status: response.status, answer: await response.json() };
}

// The call `id` once it ended, asked again until it has.
async function ended(token: string, id: string): Promise<any> {
  for (let turn = 0; turn < 200; turn += 1) {
    const { answer } = await ask(token, `/use/${id}`);

    if (answer.state !== undefined && answer.state !== "running") {
      return answer;
    }

    await Bun.sleep(50);
  }

  throw new Error(`the call ${id} never ended`);
}

describe("asynchronous calls", () => {
  test("a call is kept and run by the host, and its agent is woken through the receiver", async () => {
    const token = await session();
    const { status, answer } = await ask(token, "/use", {
      tool: "notes/keep_note",
      arguments: { text: "the note" },
      memory: "what it was waiting for",
      room: 42,
      agent: "tester",
    });

    expect(status).toBe(200);
    expect(answer.state).toBe("running");
    expect(answer.id).toMatch(/^notes-keep_note-[0-9a-f]{8}$/);

    const call = await ended(token, answer.id);

    expect(call.state).toBe("done");
    expect(call.tool).toBe("notes/keep_note");
    expect(call.summary).toBe("notes/keep_note answered: kept:the note");
    expect(typeof call.endedAt).toBe("number");
    // The answer and the end stand with the host, under its own data root.
    expect(existsSync(join(scratch, "data", "use", answer.id, "request.json"))).toBe(true);
    expect(existsSync(join(scratch, "data", "use", answer.id, "answer.json"))).toBe(true);
    expect(existsSync(join(scratch, "data", "use", answer.id, "end.json"))).toBe(true);

    // The wake reached the receiver as a fact naming the call, for the room and agent asked.
    expect(call.wake.error).toBeUndefined();
    expect(woken.length).toBe(1);
    expect(woken[0].agentName).toBe("tester");
    expect(woken[0].room).toBe(42);
    expect(woken[0].sort).toBe("use-ended");
    expect(woken[0].said.fact.use).toBe(answer.id);
    expect(woken[0].said.fact.tool).toBe("notes/keep_note");
    expect(woken[0].said.fact.state).toBe("done");
    expect(woken[0].said.fact.summary).toBe("notes/keep_note answered: kept:the note");
    expect(woken[0].said.fact.memory).toBe("what it was waiting for");
  });

  test("the calls a session asked read back through the face, and no other session's do", async () => {
    const token = await session();
    const started = await ask(token, "/use", { tool: "notes/keep_note", arguments: { text: "second" } });

    await ended(token, started.answer.id);

    const page = await ask(token, "/use");
    const ids = page.answer.items.map((call: any) => call.id);

    expect(ids).toContain(started.answer.id);

    const other = await session();
    const refused = await ask(other, `/use/${started.answer.id}`);

    expect(refused.status).toBe(404);
    expect(refused.answer.code).toBe("not-found");
  });

  test("an unknown tool is refused before anything is kept", async () => {
    const token = await session();
    const refused = await ask(token, "/use", { tool: "notes/nope", arguments: {} });

    expect(refused.status).toBe(404);
    expect(refused.answer.error).toContain("notes/nope");
  });

  test("a field the host does not read is given no effect, and the call runs to its answer", async () => {
    const token = await session();
    const started = await ask(token, "/use", {
      tool: "notes/keep_note",
      arguments: { text: "kept" },
      maxSeconds: 1,
    });

    const call = await ended(token, started.answer.id);

    expect(call.state).toBe("done");
    expect(call.maxSeconds).toBeUndefined();
  });
});
