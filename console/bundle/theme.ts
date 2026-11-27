import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";

import postcss from "postcss";
import valueParser from "postcss-value-parser";

import type { Node as ValueNode } from "postcss-value-parser";

// A stylesheet of the layer's theme as the bundle's build reads it from a folder of its own: none of the
// sources it scans, an import a caller names taken as the caller writes it, and every other relative import
// pointed at the file beside the stylesheet it was read from, so a file it imports keeps resolving its own
// relative addresses where it stands.

/** The address an import's params name: the quoted string or `url()` that opens them. */
function importTarget(nodes: ValueNode[]): ValueNode | undefined {
  const first = nodes.find((node) => node.type !== "space" && node.type !== "comment");

  if (first?.type === "string") return first;
  if (first?.type === "function" && first.value === "url") return first.nodes.find((node) => node.type !== "space");

  return undefined;
}

function isRelative(address: string): boolean {
  return address.startsWith("./") || address.startsWith("../");
}

/**
 * The stylesheet at `file` with its `@source` rules dropped and its imports rewritten: the params of an
 * import whose address `imports` names become that entry's value, and a relative address becomes the
 * absolute path of the file it names.
 */
export function themeStylesheet(file: string, imports: Readonly<Record<string, string>> = {}): string {
  const root = postcss.parse(readFileSync(file, "utf8"), { from: file });

  root.walkAtRules("source", (rule) => {
    rule.remove();
  });
  root.walkAtRules("import", (rule) => {
    const params = valueParser(rule.params);
    const target = importTarget(params.nodes);

    if (target === undefined) return;

    const named = imports[target.value];

    if (named !== undefined) rule.params = named;
    else if (isRelative(target.value)) {
      target.value = resolve(dirname(file), target.value);
      rule.params = valueParser.stringify(params.nodes);
    }
  });

  return root.toString();
}

/** A path as the quoted address an import names it by. */
export function quoted(path: string): string {
  return JSON.stringify(path);
}
