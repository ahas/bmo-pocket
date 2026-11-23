# Making a tool

How a tool is written, tested and put live on the tool host, in the order the work is done. The host's own
documents state each mechanism whole: the manifest's schema and the call protocol in `docs/api.md`, the
catalogue and a call in `docs/mcp-host.md`, a put and its versions in `docs/registry.md`. This document is the
author's road through them.

## When a tool is made

- **A tool is made for work that comes back.** Where the same kind of request has been done by hand more than
  once, or plainly will be, and no tool the host serves does it, the work is made a tool once rather than done by
  hand each time. A one-off is done in a shell and thrown away.
- **Look first.** `bmo tool list <words>` lists every entry the host serves whose name or description holds the
  words, and `bmo tool describe <namespace>/<name>` says what one takes; a session finds the same with
  `bmo use --tools --query <words>`. A tool that nearly fits is changed in its own repository rather than
  written again beside it.
- **One tool does one kind of work,** with one entry per thing a caller asks of it. An entry answers what it was
  asked and does nothing beyond its description.

## Where a tool stands

- **A tool bound to no service is a repository of its own,** named `bmo-<name>`, its checkout standing in the
  set's `tools/` folder. It is put by hand (「Putting it live」).
- **A tool that is one service's door** — it calls that service's routes, reads its store or ships with its
  releases — stands in that service's `tools/<name>/` folder. The service's deployment builds it into its
  release and registers it with `bmo-pocket put --placed --service <repository>`, so it goes live with the
  service and is never put by hand.
- **Neither reads another repository's files.** A tool's build is handed no path into another repository, so a
  tool carries what it runs, and reaches a service only through that service's documented routes.
- **The tool's folder holds:** its manifest, `tool.toml`; one JSON Schema file per entry's input and, where it
  answers an object, one for its output; the executable or the source its build makes it from; a manual per entry
  where the description is not enough; and its tests.

## The examples

- **`examples/` holds one tool written three times,** in Rust (`count-words-rust`), TypeScript
  (`count-words-ts`) and Python (`count-words-py`). Each declares one entry in the `example` namespace that
  answers how many words and characters a text holds, as text and as the object its output schema states.
- **A new tool starts as a copy of the example in its language:** the folder copied into the tool's repository
  or the service's `tools/`, then the tool's name, its entry, its schemas and its answer changed.
  - The Rust example is a member of this repository's Cargo workspace, so its `Cargo.toml` takes its version,
    edition and dependency versions from the workspace: a copy standing alone names them itself.
  - The Python and TypeScript examples carry a copy of their language's kit beside the executable, and a copy
    keeps it as it stands in `sdk/`.
- **The repository's tests put every example** into a scratch registry, call it through the host as a session
  calls it, and run each example's own check (`docs/mcp-host.md`, 「Tests」), so an example that stops working
  fails the suite.

## The manifest

`tool.toml` beside the schema files. `docs/api.md` (「The manifest」) states every key and what does not read; a
key it does not name is refused. What an author sets:

### `[tool]`

| Key | What to set |
|---|---|
| `name` | The tool's name: lower-case letters, digits, `-` and `_`, the same as its folder's. A put of a name already registered upgrades that tool |
| `protocol` | `1` |
| `language` | What it is written in, for a reader |
| `build` | Where the folder needs building: the command, run through `sh` in the tool's folder, that puts what the run command needs into the folder `BMO_TOOL_OUT` names. A Rust tool builds into the folder `BMO_CARGO_TARGET` names and copies its executable out. With no build the folder is placed whole |
| `check` | Its tests, run through `sh` in the tool's folder |
| `run` | The run command as words, read in the placed folder: the first is a file of the placed folder where one of that name stands there, and found on the host's path otherwise |
| `mode` | `call`, a process per call, unless the tool must keep something across one session's calls: then `server`, one MCP server per session record |
| `shared` | The shared helpers its calls are handed, by name, where it uses one |
| `after_deploy` | A command run from the placed folder once it is placed, where the tool needs one |

`timeout` and an entry's `unbounded` are read and given no effect, since no call is bounded by a clock: an author
sets neither.

### `[[entry]]`

