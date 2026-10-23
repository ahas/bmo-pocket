import { mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { basename, dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import tailwindcss from "@tailwindcss/vite";
import vue from "@vitejs/plugin-vue";
import AutoImport from "unplugin-auto-import/vite";
import Components from "unplugin-vue-components/vite";
import { defineConfig } from "vite";

import type { Plugin } from "vite";

// The tool host's screen as the browser module a console loads (docs/api.md, 「The console face」). What a
// console shares — `vue`, `vue-router`, `vue-i18n`, `nuxt/app`, `@nuxt/ui` and bmo-ui — stays an import of the
// package's own path, which the console resolves to its copy; everything else is carried in the module. The
// names Nuxt would import and register by itself are read from the declarations `nuxi prepare` writes, so the
// module takes each from the package a console shares. The build writes three files, the module, its
// stylesheet and its words, into the folder `BMO_POCKET_CONSOLE_OUT` names, which the host serves.

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, "..");
const generated = join(root, ".nuxt");
const packages = join(root, "node_modules");
const out = process.env.BMO_POCKET_CONSOLE_OUT ?? join(root, ".output", "console");

const SHARED = /^(?:vue|vue-router|vue-i18n|nuxt\/app|@nuxt\/ui(?:\/.*)?|bmo-ui\/.+)$/;

const PAGE_MACRO = join(here, "page-meta.ts");

// The names `nuxt/app` exports, read off its entry.
function nuxtAppNames(): Set<string> {
  const entry = readFileSync(join(packages, "nuxt", "dist", "app", "index.js"), "utf8");
  const names = [...entry.matchAll(/export\s*\{([^}]*)\}/g)].flatMap((found) => found[1]!.split(","));

  return new Set(names.map((name) => name.trim().split(/\s+as\s+/).pop()!).filter((name) => name !== ""));
}

const NUXT_APP = nuxtAppNames();

// Where the module takes a name Nuxt's declarations import from: a shared package's own path, the screen's
// own file, or nowhere where it is neither.
function sourceOf(name: string, from: string): string | null {
  if (from === "vue" || from === "vue-router" || from === "vue-i18n") return from;
  if (from.startsWith("#app/")) return NUXT_APP.has(name) ? "nuxt/app" : null;

  const file = resolve(generated, from);
  const inPackages = relative(packages, file);

  if (!inPackages.startsWith("..")) {
    const ui = inPackages.match(/^@nuxt\/ui\/dist\/runtime\/(.+)$/);

    if (ui !== null) return `@nuxt/ui/${ui[1]}`;
    if (inPackages.startsWith("bmo-ui/")) return inPackages;

    return null;
  }

  return relative(root, file).startsWith("..") ? null : file;
}

interface Declared {
  name: string;
  from: string;
}

function declaredImports(): Declared[] {
  const text = readFileSync(join(generated, "imports.d.ts"), "utf8");

  return [...text.matchAll(/export\s*\{([^}]*)\}\s*from\s*'([^']+)'/g)].flatMap((found) =>
    found[1]!
      .split(",")
      .map((name) => name.trim())
      .filter((name) => name !== "" && !name.startsWith("type "))
      .map((name) => ({ name, from: found[2]! }))
  );
}

function declaredComponents(): Declared[] {
  const text = readFileSync(join(generated, "components.d.ts"), "utf8");

  return [...text.matchAll(/^export const (\w+): typeof import\("([^"]+)"\)\['default'\]$/gm)].map((found) => ({
    name: found[1]!,
    from: found[2]!
  }));
}

function sources(folder: string): string[] {
  return readdirSync(folder, { withFileTypes: true, recursive: true })
    .filter((entry) => entry.isFile() && /\.(?:vue|ts)$/.test(entry.name))
    .map((entry) => join(entry.parentPath, entry.name));
}

// `$fetch` is the console's own fetcher, a global of the page; the page macro is the module's no-op.
const imports = new Map<string, string>([["definePageMeta", PAGE_MACRO]]);
const components = new Map<string, string>();
const unshared: string[] = [];

for (const { name, from } of declaredImports()) {
  if (name === "$fetch" || imports.has(name)) continue;

  const source = sourceOf(name, from);

  if (source === null) unshared.push(name);
  else imports.set(name, source);
}

