# The toolbook

## What it is

- **A session is shown by name only the tools it uses often, and reaches every other tool it holds through
  one small server, the toolbook,** which lists them, describes one and calls one on the session's behalf.
- **The toolbook is a layer of the tool host** (`docs/mcp-host.md`), not a server of its own and not a family of
  tools: it answers from the same catalogue and dispatches through the same path a direct call takes.

## Why

- **Every schema a session is shown stands in its prompt.** A session holding a namespace of many tools would
  carry every one's description and schema from its first request, though most sessions call few of them.
- **Adding a tool must not move a running session's prefix.** A booked tool joins the list the toolbook answers
  with, not the tool list the session was started with, so a tool placed, upgraded or taken away changes no
  session's prompt.

## The tools a session is shown

- **Resident tools,** called by their own names.
- **The toolbook's four tools,** under the server name `toolbook`, for every record the host serves:
  - **`list`:** the booked tools this session may call, one line each: the tool's full name, which of `call` and
    `act` reaches it, and the first sentence of its description. An optional `query` narrows the list to the tools
    whose full name or description holds it.
  - **`describe`:** one tool's description, its input schema, its read-only hint and, where it declares one, the
    schema of what it answers with, and which of `call` and `act` reaches it.
  - **`call`:** calls one read-only tool with its arguments.
  - **`act`:** calls one tool that acts.
- **The toolbook's own descriptions say how a booked tool is reached,** and nothing else need. `list`'s
  description tells the session that a tool its instructions name and it is not shown is one of the toolbook's,
  reached by its full name; so booking a tool, or making it resident again, moves no prompt, and every harness
  reads the same sentence, since a tool's description is part of every tool list.
- **Why a call and an act.** CLIs send calls to tools declared read-only side by side. A proxy declared read-only
  may only forward to read-only tools, so `call` is declared read-only and refuses a tool that acts, naming `act`;
  `act` is not declared read-only and forwards to any tool. Parallel reads therefore keep going through the
  toolbook.
- **A tool's full name is its namespace and its name, joined by a slash** — `<namespace>/<name>` — which is also
  how some CLIs name an MCP tool. The two arguments of `call` and `act` are that full name, `tool`, and the tool's
  own arguments as one object, `arguments`.
- **Usage reaches the session only as a tool result.** Nothing the toolbook answers is put into a system prompt, a
  warm prefix or a request's prompt, so a session's cached prefix never moves for it. The `list` and `describe`
  answers stand in the session like any other result.

## The rules a prompt carries

- **The toolbook's descriptions say how one booked tool is reached; a session's instructions say how tools are
  found and used at all.** The host owns that paragraph, since the toolbook is its own, and serves it as text on
  the control socket's `GET /discipline` (`docs/api.md`, 「Discipline」). An issuer writes it whole into the system
  prompt of each session it hands the toolbook to, and writes nothing of its own about the toolbook beside it.
- **What it tells the session:**
  - an unknown tool is found with `list`, read with `describe` before its arguments are filled, and called by its
    full name: a read through `call`, an action through `act`;
  - the tools reached through the toolbook change while the session runs, so a tool's name is never taken from
    memory or guessed, but found again;
  - long work is not waited on: where a tool answers that it has started something, its result comes back as a
    message, and the session goes on rather than calling again to see whether it is done;
  - a refusal of the arguments carries the tool's schema, so the session mends them and calls once more.
- **It is one English paragraph under a heading,** kept as `prompts/tool-use.md` and built into the host, so the
  text a host serves is the text of the version it runs. It moves no running session's prefix: an issuer reads it
  when it builds a session's prompt.

## Which tools are resident

- **The record's issuer names them.** A session record carries `shown`, the full names of the entries its
  namespaces list by name, resolved by the issuer by a rule of its own; the host reads that list and nothing else,
  and a tool declares nothing of it. An entry is resident for a session where its
  record's `shown` names it, and booked otherwise, so an entry `shown` does not name stands in the toolbook of
  every session holding it.
- **A record without `shown` is shown none by name:** every entry it holds stands in its toolbook.
- **A tool's namespace lists only the shown entries the session holds.** Every other entry it holds is left out of
  the namespace's tool list and stands in its toolbook instead. The host still answers a direct call to a booked
  entry the session holds, so a session whose CLI listed its tools before the entry was booked goes on calling it
  by the name it was shown.
- **Which entries an issuer shows, and by what rule,** is the issuer's policy; the host carries no residency rule.

## Each session's toolbook

- **A session's toolbook holds the entries its record's capabilities open that its `shown` does not name,** in
  every namespace its record names, read from the catalogue as the host last read it, as its namespaces' lists hold
  the shown ones. So each session has a toolbook of its own, made of what it holds, and no tool names a holder.
- **A tool added or taken away is in or out of every toolbook from the next `list`,** with no restart of the host
  or of the session: the toolbook answers from the catalogue as the host last read it (`docs/mcp-host.md`,
  「Placed tools」).

## How a proxied call is dispatched

- **One path for both ways in.** A direct call and a toolbook call reach the tool through the same dispatch in the
  host:
  - the session record is looked up by the token;
  - the tool must be one the session holds (its record names the tool's namespace and holds the tool's
    capabilities, or a bundle of it), or the call is refused as an unknown tool, by the same words either way;
  - for a tool declaring a grant, the record's grants must hold it (`docs/mcp-host.md`, 「Grants」);
  - the tool runs with the session's record and its answer is returned unchanged.
- **A call takes as long as its tool does either way.** The toolbook adds no wait and no cut of its own, so an
  unbounded entry called through `act` waits for its work exactly as it does called directly.
- **The arguments are checked against the tool's schema in the host,** before the tool runs, with a maintained
  JSON Schema validator, the `jsonschema` crate. A refusal names each argument that failed and why, and carries the
  tool's schema, so the caller can call again without asking `describe` first.
- **A full name the session is not shown** — no tool of that name, a namespace its record does not name or holds
  nothing of, a tool its record does not open — is refused as unknown by the same words a direct call is.
- **`call` handed a tool that acts** is refused with `act` named, before anything runs.
- **`list` and `describe` show only what `call` and `act` would accept.** Each is built from the same session
  filter, so no listed tool is refused as unknown and no unlisted tool is callable.
- **Every harness configuration names `toolbook` beside its namespaces,** at the host's `/mcp/toolbook`, with the
  same bearer header.
- **A harness's allow list names servers, never single MCP tools.** A configuration that lets a harness reach a
  server's tools lets it reach `toolbook` too. Narrowing which tools a session may use is a capability its record
  holds, never an entry in an allow or deny list, since such an entry cannot see a tool reached through the
  toolbook.

## How a proxied call is read

- **A call through the toolbook is the tool it reached.** Whatever shows or counts a session's calls reads a `call`
  or an `act` of the `toolbook` server as the inner call: the inner tool's namespace and name from `tool`, its
  arguments from `arguments`, and the fact that it came through the toolbook. The shape it reads is the one this
  document states (「The tools a session is shown」), which the host holds stable as part of its boundary
  (`docs/api.md`).
- **`list` and `describe` show as themselves,** as calls of the toolbook server, since they reached no tool.
