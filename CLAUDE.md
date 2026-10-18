# Working in this repository

This repository is the tool host, `bmo-pocket`: the `mcp-host` process serving every agent session's MCP
tools, the toolbook that lets a session reach every tool it holds without each one standing in its prompt,
the registry of installed tools, and the tool SDK (`sdk/`): the contract every tool is written to and the
helpers for tools in other languages. Tools are not this repository's: a standalone tool is a repository of
its own, and a tool bound to one service stands in that service's `tools/` folder. The host knows neither by
name; it reads their manifests.

## Rules

- **`docs/` is canonical.** Read the document covering a change before changing code. A change that would
  make it wrong or incomplete repairs it first, stating what is true now, and the code is then written to
  match.
- **No dependency from another repository but the base libraries** `bmo-chat`, `bmo-chat-client` and
  `bmo-ui`: no `bmo-*` crate or package of another unit, no path outside the repository, no git source.
  Another unit is reached over its documented API. `tests/repository.rs` fails on any of these.
- **A base never knows its consumers.** The host supplies the mechanism and a safe default; no code here
  names a service, a tool or an agent that uses it.
- **English only** in code, comments, strings, tests and documents, and in commit messages. Messages the
  host answers with are English sentences or status codes; people's text is rendered by the web unit's
  language packs. `tests/repository.rs` fails on Hangul in any `.rs`, `.ts` or `.py` file.
- **No agent names, owner names, host names, home paths or private network addresses** in code, documents
  or commits; tests use neutral names.
- **Tests live in `tests/`.** A crate's tests are in its own `tests/`; checks that span the repository are in
  the root `tests/`. No test module sits in `src/`.
- **No directory named after its parent** (no `x/x`), **no folder named for a language**, and **only Cargo
  manifests at the root**: the root `Cargo.toml` is the workspace and the repository's own package.
- **Documents state what is true now**: no change history, no quoted source and no file-and-line pins.

## Checks

A change moves through three stages in order:

1. Writing: `cargo check --workspace --all-targets`.
2. Review: read the whole diff against what the change is for and the document governing it, then
   `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all --check` (108 columns,
   2-space indentation, as `rustfmt.toml` sets).
3. Verification: the test targets the change touched, then `cargo test --workspace` once.

The version is the workspace's, in semver. `testbed.toml` is the testbed's declaration: how it tests, checks,
builds and deploys the repository.