| Key | What to set |
|---|---|
| `namespace` | The namespace it is shown under (「Namespaces, capabilities and grants」) |
| `name` | Lower-case letters, digits and `_`; with the namespace it makes the full name, `<namespace>/<name>`, which no other registered tool may hold |
| `title` | A short human title |
| `description` | What it does and answers, never when to reach for it. The first sentence is its line in the toolbook's list, so it stands alone |
| `manual` | A Markdown file inside the folder saying how to call it, with its arguments and examples; `describe` answers it whole after the description |
| `input` | The input schema's file, beside the manifest |
| `output` | The output schema's file, where it answers an object |
| `hints` | `read_only` for an entry that changes nothing, which is reached by `call`; any other is reached by `act`. `destructive` where it takes something away, `idempotent` where a second identical call does what the first did, `open_world` where it reaches outside this machine |
| `capabilities` | What a session must hold to be shown it; at least one |
| `grant` | The resource a session's grants must hold for a call of it to run, for an entry that acts on something some sessions may touch and others not |
| `system` | For the system alone: shown to no session, reached by another tool or the system itself |
| `ends_at_stop` | Where a call may wait on what stops the host, so the host's stop does not wait on it |

An entry may also declare a bundle's narrower form, a listing line, facts, the addresses it claims and the times
the host calls it by itself; `docs/api.md` states each table, and `docs/mcp-host.md` what the host does with it.

### `[app]`

| Key | What to set |
|---|---|
| `key` | Where the tool is something a company, a unit or a person may want off — a product of its own rather than a part of the system — the app's key: a lower-case letter, then lower-case letters, digits and `-`. A screen of the same work names the same key |
| `name` | The name a person sees it by in the settings |
| `description` | What it is for, one or two sentences |
| `icon` | The icon a screen shows it by |
| `offered` | `false` for an app made for some companies alone, which the platform's administrator then offers to them on the delegation chain; true otherwise |
| `on` | `false` for an app a person turns on for themselves; true otherwise |

A tool the system cannot work without names no app, and no setting ever withholds it. A put of a tool naming one
declares its two settings on the account service's delegation chain (`docs/api.md`, 「An app on the delegation
chain」), so the account service must be reachable where it is put. An app that is a unit's console screen alone,
with no tool, is declared by the unit's deployment with `bmo pocket app <file>`, a file holding the same `[app]`
table alone.

## Entries and their schemas

- **The input schema is a JSON Schema of an object,** one property per argument, each with a `description` the
  caller reads. The host checks every call's arguments against it before the tool runs and refuses a misfit,
  naming each fault and carrying the schema, so the tool reads arguments that already fit. Name what is
  required, and refuse what the schema does not name where a stray argument would be a mistake.
- **The output schema** states the object an entry answers in `structuredContent`, beside the same object as
  JSON in its text, which a caller reading text alone reads. An entry that answers only text names none.
- **A description and a manual are what a model reads to call it.** They say what the entry does, what each
  argument means and what comes back, in plain English, and nothing about who calls it.

## Namespaces, capabilities and grants

- **A tool is seen only by the sessions whose record names its namespace and holds its capability.** The record
  is written by the session's issuer before the session starts (`docs/mcp-host.md`, 「The session record」); the
  host compares names and asks nobody.
- **So choose names the intended sessions already hold.** A tool meant for an issuer's sessions goes under a
  namespace their records already name and asks a capability their records already hold, and is reached by
  them from the next call after it is put. A new namespace or a new capability is held by no session until its
  issuer names it, which is a change of the issuer's own; the issuer's documents say what its records hold.
- **Every entry asks for at least one capability,** unless it is for the system alone, so no tool is shown to
  every session by default.
- **A grant is for an entry that acts on a resource** some sessions may touch and others not: the resource's name
  is agreed with the issuer, who writes it into the records allowed to act on it, and the host refuses a call of
  the entry under any other record before the tool runs. A tool that only reads declares none.
- **`--allow` is the person putting it deciding what it may ask for.** A put prints every capability and grant
  the tool asks for and refuses, naming them, those no `--allow` names (「Putting it live」). So a tool never
  starts asking for more than was allowed: an upgrade asking for something new is refused until it is allowed
  again by name.

## The call protocol

`docs/api.md` (「The call protocol」) states it whole; what a tool does:

- **A call is one process.** The host starts the run command in the placed folder, in a clean environment — the
  host's `PATH`, `HOME` and `XDG_RUNTIME_DIR`, the variables the session's record hands its calls, and nothing
  else of the host's — and hands it one JSON object on standard input: the protocol version, the kind `call`,
  the entry's full name, its arguments, the session record, the call's call-back socket, and the socket of each
  shared helper it names.
- **It answers one JSON object on standard output and ends cleanly:** the tool result, its `content` (text
  pieces, or images), `isError`, and `structuredContent` where the entry declares an output schema. What it
  writes on standard error goes to the host's log.
