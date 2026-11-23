// The tool's own tests, its manifest's check: the executable run as the host runs it, one handed object on
// standard input and one answer read off standard output.

import { expect, test } from "bun:test";

const here = import.meta.dir;

// A handed object as the host writes it, under a record holding the entry's capability.
function handed(fields: Record<string, unknown>): Record<string, unknown> {
  return {
    protocol: 1,
    kind: "call",
    session: { holderName: "tester", namespaces: ["example"], capabilities: ["example.read"], grants: [] },
    ...fields,
  };
}

// The executable run once in its folder: its exit code, what it wrote on standard output and on standard error.
async function run(said: string): Promise<{ code: number; out: string; err: string }> {
  const child = Bun.spawn(["bun", "count.ts"], { cwd: here, stdin: new Blob([said]), stdout: "pipe", stderr: "pipe" });
  const [out, err, code] = await Promise.all([
    new Response(child.stdout).text(),
    new Response(child.stderr).text(),
    child.exited,
  ]);

  return { code, out, err };
}

test("a call is answered with the count as text and as an object", async () => {
  const { code, out } = await run(
    JSON.stringify(handed({ entry: "example/count_words_ts", arguments: { text: "one two  three\n" } })),
  );
  const answer = JSON.parse(out);

  expect(code).toBe(0);
  expect(answer.isError).toBe(false);
  expect(answer.structuredContent).toEqual({ words: 3, characters: 15 });
  expect(JSON.parse(answer.content[0].text)).toEqual(answer.structuredContent);
});

test("characters are code points", async () => {
  const { out } = await run(
    JSON.stringify(handed({ entry: "example/count_words_ts", arguments: { text: "café au lait" } })),
  );

  expect(JSON.parse(out).structuredContent).toEqual({ words: 3, characters: 12 });
});

test("an entry it does not hold is answered as a failure", async () => {
  const { code, out } = await run(JSON.stringify(handed({ entry: "example/other", arguments: {} })));
  const answer = JSON.parse(out);

  expect(code).toBe(0);
  expect(answer.isError).toBe(true);
  expect(answer.content[0].text).toContain("ERROR: ");
});

test("an object that does not read ends unclean", async () => {
  const { code, err } = await run("not json");

  expect(code).not.toBe(0);
  expect(err).toContain("the call does not read");
});
