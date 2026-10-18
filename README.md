# pocket

The tool host, `bmo-pocket`: one process serving every agent session's MCP tools. It reads each tool's
`tool.toml` manifest, whether the tool stands in a repository of its own or in a service repository's
`tools/` folder, and serves its entries under the namespaces and capabilities a session's record holds. A
session is shown a few resident tools by name and reaches every other tool through its toolbook, so adding or
taking away a tool never changes what a running session's prompt begins with.

The repository also holds the tool SDK: the contract every tool is written to, which Rust tools link, and the
helpers tools written in TypeScript or Python import.

## Layout

| Folder | Holds |
|---|---|
| `docs/` | the canonical documents: the host (`mcp-host.md`), the toolbook (`toolbook.md`) and the boundary every other unit matches (`api.md`) |
| `host/` | the tool host, `mcp-host` |
| `sdk/` | the tool SDK: `sdk/contract/`, the `bmo-tool` crate holding the call protocol and the manifest schema, which Rust tools link; `sdk/ts-kit/` and `sdk/py-kit/`, the helpers a TypeScript or Python tool imports |
| `tests/` | the checks that span the repository |

## Build and test

```sh
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

## Version

The workspace's version, in semver, is the repository's: a change to a contract another repository reads
(the call protocol, the manifest schema, the session record, the host's routes) raises the major version.
