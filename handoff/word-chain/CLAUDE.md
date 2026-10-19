# Working in this repository

This repository is the word chain, `bmo-word-chain`: the Korean word-chain game, its word book, the games standing
in the rooms, the service that reads the rooms' messages off the receiver and says the games' lines through it,
its control routes, and its tool, which the tool host serves. It knows no other unit's code: the receiver, the web
unit and the tool host are reached over their documented APIs (`docs/api.md`).

## Rules

- **`docs/` is canonical.** Read the document covering a change before changing code. A change that would make it
  wrong or incomplete repairs it first, stating what is true now, and the code is then written to match.
- **No dependency from another repository but the base libraries** `bmo-chat`, `bmo-chat-client` and `bmo-ui`.
  `tests/repository.rs` fails on any other `bmo-*` dependency.
- **English only** in code, comments, strings, tests and documents, and in commit messages. A game's lines are
  codes the web unit's language pack renders; what a route or the tool answers is an English sentence with a
  code. The Korean words a game reads — what a room says to it, the language's rules, the dictionaries' terms —
  are data in `policy/word-chain.toml`, and the Korean the tests play with is data under `tests/fixtures/`, never
  in a `.rs` file. `tests/repository.rs` fails on Hangul in any `.rs` file.
- **No agent names, owner names, host names, home paths or private network addresses** in code, documents or
  commits; the robot's name and the agent a game's lines are said as are the configuration's.
- **Tests live in `tests/`.** No test module sits in `src/`.
- **Documents state what is true now**: no change history, no quoted source and no file-and-line pins.
- **`graph/` is the code dependency graph**, written only by `bmo graph` run with `BMO_SOURCE_DIR` naming this
  checkout. A commit that moves a feature's files, symbols or edges brings its node along in the same commit.

## Checks

A change moves through three stages in order:

1. Writing: `cargo check --all-targets`.
2. Review: read the whole diff against what the change is for and the document governing it, then
   `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` (108 columns, 2-space indentation, as
   `rustfmt.toml` sets).
3. Verification: the test targets the change touched, then `cargo test` once.

The version is the package's, in semver. `testbed.toml` is the testbed's declaration: how it tests, checks, builds
and deploys the repository, the tool's registration with the tool host among it.
