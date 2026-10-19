# The boundary

**Fixed API, version 1.0.0.** This document is the tool host's fixed API: every contract it exposes to another unit
and every call it makes on one, grouped by the resource each acts on. It is fixed before any caller is connected
to it, and every caller is served the same routes in the same shapes. Another unit reaches the host only across it,
and the host reaches another unit only by the calls in 「What the host calls」: never by its code, its tables, its
files or its process. How the host behaves behind these contracts is `docs/mcp-host.md` and `docs/toolbook.md`;
how tools are installed is `docs/registry.md`.

## Versioning

- **The boundary carries the repository's one version, in semver,** the workspace's version in `Cargo.toml`, which
  the OpenAPI document and both clients carry too.
- **The major moves on a change an existing caller or tool reads:** the call protocol, the manifest schema, the
  session record, a route, a body, an error code, a variable or the CLI's words. The minor moves on an addition of
  one; the patch on anything else.
- **The call protocol has a version of its own, 1,** which every manifest and every handed object names; a
  manifest speaking another is left out rather than misread.

## The wire format

- **The common wire format of bmo-next,** as bmo-chat's `docs/conventions.md` states it: keys in camelCase, times as
  one integer of epoch milliseconds, error codes in kebab-case, and a list paged by one cursor and `limit`. This
  document states only what the host serves in it.
- **Every failure and every refusal is a non-2xx status** with a JSON body of two keys: `code`, a stable code a
  caller decides on, and `error`, an English sentence saying why. No 2xx answer stands for a refusal of the host's.
- **A tool's own answer is not the host's refusal.** A call the host made and the tool answered is 200 with the
  tool's result as the tool wrote it, `isError` among it where the tool says it failed.

### Error codes

| Code | Status | Answered where |
|---|---|---|
| `no-token` | 401 | a request on the tools face carries no bearer token |
| `unknown-token` | 401 | the token names no record: never issued, revoked, or past its deadline |
| `unknown-namespace` | 404 | the tools face or `GET /facts/{namespace}` names a namespace no placed entry is declared under |
| `no-listing` | 404 | `GET /listings/{namespace}`, where no entry of the namespace declares a listing line |
| `unknown-entry` | 404 | `GET /catalogue/{namespace}/{name}` names no entry, or `POST /calls` names one the record does not hold |
| `malformed-record` | 400 | `POST /sessions` is handed what does not read as a session record |
| `malformed-request` | 400 | a body or a query that does not read as the route asks: `POST /calls`, `POST /claims`, a cursor |
| `arguments-misfit` | 422 | `POST /calls`, where the arguments do not fit the entry's schema in the form held; the sentence names each misfit and carries the schema |
| `grant-refused` | 403 | `POST /calls`, where the grant lock refuses the call; the sentence names the missing grant or why none could be asked |
| `tool-failed` | 502 | `POST /calls`, where the tool exited uncleanly, answered what is not a tool result, or ran past its timeout |
| `unwritable-record` | 500 | a record could not be kept or taken back on disk |
| `unknown-route` | 404 | a path the control socket does not serve |
| `method-not-allowed` | 405 | a method a served path does not take |

## The OpenAPI document and the clients

- **`docs/openapi.json` states every route of the control socket** in OpenAPI 3.1: each route's method, path,
  query, request body, answer and the error answers it can give, every JSON body as a JSON Schema. It is generated
  by the Rust client from the types it sends and reads, which are the types the host answers with, and never edited
  by hand: `cargo run -p bmo-pocket-client --example openapi > docs/openapi.json` writes it again, and the client's
  tests fail where the committed file differs from what it generates. This document is the canonical statement:
  where the two disagree, the code is repaired and the file generated again.
- **The Rust client, the crate `bmo-pocket-client` in `client/`,** holds the wire types and one call per route over
  the control socket. A failure reads as the route's error answer, its `code` and `error` with it, or as the socket
  not being reached at all, which a caller waiting on a restarting host tells apart.
- **The TypeScript client, the package `@bmo/pocket-client` in `client/`,** is generated from the same document:
  its types by `openapi-typescript` into `openapi.ts`, which its tests regenerate and compare, and its calls by
  `openapi-fetch` over the control socket. `bun run generate` in `client/` writes `openapi.ts` again.
