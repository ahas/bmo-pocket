# word-chain

The word chain, `bmo-word-chain`: the Korean word-chain game (끝말잇기) a room plays together with its agent. Its
word book is built from KKuTu's word list and the National Institute of Korean Language's dictionaries; its
service reads the rooms' messages off the receiver's event stream while a game stands in a room, judges every move
against the book with no model, plays the robot's moves, and says each step's lines in the room as codes the web
unit renders. Its tool opens and stops a game; the tool host serves it.

## Layout

| Folder | Holds |
|---|---|
| `deploy/` | the service's user unit, `bmo-word-chain.service`, which a deployment links from the deploy tree |
| `docs/` | the canonical documents: the game in a room (`word-chain.md`) and the boundary every other unit matches (`api.md`) |
| `graph/` | the code dependency graph: one node per feature with its files and what it reads or calls, written only by `bmo graph` |
| `policy/` | what a game is played by: the words a room says, the times, the points, the initial-sound rule and the word book's sources |
| `src/` | the game, the word book, the games in the rooms and their lines, the service, its control routes and the tool |
| `tests/` | the tests, and the Korean they play with as fixtures under `tests/fixtures/` |

The tool's manifest, `tool.toml`, and its two schemas stand at the root, where the tool host reads them.

## Build and test

```sh
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

## Running

- `bmo-word-chain install` installs the word book under the data root; `bmo-word-chain word <word>` looks a word up.
- `bmo-word-chain serve` runs the service on the configuration `env/word-chain.toml` under the data root names
  (`docs/api.md`, 「Configuration」); `bmo-word-chain health` ends 0 once it answers.
- `bmo-word-chain start` and `stop` open and stop a game through the service; `bmo-word-chain tool` answers one
  call of the tool host's protocol.

## Version

The package's version, in semver, is the repository's: a change to a contract another repository reads (a route, a
code or its fields, the tool's entries) raises the major version.
