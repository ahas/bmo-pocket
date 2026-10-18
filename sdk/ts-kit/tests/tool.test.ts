import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { createServer, type Server } from "node:net";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

import { askLine, plain, type Answer } from "../src/tool";

// Both helpers are held to one set of cases: each answers a stand-in tool's calls, run as the host runs
// a placed executable, a process handed one object on standard input.

const dir = mkdtempSync(join(tmpdir(), "tool-sdk-"));
const socket = join(dir, "callback.sock");
const asked: unknown[] = [];
let host: Server;

beforeAll(async () => {
  // A stand-in for the host's side of a call-back socket: it keeps what it is asked and answers a call
  // with the entry's name, and a claim with the address it was asked for.
  host = createServer((conn) => {
    let said = "";

    conn.setEncoding("utf8");
    conn.on("data", (piece: string) => {
      said += piece;
      if (!said.includes("\n")) return;

      const one = JSON.parse(said.split("\n")[0]!);

      asked.push(one);
      conn.end(`${JSON.stringify(one.kind === "call" ? { content: [{ type: "text", text: `called ${one.entry}` }], isError: false } : { entry: "web/stand_in", arguments: { page: one.address } })}\n`);
    });
  });
  await new Promise<void>((done) => host.listen(socket, done));
});

afterAll(() => {
  host.close();
  rmSync(dir, { recursive: true, force: true });
});

const typescriptTool = join(dir, "stand-in.ts");
const pythonTool = join(dir, "stand_in.py");

writeFileSync(
  typescriptTool,
  `import { answer, structured, text } from ${JSON.stringify(resolve(import.meta.dir, "../src/tool.ts"))};

await answer({
  "test/echo": (call) => structured({ said: call.arguments.said, line: call.session.line }),
  "test/relay": async (call) => text(\`\${(await call.callBack("test/other", { a: 1 })).content[0].text}; \${(await call.claim("https://a.test/x")).entry}\`),
  "test/broken": () => { throw new Error("it broke"); },
});
`,
);
writeFileSync(
  pythonTool,
  `import sys
sys.path.insert(0, ${JSON.stringify(resolve(import.meta.dir, "../../py-kit/src"))})
import bmo_tool


def relay(call):
    said = call.call_back("test/other", {"a": 1})["content"][0]["text"]
    return bmo_tool.text(f"{said}; {call.claim('https://a.test/x')['entry']}")


def broken(call):
    raise RuntimeError("it broke")


bmo_tool.answer({
    "test/echo": lambda call: bmo_tool.structured({"said": call.arguments["said"], "line": call.session["line"]}),
    "test/relay": relay,
    "test/broken": broken,
})
`,
);

const session = { line: "chat", capabilities: [], dataRoot: "/d", archive: "/d/bmo.db" };

// Run apart, since the stand-in host answers its call-backs on this process's own event loop.
async function run(command: string[], input: string): Promise<{ answer: any; code: number; stderr: string }> {
  const ran = Bun.spawn(command, { stdin: Buffer.from(input), stdout: "pipe", stderr: "pipe", cwd: dir });
  const [out, stderr, code] = await Promise.all([new Response(ran.stdout).text(), new Response(ran.stderr).text(), ran.exited]);

  return { answer: out ? JSON.parse(out) : null, code, stderr };
}

for (const [language, command] of [["TypeScript", ["bun", typescriptTool]], ["Python", ["python3", pythonTool]]] as const) {
  describe(`the ${language} helper`, () => {
    const call = (entry: string, args: unknown = {}) =>
      run([...command], JSON.stringify({ protocol: 1, kind: "call", entry, arguments: args, session, callback: socket }));

    test("answers the entry it is asked for, an object as its text and its structured content", async () => {
      const { answer, code } = await call("test/echo", { said: "café ☕" });

      expect(code).toBe(0);
      expect(answer.isError).toBe(false);
      expect(answer.structuredContent).toEqual({ said: "café ☕", line: "chat" });
      expect(JSON.parse(answer.content[0].text)).toEqual({ said: "café ☕", line: "chat" });
    });

    test("asks the host for another entry and a claim, one line each, under the call's socket", async () => {
      asked.length = 0;

      const { answer } = await call("test/relay");

      expect(plain(answer as Answer)).toBe("called test/other; web/stand_in");
      expect(asked).toEqual([{ kind: "call", entry: "test/other", arguments: { a: 1 } }, { kind: "claim", address: "https://a.test/x" }]);
    });

    test("answers an entry it does not hold and a handler that threw as failed calls", async () => {
      expect((await call("test/none")).answer).toEqual({ content: [{ type: "text", text: "ERROR: this executable holds no entry test/none" }], isError: true });
      expect((await call("test/broken")).answer).toEqual({ content: [{ type: "text", text: "ERROR: it broke" }], isError: true });
    });

    test("answers a claim with none and a resource request as an executable serving none", async () => {
      const ask = async (rest: object) => (await run([...command], JSON.stringify({ protocol: 1, session, ...rest }))).answer;

      expect(await ask({ kind: "claim", entry: "test/echo", address: "https://a.test" })).toEqual({ arguments: null });
      expect(await ask({ kind: "resources_list" })).toEqual({ resources: [] });
      expect(await ask({ kind: "resources_read", uri: "paper://x" })).toEqual({ error: "this executable serves no resources" });
    });

    test("refuses a call that does not read on standard error, ending unclean", async () => {
      for (const bad of ["not json", JSON.stringify({ protocol: 2, kind: "call", entry: "test/echo", session })]) {
        const ran = await run([...command], bad);

        expect(ran.code).toBe(1);
        expect(ran.answer).toBeNull();
        expect(ran.stderr).toContain("the call does not read:");
      }
    });
  });
}

test("a socket that answers nothing is said to have", async () => {
  const quiet = createServer((conn) => conn.end());
  const at = join(dir, "quiet.sock");

  await new Promise<void>((done) => quiet.listen(at, done));
  await expect(askLine(at, { kind: "claim", address: "x" })).rejects.toThrow("answered nothing");
  quiet.close();
});