- **A client carries this contract and nothing else.** A caller uses one of them and writes none of its own.

## The tools face: MCP over HTTP

- **On the loopback address only,** at the port `--port` or `BMO_MCP_HOST_PORT` names, 3290 where neither does.
- **`/mcp/<namespace>`** for every namespace a placed manifest declares, and **`/mcp/toolbook`** for the toolbook;
  each is a streamable HTTP MCP server with no protocol session kept: every request stands alone, each answer one
  JSON body. The OpenAPI document does not state this face: it is MCP's own protocol.
- **Every request carries `Authorization: Bearer <token>`,** the token a record was issued with. A request without
  one is answered 401 `no-token`; a token naming no record, 401 `unknown-token`; a namespace no placed manifest
  declares, 404 `unknown-namespace`. Each in the common error shape.
- **What a namespace serves:** `tools/list` lists the entries the record holds in it that its `shown` names, each
  with its name, title, description, input schema, output schema where it declares one, and its annotations
  (`readOnlyHint`, `destructiveHint`, `idempotentHint`, `openWorldHint`); `tools/call` calls any entry the record
  holds in it, resident or booked; `resources/list` and `resources/read` serve the resources of every tool
  declaring them in it, for a record holding their capabilities.
- **What the toolbook serves:** four tools, `list` (`query?`), `describe` (`tool`), `call` (`tool`, `arguments`)
  and `act` (`tool`, `arguments`), where `tool` is a full name, `<namespace>/<name>`, and `arguments` the inner
  tool's own arguments as one object (`docs/toolbook.md`). A reader of a session's calls reads a `call` or an `act`
  of the server `toolbook` as the inner call by these two arguments.
- **A refusal a model reads is a tool result:** a call the record may not make, arguments that do not fit, a grant
  the lock refuses and a tool that failed are each answered as a tool result with `isError` true and an English
  text, since the protocol's caller is a model.
- **Every result says it is complete,** and a listing or a resource read says it may be kept by the asking session
  alone and never past its answer, for a client on a protocol revision that asks; an earlier revision is answered
  without them.

## The control socket

- **HTTP/1.1 over a Unix socket,** at the path `--control` names, or `bmo/mcp-host.sock` under the user's runtime
  directory. Only the host's own user can reach it, so it carries no token. Bodies are JSON both ways unless a route
  says otherwise.

### Sessions

| Route | Request | Answer |
|---|---|---|
| `POST /sessions` | a session record | 200 `{"token"}`; 400 `malformed-record` naming why the record does not read; 500 `unwritable-record` |
| `DELETE /sessions/{token}` | | 204, whether or not the token named a record, what its tools kept ended; 500 `unwritable-record` |

#### The session record

One JSON object. A field the record does not name is refused, so an issuer handing something new is told rather than
ignored.

