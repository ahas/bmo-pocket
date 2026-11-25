**English** | [한국어](README.ko.md) | [日本語](README.ja.md) | [简体中文](README.zh-CN.md)

# pocket

The tool host, `bmo-pocket`: one process serving the MCP tools of every tool session. It reads each tool's
`tool.toml` manifest, whether the tool stands in a repository of its own or in a service repository's `tools/`
folder, and serves its entries under the namespaces, capabilities and grants a session's record carries. A session
is shown a few resident tools by name and reaches every other tool through its toolbook, so adding or taking away a
tool never changes what a running session's prompt begins with. A harness reaches it over HTTP, or through its
harness plugin, `bmo-pocket mcp`.

The repository also holds the tool SDK: the contract every tool is written to, the call protocol and the manifest
schema as the host reads them, and the one-file helpers a tool written in TypeScript or Python carries.

How to make a tool, test it and put it live with no deployment is `docs/authoring.md`.

## Layout

| Folder | Holds |
|---|---|
| `client/` | the control socket's client: the Rust crate `bmo-pocket-client`, which generates `docs/openapi.json`, and the TypeScript package `@bmo/pocket-client` generated from it |
| `console/` | the host's screen as a console bundle, `bmo-pocket-console`, on the pinned bmo-ui release it vendors, built into the files the console face serves, and the host's declaration of its manifest (`docs/api.md`, 「The console face」) |
| `deploy/` | the host's user unit, `bmo-mcp-host.service`, which a deployment links from the deploy tree |
| `docs/` | the canonical documents: making a tool (`authoring.md`), the host (`mcp-host.md`), the toolbook (`toolbook.md`), the registry (`registry.md`), the company catalogue of shared personal apps (`catalogue.md`), the boundary every other unit matches (`api.md`) and its OpenAPI document (`openapi.json`) |
| `examples/` | one example tool written in Rust, TypeScript and Python, each put and called by the repository's tests |
| `host/` | the tool host, `mcp-host`, and the command `bmo-pocket` (the registry's verbs and `mcp`), with their tests in `host/tests/` |
| `plugin/` | the harness plugin, `bmo-pocket-plugin`: the MCP server `bmo-pocket mcp` serves and its `bmo-pocket` skill |
| `prompts/` | the prompt text the host serves, `tool-use.md`: the paragraph on finding and using tools a session's system prompt carries |
| `sdk/` | the tool SDK: `sdk/contract/`, the `bmo-tool` crate holding the call protocol and the manifest schema as the host reads them; `sdk/ts-kit/` and `sdk/py-kit/`, the helpers a TypeScript or Python tool imports |
| `tests/` | the checks that span the repository, and `tests/host-contract/`, the host held to what an installed CLI reads of it |

## Personal data

No personal data stands in this repository. `PERSONAL-DATA.md` states what may never stand here and the check
that holds it: the testbed's `personal-data` check reads every tracked file and names the file, the line and the kind of
any mail address, home path, telephone number, token or private network address it finds, never the value.

## Build and test

```sh
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
(cd sdk/ts-kit && bun test)
(cd client && bun test)
(cd console && bun install && bun run check)
(cd tests/host-contract && bun test)
```

## Version

The version is the set's, read off the git tags when a build is made; the version fields of the crates and
packages are placeholders that never move, as `docs/api.md` states in 「Version」.