for (const { name, from } of declaredComponents()) {
  const source = sourceOf("default", from);

  if (source === null) unshared.push(name);
  else components.set(name, source);
}

// A name the screen uses that a console does not share would fail in the console's page, so the build
// refuses it first.
function sharedOnly(): Plugin {
  return {
    name: "bmo-pocket-shared-only",
    buildStart() {
      const files = sources(join(root, "app"));
      const used = unshared.filter((name) =>
        files.some((file) => new RegExp(`(?<![\\w$.])${name.replace(/\$/g, "\\$")}(?![\\w$])`).test(readFileSync(file, "utf8")))
      );

      if (used.length > 0) this.error(`The screen uses what a console does not share: ${used.join(", ")}`);
    }
  };
}

// The layer's stylesheet with none of the sources it scans, so the module's stylesheet carries the
// utilities of the screen's own sources alone; the console's stylesheet carries the rest.
const THEME = join(generated, "console-theme");

function withoutSources(text: string): string {
  return text.replace(/@source[^;]*;/g, "");
}

function writeTheme(): void {
  const ui = join(packages, "@nuxt", "ui", "dist", "runtime");
  const layer = join(packages, "bmo-ui", "app", "assets", "css", "main.css");

  mkdirSync(THEME, { recursive: true });
  writeFileSync(join(THEME, "ui.css"), withoutSources(readFileSync(join(generated, "ui.css"), "utf8")));
  writeFileSync(
    join(THEME, "nuxt-ui.css"),
    withoutSources(readFileSync(join(ui, "index.css"), "utf8"))
      .replace('@import "#build/ui.css"', `@import "${join(THEME, "ui.css")}"`)
      .replace('@import "./keyframes.css"', `@import "${join(ui, "keyframes.css")}"`)
  );
  writeFileSync(
    join(THEME, "layer.css"),
    withoutSources(readFileSync(layer, "utf8"))
      .replace('@import "tailwindcss";', '@import "tailwindcss" source(none);')
      .replace('@import "@nuxt/ui";', `@import "${join(THEME, "nuxt-ui.css")}";`)
  );
}

writeTheme();

// The bundle's words: one JSON object keyed by locale, each locale's tree from its file.
function words(): Plugin {
  const folder = join(root, "i18n", "locales");

  return {
    name: "bmo-pocket-words",
    generateBundle() {
      const tree = Object.fromEntries(
        readdirSync(folder)
          .filter((file) => file.endsWith(".json"))
          .sort()
          .map((file) => [basename(file, ".json"), JSON.parse(readFileSync(join(folder, file), "utf8")) as unknown])
      );

      this.emitFile({ type: "asset", fileName: "words.json", source: `${JSON.stringify(tree)}\n` });
    }
  };
}

export default defineConfig({
  root,
  logLevel: "warn",
  plugins: [
    sharedOnly(),
    vue({ isProduction: true }),
    AutoImport({
      imports: [...imports].map(([name, from]) => ({ from, imports: [name] })),
      include: [/\.[cm]?ts$/, /\.vue$/, /\.vue\?vue/],
      dts: false,
      vueTemplate: true
    }),
    Components({
      dirs: [],
      dts: false,
      resolvers: [(name: string) => components.get(name)]
    }),
    tailwindcss(),
    words()
  ],
  resolve: {
    alias: [{ find: /^#console-theme$/, replacement: join(THEME, "layer.css") }]
  },
  define: {
    "import.meta.server": "false",
    "import.meta.client": "true",
    "import.meta.dev": "false",
    "process.env.NODE_ENV": JSON.stringify("production"),
    __VUE_OPTIONS_API__: "true",
    __VUE_PROD_DEVTOOLS__: "false",
    __VUE_PROD_HYDRATION_MISMATCH_DETAILS__: "false"
  },
  build: {
    outDir: out,
    emptyOutDir: true,
    target: "es2022",
    minify: true,
    sourcemap: false,
    copyPublicDir: false,
    cssCodeSplit: false,
    lib: {
      entry: join(here, "entry.ts"),
      formats: ["es"],
      fileName: () => "module.js",
      cssFileName: "style"
    },
    rollupOptions: {
      external: (id) => SHARED.test(id),
      output: { codeSplitting: false }
    }
  }
});
