# The boundary

This document is the word chain's boundary: its configuration, every route it serves, its tool, every code its
games' lines go out as, and every call it makes on another unit. Another unit reaches the word chain only across
it, and the word chain reaches another unit only by the calls below.

## Versioning

- **The boundary carries the repository's one version, in semver,** the version in `Cargo.toml`.
- **The major moves on a change an existing caller reads:** a route, a code or its fields, the tool's entries or
  their arguments, a configuration key. The minor moves on an addition of one; the patch on anything else.

## Configuration

One TOML file, `env/word-chain.toml` under the data root once deployed, which the service, its command line and its
tool all read; `--config` names another.

| Key | Meaning |
|---|---|
| `listen` | The address the control routes answer on |
| `token` | The bearer token every control route takes |
| `data_root` | The data root the word book stands under, `word-chain/ko.sqlite` |
| `language` | The language a room's lines are rendered in |
| `agent` | The registration name of the agent a game's lines are said as |
| `robot_name` | The name the robot plays under |
| `[receiver]` `base`, `token` | The receiver's base address and the token it takes |
| `[web]` `base`, `token` | The web unit's base address and the token it takes |

## The control routes

- **On the configured `listen` address,** every route but `GET /health` behind `Authorization: Bearer <token>`,
  answered 401 `unauthorized` without it. Bodies are JSON both ways, camelCase as the tool sends them; a refusal
  carries `ok: false`, a `code` and an English `error`.

| Route | Body | Answer |
|---|---|---|
| `POST /word-chain/start` | `chatId`; `starterId?` with `starterName`; `robot` (null, `easy`, `normal` or `hard`); `turnSeconds?`; `loanwords?` (true unless false); `manner?`; `opening?` | 200 `ok`, `code` (`opened` or `asked-again`), `line`; 400 `bad-request`; 409 with the refusal's `code`: `no-book`, `in-game`, `unusable` |
| `POST /word-chain/stop` | `chatId`; `playerId?` | 200 `ok`, `code` `stopped`, `line`; 400 `bad-request`; 404 with `none`, `several` or `not-theirs` |
| `GET /health` | | `ok` |

## The tool

- **`tool.toml`,** the tool host's manifest (the host's `docs/api.md`, 「The manifest」): two entries of the `bmo`
  namespace asking for `room.tools`, `word_chain_start` and `word_chain_stop`, their schemas beside it. The host
  runs `bmo-word-chain tool`, handed one call of its protocol 1 on standard input.
- **The room** is the one the arguments' `chat` names, else the turn's own, `BMO_TURN_CHAT` of the record's run.
- **The service** is reached at the address and with the token of the configuration under the record's `dataRoot`;
  the route's `line` is the tool's answer, and its `error` the tool's refusal.
- **`word_chain_start`'s arguments:** `bmo` (`off`, `easy`, `normal` or `hard`, `normal` where none is given) for
  the robot, `starter` and `starter_name` for the person who asked, `seconds`, `word` for the opening word,
  `no_loanwords` and `manner`. **`word_chain_stop`'s:** `player`. Both take `chat`.

## The codes

Each line a game says goes out as a code and its fields, rendered by the web unit as `word-chain.<code>`. A word,
a syllable and a name are carried as they stand; the marks around them and the particles after them are the
pack's.

| Code | Fields | What it says |
|---|---|---|
| `game` | `names`, `listed` | The game a message is of, by its people, the first `listed` shown and the rest counted |
| `call` | `robot` (null or the difficulty), `robot_name`, `loanwords`, `manner`, `seconds` (null for no limit), `idle_minutes`, `wait_seconds`, `opening` (null or the word), `join_name` (null, or the name to join by where another game recruits) | The call for players: the rules, how to join, when it starts, the word it opens on |
| `joined` | `name`, `count` | A player joined, the game now this many |
| `started` | `order` (the names in order) | Play started |
| `first-word`, `new-word` | `word` | The word the first round, or a new round, opens on |
| `meaning` | `word`, `meaning` | What a word that is not basic means |
| `turn` | `name`, `openings` (the syllables), `seconds` (null for no limit) | Whose move it is |
| `taken` | `word`, `points` | A player's word taken |
| `taken-robot` | `name`, `word`, `points` | The robot's word taken |
| `off-chain` | `word`, `openings` | A word refused for not following |
| `not-in-book`, `loanword`, `used`, `manner` | `word` | A word refused for the reason the code names |
| `timeout`, `gave-up`, `stuck` | `name` | A player out for want of time, by giving up, or with nothing to play |
| `hanbang` | `openings`, `name`, `by` (null, or `name` and `points` of the player scoring it) | A player out on a word nothing follows |
| `nobody` | | A game nobody joined folds |
| `stopped`, `decided` | | A game stopped, or played out; its ranking follows |
| `idle` | `minutes` | A game closed for want of play; its ranking follows |
| `ranking` | `place`, `name`, `score`, `words` | One row of the ranking |
| `ask-again` | `unusable` (`reason`: `not-in-book`, `loanword` or `no-follower`; `word`), `name` | The asker asked for another opening word |
| `ask-again-bare` | `unusable` | An offered opening word that cannot open either |
| `ask-first` | `name` | The asker told an opening word is wanted first |
| `no-opening` | | The game folded for want of a usable opening word |
| `folded` | | A game waiting for its opening word stopped |
| `pick` | `for` (`join` or `stop`), `games` (each game's names), `example` | Which of several games a word is for |

## What the word chain calls

### The receiver

All under the receiver's bearer token, as its `docs/boundary-api.md` states them.

| Call | Read of the answer |
|---|---|
| `GET /events?after=&run=` | the event stream, read for as long as the service runs and opened again from its cursor when it breaks: `message` events hand the room's message on where a game stands in it, `room-removed` lets the room's games go |
| `GET /rooms/{chat}/messages/{message}` | `sender`, `sender_name`, `text` of a message a game may take |
| `POST /messages/sender-is-owner` | `owner`: whether the owner sent it, who stops another's game by naming it |
| `POST /sends` | a step's message said in its room as the configured `agent`: `asker` (`chat`, `agent`, `job` false), `tool` `say`, `input` (`chat`, `text`) |

### The web unit

| Call | Read of the answer |
|---|---|
| `POST /words/render` | `code` (`word-chain.<code>`), the line's fields beside it, `language` | `text`; any other answer and the line goes out as its code and fields |

## Missing in siblings

What another repository's own boundary must gain for the word chain to stand on it, each for that repository's
worker:

- **bmo-web:** render every code of 「The codes」 under `word-chain.` through `POST /words/render`, from the
  `word-chain` part of its `ko` pack, choosing the particles and the marks as the pack's lines do.
- **bmo-receiver:** say how a message a game takes is marked taken, so it wakes no session and goes no further, as
  any message a game takes did; and a way to say a game's message as a fixed line, on the room's road with no
  output gate and opening no session, which `POST /sends` with `say` does not yet state.
- **bmo-pocket:** the tool registered by `bmo pocket put` from this repository, allowing `room.tools`.
