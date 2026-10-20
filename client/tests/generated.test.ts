import { expect, test } from "bun:test";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

// The committed types are what `openapi-typescript` generates from the committed docs/openapi.json, so the
// TypeScript client carries the same contract the Rust client does (docs/api.md, 「The OpenAPI document and the
// clients」): `bun run generate` writes them again.

const here = resolve(import.meta.dir, "..");

test("the committed types are a fresh generation of docs/openapi.json", async () => {
  const scratch = mkdtempSync(join(tmpdir(), "pocket-types-"));

  try {
    const fresh = join(scratch, "openapi.d.ts");
    const generated = Bun.spawn([join(here, "node_modules", ".bin", "openapi-typescript"), join(here, "..", "docs", "openapi.json"), "--output", fresh], {
      cwd: here,
      stdout: "ignore",
      stderr: "pipe",
    });

    expect(await generated.exited).toBe(0);
    expect(readFileSync(join(here, "generated", "openapi.d.ts"), "utf8")).toBe(readFileSync(fresh, "utf8"));
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
});
