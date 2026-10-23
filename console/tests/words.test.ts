import { expect, test } from "bun:test";
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

// The bundle's words (bmo-ui's `docs/console-bundles.md`, "How a bundle mounts"): every key a screen source names
// stands in every locale, under the bundle's namespace, and every locale holds the same keys.

const root = join(import.meta.dir, "..");
const locales = join(root, "i18n", "locales");

function keys(tree: unknown, under = ""): string[] {
  if (typeof tree !== "object" || tree === null) return [under];

  return Object.entries(tree).flatMap(([name, value]) => keys(value, under === "" ? name : `${under}.${name}`));
}

function sources(folder: string): string[] {
  return readdirSync(folder, { withFileTypes: true, recursive: true })
    .filter((entry) => entry.isFile() && /\.(?:vue|ts)$/.test(entry.name))
    .map((entry) => readFileSync(join(entry.parentPath, entry.name), "utf8"));
}

const trees = Object.fromEntries(
  readdirSync(locales).map((file) => [file.replace(/\.json$/, ""), JSON.parse(readFileSync(join(locales, file), "utf8")) as unknown])
);

test("every locale holds the same keys, all under the bundle's namespace", () => {
  const [first, ...rest] = Object.values(trees).map((tree) => keys(tree).sort());

  expect(first!.every((key) => key.startsWith("pocket."))).toBe(true);

  for (const other of rest) expect(other).toEqual(first!);
});

test("every key the screen names stands in the words", () => {
  const named = new Set(sources(join(root, "app")).flatMap((text) => [...text.matchAll(/["'](pocket\.[a-z_]+\.[a-z_]+)["']/g)].map((found) => found[1]!)));
  const held = new Set(keys(trees.ko));

  expect([...named].filter((key) => !held.has(key))).toEqual([]);
});
