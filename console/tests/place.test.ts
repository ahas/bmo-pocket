import { expect, test } from "bun:test";

import { firstSentence, matches, placeOf, queryOf } from "../app/domain/pocket-place";

test("a query names the view, the namespace, the tool and the search, the catalogue by default", () => {
  expect(placeOf({})).toEqual({ view: "catalogue", namespace: null, tool: null, q: "" });
  expect(placeOf({ view: "servers", q: "mail" })).toEqual({ view: "servers", namespace: null, tool: null, q: "mail" });
  expect(placeOf({ view: "nowhere", namespace: ["web", "other"], tool: "fetch" })).toEqual({
    view: "catalogue",
    namespace: "web",
    tool: "fetch",
    q: ""
  });
});

test("a place is written back as the query that names it, its empty parts left out", () => {
  const place = { view: "catalogue" as const, namespace: "web", tool: null, q: "" };

  expect(queryOf(place)).toEqual({ namespace: "web" });
  expect(placeOf(queryOf({ view: "sessions", namespace: null, tool: null, q: "desk" }))).toEqual({
    view: "sessions",
    namespace: null,
    tool: null,
    q: "desk"
  });
});

test("a search matches where every word stands in one of the texts, ignoring case", () => {
  expect(matches("", ["anything"])).toBe(true);
  expect(matches("Read note", ["read_note", "Says the NOTE back."])).toBe(true);
  expect(matches("read mail", ["read_note", null])).toBe(false);
});

test("a description is listed by its first sentence", () => {
  expect(firstSentence("Says the note back. It reads nothing else.")).toBe("Says the note back.");
  expect(firstSentence("Finds pages\nacross lines")).toBe("Finds pages");
  expect(firstSentence("v1.2 of the tool")).toBe("v1.2 of the tool");
});