- **A refusal the caller reads is a result with `isError` true** and an English sentence saying why; a tool that
  could not read what it answers from says its reason after `ERROR: `. An answer that does not read, or an
  unclean exit, is a failed call, named with the exit and the last lines of standard error.
- **A call waits for its tool however long it takes,** and is ended with its caller: a tool keeps no clock of its
  own over the host's call, and work that outlasts its caller is not left running.
- **The call-back socket** asks the host for another entry, under the same record and through the same schema,
  capability and grant checks, so a tool reaches another only as far as the session it serves could.
- **What each language uses:**
  - **Python:** the kit `sdk/py-kit/src/bmo_tool.py`, one file on the standard library: `answer` reads the call
    and writes the answer given the handlers by full entry name, and `text`, `structured`, `refusal` and
    `failure` make a result; a handler's `call_back` asks another entry.
  - **TypeScript:** the kit `sdk/ts-kit/src/tool.ts`, one file run by Bun, with the same verbs (`callBack`
    for another entry).
  - **Rust:** the protocol read and written with a JSON library, as the Rust example does with `serde_json`. The
    `bmo-tool` crate under `sdk/contract/` states the protocol and the manifest as the host reads them; a tool in
    another repository links no crate of this one.
  - **Any other language** reads one JSON object and writes one; nothing else is asked of it.
- **A server tool** (`mode = "server"`) is started at a record's first call with the same object, less the call,
  in its `BMO_TOOL_START` variable, speaks MCP on standard input and output, answers each call as a
  `tools/call` of the entry's name, and ends with the record.

## Testing a tool

Three steps, the first two touching no host:

1. **Its own tests,** the manifest's `check`, run in the tool's folder. They run the executable as the host does:
   a handed object on standard input, the answer read off standard output, as each example's tests do.
2. **Through a scratch host:** put the folder into a scratch data root and call it there with the host's own
   binary, which checks the schema, the capabilities and the grants exactly as the running host does and leaves
   the running host and its registry untouched:
   - `bmo-pocket --data-root <scratch> put <folder> --allow <capability>`
   - `mcp-host call <namespace> <name> --data-root <scratch> --tools <an empty folder>`, with the host's own
     binary, `mcp-host`, which stands beside `bmo-pocket` in the tool host's deploy tree, handed
     `{"session": …, "arguments": …}` on standard input, where `session` is a record as an issuer writes it
     (`docs/api.md`, 「The session record」) naming the tool's namespace and holding its capability. The empty
     tools folder keeps the deployed tools out, so the scratch registry alone is read.
3. **Live, once it is put:** `bmo tool describe <namespace>/<name>` reads it as the host serves it, and
   `bmo tool call <namespace>/<name> '<arguments as one JSON object>'` calls it under a session of its own
   holding the entry's capabilities and grant.

A call that fails says why in the host's log, with the tool's own standard error.

## Putting it live

- **`bmo pocket put <tool> --allow <name>…`** places a tool and registers it, served from the next call with no
  restart of the host or of any session (`docs/registry.md`). `<tool>` is the tool's folder, its repository's git
  address, or its bare name put from the configured source template. Each `--allow` names one capability or
  grant the tool may ask for; name exactly those its manifest asks for and its owner approved.
- **What it checks first:** the manifest reads and every schema is a JSON Schema; the name is no service's
  registration; no full name is held by another registered tool; and every capability and grant asked for is
  allowed. Each refusal names what failed, and nothing is placed.
- **What it says:** each capability and grant the tool asks for, each schedule it declares, and what it placed.
- **How sessions find it:** a booked entry stands in the toolbook of every session holding it, found with
  `bmo use --tools` and called with `bmo use`; it changes no running session's prompt.
- **`bmo pocket list`** prints every registered tool, its version, where it was put from and its entries.
- **`bmo pocket take <name>`** takes it out of the registry: it is refused as unknown from the next call, and a
  call already running ends as it would have.
- **Who puts it:** whoever installs it — a person by hand, a supervisor on its owner's word, or a service's
  deployment for that service's own tools. A run working on the tool's code writes, tests and commits it; it
  puts nothing.

## Versions and going back

- **Every put places a version of its own** and moves the tool's pointer onto it in one step; putting the same
  name again is the upgrade. `bmo pocket list` names the version in place and where it came from.
- **A version leaves once nothing holds it:** a call, a server or a shared helper started from a version goes on
  in it to its end, so an upgrade or a take never breaks a call in flight, and the host takes the old version
  away afterwards.
- **No old version is kept to go back to.** Going back is putting the earlier source again: the folder checked
  out at the earlier commit, or the repository's main once a revert stands there. A tool taken by mistake is put
  again the same way.
