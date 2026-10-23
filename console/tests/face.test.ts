import { afterAll, beforeAll, expect, test } from "bun:test";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

import { consoleManifest } from "bmo-ui/shared/console-manifest";

// The console face as a console meets it (docs/api.md, 「The console face」): the workspace's build of `mcp-host`, or
// the one `MCP_HOST_BINARY` names, started with a console port and a bundle folder, its manifest read through
// bmo-ui's own reading of the contract, and each file it names answered to the console token and the role.

const binary = process.env.MCP_HOST_BINARY ?? resolve(import.meta.dir, "../../target/debug/mcp-host");
const MODULE = "export default { routes: [] };\n";
const STYLESHEET = ".pocket {}\n";

let scratch = "";
let sockets = "";
let host: ReturnType<typeof Bun.spawn> | undefined;
let base = "";
let token = "";
let words = "";

beforeAll(async () => {
  if (!existsSync(binary)) {
    throw new Error(`${binary} is not built: cargo build -p mcp-host`);
  }

  scratch = mkdtempSync(join(tmpdir(), "pocket-console-"));
  sockets = mkdtempSync("/tmp/pk-");

  const bundle = join(scratch, "bundle");
  const locales = join(import.meta.dir, "..", "i18n", "locales");

  // The words as the build writes them: each locale's tree under its code.
  words = JSON.stringify(
    Object.fromEntries(readdirSync(locales).sort().map((file) => [file.replace(/\.json$/, ""), JSON.parse(readFileSync(join(locales, file), "utf8"))]))
  );
  mkdirSync(bundle, { recursive: true });
  mkdirSync(join(scratch, "tools"), { recursive: true });
  writeFileSync(join(bundle, "module.js"), MODULE);
  writeFileSync(join(bundle, "style.css"), STYLESHEET);
  writeFileSync(join(bundle, "words.json"), words);

  host = Bun.spawn(
    [binary, "serve", "--port", "0", "--data-root", join(scratch, "data"), "--control", join(sockets, "c.sock"), "--tools", join(scratch, "tools"), "--console-port", "0", "--console-bundle", bundle],
    { stdout: "pipe", stderr: "ignore" }
  );

  const said = new TextDecoder().decode((await (host.stdout as ReadableStream<Uint8Array>).getReader().read()).value);
  const port = /console 127\.0\.0\.1:(\d+)/.exec(said)?.[1];

  expect(port).toBeDefined();
  base = `http://127.0.0.1:${port}`;
  token = readFileSync(join(scratch, "data", "tool-host", "console-token"), "utf8").trim();
});

afterAll(async () => {
  host?.kill();
  await host?.exited;
  rmSync(scratch, { recursive: true, force: true });
  rmSync(sockets, { recursive: true, force: true });
});

function ask(path: string, role?: string): Promise<Response> {
  const headers: Record<string, string> = { authorization: `Bearer ${token}` };

  if (role !== undefined) headers["owner-role"] = role;

  return fetch(`${base}${path}`, { headers });
}

test("the manifest reads through the contract's own reading and names the screen", async () => {
  const answered = await ask("/console-manifest");

  expect(answered.status).toBe(200);

  const manifest = consoleManifest(await answered.json());

  expect([manifest.name, manifest.serviceName, manifest.screenPath, manifest.role]).toEqual(["pocket", "pocket", "/pocket", "administrator"]);
  expect([manifest.apiPrefix, manifest.ownerKey, manifest.events, manifest.calls]).toEqual(["/", false, false, []]);
  expect(manifest.title).toBe("pocket.title.screen");
});

test("the words are answered with no role, and the module and the stylesheet to the administrator alone", async () => {
  const manifest = consoleManifest(await (await ask("/console-manifest")).json());

  expect(await (await ask(manifest.words)).text()).toBe(words);
  expect((await ask(manifest.bundle)).status).toBe(403);
  expect((await ask(manifest.bundle, "member")).status).toBe(403);

  const module = await ask(manifest.bundle, "administrator");

  expect([module.status, await module.text()]).toEqual([200, MODULE]);
  expect(module.headers.get("content-type")).toBe("text/javascript; charset=utf-8");
  expect(await (await ask(manifest.stylesheet!, "administrator")).text()).toBe(STYLESHEET);
});

test("the screen's reads answer the administrator, and nothing answers another token", async () => {
  for (const path of ["/namespaces", "/schedules", "/tool-sessions", "/servers"]) {
    const read = await ask(path, "administrator");

    expect([path, read.status]).toEqual([path, 200]);
    expect(await read.json()).toEqual({ items: [], more: false });
  }

  const issuer = readFileSync(join(scratch, "data", "tool-host", "issuer-token"), "utf8").trim();
  const refused = await fetch(`${base}/console-manifest`, { headers: { authorization: `Bearer ${issuer}` } });

  expect(refused.status).toBe(401);
  expect(((await refused.json()) as { code: string }).code).toBe("unauthorized");
});
