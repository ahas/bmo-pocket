import { afterAll, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { quoted, themeStylesheet } from "../bundle/theme";

// The layer's stylesheet as the bundle's build copies it into a folder of its own (`bundle/theme.ts`): the
// imports it names are taken as written, every other relative import points at the file beside the original,
// and none of the sources it scans is left.

const folder = mkdtempSync(join(tmpdir(), "console-theme-"));

afterAll(() => {
  rmSync(folder, { recursive: true, force: true });
});

const css = join(folder, "assets", "css");
const stylesheet = join(css, "main.css");

mkdirSync(css, { recursive: true });
writeFileSync(
  stylesheet,
  [
    '@import "tailwindcss";',
    '@import "@nuxt/ui";',
    '@import "./faces.css";',
    "@import url(../shared.css) layer(base);",
    '@source "../../app";',
    "@theme { --font-sans: Face; }",
    ""
  ].join("\n")
);

test("a copied stylesheet names its imports by where they resolve and scans no sources", () => {
  const copied = themeStylesheet(stylesheet, {
    tailwindcss: `${quoted("tailwindcss")} source(none)`,
    "@nuxt/ui": quoted("/theme/nuxt-ui.css")
  });

  expect(copied).toBe(
    [
      '@import "tailwindcss" source(none);',
      '@import "/theme/nuxt-ui.css";',
      `@import "${join(css, "faces.css")}";`,
      `@import url(${join(folder, "assets", "shared.css")}) layer(base);`,
      "@theme { --font-sans: Face; }",
      ""
    ].join("\n")
  );
});
