import { expect, test } from "bun:test";

import { roomOf } from "../app/domain/pocket-room";
import { schemaFields, stateWord, valueRows } from "../app/domain/pocket-value";

// What the screen shows of a value, a schema and the room its box has (docs/mcp-host.md, 「How the screen reads」):
// never JSON, a schema as its fields, and the catalogue's columns by its own box.

const t = (key: string, values?: Record<string, unknown>) => (values === undefined ? key : `${key} ${JSON.stringify(values)}`);

test("an object stands as its named fields, a nested one under its parent's name, and holds no JSON", () => {
  const rows = valueRows(t, { role: "worker", place: { line: "a", at: 3 }, kept: true, tags: ["x", "y"] });

  expect(rows).toEqual([
    { label: "role", value: "worker", mono: true },
    { label: "place › line", value: "a", mono: true },
    { label: "place › at", value: "3", mono: false },
    { label: "kept", value: "pocket.status.yes", mono: false },
    { label: "tags › 1", value: "x", mono: true },
    { label: "tags › 2", value: "y", mono: true }
  ]);
  expect(rows.every((row) => !/[{}[\]]/.test(row.value))).toBe(true);
  expect(valueRows(t, {})).toEqual([]);
});

test("a yes, a no and a value not known are state words", () => {
  expect(stateWord(t, true)).toBe("pocket.status.yes");
  expect(stateWord(t, false)).toBe("pocket.status.no");
  expect(stateWord(t, null)).toBe("pocket.status.unknown");
});

test("a schema stands as its fields: type, whether required, what it is for and what it takes, nested ones under it", () => {
  const fields = schemaFields(t, {
    type: "object",
    required: ["key"],
    properties: {
      key: { type: ["string", "null"], description: "The memory's key." },
      limit: { type: "integer", minimum: 1, maximum: 200, default: 20 },
      status: { enum: ["active", "done"] },
      edits: { type: "array", items: { type: "object", properties: { old: { type: "string" } } } }
    }
  });

  expect(fields.map((one) => [one.name, one.depth, one.type, one.required])).toEqual([
    ["key", 0, "string | null", true],
    ["limit", 0, "integer", false],
    ["status", 0, "", false],
    ["edits", 0, "array<object>", false],
    ["old", 1, "string", false]
  ]);
  expect(fields[0]!.description).toBe("The memory's key.");
  expect(fields[1]!.takes).toEqual([`pocket.schema.range ${JSON.stringify({ from: 1, to: 200 })}`, `pocket.schema.default ${JSON.stringify({ value: "20" })}`]);
  expect(fields[2]!.takes).toEqual([`pocket.schema.choices ${JSON.stringify({ values: "active, done" })}`]);
  expect(schemaFields(t, { type: "string" })).toEqual([]);
});

test("the catalogue's columns follow the width of its own box", () => {
  expect(roomOf(360)).toBe("one");
  expect(roomOf(768)).toBe("two");
  expect(roomOf(1280)).toBe("three");
});