| Field | Type | Meaning |
|---|---|---|
| `line` | string, required | The run line the record is issued for, a name a tool may read |
| `role` | string | The role of the agent it is issued for, a name a tool may read |
| `capabilities` | array of strings, required | The capabilities the record holds, as the issuer resolved them |
| `shown` | array of strings | The full names of the entries its namespaces list by name; none listed by name where absent |
| `checkout` | path | The checkout the run works in |
| `dataRoot` | path, required | The data root the run reads; handed to its tools as `BMO_DATA_DIR` |
| `archive` | string | The archive address its tools read; handed to them as `BMO_ARCHIVE_URL` where present |
| `place` | path | A resident program's place: the folder whose `run.json` names the run going |
| `runVariables` | object of strings | The run's own variables, for a run spawned for itself |
| `fragmentDesk` | path | The fragment desk an editor's batch opened |
| `research` | object | What a research run is held to: `closesAt` (epoch ms), `owner`, `source` (`search` or `browse`), `role`, `branches`, `chat` (a room's id), `strategy`, `hostIntervalMs`, `engineIntervalMs`, `rulebooks` (array of strings) |
| `deadline` | epoch milliseconds | When a run's record lapses; a resident program's has none |

- **The turn a call is made in** is the record's `runVariables`, overlaid by the `env` object of `run.json` in its
  `place` where one stands and is not a person's (`"person": true`): of those, `BMO_RUN_NAME`, `BMO_TASK_LOCKED`,
  `BMO_WORK_ORDER_STEP` and every `BMO_TURN_` variable. The host reads `BMO_TURN_CHAT`, `BMO_TURN_PRIVILEGED` and
  `BMO_TURN_USER` of it for the grant lock, and hands the whole turn to each call's environment.

### Namespaces

| Route | Answer |
|---|---|
| `GET /listings/{namespace}` | 200 the namespace's listing as plain text, one line per entry that declares one, in its order; 404 `no-listing` where no entry does |
| `GET /facts/{namespace}` | 200 the namespace's facts: `digest`, the SHA-256 of what it shows, and each list, verb, record and icon its entries add, as `docs/mcp-host.md` states them; 404 `unknown-namespace` where no entry is declared under it |

### The catalogue

| Route | Query | Answer |
|---|---|---|
| `GET /catalogue` | `after` or `before`, an entry's full name; `limit` | 200 an array of entries in order of full name; 400 `malformed-request` for both cursors at once or a `limit` that is not a positive integer |
| `GET /catalogue/{namespace}/{name}` | | 200 one entry with its description and schemas; 404 `unknown-entry` |

- **The cursor is an entry's full name,** its identifier: `after` answers the entries whose full names sort after
  it, `before` those that sort before it, each in ascending order of full name, at most `limit` of them nearest the
  cursor. Without a cursor the catalogue answers from its start; without `limit`, every entry from the cursor on.
  Every entry is answered, those for the system alone among them.

| Field of an entry | Type | Meaning |
|---|---|---|
| `name` | string | Its full name, `<namespace>/<name>` |
| `readOnly` | boolean | Whether it only reads |
| `reachedBy` | `call` or `act` | The toolbook tool it is called through |
| `capabilities` | array of strings | What a record must hold to be shown it whole |
| `grant` | string or null | The room relation the sender of the message it acts on must hold |
| `system` | boolean | Whether it is for the system alone |
| `resident` | boolean | Whether its namespace lists it by name; false in the catalogue, which no record is shown |
| `placed` | string | Its tool's placed folder name |
| `description` | string | What it does |
| `icon` | string or null | The icon its manifest declares for it, a name its readers know |
| `schedules` | array | Its schedules, each as below; empty where it declares none |

| Field of a schedule | Type | Meaning |
|---|---|---|
| `cron` | string | The pattern its manifest declares |
| `arguments` | object | The arguments each scheduled call is handed |
| `nextAt` | epoch ms or null | When it is next due; null where the pattern names no time to come |
| `lastAt` | epoch ms or null | When its last call was made since the host started; null where none was |
| `lastError` | string or null | Why its last call failed, the host's refusal or the tool's own failed result; null where it did not |
| `running` | boolean | Whether a call of it is running now |

- **One entry described** carries the fields above and `described`, the text the toolbook's `describe` answers for
  it, `inputSchema`, and `outputSchema`, null where it declares none.

### Calls

| Route | Request | Answer |
|---|---|---|
| `POST /calls` | `{session, tool, arguments?}`: a record, a full name and the arguments | 200 the tool result as the tool answered it; 400 `malformed-request`; 404 `unknown-entry`; 422 `arguments-misfit`; 403 `grant-refused`; 502 `tool-failed` |
| `POST /claims` | `{session, address}` | 200 `{"entry", "arguments"}`, the entry the record holds that claims the address and the arguments it reads the page with, both null where no entry claims it; 400 `malformed-request` |

- **A call through `/calls` is dispatched as a session's call is,** under the record it carries, through the same
  schema, capability and grant checks, and reaches the entries for the system alone too. What that record's tools
  kept ends as it answers.

### Health

| Route | Answer |
|---|---|
| `GET /health` | 200 `ok`, as plain text |

## The variables

| Variable | Read by | Meaning |
|---|---|---|
| `BMO_MCP_HOST_PORT` | the host; a line's configuration | The loopback port the tools face listens on |
| `BMO_MCP_TOKEN` | a session's CLI | The bearer token its configuration's header expands; set in the program's environment by its launcher |
| `BMO_MCP_HOST_SOCKET` | a launcher; the clients | Where it reaches the control socket, where it does not stand at the default path |
| `BMO_DATA_DIR` | the host; every call | The host's data root; in a call's environment, the record's |
| `BMO_ARCHIVE_URL` | every call | The record's `archive` |
| `BMO_TOOL_START` | a server tool | Its start object (「The call protocol」) |
| `BMO_SCHEDULED_AT` | a scheduled call | The time it was due, in epoch milliseconds (「Schedules」) |
| `BMO_ACCOUNTS_URL`, `BMO_ACCOUNTS_TOKEN_FILE`, `BMO_GRANT_LOCK` | the host | The account service it asks, the file holding its bearer token, and the grant lock's switch (「What the host calls」) |
| `BMO_POCKET_SOURCE` | `bmo-pocket` | The source template a bare tool name is put from (「The CLI」) |

- **The deployed host reads the host's variables from `env/pocket.env` under the data root,** where the deployment
  writes them; its user unit names that file.

## The call protocol

Version 1. The host starts a placed tool's run command in its placed folder, in a process group of its own, with a
clean environment: the host's `PATH`, `HOME` and `XDG_RUNTIME_DIR`, each variable `--pass-env` names, `BMO_DATA_DIR`,
`BMO_ARCHIVE_URL` where the record names an archive, and the record's turn.

### What a tool is handed

One JSON object on standard input, or in `BMO_TOOL_START` for a server's start:

| Field | Meaning |
|---|---|
| `protocol` | 1 |
| `kind` | What is asked, with the fields of its kind beside it (below) |
| `session` | The session record the call is made under |
| `callback` | The call's own call-back socket into the host; none for a helper |
| `shared` | Each shared helper's socket, by the helper's name; none where the tool names none |

| `kind` | Its fields | Answered on standard output with |
|---|---|---|
| `call` | `entry` (full name), `arguments` | an MCP tool result: `content` (an array of content items), `isError`, and `structuredContent` where the entry declares an output schema |
| `claim` | `entry`, `address` | `{"arguments": …}`, the arguments the entry reads the address with, or `{"arguments": null}` |
| `resources_list` | | `{"resources": [...]}`, MCP resource descriptions |
| `resources_read` | `uri` | `{"contents": [...]}`, MCP resource contents, or `{"error": "<why>"}` where it holds none |
| `helper` | `socket` | a first line on standard output once it listens on `socket`, then it serves the record's calls until asked to end |
| `start` | | (a server's start, in `BMO_TOOL_START`) MCP over standard input and output, each call a `tools/call` of the entry's name |

- **A call answers one JSON object and ends cleanly.** An exit that is not clean, or an answer that does not read as
  what its kind asks, is a failed call, named with its exit and the last lines it wrote on standard error, which
  goes to the host's log.
- **A refusal a model reads** is a tool result with `isError` true and an English text; a tool that could not read
  what it answers out of answers its reason after `ERROR: `.
- **A call past its timeout,** the entry's or the tool's, has its process group ended and fails; an unbounded entry
  has none. A call whose caller goes away has its process group ended.
- **A server** keeps one process per record across its calls, ends with the record, and is started again at the
  record's next call after a call past its timeout ended it.

### The call-back socket

- **One JSON object asked on one line, one answered on one line,** on the socket `callback` names, which stands as
  long as the call (a server's, as long as the server):

| `kind` | Its fields | Answered with |
|---|---|---|
| `call` | `entry` (full name), `arguments?` | the entry's tool result, dispatched under the same record through the same schema, capability and grant checks as a session's call; an entry for the system alone among those reached |
| `claim` | `address` | `{"entry", "arguments"}` of the entry the record holds that claims the address, or `{"entry": null}` |

### A shared helper's socket

- **The helper's own protocol,** between it and the tools that name it: the host only starts it, hands its socket to
  each call of the record naming it, and ends it with the record — `SIGTERM`, ten seconds, then its process group.

## The manifest

`tool.toml`, beside the schema files its entries name. A key the schema does not name is refused, naming it.

### `[tool]`

| Key | Type | Meaning |
|---|---|---|
| `name` | string, required | The tool's name, which is its folder's |
| `protocol` | integer, required | The call protocol it speaks, 1 |
| `language` | string | What it is written in, for a reader |
| `build` | string | The command run through `sh` in the tool's source folder that puts what the run command reads into `BMO_TOOL_OUT`; none where the folder is placed whole |
| `check` | string | Its tests, run through `sh` in the tool's source folder |
| `run` | array of strings, required | The run command as words: the first is a file of the placed folder where one of that name stands there, and found on the path otherwise |
| `mode` | `call` or `server` | A process per call, or one MCP server per record; `call` by default |
| `timeout` | integer, required | A call's timeout in seconds, or a helper's bound on coming up; more than 0 |
| `shared` | array of strings | The shared helpers its calls are handed, by name |
| `after_deploy` | array of strings | A command as words run from the placed folder once the tool is placed |

### `[[entry]]`

| Key | Type | Meaning |
|---|---|---|
| `namespace` | string, required | The namespace it is shown under: a lower-case letter, then lower-case letters, digits, `_` and `-`; never `toolbook` |
| `name` | string, required | Lower-case letters, digits and `_` |
| `title` | string | Its title |
| `description` | string, required | What it does; its first sentence is its line in the toolbook's list |
| `input` | string, required | The input schema's file, beside the manifest |
| `output` | string | The output schema's file, where it answers an object |
| `hints` | table | `read_only`, `destructive`, `open_world` (booleans) and `idempotent`; a read-only entry is reached by `call`, any other by `act` |
| `capabilities` | array of strings | What a record must hold to be shown it whole; required unless `system` |
| `grant` | string | The room relation the sender of the message it acts on must hold (`docs/mcp-host.md`, 「The grant lock」) |
| `timeout` | integer | A timeout of its own, in seconds |
| `unbounded` | boolean | A call that waits on what it set off as long as that takes, with no timeout; not with `timeout` |
| `system` | boolean | For the system alone: shown to no session, reached through the call-back socket and `/calls` |

- **`[entry.bundle.<capability>]`:** the narrower form a record holding that capability is shown the entry in:
  `shown`, the arguments it shows, and `required`, those it requires.
- **`[entry.listing]`:** `line`, the line it adds to its namespace's listing, and `order`.
- **`[entry.facts]`:** `verb`, the verb a sending entry needs; `record`, the record a recording entry leaves;
  `icon`, the icon its readers show it by, in its namespace's facts and its catalogue entry; and `lists`, by a
  list's name, the item it adds to that list of its namespace's facts.
- **`[entry.claims]`:** `hosts`, the hosts whose pages it claims, each with its subdomains.
- **`[[entry.schedule]]`:** a time the host calls the entry by itself (「Schedules」): `cron`, the pattern, and
  `arguments`, the table each call is handed, none by default.

### The other tables

- **`[resources]`:** `namespace`, the namespace its executable serves resources under, and `capabilities`, what a
  record must hold to be shown them.
- **`[helper]`:** `description`, what it holds for a record's calls, in a shared helper's manifest in place of
  entries.
- **`[card]`:** any table, a declaration another unit reads of the tool; the host reads none of it.

### What does not read

- A protocol other than 1, a name other than its folder's, a timeout of 0 or a run command naming nothing.
- Neither an entry nor resources, or a helper's manifest with entries.
- An entry with a namespace or name outside its alphabet, declared twice, with no description, asking for no
  capability without being for the system alone, with a capability or a grant with no name, unbounded with a
  timeout, or naming a schema file not beside the manifest or not a JSON Schema.
- A schedule whose pattern does not read.
- Resources naming no namespace or no capability; a server tool declaring claims or resources.
- An entry whose full name another tool already holds.

## Schedules

- **An entry declaring a schedule is called by the host itself** at each time its pattern names, with the
  schedule's arguments, as `docs/mcp-host.md` states (「Schedules」).
- **The pattern is a cron pattern as the `croner` library reads it:** minute, hour, day of month, month and day of
  week, an optional seconds field before them and an optional year after, and nicknames written with a leading `@`, such as `daily`. It is
  read in the host machine's local time zone.
- **A scheduled call is made under a record of its own:** `line` `schedule`, the entry's own capabilities, the
  host's data root, and the run variables `BMO_TURN_PRIVILEGED` `1`, since no person's message opened it, and
  `BMO_SCHEDULED_AT`, the time it was due. It passes the schema and grant checks a call does.
- **What a schedule stands at is read in its entry's catalogue entry** (「The catalogue」).

## The CLI

- **`bmo-pocket`,** which `bmo pocket <verb>` hands its words to by the `bmo-<command>` rule. Its verbs stand on the
  data root `--data-root` or `BMO_DATA_DIR` names. It prints what it did on standard output and why it refused on
  standard error, and ends 0 where it did what was asked, 1 where it refused, and 2 where its words do not read.
- **A tool registers itself:** whoever installs a tool, a person or a service's deployment, runs `put` for it, and
  no list of tools stands in the host or in this document.

| Verb | What it does |
|---|---|
| `put <tool> [--allow <name>]… [--service <repo>] [--placed] [--source <template>]` | Places a tool and registers it, or upgrades the tool registered under its name: `<tool>` is a local folder, a git address (one holding `://`, or `git@`, or ending in `.git`), cloned, or a bare name, made into the repository name `bmo-<name>` and put from the source template `--source` or `BMO_POCKET_SOURCE` names, `{repo}` in it standing for that repository name (a bare name is refused where none is configured) |
| `take <name> [--service <repo>]` | Takes the tool registered under the name out of the registry; a service's own tool only with `--service` naming that service |
| `list [--json]` | Every registered tool: its name, its version, where it was put from, the service it is registered for and its entries; `--json` prints the pointers (`docs/registry.md`, 「A pointer」) as a JSON array |

- **What `put` checks before anything is placed:** the manifest reads (「The manifest」); its name is no other
  kind's registration, and each of its entries' full names is held by no other registered tool; and every
  capability and grant it asks for is named by an `--allow`. It prints each one asked for, and each schedule the
  tool declares, and refuses naming those not allowed.
- **What `put` places:** it runs the manifest's `build` with `BMO_TOOL_OUT` naming a new version folder and
  `BMO_CARGO_TARGET` a build folder of its own, and copies the manifest and its schema files beside what the build
  put there; with no `build`, or with `--placed` for a folder a service's deployment already built, it copies the
  folder whole. Then it points the registry at the new version in one step, and runs `after_deploy` where the
  manifest names one.
- **`--service <repo>`** registers a service repository's own tool: put from a local folder, marked as that
  repository's. A tool put without `--service` never replaces one a service registered, and a service never
  replaces one put without it.
- **`take` and an upgrade** only move the registry's pointer; the host takes a retired version away once no call
  holds it (`docs/registry.md`).

## What the host calls

### The account service

- **Where:** the base address `--accounts` or `BMO_ACCOUNTS_URL` names, with the bearer token the file
  `--accounts-token-file` or `BMO_ACCOUNTS_TOKEN_FILE` names; the grant lock stands only where an address is named
  and `--grant-lock` or `BMO_GRANT_LOCK` does not read `off`.
- **The calls, as the account service's own boundary document in bmo-web states them** (its JSON keys as it writes
  them):

| Call | Request | Read of the answer |
|---|---|---|
| `GET /accounts/model` | | `in_force`; the lock stands only where it is true |
| `POST /checks/object` | `kind` `bmo_tools`, `object` the capability, `relation` `use`, `user` from `BMO_TURN_USER` | `allowed`, `grant` |
| `POST /checks/message` | `room` from `BMO_TURN_CHAT`, `message` from the call's `message` argument, `relation` the entry's `grant` | `allowed`, `grant` |

- **A call that does not answer, or answers other than 200,** refuses the tool call with an English sentence naming
  the account service and what it answered; a refusal's `code` and `error` are carried in it.
